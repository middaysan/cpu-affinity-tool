//! Opt-in real Windows round trips. ROOT selects each ignored test after recovery review.
#![cfg(all(test, windows, feature = "tuning-test-controls"))]

use super::{
    config::{
        Association, ManualTarget, ModulePreference, Profile, TuningConfig, TuningEnvelope,
        TuningLoad,
    },
    persistence::{source_identifier, ActivationFacts, DiskRecoveryStore},
    platform::{
        codec::ModuleEnvelope,
        contract::*,
        controller::{Controller, LifetimeEvidence, OwnerKey, RecoveryIntent},
        registry::{FieldChange, Module, Registry},
    },
    tweaks::{self, scheduling::SchedulingHandoff},
};
use crate::app::{
    features::rules::RulesContext,
    models::{AppStateStorage, AppToRun, CoreGroup},
};
use os_api::{
    gaming::{self, ProcessIdentity, ProcessSettingsFields, UserIdentity},
    OS,
};
use serde_json::{json, Value};
use std::{
    fs,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

type TestResult<T = ()> = Result<T, String>;
static NATIVE_SERIAL: Mutex<()> = Mutex::new(());
/// Test-only legacy fixture: retired CPU options remain verifiable for recovery.
fn legacy_cpu_configuration(
    module: &dyn Module,
    user: &UserIdentity,
    options: os_api::gaming_cpu_power::CpuPowerOptions,
) -> TestResult<ModuleEnvelope> {
    let requests = checked(os_api::gaming_cpu_power::prepare_requests(user, &options))?;
    let mut config = module.editor().default_config();
    config.payload = json!({"ac":"Performance","dc":"Balanced","ac_only":true,
        "cpu":{"maximum_mhz":options.performance_mhz,"efficient_mhz":options.efficient_mhz,
        "keep_high":options.keep_high,"allow_parking":options.allow_parking,"requests":requests}});
    if !module.editor().accepts_config(&config) {
        return Err("Invalid retained CPU fixture".into());
    }
    Ok(config)
}
#[test]
#[ignore = "Explicit read-only CPU power preparation; no setting or recovery writes"]
fn native_cpu_power_prepare_readonly() -> TestResult {
    let user = checked(gaming::interactive_user())?;
    for high in [false, true] {
        let requests = checked(os_api::gaming_cpu_power::prepare_requests(
            &user,
            &os_api::gaming_cpu_power::CpuPowerOptions {
                performance_mhz: 4500,
                keep_high: high,
                ..Default::default()
            },
        ))?;
        eprintln!(
            "CPU AC high={high}: {}",
            checked(serde_json::to_string(&requests))?
        );
    }
    Ok(())
}
#[test]
#[ignore = "ROOT: separate hybrid AC ceilings and both parking choices; exact disk-reloaded Undo"]
fn native_roundtrip_tweak_cpu_power_hybrid_parking() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let user = checked(gaming::interactive_user())?;
    let hybrid = checked(os_api::gaming_cpu_power::hybrid_cores(&user))?;
    eprintln!("Two verified CPU classes: {hybrid}");
    let original_plan = checked(gaming::read_power_plan(&user))?;
    let original_dc = checked(gaming::read_power_mode(&user, gaming::PowerSource::Dc))?;
    for (high, parking) in [(false, true), (true, false)] {
        let registry = tweaks::registry();
        let module = module(&registry, "power_mode");
        let target = user_target(&user);
        assert_eq!(module.observe(&target).capability, Capability::Available);
        let desired = legacy_cpu_configuration(
            module.as_ref(),
            &user,
            os_api::gaming_cpu_power::CpuPowerOptions {
                performance_mhz: 4500,
                efficient_mhz: hybrid.then_some(2500),
                keep_high: high,
                allow_parking: Some(parking),
            },
        )?;
        let requests: Vec<os_api::gaming_cpu_power::CpuPowerRequest> = checked(
            serde_json::from_value(desired.payload["cpu"]["requests"].clone()),
        )?;
        for request in &requests {
            use os_api::gaming_cpu_power::CpuPowerSetting::*;
            match request.key.setting {
                Frequency if hybrid => assert_eq!(request.value, 2500),
                Frequency1 if hybrid => assert_eq!(request.value, 4500),
                ParkingMinimum | ParkingMinimum1 => {
                    assert_eq!(request.value, if parking { 0 } else { 100 })
                }
                _ => {}
            }
        }
        eprintln!(
            "New CPU policy high={high}, parking={parking}, changed fields={}",
            requests.len()
        );
        roundtrip(registry, module, target, desired, || {
            assert_eq!(checked(gaming::read_power_plan(&user))?, original_plan);
            assert_eq!(
                checked(gaming::read_power_mode(&user, gaming::PowerSource::Dc))?,
                original_dc
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
#[ignore = "Explicit read-only diagnosis: inspect compact tweak targets and values; no apply or recovery writes"]
fn native_compact_catalog_readonly() {
    let registry = tweaks::registry();
    for descriptor in registry.descriptors() {
        let module = registry.get(&descriptor.id).unwrap();
        if module.editor().tweak().is_none() || module.editor().selects_saved_exe() {
            continue;
        }
        let availability = module.editor().availability();
        if !availability.is_available() {
            println!("{}: {:?}", descriptor.id.as_str(), availability);
            continue;
        }
        let targets = module.targets();
        let observed = module.observe(targets.first().unwrap_or(&TargetIdentity::Machine));
        println!(
            "{}: targets={}, capability={:?}, value={}",
            descriptor.id.as_str(),
            targets.len(),
            observed.capability,
            observed.summary.summary
        );
    }
    struct ReadOnlySave;
    impl super::StatePersistence for ReadOnlySave {
        fn save(
            &mut self,
            _: &mut AppStateStorage,
        ) -> Result<super::service::SavedState, super::SaveError> {
            panic!("Read-only catalog diagnosis must not save settings")
        }
    }
    let service = super::Tuning::from_registry(registry).headless(
        AppStateStorage::test_fixture(|_| {}),
        Box::new(ReadOnlySave),
    );
    let runtime = super::TuningRuntime::start(
        service,
        Arc::new(tokio::sync::RwLock::new(Default::default())),
    );
    let started = std::time::Instant::now();
    loop {
        let snapshot = runtime.snapshot();
        if snapshot
            .tweak_cards
            .iter()
            .filter(|card| card.feature.as_str() != "gpu_preference")
            .all(|card| card.observed.is_some())
        {
            println!(
                "Worker published real compact values after {:?}",
                started.elapsed()
            );
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "Worker left compact cards unobserved: {:?}",
            snapshot.operation
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    runtime.begin_shutdown();
}
fn checked<T, E: std::fmt::Debug>(value: Result<T, E>) -> TestResult<T> {
    value.map_err(|error| format!("{error:?}"))
}
fn module(registry: &Registry, id: &str) -> Arc<dyn Module> {
    registry.get(&FeatureId::new(id).unwrap()).unwrap().clone()
}
fn config(module: &Arc<dyn Module>, payload: Value) -> ModuleEnvelope {
    let mut config = module.editor().default_config();
    config.payload = payload;
    assert!(module.editor().accepts_config(&config));
    config
}
fn user_target(user: &UserIdentity) -> TargetIdentity {
    TargetIdentity::User {
        sid: user.sid.clone(),
        logon: user.logon,
        session: None,
    }
}
fn process_target(process: &ProcessIdentity) -> TargetIdentity {
    TargetIdentity::Process {
        pid: process.pid,
        creation_token: process.creation_token,
        image_path: process.image_path.clone(),
    }
}

struct OriginalField {
    change: FieldChange,
    original: Vec<u8>,
}
/// The independent fallback never uses a journal as permission to overwrite foreign state.
struct Rollback {
    module: Arc<dyn Module>,
    fields: Vec<OriginalField>,
    armed: bool,
}
impl Drop for Rollback {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        for field in self.fields.iter().rev() {
            match self.module.read(&field.change.key) {
                Ok(actual) if actual == field.original => {}
                Ok(actual) if actual == field.change.desired => {
                    let restored = self
                        .module
                        .write(&field.change.key, &actual, &field.original);
                    let verified = self.module.read(&field.change.key);
                    if restored.is_err() || verified.as_ref() != Ok(&field.original) {
                        eprintln!(
                            "EMERGENCY ROLLBACK UNVERIFIED for {}: {restored:?}, {verified:?}",
                            field.change.key.field.as_str()
                        );
                    }
                }
                actual => eprintln!(
                    "EMERGENCY ROLLBACK REFUSED foreign/unknown state for {}: {actual:?}",
                    field.change.key.field.as_str()
                ),
            }
        }
    }
}

fn native_directory(id: &str) -> TestResult<PathBuf> {
    let serial = checked(SystemTime::now().duration_since(UNIX_EPOCH))?.as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "cpu-affinity-native-roundtrip-{}-{serial}-{id}",
        std::process::id()
    ));
    checked(fs::create_dir(&directory))?;
    eprintln!("Isolated native evidence: {}", directory.display());
    Ok(directory)
}
struct SavedIntent {
    path: PathBuf,
    state: AppStateStorage,
    config: TuningConfig,
    owner: OwnerKey,
    activation: Option<ActivationFacts>,
}
fn save_intent(
    directory: &std::path::Path,
    target: &TargetIdentity,
    config: &ModuleEnvelope,
) -> TestResult<SavedIntent> {
    let path = directory.join("state.json");
    let mut state = AppStateStorage::load_from_path(&path);
    let profile = ProfileId::new(1).unwrap();
    let mut saved = TuningConfig::default();
    saved.next_profile = 2;
    let process = match target {
        TargetIdentity::Process {
            pid,
            creation_token,
            image_path,
        } => Some((*pid, *creation_token, image_path.clone())),
        _ => None,
    };
    saved.profiles.push(Profile {
        version: 1,
        id: profile,
        name: "Native rollback test".into(),
        modules: vec![ModulePreference {
            feature: config.feature.clone(),
            enabled: true,
            mode: if process.is_some() {
                ActivationMode::Session
            } else {
                ActivationMode::Manual
            },
            config: config.clone(),
        }],
    });
    let (owner, activation) = if let Some((pid, token, image)) = process {
        state.groups = vec![CoreGroup {
            name: "Owned native child".into(),
            cores: vec![0],
            is_hidden: false,
            run_all_button: false,
            programs: vec![AppToRun::new_path(
                PathBuf::from(&image),
                vec![],
                PathBuf::from(&image),
                os_api::PriorityClass::Normal,
                false,
            )],
        }];
        state.rule_identities = None;
        let rules = RulesContext::from_storage(&state);
        let group = rules.group_id_for_index(0).unwrap();
        let rule = rules.rule_id_for_index(0, 0).unwrap();
        state.rule_identities = Some(rules.to_persisted_identities());
        saved.associations.push(Association {
            version: 1,
            group: group.clone(),
            rule: rule.clone(),
            profile,
            eligibility: ActivationEligibility::Running,
        });
        (
            OwnerKey::Session {
                profile,
                feature: config.feature.clone(),
                group: group.0,
                rule: rule.0,
                pid,
                token,
            },
            Some(ActivationFacts {
                pid,
                token,
                image,
                trigger: ActivationEligibility::Running,
            }),
        )
    } else {
        saved.manual_targets.push(ManualTarget {
            profile,
            feature: config.feature.clone(),
            target: target.clone(),
        });
        (
            OwnerKey::Manual {
                profile,
                feature: config.feature.clone(),
            },
            None,
        )
    };
    state.tuning = checked(TuningEnvelope::from_config(&saved))?;
    checked(state.try_save_to_path(&path))?;
    assert_eq!(
        AppStateStorage::load_from_path(&path).tuning.decode(),
        TuningLoad::Known(Box::new(saved.clone()))
    );
    Ok(SavedIntent {
        path,
        state,
        config: saved,
        owner,
        activation,
    })
}

/// One actual source, one module and one disk journal. No synthetic backend is accepted here.
fn roundtrip(
    registry: Registry,
    module: Arc<dyn Module>,
    target: TargetIdentity,
    config: ModuleEnvelope,
    verify_unowned: impl Fn() -> TestResult,
) -> TestResult {
    let target = checked(module.canonical_target(&target))?;
    let observation = module.observe(&target);
    if observation.capability != Capability::Available {
        return Err(format!(
            "NOT READY: {:?}; {}",
            observation.capability, observation.summary.summary
        ));
    }
    let changes = checked(module.fields(&target, &config))?;
    let fields = changes
        .into_iter()
        .map(|change| {
            checked(module.read(&change.key)).map(|original| OriginalField { change, original })
        })
        .collect::<TestResult<Vec<_>>>()?;
    if !fields
        .iter()
        .any(|field| field.original != field.change.desired)
    {
        return Err("Unsupported/not ready: this adapter has no actual changed value on this host; no-op is not native evidence".into());
    }
    let mut rollback = Rollback {
        module: module.clone(),
        fields,
        armed: false,
    };
    let directory = native_directory(config.feature.as_str())?;
    let SavedIntent {
        path,
        mut state,
        config: mut saved,
        owner,
        activation,
    } = save_intent(&directory, &target, &config)?;
    let source = checked(source_identifier(&path))?;
    let store = checked(DiskRecoveryStore::open(&directory))?;
    let mut controller = checked(Controller::restore(
        Box::new(store),
        source.clone(),
        &registry,
    ))?;
    for field in &rollback.fields {
        assert_eq!(
            checked(module.read(&field.change.key))?,
            field.original,
            "field changed before native dispatch"
        );
    }
    rollback.armed = true;
    if let Some(activation) = activation {
        checked(controller.acquire_session(
            owner.clone(),
            module.clone(),
            target,
            config,
            activation,
        ))?;
    } else {
        checked(controller.acquire(owner.clone(), module.clone(), target, config))
            .map_err(|error| format!("native acquire: {error}"))?;
    }
    for field in &rollback.fields {
        assert_eq!(
            checked(module.read(&field.change.key))?,
            field.change.desired
        );
        let snapshot = controller
            .fields()
            .into_iter()
            .find(|snapshot| snapshot.key == field.change.key)
            .unwrap();
        assert_eq!(snapshot.baseline, field.original);
    }
    verify_unowned()?;
    drop(controller);

    // A fresh registry has no runtime scheduling metadata. Hydration must retain exact originals.
    let fresh_registry = tweaks::registry();
    let store = checked(DiskRecoveryStore::open(&directory))?;
    let mut controller = checked(Controller::restore(
        Box::new(store),
        source,
        &fresh_registry,
    ))?;
    assert_eq!(
        AppStateStorage::load_from_path(&path).tuning.decode(),
        TuningLoad::Known(Box::new(saved.clone()))
    );
    checked(controller.recover_owner(&owner, RecoveryIntent::Keep))?;
    for field in &rollback.fields {
        assert_eq!(
            checked(module.read(&field.change.key))?,
            field.change.desired
        );
        let snapshot = controller
            .fields()
            .into_iter()
            .find(|snapshot| snapshot.key == field.change.key)
            .unwrap();
        assert_eq!(
            snapshot.baseline, field.original,
            "disk hydration recaptured the original"
        );
    }
    saved.profiles[0].modules[0].enabled = false;
    state.tuning = checked(TuningEnvelope::from_config(&saved))?;
    checked(state.try_save_to_path(&path))?;
    assert_eq!(
        AppStateStorage::load_from_path(&path).tuning.decode(),
        TuningLoad::Known(Box::new(saved))
    );
    checked(controller.release(&owner))
        .map_err(|error| format!("native roundtrip release: {error}"))?;
    assert!(controller.is_empty());
    for field in &rollback.fields {
        assert_eq!(checked(module.read(&field.change.key))?, field.original);
    }
    verify_unowned()?;
    drop(controller);
    let store = checked(DiskRecoveryStore::open(&directory))?;
    let controller = checked(Controller::restore(
        Box::new(store),
        checked(source_identifier(&path))?,
        &fresh_registry,
    ))?;
    assert!(
        controller.is_empty(),
        "terminal recovery file still owns native fields"
    );
    rollback.armed = false;
    Ok(())
}

struct OwnedChild(Child);
impl OwnedChild {
    fn spawn() -> TestResult<(Self, ProcessIdentity)> {
        let image = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot missing")?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let child = checked(
            Command::new(image)
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 600",
                ])
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn(),
        )?;
        let child = Self(child);
        let identity = checked(OS::tuning_process_identity(child.0.id()))?;
        let process = ProcessIdentity {
            pid: identity.pid(),
            creation_token: identity.creation_time(),
            image_path: identity.image_path().into(),
        };
        Ok((child, process))
    }
    fn finish(mut self) -> TestResult {
        self.kill_and_wait()
    }
    fn kill_and_wait(&mut self) -> TestResult {
        if checked(self.0.try_wait())?.is_none() {
            checked(self.0.kill())?;
        }
        checked(self.0.wait())?;
        Ok(())
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn child_roundtrip(id: &str, choose: impl Fn(&ProcessIdentity) -> TestResult<Value>) -> TestResult {
    let (child, process) = OwnedChild::spawn()?;
    let result = (|| {
        let handoff = Arc::new(SchedulingHandoff::default());
        checked(handoff.started(
            process.pid,
            process.creation_token,
            ProcessSettingsFields {
                affinity: false,
                priority: false,
            },
        ))?;
        let registry = tweaks::registry_with_scheduling(handoff);
        let module = module(&registry, id);
        let config = config(&module, choose(&process)?);
        let other_bit = if id == "process_qos" {
            Some(gaming::IGNORE_TIMER_RESOLUTION)
        } else if id == "process_timer_policy" {
            Some(gaming::EXECUTION_SPEED)
        } else {
            None
        };
        let other_policy = other_bit
            .map(|bit| checked(gaming::read_process_policy(&process, bit)))
            .transpose()?;
        roundtrip(registry, module, process_target(&process), config, || {
            if let Some(bit) = other_bit {
                assert_eq!(
                    checked(gaming::read_process_policy(&process, bit))?,
                    other_policy.unwrap()
                );
            }
            Ok(())
        })
    })();
    let terminal = child.finish();
    result.and(terminal)
}

#[test]
#[ignore = "ROOT only after disk recovery GREEN/review: changes an existing user power scheme, verifies disk reload and rollback"]
fn native_roundtrip_power_plan() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let user = checked(gaming::interactive_user())?;
    let original = checked(gaming::read_power_plan(&user))?;
    let alternate = checked(gaming::power_schemes(&user))?
        .into_iter()
        .find(|scheme| scheme.id != original)
        .ok_or("Unsupported/not ready: no alternate existing power scheme")?;
    let registry = tweaks::registry();
    let module = module(&registry, "power_plan");
    let config = config(&module, json!({"scheme": alternate.id}));
    roundtrip(registry, module, user_target(&user), config, || Ok(()))
}

/// Create a changed precondition only when necessary, retaining its durable original.
fn recommended_tweak_roundtrip(id: &str, target: TargetIdentity) -> TestResult {
    use os_api::windows_tweaks::{self as api, StoredDword};
    let registry = tweaks::registry();
    let module = module(&registry, id);
    let target = checked(module.canonical_target(&target))?;
    let desired = checked(module.editor().tweak_config(&target))?;
    let mut change = checked(module.fields(&target, &desired))?
        .into_iter()
        .next()
        .ok_or("No owned field")?;
    let original = checked(module.read(&change.key))?;
    let dc_original = if id == "power_mode" {
        Some(checked(gaming::read_power_mode(
            &checked(gaming::interactive_user())?,
            gaming::PowerSource::Dc,
        ))?)
    } else {
        None
    };
    let mut preparation = None;
    if original == change.desired {
        change.desired = match id {
            "game_mode" => checked(serde_json::to_vec(&StoredDword::Value(0)))?,
            "background_capture" => checked(serde_json::to_vec(&StoredDword::Value(1)))?,
            "windowed_optimizations" => checked(serde_json::to_vec(&Some("0")))?,
            "gpu_preference" => checked(serde_json::to_vec(&Some("1")))?,
            "power_mode" => gaming::POWER_MODE_BALANCED.0.to_vec(),
            "display_refresh" => {
                let display = checked(api::primary_display_target())?;
                let modes = checked(api::display_modes(&display))?;
                let alternate = modes
                    .compatible
                    .iter()
                    .filter(|mode| **mode != modes.current)
                    .max_by_key(|mode| mode.frequency)
                    .ok_or(
                        "NOT READY: no alternate compatible refresh mode for a real round trip",
                    )?;
                checked(serde_json::to_vec(alternate))?
            }
            _ => return Err("Unsupported native precondition".into()),
        };
        let prepared = Arc::new(PrepareAccessibilityHotkey {
            native: module.clone(),
            change: change.clone(),
        });
        let mut prepare_registry = Registry::default();
        checked(prepare_registry.register(prepared.clone()))?;
        let directory = native_directory("tweak_precondition")?;
        let saved = save_intent(&directory, &target, &desired)?;
        let mut controller = checked(Controller::restore(
            Box::new(checked(DiskRecoveryStore::open(&directory))?),
            checked(source_identifier(&saved.path))?,
            &prepare_registry,
        ))?;
        let rollback = Rollback {
            module: module.clone(),
            fields: vec![OriginalField {
                change: change.clone(),
                original: original.clone(),
            }],
            armed: true,
        };
        checked(controller.acquire(
            saved.owner.clone(),
            prepared,
            target.clone(),
            desired.clone(),
        ))
        .map_err(|error| format!("prepare native precondition: {error}"))?;
        assert_eq!(checked(module.read(&change.key))?, change.desired);
        preparation = Some((controller, saved.owner, rollback));
    }
    let desired = checked(module.editor().tweak_config(&target))?;
    let result = roundtrip(registry, module.clone(), target, desired, || {
        if let Some(original) = dc_original {
            assert_eq!(
                checked(gaming::read_power_mode(
                    &checked(gaming::interactive_user())?,
                    gaming::PowerSource::Dc
                ))?,
                original
            );
        }
        Ok(())
    });
    let cleanup = (|| {
        if let Some((controller, owner, _)) = &mut preparation {
            checked(controller.release(owner))
                .map_err(|error| format!("release native precondition: {error}"))?;
        }
        assert_eq!(checked(module.read(&change.key))?, original);
        if let Some((_, _, rollback)) = &mut preparation {
            rollback.armed = false;
        }
        Ok(())
    })();
    result.and(cleanup)
}

#[test]
#[ignore = "ROOT: real stored Game Mode preference, durable reload and exact undo"]
fn native_roundtrip_tweak_game_mode() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    recommended_tweak_roundtrip(
        "game_mode",
        user_target(&checked(gaming::interactive_user())?),
    )
}
#[test]
#[ignore = "ROOT: only the background capture preference, durable reload and exact undo"]
fn native_roundtrip_tweak_background_capture() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    recommended_tweak_roundtrip(
        "background_capture",
        user_target(&checked(gaming::interactive_user())?),
    )
}
#[test]
#[ignore = "ROOT: windowed preference owned field, durable reload and exact undo; Auto HDR dependency is not changed"]
fn native_roundtrip_tweak_windowed() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    recommended_tweak_roundtrip(
        "windowed_optimizations",
        user_target(&checked(gaming::interactive_user())?),
    )
}
#[test]
#[ignore = "ROOT: GPU preference for this owned test EXE only, durable reload and exact undo"]
fn native_roundtrip_tweak_gpu() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let path = checked(std::env::current_exe())?;
    let target = checked(os_api::windows_tweaks::qualify_gpu_target(
        &path.to_string_lossy(),
    ))?;
    let target = TargetIdentity::Device {
        identity: format!("gpu_exe:{}", checked(serde_json::to_string(&target))?),
    };
    recommended_tweak_roundtrip("gpu_preference", target)
}

