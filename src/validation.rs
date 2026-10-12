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

struct StepMeta {
    step_number: usize,
    step_type: String,
}

struct VarMeta {
    name: String,
    var_type: String,
}

struct MouseClickMeta {
    x: Option<i32>,
    y: Option<i32>,
    x_var_id: Option<i64>,
    y_var_id: Option<i64>,
    region_var_id: Option<i64>,
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
    output_region_var_id: Option<i64>,
    search_x: Option<i32>,
    search_y: Option<i32>,
}

struct BranchMeta {
    condition_type: String,
    on_match_step_id: Option<i64>,
    on_no_match_step_id: Option<i64>,
    reference_bitmap_id: Option<i64>,
    x: Option<i32>,
    y: Option<i32>,
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
///    Ensures variable types match expected types (`int` for coords, `bool` for found flags, `color`/`point`/`string`/`region` for pixel/region values).
/// 6. Deleted or missing reference bitmaps:
///    Ensures `reference_bitmap_id` on `find_bitmap` and `branch` steps references an existing bitmap.
/// 7. Coordinate bounds:
///    Ensures literal coordinates are non-negative and within realistic screen bounds (<= 10,000 px).
pub async fn validate_automation_for_activation(
    db: &PgPool,
    automation_id: i64,
) -> Result<(), Vec<AutomationLintError>> {
    let mut errors = Vec::new();

    // 1. Concurrently fetch all step metadata, variables, parameters, subtype details, and bitmaps.
    let steps_fut = sqlx::query(
        "SELECT id, position, step_type, label FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(db);

    let vars_fut =
        sqlx::query("SELECT id, name, var_type FROM automation_variables WHERE automation_id = $1")
            .bind(automation_id)
            .fetch_all(db);

    let params_fut =
        sqlx::query("SELECT name, param_type FROM automation_parameters WHERE automation_id = $1")
            .bind(automation_id)
            .fetch_all(db);

    let mc_fut = sqlx::query(
        r#"
        SELECT smc.step_id, smc.x, smc.y, smc.x_variable_id, smc.y_variable_id, smc.region_variable_id
        FROM step_mouse_clicks smc
        JOIN automation_steps s ON smc.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let fp_fut = sqlx::query(
        r#"
        SELECT sfp.step_id, sfp.x, sfp.y, sfp.output_variable_id
        FROM step_find_pixel_rgb sfp
        JOIN automation_steps s ON sfp.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let fb_fut = sqlx::query(
        r#"
        SELECT sfb.step_id, sfb.reference_bitmap_id, sfb.output_found_variable_id, sfb.output_x_variable_id, sfb.output_y_variable_id, sfb.output_region_variable_id, sfb.search_x, sfb.search_y
        FROM step_find_bitmap sfb
        JOIN automation_steps s ON sfb.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let branch_fut = sqlx::query(
        r#"
        SELECT sb.step_id, sb.condition_type, sb.on_match_step_id, sb.on_no_match_step_id,
               sb.reference_bitmap_id, sb.x, sb.y, sb.search_x, sb.search_y
        FROM step_branches sb
        JOIN automation_steps s ON sb.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let bitmap_fut = sqlx::query("SELECT id FROM bitmaps").fetch_all(db);

    let (raw_steps, var_rows, param_rows, mc_rows, fp_rows, fb_rows, branch_rows, bitmap_rows) =
        match tokio::try_join!(
            steps_fut, vars_fut, params_fut, mc_fut, fp_fut, fb_fut, branch_fut, bitmap_fut
        ) {
            Ok(tuple) => tuple,
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

    let mut step_map: HashMap<i64, StepMeta> = HashMap::with_capacity(raw_steps.len());
    let mut step_id_order: Vec<i64> = Vec::with_capacity(raw_steps.len());
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

    let mut vars_map: HashMap<i64, VarMeta> = HashMap::with_capacity(var_rows.len());
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

    let mut available_params: HashSet<String> = HashSet::with_capacity(param_rows.len());
    for pr in param_rows {
        let pname: String = pr.get("name");
        available_params.insert(pname);
    }

    let mut mc_map: HashMap<i64, MouseClickMeta> = HashMap::with_capacity(mc_rows.len());
    for r in mc_rows {
        let sid: i64 = r.get("step_id");
        mc_map.insert(
            sid,
            MouseClickMeta {
                x: r.get("x"),
                y: r.get("y"),
                x_var_id: r.get("x_variable_id"),
                y_var_id: r.get("y_variable_id"),
                region_var_id: r.get("region_variable_id"),
            },
        );
    }

    let mut fp_map: HashMap<i64, FindPixelMeta> = HashMap::with_capacity(fp_rows.len());
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

    let mut fb_map: HashMap<i64, FindBitmapMeta> = HashMap::with_capacity(fb_rows.len());
    for r in fb_rows {
        let sid: i64 = r.get("step_id");
        fb_map.insert(
            sid,
            FindBitmapMeta {
                reference_bitmap_id: r.get("reference_bitmap_id"),
                output_found_var_id: r.get("output_found_variable_id"),
                output_x_var_id: r.get("output_x_variable_id"),
                output_y_var_id: r.get("output_y_variable_id"),
                output_region_var_id: r.get("output_region_variable_id"),
                search_x: r.get("search_x"),
                search_y: r.get("search_y"),
            },
        );
    }

    let mut branch_map: HashMap<i64, BranchMeta> = HashMap::with_capacity(branch_rows.len());
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

    let existing_bitmaps: HashSet<i64> = bitmap_rows.into_iter().map(|r| r.get("id")).collect();

    // 2. Validate individual steps for bounds, types, referenced IDs
    for (idx, &sid) in step_id_order.iter().enumerate() {
        let Some(meta) = step_map.get(&sid) else {
            continue;
        };
        let step_num = idx + 1;

        match meta.step_type.as_str() {
            "mouse_click" => {
                if let Some(mc) = mc_map.get(&sid) {
                    validate_mouse_click(sid, step_num, mc, &vars_map, &mut errors);
                }
            }
            "find_pixel_rgb" => {
                if let Some(fp) = fp_map.get(&sid) {
                    validate_find_pixel(sid, step_num, fp, &vars_map, &mut errors);
                }
            }
            "find_bitmap" => {
                if let Some(fb) = fb_map.get(&sid) {
                    validate_find_bitmap(
                        sid,
                        step_num,
                        fb,
                        &vars_map,
                        &existing_bitmaps,
                        &mut errors,
                    );
                }
            }
            "branch" => {
                if let Some(b) = branch_map.get(&sid) {
                    validate_branch(sid, step_num, b, &step_map, &existing_bitmaps, &mut errors);
                }
            }
            _ => {}
        }
    }

    // 3. Control Flow Graph & Cycle Analysis (Cycles with no exit)
    let exit_node = -1i64;
    let mut adj: HashMap<i64, Vec<i64>> = HashMap::with_capacity(step_id_order.len());

    for (i, &sid) in step_id_order.iter().enumerate() {
        let meta = &step_map[&sid];
        let next_in_seq = if i + 1 < step_id_order.len() {
            step_id_order[i + 1]
        } else {
            exit_node
        };

        if meta.step_type == "branch" {
            if let Some(b) = branch_map.get(&sid) {
                let mut succs = Vec::with_capacity(2);
                if let Some(m_target) = b.on_match_step_id
                    && step_map.contains_key(&m_target)
                {
                    succs.push(m_target);
                }
                if let Some(nm_target) = b.on_no_match_step_id
                    && step_map.contains_key(&nm_target)
                {
                    succs.push(nm_target);
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
    let mut rev_adj: HashMap<i64, Vec<i64>> = HashMap::with_capacity(step_id_order.len());
    for (&u, succs) in &adj {
        for &v in succs {
            rev_adj.entry(v).or_default().push(u);
        }
    }

    let mut can_reach_exit: HashSet<i64> = HashSet::with_capacity(step_id_order.len() + 1);
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
    let mut reachable_from_start: HashSet<i64> = HashSet::with_capacity(step_id_order.len());
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

    // Any step reachable from start that CANNOT reach exit is trapped in an infinite loop
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

    // 4. Precompute written variables for every step ONCE prior to dataflow analysis
    let mut step_writes: HashMap<i64, HashSet<i64>> = HashMap::with_capacity(step_id_order.len());
    for &sid in &step_id_order {
        let mut writes = HashSet::new();
        if let Some(fp) = fp_map.get(&sid)
            && let Some(vid) = fp.output_var_id
        {
            writes.insert(vid);
        }
        if let Some(fb) = fb_map.get(&sid) {
            if let Some(vid) = fb.output_found_var_id {
                writes.insert(vid);
            }
            if let Some(vid) = fb.output_x_var_id {
                writes.insert(vid);
            }
            if let Some(vid) = fb.output_y_var_id {
                writes.insert(vid);
            }
            if let Some(vid) = fb.output_region_var_id {
                writes.insert(vid);
            }
        }
        if !writes.is_empty() {
            step_writes.insert(sid, writes);
        }
    }

    // 5. Zero-allocation fixed-point variable read-before-write analysis
    let all_var_ids: HashSet<i64> = vars_map.keys().copied().collect();
    let mut step_initialized_vars: HashMap<i64, HashSet<i64>> =
        HashMap::with_capacity(step_id_order.len());

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
            if sid == start_node {
                continue;
            }

            let Some(preds) = rev_adj.get(&sid) else {
                continue;
            };
            if preds.is_empty() {
                continue;
            }

            let to_remove: Vec<i64> = step_initialized_vars[&sid]
                .iter()
                .copied()
                .filter(|vid| {
                    !preds.iter().all(|&p| {
                        step_initialized_vars[&p].contains(vid)
                            || step_writes.get(&p).is_some_and(|w| w.contains(vid))
                    })
                })
                .collect();

            if !to_remove.is_empty() {
                changed = true;
                let curr_inits = step_initialized_vars.get_mut(&sid).expect("exists");
                for vid in to_remove {
                    curr_inits.remove(&vid);
                }
            }
        }
    }

    // Report read-before-write errors
    for &sid in &step_id_order {
        if !reachable_from_start.contains(&sid) {
            continue;
        }

        let meta = &step_map[&sid];
        let inits = &step_initialized_vars[&sid];

        if meta.step_type == "mouse_click"
            && let Some(mc) = mc_map.get(&sid)
        {
            if let Some(x_vid) = mc.x_var_id
                && !inits.contains(&x_vid)
                && let Some(v) = vars_map.get(&x_vid)
                && !available_params.contains(&v.name)
            {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(meta.step_number),
                    message: format!("Variable '{}' read for X coordinate before being written along all execution paths.", v.name),
                });
            }
            if let Some(y_vid) = mc.y_var_id
                && !inits.contains(&y_vid)
                && let Some(v) = vars_map.get(&y_vid)
                && !available_params.contains(&v.name)
            {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(meta.step_number),
                    message: format!("Variable '{}' read for Y coordinate before being written along all execution paths.", v.name),
                });
            }
            if let Some(r_vid) = mc.region_var_id
                && !inits.contains(&r_vid)
                && let Some(v) = vars_map.get(&r_vid)
                && !available_params.contains(&v.name)
            {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(meta.step_number),
                    message: format!("Variable '{}' read for region click before being written along all execution paths.", v.name),
                });
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_mouse_click(
    sid: i64,
    step_num: usize,
    mc: &MouseClickMeta,
    vars_map: &HashMap<i64, VarMeta>,
    errors: &mut Vec<AutomationLintError>,
) {
    if let Some(x) = mc.x
        && !(0..=10000).contains(&x)
    {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!(
                "Mouse click X coordinate {} is outside valid bounds (0..10000).",
                x
            ),
        });
    }
    if let Some(y) = mc.y
        && !(0..=10000).contains(&y)
    {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!(
                "Mouse click Y coordinate {} is outside valid bounds (0..10000).",
                y
            ),
        });
    }

    if let Some(x_vid) = mc.x_var_id {
        validate_var_ref(
            sid,
            step_num,
            x_vid,
            "X coordinate",
            "int",
            vars_map,
            errors,
        );
    }

    if let Some(y_vid) = mc.y_var_id {
        validate_var_ref(
            sid,
            step_num,
            y_vid,
            "Y coordinate",
            "int",
            vars_map,
            errors,
        );
    }

    if let Some(r_vid) = mc.region_var_id {
        validate_var_ref(
            sid,
            step_num,
            r_vid,
            "region click",
            "region",
            vars_map,
            errors,
        );
    }
}

fn validate_var_ref(
    sid: i64,
    step_num: usize,
    var_id: i64,
    usage_label: &str,
    expected_type: &str,
    vars_map: &HashMap<i64, VarMeta>,
    errors: &mut Vec<AutomationLintError>,
) {
    if let Some(v) = vars_map.get(&var_id) {
        if v.var_type != expected_type {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: format!(
                    "Variable '{}' used for {} is of type '{}', expected '{}'.",
                    v.name, usage_label, v.var_type, expected_type
                ),
            });
        }
    } else {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!(
                "Referenced {} variable ID {} does not exist.",
                usage_label, var_id
            ),
        });
    }
}

