#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

fn extract_input_value(html: &str, field_name: &str) -> Option<String> {
    let pattern = format!(r#"name="{}" value=""#, field_name);
    if let Some(pos) = html.find(&pattern) {
        let start = pos + pattern.len();
        if let Some(end) = html[start..].find('"') {
            return Some(html[start..start + end].to_string());
        }
    }
    None
}

fn extract_form_action_and_inputs(html: &str) -> Option<(String, Vec<(String, String)>)> {
    let target_header = "Upload Reference Image File";
    let section_start = html.find(target_header)?;
    let form_start = html[section_start..].find("<form ")? + section_start;
    let form_end = html[form_start..].find("</form>")? + form_start;
    let form_str = &html[form_start..form_end];

    let action_pat = r#"action=""#;
    let action_pos = form_str.find(action_pat)?;
    let action_start = action_pos + action_pat.len();
    let action_end = form_str[action_start..].find('"')?;
    let action_url = form_str[action_start..action_start + action_end].to_string();

    let mut fields = Vec::new();
    let mut search_idx = 0;
    while let Some(pos) = form_str[search_idx..].find("<input ") {
        let abs_pos = search_idx + pos;
        let tag_end = form_str[abs_pos..]
            .find('>')
            .map(|p| abs_pos + p)
            .unwrap_or(form_str.len());
        let tag_str = &form_str[abs_pos..tag_end];

        if tag_str.contains("type=\"hidden\"")
            && let (Some(name), Some(val)) = (
                extract_attr_value(tag_str, "name"),
                extract_attr_value(tag_str, "value"),
            )
            && name != "csrf_token"
        {
            fields.push((name, val));
        }

        search_idx = tag_end + 1;
    }

    Some((action_url, fields))
}

fn extract_attr_value(tag_str: &str, attr_name: &str) -> Option<String> {
    let pattern = format!(r#"{}=""#, attr_name);
    if let Some(pos) = tag_str.find(&pattern) {
        let start = pos + pattern.len();
        if let Some(end) = tag_str[start..].find('"') {
            return Some(tag_str[start..start + end].to_string());
        }
    }
    None
}

fn create_sample_png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        let g = ((y * 255) / height.max(1)) as u8;
        for x in 0..width {
            let r = ((x * 255) / width.max(1)) as u8;
            pixels.push(r);
            pixels.push(g);
            pixels.push(200);
        }
    }
    deskdispatch::routes::bitmaps::encode_png(width, height, &pixels)
}

fn dump_docker_logs() {
    println!("\n=================== DOCKER PROCESSES ===================");
    if let Ok(ps_out) = Command::new("docker").args(["compose", "ps"]).output() {
        println!("{}", String::from_utf8_lossy(&ps_out.stdout));
    }

    println!("\n=================== DOCKER LOGS DUMP START ===================");
    let output = Command::new("docker")
        .args(["compose", "logs", "--tail=200"])
        .output();

    match output {
        Ok(out) => {
            println!("--- STDOUT ---");
            println!("{}", String::from_utf8_lossy(&out.stdout));
            println!("--- STDERR ---");
            println!("{}", String::from_utf8_lossy(&out.stderr));
        }
        Err(e) => {
            println!("Failed to run `docker compose logs`: {}", e);
        }
    }
    println!("=================== DOCKER LOGS DUMP END ===================\n");
}

struct DockerLogGuard;

impl Drop for DockerLogGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            dump_docker_logs();
        }
    }
}