#[test]
#[ignore = "ROOT: owned temporary EXE removed after Apply; GPU preference disk reload and exact Undo"]
fn native_roundtrip_tweak_gpu_removed_executable() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let directory = native_directory("removed_gpu_exe")?;
    let path = directory.join("owned-test.exe");
    checked(fs::write(
        &path,
        b"Owned GPU preference target; never executed",
    ))?;
    let qualified = checked(os_api::windows_tweaks::qualify_gpu_target(
        &path.to_string_lossy(),
    ))?;
    let target = TargetIdentity::Device {
        identity: format!("gpu_exe:{}", checked(serde_json::to_string(&qualified))?),
    };
    let registry = tweaks::registry();
    let module = module(&registry, "gpu_preference");
    let config = checked(module.editor().tweak_config(&target))?;
    let removed = std::cell::Cell::new(false);
    roundtrip(registry, module, target, config, || {
        if !removed.replace(true) {
            checked(fs::remove_file(&path))?;
        }
        assert!(!path.exists(), "Undo must not depend on restoring the EXE");
        Ok(())
    })
}
#[test]
#[ignore = "ROOT: temporary main-screen Hz only, durable reload and exact undo; no resolution change"]
fn native_roundtrip_tweak_display() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let target = checked(os_api::windows_tweaks::primary_display_target())?;
    let target = TargetIdentity::Device {
        identity: format!("display:{}", checked(serde_json::to_string(&target))?),
    };
    recommended_tweak_roundtrip("display_refresh", target)
}

