//! Ephemeral real-store driver; no fixture constructor or public writer.
use super::{
    UnknownReason, capsule,
    driver::{
        CapsuleShape, Component, ContextFacts, Counts, Disposition, DriverFailure, DriverResult,
        RegistryDriver, Scope, ValueHeader, View,
    },
    native_context::NativeContext,
    native_key::{NativeKey, refuse_symbolic_link_header, set_new_marker_once},
    native_plumbing::component_plan,
    primitives::{self, PrivateProbeName},
    registry_descriptor::ManagedDescriptor,
    registry_policy, security_descriptor,
};
use crate::tuning::TuningStore;
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};

pub(super) struct NativeDriver<'a> {
    pub store: &'a mut TuningStore,
    pub context: &'a NativeContext,
}
fn key_scope(component: Component) -> Scope {
    if component == Component::Software {
        Scope::Ancestor
    } else {
        Scope::Managed
    }
}
fn check_security(key: &NativeKey, scope: Scope) -> DriverResult<()> {
    if scope != key_scope(key.component()) {
        return Err(DriverFailure::Unknown(UnknownReason::Context));
    }
    refuse_symbolic_link_header(key.key())?;
    let bytes = primitives::security_descriptor_bytes(key.key())?;
    let facts = security_descriptor::parse_descriptor(bytes.bytes(), scope)
        .map_err(DriverFailure::Unknown)?;
    registry_policy::validate(&facts, scope).map_err(DriverFailure::Unknown)
}
impl RegistryDriver for NativeDriver<'_> {
    type Key = NativeKey;
    fn verify_context(&mut self) -> DriverResult<ContextFacts> {
        self.context.verify(self.store)?;
        // Facts are emitted only after this actual verification, never retained
        // as authority or supplied by a caller to construct the context.
        Ok(ContextFacts {
            native64_build: true,
            canonical_lease: true,
            elevated: true,
            high_or_system_integrity: true,
            exact_machine_and_token: true,
        })
    }
    fn journal_is_absent(&mut self) -> DriverResult<bool> {
        Ok(self.store.read().map_err(DriverFailure::Native)?.is_none())
    }
    fn open_component(
        &mut self,
        parent: Option<&NativeKey>,
        component: Component,
        view: View,
    ) -> DriverResult<Option<NativeKey>> {
        NativeKey::open(parent, component, view)
    }
    fn create_component(
        &mut self,
        parent: &NativeKey,
        component: Component,
        volatile: bool,
        view: View,
    ) -> DriverResult<(NativeKey, Disposition)> {
        // Reject routes/volatility before any SDK call, even local conversion.
        let plan = component_plan(Some(parent.component()), component, view)?;
        plan.validate_creation(volatile)?;
        check_security(parent, key_scope(parent.component()))?;
        let descriptor = ManagedDescriptor::new()?;
        self.context.verify(self.store)?;
        let key = NativeKey::create(parent, component, volatile, view, &descriptor)?;
        check_security(&key, plan.scope())?;
        let disposition = key.disposition();
        Ok((key, disposition))
    }
    fn validate_security(&mut self, key: &NativeKey, scope: Scope) -> DriverResult<()> {
        check_security(key, scope)
    }
    fn validate_namespace(
        &mut self,
        keys: &[(Component, NativeKey)],
        view: View,
        expected: Option<&CapsuleShape>,
    ) -> DriverResult<bool> {
        if keys.is_empty() || keys.len() > 4 {
            return Ok(false);
        }
        let fixed = [
            Component::Software,
            Component::Application,
            Component::Tuning,
            Component::Marker,
        ];
        let mut current = Vec::with_capacity(4);
        for (index, (component, retained)) in keys.iter().enumerate() {
            if *component != fixed[index] || retained.component() != *component {
                return Ok(false);
            }
            let parent = current.last();
            let plan = component_plan(parent.map(NativeKey::component), *component, view)?;
            // Validate both the retained key and its newly reopened current
            // namespace, with actual link + Scope security on every handle.
            check_security(retained, plan.scope())?;
            let Some(reopened) = NativeKey::open(parent, *component, view)? else {
                return Ok(false);
            };
            check_security(&reopened, plan.scope())?;
            current.push(reopened);
        }
        if let Some(expected) = expected {
            if current.len() != 4 {
                return Ok(false);
            }
            let marker = current.last().expect("complete current fixed chain");
            if primitives::marker_counts(marker.key())?
                != (Counts {
                    values: 1,
                    subkeys: 0,
                })
            {
                return Ok(false);
            }
            if primitives::read_capsule(marker.key())? != *expected {
                return Ok(false);
            }
        }
        // No undocumented key-object IDs or textual-path identity inference.
        Ok(true)
    }
    fn marker_counts(&mut self, key: &NativeKey) -> DriverResult<Counts> {
        require_marker(key)?;
        primitives::marker_counts(key.key())
    }
    fn capsule_header(&mut self, key: &NativeKey) -> DriverResult<Option<ValueHeader>> {
        require_marker(key)?;
        primitives::capsule_header(key.key())
    }
    fn capsule_data(
        &mut self,
        key: &NativeKey,
        buffer: &mut [u8; 56],
    ) -> DriverResult<ValueHeader> {
        require_marker(key)?;
        primitives::capsule_data(key.key(), buffer)
    }
    fn generate_capsule(&mut self) -> DriverResult<[u8; 56]> {
        let mut nonce = [0; 32];
        unsafe { BCryptGenRandom(None, &mut nonce, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
            .ok()
            .map_err(|error| {
                DriverFailure::Native(super::super::native_error("canonical capsule nonce", error))
            })?;
        capsule::encode(nonce).map_err(DriverFailure::Unknown)
    }
    fn set_new_capsule(&mut self, key: &NativeKey, bytes: &[u8; 56]) -> DriverResult<()> {
        require_marker(key)?;
        self.context.verify(self.store)?;
        set_new_marker_once(key, bytes)
    }
    fn volatility_probe(&mut self, key: &NativeKey, view: View) -> DriverResult<u32> {
        require_marker(key)?;
        let name = PrivateProbeName::random()?;
        self.context.verify(self.store)?;
        Ok(primitives::probe_nonvolatile_child_once(key.key(), view, &name)?.native_code)
    }
}
fn require_marker(key: &NativeKey) -> DriverResult<()> {
    if key.component() == Component::Marker {
        Ok(())
    } else {
        Err(DriverFailure::Unknown(UnknownReason::Context))
    }
}
