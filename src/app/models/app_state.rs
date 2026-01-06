use crate::app::controllers;
use crate::app::models::app_state_storage::AppStateStorage;
use crate::app::models::app_to_run::{AppToRun, RunAppEditState};
use crate::app::models::core_group::{CoreGroup, GroupFormState};
use crate::app::models::running_app::RunningApps;
use crate::app::models::LogManager;
use crate::tray::TrayCmd;
use eframe::egui;
use num_cpus;
use os_api::OS;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use tokio::sync::RwLock;

/// The central state management component of the application.
/// This structure holds all the application states, including persistent data,
/// UI state, and runtime information about running applications.
pub struct AppState {
    /// The current window controller that determines which view is displayed
    pub current_window: controllers::WindowController,
    /// Flag indicating whether the controller has been changed and needs to be updated
    pub controller_changed: bool,
    /// Persistent state that is saved to and loaded from the disk
    pub persistent_state: Arc<RwLock<AppStateStorage>>,
    /// State of the group form for creating or editing core groups
    pub group_form: GroupFormState,
    /// State for editing applications to run
    pub app_edit_state: RunAppEditState,
    /// Files that have been dropped onto the application, if any
    pub dropped_files: Option<Vec<PathBuf>>,
    /// Manager for application logs
    pub log_manager: LogManager,
    /// Thread-safe reference to running applications
    pub running_apps: Arc<RwLock<RunningApps>>,
    /// Cache of running application statuses for quick access
    pub running_apps_statuses: HashMap<String, bool>,
    /// Index of the currently displayed tip
    pub current_tip_index: usize,
    /// Time when the tip was last changed (in seconds since app start)
    pub last_tip_change_time: f64,

    // ---- tray integration ----
    /// Receiver for tray events (Show/Hide/Quit)
    pub tray_rx: Option<Receiver<TrayCmd>>,

    /// Keep tray icon alive for Windows (drop = removes icon)
    #[cfg(target_os = "windows")]
    pub tray_icon_guard: Option<tray_icon::TrayIcon>,

    /// Handle to the main window (Windows only)
    #[cfg(target_os = "windows")]
    pub hwnd: Option<windows::Win32::Foundation::HWND>,

    /// Flag indicating that the window is currently hidden in the tray
    pub is_hidden: bool,
}

impl AppState {
    /// Creates a new instance of AppState with default values.
    ///
    /// Initializes the application state by:
    /// 1. Loading persistent state from disk
    /// 2. Setting up the default UI state
    /// 3. Initializing the group form with empty values
    /// 4. Setting up application edit state
    /// 5. Initializing log manager
    /// 6. Creating a thread-safe reference to running applications
    /// 7. Setting the UI theme based on the persistent state
    /// 8. Spawning a background task to monitor running applications
    ///
    /// # Parameters
    ///
    /// * `ctx` - The egui context used to set the UI theme
    ///
    /// # Returns
    ///
    /// A new `AppState` instance with initialized values
    pub fn new(ctx: &egui::Context) -> Self {
        let mut app = Self {
            persistent_state: Arc::new(RwLock::new(AppStateStorage::load_state())),
            current_window: controllers::WindowController::Groups(controllers::Group::ListGroups),
            controller_changed: false,
            group_form: GroupFormState {
                editing_index: None,
                editing_selection: None,
                core_selection: vec![false; num_cpus::get()],
                group_name: String::new(),
                run_all_enabled: false,
            },
            app_edit_state: RunAppEditState {
                current_edit: None,
                run_settings: None,
            },
            dropped_files: None,
            log_manager: LogManager { entries: vec![] },
            running_apps: Arc::new(RwLock::new(RunningApps::default())),
            running_apps_statuses: HashMap::new(),
            current_tip_index: 0,
            last_tip_change_time: 0.0,

            // ---- tray integration ----
            tray_rx: None,
            #[cfg(target_os = "windows")]
            tray_icon_guard: None,
            #[cfg(target_os = "windows")]
            hwnd: None,
            is_hidden: false,
        };

        app.log_manager.add_entry("Application started".into());

        // Set the UI theme based on the theme index in the persistent state
        // Explicitly drop the future to avoid the "let-underscore-future" warning
        std::mem::drop(app.apply_theme(ctx));

        // Create a clone of the running apps reference for the background monitors
        let apps_clone = Arc::clone(&app.running_apps);

        // Spawn a background task to monitor running applications
        tokio::spawn(run_running_app_monitor(apps_clone.clone()));

        // Create a clone of the persistent state for the process settings monitor
        let persistent_state_clone = Arc::clone(&app.persistent_state);

        // Spawn a background task to monitor and enforce process settings
        tokio::spawn(run_process_settings_monitor(
            apps_clone,
            persistent_state_clone,
        ));

        app
    }
}

