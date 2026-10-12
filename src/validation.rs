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

#[derive(Debug, Clone)]
struct VarMeta {
    name: String,
    var_type: String,
}

#[derive(Debug, Clone)]
struct MouseClickMeta {
    x: Option<i32>,
    y: Option<i32>,
    x_var_id: Option<i64>,
    y_var_id: Option<i64>,
    region_var_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct FindPixelMeta {
    x: i32,
    y: i32,
    output_var_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct FindBitmapMeta {
    reference_bitmap_id: i64,
    output_found_var_id: Option<i64>,
    output_x_var_id: Option<i64>,
    output_y_var_id: Option<i64>,
    output_region_var_id: Option<i64>,
    search_x: Option<i32>,
    search_y: Option<i32>,
}

#[derive(Debug, Clone)]
struct BranchMeta {
    condition_type: String,
    on_match_step_id: Option<i64>,
    on_no_match_step_id: Option<i64>,
    reference_bitmap_id: Option<i64>,
    x: Option<i32>,
    y: Option<i32>,
}

#[derive(Debug, Clone)]
enum StepSubtype {
    MouseClick(MouseClickMeta),
    FindPixel(FindPixelMeta),
    FindBitmap(FindBitmapMeta),
    Branch(BranchMeta),
    Other,
}

#[derive(Debug, Clone)]
struct StepNode {
    id: i64,
    step_number: usize,
    step_type: String,
    subtype: StepSubtype,
}

/// Compact bitset for variable initialization tracking.
/// Every `VarBitSet` for an automation shares the exact same word vector length `num_words = num_vars.div_ceil(64)`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VarBitSet {
    words: Vec<u64>,
}

impl VarBitSet {
    fn empty(num_vars: usize) -> Self {
        let num_words = num_vars.div_ceil(64).max(1);
        Self {
            words: vec![0; num_words],
        }
    }

    fn all(num_vars: usize) -> Self {
        let num_words = num_vars.div_ceil(64).max(1);
        if num_vars == 0 {
            return Self::empty(0);
        }
        let mut words = vec![u64::MAX; num_words];
        let rem = num_vars % 64;
        if rem != 0 {
            if let Some(last) = words.last_mut() {
                *last = (1u64 << rem) - 1;
            }
        }
        Self { words }
    }

    fn set(&mut self, idx: usize) {
        let w_idx = idx / 64;
        let b_idx = idx % 64;
        if let Some(w) = self.words.get_mut(w_idx) {
            *w |= 1u64 << b_idx;
        }
    }

    fn contains(&self, idx: usize) -> bool {
        let w_idx = idx / 64;
        let b_idx = idx % 64;
        self.words
            .get(w_idx)
            .is_some_and(|w| (w & (1u64 << b_idx)) != 0)
    }

    fn union_with(&mut self, other: &Self) {
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a |= *b;
        }
    }

    fn intersect_with(&mut self, other: &Self) {
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a &= *b;
        }
    }
}

