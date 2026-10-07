//! Fixed native routing and token policy, with no root/hardware/store calls.
use super::{
    UnknownReason,
    driver::{Component, DriverFailure, DriverResult, Scope, View},
    native_token::{Integrity, TokenSnapshot},
};
use windows::core::PCWSTR;

/// Private output of the fixed-parent planner, never a caller-selected path.
pub(super) struct FixedComponentPlan {
    component: Component,
    scope: Scope,
    #[cfg(test)]
    view: View,
    open_mask: u32,
    create_mask: Option<u32>,
    volatile: Option<bool>,
}
impl FixedComponentPlan {
    #[cfg(test)]
    pub fn component(&self) -> Component {
        self.component
    }
    #[cfg(test)]
    pub fn name(&self) -> &'static str {
        match self.component {
            Component::Software => "SOFTWARE",
            Component::Application => "CpuAffinityTool",
            Component::Tuning => "Tuning",
            Component::Marker => "ContinuityV1",
        }
    }
    pub fn wide(&self) -> PCWSTR {
        match self.component {
            Component::Software => windows::core::w!("SOFTWARE"),
            Component::Application => windows::core::w!("CpuAffinityTool"),
            Component::Tuning => windows::core::w!("Tuning"),
            Component::Marker => windows::core::w!("ContinuityV1"),
        }
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    #[cfg(test)]
    pub fn view(&self) -> View {
        self.view
    }
    pub fn open_mask(&self) -> u32 {
        self.open_mask
    }
    #[cfg(test)]
    pub fn create_mask(&self) -> Option<u32> {
        self.create_mask
    }
    #[cfg(test)]
    pub fn volatile(&self) -> Option<bool> {
        self.volatile
    }
    /// Production creation uses this before any SDK call; SOFTWARE and any
    /// volatility mismatch are refused, without a caller-selected access mask.
    pub fn validate_creation(&self, requested_volatile: bool) -> DriverResult<u32> {
        match (self.create_mask, self.volatile) {
            (Some(mask), Some(volatile))
                if self.component != Component::Software && volatile == requested_volatile =>
            {
                Ok(mask)
            }
            _ => Err(DriverFailure::Unknown(UnknownReason::Context)),
        }
    }
}

/// Production open/create must call this before its first SDK operation.
/// SOFTWARE is existing root/Ancestor only; each other parent is exact.
pub(super) fn component_plan(
    parent: Option<Component>,
    component: Component,
    view: View,
) -> DriverResult<FixedComponentPlan> {
    if !matches!(
        (parent, component),
        (None, Component::Software)
            | (Some(Component::Software), Component::Application)
            | (Some(Component::Application), Component::Tuning)
            | (Some(Component::Tuning), Component::Marker)
    ) {
        return Err(DriverFailure::Unknown(UnknownReason::Context));
    }
    let open_mask = super::registry_policy::KEY_READ | view.flag();
    let (scope, create_mask, volatile) = match component {
        Component::Software => (Scope::Ancestor, None, None),
        Component::Application | Component::Tuning => {
            (Scope::Managed, Some(open_mask), Some(false))
        }
        Component::Marker => (Scope::Managed, Some(open_mask | 2), Some(true)),
    };
    Ok(FixedComponentPlan {
        component,
        scope,
        #[cfg(test)]
        view,
        open_mask,
        create_mask,
        volatile,
    })
}

/// This helper reads only an actual retained native snapshot. Its shape result
/// cannot manufacture a token snapshot, NativeContext or public native guard.
pub(super) fn admit_retained_token(snapshot: &TokenSnapshot) -> DriverResult<()> {
    token_policy(snapshot.elevated(), snapshot.integrity())
}

/// Pure classification used by the retained-token admission, never cached as
/// authorization for later native calls. Valid known High/System only.
pub(super) fn token_policy(elevated: bool, integrity: Integrity) -> DriverResult<()> {
    if elevated && matches!(integrity, Integrity::High | Integrity::System) {
        Ok(())
    } else {
        Err(DriverFailure::Unknown(UnknownReason::Context))
    }
}
