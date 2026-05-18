use crate::app::models::AppToRun;
use os_api::{InstalledAppCatalogEntry, InstalledAppCatalogTarget, PriorityClass, OS};
use std::path::PathBuf;

pub struct DiscoveredApps {
    pub apps: Vec<AppToRun>,
    pub first_error: Option<String>,
}

pub fn apps_from_dropped_paths(dropped_paths: Vec<PathBuf>) -> DiscoveredApps {
    let mut discovered = DiscoveredApps {
        apps: Vec::new(),
        first_error: None,
    };

    for path in dropped_paths {
        match OS::parse_dropped_file(path.clone()) {
            Ok((target, args)) => {
                discovered.apps.push(AppToRun::new_path(
                    path,
                    args,
                    target,
                    PriorityClass::Normal,
                    false,
                ));
            }
            Err(err) => {
                discovered.first_error = Some(err);
                break;
            }
        }
    }

    discovered
}

pub fn app_from_installed_entry(entry: InstalledAppCatalogEntry) -> Result<AppToRun, String> {
    match entry.target {
        InstalledAppCatalogTarget::Aumid(aumid) => Ok(AppToRun::new_installed(
            entry.name,
            aumid,
            PriorityClass::Normal,
            false,
        )),
        InstalledAppCatalogTarget::Path(path) => {
            let (target, args) = OS::parse_dropped_file(path.clone())?;
            let mut app = AppToRun::new_path(path, args, target, PriorityClass::Normal, false);
            app.name = entry.name;
            Ok(app)
        }
    }
}

pub fn list_supported_start_apps() -> Result<Vec<InstalledAppCatalogEntry>, String> {
    OS::list_supported_start_apps()
}

pub fn supports_installed_app_picker() -> bool {
    OS::supports_installed_app_picker()
}