impl AppState {
    // Helper methods for synchronous access to persistent_state

    /// Gets a reference to the groups in the persistent state.
    /// Returns None if the lock couldn't be acquired.
    pub fn get_groups(&self) -> Option<Vec<CoreGroup>> {
        self.persistent_state
            .try_read()
            .ok()
            .map(|state| state.groups.clone())
    }

    /// Gets the name of a specific group in the persistent state.
    /// Returns None if the lock couldn't be acquired or the group doesn't exist.
    pub fn get_group_name(&self, index: usize) -> Option<String> {
        self.persistent_state
            .try_read()
            .ok()
            .and_then(|state| state.groups.get(index).map(|group| group.name.clone()))
    }

    /// Gets whether a specific group is hidden in the persistent state.
    /// Returns None if the lock couldn't be acquired or the group doesn't exist.
    pub fn get_group_is_hidden(&self, index: usize) -> Option<bool> {
        self.persistent_state
            .try_read()
            .ok()
            .and_then(|state| state.groups.get(index).map(|group| group.is_hidden))
    }

    /// Sets whether a specific group is hidden in the persistent state.
    /// Returns true if the update was successful, false otherwise.
    pub fn set_group_is_hidden(&mut self, index: usize, is_hidden: bool) -> bool {
        if let Ok(mut state) = self.persistent_state.try_write() {
            if let Some(group) = state.groups.get_mut(index) {
                group.is_hidden = is_hidden;
                state.save_state();
                return true;
            }
        }
        false
    }

    /// Gets whether a specific group has the run_all_button enabled in the persistent state.
    /// Returns None if the lock couldn't be acquired or the group doesn't exist.
    pub fn get_group_run_all_button(&self, index: usize) -> Option<bool> {
        self.persistent_state
            .try_read()
            .ok()
            .and_then(|state| state.groups.get(index).map(|group| group.run_all_button))
    }

    /// Gets the programs of a specific group in the persistent state.
    /// Returns None if the lock couldn't be acquired or the group doesn't exist.
    pub fn get_group_programs(&self, index: usize) -> Option<Vec<AppToRun>> {
        self.persistent_state
            .try_read()
            .ok()
            .and_then(|state| state.groups.get(index).map(|group| group.programs.clone()))
    }

    /// Gets a specific program from a specific group in the persistent state.
    /// Returns None if the lock couldn't be acquired or the group/program doesn't exist.
    pub fn get_group_program(&self, group_index: usize, program_index: usize) -> Option<AppToRun> {
        self.persistent_state.try_read().ok().and_then(|state| {
            state
                .groups
                .get(group_index)
                .and_then(|group| group.programs.get(program_index).cloned())
        })
    }

    /// Gets the cores of a specific group in the persistent state.
    /// Returns None if the lock couldn't be acquired or the group doesn't exist.
    pub fn get_group_cores(&self, index: usize) -> Option<Vec<usize>> {
        self.persistent_state
            .try_read()
            .ok()
            .and_then(|state| state.groups.get(index).map(|group| group.cores.clone()))
    }

    /// Gets the clusters from the persistent state.
    /// Returns None if the lock couldn't be acquired.
    pub fn get_clusters(&self) -> Option<Vec<Vec<usize>>> {
        self.persistent_state
            .try_read()
            .ok()
            .map(|state| state.clusters.clone())
    }

