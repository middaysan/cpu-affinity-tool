//! Private production-used plans and requested descriptor shape, never authority.
use super::{
    UnknownReason,
    driver::{Component, DriverFailure, Scope, View},
    native_plumbing,
    native_token::{Integrity, RetainedCurrentToken},
    native_token_tests::actual_integrity_attributes_supported,
    registry_descriptor::ManagedDescriptor,
    registry_policy::{self, SidClass},
    security_descriptor,
};
use std::mem::size_of;
use windows::Win32::Security::SECURITY_ATTRIBUTES;

const READ_NATIVE64: u32 = 0x0002_0119;
const READ_SET_NATIVE64: u32 = READ_NATIVE64 | 2;

#[test]
fn plumbing_fixed_parent_chain_names_scopes_view_and_masks_are_exact() {
    for (parent, component, name, scope, create_mask, volatile) in [
        (
            None,
            Component::Software,
            "SOFTWARE",
            Scope::Ancestor,
            None,
            None,
        ),
        (
            Some(Component::Software),
            Component::Application,
            "CpuAffinityTool",
            Scope::Managed,
            Some(READ_NATIVE64),
            Some(false),
        ),
        (
            Some(Component::Application),
            Component::Tuning,
            "Tuning",
            Scope::Managed,
            Some(READ_NATIVE64),
            Some(false),
        ),
        (
            Some(Component::Tuning),
            Component::Marker,
            "ContinuityV1",
            Scope::Managed,
            Some(READ_SET_NATIVE64),
            Some(true),
        ),
    ] {
        let plan = native_plumbing::component_plan(parent, component, View::Native64).unwrap();
        assert_eq!(plan.component(), component);
        assert_eq!(plan.name(), name);
        assert!(!plan.name().contains(['\\', '/', '\0']));
        // SAFETY: a plan exposes only one fixed, static, terminated component.
        assert_eq!(unsafe { plan.wide().to_string() }.unwrap(), name);
        assert_eq!(plan.scope(), scope);
        assert_eq!(plan.view(), View::Native64);
        assert_eq!(plan.view().flag(), 0x0100);
        assert_eq!(plan.open_mask(), READ_NATIVE64);
        assert_eq!(plan.create_mask(), create_mask);
        assert_eq!(plan.volatile(), volatile);
    }
}

#[test]
fn plumbing_root_skipped_reversed_and_marker_parent_routes_refuse_before_sdk() {
    let components = [
        Component::Software,
        Component::Application,
        Component::Tuning,
        Component::Marker,
    ];
    for parent in [
        None,
        Some(Component::Software),
        Some(Component::Application),
        Some(Component::Tuning),
        Some(Component::Marker),
    ] {
        for component in components {
            let valid = matches!(
                (parent, component),
                (None, Component::Software)
                    | (Some(Component::Software), Component::Application)
                    | (Some(Component::Application), Component::Tuning)
                    | (Some(Component::Tuning), Component::Marker)
            );
            if !valid {
                assert!(matches!(
                    native_plumbing::component_plan(parent, component, View::Native64),
                    Err(DriverFailure::Unknown(UnknownReason::Context))
                ));
            }
        }
    }
}

#[test]
fn plumbing_software_creation_and_wrong_volatility_refuse_before_sdk() {
    let software =
        native_plumbing::component_plan(None, Component::Software, View::Native64).unwrap();
    for volatile in [false, true] {
        assert_eq!(
            software.validate_creation(volatile),
            Err(DriverFailure::Unknown(UnknownReason::Context))
        );
    }
    for (parent, component, volatile, expected_mask) in [
        (
            Component::Software,
            Component::Application,
            false,
            READ_NATIVE64,
        ),
        (
            Component::Application,
            Component::Tuning,
            false,
            READ_NATIVE64,
        ),
        (
            Component::Tuning,
            Component::Marker,
            true,
            READ_SET_NATIVE64,
        ),
    ] {
        let plan =
            native_plumbing::component_plan(Some(parent), component, View::Native64).unwrap();
        assert_eq!(plan.validate_creation(volatile).unwrap(), expected_mask);
        assert_eq!(
            plan.validate_creation(!volatile),
            Err(DriverFailure::Unknown(UnknownReason::Context))
        );
    }
}

