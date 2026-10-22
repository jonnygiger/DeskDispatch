use crate::api_client::WorkerApiClient;
use crate::config::WorkerConfig;
use crate::engine::*;
use chrono::Utc;
use deskdispatch_protocol::*;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{info, warn};

pub struct StepRunner {
    pub config: WorkerConfig,
    pub api_client: WorkerApiClient,
    pub input: InputController,
    pub variables: HashMap<i64, serde_json::Value>,
    pub bitmap_cache: HashMap<String, CapturedImage>,
    pub task_output_type: Option<String>,
    pub task_output_value: Option<String>,
}

impl StepRunner {
    pub fn new(config: WorkerConfig, api_client: WorkerApiClient) -> Self {
        let input = InputController::new(&config.display);
        Self {
            config,
            api_client,
            input,
            variables: HashMap::new(),
            bitmap_cache: HashMap::new(),
            task_output_type: None,
            task_output_value: None,
        }
    }

    pub async fn execute_automation(
        &mut self,
        task_run_id: i64,
        automation_val: &serde_json::Value,
    ) -> Result<(), String> {
        let payload: FullAutomationPayload = serde_json::from_value(automation_val.clone())
            .map_err(|e| format!("Failed to parse FullAutomationPayload: {}", e))?;

        self.variables.clear();
        self.task_output_type = None;
        self.task_output_value = None;

        info!(
            task_run_id = task_run_id,
            automation_name = %payload.name,
            steps_count = payload.steps.len(),
            "Starting automation execution"
        );

        let mut step_index = 0usize;
        let mut seq = 1i32;

        while step_index < payload.steps.len() {
            let step = &payload.steps[step_index];
            let started_at = Utc::now();

            let (next_index_opt, step_id, result_status, updates, captured_data) = self
                .execute_single_step(task_run_id, step, step_index, &payload.steps)
                .await?;

            let completed_at = Utc::now();

            // Screen capture for step execution result
            let screen = capture_screen(
                &self.config.display,
                self.config.screen_width,
                self.config.screen_height,
            );

            let mut screenshot_object_key = None;
            if let Ok(png_bytes) = screen.encode_png()
                && let Ok(upload_info) =
                    self.api_client.get_screenshot_upload_url(task_run_id).await
            {
                if self
                    .api_client
                    .upload_image_to_presigned_url(&upload_info.upload_url, png_bytes)
                    .await
                    .is_ok()
                {
                    screenshot_object_key = Some(upload_info.object_key.clone());
                    let _ = self
                        .api_client
                        .commit_screenshot(
                            task_run_id,
                            step_id,
                            &upload_info.object_key,
                            self.config.screen_width,
                            self.config.screen_height,
                        )
                        .await;
                }
            }

            // Record step updates into local variable store
            if let Some(ref up_vec) = updates {
                for item in up_vec {
                    self.variables.insert(item.variable_id, item.value.clone());
                }
            }

            let step_req = StepResultRequest {
                seq: Some(seq),
                step_id,
                result: result_status.clone(),
                captured_r: captured_data.captured_r,
                captured_g: captured_data.captured_g,
                captured_b: captured_data.captured_b,
                captured_rgb: captured_data.captured_r.map(|r| {
                    vec![
                        r,
                        captured_data.captured_g.unwrap_or(0),
                        captured_data.captured_b.unwrap_or(0),
                    ]
                }),
                captured_found: captured_data.captured_found,
                captured_x: captured_data.captured_x,
                captured_y: captured_data.captured_y,
                captured_xy: captured_data
                    .captured_x
                    .map(|x| vec![x, captured_data.captured_y.unwrap_or(0)]),
                screenshot_object_key,
                started_at: Some(started_at),
                completed_at: Some(completed_at),
                timestamp: Some(completed_at),
                variable_updates: updates,
            };

            self.api_client
                .post_step_result(task_run_id, &step_req)
                .await?;

            seq += 1;

            if result_status == "failed" {
                self.api_client
                    .complete_task_run(
                        task_run_id,
                        "failed",
                        Some("Step execution failed".to_string()),
                        self.task_output_type.clone(),
                        self.task_output_value.clone(),
                    )
                    .await?;
                return Ok(());
            }

            step_index = match next_index_opt {
                Some(idx) => idx,
                None => step_index + 1,
            };
        }

        self.api_client
            .complete_task_run(
                task_run_id,
                "succeeded",
                None,
                self.task_output_type.clone(),
                self.task_output_value.clone(),
            )
            .await?;

        info!(
            task_run_id = task_run_id,
            "Automation execution completed successfully"
        );
        Ok(())
    }