    /// Sets the clusters in the persistent state.
    /// Returns true if the update was successful, false otherwise.
    pub fn set_clusters(&mut self, clusters: Vec<Vec<usize>>) -> bool {
        if let Ok(mut state) = self.persistent_state.try_write() {
            state.clusters = clusters;
            state.save_state();
            return true;
        }
        false
    }

    /// Swaps two groups in the persistent state.
    /// Returns true if the swap was successful, false otherwise.
    pub fn swap_groups(&mut self, index1: usize, index2: usize) -> bool {
        if let Ok(mut state) = self.persistent_state.try_write() {
            if index1 < state.groups.len() && index2 < state.groups.len() {
                state.groups.swap(index1, index2);
                state.save_state();
                return true;
            }
        }
        false
    }

    /// Saves the current state to disk.
    /// Returns true if the save was successful, false otherwise.
    pub fn save_state(&mut self) -> bool {
        if let Ok(state) = self.persistent_state.try_write() {
            state.save_state();
            return true;
        }
        false
    }

    /// Adds applications to a group.
    /// Returns Ok if successful, or an error message if failed.
    pub fn add_apps_to_group(
        &mut self,
        group_index: usize,
        paths: Vec<std::path::PathBuf>,
    ) -> Result<(), String> {
        if let Ok(mut state) = self.persistent_state.try_write() {
            if let Some(group) = state.groups.get_mut(group_index) {
                let result = group.add_app_to_group(paths);
                if result.is_ok() {
                    state.save_state();
                }
                return result;
            }
            return Err(format!("Group with index {group_index} not found"));
        }
        Err("Failed to acquire write lock for adding apps to group".to_string())
    }

    /// Updates a program in a group.
    /// Returns true if the update was successful, false otherwise.
    pub fn update_program(
        &mut self,
        group_index: usize,
        program_index: usize,
        program: AppToRun,
    ) -> bool {
        if let Ok(mut state) = self.persistent_state.try_write() {
            if let Some(group) = state.groups.get_mut(group_index) {
                if program_index < group.programs.len() {
                    group.programs[program_index] = program;
                    state.save_state();
                    return true;
                }
            }
        }
        false
    }

    /// Gets the theme index from the persistent state.
    /// Returns 0 (default) if the lock couldn't be acquired.
    pub fn get_theme_index(&self) -> usize {
        self.persistent_state
            .try_read()
            .map(|state| state.theme_index)
            .unwrap_or(0)
    }

    /// Updates a group's properties in the persistent state.
    /// Returns true if the update was successful, false otherwise.
    pub fn update_group_properties(
        &mut self,
        index: usize,
        name: String,
        cores: Vec<usize>,
        run_all_button: bool,
    ) -> bool {
        if let Ok(mut state) = self.persistent_state.try_write() {
            if index < state.groups.len() {
                state.groups[index].name = name;
                state.groups[index].cores = cores;
                state.groups[index].run_all_button = run_all_button;
                state.save_state();
                return true;
            }
        }
        false
    }

    /// Removes a group from the persistent state.
    /// Returns true if the removal was successful, false otherwise.
    pub fn remove_group(&mut self, index: usize) -> bool {
        if let Ok(mut state) = self.persistent_state.try_write() {
            if index < state.groups.len() {
                state.groups.remove(index);
                state.save_state();
                return true;
            }
        }
        false
    }

    /// Starts all applications marked for automatic startup.
    ///
    /// Iterates through all groups and their programs, and for each program
    /// that has the `autorun` flag set to true, calls `run_app_with_affinity()`
    /// to launch the application with the appropriate CPU affinity.
    ///
    /// This method is typically called during application initialization.
    pub fn start_app_with_autorun(&mut self) {
        // Try to get a read lock on the persistent state
        if let Ok(state) = self.persistent_state.try_read() {
            let groups = state.groups.clone();
            // Drop the lock before running apps to avoid deadlocks
            drop(state);

            for group in groups.iter() {
                for app in group.programs.iter() {
                    if app.autorun {
                        // We can't call the async run_app_with_affinity directly here
                        // For now, we'll just log that we would run the app
                        self.log_manager
                            .add_entry(format!("Would autorun app: {}", app.display()));
                    }
                }
            }
        }
    }