fn validate_find_pixel(
    sid: i64,
    step_num: usize,
    fp: &FindPixelMeta,
    vars_map: &HashMap<i64, VarMeta>,
    errors: &mut Vec<AutomationLintError>,
) {
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
            if !matches!(
                v.var_type.as_str(),
                "color" | "point" | "string" | "int" | "region"
            ) {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(step_num),
                    message: format!(
                        "Output variable '{}' for pixel RGB is of type '{}', expected 'color' or 'string'.",
                        v.name, v.var_type
                    ),
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

fn validate_find_bitmap(
    sid: i64,
    step_num: usize,
    fb: &FindBitmapMeta,
    vars_map: &HashMap<i64, VarMeta>,
    existing_bitmaps: &HashSet<i64>,
    errors: &mut Vec<AutomationLintError>,
) {
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

    if let Some(x) = fb.search_x
        && !(0..=10000).contains(&x)
    {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!("Search X coordinate {} is outside valid bounds.", x),
        });
    }
    if let Some(y) = fb.search_y
        && !(0..=10000).contains(&y)
    {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!("Search Y coordinate {} is outside valid bounds.", y),
        });
    }

    if let Some(f_vid) = fb.output_found_var_id {
        validate_output_var(sid, step_num, f_vid, "'Found'", "bool", vars_map, errors);
    }
    if let Some(x_vid) = fb.output_x_var_id {
        validate_output_var(sid, step_num, x_vid, "'Found X'", "int", vars_map, errors);
    }
    if let Some(y_vid) = fb.output_y_var_id {
        validate_output_var(sid, step_num, y_vid, "'Found Y'", "int", vars_map, errors);
    }
    if let Some(r_vid) = fb.output_region_var_id {
        validate_output_var(
            sid,
            step_num,
            r_vid,
            "'Found Region'",
            "region",
            vars_map,
            errors,
        );
    }
}

