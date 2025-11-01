use std::path::PathBuf;

use os_api::PriorityClass;
use serde::{Deserialize, Serialize};

/// State for editing an application that will be run with a specific CPU affinity.
/// This structure is used to track the current application being edited and its run settings.
pub struct RunAppEditState {
    /// The application currently being edited, if any
    pub current_edit: Option<AppToRun>,
    /// Optional run settings as a tuple of (group_index, program_index)
    pub run_settings: Option<(usize, usize)>,
}

/// Represents an application that can be run with a specific CPU affinity.
/// This structure contains all the information needed to launch an application
/// with the desired settings.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppToRun {
    /// Display name of the application
    pub name: String,
    /// Original path from which the application was dropped (for UI display)
    pub dropped_path: PathBuf,
    /// Command-line arguments to pass to the application
    pub args: Vec<String>,
    /// Actual path to the binary executable
    pub bin_path: PathBuf,
    /// Execution location of the executable
    pub working_dir: PathBuf,
    /// Whether the application should from a user specified directory.
    pub custom_working_dir: bool,
    /// Whether the application should start automatically on application startup
    pub autorun: bool,
    /// Process priority class to assign to the application
    pub priority: PriorityClass,
}

impl AppToRun {
    /// Creates a new instance of `AppToRun` with the specified parameters.
    ///
    /// Extracts the application name from the dropped path's filename,
    /// removing the file extension.
    ///
    /// # Parameters
    ///
    /// * `dropped_path` - The path from which the application was dropped
    /// * `args` - Command-line arguments to pass to the application
    /// * `bin_path` - Path to the binary executable
    /// * `priority` - Process priority class to assign to the application
    /// * `autorun` - Whether the application should start automatically on application startup
    ///
    /// # Returns
    ///
    /// A new `AppToRun` instance with the specified parameters and extracted name
    pub fn new(
        dropped_path: PathBuf,
        args: Vec<String>,
        bin_path: PathBuf,
        working_dir: PathBuf,
        custom_working_dir: bool,
        priority: PriorityClass,
        autorun: bool,
    ) -> Self {
        let name = dropped_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string()
            .rsplit('.')
            .next_back()
            .unwrap()
            .to_string();

        Self {
            name,
            dropped_path,
            args,
            bin_path,
            working_dir,
            custom_working_dir,
            autorun,
            priority,
        }
    }

    /// Returns a formatted string representation of the application.
    ///
    /// The string includes the binary path, arguments, source path, and priority.
    ///
    /// # Returns
    ///
    /// A formatted string representation of the application
    pub fn display(&self) -> String {
        format!(
            "{} {}(src: {}) P({:?})",
            self.bin_path.display(),
            self.args.join(" "),
            self.dropped_path.display(),
            self.priority
        )
    }

    /// Generates a unique key for the application based on its path, arguments, and priority.
    ///
    /// This key can be used to identify the application in collections.
    ///
    /// # Returns
    ///
    /// A string that uniquely identifies the application
    pub fn get_key(&self) -> String {
        format!(
            "{} {} {:?}",
            self.bin_path.display(),
            self.args.join(" "),
            self.priority
        )
    }
}