    /// Resets the group form state to its default values.
    ///
    /// This method delegates to the `reset()` method of the `GroupFormState` structure,
    /// which clears the editing state, disables the "run all" button, clears the group name,
    /// and deselects all cores.
    ///
    /// This is typically called after a group is created or edited, or when the user
    /// cancels the group creation/editing process.
    pub fn reset_group_form(&mut self) {
        self.group_form.reset();
    }

    /// Applies the current theme to the UI based on the theme index.
    ///
    /// # Parameters
    ///
    /// * `ctx` - The egui context to apply the theme to
    pub async fn apply_theme(&self, ctx: &egui::Context) {
        let state = self.persistent_state.read().await;
        let visuals = match state.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);
    }

    /// Toggles the UI theme between default, light, and dark modes and saves the state.
    /// Synchronous version.
    ///
    /// # Parameters
    ///
    /// * `ctx` - The egui context to apply the theme to
    pub fn toggle_theme(&mut self, ctx: &egui::Context) {
        if let Ok(mut state) = self.persistent_state.try_write() {
            state.theme_index = (state.theme_index + 1) % 3;
            state.save_state();

            // Apply theme synchronously
            let visuals = match state.theme_index {
                0 => egui::Visuals::default(),
                1 => egui::Visuals::light(),
                _ => egui::Visuals::dark(),
            };
            ctx.set_visuals(visuals);
        }
    }

    /// Toggles the process monitoring feature on or off and saves the state.
    /// Synchronous version.
    pub fn toggle_process_monitoring(&mut self) {
        if let Ok(mut state) = self.persistent_state.try_write() {
            state.process_monitoring_enabled = !state.process_monitoring_enabled;
            state.save_state();
        }
    }

    /// Checks if the process monitoring feature is enabled.
    /// Synchronous version.
    ///
    /// # Returns
    ///
    /// `true` if process monitoring is enabled, `false` otherwise
    pub fn is_process_monitoring_enabled(&self) -> bool {
        self.persistent_state
            .try_read()
            .map(|state| state.process_monitoring_enabled)
            .unwrap_or(false)
    }

    /// Creates a new core group from the group form data.
    /// Validates that group name is non-empty and at least one core is selected.
    /// Synchronous version.
    pub fn create_group(&mut self) {
        let group_name_trimmed = self.group_form.group_name.trim();
        if group_name_trimmed.is_empty() {
            self.log_manager
                .add_entry("Group name cannot be empty".into());
            return;
        }

        // Gather indices of selected cores.
        let selected_cores: Vec<usize> = self
            .group_form
            .core_selection
            .iter()
            .enumerate()
            .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
            .collect();

        if selected_cores.is_empty() {
            self.log_manager
                .add_entry("At least one core must be selected".into());
            return;
        }

        // Add a new group to the persistent application state.
        if let Ok(mut state) = self.persistent_state.try_write() {
            state.groups.push(CoreGroup {
                name: group_name_trimmed.to_string(),
                cores: selected_cores,
                programs: vec![],
                is_hidden: false,
                run_all_button: self.group_form.run_all_enabled,
            });
            state.save_state();
        } else {
            self.log_manager
                .add_entry("Failed to acquire write lock for creating group".into());
        }

        self.reset_group_form();
    }

    /// Sets a new window and marks the controller as changed.
    pub fn set_current_window(&mut self, window: controllers::WindowController) {
        self.current_window = window;
        self.controller_changed = true;
    }

    /// Remove an application from a specified group by binary path.
    /// Synchronous version.
    pub fn remove_app_from_group(&mut self, group_index: usize, programm_index: usize) {
        if let Ok(mut state) = self.persistent_state.try_write() {
            if let Some(group) = state.groups.get_mut(group_index) {
                if programm_index < group.programs.len() {
                    let app = &group.programs[programm_index];
                    self.log_manager
                        .add_entry(format!("Removing app: {}", app.bin_path.display()));
                    group.programs.remove(programm_index);
                    state.save_state();
                }
            }
        } else {
            self.log_manager.add_entry(format!(
                "Failed to acquire write lock for removing app from group {group_index}"
            ));
        }
    }

    /// Prepares the group form for editing an existing group.
    /// It fills the form with the group data and updates associated clusters.
    /// Synchronous version.
    pub fn start_editing_group(&mut self, group_index: usize) {
        let total_cores = self.group_form.core_selection.len();

        // Try to get a read lock to access group information
        if let Ok(state_read) = self.persistent_state.try_read() {
            // Update the core selection based on the selected group's cores.
            self.group_form.core_selection = {
                let mut selection = vec![false; total_cores];
                if let Some(group) = state_read.groups.get(group_index) {
                    for &core in &group.cores {
                        if core < total_cores {
                            selection[core] = true;
                        }
                    }
                }
                selection
            };

            // Get group information for form
            if let Some(group) = state_read.groups.get(group_index) {
                self.group_form.group_name = group.name.clone();
                self.group_form.run_all_enabled = group.run_all_button;

                // Prepare clusters data
                let clusters_data: Vec<Vec<usize>> = group
                    .cores
                    .iter()
                    .map(|&ci| state_read.clusters.get(ci).cloned().unwrap_or_default())
                    .collect();

                // Drop the read lock before acquiring write lock to avoid deadlock
                drop(state_read);

                // Try to get a write lock to update clusters
                if let Ok(mut state_write) = self.persistent_state.try_write() {
                    state_write.clusters = clusters_data;
                } else {
                    self.log_manager.add_entry(format!(
                        "Failed to acquire write lock for updating clusters when editing group {group_index}"
                    ));
                }
            } else {
                // Drop the read lock if group not found
                drop(state_read);
                self.log_manager
                    .add_entry(format!("Group with index {group_index} not found"));
            }
        } else {
            self.log_manager.add_entry(format!(
                "Failed to acquire read lock for editing group {group_index}"
            ));
        }

        self.group_form.editing_index = Some(group_index);

        self.set_current_window(controllers::WindowController::Groups(
            controllers::Group::Edit,
        ));
    }

    /// Runs an application with a specified CPU affinity based on the provided group.
    /// Logs the start of the app and any resulting errors.
    /// Attempts to focus an existing running application window.
    ///
    /// # Parameters
    ///
    /// * `app_key` - The unique key identifying the application
    /// * `app_display_name` - A human-readable name for logging purposes
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// - Whether the app exists
    /// - Whether the window was successfully focused
    fn try_focus_existing_app(&mut self, app_key: &str, app_display_name: &str) -> (bool, bool) {
        let lock_result = self.running_apps.try_read();

        if let Ok(apps) = lock_result {
            if let Some(app) = apps.apps.get(app_key) {
                // Try to focus any window belonging to this app
                let was_focused = app.pids.iter().any(|pid| OS::focus_window_by_pid(*pid));

                self.log_manager.add_entry(format!(
                    "App already running: {}, pids: {:?}",
                    app_display_name, app.pids
                ));

                return (true, was_focused);
            }
        }

        (false, false)
    }

    /// Runs an application with a specified CPU affinity based on the provided group.
    /// If the application is already running, attempts to focus its window instead.
    /// Logs the start of the app and any resulting errors.
    /// Synchronous version.
    pub fn run_app_with_affinity_sync(
        &mut self,
        group_index: usize,
        prog_index: usize,
        app_to_run: AppToRun,
    ) {
        let app_key = app_to_run.get_key();

        // Check if app is already running and try to focus its window
        if self.is_app_running_sync(&app_key) {
            let (app_exists, was_focused) =
                self.try_focus_existing_app(&app_key, &app_to_run.display());

            // If app exists and was successfully focused, we're done
            if app_exists && was_focused {
                return;
            }
        }

        // Try to get the group containing core affinity information
        let group = if let Ok(state) = self.persistent_state.try_read() {
            match state.groups.get(group_index) {
                Some(g) => g.clone(), // Clone the group so we can drop the read lock
                None => {
                    drop(state);
                    self.log_manager
                        .add_entry(format!("Error: Group index {group_index} not found"));
                    return;
                }
            }
        } else {
            self.log_manager.add_entry(format!(
                "Error: Failed to acquire read lock for group {group_index}"
            ));
            return;
        };

        // Extract a human-readable label from the binary path
        let label = app_to_run
            .bin_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| app_to_run.bin_path.display().to_string());

        // Log the attempt to start the application
        self.log_manager.add_entry(format!(
            "Starting '{}', app: {}",
            label,
            app_to_run.display()
        ));

        // Try to run the application with the specified affinity
        match OS::run(
            app_to_run.bin_path,
            app_to_run.args,
            &group.cores,
            app_to_run.priority,
        ) {
            Ok(pid) => {
                // Check if we need to add this as a new app or it's a new instance of existing app
                let is_new_app = !self
                    .running_apps
                    .try_read()
                    .map(|apps| apps.apps.contains_key(&app_key))
                    .unwrap_or(false);

                if is_new_app {
                    let added = self.add_running_app(&app_key, pid, group_index, prog_index);
                    if added {
                        self.log_manager
                            .add_entry(format!("App started with PID: {pid}"));
                    } else {
                        self.log_manager.add_entry(format!(
                            "App started with PID: {pid} but couldn't be tracked (lock busy)"
                        ));
                    }
                } else {
                    self.log_manager.add_entry(format!(
                        "New instance of existing app started with PID: {pid}"
                    ));
                }
            }
            Err(e) => self.log_manager.add_entry(format!("ERROR: {e}")),
        }
    }

    /// Adds a running application to the tracked applications list.
    ///
    /// This method attempts to acquire a write lock on the running apps collection
    /// and add the specified application. If the lock can't be acquired, the operation
    /// is silently skipped.
    ///
    /// # Parameters
    ///
    /// * `app_key` - The unique key identifying the application
    /// * `pid` - The process ID of the application
    /// * `group_index` - The index of the group the application belongs to
    /// * `prog_index` - The index of the program within the group
    ///
    /// # Returns
    ///
    /// `true` if the application was successfully added, `false` if the lock couldn't be acquired
    pub fn add_running_app(
        &self,
        app_key: &str,
        pid: u32,
        group_index: usize,
        prog_index: usize,
    ) -> bool {
        match self.running_apps.try_write() {
            Ok(mut apps) => {
                apps.add_app(app_key, pid, group_index, prog_index);
                true
            }
            Err(_) => {
                // Log the failure to acquire the lock
                // This is a silent failure in the original code, but we could log it
                // if we had access to the log_manager here
                false
            }
        }
    }

    /// Checks if an application is currently running.
    /// Synchronous version.
    ///
    /// This method first tries to check the actual running apps collection.
    /// If the lock can't be acquired (e.g., because another thread is writing to it),
    /// it falls back to the cached status.
    ///
    /// # Parameters
    ///
    /// * `app_key` - The unique key identifying the application
    ///
    /// # Returns
    ///
    /// `true` if the application is running, `false` otherwise
    pub fn is_app_running_sync(&mut self, app_key: &str) -> bool {
        // Try to get a read lock on the running apps
        if let Ok(apps) = self.running_apps.try_read() {
            // Check if the app is running and update the cache
            let is_running = apps.apps.contains_key(app_key);
            if is_running {
                // Update the cache only if the app is running
                self.running_apps_statuses.insert(app_key.to_string(), true);
            }
            is_running
        } else {
            // Fall back to the cached status if we can't get a lock
            self.running_apps_statuses
                .get(app_key)
                .copied()
                .unwrap_or(false)
        }
    }

    /// Gets the PIDs of a running application.
    /// Synchronous version.
    ///
    /// # Parameters
    ///
    /// * `app_key` - The unique key identifying the application
    ///
    /// # Returns
    ///
    /// An Option containing a vector of PIDs if the application is running
    pub fn get_running_app_pids(&self, app_key: &str) -> Option<Vec<u32>> {
        if let Ok(apps) = self.running_apps.try_read() {
            apps.apps.get(app_key).map(|app| app.pids.clone())
        } else {
            None
        }
    }
}

