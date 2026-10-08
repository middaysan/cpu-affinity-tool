//! Public application flows over actual saved fixture bytes. These tests supply
//! no native proof and do not establish canonical process-restart admission.
use super::{
    editors,
    scalar::{ScalarConfig, ScalarModule},
    user_setting::{UserConfig, UserModule},
};
use crate::app::{
    features::{
        rules::RulesContext,
        tuning::{
            config::{ModulePreference, TuningLoad},
            platform::{
                catalog::{Catalog, Module},
                codec::ModuleEnvelope,
                contract::*,
                coordinator::OwnerView,
                runner::ShutdownResult,
            },
            presentation::EditorDraft,
            service::{SavedState, StatePersistence},
            HeadlessTuning, ModuleCard, SaveError, TuningConfigurationState, TuningOperationState,
            TuningRequest, TuningRuntime, TuningSnapshot,
        },
    },
    models::{AppStateStorage, AppToRun, CoreGroup},
    runtime::AppState,
    shell::presenters::{shared_elements::ui_font_definitions, tuning},
};
use eframe::egui;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex, RwLock,
    },
    thread,
    time::{Duration, Instant},
};

const FILE_LIMIT: u64 = 16 * 1024 * 1024;
const POLL_LIMIT: Duration = Duration::from_secs(8);

#[test]
fn p4_review_delayed_adoption_refuses_stale_base_candidate_without_rebasing() {
    let mut flow = Flow::new(false);
    let initial = flow.app.persistent_state.read().unwrap().clone();
    let initial_ids = flow.app.rules.to_persisted_identities();
    let mut candidate = initial.clone();
    candidate.theme_index = 1;
    flow.app
        .request_tuning(TuningRequest::SaveBase {
            candidate: Box::new(candidate),
            expected_revision: 0,
        })
        .unwrap();
    let until = Instant::now() + POLL_LIMIT;
    while matches!(
        flow.app.tuning_snapshot().operation,
        TuningOperationState::Pending { .. }
    ) {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(flow.app.tuning_snapshot().state.theme_index, 1);
    assert_eq!(
        flow.app.get_theme_index(),
        0,
        "adoption was deliberately delayed"
    );
    let saved_before = bounded_read(&flow.state_path).unwrap();
    let writes_before = flow.io.writers.lock().unwrap().len();

    flow.app.toggle_process_monitoring();

    assert!(matches!(
        flow.app.tuning_snapshot().operation,
        TuningOperationState::Refused {
            error: SaveError::Pending,
            ..
        }
    ));
    assert_eq!(bounded_read(&flow.state_path).unwrap(), saved_before);
    assert_eq!(flow.io.writers.lock().unwrap().len(), writes_before);
    assert_eq!(flow.app.get_theme_index(), 1);
    assert_eq!(flow.app.rules.to_persisted_identities(), initial_ids);
    assert_eq!(
        flow.app
            .persistent_state
            .read()
            .unwrap()
            .process_monitoring_enabled,
        initial.process_monitoring_enabled
    );

    // A newly adopted draft may retry, retaining the already saved theme.
    flow.app.toggle_process_monitoring();
    let saved: AppStateStorage =
        serde_json::from_slice(&bounded_read(&flow.state_path).unwrap()).unwrap();
    assert_eq!(saved.theme_index, 1);
    assert_eq!(
        saved.process_monitoring_enabled,
        !initial.process_monitoring_enabled
    );
    flow.finish();
}

struct FixturePersistence {
    path: PathBuf,
    fail: Arc<AtomicBool>,
    io: Arc<FixtureIo>,
}

#[derive(Default)]
struct FixtureIo {
    next_gate: Mutex<Option<SaveGate>>,
    writers: Mutex<Vec<thread::ThreadId>>,
}

struct SaveGate {
    entered: mpsc::SyncSender<thread::ThreadId>,
    release: mpsc::Receiver<()>,
    completed: mpsc::SyncSender<thread::ThreadId>,
}

struct HeldSave {
    entered: mpsc::Receiver<thread::ThreadId>,
    release: Option<mpsc::SyncSender<()>>,
    completed: mpsc::Receiver<thread::ThreadId>,
}

impl HeldSave {
    fn release(&mut self) {
        self.release.take().unwrap().send(()).unwrap();
    }
}

impl Drop for HeldSave {
    fn drop(&mut self) {
        // A failed assertion may unblock the owned fixture, but it never erases
        // uncertain saved state or claims terminal completion.
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl StatePersistence for FixturePersistence {
    fn save(&mut self, candidate: &mut AppStateStorage) -> Result<SavedState, SaveError> {
        let writer = thread::current().id();
        self.io.writers.lock().unwrap().push(writer);
        if self.fail.load(Ordering::Acquire) {
            return Err(SaveError::Storage);
        }
        let gate = self.io.next_gate.lock().unwrap().take();
        if let Some(gate) = &gate {
            gate.entered.send(writer).map_err(|_| SaveError::Storage)?;
            gate.release
                .recv_timeout(POLL_LIMIT * 2)
                .map_err(|_| SaveError::Storage)?;
        }
        let bytes = serde_json::to_vec(candidate).map_err(|_| SaveError::Storage)?;
        if bytes.len() as u64 > FILE_LIMIT {
            return Err(SaveError::Storage);
        }
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|_| SaveError::Storage)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| SaveError::Storage)?;
        drop(file);
        let bytes = bounded_read(&self.path).map_err(|_| SaveError::Storage)?;
        if let Some(gate) = gate {
            gate.completed
                .send(writer)
                .map_err(|_| SaveError::Storage)?;
        }
        Ok(SavedState::Fixture(bytes))
    }
}

fn bounded_read(path: &PathBuf) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(std::io::Error::other("fixture state exceeds bound"));
    }
    Ok(bytes)
}

