use super::*;
fn topology() -> Vec<os_api::background::PlacementCpu> {
    (0..4)
        .map(|i| os_api::background::PlacementCpu {
            id: 100 + i,
            group: 0,
            logical: i as u8,
            physical_core: i / 2,
            efficiency_class: if i < 2 { 8 } else { 0 },
            numa: 0,
            cache: 0,
            allowed: true,
            reserved: false,
        })
        .collect()
}
#[test]
fn background_placement_uses_a_stable_full_session_topology_and_never_expands_baseline() {
    let topology = topology();
    let config = resolve_pool(
        &Config {
            cpu_mode: CpuMode::EfficientCores,
            ..Default::default()
        },
        &topology,
    )
    .unwrap();
    assert_eq!(config.manual_ids, vec![102, 103]);
    let witness = placement_plan(&config, &topology, &[]).unwrap().unwrap();
    assert!(witness.permits(&[], &[102, 103]));
    assert!(witness.permits(&[102, 103], &[]));
    assert!(placement_plan(&config, &topology, &[100, 101]).is_err());
    let mut changed = topology.clone();
    changed[3].reserved = true;
    assert!(placement_plan(&config, &changed, &[]).is_err());
    assert!(resolve_pool(
        &Config {
            cpu_mode: CpuMode::ManualPool,
            manual_ids: vec![102],
            manual_topology: topology.clone(),
            ..Default::default()
        },
        &topology
    )
    .is_err());
    let mut homogeneous = topology.clone();
    for cpu in &mut homogeneous {
        cpu.efficiency_class = 0;
    }
    assert!(resolve_pool(
        &Config {
            cpu_mode: CpuMode::EfficientCores,
            ..Default::default()
        },
        &homogeneous
    )
    .is_err());
    assert!(placement_plan(&Config::default(), &homogeneous, &[])
        .unwrap()
        .is_none());
}
use crate::app::features::system_tweaks::{
    platform::{
        controller::{Controller, OwnerKey},
        registry::Module,
    },
    presentation::ModuleEditor,
};

#[test]
fn background_defaults_and_config_are_bounded_and_do_not_enable_placement() {
    let module = BackgroundWorkload::default();
    let envelope = module.default_config();
    let config = configured(&envelope).unwrap();
    assert_eq!(config.cpu_mode, CpuMode::PriorityEco);
    assert!(config.games.is_empty());
    assert!(module.accepts_config(&envelope));
    let mut future = envelope.clone();
    future.version = SchemaVersion::new(2).unwrap();
    assert!(!module.accepts_config(&future));
    let mut unknown = envelope.clone();
    unknown.payload["unknown"] = true.into();
    assert!(!module.accepts_config(&unknown));
    let mut oversized = Config::default();
    oversized.games = (0..65)
        .map(|i| GameRule {
            group: format!("g{i}"),
            rule: format!("r{i}"),
        })
        .collect();
    assert!(!oversized.valid());
}

#[test]
fn background_fields_use_the_same_canonical_resources_and_preserve_low_priority() {
    let target = TargetIdentity::Process {
        pid: 99,
        creation_token: 900,
        image_path: "worker.exe".into(),
    };
    let config = Config::default();
    let qos = os_api::gaming::OwnedPolicy {
        control: 0,
        state: 0,
    };
    let plan = plan_fields(
        &target,
        os_api::background::Role::DiagnosticsTelemetry,
        Some(0x20),
        Some(qos),
        &config,
    );
    assert_eq!(
        plan.changes
            .iter()
            .map(|change| change.key.field.as_str())
            .collect::<Vec<_>>(),
        vec!["process_priority_class", "process_execution_speed"]
    );
    let low = plan_fields(
        &target,
        os_api::background::Role::DiagnosticsTelemetry,
        Some(0x40),
        Some(qos),
        &config,
    );
    assert_eq!(low.changes.len(), 1);
    assert_eq!(low.noops, 1);
    assert_eq!(low.changes[0].key.field.as_str(), "process_execution_speed");
    for priority in [0x80, 0x8000, 0x100] {
        assert!(
            plan_fields(
                &target,
                os_api::background::Role::DiagnosticsTelemetry,
                Some(priority),
                Some(qos),
                &config
            )
            .blocked
        );
        assert!(plan_fields(
            &target,
            os_api::background::Role::DiagnosticsTelemetry,
            Some(priority),
            Some(qos),
            &config
        )
        .changes
        .is_empty());
    }
}

