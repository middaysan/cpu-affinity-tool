use eframe::egui;

#[derive(Clone)]
pub enum Group {
    ListGroups,
    Create,
    Edit,
}

#[derive(Clone)]
pub enum WindowController {
    Groups(Group),
    Logs,
    AppRunSettings,
}

impl Default for WindowController {
    fn default() -> Self {
        WindowController::Groups(Group::ListGroups)
    }
}

#[derive(Clone)]
pub struct MainController {
    pub window_controller: WindowController,
}

impl Default for MainController {
    fn default() -> Self {
        MainController::new()
    }
}

impl MainController {
    pub fn new() -> Self {
        MainController {
            window_controller: WindowController::Groups(Group::ListGroups),
        }
    }

    pub fn render_with<F>(&self, ctx: &egui::Context, mut dispatch_fn: F)
    where
        F: FnMut(&MainController, &egui::Context),
    {
        dispatch_fn(self, ctx);
    }

    pub fn set_window(&mut self, controller: WindowController) {
        self.window_controller = controller;
    }
}
