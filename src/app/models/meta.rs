pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Constants for testing different CPU configurations.
/// If TEST_CPU_MODEL is not empty, it will be used instead of the actual system CPU model.
pub const TEST_CPU_MODEL: &str = "";
/// If TEST_TOTAL_THREADS is greater than 0, it will be used instead of the actual system thread count.
pub const TEST_TOTAL_THREADS: usize = 0;
