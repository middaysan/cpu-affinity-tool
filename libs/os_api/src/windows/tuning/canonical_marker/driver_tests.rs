//! Private operation traces model owned shape/metadata only, never native proof.
use super::{TuningContinuityCandidateMode as Mode, capsule, driver::*};

const CHAIN: [Component; 4] = [
    Component::Software,
    Component::Application,
    Component::Tuning,
    Component::Marker,
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Context,
    Journal,
    Open(Option<Component>, Component, View),
    Create(Component, Component, bool, View),
    Security(Component, Scope),
    Namespace(Vec<Component>, View, Option<[u8; 32]>),
    Counts(Counts),
    Header(Option<ValueHeader>),
    Data(ValueHeader),
    Generate,
    Set([u8; 56]),
    Probe(View),
}
impl Call {
    fn registry(&self) -> bool {
        !matches!(self, Self::Context | Self::Journal | Self::Generate)
    }
}
fn index(component: Component) -> usize {
    CHAIN.iter().position(|item| *item == component).unwrap()
}
fn context() -> ContextFacts {
    ContextFacts {
        native64_build: true,
        canonical_lease: true,
        elevated: true,
        high_or_system_integrity: true,
        exact_machine_and_token: true,
    }
}
fn encoded() -> [u8; 56] {
    // Deterministic private codec data; no canonical observation is constructed.
    capsule::encode([9; 32]).unwrap()
}
#[derive(Clone, Copy)]
enum PostProbeChange {
    Context,
    Security(Component),
    Namespace,
    Counts(Counts),
}
struct Fake {
    calls: Vec<Call>,
    present: [bool; 4],
    disposition: [Disposition; 4],
    context: ContextFacts,
    journal_absent: bool,
    raw: Option<Vec<u8>>,
    kind: u32,
    extra_values: u32,
    subkeys: u32,
    counts_override: Option<Counts>,
    header_override: Option<Option<ValueHeader>>,
    data_header_override: Option<ValueHeader>,
    generated: [u8; 56],
    security_denied: Option<Component>,
    namespace_valid: bool,
    probe_code: u32,
    change_after_probe: Option<(u32, Vec<u8>)>,
    post_probe_change: Option<PostProbeChange>,
    corrupt_set_last_byte: bool,
    partial_set_failure: bool,
    fail_at: Option<usize>,
}
impl Fake {
    fn existing() -> Self {
        Self {
            calls: vec![],
            present: [true; 4],
            disposition: [Disposition::New; 4],
            context: context(),
            journal_absent: true,
            raw: Some(encoded().to_vec()),
            kind: 3,
            extra_values: 0,
            subkeys: 0,
            counts_override: None,
            header_override: None,
            data_header_override: None,
            generated: encoded(),
            security_denied: None,
            namespace_valid: true,
            probe_code: 1021,
            change_after_probe: None,
            post_probe_change: None,
            corrupt_set_last_byte: false,
            partial_set_failure: false,
            fail_at: None,
        }
    }
    fn fresh() -> Self {
        Self {
            present: [true, false, false, false],
            raw: None,
            ..Self::existing()
        }
    }
    fn record(&mut self, call: Call) -> DriverResult<()> {
        self.calls.push(call);
        if self.fail_at == Some(self.calls.len()) {
            Err(DriverFailure::Native(
                crate::tuning::TuningNativeError::Io {
                    operation: "private fake boundary",
                    code: 1117,
                },
            ))
        } else {
            Ok(())
        }
    }
    fn header(&self) -> Option<ValueHeader> {
        self.header_override.unwrap_or_else(|| {
            self.raw.as_ref().map(|raw| ValueHeader {
                kind: self.kind,
                size: raw.len() as u32,
            })
        })
    }
    fn registry_calls(&self) -> usize {
        self.calls.iter().filter(|call| call.registry()).count()
    }
    fn sets(&self) -> usize {
        self.calls
            .iter()
            .filter(|call| matches!(call, Call::Set(_)))
            .count()
    }
    fn probes(&self) -> usize {
        self.calls
            .iter()
            .filter(|call| matches!(call, Call::Probe(_)))
            .count()
    }
}
impl RegistryDriver for Fake {
    type Key = Component;