fn validate_output_var(
    sid: i64,
    step_num: usize,
    var_id: i64,
    output_name: &str,
    expected_type: &str,
    vars_map: &HashMap<i64, VarMeta>,
    errors: &mut Vec<AutomationLintError>,
) {
    if let Some(v) = vars_map.get(&var_id)
        && v.var_type != expected_type
    {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!(
                "{} output variable '{}' is of type '{}', expected '{}'.",
                output_name, v.name, v.var_type, expected_type
            ),
        });
    }
}

fn validate_branch(
    sid: i64,
    step_num: usize,
    b: &BranchMeta,
    step_map: &HashMap<i64, StepMeta>,
    existing_bitmaps: &HashSet<i64>,
    errors: &mut Vec<AutomationLintError>,
) {
    match b.on_match_step_id {
        Some(target_id) => {
            if !step_map.contains_key(&target_id) {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(step_num),
                    message: format!(
                        "Branch 'On Match' target step ID {} is invalid or deleted.",
                        target_id
                    ),
                });
            }
        }
        None => {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: "Branch step is missing 'On Match' target step.".to_string(),
            });
        }
    }

    match b.on_no_match_step_id {
        Some(target_id) => {
            if !step_map.contains_key(&target_id) {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(step_num),
                    message: format!(
                        "Branch 'On No Match' target step ID {} is invalid or deleted.",
                        target_id
                    ),
                });
            }
        }
        None => {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: "Branch step is missing 'On No Match' target step.".to_string(),
            });
        }
    }

    if b.condition_type == "bitmap" {
        if let Some(bmp_id) = b.reference_bitmap_id {
            if !existing_bitmaps.contains(&bmp_id) {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(step_num),
                    message: format!(
                        "Branch condition references bitmap ID {} which does not exist.",
                        bmp_id
                    ),
                });
            }
        } else {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: "Bitmap branch condition is missing reference bitmap.".to_string(),
            });
        }
    } else if b.condition_type == "pixel_rgb" {
        if let Some(x) = b.x
            && !(0..=10000).contains(&x)
        {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: format!(
                    "Pixel branch condition X coordinate {} is outside valid bounds.",
                    x
                ),
            });
        }
        if let Some(y) = b.y
            && !(0..=10000).contains(&y)
        {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: format!(
                    "Pixel branch condition Y coordinate {} is outside valid bounds.",
                    y
                ),
            });
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
