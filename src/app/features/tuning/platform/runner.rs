//! Bounded serial dispatch. The worker owns the coordinator for its full lifetime.
use super::contract::{OperationId, OwnerId};
use super::coordinator::{
    Command, CommandResult, Coordinator, CoordinatorSnapshot, ExecutionControl, GateError,
    WriteIntent,
};
use super::journal::{JournalPhase, JournalStamp};
use super::writer::{SavedRevision, WorkerAuthority, WriterStatus};
#[cfg(test)]
#[path = "runner_reservation_tests.rs"]
mod reservation_tests;
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
    Reserved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReservationError {
    Closed,
    Busy,
    QueueFull,
    Exhausted,
    Stale,
    InvalidPhase,
    Refused,
    TimedOut,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavePhase {
    Queued,
    Granted,
    PublicationStarting,
    PublicationStarted,
    Installing,
    Aborting,
    Installed,
    Aborted,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveSnapshot {
    pub operation: OperationId,
    pub phase: SavePhase,
}
struct ReservationState {
    phase: SavePhase,
    saved: Option<Arc<SavedRevision>>,
    acknowledgements: Option<Arc<super::writer::VerifiedAcknowledgements>>,
}
impl ReservationState {
    fn grant(&mut self) {
        if self.phase == SavePhase::Queued {
            self.phase = SavePhase::Granted;
        }
    }
}
struct ReservationData {
    id: OperationId,
    state: Mutex<ReservationState>,
    changed: Condvar,
    outstanding: AtomicBool,
    abandoned: AtomicBool,
}
impl ReservationData {
    fn phase(&self) -> SavePhase {
        self.state.lock().unwrap().phase
    }
    fn wait_while(
        &self,
        timeout: Duration,
        pending: impl Fn(SavePhase) -> bool,
    ) -> Result<SavePhase, ReservationError> {
        let guard = self.state.lock().unwrap();
        let (guard, result) = self
            .changed
            .wait_timeout_while(guard, timeout, |s| pending(s.phase))
            .unwrap();
        if result.timed_out() && pending(guard.phase) {
            Err(ReservationError::TimedOut)
        } else if guard.phase == SavePhase::Uncertain {
            Err(ReservationError::Refused)
        } else {
            Ok(guard.phase)
        }
    }
}
// One non-clonable continuation; its observer state is not publication permission.
pub(crate) struct SaveReservation {
    data: Arc<ReservationData>,
    shared: Weak<Shared>,
    sender: SyncSender<Dispatch>,
}
impl SaveReservation {
    pub(in crate::app::features::tuning) fn acknowledgements(
        &self,
    ) -> Option<Arc<super::writer::VerifiedAcknowledgements>> {
        if self.phase() != SavePhase::PublicationStarted {
            return None;
        }
        self.data.state.lock().unwrap().acknowledgements.clone()
    }
    pub(crate) fn phase(&self) -> SavePhase {
        self.data.phase()
    }
    pub(crate) fn wait(&self, timeout: Duration) -> Result<SavePhase, ReservationError> {
        self.data
            .wait_while(timeout, |phase| phase == SavePhase::Queued)
    }
    fn enqueue(
        &self,
        pending: SavePhase,
        permitted: impl Fn(SavePhase) -> bool,
        message: Dispatch,
    ) -> Result<(), ReservationError> {
        let shared = self.shared.upgrade().ok_or(ReservationError::Closed)?;
        let admission = shared.admission.lock().unwrap();
        if !admission
            .reservation
            .as_ref()
            .is_some_and(|data| Arc::ptr_eq(data, &self.data))
        {
            return Err(ReservationError::Stale);
        }
        if admission.sender.is_none()
            && !matches!(pending, SavePhase::Installing | SavePhase::Uncertain)
        {
            return Err(ReservationError::Closed);
        }
        let mut state = self.data.state.lock().unwrap();
        if !permitted(state.phase) {
            return Err(ReservationError::InvalidPhase);
        }
        let previous = state.phase;
        state.phase = pending;
        if pending == SavePhase::PublicationStarting {
            self.data.outstanding.store(true, Ordering::Release);
        }
        match self.sender.try_send(message) {
            Ok(()) => {}
            Err(error) => {
                state.phase = previous;
                if pending == SavePhase::PublicationStarting {
                    self.data.outstanding.store(false, Ordering::Release);
                }
                return Err(match error {
                    TrySendError::Full(_) => ReservationError::QueueFull,
                    TrySendError::Disconnected(_) => ReservationError::Closed,
                });
            }
        }
        drop(state);
        drop(admission);
        shared.publish_save(&self.data);
        Ok(())
    }
    pub(crate) fn abort(&mut self, timeout: Duration) -> Result<(), ReservationError> {
        self.enqueue(
            SavePhase::Aborting,
            |p| matches!(p, SavePhase::Queued | SavePhase::Granted),
            Dispatch::Abort(Arc::clone(&self.data)),
        )?;
        match self
            .data
            .wait_while(timeout, |p| p == SavePhase::Aborting)?
        {
            SavePhase::Aborted => Ok(()),
            _ => Err(ReservationError::Refused),
        }
    }
    pub(crate) fn begin_publication(&mut self, timeout: Duration) -> Result<(), ReservationError> {
        self.enqueue(
            SavePhase::PublicationStarting,
            |p| p == SavePhase::Granted,
            Dispatch::StartPublication(Arc::clone(&self.data)),
        )?;
        match self
            .data
            .wait_while(timeout, |p| p == SavePhase::PublicationStarting)?
        {
            SavePhase::PublicationStarted => Ok(()),
            _ => Err(ReservationError::Refused),
        }
    }
    pub(crate) fn publication_uncertain(&mut self) -> Result<(), ReservationError> {
        self.enqueue(
            SavePhase::Uncertain,
            |p| p == SavePhase::PublicationStarted,
            Dispatch::Uncertain(Arc::clone(&self.data)),
        )
    }
    fn install(
        &mut self,
        revision: SavedRevision,
        timeout: Duration,
    ) -> Result<(), ReservationError> {
        self.enqueue(
            SavePhase::Installing,
            |p| p == SavePhase::PublicationStarted,
            Dispatch::Install(Arc::clone(&self.data), Arc::new(revision)),
        )?;
        match self
            .data
            .wait_while(timeout, |p| p == SavePhase::Installing)?
        {
            SavePhase::Installed => Ok(()),
            _ => Err(ReservationError::Refused),
        }
    }
    #[cfg(windows)]
    pub(crate) fn install_native(
        &mut self,
        proof: super::configured_intent::NativeConfigurationProof,
        timeout: Duration,
    ) -> Result<(), ReservationError> {
        self.install(SavedRevision::Native(Box::new(proof)), timeout)
    }
    #[cfg(test)]
    pub(in crate::app::features::tuning) fn install_fixture(
        &mut self,
        bytes: &[u8],
        origin: super::contract::ConfigurationOrigin,
        timeout: Duration,
    ) -> Result<(), ReservationError> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(ReservationError::Refused);
        }
        self.install(
            SavedRevision::Fixture {
                bytes: Arc::from(bytes),
                origin,
            },
            timeout,
        )
    }
}
impl Drop for SaveReservation {
    fn drop(&mut self) {
        if !matches!(self.phase(), SavePhase::Installed | SavePhase::Aborted) {
            self.data.abandoned.store(true, Ordering::Release);
            let _ = self
                .sender
                .try_send(Dispatch::Abandon(Arc::clone(&self.data)));
        }
    }
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
    pub writer: super::writer::WriterStatus,
    pub save: Option<SaveSnapshot>,
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
enum Dispatch {
    Command(Box<WorkItem>),
    Reserve(Arc<ReservationData>),
    StartPublication(Arc<ReservationData>),
    Abort(Arc<ReservationData>),
    Install(Arc<ReservationData>, Arc<SavedRevision>),
    Uncertain(Arc<ReservationData>),
    Abandon(Arc<ReservationData>),
    Stop,
}
struct Admission {
    sender: Option<SyncSender<Dispatch>>,
    reservation: Option<Arc<ReservationData>>,
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
    fn publish_save(&self, data: &ReservationData) {
        let state = data.state.lock().unwrap();
        let mut runtime = self.runtime.lock().unwrap();
        if runtime
            .save
            .is_none_or(|current| current.operation <= data.id)
        {
            runtime.save = Some(SaveSnapshot {
                operation: data.id,
                phase: state.phase,
            });
        }
    }
    fn current_reservation(&self, data: &Arc<ReservationData>) -> bool {
        self.admission
            .lock()
            .unwrap()
            .reservation
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, data))
    }
    fn set_save_phase(&self, data: &ReservationData, phase: SavePhase) {
        data.state.lock().unwrap().phase = phase;
        data.changed.notify_all();
        self.publish_save(data);
    }
    fn grant_reservation(&self, data: &ReservationData) {
        data.state.lock().unwrap().grant();
        data.changed.notify_all();
        self.publish_save(data);
    }
    fn release_reservation(&self, data: &Arc<ReservationData>, phase: SavePhase) {
        let mut admission = self.admission.lock().unwrap();
        if admission
            .reservation
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, data))
        {
            data.outstanding.store(false, Ordering::Release);
            admission.reservation = None;
            if data.abandoned.load(Ordering::Acquire) {
                admission.sender.take();
            }
            self.set_save_phase(data, phase);
        }
    }
    fn publication_outstanding(&self) -> bool {
        self.admission
            .lock()
            .unwrap()
            .reservation
            .as_ref()
            .is_some_and(|data| data.outstanding.load(Ordering::Acquire))
    }
    fn retain_uncertain_publication(&self) -> ! {
        if let Some(data) = self.admission.lock().unwrap().reservation.as_ref().cloned() {
            self.set_save_phase(&data, SavePhase::Uncertain);
        }
        // The exact worker stack still owns authority/coordinator. No replacement
        // or stale-proof shutdown is safe when the caller's publication is unknown.
        loop {
            thread::park();
        }
    }
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
        if let Some(sender) = admission.sender.take() {
            let _ = sender.try_send(Dispatch::Stop);
        }
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
    pub(crate) fn reserve_configuration(&self) -> Result<SaveReservation, ReservationError> {
        let mut admission = self.shared.admission.lock().unwrap();
        if admission.sender.is_none() {
            return Err(ReservationError::Closed);
        }
        if admission.reservation.is_some() {
            return Err(ReservationError::Busy);
        }
        let next = admission
            .next_operation
            .ok_or(ReservationError::Exhausted)?;
        let data = Arc::new(ReservationData {
            id: OperationId::new(next).unwrap(),
            state: Mutex::new(ReservationState {
                phase: SavePhase::Queued,
                saved: None,
                acknowledgements: None,
            }),
            changed: Condvar::new(),
            outstanding: AtomicBool::new(false),
            abandoned: AtomicBool::new(false),
        });
        let sender = admission.sender.as_ref().unwrap().clone();
        match sender.try_send(Dispatch::Reserve(Arc::clone(&data))) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(ReservationError::QueueFull),
            Err(TrySendError::Disconnected(_)) => return Err(ReservationError::Closed),
        }
        admission.next_operation = next.checked_add(1);
        admission.reservation = Some(Arc::clone(&data));
        self.shared.publish_save(&data);
        Ok(SaveReservation {
            data,
            shared: Arc::downgrade(&self.shared),
            sender,
        })
    }
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
        if admission.reservation.is_some() {
            return Err(SubmitError::Reserved);
        }
        if config_generation == 0 {
            return Err(SubmitError::InvalidGeneration);
        }
        if matches!(&command, Command::Acquire { binding, .. } | Command::UpdateMembership { binding, .. } if binding
                .configured()
                .is_some_and(|intent| intent.generation.get() != config_generation))
        {
            // The ticket and durable intent must identify the same saved
            // configuration before admission can cancel any earlier work.
            return Err(SubmitError::InvalidGeneration);
        }
        let generation_owner = match &command {
            Command::Acquire { owner, .. }
            | Command::Release { owner }
            | Command::UpdateMembership { owner, .. } => Some(*owner),
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
        match admission
            .sender
            .as_ref()
            .unwrap()
            .try_send(Dispatch::Command(Box::new(item)))
        {
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
        if admission.reservation.is_some() {
            return Err(RetirementError::Busy);
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
struct WorkerControl<'a> {
    shared: Arc<Shared>,
    ticket: Option<Arc<TicketData>>,
    authority: &'a mut WorkerAuthority,
    possible_write: bool,
    transition_committed: bool,
}
impl WorkerControl<'_> {
    fn stamp(&self) -> JournalStamp {
        self.ticket
            .as_ref()
            .map_or(JournalStamp::Shutdown, |ticket| JournalStamp::Operation {
                operation: ticket.stamp.operation,
                config_generation: ticket.stamp.config_generation,
            })
    }
    fn commit(
        &mut self,
        phase: JournalPhase,
        snapshot: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        let stamp = self.stamp();
        let result = self.authority.commit(stamp, phase, snapshot);
        self.shared.runtime.lock().unwrap().writer = self.authority.status();
        result
    }
}
impl ExecutionControl for WorkerControl<'_> {
    fn before_command(
        &mut self,
        command: &Command,
        snapshot: &CoordinatorSnapshot,
    ) -> Result<(), GateError> {
        self.authority.admit_command(command, snapshot)?;
        if matches!(command,Command::Release {owner} if !snapshot.owners.iter().any(|record|record.owner==*owner))
        {
            // Independently saved exact Disabled input was admitted above. Even
            // an unexecuted withdrawal needs a durable zero-write boundary.
            self.commit(JournalPhase::Transition, snapshot)?;
        }
        Ok(())
    }
    fn before_shutdown(&mut self, snapshot: &CoordinatorSnapshot) -> Result<(), GateError> {
        self.authority.begin_shutdown(snapshot)
    }
    fn cancellation_requested(&self) -> bool {
        self.ticket
            .as_ref()
            .is_some_and(|ticket| ticket.cancellable && ticket.cancelled.load(Ordering::Acquire))
    }
    fn before_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.authority.check_write(intent)?;
        // Cleanup is never abandoned because the initiating generation expired.
        if !intent.operation.is_restoration()
            && !self.possible_write
            && self.cancellation_requested()
        {
            return Err(GateError::Cancelled);
        }
        let step =
            u32::try_from(intent.step).map_err(|_| GateError::Rejected("invalid step index"))?;
        self.commit(
            JournalPhase::BeforeWrite {
                step,
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )?;
        self.possible_write = true;
        let mut runtime = self.shared.runtime.lock().unwrap();
        runtime.possible_write = true;
        runtime.coordinator = intent.ownership.clone();
        Ok(())
    }
    fn before_execute(&mut self, _: &WriteIntent<'_>) -> Result<(), GateError> {
        self.authority.after_intent()
    }
    fn after_verified_write(&mut self, intent: &WriteIntent<'_>) -> Result<(), GateError> {
        self.shared.runtime.lock().unwrap().coordinator = intent.ownership.clone();
        let step =
            u32::try_from(intent.step).map_err(|_| GateError::Rejected("invalid step index"))?;
        self.commit(
            JournalPhase::VerifiedWrite {
                step,
                effect: intent.operation.effect(),
            },
            &intent.ownership,
        )
    }
    fn commit_transition(&mut self, snapshot: &CoordinatorSnapshot) -> Result<(), GateError> {
        self.commit(JournalPhase::Transition, snapshot)?;
        self.transition_committed = true;
        Ok(())
    }
    fn publish(&mut self, snapshot: CoordinatorSnapshot) {
        self.shared.runtime.lock().unwrap().coordinator = snapshot;
    }
}
struct WorkerExit {
    coordinator: Option<Coordinator>,
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
    #[cfg(test)]
    pub(in crate::app::features::tuning) fn start_saved_fixture(
        limits: RunnerLimits,
        catalog: super::catalog::Catalog,
        state: std::path::PathBuf,
        journal: std::path::PathBuf,
    ) -> Result<Self, StartError> {
        reservation_tests::start_saved_fixture(limits, catalog, state, journal)
    }
    /// F2 production admission is read-only even when a writable module exists.
    pub fn start(coordinator: Coordinator, limits: RunnerLimits) -> Result<Self, StartError> {
        Self::spawn(coordinator, limits, WriteAdmission::ReadOnly)
    }
    // Canonical metadata initialization is explicit and remains outside the
    // current application composition. No caller can inject a permissive store.
    #[cfg(windows)]
    pub(in crate::app::features::tuning) fn start_native(
        catalog: super::catalog::Catalog,
        storage: &crate::app::models::AppStateStorage,
        limits: RunnerLimits,
    ) -> Result<Self, StartError> {
        let configuration = storage.native_configuration();
        Self::spawn_factory(CoordinatorSnapshot::default(), limits, move || {
            super::writer::open_native(catalog, configuration)
        })
    }
    #[cfg(test)]
    fn start_fake(coordinator: Coordinator, limits: RunnerLimits) -> Result<Self, StartError> {
        Self::spawn(coordinator, limits, WriteAdmission::Fake)
    }
    #[cfg(test)]
    pub(super) fn start_test_durable(
        coordinator: Coordinator,
        limits: RunnerLimits,
        factory: impl FnOnce() -> super::writer::WorkerAuthority + Send + 'static,
    ) -> Result<Self, StartError> {
        let initial = coordinator.snapshot();
        Self::spawn_factory(initial, limits, move || (coordinator, factory()))
    }
    #[cfg(test)]
    pub(super) fn start_test_writer(
        limits: RunnerLimits,
        factory: impl FnOnce() -> (Coordinator, WorkerAuthority) + Send + 'static,
    ) -> Result<Self, StartError> {
        Self::spawn_factory(CoordinatorSnapshot::default(), limits, factory)
    }
    fn spawn(
        coordinator: Coordinator,
        limits: RunnerLimits,
        write_admission: WriteAdmission,
    ) -> Result<Self, StartError> {
        let initial = coordinator.snapshot();
        Self::spawn_factory(initial, limits, move || {
            let authority = match write_admission {
                WriteAdmission::ReadOnly => WorkerAuthority::ReadOnly,
                #[cfg(test)]
                WriteAdmission::Fake => WorkerAuthority::Fake,
            };
            (coordinator, authority)
        })
    }
    fn spawn_factory(
        initial: CoordinatorSnapshot,
        limits: RunnerLimits,
        factory: impl FnOnce() -> (Coordinator, WorkerAuthority) + Send + 'static,
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
                reservation: None,
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
                coordinator: initial,
                shutdown_result: None,
                writer: super::writer::WriterStatus::ReadOnly,
                save: None,
            }),
        });
        let state = Arc::clone(&shared);
        // A Send factory creates the potentially !Send authority here. Its
        // ownership covers commands, panic handling and shutdown publication.
        let worker = thread::Builder::new()
            .name("tuning-runner".into())
            .spawn(move || worker_loop(factory, receiver, state, done))
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
                self.retained = exit.coordinator;
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
    factory: impl FnOnce() -> (Coordinator, WorkerAuthority),
    receiver: Receiver<Dispatch>,
    shared: Arc<Shared>,
    done: SyncSender<()>,
) -> WorkerExit {
    let (mut coordinator, mut authority) = match catch_unwind(AssertUnwindSafe(factory)) {
        Ok(state) => state,
        Err(_) => {
            complete_faulted(&shared);
            let _ = done.send(());
            return WorkerExit {
                coordinator: None,
                faulted: true,
                recovery_required: true,
            };
        }
    };
    let (faulted, recovery_required) = match catch_unwind(AssertUnwindSafe(|| {
        let snapshot = coordinator.snapshot();
        {
            let mut runtime = shared.runtime.lock().unwrap();
            runtime.writer = authority.status();
            runtime.coordinator = snapshot;
        }
        worker_body(&mut coordinator, &receiver, &shared, &mut authority)
    })) {
        Ok(result) => result,
        Err(_) => {
            // A module's codec can also panic during snapshot export after its
            // command returned. Do not call that code again to report failure.
            complete_faulted(&shared);
            (true, true)
        }
    };
    if shared.publication_outstanding() {
        shared.retain_uncertain_publication();
    }
    // The lease is still held while faulted tickets are made terminal. Only the
    // worker releases it, after its last possible publication/native operation.
    drop(authority);
    let _ = done.send(());
    WorkerExit {
        coordinator: Some(coordinator),
        faulted,
        recovery_required,
    }
}