    async fn execute_single_step(
        &mut self,
        _task_run_id: i64,
        step: &StepPayload,
        _current_index: usize,
        all_steps: &[StepPayload],
    ) -> Result<
        (
            Option<usize>,
            i64,
            String,
            Option<Vec<VariableUpdateItem>>,
            StepCapturedData,
        ),
        String,
    > {
        let mut captured = StepCapturedData::default();
        let mut updates = Vec::new();

        match step {
            StepPayload::MouseClick {
                id,
                post_delay_ms,
                x,
                y,
                x_variable_id,
                y_variable_id,
                region_variable_id,
                button,
                click_type,
                ..
            } => {
                let mut target_x = x.unwrap_or(0);
                let mut target_y = y.unwrap_or(0);

                if let Some(vid) = x_variable_id
                    && let Some(val) = self.variables.get(vid)
                    && let Some(num) = val.as_i64()
                {
                    target_x = num as i32;
                }

                if let Some(vid) = y_variable_id
                    && let Some(val) = self.variables.get(vid)
                    && let Some(num) = val.as_i64()
                {
                    target_y = num as i32;
                }

                if let Some(vid) = region_variable_id
                    && let Some(val) = self.variables.get(vid)
                {
                    if let Some(tl) = val.get("top_left").and_then(|v| v.as_array())
                        && let Some(br) = val.get("bottom_right").and_then(|v| v.as_array())
                    {
                        if tl.len() >= 2 && br.len() >= 2 {
                            let x1 = tl[0].as_i64().unwrap_or(0) as i32;
                            let y1 = tl[1].as_i64().unwrap_or(0) as i32;
                            let x2 = br[0].as_i64().unwrap_or(0) as i32;
                            let y2 = br[1].as_i64().unwrap_or(0) as i32;
                            target_x = (x1 + x2) / 2;
                            target_y = (y1 + y2) / 2;
                        }
                    }
                }

                self.input
                    .mouse_click(target_x, target_y, button, click_type)?;
                captured.captured_x = Some(target_x);
                captured.captured_y = Some(target_y);

                tokio::time::sleep(Duration::from_millis((*post_delay_ms).max(0) as u64)).await;

                Ok((None, *id, "success".to_string(), None, captured))
            }

            StepPayload::WindowControl {
                id,
                post_delay_ms,
                action,
                match_pattern,
                match_type,
                timeout_ms,
                ..
            } => {
                if let Err(e) =
                    self.input
                        .window_control(action, match_pattern, match_type, *timeout_ms)
                {
                    warn!("Window control failed: {}", e);
                    return Ok((None, *id, "failed".to_string(), None, captured));
                }

                tokio::time::sleep(Duration::from_millis((*post_delay_ms).max(0) as u64)).await;

                Ok((None, *id, "success".to_string(), None, captured))
            }

            StepPayload::TypeText {
                id,
                post_delay_ms,
                text_to_type,
                delay_between_keys_ms,
                ..
            } => {
                self.input
                    .type_text(text_to_type, (*delay_between_keys_ms).max(0) as u64)?;

                tokio::time::sleep(Duration::from_millis((*post_delay_ms).max(0) as u64)).await;

                Ok((None, *id, "success".to_string(), None, captured))
            }

            StepPayload::KeyPress {
                id,
                post_delay_ms,
                key_combo,
                ..
            } => {
                self.input.key_press(key_combo)?;

                tokio::time::sleep(Duration::from_millis((*post_delay_ms).max(0) as u64)).await;

                Ok((None, *id, "success".to_string(), None, captured))
            }

            StepPayload::FindPixelRgb {
                id,
                post_delay_ms,
                x,
                y,
                output_variable_id,
                ..
            } => {
                let screen = capture_screen(
                    &self.config.display,
                    self.config.screen_width,
                    self.config.screen_height,
                );

                if let Some((r, g, b)) = screen.get_pixel_rgb(*x as u32, *y as u32) {
                    captured.captured_r = Some(r as i16);
                    captured.captured_g = Some(g as i16);
                    captured.captured_b = Some(b as i16);

                    if let Some(vid) = output_variable_id {
                        updates.push(VariableUpdateItem {
                            variable_id: *vid,
                            value: serde_json::json!([r, g, b]),
                        });
                    }
                }

                tokio::time::sleep(Duration::from_millis((*post_delay_ms).max(0) as u64)).await;

                let updates_opt = if updates.is_empty() {
                    None
                } else {
                    Some(updates)
                };
                Ok((None, *id, "success".to_string(), updates_opt, captured))
            }

            StepPayload::FindBitmap {
                id,
                post_delay_ms,
                reference_bitmap_key,
                search_x,
                search_y,
                search_width,
                search_height,
                match_threshold,
                output_found_variable_id,
                output_x_variable_id,
                output_y_variable_id,
                output_region_variable_id,
                ..
            } => {
                let template_img = match self
                    .get_or_download_bitmap(reference_bitmap_key.as_deref())
                    .await
                {
                    Ok(img) => img,
                    Err(e) => {
                        warn!("Failed to load reference bitmap: {}", e);
                        return Ok((None, *id, "failed".to_string(), None, captured));
                    }
                };

                let screen = capture_screen(
                    &self.config.display,
                    self.config.screen_width,
                    self.config.screen_height,
                );

                let match_res = search_bitmap(
                    &screen,
                    &template_img,
                    *search_x,
                    *search_y,
                    *search_width,
                    *search_height,
                    *match_threshold,
                );

                captured.captured_found = Some(match_res.found);
                captured.captured_x = Some(match_res.x as i32);
                captured.captured_y = Some(match_res.y as i32);

                if let Some(vid) = output_found_variable_id {
                    updates.push(VariableUpdateItem {
                        variable_id: *vid,
                        value: serde_json::json!(match_res.found),
                    });
                }

                if let Some(vid) = output_x_variable_id {
                    updates.push(VariableUpdateItem {
                        variable_id: *vid,
                        value: serde_json::json!(match_res.x),
                    });
                }

                if let Some(vid) = output_y_variable_id {
                    updates.push(VariableUpdateItem {
                        variable_id: *vid,
                        value: serde_json::json!(match_res.y),
                    });
                }

                if let Some(vid) = output_region_variable_id {
                    updates.push(VariableUpdateItem {
                        variable_id: *vid,
                        value: serde_json::json!({
                            "top_left": [match_res.x, match_res.y],
                            "bottom_right": [
                                match_res.x + template_img.width,
                                match_res.y + template_img.height
                            ]
                        }),
                    });
                }

                tokio::time::sleep(Duration::from_millis((*post_delay_ms).max(0) as u64)).await;

                let status = if match_res.found { "success" } else { "failed" };
                let updates_opt = if updates.is_empty() {
                    None
                } else {
                    Some(updates)
                };
                Ok((None, *id, status.to_string(), updates_opt, captured))
            }

            StepPayload::Branch {
                id,
                post_delay_ms,
                condition,
                on_match_step_id,
                on_no_match_step_id,
                ..
            } => {
                let is_match = match condition {
                    BranchConditionPayload::PixelRgb {
                        x,
                        y,
                        expected_r,
                        expected_g,
                        expected_b,
                        tolerance,
                        ..
                    } => {
                        let px = x.unwrap_or(0).max(0) as u32;
                        let py = y.unwrap_or(0).max(0) as u32;

                        let screen = capture_screen(
                            &self.config.display,
                            self.config.screen_width,
                            self.config.screen_height,
                        );

                        if let Some((r, g, b)) = screen.get_pixel_rgb(px, py) {
                            captured.captured_r = Some(r as i16);
                            captured.captured_g = Some(g as i16);
                            captured.captured_b = Some(b as i16);
                            captured.captured_x = Some(px as i32);
                            captured.captured_y = Some(py as i32);

                            let tol = *tolerance;
                            (r as i16 - expected_r).abs() <= tol
                                && (g as i16 - expected_g).abs() <= tol
                                && (b as i16 - expected_b).abs() <= tol
                        } else {
                            false
                        }
                    }

                    BranchConditionPayload::Bitmap {
                        reference_bitmap_key,
                        search_x,
                        search_y,
                        search_width,
                        search_height,
                        match_threshold,
                        ..
                    } => {
                        if let Ok(template_img) = self
                            .get_or_download_bitmap(reference_bitmap_key.as_deref())
                            .await
                        {
                            let screen = capture_screen(
                                &self.config.display,
                                self.config.screen_width,
                                self.config.screen_height,
                            );

                            let match_res = search_bitmap(
                                &screen,
                                &template_img,
                                *search_x,
                                *search_y,
                                *search_width,
                                *search_height,
                                match_threshold.unwrap_or(0.8),
                            );

                            captured.captured_found = Some(match_res.found);
                            captured.captured_x = Some(match_res.x as i32);
                            captured.captured_y = Some(match_res.y as i32);

                            match_res.found
                        } else {
                            false
                        }
                    }

                    BranchConditionPayload::WindowState {
                        match_pattern,
                        match_type,
                        check_type,
                        timeout_ms,
                        ..
                    } => {
                        self.input.check_window_state(
                            match_pattern,
                            match_type,
                            check_type,
                            *timeout_ms,
                        )
                    }
                };

                let target_step_id = if is_match {
                    *on_match_step_id
                } else {
                    *on_no_match_step_id
                };

                let next_step_idx = target_step_id.and_then(|sid| {
                    all_steps.iter().position(|s| match s {
                        StepPayload::MouseClick { id, .. } => *id == sid,
                        StepPayload::KeyPress { id, .. } => *id == sid,
                        StepPayload::TypeText { id, .. } => *id == sid,
                        StepPayload::WindowControl { id, .. } => *id == sid,
                        StepPayload::FindPixelRgb { id, .. } => *id == sid,
                        StepPayload::FindBitmap { id, .. } => *id == sid,
                        StepPayload::Branch { id, .. } => *id == sid,
                        StepPayload::OcrExtract { id, .. } => *id == sid,
                    })
                });

                let status = if is_match {
                    "branch_matched"
                } else {
                    "branch_not_matched"
                };

                tokio::time::sleep(Duration::from_millis((*post_delay_ms).max(0) as u64)).await;

                Ok((next_step_idx, *id, status.to_string(), None, captured))
            }

            StepPayload::OcrExtract {
                id,
                post_delay_ms,
                search_x,
                search_y,
                search_width,
                search_height,
                region_variable_id,
                output_variable_id,
                set_as_task_output,
                ..
            } => {
                let screen = capture_screen(
                    &self.config.display,
                    self.config.screen_width,
                    self.config.screen_height,
                );

                let mut crop_x = search_x.unwrap_or(0).max(0) as u32;
                let mut crop_y = search_y.unwrap_or(0).max(0) as u32;
                let mut crop_w = search_width
                    .unwrap_or(screen.width as i32 - crop_x as i32)
                    .max(1) as u32;
                let mut crop_h = search_height
                    .unwrap_or(screen.height as i32 - crop_y as i32)
                    .max(1) as u32;

                if let Some(vid) = region_variable_id
                    && let Some(val) = self.variables.get(vid)
                {
                    if let Some(tl) = val.get("top_left").and_then(|v| v.as_array())
                        && let Some(br) = val.get("bottom_right").and_then(|v| v.as_array())
                    {
                        if tl.len() >= 2 && br.len() >= 2 {
                            let x1 = tl[0].as_i64().unwrap_or(0).max(0) as u32;
                            let y1 = tl[1].as_i64().unwrap_or(0).max(0) as u32;
                            let x2 = br[0].as_i64().unwrap_or(0).max(0) as u32;
                            let y2 = br[1].as_i64().unwrap_or(0).max(0) as u32;
                            crop_x = x1;
                            crop_y = y1;
                            crop_w = x2.saturating_sub(x1).max(1);
                            crop_h = y2.saturating_sub(y1).max(1);
                        }
                    }
                }

                let cropped = screen.crop(crop_x, crop_y, crop_w, crop_h);
                let ocr_text = perform_ocr_on_captured_image(&cropped);

                if let Some(vid) = output_variable_id {
                    updates.push(VariableUpdateItem {
                        variable_id: *vid,
                        value: serde_json::json!(ocr_text),
                    });
                }

                if *set_as_task_output {
                    self.task_output_type = Some("ocr_text".to_string());
                    self.task_output_value = Some(ocr_text);
                }

                tokio::time::sleep(Duration::from_millis((*post_delay_ms).max(0) as u64)).await;

                let updates_opt = if updates.is_empty() {
                    None
                } else {
                    Some(updates)
                };
                Ok((None, *id, "success".to_string(), updates_opt, captured))
            }
        }
    }

