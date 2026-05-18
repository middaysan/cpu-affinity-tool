use crate::app::shared::ids::GroupId;
use os_api::InstalledAppCatalogEntry;
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub struct InstalledAppPickerSession {
    pub target_group_id: Option<GroupId>,
    pub query: String,
    pub entries: Vec<InstalledAppCatalogEntry>,
    pub selected_entry_index: Option<usize>,
    pub is_refreshing: bool,
    pub last_error: Option<String>,
    pub needs_focus: bool,
    pub refresh_rx: Option<Receiver<Result<Vec<InstalledAppCatalogEntry>, String>>>,
}
