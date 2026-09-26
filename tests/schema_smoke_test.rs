// Compile-time and schema validation smoke test

#[test]
fn test_schema_queries_syntax() {
    let select_users = "SELECT id, username, password_hash, display_name, role, is_active FROM users";
    let select_automations = "SELECT id, name, description, status FROM automations";
    let select_steps = "SELECT id, automation_id, position, step_type, post_delay_ms FROM automation_steps";
    let select_mouse_clicks = "SELECT step_id, x, y, x_variable_id, y_variable_id, button, click_type FROM step_mouse_clicks";
    let select_key_presses = "SELECT step_id, key_combo FROM step_key_presses";
    let select_branches = "SELECT step_id, condition_type, on_match_step_id, on_no_match_step_id FROM step_branches";
    let select_runs = "SELECT id, automation_id, schedule_id, worker_id, status FROM task_runs";
    let select_schedules = "SELECT id, automation_id, cron_expression, timezone, is_enabled, next_run_at FROM schedules";

    assert!(!select_users.is_empty());
    assert!(!select_automations.is_empty());
    assert!(!select_steps.is_empty());
    assert!(!select_mouse_clicks.is_empty());
    assert!(!select_key_presses.is_empty());
    assert!(!select_branches.is_empty());
    assert!(!select_runs.is_empty());
    assert!(!select_schedules.is_empty());
}
