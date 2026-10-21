//! Native boundary for the optional local agent API.
mod access;
mod activation;
pub use activation::activate_registered_aumid;
mod identity;
mod launch;
mod process;
mod profile;
pub use process::{ExactProcess, ProcessState, process_instance_exited};
pub use profile::profile_key;
pub fn cancel_output_read(thread: &std::thread::JoinHandle<()>) {
    use std::os::windows::io::AsRawHandle;
    unsafe {
        let _ = windows::Win32::System::IO::CancelSynchronousIo(
            windows::Win32::Foundation::HANDLE(thread.as_raw_handle()),
        );
    }
}
pub use access::{access_policy_directory, random_secret, read_access_policy, write_access_policy};
/// A fresh, exclusively held export spool, removed by Windows when its handle closes.
pub fn create_export_file() -> Result<std::fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_TEMPORARY, FILE_FLAG_DELETE_ON_CLOSE,
    };
    let path = std::env::temp_dir().join(format!("cpu-affinity-export-{}.tmp", random_secret()?));
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(0)
        .custom_flags(FILE_ATTRIBUTE_TEMPORARY.0 | FILE_FLAG_DELETE_ON_CLOSE.0)
        .open(path)
        .map_err(|e| format!("Cannot create private export: {e}"))
}
pub use launch::{LaunchSpec, ManagedProcess, launch_executable};
