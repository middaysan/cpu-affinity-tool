use std::sync::Arc;
use tokio::sync::RwLock;
use std::path::PathBuf;

#[test]
fn test_index_shift_logic() {
    // This is a conceptual test to show why using indices is dangerous
    struct Group {
        name: String,
        cores: Vec<usize>,
    }

    struct RunningApp {
        group_index: usize,
    }

    let mut groups = vec![
        Group { name: "Group 0".into(), cores: vec![0] },
        Group { name: "Group 1".into(), cores: vec![1] },
    ];

    let app = RunningApp { group_index: 1 }; // Points to Group 1

    // Simulate removing Group 0
    groups.remove(0);

    // Now app.group_index 1 is OUT OF BOUNDS or points to wrong group if more groups were added.
    assert!(app.group_index >= groups.len());
}