/// Files are removed explicitly after verified worker completion. A failed or
/// uncertain test deliberately leaves the saved state and journal for inspection.
struct Flow {
    app: AppState,
    state_path: PathBuf,
    journal_path: PathBuf,
    fail_save: Arc<AtomicBool>,
    io: Arc<FixtureIo>,
    scalar: ScalarModule,
    user: UserModule,
}

impl Flow {
    fn new(with_rules: bool) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let state_path = PathBuf::from("target").join(format!(
            "p5-app-flow-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let journal_path = state_path.with_extension("journal");
        assert!(!journal_path.exists());
        let mut initial = AppStateStorage::test_fixture(|state| {
            if with_rules {
                let app = AppToRun::new_installed(
                    "Same executable".into(),
                    "fixture!game".into(),
                    os_api::PriorityClass::Normal,
                    false,
                );
                state.groups.push(CoreGroup {
                    name: "Saved rules".into(),
                    cores: vec![0],
                    programs: vec![app.clone(), app],
                    is_hidden: false,
                    run_all_button: false,
                });
            }
        });
        initial.rule_identities =
            Some(RulesContext::from_storage(&initial).to_persisted_identities());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&state_path)
            .unwrap();
        file.write_all(&serde_json::to_vec(&initial).unwrap())
            .unwrap();
        file.sync_all().unwrap();
        drop(file);
        let scalar = ScalarModule::new(7);
        let user = UserModule::new(None, false);
        let fail_save = Arc::new(AtomicBool::new(false));
        let io = Arc::new(FixtureIo::default());
        let mut app = AppState::new_for_test(Arc::new(RwLock::new(initial.clone())), 4);
        let runtime = Self::runtime(
            &app,
            initial,
            &scalar,
            &user,
            (&state_path, &journal_path),
            &fail_save,
            &io,
        );
        app.replace_tuning_for_test(runtime, editors::registry(scalar.clone(), user.clone()));
        Self {
            app,
            state_path,
            journal_path,
            fail_save,
            io,
            scalar,
            user,
        }
    }

    fn runtime(
        app: &AppState,
        initial: AppStateStorage,
        scalar: &ScalarModule,
        user: &UserModule,
        paths: (&PathBuf, &PathBuf),
        fail: &Arc<AtomicBool>,
        io: &Arc<FixtureIo>,
    ) -> TuningRuntime {
        let mut catalog = Catalog::default();
        catalog.register(scalar.clone()).unwrap();
        catalog.register(user.clone()).unwrap();
        let service = HeadlessTuning::new(
            initial,
            Box::new(FixturePersistence {
                path: paths.0.clone(),
                fail: fail.clone(),
                io: io.clone(),
            }),
            catalog,
        );
        TuningRuntime::start_fixture(
            service,
            app.runtime.running_apps_handle(),
            paths.0.clone(),
            paths.1.clone(),
            vec![
                super::machine_target(),
                TargetIdentity::Session(super::session()),
            ],
        )
    }

    fn disk_state(&self) -> AppStateStorage {
        serde_json::from_slice(&bounded_read(&self.state_path).unwrap()).unwrap()
    }

    fn hold_next_save(&self) -> HeldSave {
        let (entered, entered_rx) = mpsc::sync_channel(1);
        let (release, release_rx) = mpsc::sync_channel(1);
        let (completed, completed_rx) = mpsc::sync_channel(1);
        let mut next = self.io.next_gate.lock().unwrap();
        assert!(next.is_none());
        *next = Some(SaveGate {
            entered,
            release: release_rx,
            completed,
        });
        HeldSave {
            entered: entered_rx,
            release: Some(release),
            completed: completed_rx,
        }
    }

