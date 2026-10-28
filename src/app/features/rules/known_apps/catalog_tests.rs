use super::*;

const CASES: &[(&str, &str)] = &[
    ("epic-games", "Portal/Binaries/Win64/EpicGamesLauncher.exe"),
    ("battle-net", "Battle.net.exe"),
    ("ea-app", "EADesktop.exe"),
    ("ubisoft-connect", "upc.exe"),
    ("riot-client", "RiotClientServices.exe"),
    ("gog-galaxy", "GalaxyClient.exe"),
    ("rockstar-launcher", "Launcher.exe"),
    ("obs-studio", "bin/64bit/obs64.exe"),
    ("logitech-g-hub", "lghub.exe"),
    ("steelseries-gg", "SteelSeriesGG.exe"),
    ("razer-synapse", "RazerAppEngine.exe"),
    ("microsoft-teams", "ms-teams.exe"),
    ("zoom", "bin/Zoom.exe"),
    ("new-outlook", "olk.exe"),
    ("adobe-creative-cloud", "ACC/Creative Cloud.exe"),
    ("visual-studio", "Common7/IDE/devenv.exe"),
    ("nvidia-app", "CEF/NVIDIA app.exe"),
    ("amd-software", "RadeonSoftware.exe"),
];

fn imported(relative: &str) -> AppToRun {
    let path = PathBuf::from("C:/OwnedInstall").join(relative);
    AppToRun::new_path(
        path.clone(),
        vec![],
        path,
        os_api::PriorityClass::Normal,
        false,
    )
}

#[test]
fn s108_remaining_eighteen_profiles_are_data_driven_and_scoped() {
    for &(id, relative) in CASES {
        let mut app = imported(relative);
        let (entry, _, _) = candidate(&app).unwrap_or_else(|| panic!("missing profile {id}"));
        seed(&mut app, entry);
        assert_eq!(
            app.builtin_process_binding.as_ref().unwrap()["catalog_id"],
            id
        );
        let scope = project(&app).unwrap();
        assert_eq!(
            scope.role(app.bin_path().unwrap()),
            Some(MemberRole::Main),
            "{id}"
        );
        assert_eq!(
            scope.role(&PathBuf::from("D:/OtherInstall").join(relative)),
            None,
            "{id}"
        );
        let saved = app.builtin_process_binding.clone();
        app.additional_processes.clear();
        seed(&mut app, entry);
        assert!(app.additional_processes.is_empty());
        assert_eq!(app.builtin_process_binding, saved);
        app.builtin_process_binding.as_mut().unwrap()["definition_version"] = 99.into();
        assert!(project(&app).is_err());
    }
}

#[test]
fn s108_owned_helpers_exclude_shared_runtimes_services_and_games() {
    for (main, helper) in [
        (
            "Portal/Binaries/Win64/EpicGamesLauncher.exe",
            "Engine/Binaries/Win64/EpicWebHelper.exe",
        ),
        (
            "bin/64bit/obs64.exe",
            "obs-plugins/64bit/obs-browser-page.exe",
        ),
        ("EADesktop.exe", "EACefSubProcess.exe"),
        ("lghub.exe", "lghub_agent.exe"),
        ("SteelSeriesGG.exe", "SteelSeriesGGClient.exe"),
        ("bin/Zoom.exe", "bin/CptHost.exe"),
        ("RadeonSoftware.exe", "QtWebEngineProcess.exe"),
    ] {
        let mut app = imported(main);
        let entry = candidate(&app).expect("profile").0;
        seed(&mut app, entry);
        let scope = project(&app).unwrap();
        let path = PathBuf::from("C:/OwnedInstall").join(helper);
        assert_eq!(
            scope.role(&path),
            Some(MemberRole::Helper),
            "{main}: {helper}"
        );
        assert_eq!(
            scope.role(&PathBuf::from("D:/OtherInstall").join(helper)),
            None
        );
        for excluded in [
            "EABackgroundService.exe",
            "Agent.exe",
            "vgc.exe",
            "NVDisplay.Container.exe",
            "Game/game.exe",
            "Games/msedgewebview2.exe",
        ] {
            assert_eq!(
                scope.role(&PathBuf::from("C:/OwnedInstall").join(excluded)),
                None
            );
        }
    }
    for main in ["ms-teams.exe", "olk.exe", "ACC/Creative Cloud.exe"] {
        let mut app = imported(main);
        let entry = candidate(&app).unwrap().0;
        seed(&mut app, entry);
        assert!(!app
            .additional_processes
            .iter()
            .any(|name| name.eq_ignore_ascii_case("msedgewebview2.exe")
                || name.eq_ignore_ascii_case("CoreSync.exe")));
    }
}

