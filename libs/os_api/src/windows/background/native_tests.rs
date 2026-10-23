//! Opt-in mechanism checks on a child created and terminated by this test only.
use super::{super::gaming, writes::*};
use crate::{gaming::*, windows::OS};
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn owned_thread(pid: u32) -> Result<crate::windows::common::HandleGuard, GamingError> {
    use windows::Win32::System::{Diagnostics::ToolHelp::*, Threading::*};
    let snapshot = crate::windows::common::HandleGuard(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
            .map_err(|error| super::native("owned thread snapshot", error))?,
    );
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    unsafe { Thread32First(snapshot.0, &mut entry) }
        .map_err(|error| super::native("owned thread first", error))?;
    for _ in 0..65536 {
        if entry.th32OwnerProcessID == pid {
            let handle = unsafe {
                OpenThread(
                    THREAD_QUERY_LIMITED_INFORMATION | THREAD_SET_INFORMATION,
                    false,
                    entry.th32ThreadID,
                )
            }
            .map_err(|error| super::native("owned thread open", error))?;
            return Ok(crate::windows::common::HandleGuard(handle));
        }
        if unsafe { Thread32Next(snapshot.0, &mut entry) }.is_err() {
            break;
        }
    }
    Err(GamingError::Changed)
}

#[test]
#[ignore = "private child entrypoint for native_owned_background_round_trip"]
fn native_owned_background_child() {
    assert_eq!(
        std::env::var("CPU_AFFINITY_BACKGROUND_NATIVE_CHILD").as_deref(),
        Ok("owned")
    );
    std::thread::sleep(Duration::from_secs(45));
}