#[test]
fn background_missing_getter_skips_only_that_mechanism_and_ecoqos_is_explicit() {
    let target = TargetIdentity::Process {
        pid: 99,
        creation_token: 900,
        image_path: "worker.exe".into(),
    };
    let config = Config::default();
    let plan = plan_fields(
        &target,
        os_api::background::Role::DiagnosticsTelemetry,
        Some(0x20),
        None,
        &config,
    );
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.skipped.len(), 1);
    let plan = plan_fields(
        &target,
        os_api::background::Role::FrameworkOptimizer,
        Some(0x20),
        Some(os_api::gaming::OwnedPolicy {
            control: 0,
            state: 0,
        }),
        &config,
    );
    assert_eq!(
        plan.changes.len(),
        1,
        "Framework spawner priority is not changed"
    );
    assert_eq!(
        plan.changes[0].desired,
        [1u32, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
}

#[test]
fn background_suitable_values_keep_equal_owner_intents_for_the_controller() {
    let target = TargetIdentity::Process {
        pid: 99,
        creation_token: 900,
        image_path: "worker.exe".into(),
    };
    let ready = plan_fields(
        &target,
        Role::DiagnosticsTelemetry,
        Some(0x4000),
        Some(OwnedPolicy {
            control: EXECUTION_SPEED,
            state: EXECUTION_SPEED,
        }),
        &Config::default(),
    );
    assert_eq!(ready.noops, 2);
    assert_eq!(ready.changes.len(), 2,
        "The controller must distinguish an unowned suitable baseline from a field already owned by another session");
    assert_eq!(ready.changes[0].key.field.as_str(), PRIORITY);
    assert_eq!(ready.changes[0].desired, 0x4000u32.to_le_bytes());
    assert_eq!(ready.changes[1].key.field.as_str(), QOS);

    let witness = placement_plan(
        &Config {
            cpu_mode: CpuMode::ManualPool,
            manual_ids: vec![102, 103],
            manual_topology: topology(),
            ..Default::default()
        },
        &topology(),
        &[102, 103],
    )
    .unwrap()
    .unwrap();
    let mut plan = TargetPlan {
        changes: vec![],
        noops: 0,
        skipped: vec![],
        blocked: false,
    };
    include_placement_intent(&mut plan, &target, &witness);
    assert_eq!(plan.noops, 1);
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].key.field.as_str(), SETS);
    assert_eq!(plan.changes[0].desired, cpu_bytes(&[102, 103]));
}

