//! Platform composition and per-tweak compatibility, separate from the owned-field controller.
mod linux;
mod windows;

pub(crate) use os_api::platform::OsPlatform;
use std::sync::Arc;

use super::{platform::registry::Registry, tweaks::scheduling::SchedulingHandoff};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TweakAvailability {
    Available,
    Unavailable(&'static str),
}

impl TweakAvailability {
    pub(crate) const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlatformSupport {
    pub(crate) windows10_min_build: Option<u32>,
    pub(crate) windows11_min_build: Option<u32>,
}

pub(crate) const WIN10_11: PlatformSupport = PlatformSupport {
    windows10_min_build: Some(10240),
    windows11_min_build: Some(22000),
};
pub(crate) const GAME_MODE: PlatformSupport = PlatformSupport {
    windows10_min_build: Some(15063),
    windows11_min_build: Some(22000),
};
pub(crate) const GPU_PREFERENCE: PlatformSupport = PlatformSupport {
    windows10_min_build: Some(17134),
    windows11_min_build: Some(22000),
};
pub(crate) const WINDOWS11: PlatformSupport = PlatformSupport {
    windows10_min_build: None,
    windows11_min_build: Some(22000),
};
pub(crate) const WINDOWED_GAMES: PlatformSupport = PlatformSupport {
    windows10_min_build: None,
    windows11_min_build: Some(22621),
};

impl PlatformSupport {
    pub(crate) const fn for_platform(self, platform: OsPlatform) -> TweakAvailability {
        use TweakAvailability::{Available, Unavailable};
        match platform {
            OsPlatform::Windows {
                major: 10,
                minor: 0,
                build,
                workstation: true,
            } if build >= 22000 => match self.windows11_min_build {
                Some(minimum) if build >= minimum => Available,
                Some(22621) => Unavailable("Windows 11 22H2 required"),
                Some(_) => Unavailable("A newer Windows 11 version is required"),
                None => Unavailable("Windows 10 required"),
            },
            OsPlatform::Windows {
                major: 10,
                minor: 0,
                build,
                workstation: true,
            } => match self.windows10_min_build {
                Some(minimum) if build >= minimum => Available,
                Some(_) => Unavailable("A newer Windows 10 version is required"),
                None => Unavailable("Windows 11 required"),
            },
            OsPlatform::Windows {
                workstation: false, ..
            } => Unavailable("Windows desktop required"),
            OsPlatform::Windows { .. } => Unavailable("Windows 10 or 11 required"),
            OsPlatform::Linux => Unavailable("Not available on Linux"),
            OsPlatform::Other => Unavailable("Not available on this platform"),
            OsPlatform::Unknown => Unavailable("Windows version could not be verified"),
        }
    }
}

pub(crate) fn registry() -> Registry {
    registry_with_scheduling(Arc::new(SchedulingHandoff::default()))
}

pub(crate) fn registry_with_scheduling(scheduling: Arc<SchedulingHandoff>) -> Registry {
    if super::MODULES_ENABLED {
        windows::registry_with_scheduling(scheduling)
    } else {
        linux::registry()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(build: u32) -> OsPlatform {
        OsPlatform::Windows {
            major: 10,
            minor: 0,
            build,
            workstation: true,
        }
    }

    #[test]
    fn windows_10_and_11_support_their_declared_tweaks_only() {
        assert!(WIN10_11.for_platform(windows(19045)).is_available());
        assert!(!WINDOWS11.for_platform(windows(19045)).is_available());
        assert!(!WINDOWED_GAMES.for_platform(windows(19045)).is_available());
        assert!(WINDOWS11.for_platform(windows(22000)).is_available());
        assert!(!WINDOWED_GAMES.for_platform(windows(22000)).is_available());
        assert!(WINDOWED_GAMES.for_platform(windows(22621)).is_available());
        let windows_10_only = PlatformSupport {
            windows10_min_build: Some(10240),
            windows11_min_build: None,
        };
        assert!(windows_10_only.for_platform(windows(19045)).is_available());
        assert!(!windows_10_only.for_platform(windows(22621)).is_available());
    }

    #[test]
    fn minimum_builds_are_checked_without_rejecting_newer_builds() {
        for (support, minimum) in [
            (WIN10_11, 10240),
            (GAME_MODE, 15063),
            (GPU_PREFERENCE, 17134),
        ] {
            assert!(!support.for_platform(windows(minimum - 1)).is_available());
            assert!(support.for_platform(windows(minimum)).is_available());
        }
        assert!(WINDOWED_GAMES.for_platform(windows(29999)).is_available());
    }

    #[test]
    fn linux_unknown_server_and_unrecognized_versions_are_unavailable() {
        for platform in [
            OsPlatform::Linux,
            OsPlatform::Other,
            OsPlatform::Unknown,
            OsPlatform::Windows {
                major: 10,
                minor: 0,
                build: 22621,
                workstation: false,
            },
            OsPlatform::Windows {
                major: 6,
                minor: 3,
                build: 9600,
                workstation: true,
            },
            OsPlatform::Windows {
                major: 11,
                minor: 0,
                build: 30000,
                workstation: true,
            },
        ] {
            assert!(!WIN10_11.for_platform(platform).is_available());
        }
    }

    #[test]
    fn unavailable_states_explain_the_platform_requirement() {
        assert_eq!(
            WINDOWS11.for_platform(windows(19045)),
            TweakAvailability::Unavailable("Windows 11 required")
        );
        assert_eq!(
            WINDOWED_GAMES.for_platform(windows(22000)),
            TweakAvailability::Unavailable("Windows 11 22H2 required")
        );
        assert_eq!(
            WIN10_11.for_platform(OsPlatform::Unknown),
            TweakAvailability::Unavailable("Windows version could not be verified")
        );
    }

    #[test]
    fn reserved_linux_composition_has_no_native_modules() {
        assert!(linux::registry().descriptors().next().is_none());
    }
}
