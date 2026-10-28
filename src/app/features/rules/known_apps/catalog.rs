//! Finite app-owned layouts. Shared services/runtimes are deliberately absent.
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Profile {
    pub id: &'static str,
    pub mains: &'static [&'static str],
    pub launchers: &'static [&'static str],
    pub helpers: &'static [&'static str],
    pub vendors: &'static [&'static str],
}

pub(super) const PROFILES: &[Profile] = &[
    Profile { id: "epic-games", mains: &["Portal/Binaries/Win64/EpicGamesLauncher.exe", "Portal/Binaries/Win32/EpicGamesLauncher.exe"], launchers: &[], helpers: &["Engine/Binaries/Win64/EpicWebHelper.exe", "Engine/Binaries/Win32/EpicWebHelper.exe", "Portal/Binaries/Win64/EpicWebHelper.exe"], vendors: &["Epic Games"] },
    Profile { id: "battle-net", mains: &["Battle.net.exe", "Battle.net.*/Battle.net.exe"], launchers: &["Battle.net Launcher.exe"], helpers: &["Battle.net Helper.exe", "Battle.net.*/Battle.net Helper.exe"], vendors: &["Blizzard"] },
    Profile { id: "ea-app", mains: &["EADesktop.exe"], launchers: &["EALauncher.exe"], helpers: &["EACefSubProcess.exe"], vendors: &["Electronic Arts", "EA Swiss"] },
    Profile { id: "ubisoft-connect", mains: &["upc.exe", "UbisoftConnect.exe"], launchers: &["UbisoftGameLauncher.exe", "UbisoftGameLauncher64.exe", "Uplay.exe"], helpers: &["UbisoftWebCore.exe"], vendors: &["Ubisoft"] },
    Profile { id: "riot-client", mains: &["RiotClientServices.exe"], launchers: &[], helpers: &["UX/RiotClientUx.exe", "UX/RiotClientUxRender.exe"], vendors: &["Riot Games"] },
    Profile { id: "gog-galaxy", mains: &["GalaxyClient.exe"], launchers: &[], helpers: &["GalaxyClient Helper.exe"], vendors: &["GOG"] },
    Profile { id: "rockstar-launcher", mains: &["Launcher.exe"], launchers: &["LauncherPatcher.exe"], helpers: &[], vendors: &["Rockstar", "Take-Two"] },
    Profile { id: "obs-studio", mains: &["bin/64bit/obs64.exe", "bin/32bit/obs32.exe"], launchers: &[], helpers: &["obs-plugins/64bit/obs-browser-page.exe", "obs-plugins/32bit/obs-browser-page.exe"], vendors: &["OBS", "Open Broadcaster"] },
    Profile { id: "logitech-g-hub", mains: &["lghub.exe", "system_tray/lghub_system_tray.exe"], launchers: &[], helpers: &["lghub_agent.exe"], vendors: &["Logitech", "Logi"] },
    Profile { id: "steelseries-gg", mains: &["SteelSeriesGG.exe"], launchers: &[], helpers: &["SteelSeriesGGClient.exe", "SteelSeriesGGEZClient.exe"], vendors: &["SteelSeries"] },
    Profile { id: "razer-synapse", mains: &["RazerAppEngine.exe"], launchers: &[], helpers: &[], vendors: &["Razer"] },
    Profile { id: "microsoft-teams", mains: &["ms-teams.exe"], launchers: &[], helpers: &[], vendors: &["Microsoft"] },
    Profile { id: "zoom", mains: &["bin/Zoom.exe", "bin_*/Zoom.exe"], launchers: &[], helpers: &["bin/CptHost.exe", "bin/CptControl.exe", "bin/zWebview2Agent.exe", "bin/ZoomHybridConf.exe", "bin_*/CptHost.exe", "bin_*/CptControl.exe", "bin_*/zWebview2Agent.exe", "bin_*/ZoomHybridConf.exe"], vendors: &["Zoom"] },
    Profile { id: "new-outlook", mains: &["olk.exe"], launchers: &[], helpers: &[], vendors: &["Microsoft"] },
    Profile { id: "adobe-creative-cloud", mains: &["ACC/Creative Cloud.exe"], launchers: &[], helpers: &["Utils/Adobe CEF Helper.exe", "ACC/Creative Cloud Helper.exe", "ACC/Adobe Creative Cloud Helper.exe"], vendors: &["Adobe"] },
    Profile { id: "visual-studio", mains: &["Common7/IDE/devenv.exe"], launchers: &[], helpers: &["Common7/ServiceHub/Hosts/ServiceHub.Host.CLR.x86/ServiceHub.RoslynCodeAnalysisService.exe", "Common7/ServiceHub/Hosts/ServiceHub.Host.CLR.x64/ServiceHub.RoslynCodeAnalysisService.exe", "Common7/ServiceHub/Hosts/ServiceHub.Host.dotnet.x64/ServiceHub.RoslynCodeAnalysisService.exe"], vendors: &["Microsoft"] },
    Profile { id: "nvidia-app", mains: &["CEF/NVIDIA app.exe"], launchers: &[], helpers: &["CEF/NVIDIA Overlay.exe", "Overlay/NVIDIA Overlay.exe"], vendors: &["NVIDIA"] },
    Profile { id: "amd-software", mains: &["RadeonSoftware.exe"], launchers: &[], helpers: &["QtWebEngineProcess.exe"], vendors: &["Advanced Micro Devices"] },
];

