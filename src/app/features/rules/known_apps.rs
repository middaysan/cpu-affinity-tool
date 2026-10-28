//! Import-only defaults. Generated names retain installation scope after manual edits.
use crate::app::models::{normalize_process_name, AppToRun};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
mod catalog;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Steam,
    DiscordDirect,
    DiscordLauncher,
    Catalog(&'static catalog::Profile),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemberRole {
    Main,
    Helper,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    catalog_id: String,
    definition_version: u32,
    generated_names: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Scope {
    pub entry: Entry,
    pub root: PathBuf,
    primary_relative: String,
    active: HashSet<String>,
    generated: HashSet<String>,
}

fn normalized_path(path: &Path) -> Option<String> {
    let text = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    if text.split('\\').any(|part| part == "." || part == "..") {
        return None;
    }
    Some(text.trim_end_matches('\\').to_string())
}

fn candidate(app: &AppToRun) -> Option<(Entry, PathBuf, String)> {
    let path = app.bin_path()?;
    let name = path.file_name()?.to_str()?;
    if name.eq_ignore_ascii_case("steam.exe") && !os_api::steam::is_game_launch_args(&app.args) {
        return Some((Entry::Steam, path.parent()?.into(), "steam.exe".into()));
    }
    if name.eq_ignore_ascii_case("Discord.exe") {
        let parent = path.parent()?;
        if os_api::app_identity::is_version_directory(parent.file_name()?.to_str()?) {
            let root = parent.parent()?;
            if root.file_name()?.eq_ignore_ascii_case("Discord") {
                return Some((
                    Entry::DiscordDirect,
                    root.into(),
                    format!(
                        "{}\\discord.exe",
                        parent.file_name()?.to_string_lossy().to_ascii_lowercase()
                    ),
                ));
            }
        }
    }
    if os_api::app_identity::discord_launcher_target(path, &app.args) {
        let root = path.parent()?;
        if root.file_name()?.eq_ignore_ascii_case("Discord") {
            return Some((Entry::DiscordLauncher, root.into(), String::new()));
        }
    }
    catalog::candidate(path)
        .map(|(profile, root, relative)| (Entry::Catalog(profile), root, relative))
}

fn default_names(entry: Entry) -> Vec<&'static str> {
    match entry {
        Entry::Steam => vec!["steamwebhelper.exe"],
        Entry::DiscordLauncher => vec!["Discord.exe"],
        Entry::DiscordDirect => vec![],
        Entry::Catalog(profile) => profile.names(),
    }
}

fn catalog_id(entry: Entry) -> &'static str {
    match entry {
        Entry::Steam => "steam",
        Entry::DiscordDirect | Entry::DiscordLauncher => "discord-stable",
        Entry::Catalog(profile) => profile.id,
    }
}

fn qualified(path: &Path, entry: Entry) -> bool {
    match entry {
        Entry::Catalog(profile) => {
            os_api::app_identity::qualify_catalog_image(path, profile.vendors)
        }
        _ => os_api::app_identity::qualify_installation(path, entry == Entry::DiscordLauncher),
    }
}

pub(crate) fn prepare_new_gui_import(app: &mut AppToRun) {
    if !cfg!(windows) || app.builtin_process_binding.is_some() {
        return;
    }
    let Some((entry, root, _)) = candidate(app) else {
        return;
    };
    let Some(path) = app.bin_path() else {
        return;
    };
    if qualified(path, entry) {
        let names = match entry {
            Entry::Catalog(profile) => {
                let names = profile.existing_names(&root);
                if !catalog::has_main(profile, &names) {
                    return;
                }
                names
            }
            _ => default_names(entry),
        };
        seed_names(app, entry, names);
    }
}