    async fn get_or_download_bitmap(&mut self, key: Option<&str>) -> Result<CapturedImage, String> {
        let key_str = key.ok_or_else(|| "Missing reference bitmap storage key".to_string())?;

        if let Some(cached) = self.bitmap_cache.get(key_str) {
            return Ok(cached.clone());
        }

        let media_url = format!("{}/media/bitmaps/{}", self.config.server_url, key_str);
        let bytes = self.api_client.download_image_bytes(&media_url).await?;

        let img = CapturedImage::decode_png(&bytes)?;
        self.bitmap_cache.insert(key_str.to_string(), img.clone());
        Ok(img)
    }
}

static OCR_TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn perform_ocr_on_captured_image(img: &CapturedImage) -> String {
    if let Ok(png_bytes) = img.encode_png() {
        let temp_dir = std::env::temp_dir();
        let counter = OCR_TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pid = std::process::id();
        let tmp_file = temp_dir.join(format!("deskdispatch_ocr_{}_{}.png", pid, counter));
        if std::fs::write(&tmp_file, &png_bytes).is_ok() {
            let res = if let Ok(timg) = rusty_tesseract::Image::from_path(&tmp_file) {
                rusty_tesseract::image_to_string(&timg, &rusty_tesseract::Args::default()).ok()
            } else {
                None
            };
            let _ = std::fs::remove_file(tmp_file);
            if let Some(txt) = res {
                return txt.trim().to_string();
            }
        }
    }
    String::new()
}

#[derive(Default)]
struct StepCapturedData {
    captured_r: Option<i16>,
    captured_g: Option<i16>,
    captured_b: Option<i16>,
    captured_found: Option<bool>,
    captured_x: Option<i32>,
    captured_y: Option<i32>,
}