    fn verify_context(&mut self) -> DriverResult<ContextFacts> {
        self.record(Call::Context)?;
        Ok(self.context)
    }
    fn journal_is_absent(&mut self) -> DriverResult<bool> {
        self.record(Call::Journal)?;
        Ok(self.journal_absent)
    }
    fn open_component(
        &mut self,
        parent: Option<&Component>,
        component: Component,
        view: View,
    ) -> DriverResult<Option<Component>> {
        self.record(Call::Open(parent.copied(), component, view))?;
        Ok(self.present[index(component)].then_some(component))
    }
    fn create_component(
        &mut self,
        parent: &Component,
        component: Component,
        volatile: bool,
        view: View,
    ) -> DriverResult<(Component, Disposition)> {
        self.record(Call::Create(*parent, component, volatile, view))?;
        self.present[index(component)] = true;
        Ok((component, self.disposition[index(component)]))
    }
    fn validate_security(&mut self, key: &Component, scope: Scope) -> DriverResult<()> {
        self.record(Call::Security(*key, scope))?;
        if self.security_denied == Some(*key) {
            Err(DriverFailure::Unknown(UnknownReason::UnsafeSecurity))
        } else {
            Ok(())
        }
    }
    fn validate_namespace(
        &mut self,
        keys: &[(Component, Component)],
        view: View,
        expected: Option<&CapsuleShape>,
    ) -> DriverResult<bool> {
        assert!(keys.iter().all(|(component, key)| component == key));
        self.record(Call::Namespace(
            keys.iter().map(|(component, _)| *component).collect(),
            view,
            expected.map(|shape| shape.nonce),
        ))?;
        Ok(self.namespace_valid)
    }
    fn marker_counts(&mut self, key: &Component) -> DriverResult<Counts> {
        assert_eq!(*key, Component::Marker);
        let counts = self.counts_override.unwrap_or(Counts {
            values: u32::from(self.raw.is_some()) + self.extra_values,
            subkeys: self.subkeys,
        });
        self.record(Call::Counts(counts))?;
        Ok(counts)
    }
    fn capsule_header(&mut self, key: &Component) -> DriverResult<Option<ValueHeader>> {
        assert_eq!(*key, Component::Marker);
        let header = self.header();
        self.record(Call::Header(header))?;
        Ok(header)
    }
    fn capsule_data(
        &mut self,
        key: &Component,
        buffer: &mut [u8; 56],
    ) -> DriverResult<ValueHeader> {
        assert_eq!(*key, Component::Marker);
        let header = self.data_header_override.unwrap_or_else(|| {
            self.header()
                .expect("bounded header precedes the fixed data query")
        });
        self.record(Call::Data(header))?;
        let raw = self.raw.as_ref().expect("existing private value");
        let count = raw.len().min(buffer.len());
        buffer[..count].copy_from_slice(&raw[..count]);
        Ok(header)
    }
    fn generate_capsule(&mut self) -> DriverResult<[u8; 56]> {
        self.record(Call::Generate)?;
        Ok(self.generated)
    }
    fn set_new_capsule(&mut self, key: &Component, bytes: &[u8; 56]) -> DriverResult<()> {
        assert_eq!(*key, Component::Marker);
        if self.partial_set_failure {
            self.raw = Some(bytes[..12].to_vec());
        }
        self.record(Call::Set(*bytes))?;
        if self.partial_set_failure {
            return Err(DriverFailure::Native(
                crate::tuning::TuningNativeError::Io {
                    operation: "private partial set",
                    code: 1117,
                },
            ));
        }
        self.raw = Some(bytes.to_vec());
        self.kind = 3;
        if self.corrupt_set_last_byte {
            self.raw.as_mut().unwrap()[55] ^= 1;
        }
        Ok(())
    }
    fn volatility_probe(&mut self, key: &Component, view: View) -> DriverResult<u32> {
        assert_eq!(*key, Component::Marker);
        self.record(Call::Probe(view))?;
        if self.probe_code == 0 {
            self.subkeys += 1;
        }
        if let Some((kind, raw)) = self.change_after_probe.take() {
            self.kind = kind;
            self.raw = Some(raw);
        }
        match self.post_probe_change.take() {
            Some(PostProbeChange::Context) => self.context.exact_machine_and_token = false,
            Some(PostProbeChange::Security(key)) => self.security_denied = Some(key),
            Some(PostProbeChange::Namespace) => self.namespace_valid = false,
            Some(PostProbeChange::Counts(counts)) => self.counts_override = Some(counts),
            None => {}
        }
        Ok(self.probe_code)
    }
}

