use std::ffi::{ OsString};
use std::process;
use std::sync::Once;
use sysinfo::{ System,
};

use single_instance::SingleInstance;
static mut SINGLE_INSTANCE_VAL : Option<SingleInstance> = None;
static SINGLE_INSTANCE_VAL_LOCK: Once = Once::new();

static EXECUTABLE_NAME: &str = "cpu-affinity-tool.exe";

impl crate::OS {

    pub fn is_already_running() -> bool {
        //Validate if there is an instance of the application running.
        let instance_exists =  crate::OS::ensure_single_instance("sd");

        if (!instance_exists)  {
            let mut process_name = EXECUTABLE_NAME;
            if cfg!(target_os = "linux") {
                println!("Running on macOS!");
                process_name = EXECUTABLE_NAME.trim_end_matches(".exe")
            }

            let proc = crate::OS::find_process_by_name(process_name.parse().unwrap());
            if let Some(pid) = proc {
                // println!("My pid is {}", pid);
                let focused =  crate::OS::focus_window_by_pid(pid);
                if focused {
                    println!("Windows focused");
                }
            }
            true
        } else {
            false
        }
    }

     fn ensure_single_instance(uniq_id: &str) -> bool {
        let instance =  SingleInstance::new(&uniq_id);
        match  instance{
            Ok(inst) => {
                let single = inst.is_single();
                if single {
                    unsafe {
                        SINGLE_INSTANCE_VAL_LOCK.call_once(|| {
                            SINGLE_INSTANCE_VAL = Some(inst);
                        })
                    }
                }
                single
            },
            Err(e) => {
                false
            }
        }
    }

    pub fn  find_process_by_name( name: String) -> Option<u32> {
        let s = System::new_all();
        let os_string: OsString = name.into();

        // let ssr= s.processes_by_name(os_string.as_ref());

        for process in s.processes_by_name(os_string.as_ref()) {
            let pname = process.name().to_str().unwrap();

            println!("{} {} ", process.pid(), pname);
            if pname == os_string {
                return Some(process.pid().as_u32())
            }
        }

        None
    }


    pub fn  find_process_name_by_id(process_id:  u32) -> Option<String> {
        let s = System::new_all();
        for process in s.processes() {

            let (p_id, p_process) = process;

            if p_id.as_u32() == process_id {
                return Some(p_process.name().to_str().unwrap().to_string());
            }
        }

        None
    }
}