#[test]
#[ignore = "explicit test-owned Windows process Priority/EcoQoS round trip"]
fn native_owned_background_round_trip() {
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Threading::CREATE_NO_WINDOW;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "windows::background::native_tests::native_owned_background_child",
            "--ignored",
        ])
        .env("CPU_AFFINITY_BACKGROUND_NATIVE_CHILD", "owned")
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = OwnedChild(command.spawn().unwrap());
    let identity = OS::tuning_process_identity(child.0.id()).unwrap();
    let process = ProcessIdentity {
        pid: identity.pid(),
        creation_token: identity.creation_time(),
        image_path: identity.image_path().into(),
    };
    let original_priority = gaming::read_priority(&process).unwrap();
    gaming::with_process(&process, true, |handle| {
        let topology = super::placement::topology_on_handle(Some(handle))?;
        let desired = crate::background::efficient_pool(&topology)
            .map_err(|_| GamingError::Unsupported("hybrid fixture required"))?;
        let original = super::placement::sets(handle)?;
        let witness = crate::background::PlacementWitness {
            topology,
            baseline: original.clone(),
            desired: desired.clone(),
        };
        use windows::Win32::System::Threading::*;
        let mut mask = 0usize;
        let mut system = 0usize;
        unsafe { GetProcessAffinityMask(handle, &mut mask, &mut system) }
            .map_err(|error| super::native("owned affinity baseline", error))?;
        let outside = witness
            .topology
            .iter()
            .find(|cpu| {
                cpu.group == 0 && !desired.contains(&cpu.id) && mask & (1usize << cpu.logical) != 0
            })
            .unwrap();
        unsafe { SetProcessAffinityMask(handle, 1usize << outside.logical) }
            .map_err(|error| super::native("owned affinity test", error))?;
        let refused = super::placement::write_sets_on_handle(
            handle,
            process.pid,
            &original,
            &desired,
            &witness,
        );
        unsafe { SetProcessAffinityMask(handle, mask) }
            .map_err(|error| super::native("owned affinity restore", error))?;
        assert!(
            refused.is_err(),
            "incompatible existing hard affinity must refuse placement"
        );
        assert_eq!(super::placement::sets(handle)?, original);
        let thread = owned_thread(process.pid)?;
        let mut thread_sets = 0;
        unsafe { GetThreadSelectedCpuSets(thread.0, None, &mut thread_sets).ok() }
            .map_err(|error| super::native("owned thread sets baseline", error))?;
        assert_eq!(thread_sets, 0);
        unsafe { SetThreadSelectedCpuSets(thread.0, &desired[..1]).ok() }
            .map_err(|error| super::native("owned thread override", error))?;
        let refused = super::placement::write_sets_on_handle(
            handle,
            process.pid,
            &original,
            &desired,
            &witness,
        );
        unsafe { SetThreadSelectedCpuSets(thread.0, &[]).ok() }
            .map_err(|error| super::native("owned thread override restore", error))?;
        assert!(
            refused.is_err(),
            "stronger thread-selected CPU Sets must refuse placement"
        );
        assert_eq!(super::placement::sets(handle)?, original);
        let mut changed = witness.clone();
        changed.topology[0].cache ^= 1;
        assert!(
            super::placement::write_sets_on_handle(
                handle,
                process.pid,
                &original,
                &desired,
                &changed
            )
            .is_err(),
            "changed full topology witness must refuse placement"
        );
        super::placement::write_sets_on_handle(handle, process.pid, &original, &desired, &witness)?;
        assert_eq!(gaming::read_cpu_sets(&process)?, desired);
        super::placement::write_sets_on_handle(handle, process.pid, &desired, &original, &witness)?;
        assert_eq!(gaming::read_cpu_sets(&process)?, original);
        assert!(
            original.is_empty(),
            "fixture must prove exact absence restoration"
        );
        Ok(())
    })
    .unwrap();
    eprintln!(
        "CPU Sets native affinity/thread-override/topology refusals, Apply/readback/exact empty-baseline Undo: PASS"
    );
    gaming::write_priority(&process, original_priority, 0x20).unwrap();
    gaming::with_process(&process, true, |handle| {
        write_priority_on_handle(handle, 0x20, 0x4000)?;
        assert_eq!(gaming::read_priority(&process)?, 0x4000);
        assert_eq!(
            write_priority_on_handle(handle, 0x20, 0x4000),
            Err(GamingError::Changed)
        );
        write_priority_on_handle(handle, 0x4000, 0x20)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(gaming::read_priority(&process).unwrap(), 0x20);
    eprintln!("Priority native Apply/readback/conditional refusal/Undo: PASS");

    match gaming::read_process_policy(&process, EXECUTION_SPEED) {
        Ok(original) => {
            for baseline in [
                OwnedPolicy {
                    control: 0,
                    state: 0,
                },
                OwnedPolicy {
                    control: 1,
                    state: 0,
                },
                OwnedPolicy {
                    control: 1,
                    state: 1,
                },
            ] {
                let current = gaming::read_process_policy(&process, EXECUTION_SPEED).unwrap();
                gaming::write_process_policy(&process, EXECUTION_SPEED, current, baseline).unwrap();
                gaming::with_process(&process, true, |handle| {
                    let before = power(handle)?;
                    let desired = OwnedPolicy {
                        control: 1,
                        state: 1,
                    };
                    write_qos_on_handle(handle, baseline, desired)?;
                    let active = power(handle)?;
                    assert_eq!(active.control_mask & 1, 1);
                    assert_eq!(active.state_mask & 1, 1);
                    assert_eq!(active.control_mask & !1, before.control_mask & !1);
                    assert_eq!(active.state_mask & !1, before.state_mask & !1);
                    write_qos_on_handle(handle, desired, baseline)?;
                    assert_eq!(power(handle)?, before);
                    Ok(())
                })
                .unwrap();
            }
            let current = gaming::read_process_policy(&process, EXECUTION_SPEED).unwrap();
            gaming::write_process_policy(&process, EXECUTION_SPEED, current, original).unwrap();
            assert_eq!(
                gaming::read_process_policy(&process, EXECUTION_SPEED).unwrap(),
                original
            );
            eprintln!("EcoQoS native SystemManaged/off/on, foreign bits and exact Undo: PASS");
        }
        Err(error) => eprintln!(
            "EcoQoS getter unavailable; mechanism NOT qualified and no QoS write attempted: {error:?}"
        ),
    }
    gaming::write_priority(&process, 0x20, original_priority).unwrap();
    assert_eq!(gaming::read_priority(&process).unwrap(), original_priority);
    let mut wrong = process.clone();
    wrong.creation_token ^= 1;
    assert_eq!(gaming::read_priority(&wrong), Err(GamingError::Changed));
    eprintln!(
        "Original child priority restored; PID-token mismatch refused. No service/system process changed."
    );
}