// This adapter reuses the production planner and the existing in-memory setting
// backend. It grants no native role authority and performs no Windows queries.
struct PlannedFixture {
    role: Role,
    values: Arc<crate::app::features::system_tweaks::simple_tests::Setting>,
}
impl PlannedFixture {
    fn new(role: Role, priority: Option<u32>, qos: Option<OwnedPolicy>) -> Arc<Self> {
        let values = crate::app::features::system_tweaks::simple_tests::Setting::new(ID, false);
        {
            let mut fields = values.fields.lock().unwrap();
            fields.clear();
            if let Some(priority) = priority {
                fields.insert(
                    ResourceId::new(PRIORITY).unwrap(),
                    priority.to_le_bytes().to_vec(),
                );
            }
            if let Some(qos) = qos {
                fields.insert(ResourceId::new(QOS).unwrap(), policy_bytes(qos));
            }
        }
        Arc::new(Self { role, values })
    }
}
impl ModuleEditor for PlannedFixture {
    fn availability(&self) -> crate::app::features::system_tweaks::platforms::TweakAvailability {
        self.values.availability()
    }
    fn feature(&self) -> FeatureId {
        feature(ID)
    }
    fn default_config(&self) -> ModuleEnvelope {
        Config::default().envelope()
    }
    fn accepts_config(&self, value: &ModuleEnvelope) -> bool {
        configured(value).is_ok()
    }
    fn show(&self, _: &mut eframe::egui::Ui, _: &mut ModuleEnvelope) -> Result<bool, ()> {
        Ok(false)
    }
}
impl Module for PlannedFixture {
    fn requires_session_target(&self) -> bool {
        true
    }
    fn descriptor(&self) -> ModuleDescriptor {
        let mut descriptor = self.values.descriptor();
        descriptor.target_scopes = vec![TargetScope::Process];
        descriptor.activation_modes = vec![ActivationMode::Session];
        descriptor
    }
    fn editor(&self) -> &dyn ModuleEditor {
        self
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        vec![]
    }
    fn observe(&self, target: &TargetIdentity) -> ModuleObservation {
        self.values.observe(target)
    }
    fn fields(
        &self,
        target: &TargetIdentity,
        config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        let fields = self.values.fields.lock().unwrap();
        let priority = fields
            .get(&ResourceId::new(PRIORITY).unwrap())
            .and_then(|value| value.as_slice().try_into().ok())
            .map(u32::from_le_bytes);
        let qos = fields
            .get(&ResourceId::new(QOS).unwrap())
            .map(|value| OwnedPolicy {
                control: u32::from_le_bytes(value[..4].try_into().unwrap()),
                state: u32::from_le_bytes(value[4..].try_into().unwrap()),
            });
        Ok(plan_fields(target, self.role, priority, qos, &configured(config)?).changes)
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        self.values.read(key)
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        self.values.write(key, expected, desired)
    }
}
fn acquire_planned(controller: &mut Controller, module: Arc<PlannedFixture>, id: u64) -> OwnerKey {
    use crate::app::features::system_tweaks::{
        persistence::ActivationFacts, platform::controller::SessionSource,
    };
    let owner = OwnerKey::SessionTarget {
        profile: ProfileId::new(id).unwrap(),
        feature: feature(ID),
        source: SessionSource::Game {
            group: "games".into(),
            rule: format!("game-{id}"),
            pid: 10 + id as u32,
            token: 100 + id,
        },
        target_pid: 99,
        target_token: 900,
    };
    controller
        .acquire_session(
            owner.clone(),
            module,
            TargetIdentity::Process {
                pid: 99,
                creation_token: 900,
                image_path: "fixture.exe".into(),
            },
            Config::default().envelope(),
            ActivationFacts {
                pid: 10 + id as u32,
                token: 100 + id,
                image: "game.exe".into(),
                trigger: ActivationEligibility::Running,
            },
        )
        .unwrap();
    owner
}

#[test]
fn background_original_role_plans_share_controller_fields_until_last_game_undo() {
    use crate::app::features::system_tweaks::platform::controller::RecoveryIntent;
    for role in [
        Role::Appraiser,
        Role::DeviceCensus,
        Role::FrameworkOptimizer,
        Role::DiagnosticsTelemetry,
    ] {
        let module = PlannedFixture::new(
            role,
            Some(0x20),
            Some(OwnedPolicy {
                control: 0,
                state: 0,
            }),
        );
        let baseline = module.values.fields.lock().unwrap().clone();
        let mut controller = Controller::default();
        let first = acquire_planned(&mut controller, module.clone(), 1);
        let writes = *module.values.writes.lock().unwrap();
        assert_eq!(
            writes,
            if role == Role::FrameworkOptimizer {
                1
            } else {
                2
            }
        );
        let active = module.values.fields.lock().unwrap().clone();
        let second = acquire_planned(&mut controller, module.clone(), 2);
        assert_eq!(*module.values.writes.lock().unwrap(), writes);
        assert!(controller
            .fields()
            .iter()
            .all(|field| field.owners.len() == 2));
        assert!(!controller
            .fields()
            .iter()
            .any(|field| field.key.field.as_str() == SETS));
        controller
            .recover_owner(&first, RecoveryIntent::Restore)
            .unwrap();
        assert_eq!(controller.owners(), [second.clone()].into());
        assert_eq!(*module.values.writes.lock().unwrap(), writes);
        assert_eq!(*module.values.fields.lock().unwrap(), active);
        controller
            .recover_owner(&second, RecoveryIntent::Restore)
            .unwrap();
        assert_eq!(*module.values.fields.lock().unwrap(), baseline);
        assert_eq!(*module.values.writes.lock().unwrap(), writes * 2);
        assert!(controller.owners().is_empty());
    }
}

