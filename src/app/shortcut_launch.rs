#![allow(dead_code)]

use crate::app::shared::ids::{GroupId, RuleId};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedRuleShortcutRequest {
    pub executable_path: PathBuf,
    pub app_name: String,
    pub group_name: String,
    pub group_id: GroupId,
    pub rule_id: RuleId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedRuleShortcutSpec {
    pub target_path: PathBuf,
    pub arguments: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutBuildError {
    InvalidGroupId(String),
    InvalidRuleId(String),
}

pub fn build_saved_rule_shortcut(
    request: SavedRuleShortcutRequest,
) -> Result<SavedRuleShortcutSpec, ShortcutBuildError> {
    if !crate::app::startup::is_cli_safe_id(&request.group_id.0) {
        return Err(ShortcutBuildError::InvalidGroupId(request.group_id.0));
    }
    if !crate::app::startup::is_cli_safe_id(&request.rule_id.0) {
        return Err(ShortcutBuildError::InvalidRuleId(request.rule_id.0));
    }

    let display_name = build_display_name(&request.app_name, &request.group_name);
    let working_dir = request.executable_path.parent().map(PathBuf::from);
    let arguments = vec![
        "--run-rule".to_string(),
        request.group_id.0,
        request.rule_id.0,
    ];

    Ok(SavedRuleShortcutSpec {
        target_path: request.executable_path,
        arguments,
        working_dir,
        display_name,
    })
}

fn build_display_name(app_name: &str, group_name: &str) -> String {
    let app_name = sanitize_filename_segment(app_name);
    let group_name = sanitize_filename_segment(group_name);

    let display_name = match (app_name.is_empty(), group_name.is_empty()) {
        (false, false) => format!("{app_name} - {group_name}"),
        (false, true) => app_name,
        (true, false) => group_name,
        (true, true) => "CPU Affinity Rule".to_string(),
    };

    protect_windows_filename(display_name)
}

fn protect_windows_filename(value: String) -> String {
    let value = value.trim_end_matches([' ', '.']).to_string();
    if value.is_empty() {
        return "CPU Affinity Rule".to_string();
    }

    let device_name = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let is_reserved = matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || device_name.strip_prefix("COM").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || device_name.strip_prefix("LPT").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        });

    let value = if is_reserved {
        format!("CPU Affinity Rule - {value}")
    } else {
        value
    };

    const MAX_DISPLAY_NAME_CHARS: usize = 120;
    let shortened = value
        .chars()
        .take(MAX_DISPLAY_NAME_CHARS)
        .collect::<String>();
    let shortened = shortened.trim_end_matches([' ', '.']).to_string();
    if shortened.is_empty() {
        "CPU Affinity Rule".to_string()
    } else {
        shortened
    }
}

fn sanitize_filename_segment(value: &str) -> String {
    let replaced = value
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>();

    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> SavedRuleShortcutRequest {
        SavedRuleShortcutRequest {
            executable_path: PathBuf::from("tool-dir").join("cpu-affinity-tool.exe"),
            app_name: "Ghost Recon".to_string(),
            group_name: "Performance Cores".to_string(),
            group_id: GroupId("group-12".to_string()),
            rule_id: RuleId("rule-34".to_string()),
        }
    }

    #[test]
    fn test_build_saved_rule_shortcut_uses_exact_ids_and_exe_dir() {
        let spec = build_saved_rule_shortcut(request()).unwrap();

        assert_eq!(
            spec.target_path,
            PathBuf::from("tool-dir").join("cpu-affinity-tool.exe")
        );
        assert_eq!(
            spec.arguments,
            vec![
                "--run-rule".to_string(),
                "group-12".to_string(),
                "rule-34".to_string()
            ]
        );
        assert_eq!(spec.working_dir, Some(PathBuf::from("tool-dir")));
        assert_eq!(spec.display_name, "Ghost Recon - Performance Cores");
    }

    #[test]
    fn test_build_saved_rule_shortcut_rejects_invalid_ids() {
        let mut invalid_group = request();
        invalid_group.group_id = GroupId("group 12".to_string());
        assert_eq!(
            build_saved_rule_shortcut(invalid_group),
            Err(ShortcutBuildError::InvalidGroupId("group 12".to_string()))
        );

        let mut invalid_rule = request();
        invalid_rule.rule_id = RuleId("rule/34".to_string());
        assert_eq!(
            build_saved_rule_shortcut(invalid_rule),
            Err(ShortcutBuildError::InvalidRuleId("rule/34".to_string()))
        );
    }

    #[test]
    fn test_shortcut_display_name_sanitizes_windows_filename_chars() {
        let mut request = request();
        request.app_name = r#"Bad:Game*?""#.to_string();
        request.group_name = r#"P/Cores\Fast|Now<>"#.to_string();

        let spec = build_saved_rule_shortcut(request).unwrap();

        assert_eq!(spec.display_name, "Bad Game - P Cores Fast Now");
        assert!(!spec
            .display_name
            .chars()
            .any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')));
    }

    #[test]
    fn test_shortcut_display_name_uses_fallback_for_empty_names() {
        let mut request = request();
        request.app_name = " :* ".to_string();
        request.group_name = " /? ".to_string();

        let spec = build_saved_rule_shortcut(request).unwrap();

        assert_eq!(spec.display_name, "CPU Affinity Rule");
    }

    #[test]
    fn test_shortcut_display_name_avoids_reserved_windows_names() {
        let mut request = request();
        request.app_name = "CON".to_string();
        request.group_name.clear();

        let spec = build_saved_rule_shortcut(request).unwrap();

        assert_eq!(spec.display_name, "CPU Affinity Rule - CON");
    }

    #[test]
    fn test_shortcut_display_name_trims_trailing_dots_and_limits_length() {
        let mut request = request();
        request.app_name = format!("{}...", "A".repeat(150));
        request.group_name.clear();

        let spec = build_saved_rule_shortcut(request).unwrap();

        assert_eq!(spec.display_name.len(), 120);
        assert!(!spec.display_name.ends_with('.'));
        assert!(spec.display_name.chars().all(|ch| ch == 'A'));
    }
}
