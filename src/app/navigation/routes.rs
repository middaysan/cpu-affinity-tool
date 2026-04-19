#[derive(Clone)]
pub enum GroupRoute {
    List,
    Create,
    Edit,
}

#[derive(Clone)]
pub enum WindowRoute {
    Groups(GroupRoute),
    Logs,
    AppRunSettings,
    InstalledAppPicker,
}

impl Default for WindowRoute {
    fn default() -> Self {
        Self::Groups(GroupRoute::List)
    }
}