/// Called only by the existing save worker for an explicitly tagged GUI import.
pub(crate) fn prepare_new_gui_rules(
    existing: &super::RulesContext,
    candidate: &mut crate::app::models::AppStateStorage,
) {
    visit_new_rules(existing, candidate, prepare_new_gui_import);
}
fn visit_new_rules(
    existing: &super::RulesContext,
    candidate: &mut crate::app::models::AppStateStorage,
    mut prepare: impl FnMut(&mut AppToRun),
) {
    let existing: HashSet<_> = existing
        .to_persisted_identities()
        .groups
        .into_iter()
        .flat_map(|group| group.rule_ids)
        .collect();
    let next = super::RulesContext::from_storage(candidate);
    for (g, group) in candidate.groups.iter_mut().enumerate() {
        for (r, app) in group.programs.iter_mut().enumerate() {
            if next
                .rule_id_for_index(g, r)
                .is_some_and(|id| !existing.contains(&id))
            {
                prepare(app);
            }
        }
    }
}

#[cfg(test)]
fn seed(app: &mut AppToRun, entry: Entry) {
    seed_names(app, entry, default_names(entry));
}

fn seed_names(app: &mut AppToRun, entry: Entry, names: Vec<&str>) {
    if app.builtin_process_binding.is_some() || app.additional_processes.len() + names.len() > 64 {
        return;
    }
    if entry == Entry::DiscordLauncher {
        app.additional_processes
            .retain(|name| normalize_process_name(name) != "update");
    }
    if let Entry::Catalog(profile) = entry {
        if let Some((_, _, relative)) = candidate(app) {
            if profile
                .launchers
                .iter()
                .any(|pattern| catalog::matches(pattern, &relative))
            {
                let primary = app.primary_process_name_normalized();
                app.additional_processes
                    .retain(|name| Some(normalize_process_name(name)) != primary);
            }
        }
    }
    for &name in &names {
        if !app
            .additional_processes
            .iter()
            .any(|old| normalize_process_name(old) == normalize_process_name(name))
        {
            app.additional_processes.push(name.into());
        }
    }
    app.builtin_process_binding = Some(
        serde_json::to_value(Binding {
            catalog_id: catalog_id(entry).into(),
            definition_version: 1,
            generated_names: default_names(entry)
                .iter()
                .map(|name| (*name).into())
                .collect(),
        })
        .expect("fixed binding"),
    );
}

fn provenance(app: &AppToRun) -> Result<Binding, String> {
    let raw = app
        .builtin_process_binding
        .as_ref()
        .ok_or("No built-in binding")?;
    let object = raw
        .as_object()
        .ok_or("Unsupported built-in process binding")?;
    if object.len() != 3
        || !object
            .get("catalog_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| id.len() <= 64)
        || !object
            .get("definition_version")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|version| version <= u64::from(u32::MAX))
        || !object
            .get("generated_names")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|names| {
                names.len() <= 64
                    && names.iter().all(|name| {
                        name.as_str().is_some_and(|name| {
                            !name.is_empty()
                                && name.len() <= 256
                                && !name.contains(['\\', '/', ':', '\0'])
                        })
                    })
            })
    {
        return Err("Unsupported built-in process binding".into());
    }
    let binding: Binding =
        serde_json::from_value(raw.clone()).map_err(|_| "Unsupported built-in process binding")?;
    if binding.catalog_id.len() > 64
        || binding.generated_names.len() > 64
        || binding.generated_names.iter().any(|name| {
            name.is_empty() || name.len() > 256 || name.contains(['\\', '/', ':', '\0'])
        })
    {
        return Err("Invalid built-in process binding".into());
    }
    Ok(binding)
}

pub(crate) fn project(app: &AppToRun) -> Result<Scope, String> {
    let binding = provenance(app)?;
    let (entry, root, primary_relative) = candidate(app).ok_or("Built-in helpers do not match this launch target; re-add the application to refresh its defaults")?;
    let expected = catalog_id(entry);
    let generated: HashSet<_> = binding
        .generated_names
        .iter()
        .map(|name| normalize_process_name(name))
        .collect();
    if binding.catalog_id != expected
        || binding.definition_version != 1
        || (entry == Entry::DiscordLauncher && !generated.contains("discord"))
        || generated.iter().any(|name| {
            !default_names(entry)
                .iter()
                .any(|allowed| normalize_process_name(allowed) == *name)
        })
    {
        return Err("This built-in application definition is unsupported".into());
    }
    Ok(Scope {
        entry,
        root,
        primary_relative,
        generated,
        active: app
            .additional_processes
            .iter()
            .map(|name| normalize_process_name(name))
            .collect(),
    })
}

