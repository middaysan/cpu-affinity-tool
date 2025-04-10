use eframe::egui;

#[derive(Clone)]
pub enum Group {
    ListGroups,
    CreateGroup,
    EditGroup,
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
pub struct MainPanel {
    pub window_controller: WindowController,
}

impl Default for MainPanel {
    fn default() -> Self {
        MainPanel::new()
    }
}

impl MainPanel {
    pub fn new() -> Self {
        MainPanel {
            window_controller: WindowController::Groups(Group::ListGroups),
        }
    }

    pub fn render_with<F>(&self, ctx: &egui::Context, mut dispatch_fn: F)
    where
        F: FnMut(&MainPanel, &egui::Context),
    {
        dispatch_fn(&self, ctx);
    }

    pub fn set_window(&mut self, controller: WindowController) {
        self.window_controller = controller;
    }
}
