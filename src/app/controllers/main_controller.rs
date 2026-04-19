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
