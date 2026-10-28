//! Three bounded process mechanisms; callers retain durable baseline/attempt evidence.
use super::{Resolver, native, unsupported};
use crate::background::TargetProof;
use crate::{background::PriorityDecision, background::priority_change, gaming::*};
use std::mem::size_of;
use windows::Win32::{Foundation::HANDLE, System::Threading::*};

fn priority(handle: HANDLE) -> Result<u32, GamingError> {
    let value = unsafe { GetPriorityClass(handle) };
    if value == 0 {
        Err(native(
            "GetPriorityClass background",
            windows::core::Error::from_thread(),
        ))
    } else {
        Ok(value)
    }
}

pub(super) fn power(handle: HANDLE) -> Result<PowerThrottling, GamingError> {
    let mut value = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ..Default::default()
    };
    unsafe {
        GetProcessInformation(
            handle,
            ProcessPowerThrottling,
            (&mut value as *mut PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .map_err(|e| native("GetProcessInformation background", e))?;
    if value.Version != 1 {
        return Err(unsupported("Unknown power policy version"));
    }
    Ok(PowerThrottling {
        version: value.Version,
        control_mask: value.ControlMask,
        state_mask: value.StateMask,
    })
}
fn owned(value: PowerThrottling) -> OwnedPolicy {
    OwnedPolicy {
        control: value.control_mask & EXECUTION_SPEED,
        state: value.state_mask & EXECUTION_SPEED,
    }
}

// Production reaches these mechanism helpers only through with_admitted. The
// opt-in test harness uses its own child handle without spoofing a Windows role.
pub(super) fn write_priority_on_handle(
    handle: HANDLE,
    expected: u32,
    desired: u32,
) -> Result<(), GamingError> {
    let current = priority(handle)?;
    if current != expected {
        return Err(GamingError::Changed);
    }
    if desired == 0x4000 && priority_change(current) != PriorityDecision::LowerToBelowNormal {
        return Err(GamingError::InvalidValue);
    }
    unsafe { SetPriorityClass(handle, PROCESS_CREATION_FLAGS(desired)) }
        .map_err(|e| native("SetPriorityClass background", e))?;
    if priority(handle)? != desired {
        return Err(GamingError::Changed);
    }
    Ok(())
}

pub(super) fn write_qos_on_handle(
    handle: HANDLE,
    expected: OwnedPolicy,
    desired: OwnedPolicy,
) -> Result<(), GamingError> {
    let current = power(handle)?;
    if owned(current) != expected {
        return Err(GamingError::Changed);
    }
    let merged = merge_policy(current, EXECUTION_SPEED, desired)?;
    let value = PROCESS_POWER_THROTTLING_STATE {
        Version: merged.version,
        ControlMask: merged.control_mask,
        StateMask: merged.state_mask,
    };
    unsafe {
        SetProcessInformation(
            handle,
            ProcessPowerThrottling,
            (&value as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .map_err(|e| native("SetProcessInformation background", e))?;
    let after = power(handle)?;
    if owned(after) != desired
        || after.control_mask & !EXECUTION_SPEED != current.control_mask & !EXECUTION_SPEED
        || after.state_mask & !EXECUTION_SPEED != current.state_mask & !EXECUTION_SPEED
    {
        return Err(GamingError::Changed);
    }
    Ok(())
}

impl Resolver {
    pub(crate) fn write_priority(
        &mut self,
        process: &ProcessIdentity,
        expected: u32,
        desired: u32,
    ) -> Result<(), GamingError> {
        self.priority_write(process, expected, desired, None)
    }
    pub(crate) fn write_priority_bound(
        &mut self,
        process: &ProcessIdentity,
        expected: u32,
        desired: u32,
        proof: &TargetProof,
    ) -> Result<(), GamingError> {
        self.priority_write(process, expected, desired, Some(proof))
    }
    fn priority_write(
        &mut self,
        process: &ProcessIdentity,
        expected: u32,
        desired: u32,
        proof: Option<&TargetProof>,
    ) -> Result<(), GamingError> {
        // The first version can only lower Normal or restore its exact Normal baseline.
        if !matches!((expected, desired), (0x20, 0x4000) | (0x4000, 0x20)) {
            return Err(GamingError::InvalidValue);
        }
        self.with_admitted(process, proof, |handle, role| {
            if !role.permits_priority() {
                return Err(unsupported("Spawner priority inheritance is not qualified"));
            }
            write_priority_on_handle(handle, expected, desired)
        })
    }

    pub(crate) fn write_qos(
        &mut self,
        process: &ProcessIdentity,
        expected: OwnedPolicy,
        desired: OwnedPolicy,
    ) -> Result<(), GamingError> {
        self.qos_write(process, expected, desired, None)
    }
    pub(crate) fn write_qos_bound(
        &mut self,
        process: &ProcessIdentity,
        expected: OwnedPolicy,
        desired: OwnedPolicy,
        proof: &TargetProof,
    ) -> Result<(), GamingError> {
        self.qos_write(process, expected, desired, Some(proof))
    }
    fn qos_write(
        &mut self,
        process: &ProcessIdentity,
        expected: OwnedPolicy,
        desired: OwnedPolicy,
        proof: Option<&TargetProof>,
    ) -> Result<(), GamingError> {
        self.with_admitted(process, proof, |handle, _| {
            write_qos_on_handle(handle, expected, desired)
        })
    }
}
