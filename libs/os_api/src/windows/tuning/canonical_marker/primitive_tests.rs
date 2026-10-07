//! Native returned-size decisions use no OS calls or authority construction.
use super::{UnknownReason, driver::DriverFailure, primitives::descriptor_returned_size};

#[test]
fn native_descriptor_changed_returned_size_is_unknown_without_growth_or_retry() {
    assert_eq!(descriptor_returned_size(20, 20), Ok(20));
    assert_eq!(descriptor_returned_size(65536, 65536), Ok(65536));
    let unknown = Err(DriverFailure::Unknown(UnknownReason::QueryBound));
    for (capacity, returned) in [
        (20, 19),
        (20, 0),
        (0, 0),
        (24, 28),
        (21, 24),
        (65536, 65537),
        (65537, 65537),
        (24, 20),
        (128, 64),
        (65536, 65532),
    ] {
        assert_eq!(
            descriptor_returned_size(capacity, returned),
            unknown,
            "second query changed size from {capacity} to {returned}"
        );
    }
}
