use crate::app::controllers;
use crate::app::models::app_state_storage::AppStateStorage;
use crate::app::models::app_to_run::{AppToRun, RunAppEditState};
use crate::app::models::core_group::{CoreGroup, GroupFormState};
use crate::app::models::running_app::RunningApps;
use crate::app::models::LogManager;
use os_api::OS;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use eframe::egui;
use num_cpus;
use std::path::PathBuf;

/// The central state management component of the application.
/// This structure holds all the application states, including persistent data,
/// UI state, and runtime information about running applications.
pub struct AppState {
    /// The current window controller that determines which view is displayed
    pub current_window: controllers::WindowController,
    /// Flag indicating whether the controller has been changed and needs to be updated
    pub controller_changed: bool,
    /// Persistent state that is saved to and loaded from the disk
    pub persistent_state: AppStateStorage,
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
        let app = Self {
            persistent_state: AppStateStorage::load_state(),
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
        };

        // Set the UI theme based on the theme index in the persistent state
        app.apply_theme(ctx);

        // Create a clone of the running apps reference for the background monitor
        let apps_clone = Arc::clone(&app.running_apps);

        // Spawn a background task to monitor running applications
        tokio::spawn(run_running_app_monitor(apps_clone));

        app
    }
}

impl AppState {
    /// Starts all applications marked for automatic startup.
    ///
    /// Iterates through all groups and their programs, and for each program
    /// that has the `autorun` flag set to true, calls `run_app_with_affinity()`
    /// to launch the application with the appropriate CPU affinity.
    ///
    /// This method is typically called during application initialization.
    pub fn start_app_with_autorun(&mut self) {
        let groups = self.persistent_state.groups.clone();
        for (gi, group) in groups.iter().enumerate() {
            for (pi, app) in group.programs.iter().enumerate() {
                if app.autorun {
                    self.run_app_with_affinity(gi, pi, app.clone());
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
    pub fn apply_theme(&self, ctx: &egui::Context) {
        let visuals = match self.persistent_state.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);
    }

    /// Toggles the UI theme between default, light, and dark modes and saves the state.
    ///
    /// # Parameters
    ///
    /// * `ctx` - The egui context to apply the theme to
    pub fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.persistent_state.theme_index = (self.persistent_state.theme_index + 1) % 3;
        self.apply_theme(ctx);
        self.persistent_state.save_state();
    }

    /// Creates a new core group from the group form data.
    /// Validates that group name is non-empty and at least one core is selected.
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
        self.persistent_state.groups.push(CoreGroup {
            name: group_name_trimmed.to_string(),
            cores: selected_cores,
            programs: vec![],
            is_hidden: false,
            run_all_button: self.group_form.run_all_enabled,
        });

        self.reset_group_form();
        self.persistent_state.save_state();
    }

    /// Sets a new window and marks the controller as changed.
    pub fn set_current_window(&mut self, window: controllers::WindowController) {
        self.current_window = window;
        self.controller_changed = true;
    }

    /// Remove an application from a specified group by binary path.
    pub fn remove_app_from_group(&mut self, group_index: usize, programm_index: usize) {
        if let Some(group) = self.persistent_state.groups.get_mut(group_index) {
            if programm_index < group.programs.len() {
                let app = &group.programs[programm_index];
                self.log_manager
                    .add_entry(format!("Removing app: {}", app.bin_path.display()));
                group.programs.remove(programm_index);
            }
        }
    }

    /// Prepares the group form for editing an existing group.
    /// It fills the form with the group data and updates associated clusters.
    pub fn start_editing_group(&mut self, group_index: usize) {
        let total_cores = self.group_form.core_selection.len();
        // Update the core selection based on the selected group's cores.
        self.group_form.core_selection = {
            let mut selection = vec![false; total_cores];
            for &core in &self.persistent_state.groups[group_index].cores {
                if core < total_cores {
                    selection[core] = true;
                }
            }
            selection
        };

        self.group_form.group_name = self.persistent_state.groups[group_index].name.clone();
        self.group_form.editing_index = Some(group_index);
        self.group_form.run_all_enabled = self.persistent_state.groups[group_index].run_all_button;

        // Map the cores to their corresponding clusters.
        // This is a critical operation that ensures UI consistency.
        self.persistent_state.clusters = self.persistent_state.groups[group_index]
            .cores
            .iter()
            .map(|&ci| {
                self.persistent_state
                    .clusters
                    .get(ci)
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();

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
    pub fn run_app_with_affinity(
        &mut self,
        group_index: usize,
        prog_index: usize,
        app_to_run: AppToRun,
    ) {
        let app_key = app_to_run.get_key();

        // Check if app is already running and try to focus its window
        if self.is_app_running(&app_key) {
            let (app_exists, was_focused) =
                self.try_focus_existing_app(&app_key, &app_to_run.display());

            // If app exists and was successfully focused, we're done
            if app_exists && was_focused {
                return;
            }
        }

        // Get the group containing core affinity information
        let group = match self.persistent_state.groups.get(group_index) {
            Some(g) => g,
            None => {
                self.log_manager
                    .add_entry(format!("Error: Group index {group_index} not found"));
                return;
            }
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
    pub fn is_app_running(&mut self, app_key: &str) -> bool {
        // Try to get a read lock on the running apps
        match self.running_apps.try_read() {
            Ok(apps) => {
                // We got the lock, check if the app is running and update the cache
                let is_running = apps.apps.contains_key(app_key);
                if is_running {
                    // Update the cache only if the app is running
                    self.running_apps_statuses.insert(app_key.to_string(), true);
                }
                is_running
            }
            Err(_) => {
                // Couldn't get the lock, fall back to the cached status
                self.running_apps_statuses.contains_key(app_key)
            }
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