#[test]
#[ignore = "ROOT: new AC-only performance preset, unchanged battery preference, durable reload and exact undo"]
fn native_roundtrip_tweak_power_ac_only() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    recommended_tweak_roundtrip(
        "power_mode",
        user_target(&checked(gaming::interactive_user())?),
    )
}
#[test]
#[ignore = "ROOT: AC CPU limit/high-clock request, exact absent/DWORD baseline, disk reload and Undo; no parking or battery changes"]
fn native_roundtrip_tweak_cpu_power_options() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let user = checked(gaming::interactive_user())?;
    let target = user_target(&user);
    let original_dc = checked(gaming::read_power_mode(&user, gaming::PowerSource::Dc))?;
    let original_plan = checked(gaming::read_power_plan(&user))?;
    for high in [false, true] {
        let registry = tweaks::registry();
        let module = module(&registry, "power_mode");
        let desired = legacy_cpu_configuration(
            module.as_ref(),
            &user,
            os_api::gaming_cpu_power::CpuPowerOptions {
                performance_mhz: 4500,
                keep_high: high,
                ..Default::default()
            },
        )?;
        assert!(desired.payload["cpu"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["key"]["setting"] == "Frequency"));
        let prepared = desired.payload["cpu"]["requests"].as_array().unwrap();
        eprintln!(
            "AC CPU requests, high={high}: {}",
            serde_json::to_string(prepared).unwrap()
        );
        // The other power source, original plan and parking fields remain unowned and unchanged.
        let processor = windows::core::GUID::from_u128(0x54533251_82be_4824_96c1_47b60b740d00);
        let parking = windows::core::GUID::from_u128(0x0cc5b647_c1df_4637_891a_dec35c318583);
        let parking1 = windows::core::GUID::from_u128(0x0cc5b647_c1df_4637_891a_dec35c318584);
        let plan = windows::core::GUID::from_u128(u128::from_be_bytes(original_plan.0));
        let read_parking = || -> TestResult<Vec<u32>> {
            [parking, parking1]
                .into_iter()
                .map(|setting| {
                    let mut value = 0;
                    let result = unsafe {
                        windows::Win32::System::Power::PowerReadACValueIndex(
                            None,
                            Some(&plan),
                            Some(&processor),
                            Some(&setting),
                            &mut value,
                        )
                    };
                    if result.0 != 0 {
                        return Err(format!("Read-only parking check: {}", result.0));
                    }
                    Ok(value)
                })
                .collect()
        };
        let untouched = read_parking()?;
        roundtrip(registry, module, target.clone(), desired, || {
            assert_eq!(
                checked(gaming::read_power_mode(&user, gaming::PowerSource::Dc))?,
                original_dc
            );
            assert_eq!(checked(gaming::read_power_plan(&user))?, original_plan);
            assert_eq!(read_parking()?, untouched);
            Ok(())
        })?;
    }
    Ok(())
}
#[test]
#[ignore = "ROOT: restore the explicitly selected retained CPU-power test journal; never recapture originals"]
fn native_roundtrip_tweak_cpu_power_00_restore_retained() -> TestResult {
    let _serial = NATIVE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let directory = PathBuf::from(
        std::env::var_os("CPU_POWER_TEST_RECOVERY_DIRECTORY")
            .ok_or("No explicitly selected test journal")?,
    );
    let document: super::persistence::RecoveryDocument = checked(serde_json::from_slice(
        &checked(fs::read(directory.join("recovery-v1.json")))?,
    ))?;
    checked(document.validate())?;
    assert!(!document.fields.is_empty());
    assert!(document
        .fields
        .iter()
        .all(|field| field.module.as_str() == "power_mode"
            && field.key.field.as_str().starts_with("cpu_ac_")
            && field.baseline == [0]));
    let registry = tweaks::registry();
    let module = module(&registry, "power_mode");
    let mut controller = checked(Controller::restore(
        Box::new(checked(DiskRecoveryStore::open(&directory))?),
        checked(source_identifier(&directory.join("state.json")))?,
        &registry,
    ))?;
    for owner in controller.owners() {
        checked(controller.recover_owner(&owner, RecoveryIntent::Restore))?;
    }
    assert!(controller.is_empty());
    for field in document.fields {
        assert_eq!(checked(module.read(&field.key))?, field.baseline);
    }
    Ok(())
}
fn mode_name(value: gaming::GuidValue) -> TestResult<&'static str> {
    if value == gaming::POWER_MODE_BALANCED {
        Ok("Balanced")
    } else if value == gaming::POWER_MODE_EFFICIENCY {
        Ok("Efficiency")
    } else if value == gaming::POWER_MODE_PERFORMANCE {
        Ok("Performance")
    } else {
        Err(format!("Unsupported/not ready: original power mode {:032x} has no supported configuration representation", u128::from_be_bytes(value.0)))
    }
}
fn power_mode_roundtrip(source: gaming::PowerSource) -> TestResult {
    let user = checked(gaming::interactive_user())?;
    let ac = checked(gaming::read_power_mode(&user, gaming::PowerSource::Ac))?;
    let dc = checked(gaming::read_power_mode(&user, gaming::PowerSource::Dc))?;
    let mut ac_name = mode_name(ac)?;
    let mut dc_name = mode_name(dc)?;
    let (other_source, other_original) = match source {
        gaming::PowerSource::Ac => {
            ac_name = if ac_name == "Balanced" {
                "Efficiency"
            } else {
                "Balanced"
            };
            (gaming::PowerSource::Dc, dc)
        }
        gaming::PowerSource::Dc => {
            dc_name = if dc_name == "Balanced" {
                "Efficiency"
            } else {
                "Balanced"
            };
            (gaming::PowerSource::Ac, ac)
        }
    };
    let registry = tweaks::registry();
    let module = module(&registry, "power_mode");
    let config = config(&module, json!({"ac": ac_name, "dc": dc_name}));
    roundtrip(registry, module, user_target(&user), config, || {
        assert_eq!(
            checked(gaming::read_power_mode(&user, other_source))?,
            other_original
        );
        Ok(())
    })
}
#[test]
#[ignore = "ROOT only after disk recovery GREEN/review: changes user AC Power Mode and verifies unchanged DC plus exact rollback"]
fn native_roundtrip_power_mode_ac() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    power_mode_roundtrip(gaming::PowerSource::Ac)
}
#[test]
#[ignore = "ROOT only after disk recovery GREEN/review: changes user DC Power Mode and verifies unchanged AC plus exact rollback"]
fn native_roundtrip_power_mode_dc() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    power_mode_roundtrip(gaming::PowerSource::Dc)
}

