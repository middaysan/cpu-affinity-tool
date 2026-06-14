use crate::app::features::rules::RulesContext;
use crate::app::models::AppStateStorage;
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::shortcut_launch::{
    build_saved_rule_shortcut, SavedRuleShortcutRequest, ShortcutBuildError,
};
use os_api::ShortcutSpec;
use std::path::{Path, PathBuf};

const MAX_SHORTCUT_FILENAME_ATTEMPTS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutWriteError {
    AlreadyExists,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateRuleShortcutError {
    MissingGroup,
    MissingRule,
    NoTarget,
    DraftNotLoaded,
    SaveChangesFirst,
    InvalidShortcutId,
    UnsupportedPlatform,
    CurrentExeUnavailable,
    DesktopUnavailable,
    StorageUnavailable,
    NameCollisionExhausted,
    CreateCollision,
    OsWriteFailure,
}

impl CreateRuleShortcutError {
    pub fn to_user_message(&self) -> &'static str {
        match self {
            Self::MissingGroup => "Shortcut target group was not found.",
            Self::MissingRule => "Shortcut target rule was not found.",
            Self::NoTarget => "Shortcut target is unavailable.",
            Self::DraftNotLoaded => "Shortcut target is not loaded.",
            Self::SaveChangesFirst => "Save changes before creating a shortcut.",
            Self::InvalidShortcutId => "Shortcut target ID is invalid.",
            Self::UnsupportedPlatform => "Desktop shortcut creation is only supported on Windows.",
            Self::CurrentExeUnavailable => "Current executable path is unavailable.",
            Self::DesktopUnavailable => "Desktop folder is unavailable.",
            Self::StorageUnavailable => "Saved app state is unavailable.",
            Self::NameCollisionExhausted => "Could not find an available shortcut filename.",
            Self::CreateCollision => "Shortcut filename was taken before it could be created.",
            Self::OsWriteFailure => "Shortcut could not be written.",
        }
    }
}

pub trait RuleShortcutPlatform {
    fn is_supported(&self) -> bool;
    fn current_exe_path(&mut self) -> Result<PathBuf, String>;
    fn current_user_desktop_dir(&mut self) -> Result<PathBuf, String>;
    fn shortcut_path_exists(&mut self, path: &Path) -> Result<bool, String>;
    fn create_shortcut_new(&mut self, spec: ShortcutSpec) -> Result<(), ShortcutWriteError>;
}

pub struct SystemRuleShortcutPlatform;

impl RuleShortcutPlatform for SystemRuleShortcutPlatform {
    fn is_supported(&self) -> bool {
        cfg!(all(target_os = "windows", feature = "windows"))
    }

    fn current_exe_path(&mut self) -> Result<PathBuf, String> {
        crate::app::adapters::os::current_exe_path()
    }

    fn current_user_desktop_dir(&mut self) -> Result<PathBuf, String> {
        crate::app::adapters::os::current_user_desktop_dir()
    }

    fn shortcut_path_exists(&mut self, path: &Path) -> Result<bool, String> {
        crate::app::adapters::os::shortcut_path_exists(path)
    }

    fn create_shortcut_new(&mut self, spec: ShortcutSpec) -> Result<(), ShortcutWriteError> {
        crate::app::adapters::os::create_shortcut_new(spec).map_err(|err| match err {
            crate::app::adapters::os::CreateShortcutNewError::AlreadyExists => {
                ShortcutWriteError::AlreadyExists
            }
            crate::app::adapters::os::CreateShortcutNewError::ReserveFailed(message)
            | crate::app::adapters::os::CreateShortcutNewError::WriteFailed(message) => {
                ShortcutWriteError::Failed(message)
            }
        })
    }
}

