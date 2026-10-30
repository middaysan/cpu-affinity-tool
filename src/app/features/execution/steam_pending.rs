//! Shared bounded one-shot intent for Steam games and known application launches.
use crate::app::{
    models::{AppRuntimeKey, AppToRun},
    shared::ids::RuleId,
};
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum LaunchIdentity {
    Steam(u32),
    Known(AppRuntimeKey),
}
impl From<u32> for LaunchIdentity {
    fn from(id: u32) -> Self {
        Self::Steam(id)
    }
}
pub(crate) fn identity_for_app(app: &AppToRun) -> Option<LaunchIdentity> {
    if let Some((id, _)) = app.steam_game() {
        return Some(LaunchIdentity::Steam(id));
    }
    (app.is_path_target()
        && app.builtin_process_binding.is_some()
        && crate::app::features::rules::known_apps::project(app).is_ok())
    .then(|| LaunchIdentity::Known(app.get_key()))
}

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
const WINDOW: Duration = Duration::from_secs(300);
#[derive(Default)]
struct Launches {
    intents: HashMap<(RuleId, LaunchIdentity), Intent>,
    next: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SteamLaunchBegin {
    AlreadyPending,
    Started(u64),
    Busy,
}
struct Intent {
    at: Instant,
    generation: u64,
    main_seen: bool,
    confirmed: HashSet<(u32, u64)>,
}
impl Launches {
    fn detection_overdue(&self, rule: &RuleId, id: u32, now: Instant) -> bool {
        // A diagnostic can outlive the scheduling intent; it grants no initial writes.
        self.intents
            .get(&(rule.clone(), LaunchIdentity::Steam(id)))
            .is_some_and(|intent| {
                !intent.main_seen
                    && now.saturating_duration_since(intent.at) >= Duration::from_secs(30)
            })
    }
    fn begin(
        &mut self,
        rule: &RuleId,
        id: impl Into<LaunchIdentity>,
        now: Instant,
    ) -> SteamLaunchBegin {
        self.intents
            .retain(|_, i| now.saturating_duration_since(i.at) < WINDOW);
        let key = (rule.clone(), id.into());
        if self.intents.get(&key).is_some_and(|i| !i.main_seen) {
            return SteamLaunchBegin::AlreadyPending;
        }
        if self.intents.len() >= 128 && !self.intents.contains_key(&key) {
            return SteamLaunchBegin::Busy;
        }
        let Some(generation) = self.next.checked_add(1) else {
            return SteamLaunchBegin::Busy;
        };
        self.next = generation;
        self.intents.insert(
            key,
            Intent {
                at: now,
                generation,
                main_seen: false,
                confirmed: HashSet::new(),
            },
        );
        SteamLaunchBegin::Started(generation)
    }
    fn cancel(&mut self, rule: &RuleId, id: impl Into<LaunchIdentity>, generation: u64) {
        let key = (rule.clone(), id.into());
        if self
            .intents
            .get(&key)
            .is_some_and(|i| i.generation == generation)
        {
            self.intents.remove(&key);
        }
    }
    fn get(&self, rule: &RuleId, id: impl Into<LaunchIdentity>, now: Instant) -> Option<&Intent> {
        self.intents
            .get(&(rule.clone(), id.into()))
            .filter(|i| now.saturating_duration_since(i.at) < WINDOW)
    }
}
fn launches() -> &'static Mutex<Launches> {
    static VALUE: OnceLock<Mutex<Launches>> = OnceLock::new();
    VALUE.get_or_init(Default::default)
}
pub(crate) fn launch_pending(rule: &RuleId, id: Option<LaunchIdentity>) -> bool {
    id.is_some_and(|id| {
        launches().lock().ok().is_some_and(|l| {
            l.get(rule, id, Instant::now())
                .is_some_and(|i| !i.main_seen)
        })
    })
}
pub(crate) fn begin_launch(rule: &RuleId, id: LaunchIdentity) -> SteamLaunchBegin {
    launches().lock().map_or(SteamLaunchBegin::Busy, |mut l| {
        l.begin(rule, id, Instant::now())
    })
}
pub(crate) fn cancel_launch(rule: &RuleId, id: LaunchIdentity, generation: u64) {
    if let Ok(mut l) = launches().lock() {
        l.cancel(rule, id, generation);
    }
}
#[cfg(test)]
pub(crate) fn mark_steam_launch(rule: &RuleId, id: u32) -> bool {
    matches!(begin_steam_launch(rule, id), SteamLaunchBegin::Started(_))
}
#[cfg(test)]
pub(crate) fn finish_steam_launch(rule: &RuleId, id: u32) {
    if let Ok(mut l) = launches().lock() {
        l.intents.remove(&(rule.clone(), LaunchIdentity::Steam(id)));
    }
}
pub(crate) fn initial_apply(
    rule: &RuleId,
    id: Option<LaunchIdentity>,
    pid: u32,
    token: u64,
) -> bool {
    id.is_some_and(|id| {
        launches().lock().ok().is_some_and(|l| {
            l.get(rule, id, Instant::now())
                .is_some_and(|i| i.confirmed.len() < 4096 && !i.confirmed.contains(&(pid, token)))
        })
    })
}
pub(crate) fn confirm_instance(
    rule: &RuleId,
    id: Option<LaunchIdentity>,
    pid: u32,
    token: u64,
    main: bool,
) {
    if let Some(id) = id {
        if let Ok(mut l) = launches().lock() {
            if let Some(i) = l.intents.get_mut(&(rule.clone(), id)) {
                if i.at.elapsed() < WINDOW && i.confirmed.len() < 4096 {
                    i.confirmed.insert((pid, token));
                    i.main_seen |= main;
                }
            }
        }
    }
}
pub(crate) fn steam_launch_pending(rule: &RuleId, id: Option<u32>) -> bool {
    launch_pending(rule, id.map(LaunchIdentity::Steam))
}
pub(crate) fn steam_detection_overdue(rule: &RuleId, id: u32) -> bool {
    launches()
        .lock()
        .ok()
        .is_some_and(|l| l.detection_overdue(rule, id, Instant::now()))
}
pub(crate) fn begin_steam_launch(rule: &RuleId, id: u32) -> SteamLaunchBegin {
    begin_launch(rule, LaunchIdentity::Steam(id))
}
pub(crate) fn cancel_steam_launch(rule: &RuleId, id: u32, generation: u64) {
    cancel_launch(rule, LaunchIdentity::Steam(id), generation);
}
#[cfg(test)]
pub(crate) fn steam_initial_apply(rule: &RuleId, id: Option<u32>, pid: u32, token: u64) -> bool {
    initial_apply(rule, id.map(LaunchIdentity::Steam), pid, token)
}
#[cfg(test)]
pub(crate) fn confirm_steam_instance(
    rule: &RuleId,
    id: Option<u32>,
    pid: u32,
    token: u64,
    main: bool,
) {
    confirm_instance(rule, id.map(LaunchIdentity::Steam), pid, token, main);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn s103_intent_survives_slow_launch_but_is_bounded_and_app_specific() {
        let rule = RuleId("game".into());
        let now = Instant::now();
        let mut l = Launches::default();
        assert!(matches!(
            l.begin(&rule, 570u32, now),
            SteamLaunchBegin::Started(_)
        ));
        assert!(l
            .get(&rule, 570u32, now + Duration::from_secs(120))
            .is_some());
        assert!(l.get(&rule, 571u32, now).is_none());
        assert!(l.get(&rule, 570u32, now + WINDOW).is_none());
        assert!(!l.detection_overdue(&rule, 570, now + Duration::from_secs(29)));
        assert!(l.detection_overdue(&rule, 570, now + Duration::from_secs(30)));
        assert!(l.detection_overdue(&rule, 570, now + WINDOW));
        assert!(!l.detection_overdue(&rule, 571, now + WINDOW));
        l.intents
            .get_mut(&(rule.clone(), 570u32.into()))
            .unwrap()
            .main_seen = true;
        assert!(!l.detection_overdue(&rule, 570, now + WINDOW));
    }
    #[test]
    fn s103_atomic_launch_coalesces_and_old_failure_cannot_cancel_a_new_generation() {
        let rule = RuleId("atomic".into());
        let mut l = Launches::default();
        let now = Instant::now();
        let SteamLaunchBegin::Started(first) = l.begin(&rule, 570u32, now) else {
            panic!()
        };
        assert_eq!(
            l.begin(&rule, 570u32, now),
            SteamLaunchBegin::AlreadyPending
        );
        l.intents
            .get_mut(&(rule.clone(), 570u32.into()))
            .unwrap()
            .main_seen = true;
        let SteamLaunchBegin::Started(second) = l.begin(&rule, 570u32, now) else {
            panic!()
        };
        assert_ne!(first, second);
        l.cancel(&rule, 570u32, first);
        assert_eq!(l.get(&rule, 570u32, now).unwrap().generation, second);
        l.cancel(&rule, 570u32, second);
        assert!(l.get(&rule, 570u32, now).is_none());
    }
    #[test]
    fn s103_parallel_launch_admission_has_exactly_one_dispatch_owner() {
        let shared = std::sync::Arc::new(Mutex::new(Launches::default()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let shared = shared.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    shared
                        .lock()
                        .unwrap()
                        .begin(&RuleId("parallel".into()), 570u32, Instant::now())
                })
            })
            .collect();
        let outcomes: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| matches!(o, SteamLaunchBegin::Started(_)))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| **o == SteamLaunchBegin::AlreadyPending)
                .count(),
            1
        );
    }
    #[test]
    fn s103_late_members_get_one_initial_apply_without_continuous_off_enforcement() {
        let r = RuleId("s103-one-shot".into());
        assert!(mark_steam_launch(&r, 570));
        assert!(steam_initial_apply(&r, Some(570), 1, 10));
        confirm_steam_instance(&r, Some(570), 1, 10, true);
        assert!(!steam_launch_pending(&r, Some(570)));
        assert!(!steam_initial_apply(&r, Some(570), 1, 10));
        assert!(steam_initial_apply(&r, Some(570), 2, 20));
        assert!(steam_initial_apply(&r, Some(570), 1, 11));
        finish_steam_launch(&r, 570);
    }
}