#[test]
#[ignore = "ROOT only after disk recovery GREEN/review: changes mouse acceleration tuple and verifies unchanged speed plus exact rollback"]
fn native_roundtrip_mouse_acceleration() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let user = checked(gaming::interactive_user())?;
    let original = checked(gaming::read_mouse(&user))?;
    let desired = if original.0[2] == 0 {
        [6, 10, 1]
    } else {
        [0, 0, 0]
    };
    let speed = checked(gaming::read_user_setting_validation(&user))?.mouse_speed;
    let registry = tweaks::registry();
    let module = module(&registry, "mouse_acceleration");
    let config = config(
        &module,
        json!({"threshold1": desired[0], "threshold2": desired[1], "acceleration": desired[2]}),
    );
    roundtrip(registry, module, user_target(&user), config, || {
        assert_eq!(
            checked(gaming::read_user_setting_validation(&user))?.mouse_speed,
            speed
        );
        Ok(())
    })
}

#[test]
#[ignore = "ROOT only after disk recovery GREEN/review: suppresses existing disabled-feature hotkeys, preserves complete accessibility settings and restores"]
fn native_roundtrip_accessibility_hotkeys() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let user = checked(gaming::interactive_user())?;
    let original = checked(gaming::read_user_setting_validation(&user))?;
    let registry = tweaks::registry();
    let module = module(&registry, "accessibility_hotkeys");
    let config = module.editor().default_config();
    let changes = checked(module.fields(&user_target(&user), &config))?;
    for (name, flags) in [
        ("sticky_hotkey_flags", original.sticky_flags),
        ("filter_hotkey_flags", original.filter_flags),
        ("toggle_hotkey_flags", original.toggle_flags),
    ] {
        if flags & 1 != 0 {
            assert!(
                !changes
                    .iter()
                    .any(|change| change.key.field.as_str() == name),
                "enabled feature was not skipped"
            );
        }
    }
    let target = user_target(&user);
    // This host may already have every selected hotkey disabled. Prepare only one
    // disabled feature's activation bit, using the same durable baseline guard.
    // The production module then has a real change to suppress and restore.
    let mut preparation = None;
    if !changes.iter().any(|change| {
        module
            .read(&change.key)
            .is_ok_and(|value| value != change.desired)
    }) {
        let mut change = changes
            .first()
            .cloned()
            .ok_or("No disabled accessibility feature is available for an actual round trip")?;
        let baseline = checked(module.read(&change.key))?;
        let flags = u32::from_le_bytes(
            baseline
                .as_slice()
                .try_into()
                .map_err(|_| "Invalid hotkey token")?,
        );
        change.desired = (flags | 4).to_le_bytes().to_vec();
        let prepared = Arc::new(PrepareAccessibilityHotkey {
            native: module.clone(),
            change: change.clone(),
        });
        let mut prepare_registry = Registry::default();
        checked(prepare_registry.register(prepared.clone()))?;
        let directory = native_directory("accessibility_preparation")?;
        let saved = save_intent(&directory, &target, &config)?;
        let mut controller = checked(Controller::restore(
            Box::new(checked(DiskRecoveryStore::open(&directory))?),
            checked(source_identifier(&saved.path))?,
            &prepare_registry,
        ))?;
        let rollback = Rollback {
            module: module.clone(),
            fields: vec![OriginalField {
                change: change.clone(),
                original: baseline,
            }],
            armed: true,
        };
        checked(controller.acquire(
            saved.owner.clone(),
            prepared,
            target.clone(),
            config.clone(),
        ))?;
        assert_eq!(checked(module.read(&change.key))?, change.desired);
        preparation = Some((controller, saved.owner, rollback));
    }
    let unaffected = without_hotkeys(original.clone());
    let result = roundtrip(registry, module, target, config, || {
        assert_eq!(
            without_hotkeys(checked(gaming::read_user_setting_validation(&user))?),
            unaffected
        );
        Ok(())
    });
    let cleanup = (|| {
        if let Some((controller, owner, _)) = &mut preparation {
            checked(controller.release(owner))?;
        }
        assert_eq!(
            checked(gaming::read_user_setting_validation(&user))?,
            original
        );
        if let Some((_, _, rollback)) = &mut preparation {
            rollback.armed = false;
        }
        Ok(())
    })();
    result.and(cleanup)
}