    fn reload(&mut self) {
        quit(&mut self.app);
        let saved = self.disk_state();
        let TuningLoad::Known(config) = saved.tuning.decode() else {
            panic!("this flow reloads a saved, understood profile")
        };
        let expected_owners = config.activations.len();
        let runtime = Self::runtime(
            &self.app,
            saved,
            &self.scalar,
            &self.user,
            (&self.state_path, &self.journal_path),
            &self.fail_save,
            &self.io,
        );
        self.app.replace_tuning_for_test(
            runtime,
            editors::registry(self.scalar.clone(), self.user.clone()),
        );
        let until = Instant::now() + POLL_LIMIT;
        loop {
            self.app.poll_tuning();
            let loaded = self.app.tuning_snapshot();
            let observed = loaded
                .profiles
                .iter()
                .flat_map(|profile| &profile.modules)
                .all(|card| card.observed.is_some());
            if observed && loaded.runner.as_ref().is_some_and(|runner| {
                runner.coordinator.owners.len() == expected_owners
                    && matches!(runner.writer,
                        crate::app::features::tuning::platform::writer::WriterStatus::Clean
                        | crate::app::features::tuning::platform::writer::WriterStatus::Known { .. })
            }) {
                break;
            }
            assert!(
                Instant::now() < until,
                "saved fixture hydration was not published within its bound"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn finish(mut self) {
        quit(&mut self.app);
        if self.journal_path.exists() {
            std::fs::remove_file(&self.journal_path).unwrap();
        }
        std::fs::remove_file(&self.state_path).unwrap();
    }
}

fn settle(app: &mut AppState) -> Arc<TuningSnapshot> {
    let until = Instant::now() + POLL_LIMIT;
    loop {
        app.poll_tuning();
        let snapshot = app.tuning_snapshot();
        if !matches!(snapshot.operation, TuningOperationState::Pending { .. }) {
            return snapshot;
        }
        assert!(
            Instant::now() < until,
            "public operation did not finish within its bound"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn request(app: &mut AppState, request: TuningRequest) -> Arc<TuningSnapshot> {
    app.request_tuning(request).unwrap();
    let snapshot = settle(app);
    assert_eq!(snapshot.operation, TuningOperationState::Idle);
    snapshot
}

fn create_profile(app: &mut AppState) -> ProfileId {
    let snapshot = request(
        app,
        TuningRequest::CreateProfile {
            name: "Public profile".into(),
        },
    );
    assert_eq!(snapshot.configuration, TuningConfigurationState::Known);
    assert_eq!(snapshot.profiles.len(), 1);
    snapshot.profiles[0].id
}

fn preference(config: ModuleEnvelope, enabled: bool) -> ModulePreference {
    ModulePreference {
        feature: config.feature.clone(),
        config,
        enabled,
        mode: ActivationMode::Manual,
    }
}

fn card<'a>(
    snapshot: &'a TuningSnapshot,
    profile: ProfileId,
    feature: &FeatureId,
) -> &'a ModuleCard {
    snapshot
        .profiles
        .iter()
        .find(|p| p.id == profile)
        .unwrap()
        .modules
        .iter()
        .find(|m| &m.feature == feature)
        .unwrap()
}

fn saved_preference(app: &AppState, profile: ProfileId, feature: &FeatureId) -> ModulePreference {
    let snapshot = app.tuning_snapshot();
    let saved = card(&snapshot, profile, feature);
    ModulePreference {
        feature: saved.feature.clone(),
        enabled: saved.enabled,
        mode: saved.mode,
        config: saved.config.clone(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OriginalResourceProof {
    key: ResourceKey,
    token: DesiredStateToken,
    source: FeatureId,
    snapshot: Arc<ModuleEnvelope>,
    config: Option<Arc<ModuleEnvelope>>,
}

fn baselines(snapshot: &TuningSnapshot) -> Vec<OriginalResourceProof> {
    snapshot
        .runner
        .as_ref()
        .unwrap()
        .coordinator
        .resources
        .iter()
        .map(|r| OriginalResourceProof {
            key: r.key.clone(),
            token: r.baseline.clone(),
            source: r.baseline_source.clone(),
            snapshot: r.baseline_snapshot.clone(),
            config: r.baseline_config.clone(),
        })
        .collect()
}

fn assert_same_owner_identity(actual: &[OwnerView], expected: &[OwnerView]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.owner, expected.owner);
        assert_eq!(actual.binding, expected.binding);
        assert_eq!(actual.config, expected.config);
        assert_eq!(actual.target, expected.target);
        assert_eq!(actual.mode, expected.mode);
        assert_eq!(actual.source, expected.source);
        assert_eq!(actual.resources, expected.resources);
        assert_eq!(actual.status, expected.status);
    }
}

fn render(app: &mut AppState) -> Vec<String> {
    let context = egui::Context::default();
    context.set_fonts(ui_font_definitions());
    let output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, 1400.0),
            )),
            ..Default::default()
        },
        |ui| tuning::draw_tuning(app, ui),
    );
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

fn quit(app: &mut AppState) {
    let started = Instant::now();
    app.begin_tuning_shutdown();
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "Quit must return without waiting for worker I/O"
    );
    let until = Instant::now() + POLL_LIMIT;
    while !app.tuning_shutdown_complete() {
        app.poll_tuning();
        assert!(
            Instant::now() < until,
            "Quit did not complete within its bound; retain fixture evidence"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        app.tuning_shutdown_outcome(),
        Some(ShutdownResult::Complete)
    );
    app.poll_tuning();
}

#[test]
fn p5_public_scalar_profile_save_activate_edit_disable_reload_restores_first_baseline() {
    let mut flow = Flow::new(false);
    assert_eq!(
        flow.app.tuning_snapshot().configuration,
        TuningConfigurationState::Off
    );
    let profile = create_profile(&mut flow.app);
    let config12 = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 12 })
        .unwrap();
    let feature = config12.feature.clone();
    let snapshot = request(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config12, false)),
            expected: None,
        },
    );
    let available = card(&snapshot, profile, &feature);
    assert_eq!(available.capability, Capability::Available);
    assert_eq!(available.targets, vec![super::machine_target()]);
    assert!(available.observed.is_some());
    assert_eq!(flow.scalar.value(), 7);
    assert_eq!(
        flow.scalar.writes(),
        0,
        "saving a preference must not apply it"
    );
    let snapshot = request(
        &mut flow.app,
        TuningRequest::ActivateManual {
            profile,
            feature: feature.clone(),
            target: available.targets[0].clone(),
        },
    );
    assert_eq!(flow.scalar.value(), 12);
    assert_eq!(
        card(&snapshot, profile, &feature).status,
        Some(ModuleStatus::VerifiedActive(Effect::Immediate))
    );
    let owner = snapshot.runner.as_ref().unwrap().coordinator.owners[0].owner;
    let original = baselines(&snapshot);
    assert_eq!(original.len(), 1);
    assert!(render(&mut flow.app)
        .iter()
        .any(|text| text == "Verified active"));
    let expected = saved_preference(&flow.app, profile, &feature);
    let snapshot = request(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(
                flow.scalar
                    .config_envelope(&ScalarConfig { value: 13 })
                    .unwrap(),
                true,
            )),
            expected: Some(expected),
        },
    );
    assert_eq!(flow.scalar.value(), 13);
    assert_eq!(baselines(&snapshot), original);
    assert_eq!(
        snapshot.runner.as_ref().unwrap().coordinator.owners[0].owner,
        owner
    );
    let snapshot = request(&mut flow.app, TuningRequest::DisableProfile { profile });
    assert_eq!(flow.scalar.value(), 7);
    assert!(snapshot
        .runner
        .as_ref()
        .unwrap()
        .coordinator
        .owners
        .is_empty());
    assert!(!card(&snapshot, profile, &feature).enabled);
    assert_eq!(
        flow.disk_state().tuning.decode(),
        flow.app.persistent_state.read().unwrap().tuning.decode()
    );
    flow.reload();
    let loaded = flow.app.tuning_snapshot();
    assert_eq!(loaded.profiles[0].id, profile);
    assert_eq!(loaded.profiles[0].name, "Public profile");
    let loaded_card = card(&loaded, profile, &feature);
    assert!(!loaded_card.enabled);
    assert_eq!(
        flow.scalar
            .config_codec()
            .decode(&loaded_card.config)
            .unwrap(),
        ScalarConfig { value: 13 }
    );
    assert_eq!(loaded_card.capability, Capability::Available);
    assert_ne!(
        loaded_card.status,
        Some(ModuleStatus::VerifiedActive(Effect::Immediate))
    );
    assert!(render(&mut flow.app)
        .iter()
        .any(|text| text.contains("Saved: disabled")));
    assert_eq!(flow.scalar.value(), 7);
    flow.finish();
}