fn complete_faulted(shared: &Shared) {
    shared.close(true);
    let reservation = shared
        .admission
        .lock()
        .unwrap()
        .reservation
        .as_ref()
        .cloned();
    if let Some(data) = reservation {
        shared.set_save_phase(&data, SavePhase::Uncertain);
    }
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
}

fn handle_save_dispatch(
    message: Dispatch,
    before: &CoordinatorSnapshot,
    shared: &Shared,
    authority: &mut WorkerAuthority,
) {
    if matches!(message, Dispatch::Stop) {
        let reservation = {
            let admission = shared.admission.lock().unwrap();
            admission
                .reservation
                .as_ref()
                .filter(|data| !data.outstanding.load(Ordering::Acquire))
                .cloned()
        };
        if let Some(data) = reservation {
            // Closing prevents publication start. Only a fresh original check
            // permits pre-publication terminal shutdown; it never reopens admission.
            let phase = if authority.reserve_configuration(before).is_ok() {
                SavePhase::Aborted
            } else {
                authority.block_configuration();
                SavePhase::Uncertain
            };
            shared.release_reservation(&data, phase);
        }
        return;
    }
    let data = match &message {
        Dispatch::Reserve(data)
        | Dispatch::StartPublication(data)
        | Dispatch::Abort(data)
        | Dispatch::Install(data, _)
        | Dispatch::Uncertain(data)
        | Dispatch::Abandon(data) => Arc::clone(data),
        Dispatch::Command(_) | Dispatch::Stop => unreachable!(),
    };
    if !shared.current_reservation(&data) {
        return;
    }
    let checked = match message {
        Dispatch::Reserve(_) => {
            if data.abandoned.load(Ordering::Acquire) {
                shared.close(false);
                Err(GateError::Rejected(
                    "reservation abandoned before publication",
                ))
            } else {
                authority
                    .reserve_configuration(before)
                    .map(|()| shared.grant_reservation(&data))
            }
        }
        Dispatch::StartPublication(_) => {
            if data.phase() != SavePhase::PublicationStarting {
                return;
            }
            authority.saved_acknowledgements(before).map(|ack| {
                data.state.lock().unwrap().acknowledgements = Some(Arc::new(ack));
                shared.set_save_phase(&data, SavePhase::PublicationStarted)
            })
        }
        Dispatch::Abort(_) => {
            if data.outstanding.load(Ordering::Acquire) || data.phase() != SavePhase::Aborting {
                return;
            }
            authority
                .reserve_configuration(before)
                .map(|()| shared.release_reservation(&data, SavePhase::Aborted))
        }
        Dispatch::Install(_, revision) => {
            if data.phase() != SavePhase::Installing {
                return;
            }
            data.state.lock().unwrap().saved = Some(Arc::clone(&revision));
            authority
                .install_saved(&revision, before)
                .map(|()| shared.release_reservation(&data, SavePhase::Installed))
        }
        Dispatch::Abandon(_) => {
            shared.close(false);
            Err(GateError::Rejected("reservation abandoned"))
        }
        Dispatch::Uncertain(_) => Err(GateError::Rejected(
            "configuration publication is unverified",
        )),
        Dispatch::Command(_) | Dispatch::Stop => unreachable!(),
    };
    if checked.is_err() {
        // Raw saved proof/input remains in this exact reservation; blocking the
        // session retains its original journal and native authority on the worker.
        authority.block_configuration();
        shared.runtime.lock().unwrap().writer = authority.status();
        shared.set_save_phase(&data, SavePhase::Uncertain);
    }
    shared.runtime.lock().unwrap().writer = authority.status();
}

