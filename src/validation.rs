use sqlx::PgPool;
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

#[derive(Debug, sqlx::FromRow)]
struct StepRow {
    id: i64,
    step_type: String,
}

#[derive(Debug, sqlx::FromRow)]
struct VarRow {
    id: i64,
    name: String,
    var_type: String,
}

#[derive(Debug, sqlx::FromRow)]
struct ParamRow {
    name: String,
}

#[derive(Debug, sqlx::FromRow)]
struct MouseClickRow {
    step_id: i64,
    x: Option<i32>,
    y: Option<i32>,
    x_variable_id: Option<i64>,
    y_variable_id: Option<i64>,
    region_variable_id: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct FindPixelRow {
    step_id: i64,
    x: i32,
    y: i32,
    output_variable_id: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct FindBitmapRow {
    step_id: i64,
    reference_bitmap_id: i64,
    output_found_variable_id: Option<i64>,
    output_x_variable_id: Option<i64>,
    output_y_variable_id: Option<i64>,
    output_region_variable_id: Option<i64>,
    search_x: Option<i32>,
    search_y: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct BranchRow {
    step_id: i64,
    condition_type: String,
    on_match_step_id: Option<i64>,
    on_no_match_step_id: Option<i64>,
    reference_bitmap_id: Option<i64>,
    x: Option<i32>,
    y: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct BitmapRow {
    id: i64,
}

#[derive(Debug, Clone)]
struct VarMeta {
    index: usize,
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
/// Inlined for `num_vars <= 64` to eliminate inner heap allocations on validation hot path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VarBitSet {
    w0: u64,
    extra: Option<Box<[u64]>>,
}

impl VarBitSet {
    fn empty(num_vars: usize) -> Self {
        let extra = if num_vars > 64 {
            let extra_words = (num_vars - 64).div_ceil(64);
            Some(vec![0u64; extra_words].into_boxed_slice())
        } else {
            None
        };
        Self { w0: 0, extra }
    }

    fn all(num_vars: usize) -> Self {
        let mut bs = Self::empty(num_vars);
        bs.fill_all(num_vars);
        bs
    }

    fn fill_all(&mut self, num_vars: usize) {
        if num_vars == 0 {
            self.w0 = 0;
            if let Some(extra) = &mut self.extra {
                extra.fill(0);
            }
            return;
        }

        if num_vars <= 64 {
            self.w0 = if num_vars == 64 {
                u64::MAX
            } else {
                (1u64 << num_vars) - 1
            };
            if let Some(extra) = &mut self.extra {
                extra.fill(0);
            }
        } else {
            self.w0 = u64::MAX;
            if let Some(extra) = &mut self.extra {
                extra.fill(u64::MAX);
                let rem = (num_vars - 64) % 64;
                if rem != 0 {
                    if let Some(last) = extra.last_mut() {
                        *last = (1u64 << rem) - 1;
                    }
                }
            }
        }
    }

    #[inline]
    fn copy_from(&mut self, other: &Self) {
        self.w0 = other.w0;
        if let (Some(s_extra), Some(o_extra)) = (&mut self.extra, &other.extra) {
            s_extra.copy_from_slice(o_extra);
        }
    }

    #[inline]
    fn set(&mut self, idx: usize) {
        if idx < 64 {
            self.w0 |= 1u64 << idx;
        } else if let Some(extra) = &mut self.extra {
            if let Some(w) = extra.get_mut((idx - 64) / 64) {
                *w |= 1u64 << ((idx - 64) % 64);
            }
        }
    }

    #[inline]
    fn contains(&self, idx: usize) -> bool {
        if idx < 64 {
            (self.w0 & (1u64 << idx)) != 0
        } else {
            self.extra.as_ref().is_some_and(|e| {
                e.get((idx - 64) / 64)
                    .is_some_and(|&w| (w & (1u64 << ((idx - 64) % 64))) != 0)
            })
        }
    }

    /// Intersects `self` in-place with `(pred_inits | pred_writes)` for predecessor node `p`.
    #[inline]
    fn intersect_with_pred_output(&mut self, pred_inits: &Self, pred_writes: &Self) {
        self.w0 &= pred_inits.w0 | pred_writes.w0;
        if let (Some(s_ex), Some(i_ex), Some(w_ex)) =
            (&mut self.extra, &pred_inits.extra, &pred_writes.extra)
        {
            for (sw, (&iw, &ww)) in s_ex.iter_mut().zip(i_ex.iter().zip(w_ex.iter())) {
                *sw &= iw | ww;
            }
        }
    }
}

/// Lints and validates an automation to ensure it is safe and ready for execution.
///
/// Performance-tuned: Uses strongly-typed `FromRow` queries, index-mapped step nodes, direct subtype mapping into vectors,
/// boolean reachability sets, and zero-allocation bitmask operations for variable dataflow analysis.
pub async fn validate_automation_for_activation(
    db: &PgPool,
    automation_id: i64,
) -> Result<(), Vec<AutomationLintError>> {
    let mut errors = Vec::new();

    // 1. Concurrently fetch step metadata, variables, parameters, subtype details, and bitmaps using strongly typed sqlx::query_as.
    let steps_fut = sqlx::query_as::<_, StepRow>(
        "SELECT id, step_type FROM automation_steps WHERE automation_id = $1 AND deleted_at IS NULL ORDER BY position ASC, id ASC",
    )
    .bind(automation_id)
    .fetch_all(db);

    let vars_fut = sqlx::query_as::<_, VarRow>(
        "SELECT id, name, var_type FROM automation_variables WHERE automation_id = $1",
    )
    .bind(automation_id)
    .fetch_all(db);

    let params_fut = sqlx::query_as::<_, ParamRow>(
        "SELECT name FROM automation_parameters WHERE automation_id = $1",
    )
    .bind(automation_id)
    .fetch_all(db);

    let mc_fut = sqlx::query_as::<_, MouseClickRow>(
        r#"
        SELECT smc.step_id, smc.x, smc.y, smc.x_variable_id, smc.y_variable_id, smc.region_variable_id
        FROM step_mouse_clicks smc
        JOIN automation_steps s ON smc.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let fp_fut = sqlx::query_as::<_, FindPixelRow>(
        r#"
        SELECT sfp.step_id, sfp.x, sfp.y, sfp.output_variable_id
        FROM step_find_pixel_rgb sfp
        JOIN automation_steps s ON sfp.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let fb_fut = sqlx::query_as::<_, FindBitmapRow>(
        r#"
        SELECT sfb.step_id, sfb.reference_bitmap_id, sfb.output_found_variable_id, sfb.output_x_variable_id, sfb.output_y_variable_id, sfb.output_region_variable_id, sfb.search_x, sfb.search_y
        FROM step_find_bitmap sfb
        JOIN automation_steps s ON sfb.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let branch_fut = sqlx::query_as::<_, BranchRow>(
        r#"
        SELECT sb.step_id, sb.condition_type, sb.on_match_step_id, sb.on_no_match_step_id, sb.reference_bitmap_id, sb.x, sb.y
        FROM step_branches sb
        JOIN automation_steps s ON sb.step_id = s.id
        WHERE s.automation_id = $1 AND s.deleted_at IS NULL
        "#,
    )
    .bind(automation_id)
    .fetch_all(db);

    let bitmap_fut = sqlx::query_as::<_, BitmapRow>("SELECT id FROM bitmaps").fetch_all(db);

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
        step_id_to_idx.insert(row.id, idx);
    }

    // Direct subtype mapping into contiguous vector by step index
    let mut subtypes = vec![StepSubtype::Other; n];

    for r in mc_rows {
        if let Some(&idx) = step_id_to_idx.get(&r.step_id) {
            subtypes[idx] = StepSubtype::MouseClick(MouseClickMeta {
                x: r.x,
                y: r.y,
                x_var_id: r.x_variable_id,
                y_var_id: r.y_variable_id,
                region_var_id: r.region_variable_id,
            });
        }
    }

    for r in fp_rows {
        if let Some(&idx) = step_id_to_idx.get(&r.step_id) {
            subtypes[idx] = StepSubtype::FindPixel(FindPixelMeta {
                x: r.x,
                y: r.y,
                output_var_id: r.output_variable_id,
            });
        }
    }

    for r in fb_rows {
        if let Some(&idx) = step_id_to_idx.get(&r.step_id) {
            subtypes[idx] = StepSubtype::FindBitmap(FindBitmapMeta {
                reference_bitmap_id: r.reference_bitmap_id,
                output_found_var_id: r.output_found_variable_id,
                output_x_var_id: r.output_x_variable_id,
                output_y_var_id: r.output_y_variable_id,
                output_region_var_id: r.output_region_variable_id,
                search_x: r.search_x,
                search_y: r.search_y,
            });
        }
    }

    for r in branch_rows {
        if let Some(&idx) = step_id_to_idx.get(&r.step_id) {
            subtypes[idx] = StepSubtype::Branch(BranchMeta {
                condition_type: r.condition_type,
                on_match_step_id: r.on_match_step_id,
                on_no_match_step_id: r.on_no_match_step_id,
                reference_bitmap_id: r.reference_bitmap_id,
                x: r.x,
                y: r.y,
            });
        }
    }

    let existing_bitmaps: HashSet<i64> = bitmap_rows.into_iter().map(|r| r.id).collect();

    // Construct contiguous steps vector
    let steps: Vec<StepNode> = raw_steps
        .into_iter()
        .zip(subtypes)
        .enumerate()
        .map(|(idx, (row, subtype))| StepNode {
            id: row.id,
            step_number: idx + 1,
            step_type: row.step_type,
            subtype,
        })
        .collect();

    // Map variable ID -> index & metadata
    let num_vars = var_rows.len();
    let mut vars_map: HashMap<i64, VarMeta> = HashMap::with_capacity(num_vars);
    for (idx, vr) in var_rows.into_iter().enumerate() {
        vars_map.insert(
            vr.id,
            VarMeta {
                index: idx,
                name: vr.name,
                var_type: vr.var_type,
            },
        );
    }

    let available_params: HashSet<String> = param_rows.into_iter().map(|pr| pr.name).collect();

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
        match &step.subtype {
            StepSubtype::FindPixel(fp) => {
                if let Some(vid) = fp.output_var_id
                    && let Some(v) = vars_map.get(&vid)
                {
                    step_writes[i].set(v.index);
                }
            }
            StepSubtype::FindBitmap(fb) => {
                for vid in [
                    fb.output_found_var_id,
                    fb.output_x_var_id,
                    fb.output_y_var_id,
                    fb.output_region_var_id,
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(v) = vars_map.get(&vid) {
                        step_writes[i].set(v.index);
                    }
                }
            }
            _ => {}
        }
    }

    // 5. Zero-allocation fixed-point variable read-before-write analysis
    let mut step_inits: Vec<VarBitSet> = vec![VarBitSet::all(num_vars); n];
    if !step_inits.is_empty() {
        step_inits[0] = VarBitSet::empty(num_vars); // Start node has 0 variables initialized
    }

    let mut scratch_inits = VarBitSet::empty(num_vars);
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

            // Re-use `scratch_inits` without heap allocations
            scratch_inits.fill_all(num_vars);
            for &p in preds {
                scratch_inits.intersect_with_pred_output(&step_inits[p], &step_writes[p]);
            }

            if scratch_inits != step_inits[i] {
                step_inits[i].copy_from(&scratch_inits);
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
            for (vid_opt, field_label) in [
                (mc.x_var_id, "X coordinate"),
                (mc.y_var_id, "Y coordinate"),
                (mc.region_var_id, "region click"),
            ] {
                if let Some(vid) = vid_opt
                    && let Some(v) = vars_map.get(&vid)
                    && !inits.contains(v.index)
                    && !available_params.contains(&v.name)
                {
                    errors.push(AutomationLintError {
                        step_id: Some(step.id),
                        step_number: Some(step.step_number),
                        message: format!(
                            "Variable '{}' read for {} before being written along all execution paths.",
                            v.name, field_label
                        ),
                    });
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

fn check_coord_bounds(
    val: Option<i32>,
    label: &str,
    sid: i64,
    step_num: usize,
    errors: &mut Vec<AutomationLintError>,
) {
    if let Some(v) = val
        && !(0..=10000).contains(&v)
    {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!("{} {} is outside valid bounds.", label, v),
        });
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
    let Some(v) = vars_map.get(&var_id) else {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!(
                "Referenced {} variable ID {} does not exist.",
                usage_label, var_id
            ),
        });
        return;
    };

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

    let Some(out_vid) = fp.output_var_id else {
        return;
    };

    let Some(v) = vars_map.get(&out_vid) else {
        errors.push(AutomationLintError {
            step_id: Some(sid),
            step_number: Some(step_num),
            message: format!("Output variable ID {} does not exist.", out_vid),
        });
        return;
    };

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

    check_coord_bounds(fb.search_x, "Search X coordinate", sid, step_num, errors);
    check_coord_bounds(fb.search_y, "Search Y coordinate", sid, step_num, errors);

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
    step_id_to_idx: &HashMap<i64, usize>,
    existing_bitmaps: &HashSet<i64>,
    errors: &mut Vec<AutomationLintError>,
) {
    match b.on_match_step_id {
        Some(target_id) if !step_id_to_idx.contains_key(&target_id) => {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: format!(
                    "Branch 'On Match' target step ID {} is invalid or deleted.",
                    target_id
                ),
            });
        }
        None => {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: "Branch step is missing 'On Match' target step.".to_string(),
            });
        }
        _ => {}
    }

    match b.on_no_match_step_id {
        Some(target_id) if !step_id_to_idx.contains_key(&target_id) => {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: format!(
                    "Branch 'On No Match' target step ID {} is invalid or deleted.",
                    target_id
                ),
            });
        }
        None => {
            errors.push(AutomationLintError {
                step_id: Some(sid),
                step_number: Some(step_num),
                message: "Branch step is missing 'On No Match' target step.".to_string(),
            });
        }
        _ => {}
    }

