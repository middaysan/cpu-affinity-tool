use super::OS;
use std::sync::OnceLock;
use windows::Win32::System::Threading::{
    GetActiveProcessorCount, GetActiveProcessorGroupCount, GetCurrentProcess,
    GetProcessAffinityMask, SetProcessAffinityMask,
};

// Preserve pre-startup restrictions for children whose affinity is not owned by
// a saved rule. A self-only restriction must not become their launch policy.
static ORIGINAL_MASK: OnceLock<usize> = OnceLock::new();

fn last_available_cpu(process: usize, system: usize) -> Result<usize, String> {
    let available = process & system;
    if available == 0 {
        return Err("Current process has no available logical CPU".into());
    }
    Ok((usize::BITS - 1 - available.leading_zeros()) as usize)
}

pub(super) fn original_mask() -> Option<usize> {
    ORIGINAL_MASK.get().copied()
}

pub(super) fn launch_mask(
    managed: bool,
    requested: usize,
    original: Option<usize>,
) -> Option<usize> {
    if managed { Some(requested) } else { original }
}

fn current_masks() -> Result<(usize, usize), String> {
    let mut process = 0;
    let mut system = 0;
    unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) }
        .map_err(|error| format!("Get own process affinity failed: {error}"))?;
    Ok((process, system))
}

