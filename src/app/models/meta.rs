pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Constants for testing different CPU configurations.
/// If TEST_CPU_MODEL is not empty, it will be used instead of the actual system CPU model.
pub const TEST_CPU_MODEL: &str = "";
/// If TEST_TOTAL_THREADS is greater than 0, it will be used instead of the actual system thread count.
pub const TEST_TOTAL_THREADS: usize = 0;

/// Returns the current CPU model, honoring test overrides when present.
pub fn effective_cpu_model() -> String {
    crate::app::features::topology::detect_cpu_model(TEST_CPU_MODEL)
}

/// Returns the current logical thread count, honoring test overrides when present.
pub fn effective_total_threads() -> usize {
    crate::app::features::topology::detect_total_threads(TEST_TOTAL_THREADS)
}
