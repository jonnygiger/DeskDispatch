use app::auth::UserRole;
use app::routes::account::AccountPasswordTemplate;
use app::routes::auth::LoginTemplate;
use app::routes::bitmaps::{BitmapListItem, BitmapsListTemplate};
use app::routes::errors::{InternalServerErrorTemplate, NotFoundTemplate};
use app::routes::media::DevMagnifierTemplate;
use app::routes::home::{IndexTemplate, RecentRunItem};
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
        magnifier: mag,
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

    println!("Successfully rendered Askama templates to rendered_templates/");
}
