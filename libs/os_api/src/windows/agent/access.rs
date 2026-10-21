use super::super::common::to_wide_z_str;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
};
use windows::{
    Win32::{
        Foundation::{HANDLE, HLOCAL, LocalFree},
        Security::{
            Authorization::{
                ConvertSecurityDescriptorToStringSecurityDescriptorW,
                ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
                SE_FILE_OBJECT,
            },
            Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
            DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, LABEL_SECURITY_INFORMATION,
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::{
            CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_GENERIC_WRITE, FILE_SHARE_READ, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            MoveFileExW,
        },
    },
    core::{PCWSTR, PWSTR},
};

const MAX_POLICY: usize = 256 * 1024;
pub fn access_policy_directory() -> Result<std::path::PathBuf, String> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    let value = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) }
        .map_err(|e| format!("Cannot resolve this account's local application data: {e}"))?;
    let text =
        unsafe { value.to_string() }.map_err(|_| "Invalid local application data path".to_string());
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    let path = std::path::PathBuf::from(text?);
    if !path.is_absolute() {
        return Err("Local application data must be an absolute path".into());
    }
    Ok(path.join("CpuAffinityTool").join("mcp"))
}
fn equivalent_descriptor(actual: &str, expected: &str) -> bool {
    // Windows adds the SACL auto-inherited metadata bit at creation. The exact
    // owner, protected DACL, ACEs and mandatory label must still match.
    actual.replace("S:AI(", "S:(") == expected.replace("S:AI(", "S:(")
}
fn descriptor_sddl() -> &'static str {
    "O:BAG:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)S:(ML;;NWNR;;;HI)"
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}
impl Descriptor {
    fn administrator_only() -> Result<Self, String> {
        let wide = to_wide_z_str(descriptor_sddl());
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|e| format!("Cannot construct MCP authority descriptor: {e}"))?;
        Ok(Self(descriptor))
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: false.into(),
        }
    }
    fn canonical(&self) -> Result<String, String> {
        let mut value = PWSTR::null();
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                self.0,
                1,
                OWNER_SECURITY_INFORMATION
                    | GROUP_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | LABEL_SECURITY_INFORMATION,
                &mut value,
                None,
            )
        }
        .map_err(|e| format!("Cannot verify MCP authority descriptor: {e}"))?;
        let text = unsafe { value.to_string() }.map_err(|_| "Invalid MCP security descriptor");
        unsafe {
            let _ = LocalFree(Some(HLOCAL(value.0.cast())));
        }
        text.map_err(String::from)
    }
}