/// Test-only precondition for the one accessibility round trip, using real SPI.
struct PrepareAccessibilityHotkey {
    native: Arc<dyn Module>,
    change: FieldChange,
}
impl Module for PrepareAccessibilityHotkey {
    fn descriptor(&self) -> ModuleDescriptor {
        self.native.descriptor()
    }
    fn editor(&self) -> &dyn super::presentation::ModuleEditor {
        self.native.editor()
    }
    fn targets(&self) -> Vec<TargetIdentity> {
        self.native.targets()
    }
    fn observe(&self, target: &TargetIdentity) -> super::platform::registry::ModuleObservation {
        self.native.observe(target)
    }
    fn fields(
        &self,
        _target: &TargetIdentity,
        _config: &ModuleEnvelope,
    ) -> Result<Vec<FieldChange>, ModuleError> {
        Ok(vec![self.change.clone()])
    }
    fn read(&self, key: &ResourceKey) -> Result<Vec<u8>, ModuleError> {
        self.native.read(key)
    }
    fn write(&self, key: &ResourceKey, expected: &[u8], desired: &[u8]) -> Result<(), ModuleError> {
        self.native.write(key, expected, desired)
    }
}

#[test]
#[ignore = "ROOT only: actual owned child QoS apply/disk reload/disable/rollback; child is killed and waited"]
fn native_roundtrip_process_qos() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    child_roundtrip("process_qos", |process| {
        let current = checked(gaming::read_process_policy(
            process,
            gaming::EXECUTION_SPEED,
        ))?;
        Ok(
            json!({"policy": if current.control == 1 && current.state == 1 { "DisableThrottling" } else { "EcoQos" }}),
        )
    })
}
#[test]
#[ignore = "ROOT only: actual owned child timer policy apply/disk reload/disable/rollback; creates no timer request"]
fn native_roundtrip_process_timer_policy() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    child_roundtrip("process_timer_policy", |_| {
        Ok(json!({"honor_existing_requests": true}))
    })
}
#[test]
#[ignore = "ROOT only: actual owned child real CPU Set ID apply/disk reload/disable/rollback; preserves original empty/nonempty assignment"]
fn native_roundtrip_cpu_sets() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    child_roundtrip("cpu_sets", |process| {
        let original = checked(gaming::read_cpu_sets(process))?;
        let topology = checked(gaming::cpu_topology(Some(process)))?;
        let alternate = topology
            .iter()
            .filter(|cpu| !cpu.parked && (!cpu.allocated || cpu.allocated_to_target))
            .find(|cpu| original.as_slice() != [cpu.id])
            .ok_or("Unsupported/not ready: no alternative permitted real CPU Set ID")?;
        Ok(json!({"ids": [alternate.id]}))
    })
}
#[test]
#[ignore = "ROOT only: actual owned child modest priority apply/disk reload/disable/rollback; never requests Realtime"]
fn native_roundtrip_process_priority() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    child_roundtrip("process_priority", |process| {
        let original = checked(gaming::read_priority(process))?;
        Ok(json!({"priority": if original == 0x4000 { "Normal" } else { "BelowNormal" }}))
    })
}