fn assert_views_and_scopes(calls: &[Call]) {
    for call in calls {
        match call {
            Call::Open(parent, component, view) => {
                assert_eq!(*parent, index(*component).checked_sub(1).map(|i| CHAIN[i]));
                assert_eq!(view.flag(), 0x0100);
            }
            Call::Create(parent, component, volatile, view) => {
                assert_ne!(*component, Component::Software);
                assert_eq!(*parent, CHAIN[index(*component) - 1]);
                assert_eq!(*volatile, *component == Component::Marker);
                assert_eq!(view.flag(), 0x0100);
            }
            Call::Namespace(keys, view, _) => {
                assert!(!keys.is_empty() && keys.len() <= CHAIN.len());
                assert_eq!(keys, &CHAIN[..keys.len()]);
                assert_eq!(view.flag(), 0x0100);
            }
            Call::Security(component, scope) => {
                assert_eq!(
                    *scope,
                    if *component == Component::Software {
                        Scope::Ancestor
                    } else {
                        Scope::Managed
                    }
                );
            }
            Call::Probe(view) => assert_eq!(view.flag(), 0x0100),
            _ => {}
        }
    }
}
fn not_observed(result: DriverResult<DriverObservation>) {
    assert!(!matches!(result, Ok(DriverObservation::ObservedShape(_))));
}
fn assert_full_security_replay(calls: &[Call]) {
    for component in CHAIN {
        let scope = if component == Component::Software {
            Scope::Ancestor
        } else {
            Scope::Managed
        };
        assert!(
            calls.contains(&Call::Security(component, scope)),
            "each retained component needs fresh security replay: {component:?}"
        );
    }
}
fn assert_observation_validation(calls: &[Call]) {
    assert_full_security_replay(calls);
    assert!(calls.contains(&Call::Context));
    assert!(calls.iter().any(|call| matches!(call,
        Call::Namespace(keys, View::Native64, Some(nonce)) if keys == &CHAIN && *nonce == [9; 32]
    )));
    assert!(calls.contains(&Call::Counts(Counts {
        values: 1,
        subkeys: 0
    })));
    assert!(calls.contains(&Call::Header(Some(ValueHeader { kind: 3, size: 56 }))));
    assert!(calls.contains(&Call::Data(ValueHeader { kind: 3, size: 56 })));
}
fn assert_latched(session: &mut DriverSession<Component>, fake: &mut Fake) {
    let registry = fake.registry_calls();
    let probes = fake.probes();
    let raw = fake.raw.clone();
    not_observed(session.observe(fake));
    assert_eq!(
        fake.registry_calls(),
        registry,
        "latched Unknown cannot requery native registry"
    );
    assert_eq!(
        fake.probes(),
        probes,
        "latched Unknown cannot renew a volatility probe"
    );
    assert_eq!(fake.raw, raw);
}

#[test]
fn driver_relative_operations_use_exact_chain_native64_and_security_scope() {
    for (mut fake, mode) in [
        (Fake::existing(), Mode::ObserveExisting),
        (
            Fake::fresh(),
            Mode::InitializeOnlyWhenCanonicalJournalAbsent,
        ),
    ] {
        let mut session =
            begin(&mut fake, mode).expect("valid private flow reaches an opaque session");
        assert_eq!(fake.probes(), 0, "constructor does not probe volatility");
        assert_eq!(
            fake.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Open(_, component, _) => Some(*component),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            CHAIN
        );
        for component in CHAIN {
            assert!(
                fake.calls
                    .iter()
                    .any(|call| matches!(call, Call::Security(key, _) if *key == component))
            );
        }
        let observation_start = fake.calls.len();
        assert_eq!(
            session.observe(&mut fake),
            Ok(DriverObservation::ObservedShape(
                capsule::decode(3, &encoded()).unwrap()
            ))
        );
        assert_eq!(fake.probes(), 1);
        let observed_calls = &fake.calls[observation_start..];
        let probe = observed_calls
            .iter()
            .position(|call| matches!(call, Call::Probe(_)))
            .unwrap();
        assert_eq!(observed_calls.first(), Some(&Call::Context));
        assert_eq!(observed_calls.last(), Some(&Call::Context));
        assert_observation_validation(&observed_calls[..probe]);
        assert_observation_validation(&observed_calls[probe + 1..]);
        assert_views_and_scopes(&fake.calls);
    }
}

