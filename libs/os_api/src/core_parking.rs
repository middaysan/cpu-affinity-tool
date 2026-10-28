//! Topology-qualified Windows parking policy scopes, separate from frequency controls.
use crate::{
    background::PlacementCpu,
    gaming::{GamingError, GuidValue, UserIdentity},
    gaming_cpu_power::{CpuPowerKey, CpuPowerRequest, CpuPowerSetting},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParkingScope {
    #[default]
    All,
    Performance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParkingClass {
    pub id: u8,
    pub physical_cores: u32,
    pub logical_processors: u32,
}

/// A read-only option snapshot is display data, never native mutation authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParkingSnapshot {
    pub scheme: GuidValue,
    pub overlay: GuidValue,
    pub classes: Vec<ParkingClass>,
    pub values: Vec<ParkingValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParkingValue {
    pub key: CpuPowerKey,
    pub effective: u32,
}

pub fn classes_from_topology(cpus: &[PlacementCpu]) -> Result<Vec<ParkingClass>, GamingError> {
    let invalid = || GamingError::Unsupported("CPU parking class mapping could not be verified");
    if cpus.is_empty() || cpus.len() > crate::background::MAX_CPU_SETS {
        return Err(invalid());
    }
    let mut ids = BTreeSet::new();
    let mut logicals = BTreeSet::new();
    let mut physical = BTreeMap::new();
    let mut classes: BTreeMap<u8, (BTreeSet<(u16, u32)>, u32)> = BTreeMap::new();
    for cpu in cpus {
        if cpu.efficiency_class > 1
            || cpu.logical >= 64
            || !ids.insert(cpu.id)
            || !logicals.insert((cpu.group, cpu.logical))
            || physical
                .insert((cpu.group, cpu.physical_core), cpu.efficiency_class)
                .is_some_and(|old| old != cpu.efficiency_class)
        {
            return Err(invalid());
        }
        let class = classes.entry(cpu.efficiency_class).or_default();
        class.0.insert((cpu.group, cpu.physical_core));
        class.1 += 1;
    }
    if !classes.contains_key(&0) {
        return Err(invalid());
    }
    Ok(classes
        .into_iter()
        .map(|(id, (cores, logical_processors))| ParkingClass {
            id,
            physical_cores: cores.len() as u32,
            logical_processors,
        })
        .collect())
}

pub fn settings_for_scope(
    classes: &[ParkingClass],
    scope: ParkingScope,
) -> Result<Vec<CpuPowerSetting>, GamingError> {
    if classes.iter().any(|c| {
        c.physical_cores == 0
            || c.logical_processors < c.physical_cores
            || c.logical_processors > crate::background::MAX_CPU_SETS as u32
    }) {
        return Err(GamingError::InvalidValue);
    }
    let hybrid =
        matches!(classes, [a,b] if a.id==0 && b.id==1 && a.physical_cores>0 && b.physical_cores>0);
    if !hybrid && !matches!(classes,[a] if a.id==0 && a.physical_cores>0) {
        return Err(GamingError::InvalidValue);
    }
    use CpuPowerSetting::*;
    match scope {
        ParkingScope::Performance if hybrid => Ok(vec![ParkingMinimum1, ParkingMaximum1]),
        ParkingScope::Performance => Err(GamingError::Unsupported(
            "A separate performance core class is unavailable",
        )),
        ParkingScope::All if hybrid => Ok(vec![
            ParkingMinimum,
            ParkingMaximum,
            ParkingMinimum1,
            ParkingMaximum1,
        ]),
        ParkingScope::All => Ok(vec![ParkingMinimum, ParkingMaximum]),
    }
}

impl ParkingSnapshot {
    pub fn requests(&self, scope: ParkingScope) -> Result<Vec<CpuPowerRequest>, GamingError> {
        let settings = settings_for_scope(&self.classes, scope)?;
        let allowed = settings_for_scope(&self.classes, ParkingScope::All)?;
        if self.scheme == GuidValue([0; 16])
            || self.overlay == self.scheme
            || ![
                crate::gaming::POWER_MODE_BALANCED,
                crate::gaming::POWER_MODE_EFFICIENCY,
                crate::gaming::POWER_MODE_PERFORMANCE,
            ]
            .contains(&self.overlay)
            || self.values.is_empty()
            || self.values.len() > 8
        {
            return Err(GamingError::InvalidValue);
        }
        let mut seen = BTreeSet::new();
        for value in &self.values {
            if value.effective > 100
                || !allowed.contains(&value.key.setting)
                || (value.key.scheme != self.scheme
                    && (self.overlay == GuidValue([0; 16]) || value.key.scheme != self.overlay))
                || !seen.insert(value.key.wire_id())
            {
                return Err(GamingError::InvalidValue);
            }
        }
        let mut requests = vec![];
        for scheme in [self.scheme, self.overlay]
            .into_iter()
            .filter(|s| *s != GuidValue([0; 16]))
        {
            for setting in &settings {
                let key = CpuPowerKey {
                    scheme,
                    setting: *setting,
                };
                let value = self.values.iter().find(|value| value.key == key).ok_or(
                    GamingError::Unsupported("Parking policy for this scope is unavailable"),
                )?;
                if value.effective != 100 {
                    requests.push(CpuPowerRequest { key, value: 100 });
                }
            }
        }
        Ok(requests)
    }
}

pub fn inspect(user: &UserIdentity) -> Result<ParkingSnapshot, GamingError> {
    #[cfg(windows)]
    {
        crate::windows::gaming::cpu_power::parking::inspect(user)
    }
    #[cfg(not(windows))]
    {
        let _ = user;
        Err(GamingError::Unsupported("Core parking requires Windows"))
    }
}

#[cfg(test)]
mod tests;