#[test]
fn p5_public_user_profile_edit_active_reload_disable_preserves_original_absence_and_restart() {
    let mut flow = Flow::new(false);
    let profile = create_profile(&mut flow.app);
    let config12 = flow
        .user
        .config_envelope(&UserConfig {
            value: 12,
            enabled: true,
        })
        .unwrap();
    let feature = config12.feature.clone();
    let snapshot = request(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config12, false)),
            expected: None,
        },
    );
    let available = card(&snapshot, profile, &feature);
    assert_eq!(available.capability, Capability::Available);
    assert_eq!(
        available.targets,
        vec![TargetIdentity::Session(super::session())]
    );
    assert_eq!(flow.user.native_value(), None);
    assert!(!flow.user.enabled());
    assert_eq!(flow.user.writes(), 0);
    let snapshot = request(
        &mut flow.app,
        TuningRequest::ActivateManual {
            profile,
            feature: feature.clone(),
            target: available.targets[0].clone(),
        },
    );
    assert_eq!(flow.user.value(), Some(12));
    assert!(flow.user.enabled());
    assert_eq!(
        card(&snapshot, profile, &feature).status,
        Some(ModuleStatus::StoredRestartRequired)
    );
    let owner = snapshot.runner.as_ref().unwrap().coordinator.owners[0].owner;
    let original = baselines(&snapshot);
    assert_eq!(
        original.len(),
        2,
        "both native fields retain independent original state"
    );
    assert!(render(&mut flow.app)
        .iter()
        .any(|text| text == "Saved · restart required"));
    let expected = saved_preference(&flow.app, profile, &feature);
    let snapshot = request(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(
                flow.user
                    .config_envelope(&UserConfig {
                        value: 13,
                        enabled: true,
                    })
                    .unwrap(),
                true,
            )),
            expected: Some(expected),
        },
    );
    assert_eq!(flow.user.value(), Some(13));
    assert_eq!(baselines(&snapshot), original);
    assert_eq!(
        snapshot.runner.as_ref().unwrap().coordinator.owners[0].owner,
        owner
    );
    let writes = flow.user.writes();
    flow.reload();
    let loaded = flow.app.tuning_snapshot();
    let loaded_card = card(&loaded, profile, &feature);
    assert!(loaded_card.enabled);
    assert_eq!(
        loaded_card.status,
        Some(ModuleStatus::StoredRestartRequired)
    );
    assert_eq!(
        flow.user
            .config_codec()
            .decode(&loaded_card.config)
            .unwrap(),
        UserConfig {
            value: 13,
            enabled: true
        }
    );
    assert_eq!(
        flow.user.writes(),
        writes,
        "verified fixture hydration must not rewrite target state"
    );
    assert_eq!(baselines(&loaded), original);
    assert_eq!(
        loaded.runner.as_ref().unwrap().coordinator.owners[0].owner,
        owner
    );
    let snapshot = request(&mut flow.app, TuningRequest::DisableProfile { profile });
    assert_eq!(
        flow.user.native_value(),
        None,
        "restore exact original absence rather than writing zero"
    );
    assert!(!flow.user.enabled());
    assert_eq!(
        snapshot
            .runner
            .as_ref()
            .unwrap()
            .coordinator
            .restoration_effect,
        Some(Effect::RestartRequired)
    );
    assert!(render(&mut flow.app)
        .iter()
        .any(|text| text == "Original settings restored; restart Windows to take effect."));
    assert_eq!(flow.scalar.value(), 7);
    assert_eq!(flow.scalar.writes(), 0);
    flow.finish();
}

