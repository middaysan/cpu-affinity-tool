//! Private operation driver: associated keys/shapes never issue public authority.
//! The native implementation will borrow the real store ephemerally and retain
//! actual context outside this driver. No cached fact is a native authorization.

use super::{TuningContinuityCandidateMode as Mode, capsule, control};
pub(super) use super::{
    UnknownReason,
    capsule::DecodedCapsule as CapsuleShape,
    control::{Component, ContextFacts, Disposition},
    registry_policy::Scope,
};
use crate::tuning::TuningNativeError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum View {
    Native64,
}
impl View {
    pub const fn flag(self) -> u32 {
        0x0100
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Counts {
    pub values: u32,
    pub subkeys: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ValueHeader {
    pub kind: u32,
    pub size: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DriverFailure {
    Unknown(UnknownReason),
    Native(TuningNativeError),
}
pub(super) type DriverResult<T> = Result<T, DriverFailure>;

/// Fixed components and view, never a caller path/nonce/authority conversion.
pub(super) trait RegistryDriver {
    type Key;
    /// Freshly validate the actual lease and complete retained native context.
    fn verify_context(&mut self) -> DriverResult<ContextFacts>;
    /// Native provider reads canonical bytes; presence never becomes bool clean.
    fn journal_is_absent(&mut self) -> DriverResult<bool>;
    fn open_component(
        &mut self,
        parent: Option<&Self::Key>,
        component: Component,
        view: View,
    ) -> DriverResult<Option<Self::Key>>;
    fn create_component(
        &mut self,
        parent: &Self::Key,
        component: Component,
        volatile: bool,
        view: View,
    ) -> DriverResult<(Self::Key, Disposition)>;
    fn validate_security(&mut self, key: &Self::Key, scope: Scope) -> DriverResult<()>;
    /// Reopen the fixed Native64 chain and check each retained component link.
    /// When provided, compare the complete expected capsule in that namespace.
    /// This does not replace explicit per-component security replay: SOFTWARE
    /// alone is Ancestor; Application, Tuning and Marker are always Managed.
    /// Begin replays security/namespace/count/context after its sole fresh Set;
    /// observe replays those checks and capsule reads before and after Probe.
    fn validate_namespace(
        &mut self,
        keys: &[(Component, Self::Key)],
        view: View,
        expected: Option<&CapsuleShape>,
    ) -> DriverResult<bool>;
    fn marker_counts(&mut self, key: &Self::Key) -> DriverResult<Counts>;
    fn capsule_header(&mut self, key: &Self::Key) -> DriverResult<Option<ValueHeader>>;
    /// A fixed stack buffer and repeated type/length, with no size-driven growth.
    fn capsule_data(&mut self, key: &Self::Key, buffer: &mut [u8; 56])
    -> DriverResult<ValueHeader>;
    fn generate_capsule(&mut self) -> DriverResult<[u8; 56]>;
    fn set_new_capsule(&mut self, key: &Self::Key, capsule: &[u8; 56]) -> DriverResult<()>;
    /// Closes only its returned child handle; unexpected artifacts are retained.
    fn volatility_probe(&mut self, key: &Self::Key, view: View) -> DriverResult<u32>;
}

pub(super) struct DriverSession<K> {
    keys: Vec<(Component, K)>,
    expected: CapsuleShape,
    unknown_latched: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DriverObservation {
    ObservedShape(CapsuleShape),
    Unknown,
}

pub(super) fn begin<D: RegistryDriver>(
    driver: &mut D,
    mode: Mode,
) -> DriverResult<DriverSession<D::Key>> {
    verify_context(driver)?;
    let journal_absent = driver.journal_is_absent()?;
    if mode == Mode::InitializeOnlyWhenCanonicalJournalAbsent && !journal_absent {
        return Err(DriverFailure::Unknown(UnknownReason::Initialization));
    }

    let mut keys = Vec::with_capacity(4);
    let mut generated = None;
    let mut marker_disposition = Disposition::Existing;
    for component in [
        Component::Software,
        Component::Application,
        Component::Tuning,
        Component::Marker,
    ] {
        let parent = keys.last().map(|(_, key)| key);
        let (key, disposition) = match driver.open_component(parent, component, View::Native64)? {
            Some(key) => (key, Disposition::Existing),
            None => {
                let action = control::missing_action(mode, component, journal_absent)
                    .map_err(DriverFailure::Unknown)?;
                if component == Component::Marker {
                    let bytes = driver.generate_capsule()?;
                    // Random generation is not authority; reject malformed shape
                    // before creating a marker that could retain incomplete data.
                    let shape = capsule::decode(capsule::BINARY_TYPE, &bytes)
                        .map_err(DriverFailure::Unknown)?;
                    generated = Some((bytes, shape));
                }
                let parent =
                    parent.ok_or(DriverFailure::Unknown(UnknownReason::MissingComponent))?;
                driver.create_component(
                    parent,
                    component,
                    action == control::MissingAction::CreateSingleVolatile,
                    View::Native64,
                )?
            }
        };
        keys.push((component, key));
        driver.validate_security(
            &keys.last().expect("just retained component").1,
            scope(component),
        )?;
        namespace(driver, &keys, None)?;
        if component == Component::Marker {
            marker_disposition = disposition;
        }
    }

    let marker = &keys.last().expect("complete fixed chain").1;
    let expected = if marker_disposition == Disposition::New {
        let counts = driver.marker_counts(marker)?;
        let may_set = control::may_set_new_capsule(
            mode,
            journal_absent,
            control::MarkerFacts {
                disposition: marker_disposition,
                security_verified: true,
                namespace_verified: true,
                values: counts.values,
                subkeys: counts.subkeys,
            },
        )
        .map_err(DriverFailure::Unknown)?;
        if !may_set {
            return Err(DriverFailure::Unknown(UnknownReason::Initialization));
        }
        let (bytes, shape) =
            generated.ok_or(DriverFailure::Unknown(UnknownReason::Initialization))?;
        verify_context(driver)?;
        // Only this fresh-disposition branch can ever issue a value write.
        driver.set_new_capsule(marker, &bytes)?;
        shape
    } else {
        exact_counts(driver, marker)?;
        read_capsule(driver, marker)?
    };

    // Requested creation security is not accepted as actual security. Replay
    // every retained component and exact readback before admitting a session.
    validate_observation(driver, &keys, &expected)?;
    Ok(DriverSession {
        keys,
        expected,
        unknown_latched: false,
    })
}
impl<K> DriverSession<K> {
    pub fn observe<D: RegistryDriver<Key = K>>(
        &mut self,
        driver: &mut D,
    ) -> DriverResult<DriverObservation> {
        if self.unknown_latched {
            return Ok(DriverObservation::Unknown);
        }
        let result = self.observe_once(driver);
        match result {
            Ok(observation @ DriverObservation::ObservedShape(_)) => Ok(observation),
            Ok(DriverObservation::Unknown) | Err(DriverFailure::Unknown(_)) => {
                self.unknown_latched = true;
                Ok(DriverObservation::Unknown)
            }
            Err(error @ DriverFailure::Native(_)) => {
                self.unknown_latched = true;
                Err(error)
            }
        }
    }

    fn observe_once<D: RegistryDriver<Key = K>>(
        &self,
        driver: &mut D,
    ) -> DriverResult<DriverObservation> {
        validate_observation(driver, &self.keys, &self.expected)?;
        let marker = &self.keys.last().expect("admitted fixed chain").1;
        let code = driver.volatility_probe(marker, View::Native64)?;
        if control::probe_result(code) != control::Volatility::Positive {
            return Ok(DriverObservation::Unknown);
        }
        validate_observation(driver, &self.keys, &self.expected)?;
        Ok(DriverObservation::ObservedShape(self.expected.clone()))
    }
}

fn scope(component: Component) -> Scope {
    if component == Component::Software {
        Scope::Ancestor
    } else {
        Scope::Managed
    }
}
fn verify_context<D: RegistryDriver>(driver: &mut D) -> DriverResult<()> {
    control::admit_context(driver.verify_context()?).map_err(DriverFailure::Unknown)
}
fn namespace<D: RegistryDriver>(
    driver: &mut D,
    keys: &[(Component, D::Key)],
    expected: Option<&CapsuleShape>,
) -> DriverResult<()> {
    if driver.validate_namespace(keys, View::Native64, expected)? {
        Ok(())
    } else {
        Err(DriverFailure::Unknown(UnknownReason::MissingComponent))
    }
}
fn exact_counts<D: RegistryDriver>(driver: &mut D, marker: &D::Key) -> DriverResult<()> {
    let counts = driver.marker_counts(marker)?;
    if counts
        == (Counts {
            values: 1,
            subkeys: 0,
        })
    {
        Ok(())
    } else {
        Err(DriverFailure::Unknown(UnknownReason::MalformedCapsule))
    }
}
fn read_capsule<D: RegistryDriver>(driver: &mut D, marker: &D::Key) -> DriverResult<CapsuleShape> {
    let header = driver
        .capsule_header(marker)?
        .ok_or(DriverFailure::Unknown(UnknownReason::MalformedCapsule))?;
    capsule::fixed_read_size(header.kind, header.size).map_err(DriverFailure::Unknown)?;
    let mut bytes = [0; capsule::CAPSULE_BYTES];
    let repeated = driver.capsule_data(marker, &mut bytes)?;
    capsule::fixed_read_size(repeated.kind, repeated.size).map_err(DriverFailure::Unknown)?;
    if repeated != header {
        return Err(DriverFailure::Unknown(UnknownReason::QueryBound));
    }
    capsule::decode(repeated.kind, &bytes).map_err(DriverFailure::Unknown)
}
fn validate_observation<D: RegistryDriver>(
    driver: &mut D,
    keys: &[(Component, D::Key)],
    expected: &CapsuleShape,
) -> DriverResult<()> {
    verify_context(driver)?;
    for (component, key) in keys {
        driver.validate_security(key, scope(*component))?;
    }
    namespace(driver, keys, Some(expected))?;
    let marker = &keys.last().expect("complete retained chain").1;
    exact_counts(driver, marker)?;
    if read_capsule(driver, marker)? != *expected {
        return Err(DriverFailure::Unknown(UnknownReason::MalformedCapsule));
    }
    verify_context(driver)
}
