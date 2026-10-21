use super::identity::{self, handle};
use crate::{InstalledAppCatalogTarget, OS};
pub fn activate_registered_aumid(aumid: &str) -> Result<(), String> {
    if aumid.is_empty() || aumid.len() > 512 || aumid.chars().any(char::is_control) {
        return Err("Invalid registered application identity".into());
    }
    if !OS::list_supported_start_apps()?
        .iter()
        .any(|entry| matches!(&entry.target,InstalledAppCatalogTarget::Aumid(id)if id==aumid))
    {
        return Err("The application is not in this user's supported installed-app catalog".into());
    }
    let context = identity::launch_context(false)?;
    let shell = context
        .shell
        .as_ref()
        .ok_or("No verified interactive shell")?;
    let pid = unsafe { windows::Win32::System::Threading::GetProcessId(handle(shell)) };
    if pid == 0 {
        return Err("Cannot identify the verified interactive shell".into());
    }
    super::super::shell::execute_in_verified_explorer_process(
        &format!("shell:AppsFolder\\{aumid}"),
        None,
        pid,
    )
}
