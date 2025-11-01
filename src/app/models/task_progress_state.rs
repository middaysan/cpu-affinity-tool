
/// Represents the state when an operation/task is executing.
pub struct TaskProgressState {
    /// The name of the task/operation in progress
    pub task_name: String,
    /// The operation is in progress
    pub processing: bool,
    // Progress (for progress bar)
    pub progress: f32,
}

impl TaskProgressState {
    /// Reset all group form fields to their default values.
    pub fn reset(&mut self) {
        self.task_name = String::new();
        self.processing = false;
    }
}
