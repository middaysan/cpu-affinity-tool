use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

use super::OS;

impl OS {
    pub fn get_cpu_model() -> String {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        hklm.open_subkey(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
            .and_then(|key| {
                let value: String = key.get_value("ProcessorNameString")?;
                Ok(value
                    .trim_matches(|c: char| c.is_whitespace() || c == '\0')
                    .to_string())
            })
            .unwrap_or_else(|_| "Unknown CPU".to_string())
    }
}
