//! Private decision facts cannot construct the canonical native guard/data.
use super::{TuningContinuityCandidateMode as Mode, UnknownReason};

#[derive(Clone, Copy, Debug)]
pub(super) struct ContextFacts {
    pub native64_build: bool,
    pub canonical_lease: bool,
    pub elevated: bool,
    pub high_or_system_integrity: bool,
    pub exact_machine_and_token: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Component {
    Software,
    Application,
    Tuning,
    Marker,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MissingAction {
    CreateSingleNonvolatile,
    CreateSingleVolatile,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disposition {
    New,
    Existing,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct MarkerFacts {
    pub disposition: Disposition,
    pub security_verified: bool,
    pub namespace_verified: bool,
    pub values: u32,
    pub subkeys: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Volatility {
    Positive,
    Unknown,
}
pub(super) fn admit_context(facts: ContextFacts) -> Result<(), UnknownReason> {
    if facts.native64_build
        && facts.canonical_lease
        && facts.elevated
        && facts.high_or_system_integrity
        && facts.exact_machine_and_token
    {
        Ok(())
    } else {
        Err(UnknownReason::Context)
    }
}
pub(super) fn missing_action(
    mode: Mode,
    component: Component,
    canonical_journal_absent: bool,
) -> Result<MissingAction, UnknownReason> {
    if mode == Mode::ObserveExisting
        || component == Component::Software
        || !canonical_journal_absent
    {
        return Err(UnknownReason::MissingComponent);
    }
    Ok(if component == Component::Marker {
        MissingAction::CreateSingleVolatile
    } else {
        MissingAction::CreateSingleNonvolatile
    })
}
pub(super) fn may_set_new_capsule(
    mode: Mode,
    journal_absent: bool,
    facts: MarkerFacts,
) -> Result<bool, UnknownReason> {
    if facts.disposition == Disposition::Existing {
        return Ok(false);
    }
    if mode == Mode::InitializeOnlyWhenCanonicalJournalAbsent
        && journal_absent
        && facts.security_verified
        && facts.namespace_verified
        && facts.values == 0
        && facts.subkeys == 0
    {
        Ok(true)
    } else {
        Err(UnknownReason::Initialization)
    }
}
// Shape characterization only. The operation engine performs actual repeated
// checks and owns its permanent Unknown latch before any native probe.
#[cfg(test)]
pub(super) fn may_probe(facts: MarkerFacts, capsule_valid: bool, unknown_latched: bool) -> bool {
    facts.security_verified
        && facts.namespace_verified
        && facts.values == 1
        && facts.subkeys == 0
        && capsule_valid
        && !unknown_latched
}
pub(super) fn probe_result(native_code: u32) -> Volatility {
    if native_code == 1021 {
        Volatility::Positive
    } else {
        Volatility::Unknown
    }
}
