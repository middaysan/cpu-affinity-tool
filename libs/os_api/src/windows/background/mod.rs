//! Read-only qualification. Writes must repeat admission on an exact retained process handle.
#[cfg(all(test, feature = "tuning-test-controls"))]
mod native_tests;
mod placement;
mod roles;
mod services;
mod trust;
mod writes;
pub(crate) use placement::placement_topology;
pub(crate) use placement::validate_placement;

use super::{OS, common::HandleGuard};
use crate::{
    background::*,
    gaming::{GamingError, ProcessIdentity},
};
use std::path::{Path, PathBuf};
use windows::{
    Win32::{
        Foundation::{FILETIME, HANDLE},
        System::{
            SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW},
            Threading::{
                GetCurrentProcessId, GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    },
    core::PWSTR,
};

#[derive(Default)]
pub(crate) struct Resolver {
    trust: trust::TrustCache,
}

pub(super) fn native(operation: &'static str, error: windows::core::Error) -> GamingError {
    let code = error.code().0 as u32 & 0xffff;
    if code == 5 {
        GamingError::Denied { operation, code }
    } else {
        GamingError::Native { operation, code }
    }
}
pub(super) fn unsupported(reason: &'static str) -> GamingError {
    GamingError::Unsupported(reason)
}

fn directory(windows_root: bool) -> Result<PathBuf, GamingError> {
    let mut value = vec![0u16; 32768];
    let length = unsafe {
        if windows_root {
            GetWindowsDirectoryW(Some(&mut value))
        } else {
            GetSystemDirectoryW(Some(&mut value))
        }
    } as usize;
    if length == 0 || length >= value.len() {
        return Err(unsupported("Cannot verify Windows directory"));
    }
    String::from_utf16(&value[..length])
        .map(PathBuf::from)
        .map_err(|_| unsupported("Invalid Windows directory encoding"))
}

fn identity(pid: u32) -> Result<(HandleGuard, ProcessIdentity, u32), GamingError> {
    if pid <= 4 || pid == unsafe { GetCurrentProcessId() } {
        return Err(unsupported("Reserved process"));
    }
    let guard = HandleGuard(
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(|e| native("OpenProcess background identity", e))?,
    );
    let process = identity_on_handle(guard.0, pid)?;
    let facts = OS::tuning_process_identity(pid)
        .map_err(|_| unsupported("Cannot verify process token/session"))?;
    if facts.creation_time != process.creation_token || facts.image_path != process.image_path {
        return Err(GamingError::Changed);
    }
    Ok((guard, process, facts.token.session_id()))
}

pub(super) fn identity_on_handle(handle: HANDLE, pid: u32) -> Result<ProcessIdentity, GamingError> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|e| native("GetProcessTimes background", e))?;
    let token = u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime);
    if token == 0 || exited.dwHighDateTime != 0 || exited.dwLowDateTime != 0 {
        return Err(GamingError::Changed);
    }
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|e| native("QueryFullProcessImageNameW background", e))?;
    if length == 0 || length as usize >= path.len() {
        return Err(unsupported("Invalid background image"));
    }
    let image_path = String::from_utf16(&path[..length as usize])
        .map_err(|_| unsupported("Invalid image encoding"))?;
    Ok(ProcessIdentity {
        pid,
        creation_token: token,
        image_path,
    })
}

