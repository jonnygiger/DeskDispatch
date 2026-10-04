use argon2::{
    password_hash::phc::PasswordHash,
    Argon2, PasswordHasher, PasswordVerifier,
};
use askama::Template;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use sqlx::Row;

use super::auth::HtmlTemplate;
use crate::auth::{log_audit, AuthUser};
use crate::AppState;

#[derive(Template)]
#[template(path = "account_password.html")]
pub struct AccountPasswordTemplate {
    pub csrf_token: String,
    pub error: Option<String>,
    pub success: Option<String>,
}

#[derive(Deserialize)]
pub struct PasswordForm {
    pub current_password: String,
    pub new_password: String,
    pub confirm_password: String,
}

pub async fn get_password_handler(user: AuthUser) -> impl IntoResponse {
    HtmlTemplate(AccountPasswordTemplate {
        csrf_token: user.csrf_token,
        error: None,
        success: None,
    })
}

pub async fn post_password_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<PasswordForm>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    if form.new_password != form.confirm_password {
        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AccountPasswordTemplate {
                csrf_token,
                error: Some("New passwords do not match.".to_string()),
                success: None,
            }),
        )
            .into_response();
    }

    if form.new_password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(AccountPasswordTemplate {
                csrf_token,
                error: Some("New password must be at least 8 characters long.".to_string()),
                success: None,
            }),
        )
            .into_response();
    }

    let user_row = match sqlx::query("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                HtmlTemplate(AccountPasswordTemplate {
                    csrf_token,
                    error: Some("User not found.".to_string()),
                    success: None,
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("Database error during password update: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(AccountPasswordTemplate {
                    csrf_token,
                    error: Some("Internal server error.".to_string()),
                    success: None,
                }),
            )
                .into_response();
        }
    };

    let password_hash: String = user_row.get("password_hash");

    let parsed_hash = match PasswordHash::new(&password_hash) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Invalid stored password hash: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(AccountPasswordTemplate {
                    csrf_token,
                    error: Some("Internal server error.".to_string()),
                    success: None,
                }),
            )
                .into_response();
        }
    };

    if Argon2::default()
        .verify_password(form.current_password.as_bytes(), &parsed_hash)
        .is_err()
    {
        return (
            StatusCode::UNAUTHORIZED,
            HtmlTemplate(AccountPasswordTemplate {
                csrf_token,
                error: Some("Current password is incorrect.".to_string()),
                success: None,
            }),
        )
            .into_response();
    }

    let new_password_hash = match Argon2::default().hash_password(form.new_password.as_bytes()) {
        Ok(h) => h.to_string(),
        Err(e) => {
            tracing::error!("Password hashing error: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(AccountPasswordTemplate {
                    csrf_token,
                    error: Some("Failed to hash new password.".to_string()),
                    success: None,
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(new_password_hash)
        .bind(user.id)
        .execute(&state.db)
        .await
    {
        tracing::error!("Failed to update password in database: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            HtmlTemplate(AccountPasswordTemplate {
                csrf_token,
                error: Some("Failed to update password.".to_string()),
                success: None,
            }),
        )
            .into_response();
    }

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "password_change",
        "user",
        Some(user.id),
        None,
    )
    .await;

    HtmlTemplate(AccountPasswordTemplate {
        csrf_token,
        error: None,
        success: Some("Password updated successfully.".to_string()),
    })
    .into_response()
}