#[test]
fn driver_missing_observe_never_creates_sets_or_probes_and_never_creates_software() {
    for missing in CHAIN {
        let mut fake = Fake::existing();
        fake.present[index(missing)] = false;
        assert!(begin(&mut fake, Mode::ObserveExisting).is_err());
        assert_eq!(
            fake.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Open(_, component, _) => Some(*component),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            CHAIN[..=index(missing)]
        );
        assert!(!fake.calls.iter().any(|call| matches!(
            call,
            Call::Create(..) | Call::Set(_) | Call::Probe(_) | Call::Generate
        )));
        assert_views_and_scopes(&fake.calls);
    }
    let mut fake = Fake::fresh();
    fake.present[0] = false;
    assert!(begin(&mut fake, Mode::InitializeOnlyWhenCanonicalJournalAbsent).is_err());
    assert!(
        !fake
            .calls
            .iter()
            .any(|call| matches!(call, Call::Create(..) | Call::Set(_) | Call::Generate))
    );
}

#[test]
fn driver_present_journal_and_denied_context_stop_before_all_registry_calls() {
    for mut fake in [Fake::fresh(), Fake::existing()] {
        fake.journal_absent = false;
        let raw = fake.raw.clone();
        assert!(begin(&mut fake, Mode::InitializeOnlyWhenCanonicalJournalAbsent).is_err());
        assert_eq!(fake.calls, vec![Call::Context, Call::Journal]);
        assert_eq!(fake.registry_calls(), 0);
        assert_eq!(fake.raw, raw);
    }
    for alteration in 0..5 {
        let mut fake = Fake::existing();
        match alteration {
            0 => fake.context.native64_build = false,
            1 => fake.context.canonical_lease = false,
            2 => fake.context.elevated = false,
            3 => fake.context.high_or_system_integrity = false,
            _ => fake.context.exact_machine_and_token = false,
        }
        assert!(begin(&mut fake, Mode::ObserveExisting).is_err());
        assert_eq!(fake.calls, vec![Call::Context]);
    }
    let mut fake = Fake::existing();
    fake.journal_absent = false;
    let raw = fake.raw.clone();
    let mut session = begin(&mut fake, Mode::ObserveExisting)
        .expect("present retained journal does not prohibit observing an existing marker");
    assert!(matches!(
        session.observe(&mut fake),
        Ok(DriverObservation::ObservedShape(_))
    ));
    assert_eq!(fake.raw, raw);
    assert!(
        !fake
            .calls
            .iter()
            .any(|call| matches!(call, Call::Create(..) | Call::Set(_) | Call::Generate))
    );
}