fn component_matches(pattern: &str, value: &str) -> bool {
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return pattern.eq_ignore_ascii_case(value);
    };
    value
        .get(prefix.len()..value.len().saturating_sub(suffix.len()))
        .filter(|_| {
            value
                .get(..prefix.len())
                .is_some_and(|text| text.eq_ignore_ascii_case(prefix))
                && value
                    .get(value.len().saturating_sub(suffix.len())..)
                    .is_some_and(|text| text.eq_ignore_ascii_case(suffix))
        })
        .is_some_and(|version| {
            !version.is_empty()
                && version.len() <= 80
                && version
                    .split('.')
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        })
}

pub(super) fn matches(pattern: &str, relative: &str) -> bool {
    let mut expected = pattern.split('/');
    let mut actual = relative.split(['\\', '/']);
    loop {
        match (expected.next(), actual.next()) {
            (None, None) => return true,
            (Some(pattern), Some(value)) if component_matches(pattern, value) => {}
            _ => return false,
        }
    }
}

pub(super) fn candidate(path: &Path) -> Option<(&'static Profile, PathBuf, String)> {
    PROFILES
        .iter()
        .flat_map(|profile| {
            profile
                .mains
                .iter()
                .chain(profile.launchers)
                .filter_map(move |pattern| {
                    if !path
                        .file_name()?
                        .eq_ignore_ascii_case(pattern.rsplit('/').next()?)
                    {
                        return None;
                    }
                    let depth = pattern.split('/').count();
                    let root = path.ancestors().nth(depth)?;
                    let relative = path.strip_prefix(root).ok()?.to_str()?;
                    matches(pattern, relative).then(|| {
                        (
                            profile,
                            root.to_path_buf(),
                            relative.replace('/', "\\").to_ascii_lowercase(),
                            depth,
                        )
                    })
                })
        })
        .max_by_key(|(_, _, _, depth)| *depth)
        .map(|(profile, root, relative, _)| (profile, root, relative))
}

pub(super) fn has_main(profile: &Profile, names: &[&str]) -> bool {
    profile.mains.iter().any(|pattern| {
        names.iter().any(|name| {
            pattern
                .rsplit('/')
                .next()
                .is_some_and(|main| main.eq_ignore_ascii_case(name))
        })
    })
}

impl Profile {
    pub fn names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        for path in self.mains.iter().chain(self.helpers) {
            let name = path.rsplit('/').next().expect("fixed layout");
            if !names
                .iter()
                .any(|old: &&str| old.eq_ignore_ascii_case(name))
            {
                names.push(name);
            }
        }
        names
    }

    pub fn existing_names(&self, root: &Path) -> Vec<&'static str> {
        let mut names = Vec::new();
        for pattern in self.mains.iter().chain(self.helpers) {
            if existing_pattern(root, pattern) {
                let name = pattern.rsplit('/').next().unwrap();
                if !names
                    .iter()
                    .any(|old: &&str| old.eq_ignore_ascii_case(name))
                {
                    names.push(name);
                }
            }
        }
        names
    }
}

/// Import-only finite lookup, including at most128 entries for a version component.
fn existing_pattern(root: &Path, pattern: &str) -> bool {
    let mut paths = vec![root.to_path_buf()];
    for component in pattern.split('/') {
        let mut next = Vec::new();
        for parent in paths {
            if component.contains('*') {
                let Ok(children) = std::fs::read_dir(&parent) else {
                    continue;
                };
                for (index, child) in children.enumerate() {
                    if index >= 128 {
                        return false;
                    }
                    if let Ok(child) = child {
                        if child
                            .file_name()
                            .to_str()
                            .is_some_and(|name| component_matches(component, name))
                            && next.len() < 128
                        {
                            next.push(child.path());
                        }
                    }
                }
            } else {
                next.push(parent.join(component));
            }
        }
        paths = next;
    }
    paths.iter().any(|path| {
        path.is_file()
            && os_api::app_identity::canonical_scope(root, path)
                .is_some_and(|(root, image)| image.starts_with(root))
    })
}
