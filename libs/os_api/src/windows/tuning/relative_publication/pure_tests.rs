use super::*;
use std::{cell::RefCell, rc::Rc};

#[test]
fn artifact_combinations_never_treat_interrupted_or_unknown_state_as_absent() {
    for bits in 0..8 {
        let journal = bits & 4 != 0;
        let stage = bits & 2 != 0;
        let previous = bits & 1 != 0;
        let expected = match bits {
            0 => Ok(CleanState::Absent),
            4 => Ok(CleanState::Present),
            _ => Err(TuningNativeError::UnsafeStore(
                "relative publication artifacts",
            )),
        };
        assert_eq!(clean_state(journal, stage, previous, false), expected);
        assert_eq!(
            clean_state(journal, stage, previous, true),
            Err(TuningNativeError::UnsafeStore(
                "relative publication artifacts"
            ))
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Clean,
    Old,
    CreateStage,
    WriteSync(Vec<u8>),
    CheckSource(Source),
    RenameOld,
    RenameStage,
    VerifyNew,
    DeleteOld,
    Boundary(Boundary),
}
struct Fake {
    old: bool,
    trace: Vec<Call>,
    fault: Option<usize>,
}
impl Fake {
    fn new(old: bool) -> Self {
        Self {
            old,
            trace: Vec::new(),
            fault: None,
        }
    }
    fn call(&mut self, call: Call) -> Result<()> {
        self.trace.push(call);
        if self.fault == Some(self.trace.len() - 1) {
            Err(TuningNativeError::UnsafeStore(
                "relative publication injected fault",
            ))
        } else {
            Ok(())
        }
    }
}
impl PublicationDriver for Fake {
    fn check_clean(&mut self) -> Result<()> {
        self.call(Call::Clean)
    }
    fn authenticate_old(&mut self) -> Result<bool> {
        self.call(Call::Old)?;
        Ok(self.old)
    }
    fn create_stage(&mut self) -> Result<()> {
        self.call(Call::CreateStage)
    }
    fn write_sync_stage(&mut self, bytes: &[u8]) -> Result<()> {
        self.call(Call::WriteSync(bytes.to_vec()))
    }
    fn check_source_name(&mut self, source: Source) -> Result<()> {
        self.call(Call::CheckSource(source))
    }
    fn rename_old_to_previous(&mut self) -> Result<()> {
        self.call(Call::RenameOld)
    }
    fn rename_stage_to_journal(&mut self) -> Result<()> {
        self.call(Call::RenameStage)
    }
    fn verify_new_name(&mut self) -> Result<()> {
        self.call(Call::VerifyNew)
    }
    fn delete_old_handle(&mut self) -> Result<()> {
        self.call(Call::DeleteOld)
    }
    fn boundary(&mut self, boundary: Boundary) -> Result<()> {
        self.call(Call::Boundary(boundary))
    }
}
fn expected(old: bool, bytes: &[u8]) -> Vec<Call> {
    let mut trace = vec![
        Call::Clean,
        Call::Old,
        Call::CreateStage,
        Call::WriteSync(bytes.to_vec()),
        Call::Boundary(Boundary::StageSynced),
    ];
    if old {
        trace.extend([
            Call::CheckSource(Source::Old),
            Call::RenameOld,
            Call::Boundary(Boundary::OldMoved),
        ]);
    }
    trace.extend([
        Call::CheckSource(Source::Stage),
        Call::RenameStage,
        Call::Boundary(Boundary::NewMoved),
        Call::VerifyNew,
        Call::Boundary(Boundary::Verified),
    ]);
    if old {
        trace.extend([
            Call::VerifyNew,
            Call::DeleteOld,
            Call::Boundary(Boundary::OldDeleted),
        ]);
    }
    trace.push(Call::Clean);
    trace
}

#[test]
fn protocol_requires_sync_source_namespace_checks_and_verified_exact_old_cleanup() {
    for old in [false, true] {
        let bytes = [0x13, 0x37, 0, 0xfe];
        let mut fake = Fake::new(old);
        assert_eq!(protocol(&mut fake, &bytes), Ok(()));
        assert_eq!(fake.trace, expected(old, &bytes));
    }
}
#[test]
fn every_protocol_fault_stops_without_retry_repair_or_later_operation() {
    for old in [false, true] {
        let bytes = b"bounded test payload";
        let trace = expected(old, bytes);
        for index in 0..trace.len() {
            let mut fake = Fake::new(old);
            fake.fault = Some(index);
            assert_eq!(
                protocol(&mut fake, bytes),
                Err(TuningNativeError::UnsafeStore(
                    "relative publication injected fault"
                ))
            );
            assert_eq!(fake.trace, trace[..=index]);
        }
    }
}
#[test]
fn oversized_protocol_input_is_refused_before_any_driver_call() {
    let mut fake = Fake::new(true);
    assert_eq!(
        protocol(&mut fake, &vec![0; CAPACITY + 1]),
        Err(TuningNativeError::Capacity {
            size: (CAPACITY + 1) as u64,
            limit: CAPACITY
        })
    );
    assert!(fake.trace.is_empty());
}

struct DropSpy(Rc<RefCell<Vec<&'static str>>>, &'static str);
impl Drop for DropSpy {
    fn drop(&mut self) {
        self.0.borrow_mut().push(self.1);
    }
}
#[test]
fn only_exact_unresolved_native_category_requires_whole_context_retention() {
    assert_eq!(
        retention_for(&TuningNativeError::UnsafeStore(
            "relative pending completion"
        )),
        Retention::Unresolved
    );
    for error in [
        TuningNativeError::UnsafeStore("relative publication artifacts"),
        TuningNativeError::Unsupported("relative pending completion"),
        TuningNativeError::Busy,
        TuningNativeError::Io {
            operation: "synthetic terminal",
            code: 5,
        },
    ] {
        assert_eq!(retention_for(&error), Retention::Terminal);
    }
}
#[test]
fn unresolved_context_preserves_root_lease_old_stage_and_partial_construction() {
    for fields in [
        vec!["root"],
        vec!["root", "lease"],
        vec!["root", "lease", "old", "stage", "input", "output"],
    ] {
        let trace = Rc::new(RefCell::new(Vec::new()));
        let owned = Box::new(
            fields
                .into_iter()
                .map(|label| DropSpy(Rc::clone(&trace), label))
                .collect::<Vec<_>>(),
        );
        dispose_context(owned, Retention::Unresolved);
        assert!(
            trace.borrow().is_empty(),
            "whole unresolved context remains owned"
        );
    }
}
#[test]
fn terminal_context_releases_all_owned_fields_once() {
    let trace = Rc::new(RefCell::new(Vec::new()));
    let owned = Box::new(vec![
        DropSpy(Rc::clone(&trace), "root"),
        DropSpy(Rc::clone(&trace), "lease"),
    ]);
    dispose_context(owned, Retention::Terminal);
    assert_eq!(*trace.borrow(), ["root", "lease"]);
}

#[test]
fn unresolved_retention_cannot_be_demoted_by_rejected_retry_or_terminal_error() {
    let pending = TuningNativeError::UnsafeStore("relative pending completion");
    assert_eq!(
        latch_retention(Retention::Terminal, &pending),
        Retention::Unresolved
    );
    for error in [
        pending,
        TuningNativeError::Poisoned,
        TuningNativeError::Busy,
        TuningNativeError::UnsafeStore("relative publication source changed"),
        TuningNativeError::Io {
            operation: "synthetic terminal",
            code: 5,
        },
        TuningNativeError::Capacity {
            size: (CAPACITY + 1) as u64,
            limit: CAPACITY,
        },
    ] {
        assert_eq!(
            latch_retention(Retention::Unresolved, &error),
            Retention::Unresolved
        );
    }
    assert_eq!(
        latch_retention(Retention::Terminal, &TuningNativeError::Poisoned),
        Retention::Terminal
    );
}
