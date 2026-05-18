pub fn detect_cpu_model(test_override: &str) -> String {
    #[allow(clippy::const_is_empty)]
    if !test_override.is_empty() {
        test_override.to_string()
    } else {
        crate::app::adapters::os::get_cpu_model()
    }
}

pub fn detect_total_threads(test_override: usize) -> usize {
    #[allow(clippy::absurd_extreme_comparisons)]
    if test_override > 0 {
        test_override
    } else {
        num_cpus::get()
    }
}