impl Resolver {
    pub(crate) fn qualify_target(
        &mut self,
        expected: &ProcessIdentity,
    ) -> Result<Role, GamingError> {
        self.capture_proof(expected).map(|proof| proof.role)
    }
    pub(crate) fn capture_proof(
        &mut self,
        expected: &ProcessIdentity,
    ) -> Result<TargetProof, GamingError> {
        let (handle, actual, _) = identity(expected.pid)?;
        if actual != *expected {
            return Err(GamingError::Changed);
        }
        let scm = services::snapshot()?;
        let name = Path::new(&expected.image_path)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(unsupported("Invalid process image"))?;
        let role = candidate_role(name)
            .or_else(|| {
                scm.iter()
                    .filter(|member| member.pid == expected.pid)
                    .find_map(|member| service_role(&member.name))
            })
            .ok_or(unsupported("Unknown background role"))?;
        let proof = self.qualify(expected, role, &scm)?;
        if identity_on_handle(handle.0, expected.pid)? != *expected {
            return Err(GamingError::Changed);
        }
        Ok(proof)
    }
    fn with_admitted<T>(
        &mut self,
        expected: &ProcessIdentity,
        witness: Option<&TargetProof>,
        operation: impl FnOnce(HANDLE, Role) -> Result<T, GamingError>,
    ) -> Result<T, GamingError> {
        use windows::Win32::{
            System::Threading::IsProcessCritical,
            UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
        };
        use windows::core::BOOL;
        if expected.pid <= 4 || expected.pid == unsafe { GetCurrentProcessId() } {
            return Err(unsupported("Reserved process"));
        }
        if !matches!(
            crate::platform::current_platform(),
            crate::platform::OsPlatform::Windows {
                major: 10,
                build: 22000..,
                workstation: true,
                ..
            }
        ) || !cfg!(target_arch = "x86_64")
        {
            return Err(unsupported(
                "Background CPU control requires qualified Windows 11 x64",
            ));
        }
        super::gaming::with_process(expected, true, |handle| {
            let scm = services::snapshot()?;
            let name = Path::new(&expected.image_path)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(unsupported("Invalid process image"))?;
            let role = candidate_role(name)
                .or_else(|| {
                    scm.iter()
                        .filter(|member| member.pid == expected.pid)
                        .find_map(|member| service_role(&member.name))
                })
                .ok_or(unsupported("Unknown background role"))?;
            if !role.mutation_qualified() {
                return Err(unsupported(role.qualification_note()));
            }
            let mut critical = BOOL::default();
            unsafe { IsProcessCritical(handle, &mut critical) }
                .map_err(|e| native("IsProcessCritical background", e))?;
            let mut foreground = 0;
            unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut foreground)) };
            if critical.as_bool() || foreground == 0 || foreground == expected.pid {
                return Err(unsupported(
                    "Critical, foreground or unverified interactive context",
                ));
            }
            let before = self.qualify(expected, role, &scm)?;
            if witness.is_some_and(|witness| witness != &before || !witness.matches(expected)) {
                return Err(GamingError::Changed);
            }
            let result = operation(handle, role)?;
            if identity_on_handle(handle, expected.pid)? != *expected {
                return Err(GamingError::Changed);
            }
            if self.qualify(expected, role, &services::snapshot()?)? != before {
                return Err(GamingError::Changed);
            }
            Ok(result)
        })
    }

    pub(crate) fn inspect(&mut self) -> Result<Vec<Candidate>, GamingError> {
        let tree = OS::snapshot_process_tree()
            .map_err(|_| unsupported("Process inventory unavailable"))?;
        self.inspect_named(&tree.names.into_iter().collect())
    }
    pub(crate) fn inspect_named(
        &mut self,
        names: &std::collections::BTreeMap<u32, String>,
    ) -> Result<Vec<Candidate>, GamingError> {
        if names.len() > 16384 || names.values().any(|name| name.len() > 1024) {
            return Err(unsupported("Process inventory bound"));
        }
        if !matches!(
            crate::platform::current_platform(),
            crate::platform::OsPlatform::Windows {
                major: 10,
                build: 22000..,
                workstation: true,
                ..
            }
        ) || !cfg!(target_arch = "x86_64")
        {
            return Err(unsupported(
                "Background CPU control requires qualified Windows 11 x64",
            ));
        }
        let scm = services::snapshot()?;
        let mut candidates = std::collections::BTreeMap::new();
        for (&pid, name) in names {
            if let Some(role) = candidate_role(name) {
                candidates.insert(pid, role);
            }
        }
        for member in &scm {
            if let Some(role) = service_role(&member.name) {
                candidates.insert(member.pid, role);
            }
        }
        if candidates.len() > MAX_TARGETS {
            return Err(unsupported("Background candidate capacity exceeded"));
        }
        let mut result = Vec::with_capacity(candidates.len());
        for (pid, role) in candidates {
            let mut row = Candidate {
                role,
                pid,
                process: None,
                session: None,
                verified: false,
                proof: None,
                reason: String::new(),
            };
            let qualified = (|| {
                let (handle, process, session) = identity(pid)?;
                row.process = Some(process.clone());
                row.session = Some(session);
                row.proof = Some(self.qualify(&process, role, &scm)?);
                if identity_on_handle(handle.0, pid)? != process {
                    return Err(GamingError::Changed);
                }
                Ok(())
            })();
            match qualified {
                Ok(()) => {
                    row.verified = true;
                    row.reason =
                        "Image and role verified; native release qualification required".into();
                }
                Err(error) => {
                    row.reason = format!("{error:?}");
                    row.reason.truncate(512);
                }
            }
            result.push(row);
        }
        Ok(result)
    }

    pub(super) fn qualify(
        &mut self,
        process: &ProcessIdentity,
        role: Role,
        scm: &[ServiceMember],
    ) -> Result<TargetProof, GamingError> {
        let path = Path::new(&process.image_path);
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or(unsupported("Invalid image name"))?;
        if excluded_image(name) {
            return Err(unsupported("Critical or interactive process excluded"));
        }
        let windows = directory(true)?;
        let system = directory(false)?;
        let expected = match role {
            Role::Appraiser => system.join("CompatTelRunner.exe"),
            Role::DeviceCensus => system.join("DeviceCensus.exe"),
            Role::DiagnosticsTelemetry => system.join("svchost.exe"),
            Role::SearchIndexer => system.join("SearchIndexer.exe"),
            Role::FrameworkOptimizer => {
                let roots = [
                    windows.join(r"Microsoft.NET\Framework\v4.0.30319\mscorsvw.exe"),
                    windows.join(r"Microsoft.NET\Framework64\v4.0.30319\mscorsvw.exe"),
                ];
                roots
                    .into_iter()
                    .find(|expected| {
                        expected
                            .to_string_lossy()
                            .eq_ignore_ascii_case(&process.image_path)
                    })
                    .ok_or(unsupported("Unqualified installed Framework variant"))?
            }
            Role::SearchProtocol | Role::SearchFilter => {
                return Err(unsupported(
                    "Search helper role requires independent qualification",
                ));
            }
        };
        if !expected
            .to_string_lossy()
            .eq_ignore_ascii_case(&process.image_path)
        {
            return Err(unsupported("Unexpected Windows image path"));
        }
        let image = self.trust.verify(&expected)?;
        let context = if role.service().is_some() {
            let members: Vec<_> = scm
                .iter()
                .filter(|m| m.pid == process.pid)
                .cloned()
                .collect();
            if !single_service_role(role, process.pid, &members) {
                return Err(unsupported("Shared, pending or changed service host"));
            }
            services::verify(role, process, &mut self.trust)?
        } else {
            roles::verify(role, process, &mut self.trust, scm)?
        };
        let facts = OS::tuning_process_identity(process.pid)
            .map_err(|_| unsupported("Cannot verify background token"))?;
        if facts.creation_time() != process.creation_token
            || facts.image_path() != process.image_path
        {
            return Err(GamingError::Changed);
        }
        let os_build = match crate::platform::current_platform() {
            crate::platform::OsPlatform::Windows {
                major: 10,
                build,
                workstation: true,
                ..
            } if build >= 22000 => build,
            _ => return Err(unsupported("Background process policy requires Windows 11")),
        };
        let proof = TargetProof {
            catalog_version: CATALOG_VERSION,
            role,
            pid: process.pid,
            creation_token: process.creation_token,
            image_path: process.image_path.clone(),
            sid: facts.token().sid().into(),
            session: facts.token().session_id(),
            logon: facts.token().authentication_id(),
            os_build,
            image,
            context,
        };
        if !proof.valid() {
            return Err(unsupported("Invalid background native context"));
        }
        Ok(proof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_native_admission_never_reaches_a_setter_for_self_or_reserved_pids() {
        for pid in [0, 4, std::process::id()] {
            let process = ProcessIdentity {
                pid,
                creation_token: 1,
                image_path: "not-authority.exe".into(),
            };
            let mut called = false;
            assert!(
                Resolver::default()
                    .with_admitted(&process, None, |_, _| {
                        called = true;
                        Ok(())
                    })
                    .is_err()
            );
            assert!(!called);
        }
    }

    #[test]
    fn background_native_api_cannot_set_an_arbitrary_priority_class() {
        let process = ProcessIdentity {
            pid: 123456,
            creation_token: 1,
            image_path: "not-authority.exe".into(),
        };
        for desired in [0x40, 0x80, 0x100, 0x8000, 0, 0xdead] {
            assert_eq!(
                Resolver::default().write_priority(&process, 0x20, desired),
                Err(GamingError::InvalidValue)
            );
        }
    }
}
