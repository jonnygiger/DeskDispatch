use app::auth::UserRole;
use app::routes::account::AccountPasswordTemplate;
use app::routes::auth::LoginTemplate;
use app::routes::bitmaps::{BitmapListItem, BitmapsListTemplate, BitmapsUploadTemplate, RegionPickerTopLeftTemplate};
use app::routes::errors::{InternalServerErrorTemplate, NotFoundTemplate};
use app::routes::home::{IndexTemplate, RecentRunItem};
use app::routes::media::DevMagnifierTemplate;
use app::routes::runs::*;
use app::routes::workers::*;
use askama::Template;
use chrono::Utc;
use std::fs;

fn main() {
    fs::create_dir_all("rendered_templates").unwrap();

    let login_tmpl = LoginTemplate { error: None };
    fs::write("rendered_templates/login.html", login_tmpl.render().unwrap()).unwrap();

    let pwd_tmpl = AccountPasswordTemplate {
        csrf_token: "test_csrf_token_12345".to_string(),
        error: None,
        success: Some("Password updated successfully.".to_string()),
    };
    fs::write("rendered_templates/account_password.html", pwd_tmpl.render().unwrap()).unwrap();

    let user = app::auth::AuthUser {
        id: 1,
        username: "admin".to_string(),
        display_name: "Admin User".to_string(),
        role: UserRole::Admin,
        session_id: uuid::Uuid::new_v4(),
        csrf_token: "test_csrf_token_12345".to_string(),
    };
    let sample_runs = vec![
        RecentRunItem {
            id: 101,
            automation_id: 1,
            automation_name: "Login Automation".to_string(),
            worker_id: Some(10),
            worker_name: Some("Worker Alpha".to_string()),
            status: "succeeded".to_string(),
            triggered_by: "Admin User".to_string(),
            queued_at: Utc::now(),
        },
        RecentRunItem {
            id: 102,
            automation_id: 2,
            automation_name: "Data Scraper".to_string(),
            worker_id: Some(11),
            worker_name: Some("Worker Beta".to_string()),
            status: "failed".to_string(),
            triggered_by: "Daily Schedule".to_string(),
            queued_at: Utc::now(),
        },
        RecentRunItem {
            id: 103,
            automation_id: 3,
            automation_name: "PDF Exporter".to_string(),
            worker_id: None,
            worker_name: None,
            status: "queued".to_string(),
            triggered_by: "Manual".to_string(),
            queued_at: Utc::now(),
        },
    ];
    let index_tmpl = IndexTemplate {
        user: user.clone(),
        active_automations_count: 5,
        workers_online_count: 3,
        runs_today_count: 12,
        failed_lost_runs_today_count: 1,
        recent_runs: sample_runs,
    };
    fs::write("rendered_templates/index.html", index_tmpl.render().unwrap()).unwrap();

    let not_found_tmpl = NotFoundTemplate {
        user: Some(user.clone()),
    };
    fs::write(
        "rendered_templates/404.html",
        not_found_tmpl.render().unwrap(),
    )
    .unwrap();

    let internal_err_tmpl = InternalServerErrorTemplate {
        message: Some("Something went wrong".to_string()),
        user: Some(user.clone()),
    };
    fs::write(
        "rendered_templates/500.html",
        internal_err_tmpl.render().unwrap(),
    )
    .unwrap();

    let mag = app::magnifier::ImageMagnifier::new(
        "https://via.placeholder.com/800x600.png",
        800,
        600,
        Some(100),
        Some(150),
    );
    let mag_tmpl = app::magnifier::ImageMagnifierTemplate { magnifier: &mag };
    let mag_html = format!(
        "<!DOCTYPE html><html><head><link rel=\"stylesheet\" href=\"../static/style.css\"></head><body><div style=\"padding:2rem;\">{}</div></body></html>",
        mag_tmpl.render().unwrap()
    );
    fs::write("rendered_templates/image_magnifier.html", mag_html).unwrap();

    let dev_mag_tmpl = DevMagnifierTemplate {
        magnifier: mag.clone(),
    };
    fs::write("rendered_templates/dev_magnifier.html", dev_mag_tmpl.render().unwrap()).unwrap();

    let sample_bitmap = BitmapListItem {
        id: 1,
        automation_id: Some(1),
        automation_name: Some("Sample Automation".to_string()),
        name: "Login Button Bitmap".to_string(),
        object_storage_key: "bitmaps/sample.png".to_string(),
        width: 120,
        height: 40,
        created_by: 1,
        created_by_name: "Admin User".to_string(),
        created_at: Utc::now(),
        magnifier: app::magnifier::ImageMagnifier::new(
            "/media/bitmaps/1",
            120,
            40,
            None,
            None,
        ),
    };

    let bitmaps_tmpl = BitmapsListTemplate {
        user: user.clone(),
        csrf_token: "test_csrf_token_12345".to_string(),
        bitmaps: vec![sample_bitmap],
        automation_id: None,
        automation_name: None,
    };
    fs::write("rendered_templates/bitmaps_list.html", bitmaps_tmpl.render().unwrap()).unwrap();

    let upload_tmpl = BitmapsUploadTemplate {
        user: user.clone(),
        csrf_token: "test_csrf_token_12345".to_string(),
        bitmap_name: "Login Button".to_string(),
        object_storage_key: "bitmaps/sample.png".to_string(),
        automation_id: None,
        presigned_post_url: "http://localhost:9000/deskdispatch-bucket".to_string(),
        presigned_fields: vec![
            ("key".to_string(), "bitmaps/sample.png".to_string()),
            ("policy".to_string(), "sample_policy".to_string()),
        ],
        redirect_url: "http://localhost:3000/bitmaps/commit?key=bitmaps/sample.png&name=Login%20Button".to_string(),
    };
    fs::write("rendered_templates/bitmaps_upload.html", upload_tmpl.render().unwrap()).unwrap();

    let pick_region_tmpl = RegionPickerTopLeftTemplate {
        user: user.clone(),
        csrf_token: "test_csrf_token_12345".to_string(),
        automation_id: None,
        image_url: "/static/sample_screenshot.png".to_string(),
        width: 1920,
        height: 1080,
        step_stage: 1,
        top_left_x: None,
        top_left_y: None,
        bottom_right_x: None,
        bottom_right_y: None,
        click_x: None,
        click_y: None,
        magnifier: app::magnifier::ImageMagnifier::new(
            "/static/sample_screenshot.png",
            1920,
            1080,
            None,
            None,
        ),
        return_to: None,
        mode: None,
        step_id: None,
        reference_bitmap_id: None,
    };
    fs::write("rendered_templates/bitmaps_pick_region.html", pick_region_tmpl.render().unwrap()).unwrap();

    let pick_region_stage3_tmpl = RegionPickerTopLeftTemplate {
        user: user.clone(),
        csrf_token: "test_csrf_token_12345".to_string(),
        automation_id: None,
        image_url: "/static/sample_screenshot.png".to_string(),
        width: 1920,
        height: 1080,
        step_stage: 3,
        top_left_x: Some(100),
        top_left_y: Some(100),
        bottom_right_x: Some(250),
        bottom_right_y: Some(200),
        click_x: Some(250),
        click_y: Some(200),
        magnifier: app::magnifier::ImageMagnifier::new(
            "/static/sample_screenshot.png",
            1920,
            1080,
            Some(250),
            Some(200),
        ),
        return_to: None,
        mode: None,
        step_id: None,
        reference_bitmap_id: None,
    };
    fs::write("rendered_templates/bitmaps_pick_region_stage3.html", pick_region_stage3_tmpl.render().unwrap()).unwrap();

    let step_picker_tmpl = app::routes::automations::StepTypePickerTemplate {
        user: user.clone(),
        automation_id: 1,
    };
    fs::write("rendered_templates/step_type_picker.html", step_picker_tmpl.render().unwrap()).unwrap();

    let worker_item = WorkerPcItem {
        id: 1,
        hostname: "pc-warehouse-01".to_string(),
        display_name: "Warehouse Worker PC".to_string(),
        status: "online".to_string(),
        last_heartbeat_at: Some(Utc::now()),
        screen_width: Some(1920),
        screen_height: Some(1080),
        os_info: Some("Windows 11".to_string()),
        agent_version: Some("v2.1.0".to_string()),
        created_at: Utc::now(),
        groups: vec!["Warehouse Fleet".to_string()],
    };
    let group_item = WorkerGroupItem {
        id: 1,
        name: "Warehouse Fleet".to_string(),
        description: "PCs in warehouse".to_string(),
        member_count: 1,
        member_names: vec!["Warehouse Worker PC".to_string()],
    };
    let workers_idx_tmpl = WorkersIndexTemplate {
        user: user.clone(),
        workers: vec![worker_item],
        worker_groups: vec![group_item],
    };
    fs::write("rendered_templates/workers_index.html", workers_idx_tmpl.render().unwrap()).unwrap();

    let worker_detail = WorkerDetail {
        id: 1,
        hostname: "pc-warehouse-01".to_string(),
        display_name: "Warehouse Worker PC".to_string(),
        status: "online".to_string(),
        last_heartbeat_at: Some(Utc::now()),
        screen_width: Some(1920),
        screen_height: Some(1080),
        os_info: Some("Windows 11".to_string()),
        agent_version: Some("v2.1.0".to_string()),
        created_at: Utc::now(),
        groups: vec![WorkerGroupSimple {
            id: 1,
            name: "Warehouse Fleet".to_string(),
            description: "PCs in warehouse".to_string(),
        }],
        registration_token: None,
    };
    let worker_detail_tmpl = WorkerDetailTemplate {
        user: user.clone(),
        worker: worker_detail.clone(),
        error: None,
    };
    fs::write("rendered_templates/workers_detail.html", worker_detail_tmpl.render().unwrap()).unwrap();

    let worker_new_tmpl = WorkerNewTemplate {
        user: user.clone(),
        hostname: "pc-warehouse-02.local".to_string(),
        display_name: "Warehouse Worker PC 2".to_string(),
        error: None,
    };
    fs::write("rendered_templates/workers_new.html", worker_new_tmpl.render().unwrap()).unwrap();

    let worker_edit_tmpl = WorkerEditTemplate {
        user: user.clone(),
        worker: worker_detail,
        all_groups: vec![WorkerGroupSimple {
            id: 1,
            name: "Warehouse Fleet".to_string(),
            description: "PCs in warehouse".to_string(),
        }],
        error: None,
    };
    fs::write("rendered_templates/workers_edit.html", worker_edit_tmpl.render().unwrap()).unwrap();

    let group_form_tmpl = WorkerGroupFormTemplate {
        user: user.clone(),
        group_id: Some(1),
        name: "Warehouse Fleet".to_string(),
        description: "PCs in warehouse".to_string(),
        member_worker_ids: vec![1],
        all_workers: vec![WorkerSimple {
            id: 1,
            hostname: "pc-warehouse-01".to_string(),
            display_name: "Warehouse Worker PC".to_string(),
        }],
        error: None,
        is_edit: true,
    };
    fs::write("rendered_templates/worker_group_form.html", group_form_tmpl.render().unwrap()).unwrap();

    let sample_run_item = TaskRunListItem {
        id: 4821,
        automation_id: 12,
        automation_name: "Daily Login Check".to_string(),
        schedule_id: Some(1),
        schedule_name: Some("Morning Schedule".to_string()),
        worker_id: Some(1),
        worker_name: Some("Warehouse Worker PC".to_string()),
        status: "succeeded".to_string(),
        triggered_by: "Morning Schedule".to_string(),
        queued_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: Some(Utc::now()),
        error_message: None,
    };

    let runs_list_tmpl = RunsListTemplate {
        user: user.clone(),
        csrf_token: "test_csrf_token_12345".to_string(),
        runs: vec![sample_run_item.clone()],
        automations: vec![AutomationOption {
            id: 12,
            name: "Daily Login Check".to_string(),
        }],
        workers: vec![WorkerOption {
            id: 1,
            display_name: "Warehouse Worker PC".to_string(),
        }],
        filter_automation_id: None,
        filter_worker_id: None,
        filter_status: None,
    };
    fs::write("rendered_templates/runs_list.html", runs_list_tmpl.render().unwrap()).unwrap();

    let sample_exec_step = ExecutedStepItem {
        id: 1,
        step_id: 501,
        step_number: 1,
        step_type: "mouse_click".to_string(),
        label: Some("Click login button".to_string()),
        result: Some("success".to_string()),
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
        captured_r: Some(40),
        captured_g: Some(180),
        captured_b: Some(60),
        captured_found: Some(true),
        captured_x: Some(824),
        captured_y: Some(391),
        screenshot_object_key: Some("runs/4821/step_501.png".to_string()),
        magnifier: Some(mag),
    };

    let sample_var_val = RunVariableValueItem {
        variable_id: 1,
        variable_name: "login_button_x".to_string(),
        var_type: "int".to_string(),
        value: "824".to_string(),
        set_at_step_id: Some(501),
        set_at: Utc::now(),
    };

    let run_detail_tmpl = RunDetailTemplate {
        user: user.clone(),
        csrf_token: "test_csrf_token_12345".to_string(),
        run: sample_run_item,
        steps: vec![sample_exec_step],
        variable_values: vec![sample_var_val],
        auto_refresh: false,
    };
    fs::write("rendered_templates/runs_detail.html", run_detail_tmpl.render().unwrap()).unwrap();

    println!("Successfully rendered Askama templates to rendered_templates/");
}
