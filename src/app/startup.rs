use crate::app::shared::ids::{GroupId, RuleId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupIntent {
    NormalGui,
    RunRule { group_id: GroupId, rule_id: RuleId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupIntentError {
    UnknownArgument(String),
    MissingRunRuleGroupId,
    MissingRunRuleRuleId,
    TooManyArguments,
    InvalidGroupId(String),
    InvalidRuleId(String),
}

const RUN_RULE_FLAG: &str = "--run-rule";
const MAX_ID_LEN: usize = 128;

pub fn parse_startup_args(args: &[String]) -> Result<StartupIntent, StartupIntentError> {
    match args {
        [] => Ok(StartupIntent::NormalGui),
        [flag, rest @ ..] if flag == RUN_RULE_FLAG => parse_run_rule_args(rest),
        [arg, ..] => Err(StartupIntentError::UnknownArgument(arg.clone())),
    }
}

fn parse_run_rule_args(args: &[String]) -> Result<StartupIntent, StartupIntentError> {
    let group_id = args
        .first()
        .ok_or(StartupIntentError::MissingRunRuleGroupId)?;
    let rule_id = args
        .get(1)
        .ok_or(StartupIntentError::MissingRunRuleRuleId)?;

    if args.len() > 2 {
        return Err(StartupIntentError::TooManyArguments);
    }
    if !is_cli_safe_id(group_id) {
        return Err(StartupIntentError::InvalidGroupId(group_id.clone()));
    }
    if !is_cli_safe_id(rule_id) {
        return Err(StartupIntentError::InvalidRuleId(rule_id.clone()));
    }

    Ok(StartupIntent::RunRule {
        group_id: GroupId(group_id.clone()),
        rule_id: RuleId(rule_id.clone()),
    })
}

pub(crate) fn is_cli_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn test_empty_args_start_normal_gui() {
        assert_eq!(parse_startup_args(&args(&[])), Ok(StartupIntent::NormalGui));
    }

    #[test]
    fn test_run_rule_args_parse_saved_rule_ids() {
        assert_eq!(
            parse_startup_args(&args(&["--run-rule", "group-12", "rule-34"])),
            Ok(StartupIntent::RunRule {
                group_id: GroupId("group-12".to_string()),
                rule_id: RuleId("rule-34".to_string()),
            })
        );
    }

    #[test]
    fn test_unknown_argument_is_rejected() {
        assert_eq!(
            parse_startup_args(&args(&["--binarypath", r"C:\Games\game.exe"])),
            Err(StartupIntentError::UnknownArgument(
                "--binarypath".to_string()
            ))
        );
    }

    #[test]
    fn test_run_rule_requires_exact_two_ids() {
        assert_eq!(
            parse_startup_args(&args(&["--run-rule"])),
            Err(StartupIntentError::MissingRunRuleGroupId)
        );
        assert_eq!(
            parse_startup_args(&args(&["--run-rule", "group-1"])),
            Err(StartupIntentError::MissingRunRuleRuleId)
        );
        assert_eq!(
            parse_startup_args(&args(&["--run-rule", "group-1", "rule-1", "extra"])),
            Err(StartupIntentError::TooManyArguments)
        );
    }

    #[test]
    fn test_run_rule_ids_use_cli_safe_grammar() {
        assert_eq!(
            parse_startup_args(&args(&["--run-rule", "group 1", "rule-1"])),
            Err(StartupIntentError::InvalidGroupId("group 1".to_string()))
        );
        assert_eq!(
            parse_startup_args(&args(&["--run-rule", "group-1", "rule/1"])),
            Err(StartupIntentError::InvalidRuleId("rule/1".to_string()))
        );
        assert_eq!(
            parse_startup_args(&args(&["--run-rule", "", "rule-1"])),
            Err(StartupIntentError::InvalidGroupId(String::new()))
        );
    }

    #[test]
    fn test_run_rule_ids_are_length_limited() {
        let oversized_group_id = "g".repeat(129);
        let oversized_rule_id = "r".repeat(129);

        assert_eq!(
            parse_startup_args(&vec![
                "--run-rule".to_string(),
                oversized_group_id.clone(),
                "rule-1".to_string()
            ]),
            Err(StartupIntentError::InvalidGroupId(oversized_group_id))
        );
        assert_eq!(
            parse_startup_args(&vec![
                "--run-rule".to_string(),
                "group-1".to_string(),
                oversized_rule_id.clone()
            ]),
            Err(StartupIntentError::InvalidRuleId(oversized_rule_id))
        );
    }
}