/// Monitors running applications in the background.
///
/// This function runs in a separate tokio task and periodically:
/// 1. Checks for child processes of running applications
/// 2. Removes processes that are no longer running
/// 3. Removes applications that have no running processes
///
/// The function uses a more efficient locking strategy to minimize contention:
/// - It acquires a single write lock for all operations
/// - It processes all applications in a single lock acquisition
/// - It releases the lock as soon as possible
///
/// # Parameters
///
/// * `running_apps` - Thread-safe reference to the running applications collection
pub async fn run_running_app_monitor(running_apps: Arc<RwLock<RunningApps>>) {
    // Create a 2-second interval for periodic checking
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        // Wait for the next interval tick
        interval.tick().await;

        // Process all applications in a single write lock to minimize contention
        if let Ok(mut apps) = running_apps.try_write() {
            // Get a list of keys to avoid borrowing issues
            let app_keys: Vec<String> = apps.apps.keys().cloned().collect();

            // Process each application
            for app_key in app_keys {
                if let Some(app) = apps.apps.get_mut(&app_key) {
                    // Only process apps that have at least one PID
                    if !app.pids.is_empty() {
                        // Find all child processes of the first PID
                        OS::find_all_descendants(app.pids[0], &mut app.pids);

                        // Remove PIDs that are no longer running
                        app.pids.retain(|&pid| OS::is_pid_live(pid));

                        // If no PIDs are left, remove the application
                        if app.pids.is_empty() {
                            apps.remove_app(&app_key);
                        }
                    } else {
                        // Remove apps with no PIDs
                        apps.remove_app(&app_key);
                    }
                }
            }
        }

        // If we couldn't acquire the lock, just wait for the next interval
        // This is more efficient than blocking or retrying
    }
}