#[test]
#[ignore = "ROOT only: actual priority-owned child positive exit retires durable process evidence without accessing a dead PID"]
fn native_roundtrip_process_priority_positive_exit() -> TestResult {
    let _serial = NATIVE_SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (mut child, process) = OwnedChild::spawn()?;
    let expected = checked(OS::tuning_process_identity(process.pid))?;
    assert_eq!(expected.creation_time(), process.creation_token);
    assert_eq!(expected.image_path(), process.image_path);
    let mut lifetime = checked(OS::tuning_open_process_lifetime(&expected))?;
    assert_eq!(
        checked(lifetime.poll())?,
        os_api::tuning::TuningProcessLifetime::VerifiedLive
    );
    let handoff = Arc::new(SchedulingHandoff::default());
    checked(handoff.started(
        process.pid,
        process.creation_token,
        ProcessSettingsFields {
            affinity: false,
            priority: false,
        },
    ))?;
    let registry = tweaks::registry_with_scheduling(handoff);
    let module = module(&registry, "process_priority");
    let original = checked(gaming::read_priority(&process))?;
    let config = config(
        &module,
        json!({"priority": if original == 0x4000 { "Normal" } else { "BelowNormal" }}),
    );
    let target = process_target(&process);
    let change = checked(module.fields(&target, &config))?.pop().unwrap();
    let mut rollback = Rollback {
        module: module.clone(),
        fields: vec![OriginalField {
            original: original.to_le_bytes().to_vec(),
            change,
        }],
        armed: false,
    };
    let directory = native_directory("process_priority_positive_exit")?;
    let saved = save_intent(&directory, &target, &config)?;
    let source = checked(source_identifier(&saved.path))?;
    let mut controller = checked(Controller::restore(
        Box::new(checked(DiskRecoveryStore::open(&directory))?),
        source.clone(),
        &registry,
    ))?;
    rollback.armed = true;
    checked(controller.acquire_session(
        saved.owner.clone(),
        module.clone(),
        target,
        config,
        saved.activation.unwrap(),
    ))?;
    assert_eq!(
        checked(module.read(&rollback.fields[0].change.key))?,
        rollback.fields[0].change.desired
    );
    assert_eq!(controller.fields()[0].baseline, rollback.fields[0].original);
    assert!(!checked(
        controller.release_if_exited(&saved.owner, LifetimeEvidence::Unknown)
    )?);
    assert!(!controller.is_empty());
    checked(child.kill_and_wait())?;
    assert_eq!(
        checked(lifetime.poll())?,
        os_api::tuning::TuningProcessLifetime::PositivelyExited
    );
    // The actual process object has expired. Do not query or restore its PID after this point.
    rollback.armed = false;
    assert!(checked(
        controller.release_if_exited(&saved.owner, LifetimeEvidence::Exited)
    )?);
    assert!(controller.is_empty());
    drop(controller);
    let fresh_registry = tweaks::registry();
    let controller = checked(Controller::restore(
        Box::new(checked(DiskRecoveryStore::open(&directory))?),
        source,
        &fresh_registry,
    ))?;
    assert!(controller.is_empty());
    assert!(controller.owners().is_empty());
    assert!(controller.can_admit());
    Ok(())
}

fn without_hotkeys(mut value: gaming::UserSettingValidation) -> gaming::UserSettingValidation {
    value.sticky_flags &= !gaming::ACCESSIBILITY_HOTKEY_MASK;
    value.filter_flags &= !gaming::ACCESSIBILITY_HOTKEY_MASK;
    value.toggle_flags &= !gaming::ACCESSIBILITY_HOTKEY_MASK;
    value
}