#[test]
fn p5_public_associations_use_exact_rule_ids_and_failed_or_stale_saves_retain_state_and_draft() {
    let mut flow = Flow::new(true);
    let profile = create_profile(&mut flow.app);
    let config12 = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 12 })
        .unwrap();
    let feature = config12.feature.clone();
    request(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config12, false)),
            expected: None,
        },
    );
    let group = flow.app.rules.group_id_for_index(0).unwrap();
    let first = flow.app.rules.rule_id_for_index(0, 0).unwrap();
    let second = flow.app.rules.rule_id_for_index(0, 1).unwrap();
    assert_ne!(first, second);
    let snapshot = request(
        &mut flow.app,
        TuningRequest::BindRule {
            group: group.clone(),
            rule: second.clone(),
            profile,
            eligibility: ActivationEligibility::Running,
        },
    );
    assert_eq!(snapshot.associations.len(), 1);
    assert_eq!(snapshot.associations[0].group, group);
    assert_eq!(snapshot.associations[0].rule, second);
    assert_ne!(snapshot.associations[0].rule, first);
    let expected = saved_preference(&flow.app, profile, &feature);
    let config13 = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 13 })
        .unwrap();
    flow.app.ui.tuning.selected_profile = Some(profile);
    flow.app.ui.tuning.editor = Some(EditorDraft {
        profile,
        expected: Some(expected.clone()),
        candidate: preference(config13.clone(), false),
    });
    let before = bounded_read(&flow.state_path).unwrap();
    let before_ids = flow.app.rules.to_persisted_identities();
    flow.fail_save.store(true, Ordering::Release);
    tuning::submit(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config13.clone(), false)),
            expected: Some(expected.clone()),
        },
    );
    let refused = settle(&mut flow.app);
    assert!(matches!(
        refused.operation,
        TuningOperationState::Refused {
            error: SaveError::Storage,
            ..
        }
    ));
    assert_eq!(bounded_read(&flow.state_path).unwrap(), before);
    assert_eq!(flow.app.rules.to_persisted_identities(), before_ids);
    assert_eq!(saved_preference(&flow.app, profile, &feature), expected);
    let labels = render(&mut flow.app);
    assert!(labels
        .iter()
        .any(|text| text == tuning::save_error_message(SaveError::Storage)));
    let retained = flow.app.ui.tuning.editor.as_ref().unwrap();
    assert_eq!(retained.expected, Some(expected.clone()));
    assert_eq!(retained.candidate.config, config13);
    let mut candidate = flow.app.persistent_state.read().unwrap().clone();
    candidate.groups.push(CoreGroup {
        name: "Unsaved".into(),
        cores: vec![1],
        programs: vec![],
        is_hidden: false,
        run_all_button: false,
    });
    flow.app
        .request_tuning(TuningRequest::SaveBase {
            candidate: Box::new(candidate),
            expected_revision: flow.app.tuning_snapshot().revision,
        })
        .unwrap();
    let blocked = settle(&mut flow.app);
    assert_eq!(blocked.operation, TuningOperationState::Recovering);
    assert_eq!(bounded_read(&flow.state_path).unwrap(), before);
    assert_eq!(flow.app.persistent_state.read().unwrap().groups.len(), 1);
    assert_eq!(flow.app.rules.to_persisted_identities(), before_ids);
    let labels = render(&mut flow.app);
    assert!(labels
        .iter()
        .any(|text| text == "Recovery required. Changes are paused."));
    let retained = flow.app.ui.tuning.editor.as_ref().unwrap();
    assert_eq!(retained.expected, Some(expected.clone()));
    assert_eq!(retained.candidate.config, config13);
    flow.fail_save.store(false, Ordering::Release);
    flow.app
        .request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config13, false)),
            expected: Some(expected),
        })
        .unwrap();
    assert_eq!(
        settle(&mut flow.app).operation,
        TuningOperationState::Recovering
    );
    assert_eq!(bounded_read(&flow.state_path).unwrap(), before);
    assert_eq!(flow.app.rules.to_persisted_identities(), before_ids);
    flow.app.begin_tuning_shutdown();
    let until = Instant::now() + POLL_LIMIT;
    while flow.app.tuning_shutdown_outcome().is_none() {
        flow.app.poll_tuning();
        assert!(
            Instant::now() < until,
            "uncertain save must expose its bounded shutdown result"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        flow.app.tuning_shutdown_outcome(),
        Some(ShutdownResult::TimedOut)
    );
    assert!(!flow.app.tuning_shutdown_complete());
    assert!(flow.state_path.exists());
    assert_eq!(bounded_read(&flow.state_path).unwrap(), before);
    assert_eq!(flow.scalar.value(), 7);
    assert_eq!(flow.scalar.writes(), 0);
    // Publication was started before the failed save. Its retained worker/source
    // evidence cannot be erased or reused to demonstrate a successful retry.
    drop(flow);

    let mut flow = Flow::new(true);
    let profile = create_profile(&mut flow.app);
    let config12 = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 12 })
        .unwrap();
    let feature = config12.feature.clone();
    request(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config12, false)),
            expected: None,
        },
    );
    let group = flow.app.rules.group_id_for_index(0).unwrap();
    let second = flow.app.rules.rule_id_for_index(0, 1).unwrap();
    let before_ids = flow.app.rules.to_persisted_identities();
    request(
        &mut flow.app,
        TuningRequest::BindRule {
            group: group.clone(),
            rule: second.clone(),
            profile,
            eligibility: ActivationEligibility::Running,
        },
    );
    let expected = saved_preference(&flow.app, profile, &feature);
    let config13 = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 13 })
        .unwrap();
    flow.app.ui.tuning.selected_profile = Some(profile);
    flow.app.ui.tuning.editor = Some(EditorDraft {
        profile,
        expected: Some(expected.clone()),
        candidate: preference(config13.clone(), false),
    });
    request(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config13, false)),
            expected: Some(expected.clone()),
        },
    );
    let saved = bounded_read(&flow.state_path).unwrap();
    let config14 = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 14 })
        .unwrap();
    tuning::submit(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config14, false)),
            expected: Some(expected.clone()),
        },
    );
    let refused = settle(&mut flow.app);
    assert!(matches!(
        refused.operation,
        TuningOperationState::Refused {
            error: SaveError::Invalid,
            ..
        }
    ));
    assert_eq!(bounded_read(&flow.state_path).unwrap(), saved);
    assert!(render(&mut flow.app)
        .iter()
        .any(|text| text == "Saved settings changed. Reopen the editor before saving."));
    let snapshot = request(
        &mut flow.app,
        TuningRequest::UnbindRule {
            group: group.clone(),
            rule: second,
        },
    );
    assert!(snapshot.associations.is_empty());
    assert_eq!(flow.app.rules.to_persisted_identities(), before_ids);
    flow.reload();
    assert!(flow.app.tuning_snapshot().associations.is_empty());
    assert_eq!(flow.app.rules.to_persisted_identities(), before_ids);
    assert_eq!(flow.disk_state().rule_identities, Some(before_ids));
    assert_eq!(flow.scalar.value(), 7);
    assert_eq!(flow.scalar.writes(), 0);
    flow.finish();
}

