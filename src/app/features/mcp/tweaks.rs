use super::{
    operations::{ApiError, ApiResult},
    policy::AgentAccess,
    server::LiveAccess,
    services::{decode, save_error, Services},
};
use crate::app::features::system_tweaks::{
    config::{Association, ModulePreference, Profile, TuningLoad},
    platform::{
        codec::ModuleEnvelope,
        contract::{ActivationEligibility, FeatureId, ProfileId},
    },
    RequestFence, TuningRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    request_key: String,
    expected_revision: String,
    feature_id: Option<FeatureId>,
    config: Option<ModuleEnvelope>,
    profile_id: Option<String>,
    name: Option<String>,
    preference: Option<ModulePreference>,
    expected_preference: Option<ModulePreference>,
    group_id: Option<String>,
    rule_id: Option<String>,
    eligibility: Option<ActivationEligibility>,
    target_id: Option<String>,
    action: Option<String>,
}
fn required<T>(value: Option<T>, name: &str) -> Result<T, ApiError> {
    value.ok_or_else(|| ApiError::invalid(format!("{name} is required")))
}
fn profile(value: Option<String>) -> Result<ProfileId, ApiError> {
    ProfileId::new(
        required(value, "profile_id")?
            .parse()
            .map_err(|_| ApiError::invalid("Invalid profile_id"))?,
    )
    .map_err(|_| ApiError::invalid("profile_id must be nonzero"))
}
fn snapshot_json(s: &crate::app::features::system_tweaks::TuningSnapshot) -> Value {
    json!({"configuration_revision":s.revision.to_string(),"configuration":format!("{:?}",s.configuration),"operation":format!("{:?}",s.operation),"has_changes":s.has_tweak_changes,"operation_error":s.operation_error.as_ref().map(|e|json!({"kind":format!("{:?}",e.kind),"operation":e.operation,"detail":e.detail(),"native":e.native.as_ref().map(|n|json!({"domain":n.domain,"code":n.code}))})),"display_confirmation_remaining_ms":s.display_confirmation.map(|deadline|deadline.saturating_duration_since(std::time::Instant::now()).as_millis()),"cards":s.tweak_cards.iter().map(|card|json!({"feature_id":card.feature.as_str(),"title":card.spec.title,"ready":card.ready,"observed":card.observed.as_ref().map(|o|json!({"summary":o.summary,"effect":o.effect})),"baseline":card.baseline,"owned":card.owned,"capability":format!("{:?}",card.capability),"status":format!("{:?}",card.status),"configuration":card.config})).collect::<Vec<_>>(),"pc_checks":s.pc_checks.iter().map(|check|json!({"id":check.id,"title":check.title,"summary":check.summary})).collect::<Vec<_>>(),"freshness":"Cached worker snapshot. Use tweak_refresh or tweak_pc_checks_refresh for explicit read-only native refresh; no API polling collector is added."})
}
fn profile_row(profile: &Profile) -> ApiResult {
    #[derive(serde::Serialize)]
    struct Row<'a> {
        version: u32,
        id: String,
        name: &'a str,
        modules: &'a [ModulePreference],
    }
    super::services::bounded_row(&Row {
        version: profile.version,
        id: profile.id.get().to_string(),
        name: &profile.name,
        modules: &profile.modules,
    })
}
fn association_row(association: &Association) -> ApiResult {
    #[derive(serde::Serialize)]
    struct Row<'a> {
        version: u32,
        group: &'a crate::app::shared::ids::GroupId,
        rule: &'a crate::app::shared::ids::RuleId,
        profile: String,
        eligibility: ActivationEligibility,
    }
    super::services::bounded_row(&Row {
        version: association.version,
        group: &association.group,
        rule: &association.rule,
        profile: association.profile.get().to_string(),
        eligibility: association.eligibility,
    })
}