impl OS {
    /// Startup-only hard affinity for this process, including its future workers.
    /// Classic process masks cannot safely choose a global CPU across groups.
    pub fn pin_current_process_to_last_cpu() -> Result<usize, String> {
        if unsafe { GetActiveProcessorGroupCount() } != 1 {
            return Err(
                "Self affinity requires one Windows processor group; no affinity was changed"
                    .into(),
            );
        }
        if unsafe { GetActiveProcessorCount(0) } > usize::BITS {
            return Err("Self affinity cannot represent this processor group in a native-width mask; no affinity was changed".into());
        }
        let (process, system) = current_masks()?;
        let cpu = last_available_cpu(process, system)?;
        let desired = 1usize << cpu;
        ORIGINAL_MASK.get_or_init(|| process);
        unsafe { SetProcessAffinityMask(GetCurrentProcess(), desired) }
            .map_err(|error| format!("Set own process affinity failed: {error}"))?;
        let verification = current_masks().and_then(|(actual, _)| {
            if actual == desired {
                Ok(())
            } else {
                Err("Own process affinity readback differs from the selected CPU".into())
            }
        });
        if let Err(error) = verification {
            let restore = unsafe { SetProcessAffinityMask(GetCurrentProcess(), process) };
            return Err(match restore {
                Ok(()) => error,
                Err(restore) => {
                    format!("{error}; could not restore own original affinity: {restore}")
                }
            });
        }
        Ok(cpu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::sync::{Arc, Barrier};
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, GetCurrentProcessorNumber, GetExitCodeProcess, OpenProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    #[test]
    fn self_affinity_is_verified_in_an_isolated_process() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "windows::self_affinity::tests::native_self_affinity_probe",
                "--nocapture",
            ])
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "self-affinity child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    #[ignore = "isolated own-process scheduling probe; invoked by its parent unit test"]
    fn native_self_affinity_probe() {
        if unsafe { GetActiveProcessorGroupCount() } != 1 {
            assert!(OS::pin_current_process_to_last_cpu().is_err());
            return;
        }
        let (original, system) = current_masks().unwrap();
        let cpu = last_available_cpu(original, system).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = barrier.clone();
        let existing = std::thread::spawn(move || {
            worker_barrier.wait();
            for _ in 0..32 {
                assert_eq!(unsafe { GetCurrentProcessorNumber() } as usize, cpu);
                std::thread::yield_now();
            }
        });
        OS::set_current_process_priority(crate::PriorityClass::BelowNormal).unwrap();
        assert_eq!(OS::pin_current_process_to_last_cpu().unwrap(), cpu);
        let desired = 1usize << cpu;
        assert_eq!(current_masks().unwrap().0, desired);
        assert_eq!(
            OS::get_process_priority(unsafe { GetCurrentProcessId() }).unwrap(),
            crate::PriorityClass::BelowNormal
        );
        barrier.wait();
        existing.join().unwrap();
        std::thread::spawn(move || {
            assert_eq!(unsafe { GetCurrentProcessorNumber() } as usize, cpu)
        })
        .join()
        .unwrap();
        assert_eq!(original_mask(), Some(original));
        assert_eq!(OS::pin_current_process_to_last_cpu().unwrap(), cpu);
        assert_eq!(
            original_mask(),
            Some(original),
            "idempotent pin must not lose the mask for child launches"
        );

        for (fields, cores, expected) in [
            (
                crate::gaming::ProcessSettingsFields::default(),
                vec![original.trailing_zeros() as usize],
                original & original.wrapping_neg(),
            ),
            (
                crate::gaming::ProcessSettingsFields {
                    affinity: false,
                    priority: false,
                },
                vec![],
                original,
            ),
        ] {
            // This test process is isolated; no concurrent application reader
            // sees the numeric probe value, inherited only by its own children.
            unsafe {
                std::env::set_var("CPU_AFFINITY_SELF_PROBE_MASK", expected.to_string());
            }
            let child = OS::run_with_fields(
                std::env::current_exe().unwrap(),
                vec![
                    "--ignored".into(),
                    "--exact".into(),
                    "windows::self_affinity::tests::native_launch_affinity_probe".into(),
                ],
                &cores,
                crate::PriorityClass::Normal,
                fields,
            )
            .unwrap();
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    child.pid,
                )
            }
            .unwrap();
            let _guard = crate::windows::common::HandleGuard(handle);
            assert_eq!(unsafe { WaitForSingleObject(handle, 5000) }, WAIT_OBJECT_0);
            let mut exit = 1;
            unsafe { GetExitCodeProcess(handle, &mut exit) }.unwrap();
            assert_eq!(exit, 0, "saved-rule launch inherited the self-only mask");
        }
        assert_eq!(
            current_masks().unwrap().0,
            desired,
            "launch must not temporarily widen the utility's affinity"
        );
        eprintln!(
            "SELF_AFFINITY cpu={cpu} original=0x{original:X} actual=0x{desired:X} existing_worker=true new_worker=true managed_child=true neutral_child=true priority=BelowNormal"
        );
    }

    #[test]
    #[ignore = "finite helper child for own-process scheduling probe"]
    fn native_launch_affinity_probe() {
        // Keep this owned child alive long enough for the parent to pin a
        // process handle; it exits normally even if the parent test fails.
        std::thread::sleep(std::time::Duration::from_millis(500));
        let expected = std::env::var("CPU_AFFINITY_SELF_PROBE_MASK")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(current_masks().unwrap().0, expected);
    }

    #[test]
    fn last_logical_cpu_uses_highest_available_bit_not_physical_core_count() {
        for (mask, index) in [
            (1usize, 0),
            (0b1111, 3),
            (0xffff, 15),
            (0xfffff, 19),
            (0b101010, 5),
            (1usize << (usize::BITS - 1), (usize::BITS - 1) as usize),
        ] {
            assert_eq!(last_available_cpu(mask, mask).unwrap(), index);
        }
        assert_eq!(last_available_cpu(0b1010, 0b1111).unwrap(), 3);
        assert!(last_available_cpu(0, 0xffff).is_err());
        assert!(last_available_cpu(0b10000, 0b1111).is_err());
    }

    #[test]
    fn child_launch_uses_rule_mask_or_pre_self_pin_mask() {
        assert_eq!(launch_mask(true, 0b1111, Some(0xffff)), Some(0b1111));
        assert_eq!(launch_mask(false, 0, Some(0xffff)), Some(0xffff));
        assert_eq!(launch_mask(false, 0, None), None);
    }
}
