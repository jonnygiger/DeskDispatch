use argon2::{Argon2, PasswordHasher};
use sqlx::postgres::PgPoolOptions;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dotenv_vars = deskdispatch::config::parse_dotenv();

    let get_var = |key: &str| -> Option<String> {
        env::var(key)
            .ok()
            .or_else(|| dotenv_vars.get(key).cloned())
            .filter(|v| !v.trim().is_empty())
    };

    let database_url = get_var("DATABASE_URL").unwrap_or_else(|| {
        "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string()
    });

    let admin_username = get_var("ADMIN_USERNAME").unwrap_or_else(|| "admin".to_string());
    let admin_password = get_var("ADMIN_PASSWORD").unwrap_or_else(|| "test".to_string());
    let admin_display_name = get_var("ADMIN_DISPLAY_NAME").unwrap_or_else(|| "Admin".to_string());

    println!("Connecting to database to seed admin user...");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(admin_password.as_bytes())
        .map_err(|e| format!("Password hashing error: {}", e))?
        .to_string();

    let _result = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active, must_change_password)
        VALUES ($1, $2, $3, 'admin', true, true)
        ON CONFLICT (username) DO UPDATE
        SET password_hash = EXCLUDED.password_hash,
            display_name = EXCLUDED.display_name,
            role = 'admin',
            is_active = true
        RETURNING id
        "#,
    )
    .bind(&admin_username)
    .bind(&password_hash)
    .bind(&admin_display_name)
    .fetch_one(&pool)
    .await?;

    println!(
        "Successfully seeded admin user '{}' (role: admin)!",
        admin_username
    );

    Ok(())
}