#[test]
fn driver_fresh_set_requires_security_namespace_empty_counts_and_exact_readback() {
    let mode = Mode::InitializeOnlyWhenCanonicalJournalAbsent;
    let mut fake = Fake::fresh();
    let _session = begin(&mut fake, mode).expect("one valid fresh initialization");
    assert_eq!(fake.sets(), 1);
    assert_eq!(fake.raw.as_deref(), Some(encoded().as_slice()));
    let set = fake
        .calls
        .iter()
        .position(|call| matches!(call, Call::Set(_)))
        .unwrap();
    let marker = fake
        .calls
        .iter()
        .position(|call| matches!(call, Call::Create(_, Component::Marker, true, _)))
        .unwrap();
    let generate = fake
        .calls
        .iter()
        .position(|call| matches!(call, Call::Generate))
        .unwrap();
    assert!(generate < marker);
    let before_set = &fake.calls[marker + 1..set];
    assert!(
        before_set
            .iter()
            .any(|call| matches!(call, Call::Security(Component::Marker, Scope::Managed)))
    );
    assert!(
        before_set
            .iter()
            .any(|call| matches!(call, Call::Namespace(keys, _, _) if keys == &CHAIN))
    );
    assert!(before_set.iter().any(|call| matches!(
        call,
        Call::Counts(Counts {
            values: 0,
            subkeys: 0
        })
    )));
    assert!(
        before_set.contains(&Call::Context),
        "context is freshly revalidated before Set"
    );
    let after_set = &fake.calls[set + 1..];
    assert_full_security_replay(after_set);
    assert!(
        after_set
            .iter()
            .any(|call| matches!(call, Call::Data(ValueHeader { kind: 3, size: 56 })))
    );
    assert!(after_set.iter().any(|call| matches!(
        call,
        Call::Counts(Counts {
            values: 1,
            subkeys: 0
        })
    )));
    assert!(
        after_set
            .iter()
            .any(|call| matches!(call, Call::Namespace(_, _, Some(nonce)) if *nonce == [9; 32]))
    );
    assert!(after_set.contains(&Call::Context));
    assert_views_and_scopes(&fake.calls);
    for alteration in 0..4 {
        let mut fake = Fake::fresh();
        match alteration {
            0 => fake.security_denied = Some(Component::Marker),
            1 => fake.namespace_valid = false,
            2 => {
                fake.counts_override = Some(Counts {
                    values: 1,
                    subkeys: 0,
                })
            }
            _ => {
                fake.counts_override = Some(Counts {
                    values: 0,
                    subkeys: 1,
                })
            }
        }
        assert!(begin(&mut fake, mode).is_err());
        assert_eq!(fake.sets(), 0);
    }
    let mut fake = Fake::fresh();
    fake.corrupt_set_last_byte = true;
    assert!(
        begin(&mut fake, mode).is_err(),
        "valid-but-different last nonce byte fails exact initialization readback"
    );
    assert_eq!(fake.sets(), 1);
    assert_ne!(fake.raw.as_deref(), Some(encoded().as_slice()));
    assert_eq!(fake.probes(), 0);
}

#[test]
fn driver_malformed_generated_capsule_stops_before_marker_creation_or_set() {
    let mut bad_header = encoded();
    bad_header[0] ^= 1;
    let mut zero_nonce = encoded();
    zero_nonce[24..].fill(0);
    for generated in [[0; 56], bad_header, zero_nonce] {
        let mut fake = Fake::fresh();
        fake.generated = generated;
        assert!(begin(&mut fake, Mode::InitializeOnlyWhenCanonicalJournalAbsent).is_err());
        assert!(!fake.calls.iter().any(|call| matches!(
            call,
            Call::Create(_, Component::Marker, _, _) | Call::Set(_)
        )));
        assert!(fake.raw.is_none());
    }
}

#[test]
fn driver_existing_disposition_preserves_unknown_missing_partial_and_extra_data() {
    for mode in [
        Mode::ObserveExisting,
        Mode::InitializeOnlyWhenCanonicalJournalAbsent,
    ] {
        for raw in [
            None,
            Some(vec![1; 12]),
            Some(vec![0; 56]),
            Some(encoded().to_vec()),
        ] {
            let mut fake = Fake::existing();
            fake.raw = raw.clone();
            if raw.as_deref() == Some(encoded().as_slice()) {
                assert!(begin(&mut fake, mode).is_ok());
            } else {
                assert!(begin(&mut fake, mode).is_err());
            }
            assert_eq!(fake.raw, raw);
            assert_eq!(fake.sets(), 0);
            assert!(
                !fake
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Generate | Call::Create(..) | Call::Probe(_)))
            );
        }
        let mut raced_existing = Fake::fresh();
        raced_existing.raw = Some(vec![7; 12]);
        raced_existing.disposition[3] = Disposition::Existing;
        if mode == Mode::InitializeOnlyWhenCanonicalJournalAbsent {
            assert!(begin(&mut raced_existing, mode).is_err());
            assert_eq!(raced_existing.sets(), 0);
            assert_eq!(raced_existing.raw, Some(vec![7; 12]));
        }
        for (extra_values, subkeys) in [(1, 0), (0, 1), (u32::MAX - 1, u32::MAX)] {
            let mut fake = Fake::existing();
            fake.extra_values = extra_values;
            fake.subkeys = subkeys;
            let raw = fake.raw.clone();
            assert!(begin(&mut fake, mode).is_err());
            assert_eq!(fake.raw, raw);
            assert_eq!(fake.sets(), 0);
            assert_eq!(fake.probes(), 0);
        }
    }
}