/// Lints and validates an automation to ensure it is safe and ready for execution.
///
/// Performance-tuned: Uses index-mapped step nodes, fixed-size adjacency vectors,
/// boolean reachability sets, and zero-allocation bitmask operations for variable dataflow analysis.
pub async fn validate_automation_for_activation(
    db: &PgPool,
    automation_id: i64,
) -> Result<(), Vec<AutomationLintError>> {
    let mut errors = Vec::new();

    // 1. Concurrently fetch step metadata, variables, parameters, subtype details, and bitmaps.
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

    let n = raw_steps.len();

    // Map step_id -> 0-based integer index
    let mut step_id_to_idx: HashMap<i64, usize> = HashMap::with_capacity(n);
    for (idx, row) in raw_steps.iter().enumerate() {
        let sid: i64 = row.get("id");
        step_id_to_idx.insert(sid, idx);
    }

    // Build subtype lookup maps temporarily to initialize contiguous StepNode array
    let mut mc_map: HashMap<i64, MouseClickMeta> = HashMap::with_capacity(mc_rows.len());
    for r in mc_rows {
        mc_map.insert(
            r.get("step_id"),
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
        fp_map.insert(
            r.get("step_id"),
            FindPixelMeta {
                x: r.get("x"),
                y: r.get("y"),
                output_var_id: r.get("output_variable_id"),
            },
        );
    }

    let mut fb_map: HashMap<i64, FindBitmapMeta> = HashMap::with_capacity(fb_rows.len());
    for r in fb_rows {
        fb_map.insert(
            r.get("step_id"),
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
        branch_map.insert(
            r.get("step_id"),
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

    // Construct contiguous steps vector
    let mut steps: Vec<StepNode> = Vec::with_capacity(n);
    for (idx, row) in raw_steps.into_iter().enumerate() {
        let sid: i64 = row.get("id");
        let stype: String = row.get("step_type");

        let subtype = match stype.as_str() {
            "mouse_click" => mc_map
                .remove(&sid)
                .map(StepSubtype::MouseClick)
                .unwrap_or(StepSubtype::Other),
            "find_pixel_rgb" => fp_map
                .remove(&sid)
                .map(StepSubtype::FindPixel)
                .unwrap_or(StepSubtype::Other),
            "find_bitmap" => fb_map
                .remove(&sid)
                .map(StepSubtype::FindBitmap)
                .unwrap_or(StepSubtype::Other),
            "branch" => branch_map
                .remove(&sid)
                .map(StepSubtype::Branch)
                .unwrap_or(StepSubtype::Other),
            _ => StepSubtype::Other,
        };

        steps.push(StepNode {
            id: sid,
            step_number: idx + 1,
            step_type: stype,
            subtype,
        });
    }

    // Map variable ID -> index & metadata
    let num_vars = var_rows.len();
    let mut vars_map: HashMap<i64, (usize, VarMeta)> = HashMap::with_capacity(num_vars);
    for (idx, vr) in var_rows.into_iter().enumerate() {
        let vid: i64 = vr.get("id");
        let meta = VarMeta {
            name: vr.get("name"),
            var_type: vr.get("var_type"),
        };
        vars_map.insert(vid, (idx, meta));
    }

    let mut available_params: HashSet<String> = HashSet::with_capacity(param_rows.len());
    for pr in param_rows {
        let pname: String = pr.get("name");
        available_params.insert(pname);
    }

    // 2. Validate individual steps for bounds, types, referenced IDs
    for step in &steps {
        match &step.subtype {
            StepSubtype::MouseClick(mc) => {
                validate_mouse_click(step.id, step.step_number, mc, &vars_map, &mut errors);
            }
            StepSubtype::FindPixel(fp) => {
                validate_find_pixel(step.id, step.step_number, fp, &vars_map, &mut errors);
            }
            StepSubtype::FindBitmap(fb) => {
                validate_find_bitmap(
                    step.id,
                    step.step_number,
                    fb,
                    &vars_map,
                    &existing_bitmaps,
                    &mut errors,
                );
            }
            StepSubtype::Branch(b) => {
                validate_branch(
                    step.id,
                    step.step_number,
                    b,
                    &step_id_to_idx,
                    &existing_bitmaps,
                    &mut errors,
                );
            }
            StepSubtype::Other => {}
        }
    }

    // 3. Control Flow Graph Construction (Nodes: 0..N represent steps, N is EXIT_NODE)
    let exit_node = n;
    let mut rev_adj: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    let mut successors: Vec<[Option<usize>; 2]> = vec![[None, None]; n];

    for (i, step) in steps.iter().enumerate() {
        let next_in_seq = if i + 1 < n { i + 1 } else { exit_node };

        if step.step_type == "branch" {
            if let StepSubtype::Branch(b) = &step.subtype {
                let mut count = 0;
                let mut succs = [None, None];

                if let Some(m_target) = b.on_match_step_id
                    && let Some(&m_idx) = step_id_to_idx.get(&m_target)
                {
                    succs[count] = Some(m_idx);
                    count += 1;
                }
                if let Some(nm_target) = b.on_no_match_step_id
                    && let Some(&nm_idx) = step_id_to_idx.get(&nm_target)
                {
                    succs[count] = Some(nm_idx);
                    count += 1;
                }

                if count == 0 {
                    succs[0] = Some(next_in_seq);
                }
                successors[i] = succs;
            } else {
                successors[i][0] = Some(next_in_seq);
            }
        } else {
            successors[i][0] = Some(next_in_seq);
        }

        for &succ_opt in &successors[i] {
            if let Some(s) = succ_opt {
                rev_adj[s].push(i);
            }
        }
    }

    // Reachability from EXIT_NODE (reverse BFS using boolean flag array)
    let mut can_reach_exit = vec![false; n + 1];
    let mut queue = std::collections::VecDeque::with_capacity(n + 1);
    queue.push_back(exit_node);
    can_reach_exit[exit_node] = true;

    while let Some(curr) = queue.pop_front() {
        for &p in &rev_adj[curr] {
            if !can_reach_exit[p] {
                can_reach_exit[p] = true;
                queue.push_back(p);
            }
        }
    }

    // Reachability from START_NODE (forward BFS)
    let mut reachable_from_start = vec![false; n + 1];
    queue.push_back(0); // Start node = index 0
    reachable_from_start[0] = true;

    while let Some(curr) = queue.pop_front() {
        if curr < n {
            for &succ_opt in &successors[curr] {
                if let Some(s) = succ_opt {
                    if s != exit_node && !reachable_from_start[s] {
                        reachable_from_start[s] = true;
                        queue.push_back(s);
                    }
                }
            }
        }
    }

    // Cycle check: Any step reachable from start that CANNOT reach exit is trapped in an infinite cycle
    for (i, step) in steps.iter().enumerate() {
        if reachable_from_start[i] && !can_reach_exit[i] {
            errors.push(AutomationLintError {
                step_id: Some(step.id),
                step_number: Some(step.step_number),
                message: "Step is trapped in an infinite cycle with no exit path.".to_string(),
            });
            break; // Report once per cycle
        }
    }

    // 4. Precompute written variables per step ONCE as compact bitsets
    let mut step_writes: Vec<VarBitSet> = vec![VarBitSet::empty(num_vars); n];
    for (i, step) in steps.iter().enumerate() {
        let mut writes = VarBitSet::empty(num_vars);
        match &step.subtype {
            StepSubtype::FindPixel(fp) => {
                if let Some(vid) = fp.output_var_id
                    && let Some(&(v_idx, _)) = vars_map.get(&vid)
                {
                    writes.set(v_idx);
                }
            }
            StepSubtype::FindBitmap(fb) => {
                if let Some(vid) = fb.output_found_var_id
                    && let Some(&(v_idx, _)) = vars_map.get(&vid)
                {
                    writes.set(v_idx);
                }
                if let Some(vid) = fb.output_x_var_id
                    && let Some(&(v_idx, _)) = vars_map.get(&vid)
                {
                    writes.set(v_idx);
                }
                if let Some(vid) = fb.output_y_var_id
                    && let Some(&(v_idx, _)) = vars_map.get(&vid)
                {
                    writes.set(v_idx);
                }
                if let Some(vid) = fb.output_region_var_id
                    && let Some(&(v_idx, _)) = vars_map.get(&vid)
                {
                    writes.set(v_idx);
                }
            }
            _ => {}
        }
        step_writes[i] = writes;
    }

    // 5. Zero-allocation fixed-point variable read-before-write analysis
    let mut step_inits: Vec<VarBitSet> = vec![VarBitSet::all(num_vars); n];
    if !step_inits.is_empty() {
        step_inits[0] = VarBitSet::empty(num_vars); // Start node has 0 variables initialized
    }

    let mut changed = true;
    let mut iterations = 0;
    while changed && iterations < 100 {
        changed = false;
        iterations += 1;

        for i in 1..n {
            let preds = &rev_adj[i];
            if preds.is_empty() {
                continue;
            }

            // New inits for step `i` is the INTERSECTION of (inits[p] | writes[p]) across all predecessors `p`
            let mut new_inits = VarBitSet::all(num_vars);
            for &p in preds {
                let mut pred_out = step_inits[p].clone();
                pred_out.union_with(&step_writes[p]);
                new_inits.intersect_with(&pred_out);
            }

            if new_inits != step_inits[i] {
                step_inits[i] = new_inits;
                changed = true;
            }
        }
    }

    // Report read-before-write errors
    for (i, step) in steps.iter().enumerate() {
        if !reachable_from_start[i] {
            continue;
        }

        let inits = &step_inits[i];

        if step.step_type == "mouse_click"
            && let StepSubtype::MouseClick(mc) = &step.subtype
        {
            if let Some(x_vid) = mc.x_var_id
                && let Some(&(v_idx, ref v)) = vars_map.get(&x_vid)
                && !inits.contains(v_idx)
                && !available_params.contains(&v.name)
            {
                errors.push(AutomationLintError {
                    step_id: Some(step.id),
                    step_number: Some(step.step_number),
                    message: format!("Variable '{}' read for X coordinate before being written along all execution paths.", v.name),
                });
            }
            if let Some(y_vid) = mc.y_var_id
                && let Some(&(v_idx, ref v)) = vars_map.get(&y_vid)
                && !inits.contains(v_idx)
                && !available_params.contains(&v.name)
            {
                errors.push(AutomationLintError {
                    step_id: Some(step.id),
                    step_number: Some(step.step_number),
                    message: format!("Variable '{}' read for Y coordinate before being written along all execution paths.", v.name),
                });
            }
            if let Some(r_vid) = mc.region_var_id
                && let Some(&(v_idx, ref v)) = vars_map.get(&r_vid)
                && !inits.contains(v_idx)
                && !available_params.contains(&v.name)
            {
                errors.push(AutomationLintError {
                    step_id: Some(step.id),
                    step_number: Some(step.step_number),
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
    vars_map: &HashMap<i64, (usize, VarMeta)>,
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
    vars_map: &HashMap<i64, (usize, VarMeta)>,
    errors: &mut Vec<AutomationLintError>,
) {
    if let Some((_, v)) = vars_map.get(&var_id) {
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
    vars_map: &HashMap<i64, (usize, VarMeta)>,
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
        if let Some((_, v)) = vars_map.get(&out_vid) {
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
    vars_map: &HashMap<i64, (usize, VarMeta)>,
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
    vars_map: &HashMap<i64, (usize, VarMeta)>,
    errors: &mut Vec<AutomationLintError>,
) {
    if let Some((_, v)) = vars_map.get(&var_id)
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
    step_id_to_idx: &HashMap<i64, usize>,
    existing_bitmaps: &HashSet<i64>,
    errors: &mut Vec<AutomationLintError>,
) {
    match b.on_match_step_id {
        Some(target_id) => {
            if !step_id_to_idx.contains_key(&target_id) {
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
            if !step_id_to_idx.contains_key(&target_id) {
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

    #[test]
    fn test_var_bitset_operations() {
        let mut bs = VarBitSet::empty(10);
        assert!(!bs.contains(0));

        bs.set(0);
        bs.set(5);
        assert!(bs.contains(0));
        assert!(bs.contains(5));
        assert!(!bs.contains(1));

        let all = VarBitSet::all(10);
        assert!(all.contains(0));
        assert!(all.contains(9));
        assert!(!all.contains(10));

        let mut b1 = VarBitSet::all(10);
        let mut b2 = VarBitSet::empty(10);
        b2.set(3);

        b1.intersect_with(&b2);
        assert!(!b1.contains(0));
        assert!(b1.contains(3));

        b1.union_with(&all);
        assert!(b1.contains(0));
    }

    #[test]
    fn test_var_bitset_operations_large() {
        // Test with 150 variables (> 2 x 64 bits)
        let num_vars = 150;
        let mut empty = VarBitSet::empty(num_vars);
        let all = VarBitSet::all(num_vars);

        assert!(!empty.contains(0));
        assert!(!empty.contains(65));
        assert!(!empty.contains(149));

        assert!(all.contains(0));
        assert!(all.contains(64));
        assert!(all.contains(149));
        assert!(!all.contains(150));

        // Union empty with set bit at index 100
        empty.set(100);
        assert!(empty.contains(100));

        let mut intersected = all.clone();
        intersected.intersect_with(&empty);
        assert!(!intersected.contains(0));
        assert!(intersected.contains(100));
        assert!(!intersected.contains(101));
    }
}
