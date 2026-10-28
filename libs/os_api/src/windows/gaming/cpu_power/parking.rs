//! Finite read-only parking discovery on the tuning worker. Uses existing guarded writes.
use super::*;
use crate::core_parking::{ParkingSnapshot, ParkingValue, classes_from_topology};

pub fn inspect(user: &UserIdentity) -> Result<ParkingSnapshot, GamingError> {
    with_account(user, || {
        let scheme = active_scheme()?;
        let overlay = super::super::read_power_mode(user, PowerSource::Ac)?;
        if ![
            crate::gaming::POWER_MODE_BALANCED,
            crate::gaming::POWER_MODE_EFFICIENCY,
            POWER_MODE_PERFORMANCE,
        ]
        .contains(&overlay)
            || overlay == scheme
        {
            return Err(GamingError::Unsupported(
                "The AC power-mode binding could not be verified",
            ));
        }
        let classes = classes_from_topology(&crate::background::placement_topology(None)?)?;
        let settings = crate::core_parking::settings_for_scope(
            &classes,
            crate::core_parking::ParkingScope::All,
        )?;
        let mut values = vec![];
        for scheme in [scheme, overlay]
            .into_iter()
            .filter(|s| *s != GuidValue([0; 16]))
        {
            open_scheme(scheme)?;
            for setting in &settings {
                if !metadata_available(*setting)? {
                    continue;
                }
                let key = CpuPowerKey {
                    scheme,
                    setting: *setting,
                };
                read_override(key)?;
                match effective_value(key) {
                    Ok(effective) => values.push(ParkingValue { key, effective }),
                    Err(error) if optional_unavailable(&error) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        if active_scheme()? != scheme
            || super::super::read_power_mode(user, PowerSource::Ac)? != overlay
        {
            return Err(GamingError::Changed);
        }
        Ok(ParkingSnapshot {
            scheme,
            overlay,
            classes,
            values,
        })
    })
}