/// Explicit names outside import provenance retain their existing matching semantics.
pub(crate) fn manual_names(app: &AppToRun) -> Vec<String> {
    let Ok(binding) = provenance(app) else {
        return Vec::new();
    };
    let generated: HashSet<_> = binding
        .generated_names
        .iter()
        .map(|name| normalize_process_name(name))
        .collect();
    let primary = app.primary_process_name_normalized();
    app.additional_processes
        .iter()
        .map(|name| normalize_process_name(name))
        .filter(|name| !generated.contains(name) && Some(name) != primary.as_ref())
        .collect()
}

impl Scope {
    pub fn can_activate_hidden(&self) -> bool {
        !matches!(self.entry, Entry::Catalog(_))
    }
    pub fn is_launcher(&self) -> bool {
        self.entry == Entry::DiscordLauncher
            || matches!(self.entry, Entry::Catalog(profile) if profile.launchers.iter().any(|pattern|catalog::matches(pattern,&self.primary_relative)))
    }
    pub fn qualify_launch(&self, app: &AppToRun) -> bool {
        app.bin_path()
            .is_some_and(|path| qualified(path, self.entry))
    }
    #[cfg(test)]
    pub fn role(&self, image: &Path) -> Option<MemberRole> {
        self.role_under_root(&self.root, image)
    }

    /// Main lifetime witnesses remain independent of the editable metric selection.
    pub fn primary_names(&self) -> Vec<String> {
        match self.entry {
            Entry::Steam => vec!["steam".into()],
            Entry::DiscordDirect | Entry::DiscordLauncher => vec!["discord".into()],
            Entry::Catalog(profile) => profile
                .mains
                .iter()
                .map(|path| normalize_process_name(path.rsplit('/').next().unwrap()))
                .collect(),
        }
    }
    pub fn enabled(&self, image: &Path) -> bool {
        image
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| self.active.contains(&normalize_process_name(name)))
    }
    #[cfg(test)]
    pub fn role_under_root(&self, root: &Path, image: &Path) -> Option<MemberRole> {
        if !self.enabled(image) {
            return None;
        }
        self.witness_role_under_root(root, image)
    }
    pub fn witness_role_under_root(&self, root: &Path, image: &Path) -> Option<MemberRole> {
        let root = normalized_path(root)?;
        let image = normalized_path(image)?;
        let relative = image.strip_prefix(&(root + "\\"))?;
        let name = normalize_process_name(relative.rsplit('\\').next()?);
        if let Entry::Catalog(profile) = self.entry {
            if profile
                .launchers
                .iter()
                .any(|pattern| catalog::matches(pattern, relative))
            {
                return None;
            }
            if relative == self.primary_relative
                || (self.generated.contains(&name)
                    && profile
                        .mains
                        .iter()
                        .any(|pattern| catalog::matches(pattern, relative)))
            {
                return Some(MemberRole::Main);
            }
            return (self.generated.contains(&name)
                && profile
                    .helpers
                    .iter()
                    .any(|pattern| catalog::matches(pattern, relative)))
            .then_some(MemberRole::Helper);
        }
        if self.entry == Entry::DiscordLauncher {
            let parts: Vec<_> = relative.split('\\').collect();
            return (self.generated.contains(&name)
                && parts.len() == 2
                && os_api::app_identity::is_version_directory(parts[0])
                && parts[1] == "discord.exe")
                .then_some(MemberRole::Main);
        }
        if relative == self.primary_relative {
            return Some(MemberRole::Main);
        }
        const STEAM_HELPERS: &[&str] = &[
            "bin\\cef\\cef.win64\\steamwebhelper.exe",
            "bin\\cef\\cef.win7x64\\steamwebhelper.exe",
            "bin\\cef\\cef.win32\\steamwebhelper.exe",
            "bin\\cef\\cef.win7\\steamwebhelper.exe",
        ];
        (self.entry == Entry::Steam
            && self.generated.contains(&name)
            && STEAM_HELPERS.contains(&relative))
        .then_some(MemberRole::Helper)
    }
}

