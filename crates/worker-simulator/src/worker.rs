use chrono::Utc;
use deskdispatch_protocol::*;
use reqwest::{Client, StatusCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{error, info, warn};

use crate::image_gen::generate_random_screenshot;
use crate::prng::FastPrng;
use crate::profile::WorkerProfile;

#[inline]
fn random_bool(p: f64) -> bool {
    FastPrng::gen_bool(p)
}

#[inline]
fn random_range_i32(min: i32, max: i32) -> i32 {
    FastPrng::gen_range_i32(min, max)
}

#[inline]
fn random_range_u64(min: u64, max: u64) -> u64 {
    FastPrng::gen_range_u64(min, max)
}

#[inline]
fn random_range_usize(min: usize, max: usize) -> usize {
    FastPrng::gen_range_usize(min, max)
}

#[inline]
fn random_flag() -> bool {
    FastPrng::gen_bool(0.5)
}

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub worker_index: usize,
    pub hostname: String,
    pub display_name: String,
    pub registration_token: Option<String>,
    pub psk: Option<String>,
    pub api_key: Option<String>,
    pub profile: WorkerProfile,
    pub server_url: String,
}

pub struct SimulatedWorker {
    pub config: WorkerConfig,
    pub client: Client,
    pub api_key: Option<String>,
    pub cancel_flag: Arc<AtomicBool>,
    pub current_task_run_id: Arc<tokio::sync::Mutex<Option<i64>>>,
}

impl SimulatedWorker {
    pub fn new(config: WorkerConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());

        let api_key = config.api_key.clone();