#[test]
fn background_controller_never_owns_a_suitable_baseline_and_restores_only_readable_fields() {
    use crate::app::features::system_tweaks::platform::controller::RecoveryIntent;
    for priority in [0x40, 0x4000] {
        let module = PlannedFixture::new(
            Role::DiagnosticsTelemetry,
            Some(priority),
            Some(OwnedPolicy {
                control: EXECUTION_SPEED,
                state: EXECUTION_SPEED,
            }),
        );
        let mut controller = Controller::default();
        acquire_planned(&mut controller, module.clone(), 1);
        assert!(controller.owners().is_empty());
        assert_eq!(*module.values.writes.lock().unwrap(), 0);
    }
    for (priority, qos) in [
        (Some(0x20), None),
        (
            None,
            Some(OwnedPolicy {
                control: 0,
                state: 0,
            }),
        ),
    ] {
        let module = PlannedFixture::new(Role::DiagnosticsTelemetry, priority, qos);
        let baseline = module.values.fields.lock().unwrap().clone();
        let mut controller = Controller::default();
        let owner = acquire_planned(&mut controller, module.clone(), 1);
        assert_eq!(controller.fields().len(), 1);
        assert_eq!(*module.values.writes.lock().unwrap(), 1);
        controller
            .recover_owner(&owner, RecoveryIntent::Restore)
            .unwrap();
        assert_eq!(*module.values.fields.lock().unwrap(), baseline);
        assert_eq!(*module.values.writes.lock().unwrap(), 2);
    }
}

#[test]
fn background_module_rejects_manual_ownership_before_native_preflight() {
    let module = std::sync::Arc::new(BackgroundWorkload::default());
    let owner = OwnerKey::Manual {
        profile: ProfileId::new(1).unwrap(),
        feature: module.descriptor().id,
    };
    let result = Controller::default().acquire(
        owner,
        module.clone(),
        TargetIdentity::Process {
            pid: 99,
            creation_token: 900,
            image_path: "worker.exe".into(),
        },
        module.default_config(),
    );
    assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidConfig);
}

#[test]
fn background_context_cannot_be_omitted_or_retargeted_to_a_reused_pid() {
    let proof: os_api::background::TargetProof = serde_json::from_value(serde_json::json!({
        "catalog_version": 1, "role": "diagnostics_telemetry", "pid": 99, "creation_token": 900,
        "image_path": "C:\\Windows\\System32\\svchost.exe", "sid": "S-1-5-18", "session": 0,
        "logon": 999, "os_build": 26100, "image": {"volume":1,"index":2,"size":3,"modified":4},
        "context": {"Service":{"name":"DiagTrack","service_type":16,"arguments":"-k utcsvc -p",
            "dll_path":"C:\\Windows\\System32\\diagtrack.dll", "dll_identity":{"volume":1,"index":3,"size":4,"modified":5}}}
    })).unwrap();
    let mut key = ResourceKey {
        target: TargetIdentity::Process {
            pid: 99,
            creation_token: 900,
            image_path: proof.image_path.clone(),
        },
        field: ResourceId::new("process_priority_class").unwrap(),
    };
    let bytes = serde_json::to_vec(&FieldContext {
        proof: proof.clone(),
        placement: None,
    })
    .unwrap();
    let topology: Vec<_> = (0..512u32)
        .map(|index| os_api::background::PlacementCpu {
            id: index,
            group: (index / 64) as u16,
            logical: (index % 64) as u8,
            physical_core: index,
            efficiency_class: 0,
            numa: 0,
            cache: 0,
            allowed: true,
            reserved: false,
        })
        .collect();
    let mut context = FieldContext {
        proof: proof.clone(),
        placement: Some(os_api::background::PlacementWitness {
            baseline: vec![],
            desired: (0..512).collect(),
            topology,
        }),
    };
    assert!(serde_json::to_vec(&context).unwrap().len() > MAX_FIELD_BYTES);
    let mut plan = plan_fields(
        &key.target,
        proof.role,
        Some(0x20),
        None,
        &Config::default(),
    );
    plan.changes
        .push(field(&key.target, SETS, cpu_bytes(&[100])));
    let mut notes = vec![];
    let bounded = encode_context(&mut context, &mut plan, &mut notes).unwrap();
    assert!(bounded.len() <= MAX_FIELD_BYTES);
    assert!(context.placement.is_none());
    assert!(plan
        .changes
        .iter()
        .any(|change| change.key.field.as_str() == PRIORITY));
    assert!(!plan
        .changes
        .iter()
        .any(|change| change.key.field.as_str() == SETS));
    assert!(!notes.is_empty());
    assert!(context_proof(&key, Some(&bytes)).is_ok());
    assert!(context_proof(&key, None).is_err());
    key.target = TargetIdentity::Process {
        pid: 99,
        creation_token: 901,
        image_path: proof.image_path,
    };
    assert!(context_proof(&key, Some(&bytes)).is_err());
}