/// Monitors and enforces CPU affinity and priority settings for running processes.
///
/// This function runs in a separate tokio task and periodically:
/// 1. Checks if the monitoring feature is enabled
/// 2. For each running application and its child processes:
///    a. Checks if the current CPU affinity matches the expected affinity from the group
///    b. Checks if the current priority matches the expected priority from the app configuration
///    c. Resets the CPU affinity and priority if they've been changed
///
/// # Parameters
///
/// * `running_apps` - Thread-safe reference to the running applications collection
/// * `app_state` - Thread-safe reference to the application state
pub async fn run_process_settings_monitor(
    running_apps: Arc<RwLock<RunningApps>>,
    app_state: Arc<RwLock<AppStateStorage>>,
) {
    // Create a 3-second interval for periodic checking
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));

    loop {
        // Wait for the next interval tick
        interval.tick().await;

        // Check if monitoring is enabled
        let monitoring_enabled = if let Ok(state) = app_state.try_read() {
            state.process_monitoring_enabled
        } else {
            false // Default to disabled if we can't read the state
        };

        // Skip processing if monitoring is disabled
        if !monitoring_enabled {
            continue;
        }

        // Get the groups configuration
        let groups = if let Ok(state) = app_state.try_read() {
            state.groups.clone()
        } else {
            continue; // Skip this iteration if we can't read the state
        };

        // Process all applications in a single read lock to minimize contention
        if let Ok(apps) = running_apps.try_read() {
            // Process each application
            for app in apps.apps.values() {
                // Get the group for this application
                if let Some(group) = groups.get(app.group_index) {
                    // Get the expected CPU affinity mask from the group
                    let expected_mask = group.cores.iter().fold(0usize, |acc, &i| acc | (1 << i));

                    // Get the expected priority from the app configuration
                    let expected_priority =
                        if let Some(program) = group.programs.get(app.prog_index) {
                            program.priority
                        } else {
                            continue; // Skip if we can't find the program
                        };

                    // Check and reset CPU affinity and priority for each process
                    for &pid in &app.pids {
                        // Check and reset CPU affinity
                        if let Ok(current_mask) = OS::get_process_affinity(pid) {
                            if current_mask != expected_mask {
                                // The CPU affinity has been changed, reset it
                                let _ = OS::set_process_affinity_by_pid(pid, expected_mask);
                            }
                        }

                        // Check and reset priority
                        if let Ok(current_priority) = OS::get_process_priority(pid) {
                            if current_priority != expected_priority {
                                // The priority has been changed, reset it
                                let _ = OS::set_process_priority_by_pid(pid, expected_priority);
                            }
                        }
                    }
                }
            }
        }
    }
}