#[test]
fn driver_each_begin_native_failure_stops_at_its_exact_boundary_without_retry() {
    for (mode, fresh) in [
        (Mode::ObserveExisting, false),
        (Mode::InitializeOnlyWhenCanonicalJournalAbsent, true),
    ] {
        let mut good = if fresh {
            Fake::fresh()
        } else {
            Fake::existing()
        };
        let _session =
            begin(&mut good, mode).expect("successful trace defines each native boundary");
        for boundary in 1..=good.calls.len() {
            let mut fake = if fresh {
                Fake::fresh()
            } else {
                Fake::existing()
            };
            fake.fail_at = Some(boundary);
            assert!(
                matches!(begin(&mut fake, mode), Err(DriverFailure::Native(_))),
                "native error at boundary {boundary} cannot produce a session"
            );
            assert_eq!(
                fake.calls,
                good.calls[..boundary],
                "no action after boundary {boundary}"
            );
            assert_eq!(fake.probes(), 0);
            assert!(fake.sets() <= 1);
        }
    }
    let mut fake = Fake::fresh();
    fake.partial_set_failure = true;
    assert!(begin(&mut fake, Mode::InitializeOnlyWhenCanonicalJournalAbsent).is_err());
    let retained = fake.raw.clone();
    assert_eq!(retained.as_ref().unwrap().len(), 12);
    fake.calls.clear();
    fake.partial_set_failure = false;
    assert!(begin(&mut fake, Mode::InitializeOnlyWhenCanonicalJournalAbsent).is_err());
    assert_eq!(fake.raw, retained);
    assert_eq!(
        fake.sets(),
        0,
        "an interrupted existing key cannot be repaired/reinitialized"
    );
}

#[test]
fn driver_header_bounds_and_repeated_data_type_or_size_cannot_be_bypassed() {
    for header in [
        None,
        Some(ValueHeader { kind: 1, size: 56 }),
        Some(ValueHeader { kind: 3, size: 0 }),
        Some(ValueHeader { kind: 3, size: 55 }),
        Some(ValueHeader { kind: 3, size: 57 }),
        Some(ValueHeader {
            kind: 3,
            size: u32::MAX,
        }),
    ] {
        let mut fake = Fake::existing();
        fake.header_override = Some(header);
        let raw = fake.raw.clone();
        assert!(begin(&mut fake, Mode::ObserveExisting).is_err());
        assert!(!fake.calls.iter().any(|call| matches!(call, Call::Data(_))));
        assert_eq!(fake.raw, raw);
        assert_eq!(fake.sets(), 0);
    }
    for changed in [
        ValueHeader { kind: 1, size: 56 },
        ValueHeader { kind: 3, size: 55 },
        ValueHeader { kind: 3, size: 57 },
    ] {
        let mut fake = Fake::existing();
        fake.data_header_override = Some(changed);
        assert!(begin(&mut fake, Mode::ObserveExisting).is_err());
        assert!(
            fake.calls
                .iter()
                .any(|call| matches!(call, Call::Data(header) if *header == changed))
        );
        assert_eq!(fake.sets(), 0);
    }
}

#[test]
fn driver_every_observe_native_error_latches_without_a_second_registry_or_probe_call() {
    let mut good = Fake::existing();
    let mut session = begin(&mut good, Mode::ObserveExisting).unwrap();
    good.calls.clear();
    assert!(matches!(
        session.observe(&mut good),
        Ok(DriverObservation::ObservedShape(_))
    ));
    let observation_trace = good.calls;
    for boundary in 1..=observation_trace.len() {
        let mut fake = Fake::existing();
        let mut session = begin(&mut fake, Mode::ObserveExisting).unwrap();
        fake.calls.clear();
        fake.fail_at = Some(boundary);
        assert!(
            matches!(session.observe(&mut fake), Err(DriverFailure::Native(_))),
            "native observe boundary {boundary} cannot issue positive shape"
        );
        assert_eq!(fake.calls, observation_trace[..boundary]);
        assert_latched(&mut session, &mut fake);
        assert!(fake.probes() <= 1);
    }
}

