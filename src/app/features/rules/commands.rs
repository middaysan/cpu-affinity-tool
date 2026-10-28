use crate::app::{
    models::{AppStateStorage, AppToRun, LaunchTarget},
    shared::ids::{GroupId, RuleId},
};
use os_api::PriorityClass;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleInput {
    pub name: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub aumid: Option<String>,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
    #[serde(default)]
    pub steam_executable: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub tracked_process_names: Vec<String>,
    #[serde(default)]
    pub manage_descendants: bool,
    #[serde(default)]
    pub autorun: bool,
    #[serde(default = "normal_priority")]
    pub priority: PriorityClass,
}
fn normal_priority() -> PriorityClass {
    PriorityClass::Normal
}
impl RuleInput {
    pub fn into_app(self) -> Result<AppToRun, String> {
        if self.priority == PriorityClass::Realtime {
            return Err("Realtime priority is not supported for agent-managed rule changes".into());
        }
        let launch_target = match (self.path, self.aumid, self.steam_app_id) {
            (Some(path), None, None) if self.steam_executable.is_none() => LaunchTarget::Path {
                dropped_path: path.clone(),
                bin_path: path,
            },
            (None, Some(aumid), None) if self.steam_executable.is_none() => {
                LaunchTarget::Installed { aumid }
            }
            (None, None, Some(app_id)) => LaunchTarget::Steam {
                app_id,
                executable: self.steam_executable,
            },
            _ => {
                return Err(
                    "Specify exactly one executable path, installed app AUMID or steam_app_id"
                        .into(),
                )
            }
        };
        let app = AppToRun {
            name: self.name.trim().into(),
            launch_target,
            args: self.args,
            additional_processes: self.tracked_process_names,
            builtin_process_binding: None,
            manage_descendants: self.manage_descendants,
            autorun: self.autorun,
            priority: self.priority,
        };
        validate_app(&app)?;
        Ok(app)
    }
}
pub(crate) fn validate_app(app: &AppToRun) -> Result<(), String> {
    validate_name(&app.name)?;
    if app.args.len() > 128
        || app
            .args
            .iter()
            .any(|arg| arg.contains('\0') || arg.len() > 4096)
        || app
            .args
            .iter()
            .map(|arg| arg.encode_utf16().count() + 3)
            .sum::<usize>()
            > 16000
    {
        return Err("Arguments exceed the launch budget or contain NUL".into());
    }
    match &app.launch_target {
        LaunchTarget::Steam { app_id, executable } => {
            if *app_id == 0 || !app.args.is_empty() || executable.as_ref().is_some_and(|p| !p.is_absolute() || p.as_os_str().len()>16000 || p.to_string_lossy().contains('\0') || !os_api::steam::eligible_image(p)) {
                return Err("Steam requires a nonzero AppID, no extra arguments, and an optional absolute game .exe path".into());
            }
        }
        LaunchTarget::Path { bin_path, .. } if !bin_path.is_absolute() || bin_path.to_string_lossy().contains('\0') || !bin_path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) || bin_path.as_os_str().len() > 16000 => return Err("An absolute .exe path is required; shell command strings and shortcuts are not executable targets".into()),
        LaunchTarget::Installed { aumid } if aumid.trim().is_empty() || aumid.len() > 512 || aumid.contains('\0') || !app.args.is_empty() => return Err("A valid AUMID without command-line arguments is required".into()),
        _ => {}
    }
    if app.additional_processes.len() > 64
        || app.additional_processes.iter().any(|name| {
            name.trim().is_empty() || name.len() > 256 || name.contains(['\0', '/', '\\', ':'])
        })
    {
        return Err(
            "Tracked process names must be basenames; at most 64 names of 256 bytes".into(),
        );
    }
    Ok(())
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 256 || name.contains('\0') {
        return Err("Name must contain 1–256 bytes and no NUL".into());
    }
    Ok(())
}
pub(crate) fn validate_group(
    name: &str,
    cpus: &[usize],
    total: usize,
) -> Result<(String, Vec<usize>), String> {
    super::service::validate_group_values(name, cpus, total)
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct PreferencePatch {
    pub theme_index: Option<usize>,
    pub process_monitoring_enabled: Option<bool>,
    pub cpu_load_monitoring_enabled: Option<bool>,
    pub cpu_load_history_seconds: Option<u32>,
    pub start_minimized: Option<bool>,
    pub windows_event_log_diagnostics_enabled: Option<bool>,
}
impl PreferencePatch {
    pub fn apply(&self, state: &mut AppStateStorage) -> Result<(), String> {
        if self.theme_index.is_some_and(|theme| theme > 2)
            || self
                .cpu_load_history_seconds
                .is_some_and(|seconds| ![30, 300, 1800].contains(&seconds))
        {
            return Err("Unsupported theme or history duration".into());
        }
        macro_rules! update { ($($field:ident),*) => { $(if let Some(value) = self.$field { state.$field = value; })* }; }
        update!(
            theme_index,
            process_monitoring_enabled,
            cpu_load_monitoring_enabled,
            cpu_load_history_seconds,
            start_minimized,
            windows_event_log_diagnostics_enabled
        );
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub(crate) enum BaseCommand {
    CreateGroup {
        name: String,
        cpus: Vec<usize>,
        run_all: bool,
    },
    UpdateGroup {
        group: GroupId,
        name: String,
        cpus: Vec<usize>,
        run_all: bool,
        hidden: Option<bool>,
    },
    RemoveGroup {
        group: GroupId,
    },
    MoveGroup {
        group: GroupId,
        index: usize,
    },
    CreateRule {
        group: GroupId,
        app: Box<AppToRun>,
    },
    UpdateRule {
        group: GroupId,
        rule: RuleId,
        app: Box<AppToRun>,
    },
    RemoveRule {
        group: GroupId,
        rule: RuleId,
    },
    MoveRule {
        group: GroupId,
        rule: RuleId,
        target: GroupId,
        index: usize,
    },
    Preferences(PreferencePatch),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_rule_input_cannot_persist_realtime_priority_for_later_enforcement() {
        let input: RuleInput = serde_json::from_value(serde_json::json!({"name":"Example","path":"C:\\Tools\\example.exe","priority":"Realtime"})).unwrap();
        let mut ordinary = input.clone();
        ordinary.priority = PriorityClass::Normal;
        let mut manual = ordinary.into_app().unwrap();
        manual.priority = PriorityClass::Realtime;
        assert!(
            validate_app(&manual).is_ok(),
            "Existing manual configurations remain supported"
        );
        assert!(input.into_app().is_err());
    }
    #[test]
    fn steam_rule_input_preserves_exact_game_path_and_refuses_mixed_targets() {
        let value = serde_json::json!({"name":"Game","steam_app_id":570,"steam_executable":"C:\\Games\\Game\\game.exe","tracked_process_names":["helper.exe"]});
        let app = serde_json::from_value::<RuleInput>(value.clone())
            .unwrap()
            .into_app()
            .unwrap();
        assert_eq!(app.steam_game().unwrap().0, 570);
        assert_eq!(app.additional_processes, vec!["helper.exe"]);
        for extra in [
            serde_json::json!({"path":"C:\\game.exe"}),
            serde_json::json!({"aumid":"Package!App"}),
            serde_json::json!({"args":["extra"]}),
            serde_json::json!({"steam_executable":"C:\\Steam\\steam.exe"}),
            serde_json::json!({"steam_app_id":0}),
        ] {
            let mut bad = value.clone();
            bad.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(serde_json::from_value::<RuleInput>(bad)
                .unwrap()
                .into_app()
                .is_err());
        }
    }
    #[test]
    fn group_commands_validate_the_mask_and_trim_the_name() {
        assert_eq!(
            validate_group("  Games  ", &[3, 1, 3], 4).unwrap(),
            ("Games".into(), vec![1, 3])
        );
        assert!(validate_group("Games", &[], 4).is_err());
        assert!(validate_group("Games", &[4], 4).is_err());
        assert!(validate_group("  ", &[0], 4).is_err());
    }
    #[test]
    fn rule_input_preserves_empty_tracking_and_rejects_installed_arguments() {
        let input: RuleInput = serde_json::from_value(serde_json::json!({"name":"Example","path":"C:\\Tools\\example.exe","tracked_process_names":[]})).unwrap();
        let app = input.into_app().unwrap();
        assert!(app.additional_processes.is_empty());
        assert!(!app.manage_descendants);
        let input: RuleInput = serde_json::from_value(
            serde_json::json!({"name":"Example","aumid":"Package!App","args":["bad"]}),
        )
        .unwrap();
        assert!(input.into_app().is_err());
    }
    #[test]
    fn arbitrary_shell_strings_and_relative_targets_are_not_executable_paths() {
        for path in [
            "example.exe",
            "cmd /c example",
            "C:\\Tools\\example.lnk",
            "C:\\bad\u{0}.exe",
        ] {
            let input: RuleInput =
                serde_json::from_value(serde_json::json!({"name":"Example","path":path})).unwrap();
            assert!(input.into_app().is_err());
        }
    }
}
