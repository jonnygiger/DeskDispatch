use deskdispatch::domain::TaskRunStatus;
use proptest::prelude::*;

/// Pure helper for step position compaction logic (mirroring DB compaction)
fn compact_step_positions(positions: &[f64]) -> Vec<f64> {
    let mut sorted_indices: Vec<usize> = (0..positions.len()).collect();
    sorted_indices.sort_by(|&a, &b| {
        positions[a]
            .partial_cmp(&positions[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut compacted = vec![0.0; positions.len()];
    for (new_rank, &orig_idx) in sorted_indices.iter().enumerate() {
        compacted[orig_idx] = (new_rank as f64 + 1.0) * 10.0;
    }
    compacted
}

/// Helper to check if any gap between adjacent ordered step positions is less than 0.0001
fn needs_position_compaction(ordered_positions: &[f64]) -> bool {
    for i in 0..ordered_positions.len().saturating_sub(1) {
        if (ordered_positions[i + 1] - ordered_positions[i]).abs() < 0.0001 {
            return true;
        }
    }
    false
}

/// Helper function defining valid task run state machine transitions
fn is_valid_state_transition(from: TaskRunStatus, to: TaskRunStatus) -> bool {
    match (from, to) {
        // Queued can transition to Running or Cancelled
        (TaskRunStatus::Queued, TaskRunStatus::Running) => true,
        (TaskRunStatus::Queued, TaskRunStatus::Cancelled) => true,

        // Running can transition to Cancelling, Cancelled, Succeeded, Failed, Lost
        (TaskRunStatus::Running, TaskRunStatus::Cancelling) => true,
        (TaskRunStatus::Running, TaskRunStatus::Cancelled) => true,
        (TaskRunStatus::Running, TaskRunStatus::Succeeded) => true,
        (TaskRunStatus::Running, TaskRunStatus::Failed) => true,
        (TaskRunStatus::Running, TaskRunStatus::Lost) => true,

        // Cancelling can transition to Cancelled, Failed, Lost
        (TaskRunStatus::Cancelling, TaskRunStatus::Cancelled) => true,
        (TaskRunStatus::Cancelling, TaskRunStatus::Failed) => true,
        (TaskRunStatus::Cancelling, TaskRunStatus::Lost) => true,

        // Self-transitions (idempotent updates)
        (a, b) if a == b => true,

        // All other transitions, including any transition out of terminal states, are invalid
        _ => false,
    }
}

/// Helper checking if a state is terminal
fn is_terminal_status(status: TaskRunStatus) -> bool {
    matches!(
        status,
        TaskRunStatus::Succeeded
            | TaskRunStatus::Failed
            | TaskRunStatus::Cancelled
            | TaskRunStatus::Lost
    )
}

proptest! {
    // 1. Step position sorting property: sorting produces monotonically non-decreasing order
    #[test]
    fn prop_step_positions_sorting_monotonic(mut positions in prop::collection::vec(0.1f64..10000.0f64, 1..50)) {
        positions.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for i in 0..positions.len().saturating_sub(1) {
            prop_assert!(positions[i] <= positions[i + 1]);
        }
    }

    // 2. Step position compaction property: after compaction, positions are 10.0, 20.0, 30.0...
    #[test]
    fn prop_step_position_compaction_properties(positions in prop::collection::vec(0.0001f64..5000.0f64, 1..50)) {
        let compacted = compact_step_positions(&positions);
        prop_assert_eq!(compacted.len(), positions.len());

        let mut sorted_compacted = compacted.clone();
        sorted_compacted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        for (i, &pos) in sorted_compacted.iter().enumerate() {
            let expected = (i as f64 + 1.0) * 10.0;
            prop_assert!((pos - expected).abs() < 1e-6);
        }

        // Verify minimum adjacent gap in sorted compacted positions is exactly 10.0
        for i in 0..sorted_compacted.len().saturating_sub(1) {
            let gap = sorted_compacted[i + 1] - sorted_compacted[i];
            prop_assert!((gap - 10.0).abs() < 1e-6);
        }

        // Compacted list never needs further compaction
        prop_assert!(!needs_position_compaction(&sorted_compacted));
    }

    // 3. Step position swap property: swapping two adjacent step positions preserves overall set and order bounds
    #[test]
    fn prop_step_position_swap_invariants(
        positions in prop::collection::vec(10.0f64..100.0f64, 2..20),
        swap_idx in 0..19usize
    ) {
        if swap_idx >= positions.len() - 1 {
            return Ok(());
        }

        let mut swapped = positions.clone();
        swapped.swap(swap_idx, swap_idx + 1);

        // Position set size preserved
        prop_assert_eq!(swapped.len(), positions.len());
        // Elements outside swap index remain untouched
        for i in 0..positions.len() {
            if i != swap_idx && i != swap_idx + 1 {
                prop_assert_eq!(swapped[i], positions[i]);
            }
        }
    }

    // 4. Task run state machine terminal state invariant: terminal states cannot transition to non-terminal or different terminal states
    #[test]
    fn prop_terminal_states_cannot_transition(
        terminal in prop::sample::select(vec![
            TaskRunStatus::Succeeded,
            TaskRunStatus::Failed,
            TaskRunStatus::Cancelled,
            TaskRunStatus::Lost,
        ]),
        target in prop::sample::select(vec![
            TaskRunStatus::Queued,
            TaskRunStatus::Running,
            TaskRunStatus::Cancelling,
            TaskRunStatus::Cancelled,
            TaskRunStatus::Succeeded,
            TaskRunStatus::Failed,
            TaskRunStatus::Lost,
        ])
    ) {
        prop_assert!(is_terminal_status(terminal));
        if terminal == target {
            prop_assert!(is_valid_state_transition(terminal, target));
        } else {
            prop_assert!(!is_valid_state_transition(terminal, target));
        }
    }

    // 5. Task run state machine transition validity property
    #[test]
    fn prop_task_run_state_transitions(
        from in prop::sample::select(vec![
            TaskRunStatus::Queued,
            TaskRunStatus::Running,
            TaskRunStatus::Cancelling,
            TaskRunStatus::Cancelled,
            TaskRunStatus::Succeeded,
            TaskRunStatus::Failed,
            TaskRunStatus::Lost,
        ]),
        to in prop::sample::select(vec![
            TaskRunStatus::Queued,
            TaskRunStatus::Running,
            TaskRunStatus::Cancelling,
            TaskRunStatus::Cancelled,
            TaskRunStatus::Succeeded,
            TaskRunStatus::Failed,
            TaskRunStatus::Lost,
        ])
    ) {
        let is_valid = is_valid_state_transition(from, to);

        if is_terminal_status(from) {
            prop_assert_eq!(is_valid, from == to);
        }

        if from == TaskRunStatus::Queued {
            let allowed = to == TaskRunStatus::Queued || to == TaskRunStatus::Running || to == TaskRunStatus::Cancelled;
            prop_assert_eq!(is_valid, allowed);
        }

        if from == TaskRunStatus::Cancelling {
            let allowed = to == TaskRunStatus::Cancelling || to == TaskRunStatus::Cancelled || to == TaskRunStatus::Failed || to == TaskRunStatus::Lost;
            prop_assert_eq!(is_valid, allowed);
        }
    }
}
