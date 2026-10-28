//! A single write gate, fed by the existing configuration writer and discovery loop.
use super::policy::{image_key, Policy};
use crate::app::{
    features::{
        rules::{known_apps, RulesContext},
        system_tweaks::tweaks::scheduling::fields_for_rule,
    },
    models::{AppRuntimeKey, AppStateStorage, AppToRun, RunningApps},
    shared::ids::{GroupId, RuleId},
};
use os_api::{gaming::ProcessSettingsFields, PriorityClass};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, RwLock},
};
type Instance = (u32, u64);
const MAX_INSTANCES: usize = 4096;
const MAX_ROWS: usize = 128;
const MAX_CLAIMANTS: usize = 64;
const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct RuleInfo {
    pub rule: RuleId,
    pub group: GroupId,
    pub label: String,
    pub key: AppRuntimeKey,
    pub app: AppToRun,
    pub mask: usize,
    pub fields: ProcessSettingsFields,
    selector: Selector,
}
#[derive(Clone, Debug)]
enum Selector {
    Names {
        names: HashSet<String>,
        primary: Option<(String, String)>,
    },
    Known {
        manual: Vec<String>,
        scope: Option<known_apps::Scope>,
    },
    Observed,
}
fn selector(app: &AppToRun) -> Selector {
    if app.steam_game().is_some() {
        return Selector::Observed;
    }
    if app.builtin_process_binding.is_some() {
        let scope = if cfg!(target_os = "windows") {
            known_apps::project(app).ok().map(|mut scope| {
                scope.root = PathBuf::from(canonical(&scope.root));
                scope
            })
        } else {
            None
        };
        return Selector::Known {
            manual: known_apps::manual_names(app),
            scope,
        };
    }
    let primary = app.bin_path().and_then(|path| {
        app.primary_process_name_normalized()
            .map(|name| (name, canonical(path)))
    });
    Selector::Names {
        names: app
            .additional_processes
            .iter()
            .map(|name| crate::app::models::normalize_process_name(name))
            .collect(),
        primary,
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Notice {
    pub image: String,
    pub name: String,
    pub claimants: Vec<RuleId>,
    pub owner: Option<RuleId>,
    pub different: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct View {
    pub rules: Arc<Vec<RuleInfo>>,
    pub notices: Vec<Notice>,
    pub error: Option<String>,
}

pub(crate) struct Authority {
    configuration: RwLock<()>,
    pub(crate) gate: Mutex<Gate>,
    view: RwLock<Arc<View>>,
}
impl Default for Authority {
    fn default() -> Self {
        Self {
            configuration: RwLock::new(()),
            gate: Mutex::new(Gate::default()),
            view: RwLock::new(Arc::new(View::default())),
        }
    }
}
#[derive(Default)]
pub(crate) struct Gate {
    policy: Option<Policy>,
    rules: Arc<Vec<RuleInfo>>,
    members: HashMap<Instance, Vec<RuleId>>,
    images: HashMap<Instance, String>,
    pending: HashMap<String, Vec<RuleId>>,
    generation: u64,
    monitoring_enabled: bool,
    error: Option<String>,
}

fn canonical(path: &Path) -> String {
    image_key(
        &path
            .canonicalize()
            .unwrap_or_else(|_| path.into())
            .to_string_lossy(),
    )
}
pub(crate) fn differs(a: &RuleInfo, b: &RuleInfo) -> bool {
    (a.fields.affinity && b.fields.affinity && a.mask != b.mask)
        || (a.fields.priority && b.fields.priority && a.app.priority != b.app.priority)
}

impl Gate {
    fn source(&self, source: SourceRequest<'_>) -> Result<&RuleInfo, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let spec = self
            .rules
            .iter()
            .find(|spec| &spec.rule == source.rule)
            .ok_or("Launch rule was removed")?;
        if &spec.group != source.group
            || !spec.key.namespace_equivalent(source.key, source.rule)
            || spec.fields != source.fields
            || (source.fields.affinity && spec.mask != source.mask)
            || spec.app.priority != source.app.priority
            || spec.app.launch_target != source.app.launch_target
            || spec.app.additional_processes != source.app.additional_processes
            || spec.app.manage_descendants != source.app.manage_descendants
            || spec.app.builtin_process_binding != source.app.builtin_process_binding
        {
            return Err("Launch source changed; discard stale correction".into());
        }
        Ok(spec)
    }
    pub fn configure(&mut self, state: &AppStateStorage) {
        let policy = Policy::read(state);
        let generation = policy.as_ref().map_or(u64::MAX, Policy::generation);
        self.generation = generation;
        self.monitoring_enabled = state.process_monitoring_enabled;
        self.error = policy.as_ref().err().cloned();
        self.policy = policy.ok();
        let rules = RulesContext::from_storage(state);
        let mut specs = Vec::new();
        for (g, group) in state.groups.iter().enumerate() {
            for (r, app) in group.programs.iter().enumerate() {
                if specs.len() >= 4096 {
                    self.error = Some("Too many rules for process control".into());
                    break;
                }
                let Some((group_id, rule)) = rules
                    .group_id_for_index(g)
                    .zip(rules.rule_id_for_index(g, r))
                else {
                    continue;
                };
                let mask = group
                    .cores
                    .iter()
                    .try_fold(0usize, |mask, &cpu| {
                        1usize.checked_shl(cpu as u32).map(|bit| mask | bit)
                    })
                    .unwrap_or(0);
                specs.push(RuleInfo {
                    key: rules.runtime_key(app, &rule),
                    fields: fields_for_rule(state, &group_id, &rule),
                    rule,
                    group: group_id,
                    label: format!(
                        "{} / {}",
                        group.name.chars().take(128).collect::<String>(),
                        app.name.chars().take(128).collect::<String>()
                    ),
                    selector: selector(app),
                    app: app.clone(),
                    mask,
                });
            }
        }
        let unchanged: HashSet<_> = self
            .rules
            .iter()
            .filter(|old| {
                specs.iter().any(|new| {
                    new.rule == old.rule
                        && new.app.launch_target == old.app.launch_target
                        && new.app.additional_processes == old.app.additional_processes
                        && new.app.manage_descendants == old.app.manage_descendants
                        && new.app.builtin_process_binding == old.app.builtin_process_binding
                })
            })
            .map(|rule| rule.rule.clone())
            .collect();
        self.members.retain(|_, ids| {
            ids.retain(|id| unchanged.contains(id));
            !ids.is_empty()
        });
        self.images
            .retain(|instance, _| self.members.contains_key(instance));
        self.rules = Arc::new(specs);
        for (image, mut ids) in std::mem::take(&mut self.pending) {
            ids.retain(|id| {
                self.rules
                    .iter()
                    .any(|rule| &rule.rule == id && self.admits_source(rule, &image))
            });
            if ids.len() > 1 {
                self.pending.insert(image, ids);
            }
        }
    }
    fn possible(&self, rule: &RuleInfo, image: &str) -> bool {
        let name = image
            .rsplit(['\\', '/'])
            .next()
            .map(crate::app::models::normalize_process_name)
            .unwrap_or_default();
        match &rule.selector {
            Selector::Observed => false,
            Selector::Names { names, primary } => {
                names.contains(&name)
                    && primary
                        .as_ref()
                        .is_none_or(|(primary_name, path)| primary_name != &name || path == image)
            }
            Selector::Known { manual, scope } => {
                manual.contains(&name)
                    || scope.as_ref().is_some_and(|scope| {
                        let path = Path::new(image);
                        scope.enabled(path)
                            && scope
                                .witness_role_under_root(&scope.root, path)
                                .is_some_and(|role| {
                                    role == known_apps::MemberRole::Main
                                        || self.members.values().any(|ids| ids.contains(&rule.rule))
                                })
                    })
            }
        }
    }

    fn admits_source(&self, rule: &RuleInfo, image: &str) -> bool {
        self.possible(rule, image)
            || match &rule.selector {
                Selector::Names {
                    primary: Some((_, primary)),
                    ..
                } => primary == image,
                _ => false,
            }
    }
    fn resolve(&self, image: &str, claimants: &[RuleId]) -> Result<Option<RuleId>, String> {
        if claimants.len() > MAX_CLAIMANTS {
            return Err("Too many overlapping rules for one process".into());
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let policy = self
            .policy
            .as_ref()
            .ok_or("Process control is not initialized")?;
        if let Some(choice) = policy
            .choices
            .iter()
            .find(|choice| choice.image == image && claimants.contains(&choice.owner))
        {
            return Ok(Some(choice.owner.clone()));
        }
        let applicable: Vec<_> = self
            .rules
            .iter()
            .filter(|rule| claimants.contains(&rule.rule))
            .collect();
        for (index, a) in applicable.iter().enumerate() {
            for b in &applicable[index + 1..] {
                if policy.legacy.contains(&a.rule)
                    && policy.legacy.contains(&b.rule)
                    && differs(a, b)
                {
                    return Ok(None);
                }
            }
        }
        Ok(applicable
            .into_iter()
            .min_by_key(|rule| policy.rank(&rule.rule))
            .map(|rule| rule.rule.clone()))
    }
    fn claimants(
        &self,
        instance: Option<Instance>,
        image: &str,
        caller: Option<&RuleId>,
    ) -> Vec<RuleId> {
        let mut ids = instance
            .and_then(|instance| self.members.get(&instance))
            .cloned()
            .unwrap_or_default();
        if !image.is_empty() {
            for rule in self.rules.iter() {
                if self.possible(rule, image) && !ids.contains(&rule.rule) {
                    ids.push(rule.rule.clone());
                }
            }
        }
        if let Some(caller) = caller {
            if !ids.contains(caller) {
                ids.push(caller.clone());
            }
        }
        ids.retain(|id| self.rules.iter().any(|rule| &rule.rule == id));
        ids
    }
    fn notice(&self, image: String, claimants: Vec<RuleId>) -> Option<Notice> {
        if claimants.len() < 2
            || claimants.len() > MAX_CLAIMANTS
            || image.is_empty()
            || image.len() > 16000
        {
            return None;
        }
        let rules: Vec<_> = self
            .rules
            .iter()
            .filter(|rule| claimants.contains(&rule.rule))
            .collect();
        let different = rules
            .iter()
            .enumerate()
            .any(|(index, a)| rules[index + 1..].iter().any(|b| differs(a, b)));
        let owner = self.resolve(&image, &claimants).ok().flatten();
        Some(Notice {
            name: image.rsplit(['\\', '/']).next().unwrap_or(&image).into(),
            image,
            claimants,
            owner,
            different,
        })
    }
}

pub(crate) struct WriteRequest<'a> {
    pub rule: &'a RuleId,
    pub key: &'a AppRuntimeKey,
    pub pid: u32,
    pub token: u64,
    pub image: Option<&'a Path>,
    pub mask: usize,
    pub priority: PriorityClass,
    pub fields: ProcessSettingsFields,
    pub change: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct SourceRequest<'a> {
    pub group: &'a GroupId,
    pub rule: &'a RuleId,
    pub key: &'a AppRuntimeKey,
    pub app: &'a AppToRun,
    pub mask: usize,
    pub fields: ProcessSettingsFields,
}
impl Authority {
    pub fn configuration_write(&self) -> Result<std::sync::RwLockWriteGuard<'_, ()>, String> {
        self.configuration
            .write()
            .map_err(|_| "Configuration admission is unavailable".into())
    }
    /// Called with RunningApps already held. Never wait behind a durable save.
    pub fn observation(
        &self,
        generation: u64,
        monitoring: Option<bool>,
    ) -> Result<std::sync::RwLockReadGuard<'_, ()>, String> {
        let lease = self
            .configuration
            .try_read()
            .map_err(|_| "Configuration is changing")?;
        let gate = self.lock()?;
        if gate.generation != generation
            || monitoring.is_some_and(|enabled| enabled != gate.monitoring_enabled)
        {
            return Err("Discard stale process observation".into());
        }
        Ok(lease)
    }
    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        self.lock().map_or(u64::MAX, |gate| gate.generation)
    }
    pub fn with_source<T>(
        &self,
        source: SourceRequest<'_>,
        publish: impl FnOnce(&RuleInfo) -> Result<T, String>,
    ) -> Result<T, String> {
        let gate = self.lock()?;
        publish(gate.source(source)?)
    }
    pub fn lock(&self) -> Result<MutexGuard<'_, Gate>, String> {
        self.gate
            .lock()
            .map_err(|_| "Process control is unavailable".into())
    }
    pub fn view(&self) -> Arc<View> {
        self.view
            .read()
            .map(|value| value.clone())
            .unwrap_or_default()
    }
    pub fn publish_view(&self, gate: &Gate) {
        // Resolve each real overlap set before presenting the executable. Disjoint
        // lifetimes can share an image without sharing a controller decision.
        let mut rows = std::collections::BTreeMap::new();
        let mut add = |image: &String, mut claims: Vec<RuleId>| {
            if claims.len() < 2 {
                return;
            }
            claims.sort_by(|a, b| a.0.cmp(&b.0));
            claims.dedup();
            let key = (
                image.clone(),
                claims.iter().map(|id| id.0.clone()).collect::<Vec<_>>(),
            );
            rows.entry(key).or_insert(claims);
        };
        for (&instance, image) in &gate.images {
            add(image, gate.claimants(Some(instance), image, None));
        }
        for (image, claims) in &gate.pending {
            if gate.resolve(image, claims).ok().flatten().is_none() {
                add(image, claims.clone());
            }
        }
        let notices = rows
            .into_iter()
            .filter_map(|((image, _), claims)| gate.notice(image, claims))
            .take(MAX_ROWS)
            .collect();
        if let Ok(mut view) = self.view.write() {
            *view = Arc::new(View {
                rules: gate.rules.clone(),
                notices,
                error: gate.error.clone(),
            });
        }
    }
    pub fn configure(&self, state: &AppStateStorage) {
        let Ok(_configuration) = self.configuration_write() else {
            return;
        };
        if let Ok(mut gate) = self.lock() {
            gate.configure(state);
            self.publish_view(&gate);
        }
    }
    pub fn initialize(&self, state: &AppStateStorage) {
        {
            let Ok(gate) = self.lock() else { return };
            if gate.policy.is_some() || gate.error.is_some() {
                return;
            }
        }
        let Ok(_configuration) = self.configuration_write() else {
            return;
        };
        if let Ok(mut gate) = self.lock() {
            if gate.policy.is_none() && gate.error.is_none() {
                gate.configure(state);
                self.publish_view(&gate);
            }
        }
    }
    pub fn member<'a>(
        &self,
        apps: &'a RunningApps,
        key: &AppRuntimeKey,
    ) -> Option<(&'a AppRuntimeKey, &'a crate::app::models::RunningApp)> {
        let view = self.view();
        if let Some(spec) = view.rules.iter().find(|spec| &spec.key == key) {
            return apps.apps.iter().find(|(actual, app)| {
                app.rule_id == spec.rule && actual.namespace_equivalent(key, &spec.rule)
            });
        }
        apps.apps.get_key_value(key).filter(|(_, app)| {
            view.rules.is_empty()
                || view.rules.iter().any(|spec| {
                    spec.rule == app.rule_id && spec.key.namespace_equivalent(key, &spec.rule)
                })
        })
    }
    pub fn key(&self, rule: &RuleId) -> Option<AppRuntimeKey> {
        self.view()
            .rules
            .iter()
            .find(|entry| &entry.rule == rule)
            .map(|entry| entry.key.clone())
    }
    pub fn observe(
        &self,
        apps: &RunningApps,
        generation: u64,
        mut image: impl FnMut(u32, u64) -> Option<PathBuf>,
    ) {
        let Ok(mut gate) = self.lock() else { return };
        if gate.generation != generation && !(generation == u64::MAX && gate.rules.is_empty()) {
            return;
        }
        gate.members.clear();
        gate.images.clear();
        let mut bytes = 0;
        for app in apps.apps.values() {
            for (&pid, &token) in &app.pid_instance_tokens {
                if gate.members.len() >= MAX_INSTANCES {
                    break;
                }
                let ids = gate.members.entry((pid, token)).or_default();
                if !ids.contains(&app.rule_id) {
                    if ids.len() < MAX_CLAIMANTS {
                        ids.push(app.rule_id.clone());
                    } else {
                        gate.error = Some("Too many overlapping rules for one process".into());
                    }
                }
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    gate.images.entry((pid, token))
                {
                    if let Some(path) = image(pid, token) {
                        let image = canonical(&path);
                        if image.len() <= 16000 && bytes + image.len() <= MAX_IMAGE_BYTES {
                            bytes += image.len();
                            entry.insert(image);
                        }
                    }
                }
            }
        }
        self.publish_view(&gate);
    }
    pub fn write<T>(
        &self,
        request: WriteRequest<'_>,
        action: impl FnOnce(bool) -> Result<T, String>,
    ) -> Result<(T, bool), String> {
        self.with_owner(request, None, |_, write| {
            action(write).map(|result| (result, write))
        })
    }
    pub fn initial<T>(
        &self,
        request: WriteRequest<'_>,
        action: impl FnOnce(&RuleInfo) -> Result<T, String>,
    ) -> Result<T, String> {
        if !request.change {
            return Err("Initial correction requires launch intent".into());
        }
        self.with_owner(request, None, |owner, _| action(owner))
    }
    pub fn initial_with_source<T>(
        &self,
        source: SourceRequest<'_>,
        request: WriteRequest<'_>,
        action: impl FnOnce(&RuleInfo) -> Result<T, String>,
    ) -> Result<T, String> {
        if !request.change {
            return Err("Initial correction requires launch intent".into());
        }
        self.with_owner(request, Some(source), |owner, _| action(owner))
    }
    fn with_owner<T>(
        &self,
        request: WriteRequest<'_>,
        source: Option<SourceRequest<'_>>,
        action: impl FnOnce(&RuleInfo, bool) -> Result<T, String>,
    ) -> Result<T, String> {
        if request.pid == 0 || request.token == 0 {
            return Err("Exact process identity is required".into());
        }
        let mut gate = self.lock()?;
        if let Some(source) = source {
            if source.rule != request.rule
                || !source.key.namespace_equivalent(request.key, source.rule)
            {
                return Err("Initial correction source does not match its rule".into());
            }
            gate.source(source)?;
        }
        let spec = gate
            .rules
            .iter()
            .find(|spec| &spec.rule == request.rule)
            .ok_or("Rule was removed; refresh before applying settings")?;
        if !spec.key.namespace_equivalent(request.key, request.rule)
            || (request.fields.affinity && !spec.fields.affinity)
            || (request.fields.priority && !spec.fields.priority)
            || (request.fields.affinity && spec.mask != request.mask)
            || (request.fields.priority && spec.app.priority != request.priority)
        {
            return Err("Rule settings changed; refresh before applying".into());
        }
        let image = request
            .image
            .map(canonical)
            .or_else(|| gate.images.get(&(request.pid, request.token)).cloned())
            .unwrap_or_default();
        let observed = gate
            .members
            .get(&(request.pid, request.token))
            .is_some_and(|ids| ids.contains(request.rule));
        if !observed
            && !image.is_empty()
            && !matches!(
                spec.app.launch_target,
                crate::app::models::LaunchTarget::Installed { .. }
            )
            && !gate.admits_source(spec, &image)
        {
            return Err("Process membership changed; wait for discovery before applying".into());
        }
        let claimants = gate.claimants(
            Some((request.pid, request.token)),
            &image,
            Some(request.rule),
        );
        let owner = gate.resolve(&image, &claimants)?;
        if let Some(notice) = gate.notice(image.clone(), claimants) {
            if gate.images.len() < MAX_INSTANCES
                && gate.images.values().map(String::len).sum::<usize>() + notice.image.len()
                    <= MAX_IMAGE_BYTES
            {
                gate.members
                    .insert((request.pid, request.token), notice.claimants);
                gate.images
                    .insert((request.pid, request.token), notice.image);
            }
            self.publish_view(&gate);
        }
        let Some(owner) = owner else {
            return Err("Choose which app controls this shared process".into());
        };
        let write = request.change && owner == *request.rule;
        let selected = gate
            .rules
            .iter()
            .find(|rule| rule.rule == owner)
            .ok_or("Process owner is unavailable")?;
        action(selected, write)
    }
    pub fn launch<T>(
        &self,
        rule: &RuleId,
        key: &AppRuntimeKey,
        path: &Path,
        action: impl FnOnce(&RuleInfo) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut gate = self.lock()?;
        let spec = gate
            .rules
            .iter()
            .find(|spec| &spec.rule == rule && &spec.key == key)
            .ok_or("Rule changed before launch")?;
        if spec
            .app
            .bin_path()
            .is_none_or(|expected| canonical(expected) != canonical(path))
        {
            return Err("Launch target changed; refresh before launching".into());
        }
        let image = canonical(path);
        let claimants = gate.claimants(None, &image, Some(&spec.rule));
        let owner = match gate.resolve(&image, &claimants)? {
            Some(owner) => owner,
            None => {
                if gate.pending.len() < MAX_ROWS {
                    gate.pending.insert(image, claimants);
                    self.publish_view(&gate);
                }
                return Err(
                    "Choose which app controls this shared executable before launch".into(),
                );
            }
        };
        let selected = gate
            .rules
            .iter()
            .find(|spec| spec.rule == owner)
            .ok_or("Process owner is unavailable")?;
        action(selected)
    }
    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn launch_unowned<T>(
        &self,
        path: &Path,
        action: impl FnOnce(Option<&RuleInfo>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut gate = self.lock()?;
        if let Some(error) = &gate.error {
            return Err(error.clone());
        }
        let image = canonical(path);
        let claimants = gate.claimants(None, &image, None);
        if claimants.is_empty() {
            return action(None);
        }
        let owner = match gate.resolve(&image, &claimants)? {
            Some(owner) => owner,
            None => {
                if gate.pending.len() < MAX_ROWS {
                    gate.pending.insert(image, claimants);
                    self.publish_view(&gate);
                }
                return Err(
                    "Choose which app controls this shared executable before launch".into(),
                );
            }
        };
        action(gate.rules.iter().find(|rule| rule.rule == owner))
    }
    #[cfg(all(target_os = "windows", feature = "windows"))]
    pub fn external_write<T>(
        &self,
        pid: u32,
        token: u64,
        image: &Path,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        if pid == 0 || token == 0 {
            return Err("Exact process identity is required".into());
        }
        let gate = self.lock()?;
        if gate.error.is_some() {
            return Err("Process control settings are unreadable".into());
        }
        if !gate
            .claimants(Some((pid, token)), &canonical(image), None)
            .is_empty()
        {
            return Err(
                "A saved application controls this process; edit its rule or choose its owner"
                    .into(),
            );
        }
        action()
    }
}