pub fn random_secret() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
        .ok()
        .map_err(|e| format!("Native credential generation failed: {e}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn verify(file: &File, expected: &Descriptor) -> Result<(), String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err("MCP authority cannot be a reparse point".into());
    }
    let mut actual = PSECURITY_DESCRIPTOR::default();
    unsafe {
        GetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | GROUP_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | LABEL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut actual),
        )
    }
    .ok()
    .map_err(|e| format!("Cannot inspect MCP authority security: {e}"))?;
    let actual = Descriptor(actual).canonical()?;
    let expected = expected.canonical()?;
    if !equivalent_descriptor(&actual, &expected) {
        return Err(
            "MCP policy must be administrator-owned and private to elevated readers/writers".into(),
        );
    }
    Ok(())
}
fn open_directory(path: &Path, expected: &Descriptor) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode((FILE_SHARE_READ | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE).0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .map_err(|e| format!("Cannot open MCP authority directory: {e}"))?;
    verify(&file, expected)?;
    if !file.metadata().map_err(|e| e.to_string())?.is_dir() {
        return Err("MCP authority directory is not a directory".into());
    }
    Ok(file)
}
/// The supplied directory is application-owned configuration, never a remote path argument.
pub fn read_access_policy(directory: &Path) -> Result<Option<Vec<u8>>, String> {
    if !directory.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    let expected = Descriptor::administrator_only()?;
    let _directory = open_directory(directory, &expected)?;
    let path = directory.join("access.json");
    let file = match OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot read MCP access policy: {error}")),
    };
    verify(&file, &expected)?;
    let mut bytes = Vec::new();
    file.take((MAX_POLICY + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_POLICY {
        return Err("MCP authority file exceeds its byte limit".into());
    }
    Ok(Some(bytes))
}
pub fn write_access_policy(directory: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_POLICY {
        return Err("MCP access policy exceeds 256 KiB".into());
    }
    let expected = Descriptor::administrator_only()?;
    let parent = directory
        .parent()
        .ok_or("MCP policy needs an application-data parent")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Cannot create MCP application-data directory: {e}"))?;
    if !directory.try_exists().map_err(|e| e.to_string())? {
        let wide = to_wide_z_str(&directory.to_string_lossy());
        unsafe { CreateDirectoryW(PCWSTR(wide.as_ptr()), Some(&expected.attributes())) }.map_err(
            |e| {
                format!(
                    "Cannot create protected MCP directory; administrator rights are required: {e}"
                )
            },
        )?;
    }
    let _directory = open_directory(directory, &expected)?;
    let staged = directory.join(format!("access-{}.tmp", random_secret()?));
    let wide = to_wide_z_str(&staged.to_string_lossy());
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ,
            Some(&expected.attributes()),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|e| format!("Cannot stage protected MCP policy: {e}"))?;
    let mut file = unsafe { File::from_raw_handle(handle.0) };
    let result = (|| {
        verify(&file, &expected)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        let destination = to_wide_z_str(&directory.join("access.json").to_string_lossy());
        unsafe {
            MoveFileExW(
                PCWSTR(wide.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|e| format!("Cannot publish protected MCP policy: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_sacl_metadata_does_not_relax_authority_checks() {
        let expected = descriptor_sddl();
        assert!(equivalent_descriptor(
            &expected.replace("S:(", "S:AI("),
            &expected
        ));
        for changed in [
            expected.replace("D:P", "D:"),
            expected.replace(";;;HI)", ";;;ME)"),
            expected.replace("NWNR", "NW"),
            expected.replace("O:BA", "O:WD"),
        ] {
            assert!(!equivalent_descriptor(&changed, &expected));
        }
    }
    #[test]
    fn grant_store_descriptor_keeps_authority_above_medium_integrity() {
        let text = descriptor_sddl();
        assert!(text.contains("O:BAG:BA"));
        assert!(!text.contains("(A;;FR;;;S-1-5-21-1-2-3-1001)"));
        assert!(text.contains("S:(ML;;NWNR;;;HI)"));
        assert!(!text.contains(";;;WD)"));
    }
    #[test]
    fn credentials_use_native_randomness_and_are_not_reused() {
        let first = random_secret().unwrap();
        let second = random_secret().unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
    #[test]
    #[ignore = "Requires administrator rights; touches only a new private temporary policy directory"]
    fn native_policy_roundtrip_requires_protected_authority() {
        let root = std::env::temp_dir().join(format!(
            "cpu-affinity-mcp-policy-{}",
            random_secret().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        let directory = root.join("mcp");
        assert_eq!(read_access_policy(&directory).unwrap(), None);
        write_access_policy(&directory, b"{\"probe\":1}").unwrap();
        assert_eq!(
            read_access_policy(&directory).unwrap().unwrap(),
            b"{\"probe\":1}"
        );
        write_access_policy(&directory, b"{\"probe\":2}").unwrap();
        assert_eq!(
            read_access_policy(&directory).unwrap().unwrap(),
            b"{\"probe\":2}"
        );
        let mut child = super::super::launch_executable(&super::super::LaunchSpec {
            executable: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "windows::agent::access::tests::medium_cannot_read_or_write_authority".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: None,
            cpus: None,
            priority: crate::PriorityClass::Normal,
            elevated: false,
            capture: true,
            environment: std::collections::BTreeMap::from([(
                "CPU_AFFINITY_POLICY_PROBE".into(),
                directory.join("access.json").to_str().unwrap().into(),
            )]),
        })
        .unwrap();
        child.resume().unwrap();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        for _ in 0..100 {
            if child.exit_code().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(child.exit_code().unwrap(), Some(0), "{output}");
        assert!(output.contains("medium-read-write-denied"), "{output}");
        std::fs::remove_file(directory.join("access.json")).unwrap();
        std::fs::remove_dir(directory).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    #[ignore = "Controlled medium-integrity fixture for the elevated authority test"]
    fn medium_cannot_read_or_write_authority() {
        let Some(path) = std::env::var_os("CPU_AFFINITY_POLICY_PROBE") else {
            return;
        };
        assert!(
            std::fs::read(&path).is_err(),
            "ordinary clients must not read other clients' credentials"
        );
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        let path = std::path::Path::new(&path);
        assert!(
            std::fs::rename(path, path.with_extension("moved")).is_err(),
            "ordinary client must not move authority files"
        );
        let directory = path.parent().unwrap();
        assert!(
            std::fs::rename(directory, directory.with_extension("moved")).is_err(),
            "ordinary client must not replace authority directories"
        );
        println!("medium-read-write-denied");
    }
}