#[test]
fn p5_public_rule_cleanup_and_bounded_quit_preserve_manual_owners_for_both_shapes() {
    let mut shipping = AppState::new_for_test(
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
        4,
    );
    assert!(
        shipping.tuning_snapshot().catalog.is_empty(),
        "shipping composition must not expose fixture modules"
    );
    assert!(render(&mut shipping)
        .iter()
        .any(|text| text == "No modules are available in this build."));
    quit(&mut shipping);
    let mut flow = Flow::new(true);
    let profile = create_profile(&mut flow.app);
    let scalar_config = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 12 })
        .unwrap();
    let user_config = flow
        .user
        .config_envelope(&UserConfig {
            value: 12,
            enabled: true,
        })
        .unwrap();
    for config in [scalar_config.clone(), user_config.clone()] {
        request(
            &mut flow.app,
            TuningRequest::SetModulePreference {
                profile,
                preference: Box::new(preference(config, false)),
                expected: None,
            },
        );
    }
    for (config, target) in [
        (scalar_config, super::machine_target()),
        (user_config, TargetIdentity::Session(super::session())),
    ] {
        request(
            &mut flow.app,
            TuningRequest::ActivateManual {
                profile,
                feature: config.feature,
                target,
            },
        );
    }
    let before = flow.app.tuning_snapshot();
    let owners = before.runner.as_ref().unwrap().coordinator.owners.clone();
    assert_eq!(owners.len(), 2);
    assert!(owners
        .iter()
        .all(|owner| matches!(owner.binding, ActivationBinding::Manual(_))));
    let original = baselines(&before);
    let group = flow.app.rules.group_id_for_index(0).unwrap();
    let rule = flow.app.rules.rule_id_for_index(0, 0).unwrap();
    request(
        &mut flow.app,
        TuningRequest::BindRule {
            group: group.clone(),
            rule: rule.clone(),
            profile,
            eligibility: ActivationEligibility::Running,
        },
    );
    request(
        &mut flow.app,
        TuningRequest::UnbindRule {
            group: group.clone(),
            rule: rule.clone(),
        },
    );
    request(&mut flow.app, TuningRequest::RemoveRule { group, rule });
    let after_cleanup = flow.app.tuning_snapshot();
    assert_same_owner_identity(
        &after_cleanup.runner.as_ref().unwrap().coordinator.owners,
        &owners,
    );
    assert_eq!(baselines(&after_cleanup), original);
    let scalar_writes = flow.scalar.writes();
    let user_writes = flow.user.writes();
    let saved = bounded_read(&flow.state_path).unwrap();
    quit(&mut flow.app);
    assert_eq!(flow.scalar.value(), 12);
    assert_eq!(flow.user.value(), Some(12));
    assert!(flow.user.enabled());
    assert_eq!(flow.scalar.writes(), scalar_writes);
    assert_eq!(flow.user.writes(), user_writes);
    assert_eq!(bounded_read(&flow.state_path).unwrap(), saved);
    assert_same_owner_identity(
        &flow
            .app
            .tuning_snapshot()
            .runner
            .as_ref()
            .unwrap()
            .coordinator
            .owners,
        &owners,
    );
    flow.reload();
    assert_eq!(flow.scalar.writes(), scalar_writes);
    assert_eq!(flow.user.writes(), user_writes);
    let loaded = flow.app.tuning_snapshot();
    // Hydration reconstructs the current owner snapshot. Original restoration
    // evidence remains in the independent resource baselines checked below.
    assert_same_owner_identity(&loaded.runner.as_ref().unwrap().coordinator.owners, &owners);
    assert_eq!(baselines(&loaded), original);
    request(&mut flow.app, TuningRequest::DisableProfile { profile });
    assert_eq!(flow.scalar.value(), 7);
    assert_eq!(flow.user.native_value(), None);
    assert!(!flow.user.enabled());
    let TuningLoad::Known(saved_config) = flow.disk_state().tuning.decode() else {
        panic!("saved profile must remain understood")
    };
    assert!(saved_config.activations.is_empty());
    assert!(saved_config.withdrawals.is_empty());
    assert!(saved_config.transitions.unwrap().is_empty());
    flow.finish();
}