impl View {
    pub fn label(&self, id: &RuleId) -> &str {
        self.rules
            .iter()
            .find(|rule| &rule.rule == id)
            .map_or("Unavailable app", |rule| rule.label.as_str())
    }
    pub fn needs_choice(&self, id: &RuleId) -> bool {
        self.notices.iter().any(|notice| {
            notice.different && notice.owner.is_none() && notice.claimants.contains(id)
        })
    }
    pub fn shared_elsewhere(&self, id: &RuleId) -> bool {
        self.notices.iter().any(|notice| {
            notice.different
                && notice.owner.as_ref().is_some_and(|owner| owner != id)
                && notice.claimants.contains(id)
        })
    }
    pub fn duplicate_labels(&self, id: &RuleId, app: &AppToRun) -> Vec<String> {
        let names: HashSet<_> = app
            .additional_processes
            .iter()
            .map(|name| crate::app::models::normalize_process_name(name))
            .filter(|name| !name.is_empty())
            .collect();
        self.rules
            .iter()
            .filter(|rule| {
                &rule.rule != id
                    && rule.app.additional_processes.iter().any(|name| {
                        names.contains(&crate::app::models::normalize_process_name(name))
                    })
            })
            .take(8)
            .map(|rule| rule.label.clone())
            .collect()
    }
}
impl Authority {
    pub fn validate_choices(
        &self,
        candidate: &AppStateStorage,
        choices: &[super::policy::Choice],
    ) -> Result<(), String> {
        if choices.len() > MAX_ROWS {
            return Err("Too many process owner changes".into());
        }
        let gate = self.lock()?;
        let mut next = Gate::default();
        next.configure(candidate);
        if next.error.is_some() {
            return Err("Process control settings are unreadable".into());
        }
        for choice in choices {
            let image = image_key(&choice.image);
            let mut eligible = gate.pending.get(&image).cloned().unwrap_or_default();
            for (&instance, known_image) in &gate.images {
                if known_image == &image {
                    for rule in gate.claimants(Some(instance), &image, None) {
                        if !eligible.contains(&rule) {
                            eligible.push(rule);
                        }
                    }
                }
            }
            if !eligible.contains(&choice.owner)
                || !next.rules.iter().any(|rule| rule.rule == choice.owner)
            {
                return Err(
                    "This process conflict changed; refresh before choosing its owner".into(),
                );
            }
        }
        Ok(())
    }
}