pub fn create_saved_rule_shortcut(
    storage: &AppStateStorage,
    rules: &RulesContext,
    group_id: GroupId,
    rule_id: RuleId,
    platform: &mut impl RuleShortcutPlatform,
) -> Result<PathBuf, CreateRuleShortcutError> {
    let group_index = rules
        .group_index_for_id(&group_id)
        .ok_or(CreateRuleShortcutError::MissingGroup)?;
    let rule_index = rules
        .rule_index_for_id(group_index, &rule_id)
        .ok_or(CreateRuleShortcutError::MissingRule)?;
    let group = storage
        .groups
        .get(group_index)
        .ok_or(CreateRuleShortcutError::MissingGroup)?;
    let app = group
        .programs
        .get(rule_index)
        .ok_or(CreateRuleShortcutError::MissingRule)?;

    if !platform.is_supported() {
        return Err(CreateRuleShortcutError::UnsupportedPlatform);
    }

    let executable_path = platform
        .current_exe_path()
        .map_err(|_| CreateRuleShortcutError::CurrentExeUnavailable)?;
    let desktop_dir = platform
        .current_user_desktop_dir()
        .map_err(|_| CreateRuleShortcutError::DesktopUnavailable)?;

    let saved_spec = build_saved_rule_shortcut(SavedRuleShortcutRequest {
        executable_path: executable_path.clone(),
        app_name: app.name.clone(),
        group_name: group.name.clone(),
        group_id,
        rule_id,
    })
    .map_err(|err| match err {
        ShortcutBuildError::InvalidGroupId(_) | ShortcutBuildError::InvalidRuleId(_) => {
            CreateRuleShortcutError::InvalidShortcutId
        }
    })?;

    let shortcut_path = allocate_shortcut_path(platform, &desktop_dir, &saved_spec.display_name)?;
    let spec = ShortcutSpec {
        shortcut_path: shortcut_path.clone(),
        target_path: saved_spec.target_path,
        arguments: saved_spec.arguments,
        working_dir: saved_spec.working_dir,
        icon_path: Some(executable_path),
        icon_index: 0,
    };

    platform
        .create_shortcut_new(spec)
        .map_err(|err| match err {
            ShortcutWriteError::AlreadyExists => CreateRuleShortcutError::CreateCollision,
            ShortcutWriteError::Failed(_) => CreateRuleShortcutError::OsWriteFailure,
        })?;

    Ok(shortcut_path)
}

fn allocate_shortcut_path(
    platform: &mut impl RuleShortcutPlatform,
    desktop_dir: &Path,
    display_name: &str,
) -> Result<PathBuf, CreateRuleShortcutError> {
    for attempt in 0..MAX_SHORTCUT_FILENAME_ATTEMPTS {
        let candidate = desktop_dir.join(shortcut_filename(display_name, attempt));
        let exists = platform
            .shortcut_path_exists(&candidate)
            .map_err(|_| CreateRuleShortcutError::OsWriteFailure)?;
        if !exists {
            return Ok(candidate);
        }
    }

    Err(CreateRuleShortcutError::NameCollisionExhausted)
}

