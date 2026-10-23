//! Independent registered task or installed Framework context, never a parent-PID heuristic.
use super::{directory, native, trust::TrustCache, unsupported};
use crate::{
    background::*,
    gaming::{GamingError, ProcessIdentity},
};
use windows::{
    Win32::System::{
        Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        TaskScheduler::*,
        Variant::VARIANT,
    },
    core::{BSTR, Interface},
};

struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn action_path(value: &str) -> Result<std::path::PathBuf, GamingError> {
    let lower = value.to_ascii_lowercase();
    for prefix in [r"%windir%\", r"%systemroot%\"] {
        if lower.starts_with(prefix) {
            return Ok(directory(true)?.join(&value[prefix.len()..]));
        }
    }
    Ok(std::path::PathBuf::from(value))
}

pub(super) fn verify(
    role: Role,
    process: &ProcessIdentity,
    trust: &mut TrustCache,
    scm: &[ServiceMember],
) -> Result<RoleEvidence, GamingError> {
    if role == Role::FrameworkOptimizer {
        return framework(process, scm);
    }
    let (folder, task_name) = match role {
        Role::Appraiser => (
            r"\Microsoft\Windows\Application Experience",
            "Microsoft Compatibility Appraiser",
        ),
        Role::DeviceCensus => (r"\Microsoft\Windows\Device Information", "Device"),
        _ => {
            return Err(unsupported(
                "Independent background role evidence unavailable",
            ));
        }
    };
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() && initialized.0 as u32 != 0x80010106 {
        return Err(unsupported("Task Scheduler apartment unavailable"));
    }
    let _apartment = Apartment(initialized.is_ok());
    let scheduler: ITaskService =
        unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| native("Task Scheduler role", e))?;
    let empty = VARIANT::default();
    unsafe { scheduler.Connect(&empty, &empty, &empty, &empty) }
        .map_err(|e| native("Task Scheduler connection", e))?;
    let folder_handle = unsafe { scheduler.GetFolder(&BSTR::from(folder)) }
        .map_err(|e| native("Background task folder", e))?;
    let task = unsafe { folder_handle.GetTask(&BSTR::from(task_name)) }
        .map_err(|e| native("Background registered task", e))?;
    if unsafe { task.State() }.map_err(|e| native("Background task state", e))?
        != TASK_STATE_RUNNING
    {
        return Err(unsupported("Registered background task is not running"));
    }
    let definition =
        unsafe { task.Definition() }.map_err(|e| native("Background task definition", e))?;
    let actions =
        unsafe { definition.Actions() }.map_err(|e| native("Background task actions", e))?;
    let mut count = 0;
    unsafe { actions.Count(&mut count) }.map_err(|e| native("Background task action count", e))?;
    if count != 1 {
        return Err(unsupported("Unqualified multi-action background task"));
    }
    let action = unsafe { actions.get_Item(1) }.map_err(|e| native("Background task action", e))?;
    let mut kind = TASK_ACTION_TYPE::default();
    unsafe { action.Type(&mut kind) }.map_err(|e| native("Background task action type", e))?;
    if kind != TASK_ACTION_EXEC {
        return Err(unsupported("Unqualified background COM task action"));
    }
    let action: IExecAction = action
        .cast()
        .map_err(|e| native("Background executable action", e))?;
    let mut path = BSTR::default();
    let mut arguments = BSTR::default();
    unsafe { action.Path(&mut path) }.map_err(|e| native("Background action path", e))?;
    unsafe { action.Arguments(&mut arguments) }
        .map_err(|e| native("Background action arguments", e))?;
    if !action_path(&path.to_string())?
        .to_string_lossy()
        .eq_ignore_ascii_case(&process.image_path)
    {
        return Err(unsupported("Registered task executes a different image"));
    }
    let arguments = arguments
        .to_string()
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    let (component_path, component_identity) = match role {
        Role::Appraiser if arguments == "-m:appraiser.dll -f:doscheduledtelemetryrun" => {
            let component = directory(false)?.join("appraiser.dll");
            let identity = trust.verify(&component)?;
            (
                Some(component.to_string_lossy().into_owned()),
                Some(identity),
            )
        }
        Role::DeviceCensus if arguments.is_empty() => (None, None),
        _ => return Err(unsupported("Unqualified background task arguments")),
    };
    Ok(RoleEvidence::ScheduledTask {
        task_path: format!("{folder}\\{task_name}"),
        action_path: process.image_path.clone(),
        arguments,
        component_path,
        component_identity,
    })
}

fn framework(
    process: &ProcessIdentity,
    scm: &[ServiceMember],
) -> Result<RoleEvidence, GamingError> {
    use winreg::{
        RegKey,
        enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY},
    };
    if scm.iter().any(|member| member.pid == process.pid) {
        return Err(unsupported(
            "Framework service variant requires separate native qualification",
        ));
    }
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\NET Framework Setup\NDP\v4\Full",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .map_err(|_| unsupported("Installed Framework role unavailable"))?;
    let release: u32 = key
        .get_value("Release")
        .map_err(|_| unsupported("Installed Framework release unavailable"))?;
    if release == 0 {
        return Err(unsupported("Unknown installed Framework version"));
    }
    let root = std::path::Path::new(&process.image_path)
        .parent()
        .ok_or(unsupported("Framework root unavailable"))?;
    let facts = crate::windows::OS::tuning_process_identity(process.pid)
        .map_err(|_| unsupported("Framework token unavailable"))?;
    if facts.creation_time() != process.creation_token || facts.image_path() != process.image_path {
        return Err(GamingError::Changed);
    }
    if facts.token().sid() != "S-1-5-18" || facts.token().session_id() != 0 {
        let user = crate::gaming::interactive_user()?;
        if user.sid != facts.token().sid() || user.logon != facts.token().authentication_id() {
            return Err(unsupported(
                "Framework process belongs to another interactive logon",
            ));
        }
    }
    Ok(RoleEvidence::Framework {
        installed_release: release,
        root: root.to_string_lossy().into_owned(),
        service_name: None,
    })
}