fn worker_body(
    coordinator: &mut Coordinator,
    receiver: &Receiver<Dispatch>,
    shared: &Arc<Shared>,
    authority: &mut WorkerAuthority,
) -> (bool, bool) {
    let mut faulted = false;
    loop {
        let settle_closed = {
            let admission = shared.admission.lock().unwrap();
            admission.sender.is_none() && admission.pending.is_empty()
        };
        if settle_closed {
            // A full queue may have refused Stop. After all earlier commands
            // settle, closed admission itself triggers the checked pre-start
            // shutdown. Outstanding publication is never settled by Stop.
            handle_save_dispatch(Dispatch::Stop, &coordinator.snapshot(), shared, authority);
            if shared.admission.lock().unwrap().reservation.is_none() {
                break;
            }
        }
        let Ok(message) = receiver.recv() else {
            break;
        };
        let item = match message {
            Dispatch::Command(item) => *item,
            message => {
                handle_save_dispatch(message, &coordinator.snapshot(), shared, authority);
                continue;
            }
        };
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
            authority,
            possible_write: false,
            transition_committed: false,
        };
        match catch_unwind(AssertUnwindSafe(|| {
            coordinator.handle(item.command, &mut control)
        })) {
            Ok(mut result) => {
                let snapshot = coordinator.snapshot();
                result.snapshot = snapshot.clone();
                control.authority.finish_operation();
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
                    if let Dispatch::Command(queued) = queued {
                        shared.complete(
                            &queued.ticket,
                            TerminalOutcome::WorkerPanicked,
                            coordinator.snapshot(),
                        );
                    }
                }
                break;
            }
        }
    }
    if shared.publication_outstanding() {
        shared.retain_uncertain_publication();
    }
    let mut recovery_required = false;
    if !faulted {
        let mut control = WorkerControl {
            shared: Arc::clone(shared),
            ticket: None,
            authority,
            possible_write: false,
            transition_committed: false,
        };
        match catch_unwind(AssertUnwindSafe(|| coordinator.shutdown(&mut control))) {
            Ok(mut result) => {
                result.snapshot = coordinator.snapshot();
                // An unchanged manual ledger still needs an explicit shutdown
                // stamp. Read-only runners have no durable lifecycle to publish.
                if !control.transition_committed && control.authority.has_durable_context() {
                    if let Err(error) = control.commit_transition(&result.snapshot) {
                        result.outcome = super::coordinator::Outcome::RecoveryRequired;
                        if result.error.is_none() {
                            result.error = Some(super::coordinator::CoordinatorError::Gate(error));
                        }
                    }
                }
                recovery_required = result.error.is_some()
                    || matches!(
                        result.outcome,
                        super::coordinator::Outcome::RecoveryRequired
                    );
                shared.runtime.lock().unwrap().shutdown_result = Some(Arc::new(result));
                control.authority.finish_operation();
            }
            Err(_) => {
                faulted = true;
                shared.close(true);
            }
        }
    }
    recovery_required |= matches!(authority.status(), WriterStatus::Blocked(_));
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
    pub(super) struct Block {
        entered: Mutex<mpsc::Receiver<()>>,
        released: Arc<(Mutex<bool>, Condvar)>,
        pub(super) hook: Arc<dyn Fn() + Send + Sync>,
    }
    impl Block {
        pub(super) fn once() -> Self {
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
        pub(super) fn entered(&self) {
            self.entered.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
        pub(super) fn release(&self) {
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
            binding: ActivationBinding::Unknown,
            owner: owner(who),
            config: module.config_envelope(&ScalarConfig { value }).unwrap(),
            target: target(),
            mode,
        }
    }
    fn configured_acquire(
        module: &ScalarModule,
        value: i32,
        generation: u64,
        game: bool,
    ) -> Command {
        let configured = ConfiguredIntent {
            profile: ProfileId::new(1).unwrap(),
            generation: ConfigGeneration::new(generation).unwrap(),
        };
        let binding = if game {
            use crate::app::shared::ids::{GroupId, RuleId};
            let TargetIdentity::Machine(machine) = target() else {
                unreachable!()
            };
            let session = SessionIdentity {
                user: UserIdentity {
                    machine,
                    sid: UserSid::new("S-1-5-21-10-20-30-1000").unwrap(),
                },
                session_id: 1,
                logon: LogonId::new(17).unwrap(),
                boot: BootId::new([2; 16]).unwrap(),
            };
            let process = ProcessIdentity::new(
                session.clone(),
                42,
                123,
                ImagePath::new("C:\\Games\\game.exe").unwrap(),
            )
            .unwrap();
            ActivationBinding::Game(
                GameActivation::new(
                    configured,
                    RuleActivation::new(GroupId("group-0".into()), RuleId("rule-0".into()))
                        .unwrap(),
                    ActivationEligibility::Running,
                    session,
                    vec![process],
                )
                .unwrap(),
            )
        } else {
            ActivationBinding::Manual(configured)
        };
        Command::Acquire {
            owner: owner(1),
            binding,
            config: module.config_envelope(&ScalarConfig { value }).unwrap(),
            target: target(),
            mode: if game {
                ActivationMode::Session
            } else {
                ActivationMode::Manual
            },
        }
    }
    pub(super) fn observe() -> Command {
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

    #[derive(Default)]
    struct DurableMemory {
        bytes: Option<Vec<u8>>,
        records: Vec<super::super::journal::JournalDocument>,
        calls: Vec<std::thread::ThreadId>,
        created: Option<std::thread::ThreadId>,
        dropped: Option<std::thread::ThreadId>,
        publishes: usize,
        fail_at: Option<usize>,
        before_publish: Option<Arc<dyn Fn() + Send + Sync>>,
    }
    struct MemoryStore {
        memory: Arc<Mutex<DurableMemory>>,
        // The authority is intentionally !Send: a Send factory creates it on
        // the worker instead of moving a caller-owned lease across threads.
        _local: std::rc::Rc<()>,
    }
    impl MemoryStore {
        fn open(memory: Arc<Mutex<DurableMemory>>) -> Self {
            memory.lock().unwrap().created = Some(thread::current().id());
            Self {
                memory,
                _local: std::rc::Rc::new(()),
            }
        }
    }
    impl Drop for MemoryStore {
        fn drop(&mut self) {
            self.memory.lock().unwrap().dropped = Some(thread::current().id());
        }
    }
    impl super::super::journal::JournalStore for MemoryStore {
        fn read(
            &mut self,
        ) -> Result<super::super::journal::StoredJournal, super::super::journal::JournalIoError>
        {
            let mut memory = self.memory.lock().unwrap();
            memory.calls.push(thread::current().id());
            Ok(memory.bytes.clone().map_or(
                super::super::journal::StoredJournal::Absent,
                super::super::journal::StoredJournal::Bytes,
            ))
        }
        fn publish(&mut self, bytes: &[u8]) -> Result<(), super::super::journal::JournalIoError> {
            use super::super::journal::*;
            let hook = self.memory.lock().unwrap().before_publish.clone();
            if let Some(hook) = hook {
                hook();
            }
            let mut memory = self.memory.lock().unwrap();
            memory.calls.push(thread::current().id());
            memory.publishes += 1;
            if memory.fail_at == Some(memory.publishes) {
                return Err(JournalIoError {
                    kind: JournalIoKind::Io,
                    operation: "test publication",
                    native_code: Some(5),
                });
            }
            let JournalLoad::Known(document) =
                JournalDocument::load(bytes.to_vec(), &test_machine())
            else {
                panic!("valid typed publication");
            };
            memory.records.push(document);
            memory.bytes = Some(bytes.to_vec());
            Ok(())
        }
        fn validate_authority(&mut self) -> Result<(), super::super::journal::JournalIoError> {
            self.memory
                .lock()
                .unwrap()
                .calls
                .push(thread::current().id());
            Ok(())
        }
    }
    fn test_machine() -> MachineIdentity {
        let TargetIdentity::Machine(machine) = target() else {
            unreachable!()
        };
        machine
    }
    fn durable_runner(module: &ScalarModule, memory: Arc<Mutex<DurableMemory>>) -> Runner {
        use super::super::{
            journal::{DurableSession, JournalStore},
            writer::WorkerAuthority,
        };
        Runner::start_test_durable(coordinator(module), RunnerLimits::default(), move || {
            let store = Box::new(MemoryStore::open(memory)) as Box<dyn JournalStore>;
            WorkerAuthority::Durable(Box::new(DurableSession::bootstrap(store, test_machine())))
        })
        .unwrap()
    }
    #[test]
    fn durable_factory_intent_progress_transition_and_drop_stay_on_one_worker() {
        use super::super::journal::{JournalPhase, JournalStamp};
        let module = ScalarModule::new(7);
        let memory = Arc::new(Mutex::new(DurableMemory::default()));
        let mut runner = durable_runner(&module, memory.clone());
        let ticket = runner
            .client()
            .submit(acquire(&module, 1, 12, ActivationMode::Manual), 3)
            .unwrap();
        let completed = result(&ticket);
        assert!(
            matches!(completed.outcome, TerminalOutcome::Completed(ref result) if result.outcome == super::super::coordinator::Outcome::Applied(Effect::Immediate))
        );
        assert_eq!(module.value(), 12);
        finish(&mut runner);
        let memory = memory.lock().unwrap();
        let worker = memory
            .created
            .expect("worker factory must construct its authority");
        assert_ne!(worker, thread::current().id());
        assert_eq!(memory.dropped, Some(worker));
        assert!(memory.calls.iter().all(|call| *call == worker));
        assert!(matches!(
            memory.records[0].phase,
            JournalPhase::BeforeWrite { step: 0, .. }
        ));
        assert!(matches!(
            memory.records[1].phase,
            JournalPhase::VerifiedWrite { step: 0, .. }
        ));
        assert_eq!(memory.records[2].phase, JournalPhase::Transition);
        assert_eq!(
            memory.records[0].stamp,
            JournalStamp::Operation {
                operation: ticket.stamp().operation,
                config_generation: 3
            }
        );
        assert_eq!(memory.records.last().unwrap().stamp, JournalStamp::Shutdown);
        assert_eq!(
            memory.records.last().unwrap().owners.len(),
            1,
            "manual owner survives orderly shutdown"
        );
    }
    #[test]
    fn durable_failed_intent_makes_zero_calls_and_keeps_observation_available() {
        let module = ScalarModule::new(7);
        let memory = Arc::new(Mutex::new(DurableMemory {
            fail_at: Some(1),
            ..Default::default()
        }));
        let mut runner = durable_runner(&module, memory.clone());
        let ticket = runner
            .client()
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        let completed = result(&ticket);
        assert!(
            matches!(completed.outcome, TerminalOutcome::Completed(ref result) if result.error.is_some())
        );
        assert_eq!(module.writes(), 0);
        assert_eq!(
            memory.lock().unwrap().publishes,
            1,
            "durable intent must be attempted before native write"
        );
        assert!(matches!(
            runner.snapshot().writer,
            super::super::writer::WriterStatus::Blocked(_)
        ));
        let observed = result(&runner.client().submit(observe(), 1).unwrap());
        assert!(
            matches!(observed.outcome, TerminalOutcome::Completed(ref result) if matches!(result.outcome, super::super::coordinator::Outcome::Observed(_)))
        );
        assert_eq!(runner.shutdown(WAIT), ShutdownResult::RecoveryRequired);
    }
    #[test]
    fn durable_progress_failure_keeps_original_and_attempt_without_a_second_write() {
        let module = ScalarModule::new(7);
        let memory = Arc::new(Mutex::new(DurableMemory {
            fail_at: Some(2),
            ..Default::default()
        }));
        let mut runner = durable_runner(&module, memory.clone());
        let completed = result(
            &runner
                .client()
                .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
                .unwrap(),
        );
        assert!(
            matches!(completed.outcome, TerminalOutcome::Completed(ref result) if result.outcome == super::super::coordinator::Outcome::RecoveryRequired)
        );
        assert_eq!(module.value(), 12);
        assert_eq!(module.writes(), 1);
        let snapshot = runner.snapshot().coordinator;
        assert_eq!(snapshot.resources.len(), 1);
        assert!(!snapshot.resources[0].possible.is_empty());
        let memory_guard = memory.lock().unwrap();
        assert_eq!(memory_guard.records.len(), 1);
        assert_eq!(
            memory_guard.records[0].resources[0].baseline,
            snapshot.resources[0].baseline
        );
        assert_eq!(
            memory_guard.records[0].resources[0].attempted,
            snapshot.resources[0].possible
        );
        drop(memory_guard);
        result(
            &runner
                .client()
                .submit(Command::Release { owner: owner(1) }, 1)
                .unwrap(),
        );
        assert_eq!(
            module.writes(),
            1,
            "poisoned durability cannot authorize cleanup retry"
        );
        assert_eq!(runner.shutdown(WAIT), ShutdownResult::RecoveryRequired);
    }
    #[test]
    fn durable_sharing_requires_final_commit_even_without_a_native_write() {
        let module = ScalarModule::new(7);
        let memory = Arc::new(Mutex::new(DurableMemory::default()));
        let mut runner = durable_runner(&module, memory.clone());
        result(
            &runner
                .client()
                .submit(acquire(&module, 1, 12, ActivationMode::Manual), 1)
                .unwrap(),
        );
        {
            let mut memory = memory.lock().unwrap();
            memory.fail_at = Some(memory.publishes + 1);
        }
        let completed = result(
            &runner
                .client()
                .submit(acquire(&module, 2, 12, ActivationMode::Manual), 1)
                .unwrap(),
        );
        assert!(
            matches!(completed.outcome, TerminalOutcome::Completed(ref result) if result.outcome == super::super::coordinator::Outcome::Rejected)
        );
        assert_eq!(module.writes(), 1);
        assert_eq!(completed.snapshot.owners.len(), 1);
        assert_eq!(completed.snapshot.owners[0].owner, owner(1));
        assert_eq!(runner.shutdown(WAIT), ShutdownResult::RecoveryRequired);
    }
    #[test]
    fn durable_timeout_keeps_context_until_cancelled_intent_and_shutdown_are_committed() {
        use super::super::journal::JournalStamp;
        let module = ScalarModule::new(7);
        let block = Block::once();
        let memory = Arc::new(Mutex::new(DurableMemory {
            before_publish: Some(block.hook.clone()),
            ..Default::default()
        }));
        let mut runner = durable_runner(&module, memory.clone());
        let ticket = runner
            .client()
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        block.entered();
        assert_eq!(
            runner.shutdown(Duration::from_millis(10)),
            ShutdownResult::TimedOut
        );
        assert!(memory.lock().unwrap().dropped.is_none());
        assert_eq!(module.writes(), 0);
        block.release();
        assert!(
            matches!(result(&ticket).outcome, TerminalOutcome::Completed(ref result) if result.outcome == super::super::coordinator::Outcome::Cancelled)
        );
        finish(&mut runner);
        assert_eq!(module.writes(), 0);
        let memory = memory.lock().unwrap();
        assert_eq!(memory.dropped, memory.created);
        let final_record = memory.records.last().unwrap();
        assert_eq!(final_record.stamp, JournalStamp::Shutdown);
        assert!(final_record.resources.is_empty() && final_record.owners.is_empty());
    }
    #[test]
    fn durable_factory_timeout_and_drop_retain_partially_constructed_worker_context() {
        use super::super::journal::{DurableSession, JournalStore};
        let module = ScalarModule::new(7);
        let block = Block::once();
        let hook = block.hook.clone();
        let memory = Arc::new(Mutex::new(DurableMemory::default()));
        let worker_memory = memory.clone();
        let mut runner =
            Runner::start_test_durable(coordinator(&module), RunnerLimits::default(), move || {
                let store = MemoryStore::open(worker_memory);
                // This fixture models an unconfirmed factory operation. Only
                // the test barrier supplies eventual completion, never timeout.
                hook();
                WorkerAuthority::Durable(Box::new(DurableSession::bootstrap(
                    Box::new(store) as Box<dyn JournalStore>,
                    test_machine(),
                )))
            })
            .unwrap();
        block.entered();
        assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
        assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
        let client = runner.client();
        drop(runner);
        assert!(memory.lock().unwrap().dropped.is_none());
        assert!(memory.lock().unwrap().calls.is_empty());
        assert_eq!(module.writes(), 0);
        assert!(client.submit(observe(), 1).is_err());
        block.release();
        let started = Instant::now();
        while memory.lock().unwrap().dropped.is_none() && started.elapsed() < WAIT {
            thread::yield_now();
        }
        let memory = memory.lock().unwrap();
        let worker = memory.created.expect("factory created its context");
        assert_ne!(worker, thread::current().id());
        assert_eq!(memory.dropped, Some(worker));
        assert!(memory.calls.iter().all(|call| *call == worker));
        assert_eq!(module.writes(), 0);
    }
    #[test]
    fn durable_drop_during_unconfirmed_intent_cannot_start_a_setting_write() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        let memory = Arc::new(Mutex::new(DurableMemory {
            before_publish: Some(block.hook.clone()),
            ..Default::default()
        }));
        let mut runner = durable_runner(&module, memory.clone());
        let ticket = runner
            .client()
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        block.entered();
        assert!(ticket.wait(Duration::ZERO).is_none());
        assert_eq!(runner.shutdown(Duration::ZERO), ShutdownResult::TimedOut);
        drop(runner);
        assert!(memory.lock().unwrap().dropped.is_none());
        assert!(memory.lock().unwrap().records.is_empty());
        assert_eq!(module.writes(), 0);
        assert!(ticket.wait(Duration::ZERO).is_none());
        block.release();
        assert!(
            matches!(result(&ticket).outcome, TerminalOutcome::Completed(ref result) if result.outcome == super::super::coordinator::Outcome::Cancelled)
        );
        let started = Instant::now();
        while memory.lock().unwrap().dropped.is_none() && started.elapsed() < WAIT {
            thread::yield_now();
        }
        let memory = memory.lock().unwrap();
        assert_eq!(memory.dropped, memory.created);
        assert!(memory.dropped.is_some());
        assert_eq!(module.writes(), 0);
        assert!(memory.records.last().unwrap().resources.is_empty());
    }
    #[test]
    fn durable_drop_retains_blocked_context_until_worker_finishes_compensation() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        module.set_execute_hook(block.hook.clone());
        let memory = Arc::new(Mutex::new(DurableMemory::default()));
        let runner = durable_runner(&module, memory.clone());
        let ticket = runner
            .client()
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        block.entered();
        drop(runner);
        assert!(memory.lock().unwrap().dropped.is_none());
        assert_eq!(
            memory.lock().unwrap().records.len(),
            1,
            "the possible call already has durable intent"
        );
        block.release();
        assert!(
            matches!(result(&ticket).outcome, TerminalOutcome::Completed(ref result) if result.outcome == super::super::coordinator::Outcome::Compensated)
        );
        let started = Instant::now();
        while memory.lock().unwrap().dropped.is_none() && started.elapsed() < WAIT {
            thread::yield_now();
        }
        let memory = memory.lock().unwrap();
        assert_eq!(memory.dropped, memory.created);
        assert!(memory.records.last().unwrap().resources.is_empty());
        assert_eq!(module.value(), 7);
    }
    #[test]
    fn durable_panic_keeps_last_intent_and_releases_context_only_after_terminal_failure() {
        let module = ScalarModule::new(7);
        module.set_execute_hook(Arc::new(|| panic!("test native-call boundary")));
        let memory = Arc::new(Mutex::new(DurableMemory::default()));
        let mut runner = durable_runner(&module, memory.clone());
        let ticket = runner
            .client()
            .submit(acquire(&module, 1, 12, ActivationMode::Session), 1)
            .unwrap();
        assert!(matches!(
            result(&ticket).outcome,
            TerminalOutcome::WorkerPanicked
        ));
        assert_eq!(runner.shutdown(WAIT), ShutdownResult::Faulted);
        let memory = memory.lock().unwrap();
        assert_eq!(memory.dropped, memory.created);
        assert_eq!(
            memory.records.len(),
            1,
            "faulted worker must retain intent, never publish false cleanup"
        );
        assert_eq!(memory.records[0].resources.len(), 1);
        assert!(!memory.records[0].resources[0].attempted.is_empty());
        assert!(matches!(
            runner.client().submit(observe(), 1),
            Err(SubmitError::Closed)
        ));
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
    fn configured_binding_generation_mismatch_never_enters_the_worker() {
        for game in [false, true] {
            let module = ScalarModule::new(7);
            let mut runner =
                Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
            let client = runner.client();
            for submitted in [0, 2, 4] {
                assert!(matches!(
                    client.submit(configured_acquire(&module, 12, 3, game), submitted),
                    Err(SubmitError::InvalidGeneration)
                ));
            }
            assert_eq!(module.reads(), 0);
            assert_eq!(module.writes(), 0);
            assert!(client.snapshot().coordinator.owners.is_empty());
            assert!(client.snapshot().queued.is_empty());
            let valid = client
                .submit(configured_acquire(&module, 12, 3, game), 3)
                .unwrap();
            assert_eq!(valid.stamp().operation.get(), 1);
            let completed = result(&valid);
            assert!(!completed.stale);
            assert_eq!(module.value(), 12);
            result(
                &client
                    .submit(Command::Release { owner: owner(1) }, 4)
                    .unwrap(),
            );
            assert_eq!(module.value(), 7);
            let legacy = client
                .submit(acquire(&module, 1, 13, ActivationMode::Session), 5)
                .unwrap();
            assert_eq!(legacy.stamp().operation.get(), 3);
            result(&legacy);
            assert_eq!(module.value(), 13);
            finish(&mut runner);
            assert_eq!(module.value(), 7);
        }
    }

    #[test]
    fn configured_binding_generation_rejection_preserves_the_accepted_intent() {
        let module = ScalarModule::new(7);
        let block = Block::once();
        module.set_observe_hook(Arc::clone(&block.hook));
        let mut runner = Runner::start_fake(coordinator(&module), RunnerLimits::default()).unwrap();
        let client = runner.client();
        let first = client
            .submit(configured_acquire(&module, 12, 1, false), 1)
            .unwrap();
        block.entered();
        assert!(matches!(
            client.submit(configured_acquire(&module, 13, 1, false), 2),
            Err(SubmitError::InvalidGeneration)
        ));
        assert!(client.snapshot().queued.is_empty());
        assert_eq!(module.writes(), 0);
        block.release();
        let completed = result(&first);
        assert!(!completed.stale);
        assert_eq!(module.value(), 12);
        assert_eq!(module.writes(), 1);
        let edit = client
            .submit(configured_acquire(&module, 13, 2, false), 2)
            .unwrap();
        assert_eq!(edit.stamp().operation.get(), 2);
        result(&edit);
        assert_eq!(module.value(), 13);
        result(
            &client
                .submit(Command::Release { owner: owner(1) }, 3)
                .unwrap(),
        );
        assert_eq!(module.value(), 7);
        finish(&mut runner);
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
