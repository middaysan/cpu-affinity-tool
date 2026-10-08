//! Immutable independently saved input; it is not composed into the runner yet.
use super::platform::configured_intent::{ConfiguredIntentSource, ConfiguredSnapshot};

pub(crate) struct SavedConfiguredSource {
    snapshot: ConfiguredSnapshot,
    // Keep sealed rejected input as well; Unavailable never discards its native
    // saved evidence or substitutes caller-owned configuration entries.
    #[cfg(windows)]
    proof: Option<super::platform::configured_intent::NativeConfigurationProof>,
}
impl ConfiguredIntentSource for SavedConfiguredSource {
    fn snapshot(&self) -> ConfiguredSnapshot {
        self.snapshot.clone()
    }
}
impl SavedConfiguredSource {
    #[cfg(windows)]
    pub(crate) fn from_native(
        proof: super::platform::configured_intent::NativeConfigurationProof,
        catalog: &super::platform::catalog::Catalog,
    ) -> Self {
        Self {
            snapshot: ConfiguredSnapshot::from_saved_native(proof.clone(), catalog),
            proof: Some(proof),
        }
    }
    #[cfg(test)]
    pub(in crate::app::features::tuning) fn from_fixture(
        bytes: &[u8],
        origin: super::platform::contract::ConfigurationOrigin,
        catalog: &super::platform::catalog::Catalog,
    ) -> Self {
        Self {
            snapshot: ConfiguredSnapshot::from_saved_fixture(bytes, origin, catalog),
            #[cfg(windows)]
            proof: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::features::tuning::{
        config::{TuningEnvelope, TuningLoad},
        platform::{catalog::Catalog, configured_intent::IntentDecision, contract::*},
        testing::scalar::{ScalarConfig, ScalarModule},
    };
    use serde_json::{json, Value};
    use std::{
        fs::OpenOptions,
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct SavedFile(PathBuf);
    impl SavedFile {
        fn new(tuning: &Value) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = PathBuf::from("target").join(format!(
                "p2a-saved-fixture-{}-{}.json",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let state = crate::app::models::AppStateStorage::test_fixture(|_| {});
            let mut value = serde_json::to_value(state).unwrap();
            value["tuning"] = tuning.clone();
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(&serde_json::to_vec(&value).unwrap())
                .unwrap();
            file.sync_all().unwrap();
            Self(path)
        }
        fn bytes(&self) -> Vec<u8> {
            std::fs::read(&self.0).unwrap()
        }
    }
    impl Drop for SavedFile {
        fn drop(&mut self) {
            std::fs::remove_file(&self.0).unwrap();
        }
    }
    fn setup() -> (ScalarModule, Catalog) {
        let module = ScalarModule::new(7);
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        (module, catalog)
    }
    fn intent(module: &ScalarModule, owner: u64, generation: u64, value: i32) -> Value {
        json!({"version":1,"owner":owner,"binding":ActivationBinding::Manual(ConfiguredIntent {profile:ProfileId::new(1).unwrap(),generation:ConfigGeneration::new(generation).unwrap()}),"config":module.config_envelope(&ScalarConfig {value}).unwrap(),"target":TargetIdentity::Machine(MachineIdentity{id:MachineId::new([1;16]).unwrap()}),"mode":"Manual"})
    }
    fn tuning(module: &ScalarModule, version: u32) -> Value {
        json!({"version":version,"lineage":vec![1;32],"profiles":[{"version":1,"id":1,"name":"P","modules":[{"feature":module.config_envelope(&ScalarConfig {value:12}).unwrap().feature,"enabled":true,"mode":"Manual","config":module.config_envelope(&ScalarConfig {value:12}).unwrap()}]}],"associations":[],"allocations":{"next_profile":2,"next_owner":2,"next_config_generation":2},"activations":[intent(module,1,1,12)],"withdrawals":[]})
    }
    fn query(snapshot: &ConfiguredSnapshot, value: &Value, continuation: bool) -> IntentDecision {
        let e: super::super::config::SavedIntent = serde_json::from_value(value.clone()).unwrap();
        if continuation {
            snapshot.continuation_for_origin(
                &ConfigurationOrigin::fixture(),
                e.owner,
                &e.binding,
                &e.config,
                &e.target,
                e.mode,
            )
        } else {
            snapshot.decision_for_origin(
                &ConfigurationOrigin::fixture(),
                e.owner,
                &e.binding,
                &e.config,
                &e.target,
                e.mode,
            )
        }
    }
    fn source(value: &Value, catalog: &Catalog) -> SavedConfiguredSource {
        let saved = SavedFile::new(value);
        SavedConfiguredSource::from_fixture(&saved.bytes(), ConfigurationOrigin::fixture(), catalog)
    }
    fn transition(module: &ScalarModule) -> Value {
        let mut value = tuning(module, 2);
        value["activations"] = json!([]);
        value["profiles"][0]["modules"][0]["config"] =
            serde_json::to_value(module.config_envelope(&ScalarConfig { value: 13 }).unwrap())
                .unwrap();
        value["allocations"]["next_config_generation"] = json!(3);
        value["transitions"] = json!([{"version":1,"owner":1,"generation":2,"old":intent(module,1,1,12),"new":intent(module,1,2,13)}]);
        value
    }
    #[test]
    fn p2a_saved_v1_and_v2_request_and_continuation_roles_are_distinct() {
        let (module, catalog) = setup();
        let old = intent(&module, 1, 1, 12);
        let new = intent(&module, 1, 2, 13);
        let v1 = tuning(&module, 1);
        assert_eq!(
            query(&source(&v1, &catalog).snapshot(), &old, false),
            IntentDecision::Enabled
        );
        let v2 = transition(&module);
        let raw = serde_json::to_string(&v2).unwrap();
        let envelope: TuningEnvelope = serde_json::from_str(&raw).unwrap();
        let TuningLoad::Known(config) = envelope.decode() else {
            panic!("v2 is understood");
        };
        assert_eq!(serde_json::to_value(config).unwrap(), v2);
        assert_eq!(envelope.raw(), Some(raw.as_str()));
        let snapshot = source(&v2, &catalog).snapshot();
        assert_eq!(query(&snapshot, &old, false), IntentDecision::Unknown);
        assert_eq!(query(&snapshot, &old, true), IntentDecision::Enabled);
        assert_eq!(query(&snapshot, &new, false), IntentDecision::Enabled);
        assert_eq!(query(&snapshot, &new, true), IntentDecision::Unknown);
    }
    #[test]
    fn p2a_saved_withdrawal_is_exact_and_omission_is_unknown() {
        let (module, catalog) = setup();
        let old = intent(&module, 1, 1, 12);
        let mut value = tuning(&module, 1);
        value["profiles"] = json!([]);
        value["activations"] = json!([]);
        value["withdrawals"] = json!([old.clone()]);
        let snapshot = source(&value, &catalog).snapshot();
        assert_eq!(query(&snapshot, &old, false), IntentDecision::Disabled);
        assert_eq!(
            query(&snapshot, &intent(&module, 1, 1, 13), false),
            IntentDecision::Unknown
        );
        assert_eq!(query(&snapshot, &old, true), IntentDecision::Unknown);
        value["withdrawals"] = json!([]);
        assert_eq!(
            query(&source(&value, &catalog).snapshot(), &old, false),
            IntentDecision::Unknown
        );
    }
    #[test]
    fn p2a_saved_unknown_foreign_or_unsupported_input_is_unavailable() {
        let (module, catalog) = setup();
        let value = tuning(&module, 1);
        let saved = SavedFile::new(&value);
        assert_eq!(
            SavedConfiguredSource::from_fixture(
                &saved.bytes(),
                ConfigurationOrigin::fixture().fixture_changed("lineage"),
                &catalog
            )
            .snapshot(),
            ConfiguredSnapshot::Unavailable
        );
        for change in [json!(null), json!({"version":99}), {
            let mut v = value.clone();
            v["future"] = json!(true);
            v
        }] {
            assert_eq!(
                source(&change, &catalog).snapshot(),
                ConfiguredSnapshot::Unavailable
            );
        }
        assert_eq!(
            source(&value, &Catalog::default()).snapshot(),
            ConfiguredSnapshot::Unavailable
        );
        let snapshot = source(&value, &catalog).snapshot();
        let e: super::super::config::SavedIntent =
            serde_json::from_value(intent(&module, 1, 1, 12)).unwrap();
        assert_eq!(
            snapshot.decision_for_origin(
                &ConfigurationOrigin::fixture().fixture_changed("directory"),
                e.owner,
                &e.binding,
                &e.config,
                &e.target,
                e.mode
            ),
            IntentDecision::Unknown
        );
    }
    #[test]
    fn p2a_unsaved_draft_cannot_update_saved_source() {
        let (module, catalog) = setup();
        let mut value = tuning(&module, 1);
        let saved = SavedFile::new(&value);
        let producer = SavedConfiguredSource::from_fixture(
            &saved.bytes(),
            ConfigurationOrigin::fixture(),
            &catalog,
        );
        let captured = producer.snapshot();
        value["activations"][0] = intent(&module, 1, 2, 13);
        value["profiles"][0]["modules"][0]["config"] = value["activations"][0]["config"].clone();
        value["allocations"]["next_config_generation"] = json!(3);
        assert_eq!(producer.snapshot(), captured);
        assert_eq!(
            query(&captured, &intent(&module, 1, 1, 12), false),
            IntentDecision::Enabled
        );
        assert_eq!(
            query(&captured, &value["activations"][0], false),
            IntentDecision::Unknown
        );
        assert_eq!(saved.bytes(), SavedFile::new(&tuning(&module, 1)).bytes());
    }
    #[test]
    fn p2a_inactive_unknown_preferences_do_not_drop_or_block_known_owners() {
        let (module, catalog) = setup();
        let mut value = tuning(&module, 1);
        let mut inactive = value["profiles"][0]["modules"][0].clone();
        inactive["feature"] = json!("future_module");
        inactive["config"]["feature"] = json!("future_module");
        inactive["enabled"] = json!(false);
        value["profiles"][0]["modules"]
            .as_array_mut()
            .unwrap()
            .push(inactive.clone());
        assert_eq!(
            query(
                &source(&value, &catalog).snapshot(),
                &intent(&module, 1, 1, 12),
                false
            ),
            IntentDecision::Enabled
        );
        value["activations"][0]["config"] = inactive["config"].clone();
        assert_eq!(
            source(&value, &catalog).snapshot(),
            ConfiguredSnapshot::Unavailable
        );
    }
    #[test]
    fn p2a_v2_understood_roundtrip_keeps_explicit_transition_identity() {
        let (module, _) = setup();
        let value = transition(&module);
        let raw = serde_json::to_string(&value).unwrap();
        let envelope: TuningEnvelope = serde_json::from_str(&raw).unwrap();
        let TuningLoad::Known(config) = envelope.decode() else {
            panic!("known v2 shape");
        };
        assert_eq!(serde_json::to_value(config).unwrap(), value);
        assert_eq!(serde_json::to_string(&envelope).unwrap(), raw);
        let mut v1 = value;
        v1["version"] = json!(1);
        assert_eq!(
            serde_json::from_value::<TuningEnvelope>(v1)
                .unwrap()
                .decode(),
            TuningLoad::Unknown
        );
    }
    #[test]
    fn p2a_v2_refuses_owner_overlap_tuple_transfer_rewind_and_overflow() {
        let (module, _) = setup();
        let valid = transition(&module);
        let mut variants = Vec::new();
        for (field, value) in [("owner", json!(2)), ("generation", json!(1))] {
            let mut v = valid.clone();
            v["transitions"][0][field] = value;
            variants.push(v);
        }
        for (field, value) in [
            ("owner", json!(2)),
            ("target", json!({"Machine":{"id":vec![2;16]}})),
            ("mode", json!("Session")),
            ("binding", json!({"Manual":{"profile":2,"generation":2}})),
            ("binding", json!({"Manual":{"profile":1,"generation":1}})),
        ] {
            let mut v = valid.clone();
            v["transitions"][0]["new"][field] = value;
            variants.push(v);
        }
        let mut v = valid.clone();
        v["activations"] = json!([intent(&module, 1, 2, 13)]);
        variants.push(v);
        let mut v = valid.clone();
        v["withdrawals"] = json!([intent(&module, 1, 1, 12)]);
        variants.push(v);
        let mut v = valid.clone();
        v["transitions"]
            .as_array_mut()
            .unwrap()
            .push(valid["transitions"][0].clone());
        variants.push(v);
        let mut v = valid.clone();
        v["profiles"][0]["modules"][0]["config"]["payload"] = json!(12);
        variants.push(v);
        for field in ["next_profile", "next_owner", "next_config_generation"] {
            let mut v = valid.clone();
            v["allocations"][field] = json!(u64::MAX);
            variants.push(v);
        }
        let mut v = valid;
        v["allocations"]["next_config_generation"] = json!(2);
        variants.push(v);
        for (index, v) in variants.into_iter().enumerate() {
            assert_eq!(
                serde_json::from_value::<TuningEnvelope>(v)
                    .unwrap()
                    .decode(),
                TuningLoad::Blocked,
                "case {index}"
            );
        }
    }
    #[test]
    fn p2a_v2_aggregate_owner_capacity_includes_all_record_categories() {
        let (module, catalog) = setup();
        let mut value = transition(&module);
        value["transitions"]=json!((1..=256).map(|owner|json!({"version":1,"owner":owner,"generation":2,"old":intent(&module,owner,1,12),"new":intent(&module,owner,2,13)})).collect::<Vec<_>>());
        value["allocations"]["next_owner"] = json!(257);
        assert!(matches!(
            serde_json::from_value::<TuningEnvelope>(value.clone())
                .unwrap()
                .decode(),
            TuningLoad::Known(_)
        ));
        assert!(matches!(
            source(&value, &catalog).snapshot(),
            ConfiguredSnapshot::Complete(_)
        ));
        value["withdrawals"] = json!([intent(&module, 257, 1, 12)]);
        value["allocations"]["next_owner"] = json!(258);
        assert_eq!(
            serde_json::from_value::<TuningEnvelope>(value)
                .unwrap()
                .decode(),
            TuningLoad::Blocked
        );
    }
    #[test]
    fn p2a_unknown_module_shape_cannot_bypass_understood_transition_metadata() {
        let (module, _) = setup();
        let mut value = transition(&module);
        value["transitions"][0]["new"]["config"] = json!({"future_version":2,"payload":[]});
        value["transitions"][0]["new"]["binding"]["Manual"]["generation"] = json!(1);
        assert_eq!(
            serde_json::from_value::<TuningEnvelope>(value)
                .unwrap()
                .decode(),
            TuningLoad::Blocked
        );
        let mut value = transition(&module);
        value["transitions"][0]["old"]["config"] = json!({"future_version":2,"payload":[]});
        value["transitions"][0]["new"]["target"] = json!({"Machine":{"id":vec![2;16]}});
        assert_eq!(
            serde_json::from_value::<TuningEnvelope>(value)
                .unwrap()
                .decode(),
            TuningLoad::Blocked
        );
    }
    fn game(
        generation: u64,
        rule: &str,
        pid: u32,
        eligibility: ActivationEligibility,
    ) -> ActivationBinding {
        use crate::app::shared::ids::{GroupId, RuleId};
        let session = SessionIdentity {
            user: UserIdentity {
                machine: MachineIdentity {
                    id: MachineId::new([1; 16]).unwrap(),
                },
                sid: UserSid::new("S-1-5-21-1").unwrap(),
            },
            session_id: 1,
            logon: LogonId::new(1).unwrap(),
            boot: BootId::new([3; 16]).unwrap(),
        };
        let process = ProcessIdentity::new(
            session.clone(),
            pid,
            1,
            ImagePath::new("C:\\Fixture\\actor.exe").unwrap(),
        )
        .unwrap();
        ActivationBinding::Game(
            GameActivation::new(
                ConfiguredIntent {
                    profile: ProfileId::new(1).unwrap(),
                    generation: ConfigGeneration::new(generation).unwrap(),
                },
                RuleActivation::new(GroupId("g".into()), RuleId(rule.into())).unwrap(),
                eligibility,
                session,
                vec![process],
            )
            .unwrap(),
        )
    }
    #[test]
    fn p2a_game_association_and_actor_are_exact_without_native_authority() {
        let (module, catalog) = setup();
        let mut value = transition(&module);
        value["profiles"][0]["modules"][0]["mode"] = json!("Session");
        value["associations"] =
            json!([{"version":1,"group":"g","rule":"r","profile":1,"eligibility":"Running"}]);
        for (side, generation) in [("old", 1), ("new", 2)] {
            value["transitions"][0][side]["binding"] =
                serde_json::to_value(game(generation, "r", 42, ActivationEligibility::Running))
                    .unwrap();
            value["transitions"][0][side]["mode"] = json!("Session");
        }
        let producer = source(&value, &catalog);
        let snapshot = producer.snapshot();
        assert_eq!(
            query(&snapshot, &value["transitions"][0]["new"], false),
            IntentDecision::Enabled
        );
        assert_eq!(
            query(&snapshot, &value["transitions"][0]["old"], true),
            IntentDecision::Enabled
        );
        #[cfg(windows)]
        assert!(producer.proof.is_none());
        for field in [
            "lineage",
            "principal",
            "volume_guid",
            "volume_serial",
            "directory",
        ] {
            let e: super::super::config::SavedIntent =
                serde_json::from_value(value["transitions"][0]["old"].clone()).unwrap();
            assert_eq!(
                snapshot.continuation_for_origin(
                    &ConfigurationOrigin::fixture().fixture_changed(field),
                    e.owner,
                    &e.binding,
                    &e.config,
                    &e.target,
                    e.mode
                ),
                IntentDecision::Unknown
            );
        }
        for changed in [
            game(2, "other", 42, ActivationEligibility::Running),
            game(2, "r", 43, ActivationEligibility::Running),
            game(2, "r", 42, ActivationEligibility::Foreground),
        ] {
            let mut v = value.clone();
            v["transitions"][0]["new"]["binding"] = serde_json::to_value(changed).unwrap();
            assert_eq!(
                serde_json::from_value::<TuningEnvelope>(v)
                    .unwrap()
                    .decode(),
                TuningLoad::Blocked
            );
        }
        for field in ["group", "rule", "eligibility"] {
            let mut v = value.clone();
            v["associations"][0][field] = if field == "eligibility" {
                json!("Foreground")
            } else {
                json!("other")
            };
            assert_eq!(
                source(&v, &catalog).snapshot(),
                ConfiguredSnapshot::Unavailable
            );
        }
        value["transitions"][0]["new"]["config"]["version"] = json!(2);
        value["profiles"][0]["modules"][0]["config"]["version"] = json!(2);
        assert!(matches!(
            serde_json::from_value::<TuningEnvelope>(value.clone())
                .unwrap()
                .decode(),
            TuningLoad::Known(_)
        ));
        assert_eq!(
            source(&value, &catalog).snapshot(),
            ConfiguredSnapshot::Unavailable
        );
    }
}