#[test]
fn driver_observe_drift_before_or_after_probe_latches_unknown_and_preserves_value() {
    for alteration in 0..8 {
        let mut fake = Fake::existing();
        let mut session = begin(&mut fake, Mode::ObserveExisting).unwrap();
        fake.calls.clear();
        match alteration {
            0 => fake.context.exact_machine_and_token = false,
            1 => fake.security_denied = Some(Component::Marker),
            2 => fake.namespace_valid = false,
            3 => fake.extra_values = 1,
            4 => fake.subkeys = 1,
            5 => fake.kind = 1,
            6 => fake.raw = Some(vec![1; 12]),
            _ => fake.raw.as_mut().unwrap()[55] ^= 1,
        }
        let raw = fake.raw.clone();
        not_observed(session.observe(&mut fake));
        assert_eq!(fake.probes(), 0);
        assert_eq!(fake.raw, raw);
        assert_latched(&mut session, &mut fake);
    }
    let mut changed_nonce = encoded().to_vec();
    changed_nonce[55] ^= 1;
    for change in [
        (1, encoded().to_vec()),
        (3, vec![1; 12]),
        (3, changed_nonce),
    ] {
        let mut fake = Fake::existing();
        let mut session = begin(&mut fake, Mode::ObserveExisting).unwrap();
        fake.calls.clear();
        fake.change_after_probe = Some(change.clone());
        not_observed(session.observe(&mut fake));
        assert_eq!(fake.probes(), 1);
        assert_eq!(fake.raw, Some(change.1));
        assert_latched(&mut session, &mut fake);
    }
}

#[test]
fn driver_post_probe_context_security_namespace_and_counts_drift_cannot_issue_shape() {
    for change in [
        PostProbeChange::Context,
        PostProbeChange::Security(Component::Software),
        PostProbeChange::Security(Component::Application),
        PostProbeChange::Security(Component::Tuning),
        PostProbeChange::Security(Component::Marker),
        PostProbeChange::Namespace,
        PostProbeChange::Counts(Counts {
            values: 0,
            subkeys: 0,
        }),
        PostProbeChange::Counts(Counts {
            values: 2,
            subkeys: 0,
        }),
        PostProbeChange::Counts(Counts {
            values: 1,
            subkeys: 1,
        }),
    ] {
        let mut fake = Fake::existing();
        let mut session = begin(&mut fake, Mode::ObserveExisting).unwrap();
        fake.calls.clear();
        fake.post_probe_change = Some(change);
        let raw = fake.raw.clone();
        not_observed(session.observe(&mut fake));
        assert_eq!(
            fake.probes(),
            1,
            "the injected drift begins only after the first valid probe"
        );
        assert_eq!(
            fake.raw, raw,
            "changed authority never repairs retained value bytes"
        );
        assert_latched(&mut session, &mut fake);
    }
}

#[test]
fn driver_failed_probe_latches_and_retained_child_blocks_a_new_session() {
    for code in [0, 2, 5, 87, 1018, 1722, u32::MAX] {
        let mut fake = Fake::existing();
        let mut session = begin(&mut fake, Mode::ObserveExisting).unwrap();
        fake.calls.clear();
        fake.probe_code = code;
        assert_eq!(session.observe(&mut fake), Ok(DriverObservation::Unknown));
        assert_eq!(fake.probes(), 1);
        assert_latched(&mut session, &mut fake);
        if code == 0 {
            assert_eq!(
                fake.subkeys, 1,
                "unexpected successful child creation is retained"
            );
            fake.calls.clear();
            assert!(begin(&mut fake, Mode::ObserveExisting).is_err());
            assert_eq!(fake.subkeys, 1);
            assert_eq!(
                fake.probes(),
                0,
                "a later guard cannot grow a second child artifact"
            );
            assert_eq!(fake.sets(), 0);
        }
    }
}
