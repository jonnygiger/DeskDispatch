use app::auth::UserRole;
use app::routes::account::AccountPasswordTemplate;
use app::routes::auth::LoginTemplate;
use app::routes::home::IndexTemplate;
use askama::Template;
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
    let index_tmpl = IndexTemplate { user };
    fs::write("rendered_templates/index.html", index_tmpl.render().unwrap()).unwrap();

    println!("Successfully rendered Askama templates to rendered_templates/");
}