#[test]
fn s108_version_components_are_numeric_and_launchers_have_real_main_roles() {
    assert!(catalog::matches(
        "Battle.net.*/Battle.net Helper.exe",
        "Battle.net.15136/Battle.net Helper.exe"
    ));
    for bad in [
        "Battle.net.../Battle.net Helper.exe",
        "Battle.net.evil/Battle.net Helper.exe",
        "Battle.net.1/Game/Battle.net Helper.exe",
    ] {
        assert!(!catalog::matches("Battle.net.*/Battle.net Helper.exe", bad));
    }
    for (target, main) in [
        ("EALauncher.exe", "EADesktop.exe"),
        ("LauncherPatcher.exe", "Launcher.exe"),
        ("Battle.net Launcher.exe", "Battle.net.15136/Battle.net.exe"),
        ("Uplay.exe", "upc.exe"),
    ] {
        let mut app = imported(target);
        let entry = candidate(&app).unwrap().0;
        seed(&mut app, entry);
        let scope = project(&app).unwrap();
        assert!(scope.is_launcher());
        assert_eq!(scope.role(app.bin_path().unwrap()), None);
        assert_eq!(
            scope.role(&PathBuf::from("C:/OwnedInstall").join(main)),
            Some(MemberRole::Main)
        );
    }
}

#[test]
fn s108_missing_real_main_does_not_bind_an_incomplete_launcher_installation() {
    let root = std::env::temp_dir().join(format!("cat-catalog-empty-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let profile = &catalog::PROFILES[2];
    assert!(profile.existing_names(&root).is_empty());
    assert!(!catalog::has_main(profile, &profile.existing_names(&root)));
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn s108_later_manual_known_helper_keeps_its_installation_scope() {
    let mut app = imported("EADesktop.exe");
    let entry = candidate(&app).unwrap().0;
    seed_names(&mut app, entry, vec!["EADesktop.exe"]);
    app.additional_processes.push("EACefSubProcess.exe".into());
    assert!(manual_names(&app).is_empty());
    let scope = project(&app).unwrap();
    assert_eq!(
        scope.role(Path::new("C:/OwnedInstall/EACefSubProcess.exe")),
        Some(MemberRole::Helper)
    );
    assert_eq!(
        scope.role(Path::new("D:/OtherInstall/EACefSubProcess.exe")),
        None
    );
}

#[cfg(windows)]
#[test]
#[ignore = "explicit native PE/layout import qualification; builds18 owned files but never executes them"]
fn s108_owned_catalog_native_imports_are_qualified() {
    let target = std::env::current_dir().unwrap().join("target");
    let root = target.join(format!(
        "catalog-native-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let compiler = PathBuf::from(std::env::var_os("WINDIR").unwrap())
        .join("Microsoft.NET/Framework64/v4.0.30319/csc.exe");
    let source = root.join("Fixture.cs");
    let fixture = root.join("Fixture.exe");
    for profile in catalog::PROFILES {
        std::fs::write(&source,format!("using System.Reflection; [assembly: AssemblyProduct(\"Owned catalog fixture\")] [assembly: AssemblyCompany(\"{}\")] class Fixture {{ static void Main() {{ }} }}",profile.vendors[0])).unwrap();
        let output = std::process::Command::new(&compiler)
            .args(["/nologo", "/target:winexe"])
            .arg(format!("/out:{}", fixture.display()))
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            profile.id,
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(!os_api::app_identity::qualify_catalog_image(
            &fixture,
            &["Unrelated fixture company"]
        ));
        let install = root.join(profile.id);
        for pattern in profile
            .mains
            .iter()
            .chain(profile.launchers)
            .chain(profile.helpers)
        {
            let file = install.join(pattern.replace('*', "108"));
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::copy(&fixture, &file).unwrap();
        }
        for relative in profile.mains.iter().chain(profile.launchers) {
            let file = install.join(relative.replace('*', "108"));
            let mut app = AppToRun::new_path(
                file.clone(),
                vec![],
                file,
                os_api::PriorityClass::Normal,
                false,
            );
            prepare_new_gui_import(&mut app);
            let scope = project(&app).unwrap_or_else(|e| panic!("{}: {relative}: {e}", profile.id));
            assert!(scope.qualify_launch(&app));
            assert_eq!(
                app.builtin_process_binding.as_ref().unwrap()["catalog_id"],
                profile.id
            );
            for helper in profile.helpers {
                let file = install.join(helper.replace('*', "108"));
                assert_eq!(
                    scope.role(&file),
                    Some(MemberRole::Helper),
                    "{}: {helper}",
                    profile.id
                );
            }
            let before = app.clone();
            app.additional_processes.clear();
            prepare_new_gui_import(&mut app);
            assert!(app.additional_processes.is_empty());
            assert_eq!(app.builtin_process_binding, before.builtin_process_binding);
        }
    }
    assert!(root
        .canonicalize()
        .unwrap()
        .starts_with(target.canonicalize().unwrap()));
    std::fs::remove_dir_all(&root).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "explicit read-only installed AMD UI qualification"]
fn s108_installed_amd_ui_is_qualified_without_shared_driver_processes() {
    let path = PathBuf::from(std::env::var_os("ProgramFiles").unwrap())
        .join("AMD/CNext/CNext/RadeonSoftware.exe");
    let mut app = AppToRun::new_path(
        path.clone(),
        vec![],
        path,
        os_api::PriorityClass::Normal,
        false,
    );
    prepare_new_gui_import(&mut app);
    assert_eq!(
        app.builtin_process_binding.as_ref().unwrap()["catalog_id"],
        "amd-software"
    );
    assert!(app
        .additional_processes
        .iter()
        .any(|name| name == "QtWebEngineProcess.exe"));
    assert!(!app
        .additional_processes
        .iter()
        .any(|name| name == "PresentMon-x64.exe" || name == "AMDRSServ.exe"));
}
