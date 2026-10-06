//! Bounded serial dispatch. The worker owns the coordinator for its full lifetime.
use super::contract::{OperationId, OwnerId};
use super::coordinator::{
    Command, CommandResult, Coordinator, CoordinatorSnapshot, ExecutionControl, GateError,
    WriteIntent,
};
use std::{
    collections::BTreeMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Condvar, Mutex, Weak,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_QUEUE_CAPACITY: usize = 64;
const MAX_GENERATION_OWNERS: usize = 256;
const MAX_RETIRED_INTERVALS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunnerLimits {
    pub queue_capacity: usize,
    pub generation_owners: usize,
}
impl Default for RunnerLimits {
    fn default() -> Self {
        Self {
            queue_capacity: 16,
            generation_owners: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartError {
    InvalidLimits,
    WorkerSpawn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    QueueFull,
    Closed,
    InvalidGeneration,
    StaleGeneration { current: u64 },
    GenerationCapacity,
    OperationIdExhausted,
    InvalidEnvelope,
    RetiredOwner,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetirementError {
    Closed,
    UnknownOwner,
    GenerationMismatch,
    Busy,
    TombstoneCapacity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationStamp {
    pub operation: OperationId,
    pub config_generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerPhase {
    Running,
    Stopping,
    Stopped,
    Faulted,
}
#[derive(Clone)]
pub struct RunnerSnapshot {
    pub phase: RunnerPhase,
    pub current: Option<OperationStamp>,
    pub queued: Vec<OperationStamp>,
    pub possible_write: bool,
    pub coordinator: CoordinatorSnapshot,
    pub shutdown_result: Option<Arc<CommandResult>>,
}
#[derive(Clone)]
pub enum TerminalOutcome {
    Completed(Arc<CommandResult>),
    CancelledBeforeStart,
    WorkerPanicked,
}
#[derive(Clone)]
pub struct TerminalResult {
    pub stamp: OperationStamp,
    pub stale: bool,
    pub outcome: TerminalOutcome,
    pub snapshot: CoordinatorSnapshot,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketPhase {
    Queued,
    Running,
    Terminal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownResult {
    Complete,
    RecoveryRequired,
    TimedOut,
    Faulted,
}

enum TicketState {
    Queued,
    Running,
    Terminal(Arc<TerminalResult>),
}
struct TicketData {
    stamp: OperationStamp,
    generation_owner: Option<OwnerId>,
    cancellable: bool,
    cancelled: AtomicBool,
    state: Mutex<TicketState>,
    changed: Condvar,
}
#[derive(Clone)]
pub struct Ticket {
    inner: Arc<TicketData>,
    shared: Weak<Shared>,
}
impl Ticket {
    pub fn stamp(&self) -> OperationStamp {
        self.inner.stamp
    }
    pub fn phase(&self) -> TicketPhase {
        match &*self.inner.state.lock().unwrap() {
            TicketState::Queued => TicketPhase::Queued,
            TicketState::Running => TicketPhase::Running,
            TicketState::Terminal(_) => TicketPhase::Terminal,
        }
    }
    /// Cleanup requests remain executable even if their ticket is cancelled.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
    }
    fn refresh_staleness(&self, mut result: TerminalResult) -> TerminalResult {
        if let (Some(shared), Some(owner)) = (self.shared.upgrade(), self.inner.generation_owner) {
            let admission = shared.admission.lock().unwrap();
            result.stale |= admission.is_retired(owner)
                || admission
                    .generations
                    .get(&owner)
                    .is_some_and(|current| *current > result.stamp.config_generation);
        }
        result
    }
    pub fn result(&self) -> Option<TerminalResult> {
        let result = match &*self.inner.state.lock().unwrap() {
            TicketState::Terminal(result) => Some(result.as_ref().clone()),
            _ => None,
        };
        result.map(|result| self.refresh_staleness(result))
    }
    pub fn wait(&self, timeout: Duration) -> Option<TerminalResult> {
        let guard = self.inner.state.lock().unwrap();
        let (guard, _) = self
            .inner
            .changed
            .wait_timeout_while(guard, timeout, |state| {
                !matches!(state, TicketState::Terminal(_))
            })
            .unwrap();
        let result = match &*guard {
            TicketState::Terminal(result) => Some(result.as_ref().clone()),
            _ => None,
        };
        drop(guard);
        result.map(|result| self.refresh_staleness(result))
    }
}
struct WorkItem {
    command: Command,
    ticket: Arc<TicketData>,
}
struct Admission {
    sender: Option<SyncSender<WorkItem>>,
    next_operation: Option<u64>,
    generations: BTreeMap<OwnerId, u64>,
    retired: BTreeMap<u64, u64>,
    generation_limit: usize,
    pending: BTreeMap<OperationId, Arc<TicketData>>,
    active: Option<OperationId>,
}
impl Admission {
    fn is_retired(&self, owner: OwnerId) -> bool {
        self.retired
            .range(..=owner.get())
            .next_back()
            .is_some_and(|(_, end)| *end >= owner.get())
    }
    fn retire(&mut self, owner: OwnerId) -> Result<(), RetirementError> {
        let id = owner.get();
        let previous = self
            .retired
            .range(..id)
            .next_back()
            .map(|(start, end)| (*start, *end))
            .filter(|(_, end)| end.checked_add(1) == Some(id));
        let next = id
            .checked_add(1)
            .and_then(|next_id| {
                self.retired
                    .range(next_id..)
                    .next()
                    .map(|(start, end)| (*start, *end))
            })
            .filter(|(start, _)| id.checked_add(1) == Some(*start));
        let resulting_count =
            self.retired.len() + 1 - usize::from(previous.is_some()) - usize::from(next.is_some());
        if resulting_count > MAX_RETIRED_INTERVALS {
            return Err(RetirementError::TombstoneCapacity);
        }
        let start = previous.map_or(id, |(start, _)| start);
        let end = next.map_or(id, |(_, end)| end);
        if let Some((start, _)) = previous {
            self.retired.remove(&start);
        }
        if let Some((start, _)) = next {
            self.retired.remove(&start);
        }
        self.retired.insert(start, end);
        self.generations.remove(&owner);
        Ok(())
    }
}
struct Shared {
    admission: Mutex<Admission>,
    runtime: Mutex<RunnerSnapshot>,
}
impl Shared {
    // Admission is always locked before runtime when both are needed. Native
    // work and waits never occur while either shared lock is held.
    fn update_queue(&self, admission: &Admission) {
        self.runtime.lock().unwrap().queued = admission
            .pending
            .iter()
            .filter(|(id, _)| Some(**id) != admission.active)
            .map(|(_, ticket)| ticket.stamp)
            .collect();
    }
    fn close(&self, faulted: bool) {
        let mut admission = self.admission.lock().unwrap();
        admission.sender.take();
        for ticket in admission
            .pending
            .values()
            .filter(|ticket| ticket.cancellable)
        {
            ticket.cancelled.store(true, Ordering::Release);
        }
        let mut runtime = self.runtime.lock().unwrap();
        if faulted {
            runtime.phase = RunnerPhase::Faulted;
        } else if runtime.phase == RunnerPhase::Running {
            runtime.phase = RunnerPhase::Stopping;
        }
    }
    fn start_work(&self, ticket: &TicketData) {
        let mut admission = self.admission.lock().unwrap();
        admission.active = Some(ticket.stamp.operation);
        *ticket.state.lock().unwrap() = TicketState::Running;
        let mut runtime = self.runtime.lock().unwrap();
        runtime.current = Some(ticket.stamp);
        runtime.possible_write = false;
        runtime.queued = admission
            .pending
            .iter()
            .filter(|(id, _)| Some(**id) != admission.active)
            .map(|(_, ticket)| ticket.stamp)
            .collect();
    }
    fn complete(
        &self,
        ticket: &TicketData,
        outcome: TerminalOutcome,
        snapshot: CoordinatorSnapshot,
    ) {
        let mut admission = self.admission.lock().unwrap();
        let stale = ticket.generation_owner.is_some_and(|owner| {
            admission
                .generations
                .get(&owner)
                .is_some_and(|generation| *generation > ticket.stamp.config_generation)
        });
        let result = TerminalResult {
            stamp: ticket.stamp,
            stale,
            outcome,
            snapshot: snapshot.clone(),
        };
        *ticket.state.lock().unwrap() = TicketState::Terminal(Arc::new(result));
        ticket.changed.notify_all();
        admission.pending.remove(&ticket.stamp.operation);
        if admission.active == Some(ticket.stamp.operation) {
            admission.active = None;
        }
        let mut runtime = self.runtime.lock().unwrap();
        runtime.current = None;
        runtime.coordinator = snapshot;
        if runtime.phase != RunnerPhase::Faulted {
            runtime.possible_write = false;
        }
        runtime.queued = admission
            .pending
            .values()
            .map(|ticket| ticket.stamp)
            .collect();
    }
}
#[derive(Clone)]
pub struct RunnerClient {
    shared: Arc<Shared>,
}
impl RunnerClient {
    /// Acceptance is atomic with generation advancement. Queue rejection never
    /// cancels an already accepted intent or consumes an operation identity.
    pub fn submit(&self, command: Command, config_generation: u64) -> Result<Ticket, SubmitError> {
        // Envelope validation is the serialization boundary, not module-specific
        // behavior. An accepted bounded queue must not retain unbounded payloads.
        if let Command::Acquire { config, .. } = &command {
            config
                .to_bytes()
                .map_err(|_| SubmitError::InvalidEnvelope)?;
        }
        let mut admission = self.shared.admission.lock().unwrap();
        if admission.sender.is_none() {
            return Err(SubmitError::Closed);
        }
        if config_generation == 0 {
            return Err(SubmitError::InvalidGeneration);
        }
        let generation_owner = match &command {
            Command::Acquire { owner, .. } | Command::Release { owner } => Some(*owner),
            _ => None,
        };
        if let Some(owner) = generation_owner {
            if admission.is_retired(owner) {
                return Err(SubmitError::RetiredOwner);
            }
            if let Some(current) = admission.generations.get(&owner) {
                if config_generation < *current {
                    return Err(SubmitError::StaleGeneration { current: *current });
                }
            } else if admission.generations.len() >= admission.generation_limit {
                return Err(SubmitError::GenerationCapacity);
            }
        }
        let operation = admission
            .next_operation
            .ok_or(SubmitError::OperationIdExhausted)?;
        let stamp = OperationStamp {
            operation: OperationId::new(operation).expect("nonzero allocator"),
            config_generation,
        };
        let cancellable = matches!(&command, Command::Acquire { .. } | Command::Observe { .. });
        let data = Arc::new(TicketData {
            stamp,
            generation_owner,
            cancellable,
            cancelled: AtomicBool::new(false),
            state: Mutex::new(TicketState::Queued),
            changed: Condvar::new(),
        });
        let item = WorkItem {
            command,
            ticket: Arc::clone(&data),
        };
        match admission.sender.as_ref().unwrap().try_send(item) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(SubmitError::QueueFull),
            Err(TrySendError::Disconnected(_)) => {
                admission.sender.take();
                return Err(SubmitError::Closed);
            }
        }
        admission.next_operation = operation.checked_add(1);
        if let Some(owner) = generation_owner {
            let previous = admission.generations.insert(owner, config_generation);
            if previous.is_none_or(|previous| config_generation > previous) {
                for old in admission.pending.values().filter(|old| {
                    old.cancellable
                        && old.generation_owner == Some(owner)
                        && old.stamp.config_generation < config_generation
                }) {
                    old.cancelled.store(true, Ordering::Release);
                }
            }
        }
        admission.pending.insert(stamp.operation, Arc::clone(&data));
        self.shared.update_queue(&admission);
        Ok(Ticket {
            inner: data,
            shared: Arc::downgrade(&self.shared),
        })
    }
    pub fn snapshot(&self) -> RunnerSnapshot {
        self.shared.runtime.lock().unwrap().clone()
    }
    /// An authoritative lifecycle owner may permanently retire an identity only
    /// after all its commands and managed claims are gone. Retired IDs are never
    /// accepted again, even with a larger generation.
    pub fn retire_generation_owner(
        &self,
        owner: OwnerId,
        expected_generation: u64,
    ) -> Result<(), RetirementError> {
        let mut admission = self.shared.admission.lock().unwrap();
        if admission.sender.is_none() {
            return Err(RetirementError::Closed);
        }
        if admission.is_retired(owner) {
            return Ok(());
        }
        let generation = admission
            .generations
            .get(&owner)
            .ok_or(RetirementError::UnknownOwner)?;
        if *generation != expected_generation {
            return Err(RetirementError::GenerationMismatch);
        }
        if admission
            .pending
            .values()
            .any(|ticket| ticket.generation_owner == Some(owner))
        {
            return Err(RetirementError::Busy);
        }
        {
            let runtime = self.shared.runtime.lock().unwrap();
            if runtime
                .coordinator
                .owners
                .iter()
                .any(|view| view.owner == owner)
                || runtime
                    .coordinator
                    .resources
                    .iter()
                    .any(|view| view.owners.contains(&owner))
            {
                return Err(RetirementError::Busy);
            }
        }
        admission.retire(owner)
    }
}
#[derive(Clone, Copy)]
enum WriteAdmission {
    ReadOnly,
    #[cfg(test)]
    Fake,
}
struct WorkerControl {
    shared: Arc<Shared>,
    ticket: Option<Arc<TicketData>>,
    admission: WriteAdmission,
    possible_write: bool,
}
impl ExecutionControl for WorkerControl {
    fn cancellation_requested(&self) -> bool {
        self.ticket
            .as_ref()
            .is_some_and(|ticket| ticket.cancellable && ticket.cancelled.load(Ordering::Acquire))
    }
    fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        // Cleanup is never abandoned because the initiating generation expired.
        if !intent.operation.is_restoration()
            && !self.possible_write
            && self.cancellation_requested()
        {
            return Err(GateError::Cancelled);
        }
        match self.admission {
            WriteAdmission::ReadOnly => Err(GateError::MutationDisabled),
            #[cfg(test)]
            WriteAdmission::Fake => {
                self.possible_write = true;
                let mut runtime = self.shared.runtime.lock().unwrap();
                runtime.possible_write = true;
                runtime.coordinator = intent.ownership.clone();
                Ok(())
            }
        }
    }
    fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        // S36 will replace the test admission with worker-owned durable storage
        // and the native writer lease. Verification alone is no durable commit.
        self.shared.runtime.lock().unwrap().coordinator = intent.ownership.clone();
        match self.admission {
            WriteAdmission::ReadOnly => Err(GateError::MutationDisabled),
            #[cfg(test)]
            WriteAdmission::Fake => Ok(()),
        }
    }
    fn publish(&mut self, snapshot: CoordinatorSnapshot) {
        self.shared.runtime.lock().unwrap().coordinator = snapshot;
    }
}
struct WorkerExit {
    coordinator: Coordinator,
    faulted: bool,
    recovery_required: bool,
}
pub struct Runner {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<WorkerExit>>,
    completion: Receiver<()>,
    completion_seen: bool,
    // Keep typed restoration evidence after thread termination. F3 adds durable
    // evidence before admitting any real write, including forced process death.
    retained: Option<Coordinator>,
    faulted: bool,
    recovery_required: bool,
}
impl Runner {
    /// F2 production admission is read-only even when a writable module exists.
    pub fn start(coordinator: Coordinator, limits: RunnerLimits) -> Result<Self, StartError> {
        Self::spawn(coordinator, limits, WriteAdmission::ReadOnly)
    }
    #[cfg(test)]
    fn start_fake(coordinator: Coordinator, limits: RunnerLimits) -> Result<Self, StartError> {
        Self::spawn(coordinator, limits, WriteAdmission::Fake)
    }
    fn spawn(
        coordinator: Coordinator,
        limits: RunnerLimits,
        write_admission: WriteAdmission,
    ) -> Result<Self, StartError> {
        if limits.queue_capacity == 0
            || limits.queue_capacity > MAX_QUEUE_CAPACITY
            || limits.generation_owners == 0
            || limits.generation_owners > MAX_GENERATION_OWNERS
        {
            return Err(StartError::InvalidLimits);
        }
        let (sender, receiver) = mpsc::sync_channel(limits.queue_capacity);
        let (done, completion) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            admission: Mutex::new(Admission {
                sender: Some(sender),
                next_operation: Some(1),
                generations: BTreeMap::new(),
                retired: BTreeMap::new(),
                generation_limit: limits.generation_owners,
                pending: BTreeMap::new(),
                active: None,
            }),
            runtime: Mutex::new(RunnerSnapshot {
                phase: RunnerPhase::Running,
                current: None,
                queued: vec![],
                possible_write: false,
                coordinator: coordinator.snapshot(),
                shutdown_result: None,
            }),
        });
        let state = Arc::clone(&shared);
        // No native lease is acquired on the caller. S36 must construct/drop its
        // thread-bound writer context inside this worker, around this full loop.
        let worker = thread::Builder::new()
            .name("tuning-runner".into())
            .spawn(move || worker_loop(coordinator, receiver, state, write_admission, done))
            .map_err(|_| StartError::WorkerSpawn)?;
        Ok(Self {
            shared,
            worker: Some(worker),
            completion,
            completion_seen: false,
            retained: None,
            faulted: false,
            recovery_required: false,
        })
    }
    pub fn client(&self) -> RunnerClient {
        RunnerClient {
            shared: Arc::clone(&self.shared),
        }
    }
    pub fn snapshot(&self) -> RunnerSnapshot {
        self.client().snapshot()
    }
    /// Timeout retains this exact worker, its coordinator and any future lease.
    /// Calling shutdown again can collect completion; there is no restart API.
    pub fn shutdown(&mut self, timeout: Duration) -> ShutdownResult {
        self.shared.close(false);
        if self.worker.is_none() {
            return self.finished_result();
        }
        let started = Instant::now();
        if !self.completion_seen {
            match self.completion.recv_timeout(timeout) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => self.completion_seen = true,
                Err(mpsc::RecvTimeoutError::Timeout) => return ShutdownResult::TimedOut,
            }
        }
        // Join only after actual thread completion, never converting a bounded
        // wait into an unbounded join because a completion notification arrived.
        while !self.worker.as_ref().unwrap().is_finished() {
            if started.elapsed() >= timeout {
                return ShutdownResult::TimedOut;
            }
            thread::yield_now();
        }
        match self.worker.take().unwrap().join() {
            Ok(exit) => {
                self.retained = Some(exit.coordinator);
                self.faulted = exit.faulted;
                self.recovery_required = exit.recovery_required;
            }
            Err(_) => {
                self.shared.close(true);
                self.faulted = true;
            }
        }
        self.finished_result()
    }
    fn finished_result(&self) -> ShutdownResult {
        if self.faulted {
            ShutdownResult::Faulted
        } else if self.recovery_required {
            ShutdownResult::RecoveryRequired
        } else {
            ShutdownResult::Complete
        }
    }
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.shared.close(false);
    }
}

fn worker_loop(
    mut coordinator: Coordinator,
    receiver: Receiver<WorkItem>,
    shared: Arc<Shared>,
    admission: WriteAdmission,
    done: SyncSender<()>,
) -> WorkerExit {
    let (faulted, recovery_required) = match catch_unwind(AssertUnwindSafe(|| {
        worker_body(&mut coordinator, &receiver, &shared, admission)
    })) {
        Ok(result) => result,
        Err(_) => {
            // A module's codec can also panic during snapshot export after its
            // command returned. Do not call that code again to report failure.
            shared.close(true);
            let snapshot = shared.runtime.lock().unwrap().coordinator.clone();
            let pending = shared
                .admission
                .lock()
                .unwrap()
                .pending
                .values()
                .cloned()
                .collect::<Vec<_>>();
            for ticket in pending {
                shared.complete(&ticket, TerminalOutcome::WorkerPanicked, snapshot.clone());
            }
            (true, true)
        }
    };
    let _ = done.send(());
    WorkerExit {
        coordinator,
        faulted,
        recovery_required,
    }
}

fn worker_body(
    coordinator: &mut Coordinator,
    receiver: &Receiver<WorkItem>,
    shared: &Arc<Shared>,
    admission: WriteAdmission,
) -> (bool, bool) {
    let mut faulted = false;
    while let Ok(item) = receiver.recv() {
        shared.start_work(&item.ticket);
        if item.ticket.cancellable && item.ticket.cancelled.load(Ordering::Acquire) {
            shared.complete(
                &item.ticket,
                TerminalOutcome::CancelledBeforeStart,
                coordinator.snapshot(),
            );
            continue;
        }
        let mut control = WorkerControl {
            shared: Arc::clone(shared),
            ticket: Some(Arc::clone(&item.ticket)),
            admission,
            possible_write: false,
        };
        match catch_unwind(AssertUnwindSafe(|| {
            coordinator.handle(item.command, &mut control)
        })) {
            Ok(mut result) => {
                let snapshot = coordinator.snapshot();
                result.snapshot = snapshot.clone();
                shared.complete(
                    &item.ticket,
                    TerminalOutcome::Completed(Arc::new(result)),
                    snapshot,
                );
            }
            Err(_) => {
                faulted = true;
                shared.close(true);
                shared.complete(
                    &item.ticket,
                    TerminalOutcome::WorkerPanicked,
                    coordinator.snapshot(),
                );
                // Admission is closed. Every already accepted queued ticket gets
                // a terminal failure instead of remaining pending indefinitely.
                for queued in receiver.try_iter() {
                    shared.complete(
                        &queued.ticket,
                        TerminalOutcome::WorkerPanicked,
                        coordinator.snapshot(),
                    );
                }
                break;
            }
        }
    }
    let mut recovery_required = false;
    if !faulted {
        let mut control = WorkerControl {
            shared: Arc::clone(shared),
            ticket: None,
            admission,
            possible_write: false,
        };
        match catch_unwind(AssertUnwindSafe(|| coordinator.shutdown(&mut control))) {
            Ok(mut result) => {
                result.snapshot = coordinator.snapshot();
                recovery_required = result.error.is_some()
                    || matches!(
                        result.outcome,
                        super::coordinator::Outcome::RecoveryRequired
                    );
                shared.runtime.lock().unwrap().shutdown_result = Some(Arc::new(result));
            }
            Err(_) => {
                faulted = true;
                shared.close(true);
            }
        }
    }
    let final_snapshot = coordinator.snapshot();
    {
        let mut runtime = shared.runtime.lock().unwrap();
        runtime.coordinator = final_snapshot;
        runtime.current = None;
        runtime.queued.clear();
        runtime.phase = if faulted {
            RunnerPhase::Faulted
        } else {
            RunnerPhase::Stopped
        };
        if !faulted {
            runtime.possible_write = false;
        }
    }
    (faulted, recovery_required)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::features::tuning::platform::catalog::Catalog;
    use crate::app::features::tuning::platform::contract::*;
    use crate::app::features::tuning::testing::scalar::{ScalarConfig, ScalarModule};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex,
    };

    const WAIT: Duration = Duration::from_secs(5);
    struct Block {
        entered: Mutex<mpsc::Receiver<()>>,
        released: Arc<(Mutex<bool>, Condvar)>,
        hook: Arc<dyn Fn() + Send + Sync>,
    }
    impl Block {
        fn once() -> Self {
            let (tx, rx) = mpsc::channel();
            let released = Arc::new((Mutex::new(false), Condvar::new()));
            let release = Arc::clone(&released);
            let used = AtomicBool::new(false);
            let hook = Arc::new(move || {
                if !used.swap(true, Ordering::SeqCst) {
                    tx.send(()).unwrap();
                    let (lock, cv) = &*release;
                    let mut done = lock.lock().unwrap();
                    while !*done {
                        done = cv.wait(done).unwrap();
                    }
                }
            });
            Self {
                entered: Mutex::new(rx),
                released,
                hook,
            }
        }
        fn entered(&self) {
            self.entered.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
        fn release(&self) {
            let (lock, cv) = &*self.released;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
    }
    impl Drop for Block {
        fn drop(&mut self) {
            self.release();
        }
    }
    fn owner(n: u64) -> OwnerId {
        OwnerId::new(n).unwrap()
    }
    fn target() -> TargetIdentity {
        TargetIdentity::Machine(MachineIdentity {
            id: MachineId::new([1; 16]).unwrap(),
        })
    }
    fn coordinator(module: &ScalarModule) -> Coordinator {
        let mut catalog = Catalog::default();
        catalog.register(module.clone()).unwrap();
        Coordinator::new(catalog)
    }
    fn acquire(module: &ScalarModule, who: u64, value: i32, mode: ActivationMode) -> Command {
        Command::Acquire {
            owner: owner(who),
            config: module.config_envelope(&ScalarConfig { value }).unwrap(),
            target: target(),
            mode,
        }
    }
    fn observe() -> Command {
        Command::Observe {
            feature: FeatureId::new("test_scalar").unwrap(),
            target: target(),
        }
    }
    fn result(ticket: &Ticket) -> TerminalResult {
        ticket
            .wait(WAIT)
            .expect("accepted command must become terminal")
    }
    fn finish(runner: &mut Runner) {
        assert_eq!(runner.shutdown(WAIT), ShutdownResult::Complete);
    }

    #[test]
    fn invalid_limits_and_production_gate_cannot_start_native_writes() {
        let module = ScalarModule::new(7);
        assert!(matches!(
            Runner::start(
                coordinator(&module),
                RunnerLimits {
                    queue_capacity: 0,
                    generation_owners: 1
                }
            ),
            Err(StartError::InvalidLimits)
        ));
        let mut runner = Runner::start(coordinator(&module), RunnerLimits::default()).unwrap();
        let ticket = runner
            .client()
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        let completed = result(&ticket);
        assert!(matches!(completed.outcome, TerminalOutcome::Completed(_)));
        assert_eq!(
            module.writes(),
            0,
            "F2 production runner cannot authorize an unjournaled native write"
        );
        finish(&mut runner);
    }

    #[test]
    fn queue_full_does_not_accept_or_advance_a_generation() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        module.set_observe_hook(Arc::clone(&block.hook));
        let mut runner = Runner::start_fake(
            coordinator(&module),
            RunnerLimits {
                queue_capacity: 1,
                generation_owners: 4,
            },
        )
        .unwrap();
        let client = runner.client();
        let first = client.submit(observe(), 1).unwrap();
        block.entered();
        let queued = client
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        assert!(matches!(
            client.submit(acquire(&module, 1, 13, ActivationMode::Session), 2),
            Err(SubmitError::QueueFull)
        ));
        assert_eq!(client.snapshot().queued.len(), 1);
        assert_eq!(module.writes(), 0);
        block.release();
        result(&first);
        let complete = result(&queued);
        assert!(!complete.stale);
        assert_eq!(module.value(), 12);
        finish(&mut runner);
        assert_eq!(module.value(), 7);
        assert!(matches!(
            client.submit(observe(), 1),
            Err(SubmitError::Closed)
        ));
    }

    #[test]
    fn newer_accepted_generation_cancels_before_first_write_and_cleanup_stays_current() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        module.set_observe_hook(Arc::clone(&block.hook));
        let mut runner = Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
        let client = runner.client();
        let old = client
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        block.entered();
        let new = client
            .submit(acquire(&module, 1, 13, ActivationMode::Session), 2)
            .unwrap();
        assert!(matches!(
            client.submit(acquire(&module, 1, 12, ActivationMode::Session), 1),
            Err(SubmitError::StaleGeneration { current: 2 })
        ));
        block.release();
        let old_result = result(&old);
        let new_result = result(&new);
        assert!(old_result.stale);
        assert!(!new_result.stale);
        assert_eq!(module.writes(), 1);
        assert_eq!(module.value(), 13);
        let release = client
            .submit(Command::Release { owner: owner(1) }, 3)
            .unwrap();
        release.cancel();
        result(&release);
        assert_eq!(
            module.value(),
            7,
            "release cancellation must not skip ownership cleanup"
        );
        finish(&mut runner);
    }

    #[test]
    fn cancelled_queued_observation_has_no_reads_or_ownership() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        module.set_observe_hook(Arc::clone(&block.hook));
        let mut runner = Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
        let first = runner.client().submit(observe(), 1).unwrap();
        block.entered();
        let cancelled = runner.client().submit(observe(), 1).unwrap();
        cancelled.cancel();
        block.release();
        result(&first);
        assert!(matches!(
            result(&cancelled).outcome,
            TerminalOutcome::CancelledBeforeStart
        ));
        assert_eq!(module.reads(), 1);
        assert!(runner.snapshot().coordinator.resources.is_empty());
        finish(&mut runner);
    }

    #[test]
    fn cancellation_after_possible_write_compensates_and_timeout_keeps_same_worker() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        module.set_execute_hook(Arc::clone(&block.hook));
        let mut runner = Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
        let client = runner.client();
        let ticket = client
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        block.entered();
        assert!(client.snapshot().possible_write);
        assert_eq!(client.snapshot().current, Some(ticket.stamp()));
        assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
        assert_eq!(runner.snapshot().current, Some(ticket.stamp()));
        assert!(matches!(
            client.submit(observe(), 1),
            Err(SubmitError::Closed)
        ));
        block.release();
        let completed = result(&ticket);
        assert!(matches!(completed.outcome, TerminalOutcome::Completed(_)));
        assert_eq!(module.value(), 7);
        assert!(completed.snapshot.resources.is_empty());
        finish(&mut runner);
        assert_eq!(runner.snapshot().phase, RunnerPhase::Stopped);
    }

    #[test]
    fn cancelled_existing_owner_edit_restores_previous_desired_then_original_on_release() {
        let module = ScalarModule::new(7);
        let mut runner = Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
        let client = runner.client();
        result(
            &client
                .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
                .unwrap(),
        );
        let block = Block::once();
        module.set_execute_hook(Arc::clone(&block.hook));
        let edit = client
            .submit(acquire(&module, 1, 13, ActivationMode::Session), 2)
            .unwrap();
        block.entered();
        edit.cancel();
        block.release();
        let completed = result(&edit);
        assert!(
            matches!(completed.outcome, TerminalOutcome::Completed(ref result) if result.outcome == super::super::coordinator::Outcome::Compensated)
        );
        assert_eq!(
            module.value(),
            12,
            "cancelled edit retains the previous owner's desired value"
        );
        assert_eq!(completed.snapshot.owners.len(), 1);
        result(
            &client
                .submit(Command::Release { owner: owner(1) }, 3)
                .unwrap(),
        );
        assert_eq!(
            module.value(),
            7,
            "final release retains the original first baseline"
        );
        finish(&mut runner);
    }

    #[test]
    fn manual_ownership_survives_orderly_shutdown_and_owner_generation_cache_is_bounded() {
        let module = ScalarModule::new(7);
        let mut runner = Runner::start_fake(
            coordinator(&module),
            RunnerLimits {
                queue_capacity: 2,
                generation_owners: 1,
            },
        )
        .unwrap();
        let client = runner.client();
        let manual = client
            .submit(acquire(&module, 1, 12, ActivationMode::Manual), 2)
            .unwrap();
        result(&manual);
        assert!(matches!(
            client.submit(acquire(&module, 2, 12, ActivationMode::Manual), 1),
            Err(SubmitError::GenerationCapacity)
        ));
        finish(&mut runner);
        assert_eq!(module.value(), 12);
        assert_eq!(runner.snapshot().coordinator.resources.len(), 1);
    }

    #[test]
    fn worker_panic_closes_admission_and_every_accepted_ticket_has_terminal_evidence() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        let pause = Arc::clone(&block.hook);
        module.set_execute_hook(Arc::new(move || {
            pause();
            panic!("simulated native worker panic");
        }));
        let mut runner = Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
        let client = runner.client();
        let first = client
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        block.entered();
        let queued = client.submit(observe(), 1).unwrap();
        block.release();
        assert!(matches!(
            result(&first).outcome,
            TerminalOutcome::WorkerPanicked
        ));
        assert!(matches!(
            result(&queued).outcome,
            TerminalOutcome::WorkerPanicked
        ));
        assert!(matches!(
            client.submit(observe(), 1),
            Err(SubmitError::Closed)
        ));
        assert_eq!(runner.shutdown(WAIT), ShutdownResult::Faulted);
        assert_eq!(runner.snapshot().phase, RunnerPhase::Faulted);
        assert!(
            !runner.snapshot().coordinator.resources.is_empty(),
            "panic cannot discard unresolved ownership evidence"
        );
    }

    #[test]
    fn all_native_instructions_run_on_one_worker_and_old_completed_results_become_stale() {
        use std::sync::atomic::AtomicUsize;
        let module = ScalarModule::new(7);
        let block = Block::once();
        let pause = Arc::clone(&block.hook);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let threads = Arc::new(Mutex::new(Vec::new()));
        let (inflight, maximum, seen) =
            (Arc::clone(&active), Arc::clone(&peak), Arc::clone(&threads));
        module.set_execute_hook(Arc::new(move || {
            let now = inflight.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(now, Ordering::SeqCst);
            seen.lock().unwrap().push(std::thread::current().id());
            pause();
            inflight.fetch_sub(1, Ordering::SeqCst);
        }));
        let mut runner = Runner::start_fake(
            coordinator(&module),
            RunnerLimits {
                queue_capacity: 2,
                generation_owners: 4,
            },
        )
        .unwrap();
        let client = runner.client();
        let first = client
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        block.entered();
        let release = client
            .submit(Command::Release { owner: owner(1) }, 2)
            .unwrap();
        let next = client
            .submit(acquire(&module, 2, 13, ActivationMode::Session), 1)
            .unwrap();
        block.release();
        result(&first);
        result(&release);
        result(&next);
        assert_eq!(peak.load(Ordering::SeqCst), 1);
        assert_eq!(module.writes(), 3);
        assert!(
            result(&first).stale,
            "a completed old generation is stale after a later accepted intent"
        );
        finish(&mut runner);
        assert_eq!(module.value(), 7);
        let seen = threads.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert!(seen.iter().all(|thread| thread == &seen[0]));
    }

    #[test]
    fn admission_bounds_reject_oversized_envelopes_before_queue_acceptance() {
        let module = ScalarModule::new(7);
        let mut runner = Runner::start(coordinator(&module), RunnerLimits::default()).unwrap();
        let mut command = acquire(&module, 1, 12, ActivationMode::Session);
        if let Command::Acquire { config, .. } = &mut command {
            config.payload = serde_json::json!({ "value": "x".repeat(256 * 1024) });
        }
        assert!(matches!(
            runner.client().submit(command, 1),
            Err(SubmitError::InvalidEnvelope)
        ));
        assert!(runner.snapshot().queued.is_empty());
        assert_eq!(module.reads(), 0);
        finish(&mut runner);
    }

    #[test]
    fn admission_bounds_retirement_preserves_late_command_protection_across_session_churn() {
        let module = ScalarModule::new(7);
        let mut runner = Runner::start_fake(
            coordinator(&module),
            RunnerLimits {
                queue_capacity: 1,
                generation_owners: 1,
            },
        )
        .unwrap();
        let client = runner.client();
        for identity in 1..=300 {
            let acquire = client
                .submit(acquire(&module, identity, 12, ActivationMode::Session), 1)
                .unwrap();
            result(&acquire);
            let release = client
                .submit(
                    Command::Release {
                        owner: owner(identity),
                    },
                    2,
                )
                .unwrap();
            result(&release);
            assert_eq!(client.retire_generation_owner(owner(identity), 2), Ok(()));
        }
        assert!(matches!(
            client.submit(acquire(&module, 1, 12, ActivationMode::Session), 99),
            Err(SubmitError::RetiredOwner)
        ));
        assert!(client
            .shared
            .admission
            .lock()
            .unwrap()
            .generations
            .is_empty());
        finish(&mut runner);
        assert_eq!(module.value(), 7);
    }

    #[test]
    fn failed_shutdown_cleanup_returns_recovery_required_and_keeps_error_evidence() {
        let module = ScalarModule::new(7);
        let mut runner = Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
        let ticket = runner
            .client()
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        result(&ticket);
        module.set_capability(Capability::Denied("context changed".into()));
        assert_eq!(runner.shutdown(WAIT), ShutdownResult::RecoveryRequired);
        let snapshot = runner.snapshot();
        assert_eq!(snapshot.phase, RunnerPhase::Stopped);
        let cleanup = snapshot
            .shutdown_result
            .expect("typed shutdown result must remain inspectable");
        assert!(cleanup.error.is_some());
        assert!(!cleanup.snapshot.resources.is_empty());
        assert_eq!(module.value(), 12);
    }

    #[test]
    fn retirement_refuses_pending_active_and_unresolved_ownership() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        module.set_observe_hook(Arc::clone(&block.hook));
        let mut runner = Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
        let client = runner.client();
        let ticket = client
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        block.entered();
        assert_eq!(
            client.retire_generation_owner(owner(1), 1),
            Err(RetirementError::Busy)
        );
        block.release();
        result(&ticket);
        assert_eq!(
            client.retire_generation_owner(owner(1), 1),
            Err(RetirementError::Busy)
        );
        assert_eq!(
            client.retire_generation_owner(owner(1), 2),
            Err(RetirementError::GenerationMismatch)
        );
        module.set_capability(Capability::Denied("retain unresolved".into()));
        result(
            &client
                .submit(Command::Release { owner: owner(1) }, 2)
                .unwrap(),
        );
        assert_eq!(
            client.retire_generation_owner(owner(1), 2),
            Err(RetirementError::Busy)
        );
        assert_eq!(runner.shutdown(WAIT), ShutdownResult::RecoveryRequired);
    }

    #[test]
    fn sparse_retirement_tombstones_reject_overflow_without_eviction() {
        let module = ScalarModule::new(7);
        let mut runner = Runner::start_fake(
            coordinator(&module),
            RunnerLimits {
                queue_capacity: 1,
                generation_owners: 1,
            },
        )
        .unwrap();
        let client = runner.client();
        for index in 0..MAX_RETIRED_INTERVALS {
            let identity = 2 * index as u64 + 1;
            result(
                &client
                    .submit(acquire(&module, identity, 12, ActivationMode::Session), 1)
                    .unwrap(),
            );
            result(
                &client
                    .submit(
                        Command::Release {
                            owner: owner(identity),
                        },
                        2,
                    )
                    .unwrap(),
            );
            assert_eq!(client.retire_generation_owner(owner(identity), 2), Ok(()));
        }
        let extra = 2 * MAX_RETIRED_INTERVALS as u64 + 1;
        result(
            &client
                .submit(acquire(&module, extra, 12, ActivationMode::Session), 1)
                .unwrap(),
        );
        result(
            &client
                .submit(
                    Command::Release {
                        owner: owner(extra),
                    },
                    2,
                )
                .unwrap(),
        );
        assert_eq!(
            client.retire_generation_owner(owner(extra), 2),
            Err(RetirementError::TombstoneCapacity)
        );
        assert!(matches!(
            client.submit(acquire(&module, 1, 12, ActivationMode::Session), 99),
            Err(SubmitError::RetiredOwner)
        ));
        let admission = client.shared.admission.lock().unwrap();
        assert_eq!(admission.retired.len(), MAX_RETIRED_INTERVALS);
        assert!(admission.generations.contains_key(&owner(extra)));
        drop(admission);
        finish(&mut runner);
    }
}
