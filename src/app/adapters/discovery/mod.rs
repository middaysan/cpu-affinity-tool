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

#[cfg(test)]
mod tests {
    use super::*;
    use os_api::{InstalledAppCatalogSource, InstalledAppCatalogTarget};

    #[test]
    fn test_installed_aumid_entry_converts_to_installed_app() {
        let app = app_from_installed_entry(InstalledAppCatalogEntry::new_aumid(
            "Spotify",
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            InstalledAppCatalogSource::WindowsAppsFolder,
        ))
        .unwrap();

        assert!(app.is_installed_target());
        assert_eq!(app.name, "Spotify");
        assert_eq!(
            app.installed_aumid(),
            Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify")
        );
        assert!(app.additional_processes.is_empty());
    }

    #[test]
    fn test_installed_path_entry_keeps_catalog_name_and_tracks_primary_process() {
        let app = app_from_installed_entry(InstalledAppCatalogEntry::new_path(
            "Catalog Name",
            r"C:\Tools\Sample.exe",
            InstalledAppCatalogSource::WindowsAppPaths,
        ))
        .unwrap();

        assert!(app.is_path_target());
        assert_eq!(app.name, "Catalog Name");
        assert_eq!(
            app.bin_path(),
            Some(PathBuf::from(r"C:\Tools\Sample.exe").as_path())
        );
        assert_eq!(app.additional_processes, vec!["Sample.exe".to_string()]);
    }

    #[test]
    fn test_apps_from_dropped_paths_stops_after_first_parse_error() {
        let discovered = apps_from_dropped_paths(vec![
            PathBuf::from(r"C:\Tools\Sample.exe"),
            PathBuf::from(r"C:\Tools\NoExtension"),
            PathBuf::from(r"C:\Tools\NeverParsed.exe"),
        ]);

        assert_eq!(discovered.apps.len(), 1);
        assert_eq!(
            discovered.apps[0].bin_path(),
            Some(PathBuf::from(r"C:\Tools\Sample.exe").as_path())
        );
        assert!(discovered
            .first_error
            .as_deref()
            .is_some_and(|error| error.contains("Failed to get file extension")));
    }

    #[test]
    fn test_catalog_source_contracts_match_picker_expectations() {
        assert_eq!(
            InstalledAppCatalogSource::WindowsAppsFolder.label(),
            "AppsFolder"
        );
        assert_eq!(
            InstalledAppCatalogSource::LinuxDesktopEntry.label(),
            "Desktop entry"
        );
        assert!(
            InstalledAppCatalogSource::WindowsAppsFolder.picker_priority()
                < InstalledAppCatalogSource::WindowsStartMenu.picker_priority()
        );
        assert!(InstalledAppCatalogSource::LinuxPathExecutable.hide_until_query());
        assert!(!InstalledAppCatalogSource::LinuxDesktopEntry.hide_until_query());
    }

    #[test]
    fn test_catalog_entry_builders_populate_detail_from_target() {
        let aumid = InstalledAppCatalogEntry::new_aumid(
            "Store App",
            "Package!App",
            InstalledAppCatalogSource::WindowsAppsFolder,
        );
        assert_eq!(aumid.detail, "Package!App");
        assert_eq!(
            aumid.target,
            InstalledAppCatalogTarget::Aumid("Package!App".to_string())
        );

        let path = InstalledAppCatalogEntry::new_path(
            "Tool",
            r"C:\Tools\Tool.exe",
            InstalledAppCatalogSource::WindowsAppPaths,
        )
        .with_detail("Resolved Tool");
        assert_eq!(path.detail, "Resolved Tool");
        assert_eq!(
            path.target,
            InstalledAppCatalogTarget::Path(PathBuf::from(r"C:\Tools\Tool.exe"))
        );
    }
}
