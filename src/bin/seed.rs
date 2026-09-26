use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use sqlx::postgres::PgPoolOptions;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgrespassword@localhost:5432/deskdispatch".to_string());

    let admin_username = env::var("ADMIN_USERNAME").unwrap_or_else(|_| "admin".to_string());
    let admin_password = env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "adminpassword".to_string());
    let admin_display_name = env::var("ADMIN_DISPLAY_NAME").unwrap_or_else(|_| "Admin".to_string());

    println!("Connecting to database to seed admin user...");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(admin_password.as_bytes(), &salt)
        .map_err(|e| format!("Password hashing error: {}", e))?
        .to_string();

    let _result = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active)
        VALUES ($1, $2, $3, 'admin', true)
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