impl Services {
    pub(super) fn read_tweaks(&self) -> Value {
        snapshot_json(&self.tuning.snapshot())
    }
    pub(super) fn tweak_configuration_read(&self, name: &str, args: Value) -> ApiResult {
        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct ReadArgs {
            feature_id: Option<FeatureId>,
            offset: usize,
            limit: Option<usize>,
            revision: Option<String>,
        }
        let a: ReadArgs = decode(args)?;
        let s = self.tuning.snapshot();
        if a.offset > 100000 || a.limit.is_some_and(|n| n == 0 || n > 256) {
            return Err(ApiError::invalid("Invalid page"));
        }
        if a.revision
            .is_some_and(|revision| revision != s.revision.to_string())
        {
            return Err(ApiError::new("revision_conflict", "Configuration changed"));
        }
        if name == "read_tweak_module" {
            let feature = required(a.feature_id, "feature_id")?;
            let module = self
                .registry
                .get(&feature)
                .ok_or_else(|| ApiError::invalid("Unknown module"))?;
            let selected = s
                .tweak_cards
                .iter()
                .find(|card| card.feature == feature)
                .and_then(|card| card.target.clone());
            let targets = crate::app::features::system_tweaks::service::manual_target_candidates(
                module.as_ref(),
                selected,
            );
            let descriptor = module.descriptor();
            return super::services::bounded_row(&json!({"feature_id":feature,
                "configuration_template":module.editor().default_config(),
                "help":module.editor().tweak().map(|spec|spec.help),
                "selects_saved_executable":module.editor().selects_saved_exe(),
                "activation_modes":descriptor.activation_modes,
                "manual_targets":targets.iter().skip(a.offset).take(a.limit.unwrap_or(30)).map(|target|json!({
                    "target_id":crate::app::features::system_tweaks::service::manual_target_id(target),
                    "summary":module.observe(target).summary.summary})).collect::<Vec<_>>(),
                "total_targets":targets.len(),
                "next_offset":(a.offset+a.limit.unwrap_or(30)<targets.len()).then_some(a.offset+a.limit.unwrap_or(30))}));
        }
        let config = match s.state.tuning.decode() {
            TuningLoad::Known(config) => config,
            TuningLoad::Off => Box::default(),
            _ => {
                return Err(ApiError::new(
                    "unsupported_configuration",
                    "Unknown or blocked tuning data is preserved without interpretation",
                ))
            }
        };
        let (total, rows) = if name == "list_tweak_profiles" {
            (
                config.profiles.len(),
                config
                    .profiles
                    .iter()
                    .skip(a.offset)
                    .take(a.limit.unwrap_or(30))
                    .map(profile_row)
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else {
            (
                config.associations.len(),
                config
                    .associations
                    .iter()
                    .skip(a.offset)
                    .take(a.limit.unwrap_or(50))
                    .map(association_row)
                    .collect::<Result<Vec<_>, _>>()?,
            )
        };
        let mut bytes = 0;
        let rows: Vec<_> = rows
            .into_iter()
            .take_while(|row| {
                bytes += row.to_string().len();
                bytes <= 48 * 1024
            })
            .collect();
        let next = a.offset + rows.len();
        Ok(
            json!({"configuration_revision":s.revision.to_string(),"rows":rows,"total":total,"next_offset":(next<total).then_some(next)}),
        )
    }
    pub(super) async fn tweak(
        &self,
        name: &str,
        args: Value,
        agent: &AgentAccess,
        live: &Arc<LiveAccess>,
        generation: u64,
    ) -> ApiResult {
        let a: Args = decode(args)?;
        let _key = a.request_key;
        let revision = a
            .expected_revision
            .parse::<u64>()
            .map_err(|_| ApiError::invalid("Invalid expected_revision"))?;
        let snapshot = self.tuning.snapshot();
        if snapshot.revision != revision {
            return Err(ApiError::new("revision_conflict", "Configuration changed"));
        }
        let command = match name {
            "tweak_refresh" => TuningRequest::RefreshModuleCards,
            "tweak_pc_checks_refresh" => TuningRequest::RefreshPcChecks,
            "tweak_select_app" => {
                let feature = required(a.feature_id, "feature_id")?;
                let group = crate::app::shared::ids::GroupId(required(a.group_id, "group_id")?);
                let rule = crate::app::shared::ids::RuleId(required(a.rule_id, "rule_id")?);
                let g = snapshot
                    .rules
                    .group_index_for_id(&group)
                    .ok_or_else(|| ApiError::invalid("Unknown group"))?;
                let r = snapshot
                    .rules
                    .rule_index_for_id(g, &rule)
                    .ok_or_else(|| ApiError::invalid("Unknown rule"))?;
                let path = snapshot.state.groups[g].programs[r]
                    .bin_path()
                    .ok_or_else(|| {
                        ApiError::invalid("This module needs a saved executable target")
                    })?;
                let target = self
                    .registry
                    .get(&feature)
                    .and_then(|module| module.editor().tweak_saved_executable_target(path))
                    .ok_or_else(|| {
                        ApiError::invalid("Module does not accept a saved executable")
                    })?;
                TuningRequest::SelectTweakTarget { feature, target }
            }
            "tweak_profile_create" => TuningRequest::CreateProfile {
                name: required(a.name, "name")?,
            },
            "tweak_profile_set_module" => TuningRequest::SetModulePreference {
                profile: profile(a.profile_id)?,
                preference: Box::new(required(a.preference, "preference")?),
                expected: a.expected_preference,
            },
            "tweak_profile_disable" => TuningRequest::DisableProfile {
                profile: profile(a.profile_id)?,
            },
            "tweak_profile_delete" => TuningRequest::RemoveProfile {
                profile: profile(a.profile_id)?,
            },
            "tweak_bind_rule" => TuningRequest::BindRule {
                group: crate::app::shared::ids::GroupId(required(a.group_id, "group_id")?),
                rule: crate::app::shared::ids::RuleId(required(a.rule_id, "rule_id")?),
                profile: profile(a.profile_id)?,
                eligibility: required(a.eligibility, "eligibility")?,
            },
            "tweak_unbind_rule" => TuningRequest::UnbindRule {
                group: crate::app::shared::ids::GroupId(required(a.group_id, "group_id")?),
                rule: crate::app::shared::ids::RuleId(required(a.rule_id, "rule_id")?),
            },
            "tweak_activate" => {
                let profile = profile(a.profile_id)?;
                let feature = required(a.feature_id, "feature_id")?;
                TuningRequest::ActivateSelectedManual {
                    profile,
                    feature,
                    target_id: required(a.target_id, "target_id")?,
                }
            }
            "tweak_action" => {
                let action=match required(a.action,"action")?.as_str(){"refresh"=>crate::app::features::system_tweaks::presentation::ModuleAction::Refresh,"open_settings"=>crate::app::features::system_tweaks::presentation::ModuleAction::OpenSettings,_=>return Err(ApiError::invalid("Unsupported module action"))};
                TuningRequest::ModuleAction {
                    feature: required(a.feature_id, "feature_id")?,
                    action,
                }
            }
            "tweak_apply" => TuningRequest::ApplyCurrentTweak {
                feature: required(a.feature_id, "feature_id")?,
                config: a.config,
            },
            "tweak_undo" => TuningRequest::UndoTweak {
                feature: a
                    .feature_id
                    .ok_or_else(|| ApiError::invalid("feature_id is required"))?,
            },
            "tweak_undo_all" => TuningRequest::UndoAllTweaks,
            "confirm_display" => {
                if snapshot.display_confirmation.is_none() {
                    return Err(ApiError::new(
                        "not_pending",
                        "No display confirmation is pending",
                    ));
                }
                TuningRequest::ConfirmDisplayTweak
            }
            _ => return Err(ApiError::new("unknown_tool", "Unknown tweak operation")),
        };
        if !live.current(agent, generation) {
            return Err(ApiError::new(
                "revoked",
                "Access revoked before tweak admission",
            ));
        }
        let id = self
            .tuning
            .request_guarded(
                command,
                Some(revision),
                Some(RequestFence {
                    generation: live.generation.clone(),
                    expected: generation,
                }),
            )
            .map_err(save_error)?;
        self.wake.request_repaint();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(receipt) = self.tuning.receipt(id) {
                receipt.result.map_err(save_error)?;
                self.wake.request_repaint();
                return Ok(
                    json!({"configuration_revision":receipt.revision.to_string(),"request_completed":true,"module_result":self.read_tweaks(),"note":"Module state and ownership are authoritative; configuration persistence is not proof of native apply"}),
                );
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ApiError::new(
                    "indeterminate",
                    "Tweak operation is unresolved; inspect module status before retrying",
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_and_association_ids_are_lossless_decimal_strings() {
        let id = ProfileId::new((1u64 << 60) + 1).unwrap();
        let p = Profile {
            version: 1,
            id,
            name: "Large ID".into(),
            modules: vec![],
        };
        let a = Association {
            version: 1,
            group: crate::app::shared::ids::GroupId("g".into()),
            rule: crate::app::shared::ids::RuleId("r".into()),
            profile: id,
            eligibility: ActivationEligibility::Running,
        };
        assert_eq!(profile_row(&p).unwrap()["id"], id.get().to_string());
        assert_eq!(
            association_row(&a).unwrap()["profile"],
            id.get().to_string()
        );
    }
    #[test]
    fn tweak_snapshot_exposes_native_failure_instead_of_only_queue_state() {
        let app = crate::app::runtime::AppState::new_for_test(
            Arc::new(std::sync::RwLock::new(
                crate::app::models::AppStateStorage::test_fixture(|_| {}),
            )),
            4,
        );
        let mut snapshot = (*app.tuning_snapshot()).clone();
        snapshot.operation_error = Some(
            crate::app::features::system_tweaks::platform::contract::ModuleError::new(
                crate::app::features::system_tweaks::platform::contract::ErrorKind::Denied,
                "write",
            )
            .with_detail("Native write denied")
            .unwrap(),
        );
        let result = snapshot_json(&snapshot);
        assert_eq!(result["operation_error"]["kind"], "Denied");
        assert_eq!(result["operation_error"]["detail"], "Native write denied");
    }
}