    if b.condition_type == "bitmap" {
        match b.reference_bitmap_id {
            Some(bmp_id) if !existing_bitmaps.contains(&bmp_id) => {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(step_num),
                    message: format!(
                        "Branch condition references bitmap ID {} which does not exist.",
                        bmp_id
                    ),
                });
            }
            None => {
                errors.push(AutomationLintError {
                    step_id: Some(sid),
                    step_number: Some(step_num),
                    message: "Bitmap branch condition is missing reference bitmap.".to_string(),
                });
            }
            _ => {}
        }
    } else if b.condition_type == "pixel_rgb" {
        check_coord_bounds(
            b.x,
            "Pixel branch condition X coordinate",
            sid,
            step_num,
            errors,
        );
        check_coord_bounds(
            b.y,
            "Pixel branch condition Y coordinate",
            sid,
            step_num,
            errors,
        );
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
        let writes = VarBitSet::empty(10);
        b2.set(3);

        b1.intersect_with_pred_output(&b2, &writes);
        assert!(!b1.contains(0));
        assert!(b1.contains(3));
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
        let empty_writes = VarBitSet::empty(num_vars);
        intersected.intersect_with_pred_output(&empty, &empty_writes);
        assert!(!intersected.contains(0));
        assert!(intersected.contains(100));
        assert!(!intersected.contains(101));
    }

    #[test]
    fn test_var_bitset_fill_all_and_copy_from() {
        let num_vars = 100;
        let mut bs = VarBitSet::empty(num_vars);
        assert!(!bs.contains(0));
        assert!(!bs.contains(99));

        bs.fill_all(num_vars);
        assert!(bs.contains(0));
        assert!(bs.contains(99));
        assert!(!bs.contains(100));

        let mut target = VarBitSet::empty(num_vars);
        target.copy_from(&bs);
        assert!(target.contains(0));
        assert!(target.contains(99));
    }

    #[test]
    fn test_validation_helper_functions() {
        // 1. check_coord_bounds
        let mut errors = Vec::new();
        check_coord_bounds(Some(5000), "Valid X", 1, 1, &mut errors);
        assert!(errors.is_empty());

        check_coord_bounds(Some(-1), "Invalid X lower", 1, 1, &mut errors);
        check_coord_bounds(Some(10001), "Invalid X upper", 1, 1, &mut errors);
        assert_eq!(errors.len(), 2);
        assert!(
            errors[0]
                .message
                .contains("Invalid X lower -1 is outside valid bounds")
        );
        assert!(
            errors[1]
                .message
                .contains("Invalid X upper 10001 is outside valid bounds")
        );

        // 2. validate_mouse_click and validate_var_ref
        let mut errors = Vec::new();
        let mut vars_map = HashMap::new();
        vars_map.insert(
            1,
            VarMeta {
                index: 0,
                name: "x_pos".to_string(),
                var_type: "int".to_string(),
            },
        );
        vars_map.insert(
            2,
            VarMeta {
                index: 1,
                name: "invalid_type_var".to_string(),
                var_type: "string".to_string(),
            },
        );

        let mc_valid = MouseClickMeta {
            x: Some(100),
            y: Some(200),
            x_var_id: Some(1),
            y_var_id: None,
            region_var_id: None,
        };
        validate_mouse_click(1, 1, &mc_valid, &vars_map, &mut errors);
        assert!(errors.is_empty());

        let mc_invalid = MouseClickMeta {
            x: Some(-50),
            y: Some(15000),
            x_var_id: Some(2),  // wrong type (expected int, got string)
            y_var_id: Some(99), // non-existent variable ID
            region_var_id: None,
        };
        validate_mouse_click(1, 1, &mc_invalid, &vars_map, &mut errors);
        assert_eq!(errors.len(), 4);
        assert!(
            errors[0]
                .message
                .contains("X coordinate -50 is outside valid bounds")
        );
        assert!(
            errors[1]
                .message
                .contains("Y coordinate 15000 is outside valid bounds")
        );
        assert!(
            errors[2]
                .message
                .contains("is of type 'string', expected 'int'")
        );
        assert!(
            errors[3]
                .message
                .contains("Referenced Y coordinate variable ID 99 does not exist")
        );

        // 3. validate_find_pixel
        let mut errors = Vec::new();
        let fp_valid = FindPixelMeta {
            x: 50,
            y: 50,
            output_var_id: Some(1), // int is valid
        };
        validate_find_pixel(1, 1, &fp_valid, &vars_map, &mut errors);
        assert!(errors.is_empty());

        let fp_invalid = FindPixelMeta {
            x: -10,
            y: 20000,
            output_var_id: Some(99),
        };
        validate_find_pixel(1, 1, &fp_invalid, &vars_map, &mut errors);
        assert_eq!(errors.len(), 2);
        assert!(
            errors[0]
                .message
                .contains("Pixel coordinates (-10, 20000) are outside valid bounds")
        );
        assert!(
            errors[1]
                .message
                .contains("Output variable ID 99 does not exist")
        );

        // 4. validate_find_bitmap and validate_output_var
        let mut errors = Vec::new();
        let mut bitmaps = HashSet::new();
        bitmaps.insert(10);

        let fb_valid = FindBitmapMeta {
            reference_bitmap_id: 10,
            output_found_var_id: None,
            output_x_var_id: Some(1),
            output_y_var_id: None,
            output_region_var_id: None,
            search_x: Some(100),
            search_y: Some(200),
        };
        validate_find_bitmap(1, 1, &fb_valid, &vars_map, &bitmaps, &mut errors);
        assert!(errors.is_empty());

        let fb_invalid = FindBitmapMeta {
            reference_bitmap_id: 999,     // missing bitmap
            output_found_var_id: Some(1), // wrong type (expected bool, got int)
            output_x_var_id: None,
            output_y_var_id: None,
            output_region_var_id: None,
            search_x: Some(-5),
            search_y: Some(12000),
        };
        validate_find_bitmap(1, 1, &fb_invalid, &vars_map, &bitmaps, &mut errors);
        assert_eq!(errors.len(), 4);
        assert!(
            errors[0]
                .message
                .contains("References bitmap ID 999 which does not exist")
        );
        assert!(
            errors[1]
                .message
                .contains("Search X coordinate -5 is outside valid bounds")
        );
        assert!(
            errors[2]
                .message
                .contains("Search Y coordinate 12000 is outside valid bounds")
        );
        assert!(
            errors[3]
                .message
                .contains("'Found' output variable 'x_pos' is of type 'int', expected 'bool'")
        );

        // 5. validate_branch
        let mut errors = Vec::new();
        let mut step_id_to_idx = HashMap::new();
        step_id_to_idx.insert(100, 0);
        step_id_to_idx.insert(101, 1);

        let branch_valid = BranchMeta {
            condition_type: "bitmap".to_string(),
            on_match_step_id: Some(100),
            on_no_match_step_id: Some(101),
            reference_bitmap_id: Some(10),
            x: None,
            y: None,
        };
        validate_branch(1, 1, &branch_valid, &step_id_to_idx, &bitmaps, &mut errors);
        assert!(errors.is_empty());

        let branch_invalid = BranchMeta {
            condition_type: "bitmap".to_string(),
            on_match_step_id: Some(999), // missing target
            on_no_match_step_id: None,   // missing target
            reference_bitmap_id: None,   // missing reference bitmap
            x: None,
            y: None,
        };
        validate_branch(
            1,
            1,
            &branch_invalid,
            &step_id_to_idx,
            &bitmaps,
            &mut errors,
        );
        assert_eq!(errors.len(), 3);
        assert!(
            errors[0]
                .message
                .contains("Branch 'On Match' target step ID 999 is invalid or deleted")
        );
        assert!(
            errors[1]
                .message
                .contains("Branch step is missing 'On No Match' target step")
        );
        assert!(
            errors[2]
                .message
                .contains("Bitmap branch condition is missing reference bitmap")
        );
    }
}
