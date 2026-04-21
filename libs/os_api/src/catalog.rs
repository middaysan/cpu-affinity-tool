use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstalledAppCatalogSource {
    WindowsAppsFolder,
    WindowsStartMenu,
    WindowsAppPaths,
    LinuxDesktopEntry,
    LinuxPathExecutable,
}

impl InstalledAppCatalogSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::WindowsAppsFolder => "AppsFolder",
            Self::WindowsStartMenu => "Start Menu",
            Self::WindowsAppPaths => "App Paths",
            Self::LinuxDesktopEntry => "Desktop entry",
            Self::LinuxPathExecutable => "PATH executable",
        }
    }

    pub const fn picker_priority(self) -> usize {
        match self {
            Self::WindowsAppsFolder | Self::LinuxDesktopEntry => 0,
            Self::WindowsStartMenu => 1,
            Self::WindowsAppPaths | Self::LinuxPathExecutable => 2,
        }
    }

    pub const fn hide_until_query(self) -> bool {
        matches!(self, Self::LinuxPathExecutable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledAppCatalogTarget {
    Aumid(String),
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledAppCatalogEntry {
    pub name: String,
    pub target: InstalledAppCatalogTarget,
    pub source: InstalledAppCatalogSource,
    pub detail: String,
}

impl InstalledAppCatalogEntry {
    pub fn new_aumid(
        name: impl Into<String>,
        aumid: impl Into<String>,
        source: InstalledAppCatalogSource,
    ) -> Self {
        let aumid = aumid.into();
        Self {
            name: name.into(),
            target: InstalledAppCatalogTarget::Aumid(aumid.clone()),
            source,
            detail: aumid,
        }
    }

    pub fn new_path(
        name: impl Into<String>,
        path: impl Into<PathBuf>,
        source: InstalledAppCatalogSource,
    ) -> Self {
        let path = path.into();
        let detail = path.display().to_string();
        Self {
            name: name.into(),
            target: InstalledAppCatalogTarget::Path(path),
            source,
            detail,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackageRuntimeInfo {
    pub aumid: String,
    pub package_family_name: String,
    pub install_root: PathBuf,
}
