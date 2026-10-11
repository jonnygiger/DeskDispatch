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

        if tag_str.contains("type=\"hidden\"") {
            if let (Some(name), Some(val)) = (
                extract_attr_value(tag_str, "name"),
                extract_attr_value(tag_str, "value"),
            ) {
                if name != "csrf_token" {
                    fields.push((name, val));
                }
            }
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
    let img_buf = image::RgbImage::from_fn(width, height, |x, y| {
        let r = ((x * 255) / width.max(1)) as u8;
        let g = ((y * 255) / height.max(1)) as u8;
        image::Rgb([r, g, 200])
    });
    let mut png_bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img_buf)
        .write_to(&mut png_bytes, image::ImageFormat::Png)
        .expect("Failed to encode sample PNG");
    png_bytes.into_inner()
}

#[tokio::test]
async fn test_docker_e2e_bitmap_upload_flow() {
    let _ = Command::new("docker")
        .args(["compose", "up", "-d"])
        .status();

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build reqwest HTTP client");

    let base_url = "http://localhost:3000";
    let ready_endpoint = format!("{}/readyz", base_url);

    let start_time = Instant::now();
    let timeout = Duration::from_secs(90);
    let mut is_ready = false;

    while start_time.elapsed() < timeout {
        if let Ok(resp) = client.get(&ready_endpoint).send().await {
            if resp.status().is_success() {
                is_ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    assert!(
        is_ready,
        "Docker container stack failed to report healthy readyz status within timeout"
    );

    // 1. Authenticate via POST /login with seeded credentials
    let login_url = format!("{}/login", base_url);

    let mut login_resp = client
        .post(&login_url)
        .form(&[("username", "admin"), ("password", "NewAdminPassword123!")])
        .send()
        .await
        .expect("Failed to submit login request");

    if login_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let mut login_params = HashMap::new();
        login_params.insert("username", "admin");
        login_params.insert("password", "test");
        login_resp = client
            .post(&login_url)
            .form(&login_params)
            .send()
            .await
            .expect("Failed to submit login request with initial password");
    }

    assert_eq!(
        login_resp.status(),
        reqwest::StatusCode::SEE_OTHER,
        "Login request expected 303 See Other redirect, got status: {}",
        login_resp.status()
    );

    // Extract session cookie from login response set-cookie headers
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

    // 2. Handle initial forced password change if redirected to /account/password
    let password_url = format!("{}/account/password", base_url);
    let pwd_page_resp = client
        .get(&password_url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("Failed to GET /account/password");

    if pwd_page_resp.status() == reqwest::StatusCode::OK {
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

    // 3. GET /bitmaps to obtain session CSRF token from HTML
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

    // 4. Initiate reference bitmap upload via POST /bitmaps
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
    }

    assert!(
        !presigned_fields.is_empty(),
        "Presigned hidden form fields missing in upload template"
    );

    // 5. Construct multipart/form-data request and upload PNG to S3/RustFS
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

    let redirect_location = s3_resp
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

    // 6. Follow S3 redirect to GET /bitmaps/commit
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

    // 7. Submit final POST /bitmaps/commit to persist database record
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

    // 8. Fetch GET /bitmaps and verify uploaded bitmap appears in library HTML
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
}