fn shortcut_filename(display_name: &str, attempt: usize) -> String {
    if attempt == 0 {
        return format!("{display_name}.lnk");
    }

    let suffix = format!(" ({attempt})");
    let max_base_chars = 120usize.saturating_sub(suffix.chars().count()).max(1);
    let mut base = display_name
        .chars()
        .take(max_base_chars)
        .collect::<String>()
        .trim_end_matches([' ', '.'])
        .to_string();
    if base.is_empty() {
        base = "CPU Affinity Rule".to_string();
    }

    format!("{base}{suffix}.lnk")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::features::rules::RulesContext;
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema};
    use os_api::PriorityClass;
    use std::collections::HashSet;

    struct FakePlatform {
        supported: bool,
        current_exe: Result<PathBuf, String>,
        desktop_dir: Result<PathBuf, String>,
        existing_paths: HashSet<String>,
        create_result: Result<(), ShortcutWriteError>,
        current_exe_calls: usize,
        desktop_calls: usize,
        exists_calls: Vec<PathBuf>,
        create_calls: Vec<ShortcutSpec>,
    }

    impl Default for FakePlatform {
        fn default() -> Self {
            Self {
                supported: false,
                current_exe: Err("current exe unavailable".to_string()),
                desktop_dir: Err("desktop unavailable".to_string()),
                existing_paths: HashSet::new(),
                create_result: Ok(()),
                current_exe_calls: 0,
                desktop_calls: 0,
                exists_calls: Vec::new(),
                create_calls: Vec::new(),
            }
        }
    }

    impl FakePlatform {
        fn supported() -> Self {
            Self {
                supported: true,
                current_exe: Ok(fake_current_exe()),
                desktop_dir: Ok(fake_desktop_dir()),
                create_result: Ok(()),
                ..Self::default()
            }
        }

        fn add_existing(&mut self, path: impl Into<PathBuf>) {
            self.existing_paths
                .insert(Self::path_key(path.into().as_path()));
        }

        fn path_key(path: &Path) -> String {
            path.to_string_lossy().to_ascii_lowercase()
        }
    }

    fn fake_tool_dir() -> PathBuf {
        PathBuf::from(r"C:\Tools")
    }

    fn fake_current_exe() -> PathBuf {
        fake_tool_dir().join("cpu-affinity-tool.exe")
    }

    fn fake_desktop_dir() -> PathBuf {
        PathBuf::from(r"C:\Users\Ada\Desktop")
    }

    fn fake_desktop_shortcut(file_name: &str) -> PathBuf {
        fake_desktop_dir().join(file_name)
    }

    impl RuleShortcutPlatform for FakePlatform {
        fn is_supported(&self) -> bool {
            self.supported
        }

        fn current_exe_path(&mut self) -> Result<PathBuf, String> {
            self.current_exe_calls += 1;
            self.current_exe.clone()
        }

        fn current_user_desktop_dir(&mut self) -> Result<PathBuf, String> {
            self.desktop_calls += 1;
            self.desktop_dir.clone()
        }

        fn shortcut_path_exists(&mut self, path: &Path) -> Result<bool, String> {
            self.exists_calls.push(path.to_path_buf());
            Ok(self.existing_paths.contains(&Self::path_key(path)))
        }

        fn create_shortcut_new(&mut self, spec: ShortcutSpec) -> Result<(), ShortcutWriteError> {
            self.create_calls.push(spec);
            self.create_result.clone()
        }
    }

    fn app_to_run(name: &str) -> AppToRun {
        let mut app = AppToRun::new_path(
            PathBuf::from(format!(r"C:\Games\{name}.lnk")),
            vec!["--fullscreen".to_string()],
            PathBuf::from(format!(r"C:\Games\{name}.exe")),
            PriorityClass::Normal,
            false,
        );
        app.name = name.to_string();
        app
    }

    fn installed_app(name: &str) -> AppToRun {
        AppToRun::new_installed(
            name.to_string(),
            "Vendor.Package_abc!App".to_string(),
            PriorityClass::Normal,
            false,
        )
    }

    fn storage_with(programs: Vec<AppToRun>) -> (AppStateStorage, RulesContext) {
        let storage = AppStateStorage {
            version: 7,
            groups: vec![CoreGroup {
                name: "Performance".to_string(),
                cores: vec![0, 1],
                programs,
                is_hidden: false,
                run_all_button: true,
            }],
            cpu_schema: CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
            rule_identities: None,
            loaded_version: 7,
            pending_pre_v6_backup: false,
        };
        let rules = RulesContext::from_storage(&storage);
        (storage, rules)
    }

    #[test]
    fn test_create_saved_rule_shortcut_builds_windows_spec_for_saved_rule() {
        let (storage, rules) = storage_with(vec![app_to_run("Game")]);
        let group_id = rules.group_id_for_index(0).unwrap();
        let rule_id = rules.rule_id_for_index(0, 0).unwrap();
        let mut platform = FakePlatform::supported();

        let created =
            create_saved_rule_shortcut(&storage, &rules, group_id, rule_id, &mut platform).unwrap();

        assert_eq!(created, fake_desktop_shortcut("Game - Performance.lnk"));
        assert_eq!(platform.create_calls.len(), 1);
        let spec = &platform.create_calls[0];
        assert_eq!(spec.shortcut_path, created);
        assert_eq!(spec.target_path, fake_current_exe());
        assert_eq!(spec.arguments, vec!["--run-rule", "group-0", "rule-0"]);
        assert_eq!(spec.working_dir, Some(fake_tool_dir()));
        assert_eq!(spec.icon_path, Some(fake_current_exe()));
        assert_eq!(spec.icon_index, 0);
    }

    #[test]
    fn test_create_saved_rule_shortcut_rejects_stale_targets_before_os_calls() {
        let (storage, rules) = storage_with(vec![app_to_run("Game")]);
        let group_id = rules.group_id_for_index(0).unwrap();
        let rule_id = rules.rule_id_for_index(0, 0).unwrap();
        let mut platform = FakePlatform::supported();

        assert_eq!(
            create_saved_rule_shortcut(
                &storage,
                &rules,
                GroupId("missing".to_string()),
                rule_id.clone(),
                &mut platform
            ),
            Err(CreateRuleShortcutError::MissingGroup)
        );
        assert_eq!(
            create_saved_rule_shortcut(
                &storage,
                &rules,
                group_id,
                RuleId("missing".to_string()),
                &mut platform
            ),
            Err(CreateRuleShortcutError::MissingRule)
        );
        assert_eq!(platform.current_exe_calls, 0);
        assert!(platform.create_calls.is_empty());
    }

    #[test]
    fn test_create_saved_rule_shortcut_rejects_moved_rule_under_old_group() {
        let (mut storage, rules) = storage_with(vec![app_to_run("Game")]);
        storage.groups.push(CoreGroup {
            name: "Other".to_string(),
            cores: vec![2, 3],
            programs: Vec::new(),
            is_hidden: false,
            run_all_button: true,
        });
        let old_group_id = rules.group_id_for_index(0).unwrap();
        let moved_rule_id = rules.rule_id_for_index(0, 0).unwrap();
        let moved_app = storage.groups[0].programs.remove(0);
        storage.groups[1].programs.push(moved_app);
        let moved_rules = RulesContext::from_storage(&storage);
        let mut platform = FakePlatform::supported();

        assert_eq!(
            create_saved_rule_shortcut(
                &storage,
                &moved_rules,
                old_group_id,
                moved_rule_id,
                &mut platform
            ),
            Err(CreateRuleShortcutError::MissingRule)
        );
        assert!(platform.create_calls.is_empty());
    }

    #[test]
    fn test_create_saved_rule_shortcut_unsupported_platform_short_circuits() {
        let (storage, rules) = storage_with(vec![app_to_run("Game")]);
        let group_id = rules.group_id_for_index(0).unwrap();
        let rule_id = rules.rule_id_for_index(0, 0).unwrap();
        let mut platform = FakePlatform::default();

        assert_eq!(
            create_saved_rule_shortcut(&storage, &rules, group_id, rule_id, &mut platform),
            Err(CreateRuleShortcutError::UnsupportedPlatform)
        );
        assert_eq!(platform.current_exe_calls, 0);
        assert_eq!(platform.desktop_calls, 0);
        assert!(platform.create_calls.is_empty());
    }

    #[test]
    fn test_create_saved_rule_shortcut_reports_resolver_failures() {
        let (storage, rules) = storage_with(vec![app_to_run("Game")]);
        let group_id = rules.group_id_for_index(0).unwrap();
        let rule_id = rules.rule_id_for_index(0, 0).unwrap();
        let mut exe_fail = FakePlatform::supported();
        exe_fail.current_exe = Err("no exe".to_string());

        assert_eq!(
            create_saved_rule_shortcut(
                &storage,
                &rules,
                group_id.clone(),
                rule_id.clone(),
                &mut exe_fail
            ),
            Err(CreateRuleShortcutError::CurrentExeUnavailable)
        );

        let mut desktop_fail = FakePlatform::supported();
        desktop_fail.desktop_dir = Err("no desktop".to_string());
        assert_eq!(
            create_saved_rule_shortcut(&storage, &rules, group_id, rule_id, &mut desktop_fail),
            Err(CreateRuleShortcutError::DesktopUnavailable)
        );
    }

    #[test]
    fn test_create_saved_rule_shortcut_allocates_collision_suffixes_case_insensitive() {
        let (storage, rules) = storage_with(vec![app_to_run("Game")]);
        let group_id = rules.group_id_for_index(0).unwrap();
        let rule_id = rules.rule_id_for_index(0, 0).unwrap();
        let mut platform = FakePlatform::supported();
        platform.add_existing(fake_desktop_shortcut("GAME - PERFORMANCE.LNK"));

        let created =
            create_saved_rule_shortcut(&storage, &rules, group_id, rule_id, &mut platform).unwrap();

        assert_eq!(created, fake_desktop_shortcut("Game - Performance (1).lnk"));
    }

    #[test]
    fn test_create_saved_rule_shortcut_reports_exhausted_and_race_collisions() {
        let (storage, rules) = storage_with(vec![app_to_run("Game")]);
        let group_id = rules.group_id_for_index(0).unwrap();
        let rule_id = rules.rule_id_for_index(0, 0).unwrap();
        let mut exhausted = FakePlatform::supported();
        exhausted.add_existing(fake_desktop_shortcut("Game - Performance.lnk"));
        for index in 1..100 {
            exhausted.add_existing(fake_desktop_shortcut(&format!(
                "Game - Performance ({index}).lnk"
            )));
        }

        assert_eq!(
            create_saved_rule_shortcut(
                &storage,
                &rules,
                group_id.clone(),
                rule_id.clone(),
                &mut exhausted
            ),
            Err(CreateRuleShortcutError::NameCollisionExhausted)
        );

        let mut race = FakePlatform::supported();
        race.create_result = Err(ShortcutWriteError::AlreadyExists);
        assert_eq!(
            create_saved_rule_shortcut(&storage, &rules, group_id, rule_id, &mut race),
            Err(CreateRuleShortcutError::CreateCollision)
        );
    }

    #[test]
    fn test_create_saved_rule_shortcut_uses_saved_installed_app_name() {
        let (storage, rules) = storage_with(vec![installed_app("Spotify")]);
        let group_id = rules.group_id_for_index(0).unwrap();
        let rule_id = rules.rule_id_for_index(0, 0).unwrap();
        let mut platform = FakePlatform::supported();

        let created =
            create_saved_rule_shortcut(&storage, &rules, group_id, rule_id, &mut platform).unwrap();

        assert_eq!(created, fake_desktop_shortcut("Spotify - Performance.lnk"));
        assert!(!created.to_string_lossy().contains("Vendor.Package_abc!App"));
    }
}