#[test]
fn p5_public_blocked_save_keeps_ui_pending_and_timeout_retains_same_worker_until_completion() {
    let mut flow = Flow::new(false);
    let profile = create_profile(&mut flow.app);
    let config12 = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 12 })
        .unwrap();
    let feature = config12.feature.clone();
    request(
        &mut flow.app,
        TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config12, false)),
            expected: None,
        },
    );
    let before = request(
        &mut flow.app,
        TuningRequest::ActivateManual {
            profile,
            feature: feature.clone(),
            target: super::machine_target(),
        },
    );
    let owners = before.runner.as_ref().unwrap().coordinator.owners.clone();
    let original = baselines(&before);
    let state = bounded_read(&flow.state_path).unwrap();
    let ids = flow.app.rules.to_persisted_identities();
    let writes = flow.scalar.writes();
    let expected = saved_preference(&flow.app, profile, &feature);
    let config13 = flow
        .scalar
        .config_envelope(&ScalarConfig { value: 13 })
        .unwrap();
    let mut gate = flow.hold_next_save();
    let started = Instant::now();
    let pending_id = flow
        .app
        .request_tuning(TuningRequest::SetModulePreference {
            profile,
            preference: Box::new(preference(config13.clone(), true)),
            expected: Some(expected),
        })
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "GUI request must enqueue without waiting for storage"
    );
    let writer = gate
        .entered
        .recv_timeout(POLL_LIMIT)
        .expect("save must reach the explicit fixture gate");
    assert_ne!(writer, thread::current().id());
    assert!(flow
        .io
        .writers
        .lock()
        .unwrap()
        .iter()
        .all(|saved| *saved == writer));
    let reads = flow.scalar.reads();
    let started = Instant::now();
    flow.app.poll_tuning();
    let pending = flow.app.tuning_snapshot();
    let labels = render(&mut flow.app);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "saved snapshots and rendering must stay responsive during storage I/O"
    );
    assert_eq!(
        pending.operation,
        TuningOperationState::Pending { id: pending_id }
    );
    assert!(labels
        .iter()
        .any(|text| text == "Saving or applying settings…"));
    assert_eq!(pending.revision, before.revision);
    assert_eq!(bounded_read(&flow.state_path).unwrap(), state);
    assert_eq!(flow.app.rules.to_persisted_identities(), ids);
    assert_same_owner_identity(
        &pending.runner.as_ref().unwrap().coordinator.owners,
        &owners,
    );
    assert_eq!(baselines(&pending), original);
    assert_eq!(flow.scalar.value(), 12);
    assert_eq!(flow.scalar.writes(), writes);
    assert_eq!(
        flow.scalar.reads(),
        reads,
        "presenter reads only immutable published cards"
    );
    assert_eq!(
        flow.app.request_tuning(TuningRequest::ActivateManual {
            profile,
            feature: feature.clone(),
            target: super::machine_target(),
        }),
        Err(SaveError::Pending)
    );

    let started = Instant::now();
    flow.app.begin_tuning_shutdown();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "Quit must not wait for blocked persistence"
    );
    let until = Instant::now() + POLL_LIMIT;
    while flow.app.tuning_shutdown_outcome().is_none() {
        flow.app.poll_tuning();
        assert!(
            Instant::now() < until,
            "blocked I/O must expose the bounded Quit outcome"
        );
        assert_eq!(
            gate.completed.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
    }
    let timed_out = flow.app.tuning_snapshot();
    assert_eq!(timed_out.shutdown_outcome, Some(ShutdownResult::TimedOut));
    assert!(!timed_out.shutdown_complete);
    assert_eq!(
        timed_out.operation,
        TuningOperationState::Pending { id: pending_id }
    );
    assert_eq!(timed_out.revision, before.revision);
    assert_eq!(baselines(&timed_out), original);
    assert_eq!(bounded_read(&flow.state_path).unwrap(), state);
    assert_eq!(flow.scalar.value(), 12);
    assert_eq!(flow.scalar.writes(), writes);
    assert!(flow.state_path.exists());
    assert_eq!(
        flow.app.request_tuning(TuningRequest::ActivateManual {
            profile,
            feature: feature.clone(),
            target: super::machine_target(),
        }),
        Err(SaveError::Recovering)
    );

    gate.release();
    assert_eq!(
        gate.completed
            .recv_timeout(POLL_LIMIT)
            .expect("released save must reach actual synced-file completion"),
        writer
    );
    quit(&mut flow.app);
    let completed = flow.app.tuning_snapshot();
    assert_eq!(completed.operation, TuningOperationState::Idle);
    assert!(completed.revision > before.revision);
    assert_eq!(card(&completed, profile, &feature).config, config13);
    assert_eq!(flow.app.rules.to_persisted_identities(), ids);
    assert_eq!(
        flow.disk_state().tuning.decode(),
        flow.app.persistent_state.read().unwrap().tuning.decode()
    );
    assert_eq!(baselines(&completed), original);
    let retained = &completed.runner.as_ref().unwrap().coordinator.owners;
    assert_eq!(retained.len(), owners.len());
    assert_eq!(retained[0].owner, owners[0].owner);
    assert_eq!(retained[0].target, owners[0].target);
    assert_eq!(retained[0].mode, ActivationMode::Manual);
    assert_eq!(retained[0].source, owners[0].source);
    assert_eq!(retained[0].resources, owners[0].resources);
    assert_eq!(
        retained[0].status,
        ModuleStatus::VerifiedActive(Effect::Immediate)
    );
    let ActivationBinding::Manual(binding) = &retained[0].binding else {
        panic!("accepted edit must keep Manual binding")
    };
    assert_eq!(binding.profile, profile);
    assert!(binding.generation.get() > owners[0].binding.configured().unwrap().generation.get());
    assert_eq!(retained[0].config.as_ref().unwrap().as_ref(), &config13);
    assert_eq!(flow.scalar.value(), 13);
    let observed = flow.scalar.observe(&retained[0].target).unwrap();
    let tokens = flow
        .scalar
        .observation_states(&retained[0].target, &observed, &retained[0].resources)
        .unwrap();
    for resource in &completed.runner.as_ref().unwrap().coordinator.resources {
        let actual = &tokens
            .iter()
            .find(|token| token.key == resource.key)
            .unwrap()
            .value;
        assert_eq!(&resource.confirmed, actual);
        assert_eq!(&resource.desired, actual);
        assert_eq!(
            resource.current_config.as_ref().unwrap().as_ref(),
            &config13
        );
    }
    assert_eq!(flow.scalar.writes(), writes + 1);
    assert!(
        flow.io
            .writers
            .lock()
            .unwrap()
            .iter()
            .all(|saved| *saved == writer),
        "timeout must retain the same transaction worker"
    );
    flow.finish();
}