async fn wait_for_stack_ready(client: &reqwest::Client, base_url: &str) {
    let ready_endpoint = format!("{}/readyz", base_url);
    let start_time = Instant::now();
    let timeout = Duration::from_secs(300);
    let mut is_ready = false;
    let mut poll_attempts = 0;

    println!("[E2E TEST] Waiting for readyz endpoint: {}", ready_endpoint);
    while start_time.elapsed() < timeout {
        poll_attempts += 1;
        if let Ok(resp) = client.get(&ready_endpoint).send().await {
            if resp.status().is_success() {
                println!(
                    "[E2E TEST] /readyz reported healthy status on attempt {}!",
                    poll_attempts
                );
                is_ready = true;
                break;
            } else {
                println!(
                    "[E2E TEST] /readyz status {} on attempt {}...",
                    resp.status(),
                    poll_attempts
                );
            }
        } else {
            println!(
                "[E2E TEST] Polling /readyz connection attempt {} failed...",
                poll_attempts
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    if !is_ready {
        dump_docker_logs();
    }

    assert!(
        is_ready,
        "Docker container stack failed to report healthy readyz status within timeout"
    );
    println!(
        "[E2E TEST] Stack ready after {:.1}s",
        start_time.elapsed().as_secs_f64()
    );
}

async fn login_as_admin(client: &reqwest::Client, base_url: &str) -> String {
    let login_url = format!("{}/login", base_url);

    let mut login_resp = client
        .post(&login_url)
        .form(&[("username", "admin"), ("password", "NewAdminPassword123!")])
        .send()
        .await
        .expect("Failed to submit login request with updated password");

    if login_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        println!(
            "[E2E Auth] Credentials 'NewAdminPassword123!' rejected, logging in with initial default password 'test'..."
        );
        login_resp = client
            .post(&login_url)
            .form(&[("username", "admin"), ("password", "test")])
            .send()
            .await
            .expect("Failed to submit login request with default password");
    }

    assert_eq!(
        login_resp.status(),
        reqwest::StatusCode::SEE_OTHER,
        "Login request expected 303 See Other redirect, got status: {}",
        login_resp.status()
    );

    let cookie_header = login_resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .find_map(|h| h.to_str().ok().filter(|s| s.contains("session_id")))
        .expect("Set-Cookie header with session_id missing on login response");

    let mut session_cookie = cookie_header
        .split(';')
        .next()
        .expect("Invalid cookie format")
        .to_string();

    // Check if redirected to forced password change page
    let password_url = format!("{}/account/password", base_url);
    let pwd_page_resp = client
        .get(&password_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to GET /account/password");

    if pwd_page_resp.status() == reqwest::StatusCode::OK {
        println!("[E2E Auth] Forced password change active. Updating admin password...");
        let pwd_page_html = pwd_page_resp
            .text()
            .await
            .expect("Failed to read password change page HTML");

        if let Some(pwd_csrf_token) = extract_input_value(&pwd_page_html, "csrf_token") {
            let mut change_pwd_form = HashMap::new();
            change_pwd_form.insert("csrf_token", pwd_csrf_token.as_str());
            change_pwd_form.insert("current_password", "test");
            change_pwd_form.insert("new_password", "NewAdminPassword123!");
            change_pwd_form.insert("confirm_password", "NewAdminPassword123!");

            let change_pwd_resp = client
                .post(&password_url)
                .header(reqwest::header::COOKIE, &session_cookie)
                .form(&change_pwd_form)
                .send()
                .await
                .expect("Failed to submit password change form");

            assert!(
                change_pwd_resp.status().is_success()
                    || change_pwd_resp.status() == reqwest::StatusCode::SEE_OTHER,
                "Password change submission expected 200 OK or 303 Redirect, got: {}",
                change_pwd_resp.status()
            );

            if let Some(new_cookie_header) = change_pwd_resp
                .headers()
                .get_all(reqwest::header::SET_COOKIE)
                .iter()
                .find_map(|h| h.to_str().ok().filter(|s| s.contains("session_id")))
            {
                session_cookie = new_cookie_header
                    .split(';')
                    .next()
                    .expect("Invalid new cookie format")
                    .to_string();
            }
        }
    }

    println!("[E2E Auth] Admin authentication completed successfully.");
    session_cookie
}

#[tokio::test]
async fn test_docker_e2e_bitmap_upload_flow() {
    let _guard = DockerLogGuard;
    println!("[E2E Bitmap Test] Step 1: Building Docker images...");
    let build_status = Command::new("docker")
        .args(["compose", "build"])
        .status()
        .expect("Failed to execute docker compose build");
    assert!(build_status.success(), "docker compose build failed");

    println!("[E2E Bitmap Test] Step 2: Starting Docker container stack...");
    let up_status = Command::new("docker")
        .args(["compose", "up", "-d"])
        .status()
        .expect("Failed to execute docker compose up -d");
    assert!(up_status.success(), "docker compose up -d failed");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build reqwest HTTP client");

    let base_url = "http://localhost:3000";

    println!("[E2E Bitmap Test] Step 3: Polling /readyz health check endpoint...");
    wait_for_stack_ready(&client, base_url).await;

    // 1. Authenticate as admin
    println!("[E2E Bitmap Test] Step 4: Authenticating as admin...");
    let session_cookie = login_as_admin(&client, base_url).await;

    // 2. GET /bitmaps to obtain session CSRF token from HTML
    println!("[E2E Bitmap Test] Step 5: Fetching bitmap library page...");
    let bitmaps_url = format!("{}/bitmaps", base_url);
    let list_resp = client
        .get(&bitmaps_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to fetch bitmap library list page");

    assert_eq!(
        list_resp.status(),
        reqwest::StatusCode::OK,
        "GET /bitmaps returned status code {}",
        list_resp.status()
    );

    let list_html = list_resp
        .text()
        .await
        .expect("Failed to read bitmap library list HTML response");

    let csrf_token = extract_input_value(&list_html, "csrf_token")
        .expect("CSRF token not found in /bitmaps HTML page");

    // 3. Initiate reference bitmap upload via POST /bitmaps
    println!("[E2E Bitmap Test] Step 6: Initiating reference bitmap upload...");
    let test_bitmap_name = format!("Docker E2E Bitmap {}", uuid::Uuid::new_v4().simple());
    let mut upload_form = HashMap::new();
    upload_form.insert("csrf_token", csrf_token.as_str());
    upload_form.insert("name", test_bitmap_name.as_str());

    let init_upload_resp = client
        .post(&bitmaps_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&upload_form)
        .send()
        .await
        .expect("Failed to submit POST /bitmaps upload form");

    assert_eq!(
        init_upload_resp.status(),
        reqwest::StatusCode::OK,
        "POST /bitmaps returned unexpected status {}",
        init_upload_resp.status()
    );

    let upload_template_html = init_upload_resp
        .text()
        .await
        .expect("Failed to read upload template HTML");

    let (mut presigned_post_url, presigned_fields) =
        extract_form_action_and_inputs(&upload_template_html)
            .expect("Failed to extract presigned form action and hidden fields from upload page");

    if presigned_post_url.starts_with('/') {
        presigned_post_url = format!("http://localhost:9000{}", presigned_post_url);
    } else if presigned_post_url.contains("http://rustfs:9000") {
        presigned_post_url =
            presigned_post_url.replace("http://rustfs:9000", "http://localhost:9000");
    }

    assert!(
        !presigned_fields.is_empty(),
        "Presigned hidden form fields missing in upload template"
    );

    // 4. Construct multipart/form-data request and upload PNG to S3/RustFS
    println!("[E2E Bitmap Test] Step 7: Uploading sample PNG to S3/RustFS...");
    let sample_png_bytes = create_sample_png_bytes(10, 10);
    let mut multipart = reqwest::multipart::Form::new();

    for (k, v) in presigned_fields {
        multipart = multipart.text(k, v);
    }

    let file_part = reqwest::multipart::Part::bytes(sample_png_bytes)
        .file_name("reference.png")
        .mime_str("image/png")
        .expect("Invalid mime type");

    multipart = multipart.part("file", file_part);

    let s3_resp = client
        .post(&presigned_post_url)
        .multipart(multipart)
        .send()
        .await
        .expect("Failed to upload bitmap image to S3");

    let mut redirect_location = s3_resp
        .headers()
        .get("location")
        .or_else(|| s3_resp.headers().get("Location"))
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            panic!(
                "S3 presigned POST response status {} missing Location header. Headers: {:?}",
                s3_resp.status(),
                s3_resp.headers()
            )
        });

    if redirect_location.contains("http://app:3000") {
        redirect_location = redirect_location.replace("http://app:3000", "http://localhost:3000");
    }

    // 5. Follow S3 redirect to GET /bitmaps/commit
    println!("[E2E Bitmap Test] Step 8: Confirming bitmap upload commit...");
    let commit_page_resp = client
        .get(&redirect_location)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to fetch bitmap commit confirmation page");

    assert_eq!(
        commit_page_resp.status(),
        reqwest::StatusCode::OK,
        "Commit confirmation page returned status {}",
        commit_page_resp.status()
    );

    let commit_page_html = commit_page_resp
        .text()
        .await
        .expect("Failed to read commit confirmation page HTML");

    let commit_csrf_token = extract_input_value(&commit_page_html, "csrf_token")
        .expect("CSRF token not found on commit confirmation page");
    let commit_key = extract_input_value(&commit_page_html, "key")
        .expect("Key field not found on commit confirmation page");
    let commit_name = extract_input_value(&commit_page_html, "name")
        .expect("Name field not found on commit confirmation page");

    // 6. Submit final POST /bitmaps/commit to persist database record
    let commit_endpoint = format!("{}/bitmaps/commit", base_url);
    let mut commit_form = HashMap::new();
    commit_form.insert("csrf_token", commit_csrf_token.as_str());
    commit_form.insert("key", commit_key.as_str());
    commit_form.insert("name", commit_name.as_str());

    let final_commit_resp = client
        .post(&commit_endpoint)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&commit_form)
        .send()
        .await
        .expect("Failed to submit POST /bitmaps/commit");

    assert_eq!(
        final_commit_resp.status(),
        reqwest::StatusCode::SEE_OTHER,
        "POST /bitmaps/commit expected 303 See Other redirect, got status: {}",
        final_commit_resp.status()
    );

    // 7. Fetch GET /bitmaps and verify uploaded bitmap appears in library HTML
    println!("[E2E Bitmap Test] Step 9: Verifying bitmap appears in library list...");
    let final_list_resp = client
        .get(&bitmaps_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to fetch updated bitmap library page");

    let final_list_html = final_list_resp
        .text()
        .await
        .expect("Failed to read updated bitmap library HTML");

    assert!(
        final_list_html.contains(&test_bitmap_name),
        "Uploaded bitmap name '{}' not found in library list page HTML!",
        test_bitmap_name
    );
    assert!(
        final_list_html.contains("10 × 10 px"),
        "Uploaded bitmap dimensions '10 × 10 px' not found in library list page HTML!"
    );
    println!("[E2E Bitmap Test] Bitmap upload E2E test completed successfully!");
}

#[tokio::test]
async fn test_docker_e2e_simulator_workflow_and_screenshot() {
    let _guard = DockerLogGuard;
    println!("[E2E Simulator Test] Step 1: Building Docker images...");
    let build_status = Command::new("docker")
        .args(["compose", "build"])
        .status()
        .expect("Failed to execute docker compose build");
    assert!(build_status.success(), "docker compose build failed");

    println!("[E2E Simulator Test] Step 2: Starting Docker container stack...");
    let up_status = Command::new("docker")
        .args(["compose", "up", "-d"])
        .status()
        .expect("Failed to execute docker compose up -d");
    assert!(up_status.success(), "docker compose up -d failed");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build reqwest HTTP client");

    let base_url = "http://localhost:3000";

    println!("[E2E Simulator Test] Step 3: Polling /readyz health check endpoint...");
    wait_for_stack_ready(&client, base_url).await;

    // 1. Authenticate session
    println!("[E2E Simulator Test] Step 4: Authenticating as admin...");
    let session_cookie = login_as_admin(&client, base_url).await;

    // 2. Fetch GET /automations/new to obtain session CSRF token
    println!("[E2E Simulator Test] Step 5: Fetching /automations/new form...");
    let automations_url = format!("{}/automations", base_url);
    let new_auto_url = format!("{}/automations/new", base_url);
    let new_auto_resp = client
        .get(&new_auto_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to GET /automations/new");

    let new_auto_html = new_auto_resp
        .text()
        .await
        .expect("Failed to read /automations/new HTML");

    let csrf_token = extract_input_value(&new_auto_html, "csrf_token")
        .expect("CSRF token not found in /automations/new HTML");

    // 3. Create a test automation
    println!("[E2E Simulator Test] Step 6: Creating test automation...");
    let test_auto_name = format!("Simulator Workflow {}", uuid::Uuid::new_v4().simple());
    let mut create_auto_form = HashMap::new();
    create_auto_form.insert("csrf_token", csrf_token.as_str());
    create_auto_form.insert("name", test_auto_name.as_str());
    create_auto_form.insert("description", "Docker E2E simulator workflow test");

    let create_auto_resp = client
        .post(&automations_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&create_auto_form)
        .send()
        .await
        .expect("Failed to submit create automation form");

    assert_eq!(
        create_auto_resp.status(),
        reqwest::StatusCode::SEE_OTHER,
        "POST /automations expected 303 See Other redirect, got: {}",
        create_auto_resp.status()
    );

    let location = create_auto_resp
        .headers()
        .get("location")
        .and_then(|h| h.to_str().ok())
        .expect("Missing Location header in create automation redirect");

    let auto_id_str = location.trim_start_matches("/automations/");
    let clean_auto_id_str = auto_id_str.split('?').next().unwrap_or(auto_id_str);
    let automation_id: i64 = clean_auto_id_str
        .parse()
        .expect("Failed to parse automation ID from redirect location");

    println!(
        "[E2E Simulator Test] Created automation ID: {}",
        automation_id
    );

    // 4. Add Step 1: mouse_click
    println!("[E2E Simulator Test] Step 7: Adding mouse_click and find_pixel_rgb steps...");
    let steps_url = format!("{}/automations/{}/steps", base_url, automation_id);
    let mut click_step_form = HashMap::new();
    click_step_form.insert("csrf_token", csrf_token.as_str());
    click_step_form.insert("step_type", "mouse_click");
    click_step_form.insert("label", "Click Submit Button");
    click_step_form.insert("x_mode", "fixed");
    click_step_form.insert("x", "500");
    click_step_form.insert("y_mode", "fixed");
    click_step_form.insert("y", "300");
    click_step_form.insert("button", "left");
    click_step_form.insert("click_type", "single");
    click_step_form.insert("post_delay_seconds", "0.0");

    let add_step_resp = client
        .post(&steps_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&click_step_form)
        .send()
        .await
        .expect("Failed to add mouse_click step");

    assert_eq!(add_step_resp.status(), reqwest::StatusCode::SEE_OTHER);

    // Create a color variable for find_pixel_rgb step output
    let create_var_url = format!("{}/automations/{}/variables", base_url, automation_id);
    let mut create_var_form = HashMap::new();
    create_var_form.insert("csrf_token", csrf_token.as_str());
    create_var_form.insert("name", "pixel_color");
    create_var_form.insert("var_type", "color");
    create_var_form.insert("description", "Pixel color output");

    let create_var_resp = client
        .post(&create_var_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&create_var_form)
        .send()
        .await
        .expect("Failed to create pixel_color variable");

    assert_eq!(create_var_resp.status(), reqwest::StatusCode::SEE_OTHER);

    // Fetch new step find_pixel_rgb page to extract variable ID from dropdown HTML
    let new_fp_url = format!(
        "{}/automations/{}/steps/new/find_pixel_rgb",
        base_url, automation_id
    );
    let new_fp_resp = client
        .get(&new_fp_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to GET /automations/{id}/steps/new/find_pixel_rgb");

    let new_fp_html = new_fp_resp
        .text()
        .await
        .expect("Failed to read find_pixel_rgb form HTML");

    let var_id_opt = {
        let pattern = "<option value=\"";
        let mut found = None;
        let mut search_idx = 0;
        while let Some(pos) = new_fp_html[search_idx..].find(pattern) {
            let abs_pos = search_idx + pos + pattern.len();
            if let Some(end) = new_fp_html[abs_pos..].find('"') {
                let val = &new_fp_html[abs_pos..abs_pos + end];
                let tag_end = new_fp_html[abs_pos..]
                    .find("</option>")
                    .map(|p| abs_pos + p)
                    .unwrap_or(abs_pos);
                let option_text = &new_fp_html[abs_pos..tag_end];
                if !val.is_empty() && val != "0" && option_text.contains("pixel_color") {
                    found = Some(val.to_string());
                    break;
                }
            }
            search_idx = abs_pos;
        }
        found
    };

    // Add Step 2: find_pixel_rgb
    let mut pixel_step_form = HashMap::new();
    pixel_step_form.insert("csrf_token", csrf_token.as_str());
    pixel_step_form.insert("step_type", "find_pixel_rgb");
    pixel_step_form.insert("label", "Sample Color");
    pixel_step_form.insert("x", "500");
    pixel_step_form.insert("y", "300");
    pixel_step_form.insert("post_delay_seconds", "0.0");
    if let Some(ref vid) = var_id_opt {
        pixel_step_form.insert("output_variable_id", vid.as_str());
    }

    let add_pixel_resp = client
        .post(&steps_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&pixel_step_form)
        .send()
        .await
        .expect("Failed to add find_pixel_rgb step");

    assert_eq!(add_pixel_resp.status(), reqwest::StatusCode::SEE_OTHER);

    // Activate automation via POST /automations/{id}
    println!("[E2E Simulator Test] Activating automation...");
    let auto_detail_url = format!("{}/automations/{}", base_url, automation_id);
    let mut activate_form = HashMap::new();
    activate_form.insert("csrf_token", csrf_token.as_str());
    activate_form.insert("name", test_auto_name.as_str());
    activate_form.insert("description", "Docker E2E simulator workflow test");
    activate_form.insert("status", "active");

    let activate_resp = client
        .post(&auto_detail_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&activate_form)
        .send()
        .await
        .expect("Failed to activate automation");

    assert_eq!(activate_resp.status(), reqwest::StatusCode::SEE_OTHER);

    // 5. Trigger immediate execution run via POST /automations/{id}/run-now
    println!("[E2E Simulator Test] Step 8: Triggering immediate task execution (run-now)...");
    let run_now_url = format!("{}/automations/{}/run-now", base_url, automation_id);
    let mut run_now_form = HashMap::new();
    run_now_form.insert("csrf_token", csrf_token.as_str());

    let run_now_resp = client
        .post(&run_now_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&run_now_form)
        .send()
        .await
        .expect("Failed to trigger run-now for automation");

    assert_eq!(
        run_now_resp.status(),
        reqwest::StatusCode::SEE_OTHER,
        "POST /automations/{}/run-now expected 303 redirect",
        automation_id
    );

    let run_location = run_now_resp
        .headers()
        .get("location")
        .and_then(|h| h.to_str().ok())
        .expect("Missing Location header in run-now redirect");

    assert!(
        run_location.starts_with("/runs/"),
        "Expected run-now redirect to /runs/{{id}}, but got redirect location: {}",
        run_location
    );

    let run_id_str = run_location.trim_start_matches("/runs/");
    let clean_run_id_str = run_id_str.split('?').next().unwrap_or(run_id_str);
    let task_run_id: i64 = clean_run_id_str.parse().unwrap_or_else(|e| {
        panic!(
            "Failed to parse task_run_id from redirect location '{}': {}",
            run_location, e
        )
    });

    println!(
        "[E2E Simulator Test] Task run triggered (ID: {}). Polling for assignment and execution...",
        task_run_id
    );

    // 6. Poll task run status until completed by containerized simulator service
    println!("[E2E Simulator Test] Step 9: Polling task run status until completion...");
    let run_detail_url = format!("{}/runs/{}/status-frame", base_url, task_run_id);
    let run_poll_start = Instant::now();
    let run_timeout = Duration::from_secs(300);
    let mut run_succeeded = false;
    let mut run_poll_attempts = 0;
    let mut last_status = String::from("unknown");

    while run_poll_start.elapsed() < run_timeout {
        run_poll_attempts += 1;
        let run_resp = client
            .get(&run_detail_url)
            .header(reqwest::header::COOKIE, &session_cookie)
            .send()
            .await
            .expect("Failed to poll task run detail");

        if run_resp.status() == reqwest::StatusCode::OK {
            let run_html = run_resp
                .text()
                .await
                .expect("Failed to read task run detail HTML");

            if run_html.contains("Succeeded") || run_html.contains("succeeded") {
                println!(
                    "[E2E Simulator Test] Task run {} succeeded after {}s!",
                    task_run_id,
                    run_poll_start.elapsed().as_secs()
                );
                run_succeeded = true;
                last_status = "Succeeded".to_string();
                break;
            }

            if run_html.contains("Failed") || run_html.contains("failed") {
                last_status = "Failed".to_string();
                println!(
                    "[E2E TEST] Task run {} entered terminal failure state 'Failed'!",
                    task_run_id
                );
                break;
            }

            if run_html.contains("Cancelled") || run_html.contains("cancelled") {
                last_status = "Cancelled".to_string();
                println!(
                    "[E2E TEST] Task run {} entered terminal failure state 'Cancelled'!",
                    task_run_id
                );
                break;
            }

            if run_html.contains("Running") || run_html.contains("running") {
                last_status = "Running".to_string();
            } else if run_html.contains("Queued") || run_html.contains("queued") {
                last_status = "Queued".to_string();
            }
        } else {
            println!(
                "[E2E TEST] Warning: GET /runs/{} returned HTTP status {}",
                task_run_id,
                run_resp.status()
            );
        }
        println!(
            "[E2E Simulator Test] Polling task run {} status (status: {}, attempt {}, elapsed {}s)...",
            task_run_id,
            last_status,
            run_poll_attempts,
            run_poll_start.elapsed().as_secs()
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    if !run_succeeded {
        println!("[E2E TEST] Task run polling failed or timed out. Dumping Docker logs...");
        dump_docker_logs();
    }

    assert!(
        run_succeeded,
        "Task run {} failed to complete with status succeeded within timeout (last status: '{}')",
        task_run_id, last_status
    );

    // 7. Verify screenshots arrived and can be downloaded as a ZIP archive
    println!("[E2E Simulator Test] Step 10: Downloading and verifying screenshots ZIP archive...");
    let zip_url = format!("{}/runs/{}/screenshots.zip", base_url, task_run_id);
    let zip_resp = client
        .get(&zip_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to download run screenshots ZIP");

    assert_eq!(
        zip_resp.status(),
        reqwest::StatusCode::OK,
        "GET /runs/{}/screenshots.zip expected 200 OK",
        task_run_id
    );

    let zip_bytes = zip_resp
        .bytes()
        .await
        .expect("Failed to read screenshots ZIP response bytes");

    assert!(
        zip_bytes.len() > 100,
        "Downloaded screenshots ZIP file is unexpectedly small ({} bytes)",
        zip_bytes.len()
    );

    // Confirm PK ZIP header bytes (0x50, 0x4B, 0x03, 0x04)
    assert_eq!(
        &zip_bytes[0..4],
        &[0x50, 0x4B, 0x03, 0x04],
        "Downloaded ZIP archive header signature invalid"
    );

    println!("[E2E Simulator Test] Simulator workflow E2E test completed successfully!");
}

#[tokio::test]
async fn test_docker_e2e_worker_recording_and_draft_automation_screenshots() {
    let _guard = DockerLogGuard;
    println!("[E2E Recording Test] Step 1: Building Docker images...");
    let build_status = Command::new("docker")
        .args(["compose", "build"])
        .status()
        .expect("Failed to execute docker compose build");
    assert!(build_status.success(), "docker compose build failed");

    println!("[E2E Recording Test] Step 2: Starting Docker container stack...");
    let up_status = Command::new("docker")
        .args(["compose", "up", "-d"])
        .status()
        .expect("Failed to execute docker compose up -d");
    assert!(up_status.success(), "docker compose up -d failed");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build reqwest HTTP client");

    let base_url = "http://localhost:3000";

    println!("[E2E Recording Test] Step 3: Polling /readyz health check endpoint...");
    wait_for_stack_ready(&client, base_url).await;

    // 1. Authenticate as admin
    println!("[E2E Recording Test] Step 4: Authenticating as admin...");
    let session_cookie = login_as_admin(&client, base_url).await;

    // 2. Register a dedicated recording worker node
    println!("[E2E Recording Test] Step 5: Registering recording worker via WPS PSK...");
    let reg_url = format!("{}/api/v1/workers/register", base_url);
    let reg_req_json = serde_json::json!({
        "psk": "deskdispatch_psk_prod_secret_1234",
        "hostname": "e2e-rec-worker",
        "display_name": "E2E Recording Worker"
    })
    .to_string();

    let reg_resp = client
        .post(&reg_url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(reg_req_json)
        .send()
        .await
        .expect("Failed to register worker via PSK");

    let reg_body = reg_resp.text().await.expect("Failed to read reg response");
    let reg_json: serde_json::Value =
        serde_json::from_str(&reg_body).expect("Failed to parse reg response JSON");
    let rec_worker_id = reg_json
        .get("worker_id")
        .and_then(|v| v.as_i64())
        .expect("Missing worker_id");
    let rec_api_key = reg_json
        .get("api_key")
        .and_then(|v| v.as_str())
        .expect("Missing api_key");
    let auth_header_val = format!("Bearer {}", rec_api_key);

    println!(
        "[E2E Recording Test] Registered recording worker ID: {}",
        rec_worker_id
    );

    // 3. Start recording session on the active worker
    println!("[E2E Recording Test] Step 6: Starting worker recording session...");
    let start_rec_page_url = format!("{}/workers/{}/record", base_url, rec_worker_id);
    let start_rec_page_resp = client
        .get(&start_rec_page_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to GET /workers/{id}/record");

    let start_rec_html = start_rec_page_resp
        .text()
        .await
        .expect("Failed to read record start page HTML");

    let rec_csrf_token = extract_input_value(&start_rec_html, "csrf_token")
        .expect("CSRF token not found in recording start page");

    let start_rec_action_url = format!("{}/workers/{}/record/start", base_url, rec_worker_id);
    let mut start_rec_form = HashMap::new();
    start_rec_form.insert("csrf_token", rec_csrf_token.as_str());

    let start_rec_resp = client
        .post(&start_rec_action_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&start_rec_form)
        .send()
        .await
        .expect("Failed to POST /workers/{id}/record/start");

    assert_eq!(
        start_rec_resp.status(),
        reqwest::StatusCode::SEE_OTHER,
        "Expected 303 redirect after starting recording"
    );

    let rec_redirect = start_rec_resp
        .headers()
        .get("location")
        .and_then(|h| h.to_str().ok())
        .expect("Missing Location header in start recording redirect");

    let session_id_str = rec_redirect.trim_start_matches("/recordings/");
    let session_id: i64 = session_id_str
        .split('?')
        .next()
        .unwrap_or(session_id_str)
        .parse()
        .expect("Failed to parse recording session_id from redirect location");

    println!(
        "[E2E Recording Test] Started recording session ID: {}",
        session_id
    );

    // 4. Submit recorded events via worker API (simulating worker recording actions)
    println!("[E2E Recording Test] Step 7: Submitting simulated worker recording events...");
    let worker_events_url = format!(
        "{}/api/v1/workers/recordings/{}/events",
        base_url, session_id
    );
    let upload_url_endpoint = format!(
        "{}/api/v1/workers/recordings/{}/screenshot-upload-url",
        base_url, session_id
    );

    let upload_info_resp = client
        .get(&upload_url_endpoint)
        .header("Authorization", &auth_header_val)
        .send()
        .await
        .expect("Failed to GET recording screenshot upload URL");

    assert_eq!(upload_info_resp.status(), reqwest::StatusCode::OK);
    let upload_info_body = upload_info_resp
        .text()
        .await
        .expect("Failed to read upload info body");
    let info: serde_json::Value =
        serde_json::from_str(&upload_info_body).expect("Failed to parse upload info JSON");

    let upload_url = info
        .get("upload_url")
        .and_then(|v| v.as_str())
        .expect("Missing upload_url");
    let obj_key = info
        .get("object_key")
        .and_then(|v| v.as_str())
        .expect("Missing object_key");

    let mut put_target = upload_url.to_string();
    if put_target.contains("http://rustfs:9000") {
        put_target = put_target.replace("http://rustfs:9000", "http://localhost:9000");
    }
    let sample_png = create_sample_png_bytes(100, 100);
    let put_resp = client
        .put(&put_target)
        .header("Content-Type", "image/png")
        .body(sample_png)
        .send()
        .await
        .expect("Failed to PUT event screenshot to S3");

    assert_eq!(put_resp.status(), reqwest::StatusCode::OK);
    let event_shot_key = Some(obj_key.to_string());

    let events_req_body = serde_json::json!({
        "events": [
            {
                "sequence_number": 1,
                "event_type": "mouse_click",
                "x": 500,
                "y": 300,
                "button": "left",
                "key_combo": null,
                "screenshot_object_key": event_shot_key
            },
            {
                "sequence_number": 2,
                "event_type": "key_press",
                "x": null,
                "y": null,
                "button": null,
                "key_combo": "enter",
                "screenshot_object_key": null
            }
        ]
    })
    .to_string();

    let events_post_resp = client
        .post(&worker_events_url)
        .header("Authorization", &auth_header_val)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(events_req_body)
        .send()
        .await
        .expect("Failed to POST recording events");

    assert_eq!(events_post_resp.status(), reqwest::StatusCode::OK);

    // Stop recording session via POST /recordings/{session_id}/stop
    println!("[E2E Recording Test] Step 8: Stopping recording session...");
    let stop_rec_url = format!("{}/recordings/{}/stop", base_url, session_id);
    let mut stop_rec_form = HashMap::new();
    stop_rec_form.insert("csrf_token", rec_csrf_token.as_str());

    let stop_rec_resp = client
        .post(&stop_rec_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&stop_rec_form)
        .send()
        .await
        .expect("Failed to POST /recordings/{session_id}/stop");

    assert_eq!(stop_rec_resp.status(), reqwest::StatusCode::SEE_OTHER);

    // 5. Fetch GET /recordings/{session_id}/review to review captured events and form
    println!("[E2E Recording Test] Step 9: Fetching recording review page...");
    let review_url = format!("{}/recordings/{}/review", base_url, session_id);
    let review_resp = client
        .get(&review_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to GET /recordings/{session_id}/review");

    assert_eq!(review_resp.status(), reqwest::StatusCode::OK);

    let review_html = review_resp
        .text()
        .await
        .expect("Failed to read recording review page HTML");

    let review_csrf_token = extract_input_value(&review_html, "csrf_token")
        .expect("CSRF token not found on recording review page");

    // 6. Convert recording into draft automation via POST /recordings/{session_id}/convert
    println!("[E2E Recording Test] Step 10: Converting recording to draft automation...");
    let convert_url = format!("{}/recordings/{}/convert", base_url, session_id);
    let auto_title = format!(
        "E2E Draft Recording Automation {}",
        uuid::Uuid::new_v4().simple()
    );
    let mut convert_form = HashMap::new();
    convert_form.insert("csrf_token", review_csrf_token.as_str());
    convert_form.insert("automation_name", auto_title.as_str());
    convert_form.insert(
        "automation_description",
        "Converted from E2E recording session",
    );

    let convert_resp = client
        .post(&convert_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .form(&convert_form)
        .send()
        .await
        .expect("Failed to POST /recordings/{session_id}/convert");

    assert_eq!(
        convert_resp.status(),
        reqwest::StatusCode::SEE_OTHER,
        "Expected 303 See Other redirect after converting recording"
    );

    let convert_redirect = convert_resp
        .headers()
        .get("location")
        .and_then(|h| h.to_str().ok())
        .expect("Missing Location header in convert recording redirect");

    let auto_id_str = convert_redirect.trim_start_matches("/automations/");
    let converted_auto_id: i64 = auto_id_str
        .split('?')
        .next()
        .unwrap_or(auto_id_str)
        .parse()
        .expect("Failed to parse converted automation ID from redirect URL");

    println!(
        "[E2E Recording Test] Converted to draft automation ID: {}",
        converted_auto_id
    );

    // 7. Fetch converted draft automation detail page and verify screenshot thumbnails
    println!(
        "[E2E Recording Test] Step 11: Fetching draft automation page and testing screenshot thumbnails..."
    );
    let auto_page_url = format!("{}/automations/{}", base_url, converted_auto_id);
    let auto_page_resp = client
        .get(&auto_page_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to GET /automations/{id}");

    assert_eq!(auto_page_resp.status(), reqwest::StatusCode::OK);

    let auto_page_html = auto_page_resp
        .text()
        .await
        .expect("Failed to read draft automation page HTML");

    assert!(
        auto_page_html.contains(&auto_title),
        "Draft automation title '{}' not found in page HTML",
        auto_title
    );

    // Find screenshot step ID from /media/screenshots/{step_id} link in HTML
    let step_id_opt = {
        let pattern = "/media/screenshots/";
        let mut found = None;
        let mut search_idx = 0;
        while let Some(pos) = auto_page_html[search_idx..].find(pattern) {
            let abs_pos = search_idx + pos + pattern.len();
            if let Some(end) = auto_page_html[abs_pos..].find('"') {
                let candidate = &auto_page_html[abs_pos..abs_pos + end];
                if let Ok(num) = candidate.parse::<i64>() {
                    found = Some(num);
                    break;
                }
            }
            search_idx = abs_pos;
        }
        found
    };

    let step_id = step_id_opt.expect(
        "No step screenshot thumbnail (/media/screenshots/{id}) found on draft automation page",
    );

    println!(
        "[E2E Recording Test] Found screenshot step ID {} on converted draft automation page!",
        step_id
    );

    // 8. Fetch /media/screenshots/{step_id} and follow redirect to verify image loads cleanly (200 OK)
    println!(
        "[E2E Recording Test] Step 12: Requesting /media/screenshots/{} and verifying image loads...",
        step_id
    );
    let media_url = format!("{}/media/screenshots/{}", base_url, step_id);
    let media_resp = client
        .get(&media_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to GET /media/screenshots/{step_id}");

    assert!(
        media_resp.status() == reqwest::StatusCode::FOUND
            || media_resp.status() == reqwest::StatusCode::SEE_OTHER,
        "Expected redirect for media screenshot, got {}",
        media_resp.status()
    );

    let presigned_get_location = media_resp
        .headers()
        .get("location")
        .and_then(|h| h.to_str().ok())
        .expect("Missing Location header in /media/screenshots/{step_id} redirect");

    let mut direct_s3_url = presigned_get_location.to_string();
    if direct_s3_url.contains("http://rustfs:9000") {
        direct_s3_url = direct_s3_url.replace("http://rustfs:9000", "http://localhost:9000");
    }

    println!(
        "[E2E Recording Test] Fetching presigned image URL: {}",
        direct_s3_url
    );

    let img_resp = client
        .get(&direct_s3_url)
        .send()
        .await
        .expect("Failed to fetch image from presigned GET URL");

    assert_eq!(
        img_resp.status(),
        reqwest::StatusCode::OK,
        "Presigned image GET request returned status {}, expected 200 OK!",
        img_resp.status()
    );

    let img_bytes = img_resp
        .bytes()
        .await
        .expect("Failed to read screenshot image bytes");

    assert!(
        img_bytes.len() > 10,
        "Screenshot image response body is unexpectedly small ({} bytes)",
        img_bytes.len()
    );

    // PNG header signature verification: 0x89, 'P', 'N', 'G', '\r', '\n', 0x1A, '\n'
    assert_eq!(
        &img_bytes[0..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        "Screenshot image header signature is not a valid PNG image!"
    );

    println!(
        "[E2E Recording Test] Worker recording and draft automation screenshot E2E test completed successfully!"
    );
}