        Self {
            config,
            client,
            api_key,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            current_task_run_id: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Primary execution loop for a simulated worker.
    pub async fn run(mut self, stop_signal: Arc<AtomicBool>) {
        info!(
            worker_idx = self.config.worker_index,
            hostname = %self.config.hostname,
            profile = ?self.config.profile,
            "Starting simulated worker"
        );

        // Step 1: Register worker if no API key is set
        if self.api_key.is_none() {
            let mut registered = false;
            if let Some(ref psk) = self.config.psk {
                match self.register_worker(None, Some(psk)).await {
                    Ok(key) => {
                        info!(
                            worker_idx = self.config.worker_index,
                            "Unattended worker registered successfully via PSK"
                        );
                        self.api_key = Some(key);
                        registered = true;
                    }
                    Err(e) => {
                        warn!(
                            worker_idx = self.config.worker_index,
                            "Failed to register worker via PSK: {}. Will attempt token registration...",
                            e
                        );
                    }
                }
            }

            if !registered {
                if let Some(ref reg_token) = self.config.registration_token {
                    match self.register_worker(Some(reg_token), None).await {
                        Ok(key) => {
                            info!(
                                worker_idx = self.config.worker_index,
                                "Registered successfully with API key"
                            );
                            self.api_key = Some(key);
                        }
                        Err(e) => {
                            error!(
                                worker_idx = self.config.worker_index,
                                "Failed to register worker with token: {}", e
                            );
                            return;
                        }
                    }
                } else {
                    error!(
                        worker_idx = self.config.worker_index,
                        "Worker has neither API key, PSK, nor registration token. Exiting."
                    );
                    return;
                }
            }
        }

        let api_key = match self.api_key.clone() {
            Some(k) => k,
            None => return,
        };

        // Step 2: Spawn background heartbeat loop
        let hb_worker_idx = self.config.worker_index;
        let hb_client = self.client.clone();
        let hb_api_key = api_key.clone();
        let hb_server_url = self.config.server_url.clone();
        let hb_profile = self.config.profile;
        let hb_cancel_flag = self.cancel_flag.clone();
        let hb_current_task = self.current_task_run_id.clone();
        let hb_stop_signal = stop_signal.clone();

        tokio::spawn(async move {
            let mut hb_interval = tokio::time::interval(Duration::from_secs(15));
            loop {
                hb_interval.tick().await;

                if hb_stop_signal.load(Ordering::Relaxed) {
                    break;
                }

                // Simulating power outage profile: abruptly terminate heartbeats after a delay
                if hb_profile == WorkerProfile::PowerOutage && random_bool(0.15) {
                    warn!(
                        worker_idx = hb_worker_idx,
                        "Simulating POWER OUTAGE! Heartbeat stopping abruptly."
                    );
                    break;
                }

                if hb_profile == WorkerProfile::LaggyNetwork {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }

                let task_run_id = *hb_current_task.lock().await;

                let hb_req = HeartbeatRequest {
                    status: Some("online".to_string()),
                    current_task_run_id: task_run_id,
                    screen_width: Some(1920),
                    screen_height: Some(1080),
                    os_info: Some(hb_profile.os_info()),
                    agent_version: Some(hb_profile.agent_version().to_string()),
                };

                let url = format!("{}/api/v1/workers/heartbeat", hb_server_url);
                let res = hb_client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", hb_api_key))
                    .json(&hb_req)
                    .send()
                    .await;

                match res {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(hb_resp) = resp.json::<HeartbeatResponse>().await
                            && hb_resp.cancel_requested
                        {
                            hb_cancel_flag.store(true, Ordering::Relaxed);
                            warn!(
                                worker_idx = hb_worker_idx,
                                "Server requested cancellation of active task"
                            );
                        }
                    }
                    Ok(resp) if resp.status() == StatusCode::UPGRADE_REQUIRED => {
                        warn!(
                            worker_idx = hb_worker_idx,
                            "Server returned 426 Upgrade Required (Outdated Agent Version)"
                        );
                        break;
                    }
                    Ok(resp) => {
                        warn!(
                            worker_idx = hb_worker_idx,
                            status = %resp.status(),
                            "Heartbeat failed"
                        );
                    }
                    Err(e) => {
                        error!(worker_idx = hb_worker_idx, "Heartbeat request error: {}", e);
                    }
                }
            }
        });

        // Step 3: Main polling loop
        loop {
            if stop_signal.load(Ordering::Relaxed) {
                info!(
                    worker_idx = self.config.worker_index,
                    "Stop signal received. Worker shutting down."
                );
                break;
            }

            if self.config.profile == WorkerProfile::PowerOutage && random_bool(0.2) {
                warn!(
                    worker_idx = self.config.worker_index,
                    "Simulating POWER OUTAGE / DISCONNECT during main poll loop!"
                );
                break;
            }

            if self.config.profile == WorkerProfile::LaggyNetwork {
                let extra_delay = random_range_u64(2, 8);
                tokio::time::sleep(Duration::from_secs(extra_delay)).await;
            }

            let next_url = format!("{}/api/v1/workers/next-assignment", self.config.server_url);
            let poll_res = self
                .client
                .get(&next_url)
                .header("Authorization", format!("Bearer {}", api_key))
                .send()
                .await;

            match poll_res {
                Ok(resp) if resp.status().is_success() => {
                    match resp.json::<NextAssignmentResponse>().await {
                        Ok(NextAssignmentResponse::None) => {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                        Ok(NextAssignmentResponse::ExecuteAutomation {
                            task_run_id,
                            automation,
                        }) => {
                            info!(
                                worker_idx = self.config.worker_index,
                                task_run_id = task_run_id,
                                "Received automation task execution assignment"
                            );
                            *self.current_task_run_id.lock().await = Some(task_run_id);
                            self.cancel_flag.store(false, Ordering::Relaxed);

                            let exec_res = self
                                .execute_automation(task_run_id, &automation, &api_key)
                                .await;
                            *self.current_task_run_id.lock().await = None;

                            if let Err(e) = exec_res {
                                error!(
                                    worker_idx = self.config.worker_index,
                                    task_run_id = task_run_id,
                                    "Error executing automation: {}",
                                    e
                                );
                            }
                        }
                        Ok(NextAssignmentResponse::StartRecording {
                            recording_session_id,
                        }) => {
                            info!(
                                worker_idx = self.config.worker_index,
                                recording_session_id = recording_session_id,
                                "Received start recording assignment"
                            );
                            let rec_res =
                                self.execute_recording(recording_session_id, &api_key).await;
                            if let Err(e) = rec_res {
                                error!(
                                    worker_idx = self.config.worker_index,
                                    recording_session_id = recording_session_id,
                                    "Error executing recording: {}",
                                    e
                                );
                            }
                        }
                        Err(e) => {
                            error!(
                                worker_idx = self.config.worker_index,
                                "Failed to parse NextAssignmentResponse: {}", e
                            );
                            tokio::time::sleep(Duration::from_secs(3)).await;
                        }
                    }
                }
                Ok(resp) => {
                    warn!(
                        worker_idx = self.config.worker_index,
                        status = %resp.status(),
                        "Next assignment poll returned non-200"
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Err(e) => {
                    error!(
                        worker_idx = self.config.worker_index,
                        "Next assignment poll network error: {}", e
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn register_worker(
        &self,
        token: Option<&str>,
        psk: Option<&str>,
    ) -> Result<String, String> {
        let url = format!("{}/api/v1/workers/register", self.config.server_url);
        let req = RegisterWorkerRequest {
            registration_token: token.map(|s| s.to_string()),
            token: None,
            psk: psk.map(|s| s.to_string()),
            pre_shared_key: None,
            hostname: Some(self.config.hostname.clone()),
            display_name: Some(self.config.display_name.clone()),
            mac_address: None,
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("HTTP request error: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Registration HTTP error status {}", resp.status()));
        }

        let reg_resp = resp
            .json::<RegisterWorkerResponse>()
            .await
            .map_err(|e| format!("Failed to parse register response: {}", e))?;

        Ok(reg_resp.api_key)
    }

    pub async fn fetch_task_run(
        &self,
        task_run_id: i64,
        api_key: &str,
    ) -> Result<TaskRunResponse, String> {
        let url = format!(
            "{}/api/v1/workers/task-runs/{}",
            self.config.server_url, task_run_id
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch task run: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Fetch task run HTTP status {}", resp.status()));
        }

        resp.json::<TaskRunResponse>()
            .await
            .map_err(|e| format!("Failed to parse TaskRunResponse: {}", e))
    }

    async fn execute_automation(
        &self,
        task_run_id: i64,
        automation: &serde_json::Value,
        api_key: &str,
    ) -> Result<(), String> {
        let empty_steps = Vec::new();
        let steps = automation
            .get("steps")
            .and_then(|s| s.as_array())
            .unwrap_or(&empty_steps);

        let mut overall_success = true;

        for (seq, step) in (1..).zip(steps.iter()) {
            if self.cancel_flag.load(Ordering::Relaxed) {
                warn!(
                    worker_idx = self.config.worker_index,
                    task_run_id = task_run_id,
                    "Task execution cancelled due to cancel request"
                );
                overall_success = false;
                break;
            }

            // Power outage profile during task step
            if self.config.profile == WorkerProfile::PowerOutage && random_bool(0.25) {
                warn!(
                    worker_idx = self.config.worker_index,
                    task_run_id = task_run_id,
                    "Simulating POWER OUTAGE mid-step!"
                );
                return Err("Simulated power outage mid-step".to_string());
            }

            let step_id = step
                .get("id")
                .and_then(|i| i.as_i64())
                .ok_or_else(|| "Missing step id in automation step".to_string())?;

            let step_type = step
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");

            let post_delay_ms = step
                .get("post_delay_ms")
                .and_then(|d| d.as_u64())
                .unwrap_or(100);

            info!(
                worker_idx = self.config.worker_index,
                task_run_id = task_run_id,
                seq = seq,
                step_id = step_id,
                step_type = step_type,
                "Executing automation step"
            );

            let started_at = Utc::now();
            let typing_delay = if step_type == "type_text" {
                let key_delay = step
                    .get("delay_between_keys_ms")
                    .and_then(|d| d.as_u64())
                    .unwrap_or(0);
                let char_count = step
                    .get("text_to_type")
                    .and_then(|t| t.as_str())
                    .map(|s| s.chars().count())
                    .unwrap_or(0);
                (key_delay * char_count as u64).min(500)
            } else {
                0
            };
            tokio::time::sleep(Duration::from_millis(
                post_delay_ms.min(1000) + typing_delay,
            ))
            .await;

            // Determine step result based on profile
            let (result_str, captured_r, captured_g, captured_b, captured_x, captured_y, found) =
                match self.config.profile {
                    WorkerProfile::Flaky if random_bool(0.3) => (
                        "failed",
                        Some(FastPrng::gen_range_u8_inclusive(0, 255) as i16),
                        Some(FastPrng::gen_range_u8_inclusive(0, 255) as i16),
                        Some(FastPrng::gen_range_u8_inclusive(0, 255) as i16),
                        Some(random_range_i32(0, 1920)),
                        Some(random_range_i32(0, 1080)),
                        Some(false),
                    ),
                    _ => match step_type {
                        "branch" => {
                            let matched = random_flag();
                            let res = if matched {
                                "branch_matched"
                            } else {
                                "branch_not_matched"
                            };
                            (
                                res,
                                Some(120),
                                Some(180),
                                Some(200),
                                Some(400),
                                Some(300),
                                Some(matched),
                            )
                        }
                        "window_control" => (
                            "success",
                            Some(100),
                            Some(150),
                            Some(200),
                            Some(0),
                            Some(0),
                            Some(true),
                        ),
                        _ => (
                            "success",
                            Some(FastPrng::gen_range_u8_inclusive(0, 255) as i16),
                            Some(FastPrng::gen_range_u8_inclusive(0, 255) as i16),
                            Some(FastPrng::gen_range_u8_inclusive(0, 255) as i16),
                            Some(random_range_i32(0, 1920)),
                            Some(random_range_i32(0, 1080)),
                            Some(true),
                        ),
                    },
                };

            if result_str == "failed" {
                overall_success = false;
            }

            // Generate & upload screenshot for every step execution
            let mut screenshot_object_key = None;
            match self
                .upload_task_run_screenshot(task_run_id, step_id, api_key)
                .await
            {
                Ok(key) => {
                    screenshot_object_key = Some(key);
                }
                Err(e) => {
                    warn!(
                        worker_idx = self.config.worker_index,
                        "Failed to upload step screenshot: {}", e
                    );
                }
            }

            // Periodically upload a reference bitmap for bitmap steps if needed
            if (step_type == "find_bitmap" || step_type == "branch") && random_bool(0.2) {
                let auto_id = automation.get("id").and_then(|i| i.as_i64());
                let bitmap_name = format!("sim_ref_{}_step_{}", self.config.worker_index, step_id);
                let _ = self
                    .upload_reference_bitmap(auto_id, &bitmap_name, api_key)
                    .await;
            }

            // Rich variable updates covering region, color, coordinate, and status variable types
            let mut var_updates = Vec::new();

            if let Some(var_id) = step.get("output_variable_id").and_then(|v| v.as_i64()) {
                let var_val = match step_type {
                    "find_pixel_rgb" => serde_json::json!([
                        captured_r.unwrap_or(200),
                        captured_g.unwrap_or(150),
                        captured_b.unwrap_or(100)
                    ]),
                    _ => serde_json::json!({
                        "x": captured_x.unwrap_or(800),
                        "y": captured_y.unwrap_or(600),
                        "status": result_str
                    }),
                };
                var_updates.push(VariableUpdateItem {
                    variable_id: var_id,
                    value: var_val,
                });
            }

            if let Some(var_id) = step
                .get("output_found_variable_id")
                .and_then(|v| v.as_i64())
            {
                var_updates.push(VariableUpdateItem {
                    variable_id: var_id,
                    value: serde_json::json!(found.unwrap_or(true)),
                });
            }

            if let Some(var_id) = step.get("output_x_variable_id").and_then(|v| v.as_i64()) {
                var_updates.push(VariableUpdateItem {
                    variable_id: var_id,
                    value: serde_json::json!(captured_x.unwrap_or(400)),
                });
            }

            if let Some(var_id) = step.get("output_y_variable_id").and_then(|v| v.as_i64()) {
                var_updates.push(VariableUpdateItem {
                    variable_id: var_id,
                    value: serde_json::json!(captured_y.unwrap_or(300)),
                });
            }

            if let Some(var_id) = step
                .get("output_region_variable_id")
                .and_then(|v| v.as_i64())
            {
                let x = captured_x.unwrap_or(400);
                let y = captured_y.unwrap_or(300);
                var_updates.push(VariableUpdateItem {
                    variable_id: var_id,
                    value: serde_json::json!({
                        "top_left": [x, y],
                        "bottom_right": [x + 120, y + 80]
                    }),
                });
            }

            let variable_updates = if var_updates.is_empty() {
                None
            } else {
                Some(var_updates)
            };

            let completed_at = Utc::now();

            let step_req = StepResultRequest {
                seq: Some(seq),
                step_id,
                result: result_str.to_string(),
                captured_r,
                captured_g,
                captured_b,
                captured_rgb: None,
                captured_found: found,
                captured_x,
                captured_y,
                captured_xy: None,
                screenshot_object_key,
                started_at: Some(started_at),
                completed_at: Some(completed_at),
                timestamp: Some(completed_at),
                variable_updates,
            };

            let result_url = format!(
                "{}/api/v1/workers/task-runs/{}/step-result",
                self.config.server_url, task_run_id
            );

            let res = self
                .client
                .post(&result_url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&step_req)
                .send()
                .await;

            match res {
                Ok(resp) if resp.status().is_success() => {
                    info!(
                        worker_idx = self.config.worker_index,
                        task_run_id = task_run_id,
                        step_id = step_id,
                        result = result_str,
                        "Successfully reported step result"
                    );
                }
                Ok(resp) => {
                    warn!(
                        worker_idx = self.config.worker_index,
                        task_run_id = task_run_id,
                        step_id = step_id,
                        status = %resp.status(),
                        "Posting step result returned non-200 status"
                    );
                }
                Err(e) => {
                    error!(
                        worker_idx = self.config.worker_index,
                        task_run_id = task_run_id,
                        step_id = step_id,
                        "Failed to post step result: {}",
                        e
                    );
                }
            }
        }

        // Complete task run
        let complete_url = format!(
            "{}/api/v1/workers/task-runs/{}/complete",
            self.config.server_url, task_run_id
        );

        let final_status = if overall_success {
            "succeeded"
        } else {
            "failed"
        };

        let complete_req = CompleteTaskRunRequest {
            status: final_status.to_string(),
            error_message: if overall_success {
                None
            } else {
                Some("Simulated execution step failure".to_string())
            },
        };

        let complete_res = self
            .client
            .post(&complete_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&complete_req)
            .send()
            .await;

        match complete_res {
            Ok(resp) if resp.status().is_success() => {
                info!(
                    worker_idx = self.config.worker_index,
                    task_run_id = task_run_id,
                    final_status = final_status,
                    "Successfully reported task run completion"
                );
            }
            Ok(resp) => {
                warn!(
                    worker_idx = self.config.worker_index,
                    task_run_id = task_run_id,
                    final_status = final_status,
                    status = %resp.status(),
                    "Complete task run returned non-200 status"
                );
            }
            Err(e) => {
                error!(
                    worker_idx = self.config.worker_index,
                    task_run_id = task_run_id,
                    "Failed to send complete task run request: {}",
                    e
                );
            }
        }

        info!(
            worker_idx = self.config.worker_index,
            task_run_id = task_run_id,
            final_status = final_status,
            "Completed task run execution"
        );

        Ok(())
    }

    async fn put_image_with_failsafes(
        &self,
        upload_url: &str,
        image_bytes: Vec<u8>,
    ) -> Result<(), String> {
        let (target_url, host_header) = if self.config.server_url.contains("http://app:")
            || self.config.server_url.contains("http://deskdispatch-app:")
        {
            let host_hdr = url::Url::parse(upload_url).ok().and_then(|u| {
                u.host_str().map(|h| match u.port() {
                    Some(p) => format!("{}:{}", h, p),
                    None => h.to_string(),
                })
            });
            let rewritten = upload_url
                .replace("http://localhost:9000", "http://rustfs:9000")
                .replace("http://127.0.0.1:9000", "http://rustfs:9000");
            (rewritten, host_hdr)
        } else {
            (upload_url.to_string(), None)
        };

        let mut req = self
            .client
            .put(&target_url)
            .header("Content-Type", "image/png");

        if let Some(ref h) = host_header {
            req = req.header("Host", h);
        }

        let res = req.body(image_bytes.clone()).send().await;

        match res {
            Ok(resp) if resp.status().is_success() => Ok(()),
            Ok(resp)
                if resp.status() == StatusCode::FORBIDDEN
                    || resp.status() == StatusCode::BAD_REQUEST =>
            {
                let mut retry_req = self.client.put(&target_url);
                if let Some(ref h) = host_header {
                    retry_req = retry_req.header("Host", h);
                }
                let retry_res = retry_req
                    .body(image_bytes)
                    .send()
                    .await
                    .map_err(|e| format!("Failed PUT retry without Content-Type: {}", e))?;

                if retry_res.status().is_success() {
                    Ok(())
                } else {
                    Err(format!(
                        "PUT image upload returned status {} on retry",
                        retry_res.status()
                    ))
                }
            }
            Ok(resp) => Err(format!(
                "PUT image upload returned status {}",
                resp.status()
            )),
            Err(e) => Err(format!("Failed to PUT image: {}", e)),
        }
    }

    pub async fn upload_reference_bitmap(
        &self,
        automation_id: Option<i64>,
        bitmap_name: &str,
        api_key: &str,
    ) -> Result<(i64, String), String> {
        let url = format!(
            "{}/api/v1/workers/bitmaps/upload-url",
            self.config.server_url
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
            .map_err(|e| format!("Failed to get bitmap upload URL: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Bitmap upload URL returned status {}",
                resp.status()
            ));
        }

        let upload_info = resp
            .json::<ScreenshotUploadUrlResponse>()
            .await
            .map_err(|e| format!("Failed to parse upload URL response: {}", e))?;

        let png_bytes = generate_random_screenshot();

        self.put_image_with_failsafes(&upload_info.upload_url, png_bytes)
            .await?;

        let commit_url = format!("{}/api/v1/workers/bitmaps/commit", self.config.server_url);

        let commit_req = CommitBitmapRequest {
            automation_id,
            name: bitmap_name.to_string(),
            object_key: upload_info.object_key.clone(),
            width: Some(100),
            height: Some(100),
        };

        let commit_resp = self
            .client
            .post(&commit_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&commit_req)
            .send()
            .await
            .map_err(|e| format!("Failed to commit bitmap upload: {}", e))?;

        if !commit_resp.status().is_success() {
            return Err(format!(
                "Commit bitmap returned status {}",
                commit_resp.status()
            ));
        }

        let commit_info = commit_resp
            .json::<CommitBitmapResponse>()
            .await
            .map_err(|e| format!("Failed to parse commit bitmap response: {}", e))?;

        Ok((commit_info.bitmap_id, commit_info.object_key))
    }

    async fn upload_task_run_screenshot(
        &self,
        task_run_id: i64,
        step_id: i64,
        api_key: &str,
    ) -> Result<String, String> {
        let url = format!(
            "{}/api/v1/workers/task-runs/{}/screenshot-upload-url",
            self.config.server_url, task_run_id
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
            .map_err(|e| format!("Failed to get screenshot upload URL: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Screenshot upload URL returned status {}",
                resp.status()
            ));
        }

        let upload_info = resp
            .json::<ScreenshotUploadUrlResponse>()
            .await
            .map_err(|e| format!("Failed to parse upload URL response: {}", e))?;

        let png_bytes = generate_random_screenshot();

        self.put_image_with_failsafes(&upload_info.upload_url, png_bytes)
            .await?;

        let commit_url = format!(
            "{}/api/v1/workers/task-runs/{}/screenshots/commit",
            self.config.server_url, task_run_id
        );

        let commit_req = CommitScreenshotRequest {
            step_id: Some(step_id),
            object_key: Some(upload_info.object_key.clone()),
            width: Some(1920),
            height: Some(1080),
        };

        let commit_res = self
            .client
            .post(&commit_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&commit_req)
            .send()
            .await;

        if let Ok(resp) = commit_res {
            if !resp.status().is_success() {
                warn!(
                    worker_idx = self.config.worker_index,
                    task_run_id = task_run_id,
                    step_id = step_id,
                    status = %resp.status(),
                    "Screenshot commit endpoint returned non-200 status"
                );
            }
        } else if let Err(e) = commit_res {
            warn!(
                worker_idx = self.config.worker_index,
                task_run_id = task_run_id,
                step_id = step_id,
                "Failed to call screenshot commit endpoint: {}",
                e
            );
        }

        Ok(upload_info.object_key)
    }

    async fn upload_recording_screenshot(
        &self,
        recording_session_id: i64,
        _sequence_number: i32,
        api_key: &str,
    ) -> Result<String, String> {
        let url = format!(
            "{}/api/v1/workers/recordings/{}/screenshot-upload-url",
            self.config.server_url, recording_session_id
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
            .map_err(|e| format!("Failed to get recording screenshot upload URL: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Recording screenshot upload URL returned status {}",
                resp.status()
            ));
        }

        let upload_info = resp
            .json::<ScreenshotUploadUrlResponse>()
            .await
            .map_err(|e| format!("Failed to parse upload URL response: {}", e))?;

        let png_bytes = generate_random_screenshot();

        self.put_image_with_failsafes(&upload_info.upload_url, png_bytes)
            .await?;

        Ok(upload_info.object_key)
    }

    async fn execute_recording(
        &self,
        recording_session_id: i64,
        api_key: &str,
    ) -> Result<(), String> {
        let event_types = ["mouse_click", "key_press", "find_pixel_rgb", "find_bitmap"];
        let key_combos = ["ctrl+c", "ctrl+v", "enter", "tab", "backspace", "escape"];
        let buttons = ["left", "right", "double"];

        for sequence_number in 1..=6 {
            let event_url = format!(
                "{}/api/v1/workers/recordings/{}/events",
                self.config.server_url, recording_session_id
            );

            let mut screenshot_object_key = None;
            match self
                .upload_recording_screenshot(recording_session_id, sequence_number, api_key)
                .await
            {
                Ok(key) => {
                    screenshot_object_key = Some(key);
                }
                Err(e) => {
                    warn!(
                        worker_idx = self.config.worker_index,
                        recording_session_id = recording_session_id,
                        "Failed to upload recording event screenshot: {}",
                        e
                    );
                }
            }

            let ev_type = event_types[random_range_usize(0, event_types.len())];
            let (x_opt, y_opt, btn_opt, key_opt) = match ev_type {
                "key_press" => (
                    None,
                    None,
                    None,
                    Some(key_combos[random_range_usize(0, key_combos.len())].to_string()),
                ),
                _ => (
                    Some(random_range_i32(50, 1800)),
                    Some(random_range_i32(50, 1000)),
                    Some(buttons[random_range_usize(0, buttons.len())].to_string()),
                    None,
                ),
            };

            let event_item = RecordingEventItem {
                sequence_number,
                event_type: ev_type.to_string(),
                x: x_opt,
                y: y_opt,
                button: btn_opt,
                key_combo: key_opt,
                screenshot_object_key,
                captured_at: Some(Utc::now()),
            };

            let req = PostRecordingEventsRequest {
                events: vec![event_item],
            };

            let resp = self
                .client
                .post(&event_url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&req)
                .send()
                .await
                .map_err(|e| format!("Failed to post recording events: {}", e))?;

            if resp.status().is_success()
                && let Ok(rec_resp) = resp.json::<PostRecordingEventsResponse>().await
                && rec_resp.stop_requested
            {
                break;
            }

            tokio::time::sleep(Duration::from_millis(400)).await;
        }

        let stop_url = format!(
            "{}/api/v1/workers/recordings/{}/stop",
            self.config.server_url, recording_session_id
        );

        let _ = self
            .client
            .post(&stop_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulated_worker_initialization() {
        let config = WorkerConfig {
            worker_index: 1,
            hostname: "sim-worker-1.local".to_string(),
            display_name: "Simulated Worker 1".to_string(),
            registration_token: Some("dd_reg_test123".to_string()),
            psk: None,
            api_key: Some("dd_pk_testkey123".to_string()),
            profile: WorkerProfile::Normal,
            server_url: "http://127.0.0.1:3000".to_string(),
        };

        let worker = SimulatedWorker::new(config.clone());
        assert_eq!(worker.config.worker_index, 1);
        assert_eq!(worker.api_key.as_deref(), Some("dd_pk_testkey123"));
        assert!(!worker.cancel_flag.load(Ordering::Relaxed));
    }

    #[test]
    fn test_simulated_worker_fetch_task_run_url_formatting() {
        let config = WorkerConfig {
            worker_index: 2,
            hostname: "sim-worker-2.local".to_string(),
            display_name: "Simulated Worker 2".to_string(),
            registration_token: None,
            psk: None,
            api_key: Some("dd_pk_testkey456".to_string()),
            profile: WorkerProfile::Normal,
            server_url: "http://127.0.0.1:3000".to_string(),
        };

        let worker = SimulatedWorker::new(config);
        let task_run_id = 99;
        let url = format!(
            "{}/api/v1/workers/task-runs/{}",
            worker.config.server_url, task_run_id
        );
        assert_eq!(url, "http://127.0.0.1:3000/api/v1/workers/task-runs/99");
    }
}
