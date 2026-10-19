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

#[allow(dead_code)]
fn dump_docker_logs() {
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

async fn wait_for_stack_ready(client: &reqwest::Client, base_url: &str) {
    let ready_endpoint = format!("{}/readyz", base_url);
    let start_time = Instant::now();
    let timeout = Duration::from_secs(300);
    let mut is_ready = false;
    let mut poll_attempts = 0;

    println!("[E2E TEST] Waiting for readyz endpoint: {}", ready_endpoint);
    while start_time.elapsed() < timeout {
        poll_attempts += 1;
        if let Ok(resp) = client.get(&ready_endpoint).send().await
            && resp.status().is_success()
        {
            println!(
                "[E2E TEST] /readyz reported healthy status on attempt {}!",
                poll_attempts
            );
            is_ready = true;
            break;
        }
        println!("[E2E TEST] Polling /readyz (attempt {})...", poll_attempts);
        tokio::time::sleep(Duration::from_secs(1)).await;
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
