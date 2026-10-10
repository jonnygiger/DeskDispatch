use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};

/// Result of linting an automation before activation or execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationLintError {
    pub step_id: Option<i64>,
    pub step_number: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for AutomationLintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(num) = self.step_number {
            write!(f, "Step {}: {}", num, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

/// Lints and validates an automation to ensure it is safe and ready for execution.
///
/// Checks performed:
/// 1. Automation must exist and have at least one active (non-deleted) step.
/// 2. Dangling or cross-automation branch targets:
///    Branch steps `on_match_step_id` and `on_no_match_step_id` must point to valid, non-deleted steps in the same automation.
/// 3. Cycles with no exit:
///    Control flow graph analysis ensuring every reachable path from Step 1 can eventually reach the end of the automation or a terminal step.
/// 4. Variables read before being written:
///    Ensures that any variable read by a step (e.g., mouse_click coords) is initialized by parameters or a preceding step along all reachable control flow paths.
/// 5. Type mismatches:
///    Ensures variable types match expected types (`int` for coords, `bool` for found flags, `color`/`point`/`string` for pixel values).
/// 6. Deleted or missing reference bitmaps:
///    Ensures `reference_bitmap_id` on `find_bitmap` and `branch` steps references an existing bitmap.
/// 7. Coordinate bounds:
///    Ensures literal coordinates are non-negative and within realistic screen bounds (<= 10,000 px).
pub async fn validate_automation_for_activation(
    db: &PgPool,
    automation_id: i64,
) -> Result<(), Vec<AutomationLintError>> {
    let mut errors = Vec::new();

    // 1. Fetch steps
    let raw_steps = match sqlx::query(
        "SELECT id, position, step_type, label FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            errors.push(AutomationLintError {
                step_id: None,
                step_number: None,
                message: format!("Database error fetching steps: {}", e),
            });
            return Err(errors);
        }
    };

    if raw_steps.is_empty() {
        errors.push(AutomationLintError {
            step_id: None,
            step_number: None,
            message: "Automation has no active steps.".to_string(),
        });
        return Err(errors);
    }

    struct StepMeta {
        step_number: usize,
        step_type: String,
    }

    let mut step_map: HashMap<i64, StepMeta> = HashMap::new();
    let mut step_id_order: Vec<i64> = Vec::new();

    for (idx, row) in raw_steps.iter().enumerate() {
        let sid: i64 = row.get("id");
        let stype: String = row.get("step_type");
        step_map.insert(
            sid,
            StepMeta {
                step_number: idx + 1,
                step_type: stype,
            },
        );
        step_id_order.push(sid);
    }

    // Fetch variables and parameters
    let var_rows =
        sqlx::query("SELECT id, name, var_type FROM automation_variables WHERE automation_id = $1")
            .bind(automation_id)
            .fetch_all(db)
            .await
            .unwrap_or_default();

    struct VarMeta {
        name: String,
        var_type: String,
    }
    let mut vars_map: HashMap<i64, VarMeta> = HashMap::new();
    for vr in var_rows {
        let vid: i64 = vr.get("id");
        vars_map.insert(
            vid,
            VarMeta {
                name: vr.get("name"),
                var_type: vr.get("var_type"),
            },
        );
    }

    let param_rows =
        sqlx::query("SELECT name, param_type FROM automation_parameters WHERE automation_id = $1")
            .bind(automation_id)
            .fetch_all(db)
            .await
            .unwrap_or_default();

    let mut available_params: HashSet<String> = HashSet::new();
    for pr in param_rows {
        let pname: String = pr.get("name");
        available_params.insert(pname);
    }

    // Fetch step details
    // Mouse clicks
    let mc_rows = sqlx::query(
        r#"
        SELECT smc.step_id, smc.x, smc.y, smc.x_variable_id, smc.y_variable_id
        FROM step_mouse_clicks smc
        JOIN automation_steps s ON smc.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    struct MouseClickMeta {
        x: Option<i32>,
        y: Option<i32>,
        x_var_id: Option<i64>,
        y_var_id: Option<i64>,
    }
    let mut mc_map: HashMap<i64, MouseClickMeta> = HashMap::new();
    for r in mc_rows {
        let sid: i64 = r.get("step_id");
        mc_map.insert(
            sid,
            MouseClickMeta {
                x: r.get("x"),
                y: r.get("y"),
                x_var_id: r.get("x_variable_id"),
                y_var_id: r.get("y_variable_id"),
            },
        );
    }

    // Find pixel
    let fp_rows = sqlx::query(
        r#"
        SELECT sfp.step_id, sfp.x, sfp.y, sfp.output_variable_id
        FROM step_find_pixel_rgb sfp
        JOIN automation_steps s ON sfp.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut fp_map: HashMap<i64, FindPixelMeta> = HashMap::new();
    for r in fp_rows {
        let sid: i64 = r.get("step_id");
        fp_map.insert(
            sid,
            FindPixelMeta {
                x: r.get("x"),
                y: r.get("y"),
                output_var_id: r.get("output_variable_id"),
            },
        );
    }

    // Find bitmap
    let fb_rows = sqlx::query(
        r#"
        SELECT sfb.step_id, sfb.reference_bitmap_id, sfb.output_found_variable_id, sfb.output_x_variable_id, sfb.output_y_variable_id, sfb.search_x, sfb.search_y
        FROM step_find_bitmap sfb
        JOIN automation_steps s ON sfb.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut fb_map: HashMap<i64, FindBitmapMeta> = HashMap::new();
    for r in fb_rows {
        let sid: i64 = r.get("step_id");
        fb_map.insert(
            sid,
            FindBitmapMeta {
                reference_bitmap_id: r.get("reference_bitmap_id"),
                output_found_var_id: r.get("output_found_variable_id"),
                output_x_var_id: r.get("output_x_variable_id"),
                output_y_var_id: r.get("output_y_variable_id"),
                search_x: r.get("search_x"),
                search_y: r.get("search_y"),
            },
        );
    }

    // Branches
    let branch_rows = sqlx::query(
        r#"
        SELECT sb.step_id, sb.condition_type, sb.on_match_step_id, sb.on_no_match_step_id,
               sb.reference_bitmap_id, sb.x, sb.y, sb.search_x, sb.search_y
        FROM step_branches sb
        JOIN automation_steps s ON sb.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    struct BranchMeta {
        condition_type: String,
        on_match_step_id: Option<i64>,
        on_no_match_step_id: Option<i64>,
        reference_bitmap_id: Option<i64>,
        x: Option<i32>,
        y: Option<i32>,
    }
    let mut branch_map: HashMap<i64, BranchMeta> = HashMap::new();
    for r in branch_rows {
        let sid: i64 = r.get("step_id");
        branch_map.insert(
            sid,
            BranchMeta {
                condition_type: r.get("condition_type"),
                on_match_step_id: r.get("on_match_step_id"),
                on_no_match_step_id: r.get("on_no_match_step_id"),
                reference_bitmap_id: r.get("reference_bitmap_id"),
                x: r.get("x"),
                y: r.get("y"),
            },
        );
    }

    // Fetch existing bitmap IDs
    let bitmap_rows = sqlx::query("SELECT id FROM bitmaps")
        .fetch_all(db)
        .await
        .unwrap_or_default();
    let existing_bitmaps: HashSet<i64> = bitmap_rows.into_iter().map(|r| r.get("id")).collect();

    // Check individual steps for lint rules (Targets, Bitmaps, Coordinates, Type Mismatches)
    for (idx, &sid) in step_id_order.iter().enumerate() {
        let meta = match step_map.get(&sid) {
            Some(m) => m,
            None => continue,
        };
        let step_num = idx + 1;

        match meta.step_type.as_str() {
            "mouse_click" => {
                if let Some(mc) = mc_map.get(&sid) {
                    if let Some(x) = mc.x {
                        if !(0..=10000).contains(&x) {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: format!("Mouse click X coordinate {} is outside valid bounds (0..10000).", x),
                            });
                        }
                    }
                    if let Some(y) = mc.y {
                        if !(0..=10000).contains(&y) {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: format!("Mouse click Y coordinate {} is outside valid bounds (0..10000).", y),
                            });
                        }
                    }

                    if let Some(x_vid) = mc.x_var_id {
                        if let Some(v) = vars_map.get(&x_vid) {
                            if v.var_type != "int" {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("Variable '{}' used for X coordinate is of type '{}', expected 'int'.", v.name, v.var_type),
                                });
                            }
                        } else {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: format!(
                                    "Referenced X coordinate variable ID {} does not exist.",
                                    x_vid
                                ),
                            });
                        }
                    }

                    if let Some(y_vid) = mc.y_var_id {
                        if let Some(v) = vars_map.get(&y_vid) {
                            if v.var_type != "int" {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("Variable '{}' used for Y coordinate is of type '{}', expected 'int'.", v.name, v.var_type),
                                });
                            }
                        } else {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: format!(
                                    "Referenced Y coordinate variable ID {} does not exist.",
                                    y_vid
                                ),
                            });
                        }
                    }
                }
            }
            "find_pixel_rgb" => {
                if let Some(fp) = fp_map.get(&sid) {
                    if !(0..=10000).contains(&fp.x) || !(0..=10000).contains(&fp.y) {
                        errors.push(AutomationLintError {
                            step_id: Some(sid),
                            step_number: Some(step_num),
                            message: format!(
                                "Pixel coordinates ({}, {}) are outside valid bounds (0..10000).",
                                fp.x, fp.y
                            ),
                        });
                    }

                    if let Some(out_vid) = fp.output_var_id {
                        if let Some(v) = vars_map.get(&out_vid) {
                            if !matches!(v.var_type.as_str(), "color" | "point" | "string" | "int")
                            {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("Output variable '{}' for pixel RGB is of type '{}', expected 'color' or 'string'.", v.name, v.var_type),
                                });
                            }
                        } else {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: format!("Output variable ID {} does not exist.", out_vid),
                            });
                        }
                    }
                }
            }
            "find_bitmap" => {
                if let Some(fb) = fb_map.get(&sid) {
                    if !existing_bitmaps.contains(&fb.reference_bitmap_id) {
                        errors.push(AutomationLintError {
                            step_id: Some(sid),
                            step_number: Some(step_num),
                            message: format!(
                                "References bitmap ID {} which does not exist or has been deleted.",
                                fb.reference_bitmap_id
                            ),
                        });
                    }

                    if let Some(x) = fb.search_x {
                        if !(0..=10000).contains(&x) {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: format!(
                                    "Search X coordinate {} is outside valid bounds.",
                                    x
                                ),
                            });
                        }
                    }
                    if let Some(y) = fb.search_y {
                        if !(0..=10000).contains(&y) {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: format!(
                                    "Search Y coordinate {} is outside valid bounds.",
                                    y
                                ),
                            });
                        }
                    }

                    if let Some(f_vid) = fb.output_found_var_id {
                        if let Some(v) = vars_map.get(&f_vid) {
                            if v.var_type != "bool" {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("'Found' output variable '{}' is of type '{}', expected 'bool'.", v.name, v.var_type),
                                });
                            }
                        }
                    }

                    if let Some(x_vid) = fb.output_x_var_id {
                        if let Some(v) = vars_map.get(&x_vid) {
                            if v.var_type != "int" {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("'Found X' output variable '{}' is of type '{}', expected 'int'.", v.name, v.var_type),
                                });
                            }
                        }
                    }

                    if let Some(y_vid) = fb.output_y_var_id {
                        if let Some(v) = vars_map.get(&y_vid) {
                            if v.var_type != "int" {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("'Found Y' output variable '{}' is of type '{}', expected 'int'.", v.name, v.var_type),
                                });
                            }
                        }
                    }
                }
            }
            "branch" => {
                if let Some(b) = branch_map.get(&sid) {
                    match b.on_match_step_id {
                        Some(target_id) => {
                            if !step_map.contains_key(&target_id) {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("Branch 'On Match' target step ID {} is invalid or deleted.", target_id),
                                });
                            }
                        }
                        None => {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: "Branch step is missing 'On Match' target step."
                                    .to_string(),
                            });
                        }
                    }

                    match b.on_no_match_step_id {
                        Some(target_id) => {
                            if !step_map.contains_key(&target_id) {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("Branch 'On No Match' target step ID {} is invalid or deleted.", target_id),
                                });
                            }
                        }
                        None => {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: "Branch step is missing 'On No Match' target step."
                                    .to_string(),
                            });
                        }
                    }

                    if b.condition_type == "bitmap" {
                        if let Some(bmp_id) = b.reference_bitmap_id {
                            if !existing_bitmaps.contains(&bmp_id) {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("Branch condition references bitmap ID {} which does not exist.", bmp_id),
                                });
                            }
                        } else {
                            errors.push(AutomationLintError {
                                step_id: Some(sid),
                                step_number: Some(step_num),
                                message: "Bitmap branch condition is missing reference bitmap."
                                    .to_string(),
                            });
                        }
                    } else if b.condition_type == "pixel_rgb" {
                        if let Some(x) = b.x {
                            if !(0..=10000).contains(&x) {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("Pixel branch condition X coordinate {} is outside valid bounds.", x),
                                });
                            }
                        }
                        if let Some(y) = b.y {
                            if !(0..=10000).contains(&y) {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(step_num),
                                    message: format!("Pixel branch condition Y coordinate {} is outside valid bounds.", y),
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // 2. Control Flow Graph & Cycle Analysis (Cycles with no exit)
    let exit_node = -1i64;
    let mut adj: HashMap<i64, Vec<i64>> = HashMap::new();

    for (i, &sid) in step_id_order.iter().enumerate() {
        let meta = &step_map[&sid];
        let next_in_seq = if i + 1 < step_id_order.len() {
            step_id_order[i + 1]
        } else {
            exit_node
        };

        if meta.step_type == "branch" {
            if let Some(b) = branch_map.get(&sid) {
                let mut succs = Vec::new();
                if let Some(m_target) = b.on_match_step_id {
                    if step_map.contains_key(&m_target) {
                        succs.push(m_target);
                    }
                }
                if let Some(nm_target) = b.on_no_match_step_id {
                    if step_map.contains_key(&nm_target) {
                        succs.push(nm_target);
                    }
                }
                if succs.is_empty() {
                    succs.push(next_in_seq);
                }
                adj.insert(sid, succs);
            } else {
                adj.insert(sid, vec![next_in_seq]);
            }
        } else {
            adj.insert(sid, vec![next_in_seq]);
        }
    }

    // Reverse reachability from EXIT node
    let mut rev_adj: HashMap<i64, Vec<i64>> = HashMap::new();
    for (&u, succs) in &adj {
        for &v in succs {
            rev_adj.entry(v).or_default().push(u);
        }
    }

    let mut can_reach_exit: HashSet<i64> = HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(exit_node);
    can_reach_exit.insert(exit_node);

    while let Some(curr) = queue.pop_front() {
        if let Some(preds) = rev_adj.get(&curr) {
            for &p in preds {
                if can_reach_exit.insert(p) {
                    queue.push_back(p);
                }
            }
        }
    }

    // Check reachability from start node
    let mut reachable_from_start: HashSet<i64> = HashSet::new();
    let start_node = step_id_order[0];
    let mut start_queue = std::collections::VecDeque::new();
    start_queue.push_back(start_node);
    reachable_from_start.insert(start_node);

    while let Some(curr) = start_queue.pop_front() {
        if let Some(succs) = adj.get(&curr) {
            for &s in succs {
                if s != exit_node && reachable_from_start.insert(s) {
                    start_queue.push_back(s);
                }
            }
        }
    }

    // Any step reachable from start that CANNOT reach exit is in an infinite loop / cycle with no exit
    for &sid in &step_id_order {
        if reachable_from_start.contains(&sid) && !can_reach_exit.contains(&sid) {
            let meta = &step_map[&sid];
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(meta.step_number),
                message: "Step is trapped in an infinite cycle with no exit path.".to_string(),
            });
            break; // Report once per cycle
        }
    }

    // 3. Variable Read-Before-Write Analysis
    let mut step_initialized_vars: HashMap<i64, HashSet<i64>> = HashMap::new();
    let all_var_ids: HashSet<i64> = vars_map.keys().copied().collect();

    for &sid in &step_id_order {
        if sid == start_node {
            step_initialized_vars.insert(sid, HashSet::new());
        } else {
            step_initialized_vars.insert(sid, all_var_ids.clone());
        }
    }

    let mut changed = true;
    let mut iterations = 0;
    while changed && iterations < 100 {
        changed = false;
        iterations += 1;

        for &sid in &step_id_order {
            let predecessors = rev_adj.get(&sid).cloned().unwrap_or_default();
            let incoming_inits = if sid == start_node {
                HashSet::new()
            } else if predecessors.is_empty() {
                HashSet::new()
            } else {
                let mut iter = predecessors.iter();
                if let Some(first) = iter.next() {
                    let mut set = step_initialized_vars
                        .get(first)
                        .cloned()
                        .unwrap_or_default();
                    add_step_writes(*first, &mut set, &fp_map, &fb_map);

                    for p in iter {
                        let mut p_set = step_initialized_vars.get(p).cloned().unwrap_or_default();
                        add_step_writes(*p, &mut p_set, &fp_map, &fb_map);
                        set = set.intersection(&p_set).copied().collect();
                    }
                    set
                } else {
                    HashSet::new()
                }
            };

            if let Some(current_inits) = step_initialized_vars.get_mut(&sid) {
                if *current_inits != incoming_inits {
                    *current_inits = incoming_inits;
                    changed = true;
                }
            }
        }
    }

    for &sid in &step_id_order {
        if !reachable_from_start.contains(&sid) {
            continue;
        }

        let meta = &step_map[&sid];
        let inits = &step_initialized_vars[&sid];

        if meta.step_type == "mouse_click" {
            if let Some(mc) = mc_map.get(&sid) {
                if let Some(x_vid) = mc.x_var_id {
                    if !inits.contains(&x_vid) {
                        if let Some(v) = vars_map.get(&x_vid) {
                            if !available_params.contains(&v.name) {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(meta.step_number),
                                    message: format!("Variable '{}' read for X coordinate before being written along all execution paths.", v.name),
                                });
                            }
                        }
                    }
                }
                if let Some(y_vid) = mc.y_var_id {
                    if !inits.contains(&y_vid) {
                        if let Some(v) = vars_map.get(&y_vid) {
                            if !available_params.contains(&v.name) {
                                errors.push(AutomationLintError {
                                    step_id: Some(sid),
                                    step_number: Some(meta.step_number),
                                    message: format!("Variable '{}' read for Y coordinate before being written along all execution paths.", v.name),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

struct FindPixelMeta {
    x: i32,
    y: i32,
    output_var_id: Option<i64>,
}

struct FindBitmapMeta {
    reference_bitmap_id: i64,
    output_found_var_id: Option<i64>,
    output_x_var_id: Option<i64>,
    output_y_var_id: Option<i64>,
    search_x: Option<i32>,
    search_y: Option<i32>,
}

fn add_step_writes(
    step_id: i64,
    set: &mut HashSet<i64>,
    fp_map: &HashMap<i64, FindPixelMeta>,
    fb_map: &HashMap<i64, FindBitmapMeta>,
) {
    if let Some(fp) = fp_map.get(&step_id) {
        if let Some(vid) = fp.output_var_id {
            set.insert(vid);
        }
    }
    if let Some(fb) = fb_map.get(&step_id) {
        if let Some(vid) = fb.output_found_var_id {
            set.insert(vid);
        }
        if let Some(vid) = fb.output_x_var_id {
            set.insert(vid);
        }
        if let Some(vid) = fb.output_y_var_id {
            set.insert(vid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_automation_lint_error_display() {
        let err1 = AutomationLintError {
            step_id: Some(10),
            step_number: Some(2),
            message: "Missing target step.".to_string(),
        };
        assert_eq!(format!("{}", err1), "Step 2: Missing target step.");

        let err2 = AutomationLintError {
            step_id: None,
            step_number: None,
            message: "Automation has no active steps.".to_string(),
        };
        assert_eq!(format!("{}", err2), "Automation has no active steps.");
    }
}
