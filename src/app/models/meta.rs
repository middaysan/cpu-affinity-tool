pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Constants for testing different CPU configurations.
/// If TEST_CPU_MODEL is not empty, it will be used instead of the actual system CPU model.
pub const TEST_CPU_MODEL: &str = "";
/// If TEST_TOTAL_THREADS is greater than 0, it will be used instead of the actual system thread count.
pub const TEST_TOTAL_THREADS: usize = 0;

/// Returns the current CPU model, honoring test overrides when present.
pub fn effective_cpu_model() -> String {
    #[allow(clippy::const_is_empty)]
    if !TEST_CPU_MODEL.is_empty() {
        TEST_CPU_MODEL.to_string()
    } else {
        os_api::OS::get_cpu_model()
    }
}

/// Returns the current logical thread count, honoring test overrides when present.
pub fn effective_total_threads() -> usize {
    #[allow(clippy::absurd_extreme_comparisons)]
    if TEST_TOTAL_THREADS > 0 {
        TEST_TOTAL_THREADS
    } else {
        num_cpus::get()
    }
}