pub(crate) fn launch_ordinary(app: &AppToRun) -> Result<(), String> {
    let scope = project(app)?;
    let path = app.bin_path().ok_or("Missing application executable")?;
    if !scope.is_launcher() {
        return Err("This application is not an ordinary launcher".into());
    }
    match scope.entry {
        Entry::Catalog(profile) => {
            os_api::app_identity::activate_catalog_client(path, &app.args, profile.vendors)
        }
        _ => os_api::app_identity::launch_ordinary(path, &app.args),
    }
}

pub(crate) fn activate_client(app: &AppToRun) -> Result<(), String> {
    let scope = project(app)?;
    let path = app.bin_path().ok_or("Missing application executable")?;
    match scope.entry {
        Entry::Catalog(_) => Err(
            "No existing application window was found; this client is not re-executed by Focus"
                .into(),
        ),
        _ => os_api::app_identity::activate_client(path, &app.args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use os_api::PriorityClass;
    fn app(path: &str, args: &[&str]) -> AppToRun {
        AppToRun::new_path(
            path.into(),
            args.iter().map(|a| (*a).into()).collect(),
            path.into(),
            PriorityClass::Normal,
            false,
        )
    }
    #[test]
    fn s107_defaults_are_one_time_editable_and_do_not_change_launch_settings() {
        let mut app = app(r"C:/Steam/steam.exe", &["-silent"]);
        let original = app.clone();
        seed(&mut app, Entry::Steam);
        assert_eq!(
            app.additional_processes,
            ["steam.exe", "steamwebhelper.exe"]
        );
        assert_eq!(app.args, original.args);
        assert!(!app.manage_descendants);
        app.additional_processes.clear();
        seed(&mut app, Entry::Steam);
        assert!(app.additional_processes.is_empty());
        assert_eq!(
            project(&app)
                .unwrap()
                .role(Path::new(r"C:/Steam/bin/cef/cef.win64/steamwebhelper.exe")),
            None
        );
    }
    #[test]
    fn s107_discord_launcher_is_distinct_from_its_real_members() {
        let mut app = app(
            r"C:/Users/fixture/Discord/Update.exe",
            &["--processStart", "Discord.exe"],
        );
        assert_eq!(candidate(&app).unwrap().0, Entry::DiscordLauncher);
        seed(&mut app, Entry::DiscordLauncher);
        assert_eq!(app.additional_processes, ["Discord.exe"]);
        let scope = project(&app).unwrap();
        assert_eq!(
            scope.role(Path::new(
                r"C:/Users/fixture/Discord/app-1.0.999/Discord.exe"
            )),
            Some(MemberRole::Main)
        );
        assert_eq!(
            scope.role(Path::new(r"C:/Users/fixture/Discord/Update.exe")),
            None
        );
        assert_eq!(
            scope.role(Path::new(r"C:/Other/Discord/app-1.0.999/Discord.exe")),
            None
        );
        app.args.push("--other".into());
        assert!(project(&app).is_err());
        assert!(manual_names(&app).is_empty());
    }
    #[test]
    fn s107_future_binding_stays_inert_and_removed_names_keep_provenance() {
        let mut app = app(r"C:/Steam/steam.exe", &[]);
        seed(&mut app, Entry::Steam);
        app.additional_processes = vec![
            "steam.exe".into(),
            "STEAMWEBHELPER".into(),
            "manual.exe".into(),
        ];
        assert_eq!(manual_names(&app), ["manual"]);
        app.builtin_process_binding.as_mut().unwrap()["definition_version"] = 99.into();
        assert!(project(&app).is_err());
        assert_eq!(manual_names(&app), ["manual"]);
    }

    #[test]
    fn s107_only_new_gui_rule_ids_receive_defaults_not_existing_rules_or_reordering() {
        let (mut state, _) = crate::app::features::execution::known_tracking_fixture();
        for app in &mut state.groups[0].programs {
            app.builtin_process_binding = None;
            app.additional_processes = vec!["steam.exe".into()];
        }
        let mut existing = super::super::RulesContext::from_storage(&state);
        state.rule_identities = Some(existing.to_persisted_identities());
        let old = state.groups[0].programs.clone();
        state.groups[0]
            .programs
            .push(app(r"C:/Steam/steam.exe", &["-new"]));
        let before = existing.clone();
        existing.reconcile_with_storage(&state);
        state.rule_identities = Some(existing.to_persisted_identities());
        let mut calls = 0;
        visit_new_rules(&before, &mut state, |app| {
            calls += 1;
            seed(app, Entry::Steam);
        });
        assert_eq!(calls, 1);
        assert_eq!(state.groups[0].programs[..2], old);
        assert_eq!(
            state.groups[0].programs[2].additional_processes,
            ["steam.exe", "steamwebhelper.exe"]
        );
        let mut calls = 0;
        visit_new_rules(&existing, &mut state, |_| calls += 1);
        assert_eq!(calls, 0);
    }

    #[test]
    fn s107_current_discord_root_launcher_is_classified_without_tracking_the_stub() {
        for args in [&["--processStart", "Discord.exe"][..], &[][..]] {
            let mut app = app(r"C:/Fixture/Discord/Discord.exe", args);
            assert_eq!(candidate(&app).unwrap().0, Entry::DiscordLauncher);
            seed(&mut app, Entry::DiscordLauncher);
            let scope = project(&app).unwrap();
            assert_eq!(
                scope.role(Path::new(r"C:/Fixture/Discord/Discord.exe")),
                None
            );
            assert_eq!(
                scope.role(Path::new(r"C:/Fixture/Discord/app-1.0.9265/Discord.exe")),
                Some(MemberRole::Main)
            );
            assert_eq!(app.additional_processes, ["Discord.exe"]);
        }
    }

    #[test]
    fn s107_discord_launcher_rejects_unrecognized_arguments_and_provenance() {
        for args in [&["--other"][..], &["--processStart", "Other.exe"][..]] {
            assert!(candidate(&app(r"C:/Fixture/Discord/Discord.exe", args)).is_none());
        }
        let mut direct = app(r"C:/Fixture/Discord/app-1.0.1/Discord.exe", &[]);
        seed(&mut direct, Entry::DiscordDirect);
        if let crate::app::models::LaunchTarget::Path { bin_path, .. } = &mut direct.launch_target {
            *bin_path = r"C:/Fixture/Discord/Discord.exe".into();
        }
        assert!(
            project(&direct).is_err(),
            "changing a direct executable cannot create launcher provenance"
        );
    }

    #[test]
    #[ignore = "explicit read-only installed Discord shortcut qualification; CAT_DISCORD_SHORTCUT required"]
    fn s107_installed_discord_shortcut_is_recognized() {
        let shortcut = PathBuf::from(
            std::env::var_os("CAT_DISCORD_SHORTCUT").expect("select installed shortcut"),
        );
        let (target, args) = os_api::OS::parse_dropped_file(shortcut.clone()).unwrap();
        let mut app =
            AppToRun::new_path(shortcut, args, target, os_api::PriorityClass::Normal, false);
        prepare_new_gui_import(&mut app);
        let scope = project(&app).expect("installed shortcut must retain qualified Discord intent");
        assert_eq!(scope.entry, Entry::DiscordLauncher);
        assert_eq!(app.additional_processes, ["Discord.exe"]);
        assert_eq!(scope.role(app.bin_path().unwrap()), None);
    }
}

#[cfg(test)]
#[path = "known_apps/catalog_tests.rs"]
mod catalog_tests;