#[test]
fn plumbing_fixed_sdk_descriptor_is_bounded_strict_managed_requested_shape_only() {
    let requested = ManagedDescriptor::new().unwrap();
    let bytes = requested.bytes();
    assert!((20..=65_536).contains(&bytes.len()));
    assert_eq!(
        registry_policy::descriptor_buffer_size(bytes.len() as u32).unwrap(),
        bytes.len()
    );
    let facts = security_descriptor::parse_descriptor(bytes, Scope::Managed).unwrap();
    registry_policy::validate(&facts, Scope::Managed).unwrap();
    assert_eq!(facts.owner, SidClass::Administrators);
    assert!(facts.dacl_present && facts.dacl_protected);
    assert_eq!(facts.dacl.len(), 2);
    for ace in &facts.dacl {
        assert_eq!(ace.kind, 0);
        assert_eq!(ace.flags, 0);
        assert_eq!(ace.mask, registry_policy::KEY_ALL);
    }
    assert_eq!(
        facts
            .dacl
            .iter()
            .filter(|ace| ace.sid == SidClass::System)
            .count(),
        1
    );
    assert_eq!(
        facts
            .dacl
            .iter()
            .filter(|ace| ace.sid == SidClass::Administrators)
            .count(),
        1
    );
    assert_eq!(facts.labels.len(), 1);
    let label = &facts.labels[0];
    assert_eq!(label.kind, 17);
    assert_eq!(label.flags, 0);
    assert_eq!(label.mask, 1);
    assert_eq!(label.sid, SidClass::HighIntegrity);
    let attributes = requested.attributes();
    assert_eq!(
        attributes.nLength as usize,
        size_of::<SECURITY_ATTRIBUTES>()
    );
    assert!(!attributes.bInheritHandle.as_bool());
    assert!(!attributes.lpSecurityDescriptor.is_null());
    assert!(
        attributes.lpSecurityDescriptor.cast_const().cast::<u8>() == bytes.as_ptr(),
        "requested descriptor ownership differs"
    );
    assert_eq!(
        format!("{requested:?}"),
        "ManagedDescriptor { requested_shape: redacted }"
    );
    // This is a converter allocation, never applied to a key or native context.
}

#[test]
fn plumbing_token_policy_accepts_only_elevated_known_high_or_system_shapes() {
    native_plumbing::token_policy(true, Integrity::High).unwrap();
    native_plumbing::token_policy(true, Integrity::System).unwrap();
    for integrity in [
        Integrity::Low,
        Integrity::Medium,
        Integrity::High,
        Integrity::System,
        Integrity::Unknown,
    ] {
        assert_eq!(
            native_plumbing::token_policy(false, integrity),
            Err(DriverFailure::Unknown(UnknownReason::Context))
        );
    }
    for integrity in [Integrity::Low, Integrity::Medium, Integrity::Unknown] {
        assert_eq!(
            native_plumbing::token_policy(true, integrity),
            Err(DriverFailure::Unknown(UnknownReason::Context))
        );
    }
}

#[test]
fn plumbing_actual_retained_token_admission_is_readonly_before_hardware() {
    let supported = actual_integrity_attributes_supported();
    println!(
        "actual_plumbing_token_attributes={}",
        if supported {
            "SupportedAttributes"
        } else {
            "UnknownAttributes"
        }
    );
    let token = RetainedCurrentToken::open_current().unwrap();
    if !supported {
        assert!(matches!(
            token.snapshot(),
            Err(DriverFailure::Unknown(UnknownReason::Context))
        ));
        return;
    }
    let snapshot = token.snapshot().unwrap();
    let expected =
        snapshot.elevated() && matches!(snapshot.integrity(), Integrity::High | Integrity::System);
    let decision = native_plumbing::admit_retained_token(&snapshot);
    if expected {
        decision.unwrap();
    } else {
        assert_eq!(
            decision,
            Err(DriverFailure::Unknown(UnknownReason::Context))
        );
    }
    println!(
        "actual_retained_token_policy={}/{:?}/{}",
        if snapshot.elevated() {
            "elevated"
        } else {
            "standard"
        },
        snapshot.integrity(),
        if expected {
            "AllowedShape"
        } else {
            "DeniedShape"
        }
    );
    // The predicate produces only a decision; no machine or native context is opened.
}
