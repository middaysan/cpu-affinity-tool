use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledAppCatalogTarget {
    Aumid(String),
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledAppCatalogEntry {
    pub name: String,
    pub target: InstalledAppCatalogTarget,
}
