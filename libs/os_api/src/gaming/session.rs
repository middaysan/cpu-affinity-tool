//! Pure identity and lifetime decisions; native WTS reads live in Windows.
use super::{GamingError, InteractiveSession};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionObservation {
    Present(InteractiveSession),
    Absent,
}

pub(crate) fn expired(
    saved: Option<InteractiveSession>,
    before: SessionObservation,
    after: SessionObservation,
) -> Result<bool, GamingError> {
    if saved.is_some_and(|epoch| !epoch.valid()) {
        return Err(unknown("Invalid saved interactive session identity"));
    }
    if before != after {
        return Err(unknown("Interactive session changed during verification"));
    }
    match before {
        SessionObservation::Absent => Ok(true),
        SessionObservation::Present(actual) if actual.valid() => match saved {
            Some(expected) if !expected.valid() || expected.session_id != actual.session_id => {
                Err(unknown("Invalid interactive session identity"))
            }
            Some(expected) => Ok(expected.logon_time != actual.logon_time),
            // A legacy LUID supplies its terminal number, but not the old WTS epoch.
            None => Ok(false),
        },
        _ => Err(unknown("Interactive session identity is unavailable")),
    }
}

pub(crate) fn unknown(reason: &'static str) -> GamingError {
    GamingError::Session { reason, code: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionObservation::{Absent, Present};

    fn epoch(time: i64) -> InteractiveSession {
        InteractiveSession {
            session_id: 7,
            logon_time: time,
        }
    }

    #[test]
    fn retained_authentication_record_does_not_keep_missing_desktop_alive() {
        assert_eq!(expired(None, Absent, Absent), Ok(true));
        assert_eq!(expired(Some(epoch(10)), Absent, Absent), Ok(true));
    }

    #[test]
    fn locked_or_disconnected_existing_epoch_is_not_expired() {
        // Connection/lock state deliberately is not an instance identity.
        assert_eq!(
            expired(Some(epoch(10)), Present(epoch(10)), Present(epoch(10))),
            Ok(false)
        );
        assert_eq!(
            expired(None, Present(epoch(10)), Present(epoch(10))),
            Ok(false)
        );
    }

    #[test]
    fn reused_session_number_retires_only_a_recorded_old_epoch() {
        assert_eq!(
            expired(Some(epoch(10)), Present(epoch(20)), Present(epoch(20))),
            Ok(true)
        );
        assert_eq!(
            expired(None, Present(epoch(20)), Present(epoch(20))),
            Ok(false)
        );
    }

    #[test]
    fn races_and_invalid_metadata_never_prove_expiry() {
        assert!(expired(None, Absent, Present(epoch(20))).is_err());
        assert!(expired(Some(epoch(10)), Present(epoch(10)), Present(epoch(20))).is_err());
        assert!(expired(None, Present(epoch(0)), Present(epoch(0))).is_err());
        assert!(expired(Some(epoch(0)), Absent, Absent).is_err());
        assert!(expired(Some(epoch(0)), Present(epoch(20)), Present(epoch(20))).is_err());
    }
}
