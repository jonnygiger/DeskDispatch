use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::Row;
use std::str::FromStr;

use super::auth::HtmlTemplate;
use crate::auth::{
    hash_password_async, log_audit, AuthUser, CsrfForm, RequireAdmin, UserRole,
};
use axum::routing::{get, post};
use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", get(get_users_handler).post(post_create_user_handler))
        .route("/users/new", get(get_new_user_handler))
        .route("/users/{id}", post(post_edit_user_handler))
        .route("/users/{id}/edit", get(get_edit_user_handler))
        .route("/users/{id}/deactivate", post(post_deactivate_user_handler))
        .route("/users/{id}/reset-password", get(get_reset_password_handler).post(post_reset_password_handler))
}

#[derive(Debug, Clone)]
pub struct UserListItem {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub role: UserRole,
    pub is_active: bool,
    pub must_change_password: bool,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

impl UserListItem {
    pub fn formatted_created_at(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
    }

    pub fn formatted_last_login(&self) -> String {
        match self.last_login_at {
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            None => "Never".to_string(),
        }
    }

    pub fn status_badge_class(&self) -> &'static str {
        if self.is_active {
            "badge-success"
        } else {
            "badge-danger"
        }
    }

    pub fn role_badge_class(&self) -> &'static str {
        match self.role {
            UserRole::Admin => "badge-primary",
            UserRole::Editor => "badge-neutral",
            UserRole::Viewer => "badge-neutral",
        }
    }
}

#[derive(Template)]
#[template(path = "users/index.html")]
pub struct UsersIndexTemplate {
    pub user: AuthUser,
    pub users: Vec<UserListItem>,
    pub error: Option<String>,
    pub success: Option<String>,
}

#[derive(Template)]
#[template(path = "users/new.html")]
pub struct NewUserTemplate {
    pub user: AuthUser,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub must_change_password: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateUserForm {
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub password: String,
    pub confirm_password: String,
    #[serde(default)]
    pub must_change_password: bool,
}

#[derive(Template)]
#[template(path = "users/edit.html")]
pub struct EditUserTemplate {
    pub user: AuthUser,
    pub target_user_id: i64,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub must_change_password: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct EditUserForm {
    pub display_name: String,
    pub role: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub must_change_password: bool,
}

#[derive(Template)]
#[template(path = "users/reset_password.html")]
pub struct ResetPasswordTemplate {
    pub user: AuthUser,
    pub target_user_id: i64,
    pub target_username: String,
    pub target_display_name: String,
    pub must_change_password: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct ResetPasswordForm {
    pub new_password: String,
    pub confirm_password: String,
    #[serde(default)]
    pub must_change_password: bool,
}

#[tracing::instrument(skip(state, user))]
pub async fn get_users_handler(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let rows = match sqlx::query(
        r#"
        SELECT id, username, display_name, role, is_active, must_change_password, created_at, last_login_at
        FROM users
        ORDER BY id ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to fetch users: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load users",
            )
                .into_response();
        }
    };

    let users = rows
        .into_iter()
        .filter_map(|r| {
            let role_str: String = r.get("role");
            let role = UserRole::from_str(&role_str).ok()?;
            Some(UserListItem {
                id: r.get("id"),
                username: r.get("username"),
                display_name: r.get("display_name"),
                role,
                is_active: r.get("is_active"),
                must_change_password: r.get("must_change_password"),
                created_at: r.get("created_at"),
                last_login_at: r.get("last_login_at"),
            })
        })
        .collect();

    HtmlTemplate(UsersIndexTemplate {
        user,
        users,
        error: None,
        success: None,
    })
    .into_response()
}

#[tracing::instrument(skip(user))]
pub async fn get_new_user_handler(
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    HtmlTemplate(NewUserTemplate {
        user,
        username: String::new(),
        display_name: String::new(),
        role: "editor".to_string(),
        must_change_password: true,
        error: None,
    })
    .into_response()
}

#[tracing::instrument(skip(state, user, form))]
pub async fn post_create_user_handler(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    CsrfForm(form): CsrfForm<CreateUserForm>,
) -> impl IntoResponse {
    let username = form.username.trim();
    let display_name = form.display_name.trim();
    let role_str = form.role.trim();

    if username.is_empty() || display_name.is_empty() {
        return HtmlTemplate(NewUserTemplate {
            user,
            username: username.to_string(),
            display_name: display_name.to_string(),
            role: role_str.to_string(),
            must_change_password: form.must_change_password,
            error: Some("Username and Display Name are required.".to_string()),
        })
        .into_response();
    }

    if UserRole::from_str(role_str).is_err() {
        return HtmlTemplate(NewUserTemplate {
            user,
            username: username.to_string(),
            display_name: display_name.to_string(),
            role: role_str.to_string(),
            must_change_password: form.must_change_password,
            error: Some("Invalid role selected.".to_string()),
        })
        .into_response();
    }

    if form.password != form.confirm_password {
        return HtmlTemplate(NewUserTemplate {
            user,
            username: username.to_string(),
            display_name: display_name.to_string(),
            role: role_str.to_string(),
            must_change_password: form.must_change_password,
            error: Some("Passwords do not match.".to_string()),
        })
        .into_response();
    }

    if form.password.len() < 8 {
        return HtmlTemplate(NewUserTemplate {
            user,
            username: username.to_string(),
            display_name: display_name.to_string(),
            role: role_str.to_string(),
            must_change_password: form.must_change_password,
            error: Some("Password must be at least 8 characters long.".to_string()),
        })
        .into_response();
    }

    let exists: bool = match sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE lower(username) = lower($1))",
    )
    .bind(username)
    .fetch_one(&state.db)
    .await
    {
        Ok(ex) => ex,
        Err(e) => {
            tracing::error!("Database check error for existing username: {}", e);
            return HtmlTemplate(NewUserTemplate {
                user,
                username: username.to_string(),
                display_name: display_name.to_string(),
                role: role_str.to_string(),
                must_change_password: form.must_change_password,
                error: Some("Database error checking username availability.".to_string()),
            })
            .into_response();
        }
    };

    if exists {
        return HtmlTemplate(NewUserTemplate {
            user,
            username: username.to_string(),
            display_name: display_name.to_string(),
            role: role_str.to_string(),
            must_change_password: form.must_change_password,
            error: Some(format!("Username '{}' is already taken.", username)),
        })
        .into_response();
    }

    let password_hash = match hash_password_async(
        state.rate_limiter.argon2_semaphore.clone(),
        form.password,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Password hashing error: {}", e);
            return HtmlTemplate(NewUserTemplate {
                user,
                username: username.to_string(),
                display_name: display_name.to_string(),
                role: role_str.to_string(),
                must_change_password: form.must_change_password,
                error: Some("Failed to hash password.".to_string()),
            })
            .into_response();
        }
    };

    let row = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, display_name, role, is_active, must_change_password)
        VALUES ($1, $2, $3, $4, true, $5)
        RETURNING id
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(display_name)
    .bind(role_str)
    .bind(form.must_change_password)
    .fetch_one(&state.db)
    .await;

    let new_user_id: i64 = match row {
        Ok(r) => r.get("id"),
        Err(e) => {
            tracing::error!("Failed to insert user: {}", e);
            return HtmlTemplate(NewUserTemplate {
                user,
                username: username.to_string(),
                display_name: display_name.to_string(),
                role: role_str.to_string(),
                must_change_password: form.must_change_password,
                error: Some("Failed to create user in database.".to_string()),
            })
            .into_response();
        }
    };

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "create",
        "user",
        Some(new_user_id),
        Some(serde_json::json!({
            "username": username,
            "display_name": display_name,
            "role": role_str,
            "must_change_password": form.must_change_password,
        })),
    )
    .await;

    Redirect::to("/users").into_response()
}

#[tracing::instrument(skip(state, user))]
pub async fn get_edit_user_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let row = match sqlx::query(
        "SELECT id, username, display_name, role, is_active, must_change_password FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch user: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    HtmlTemplate(EditUserTemplate {
        user,
        target_user_id: row.get("id"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        role: row.get("role"),
        is_active: row.get("is_active"),
        must_change_password: row.get("must_change_password"),
        error: None,
    })
    .into_response()
}

#[tracing::instrument(skip(state, user, form))]
pub async fn post_edit_user_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
    CsrfForm(form): CsrfForm<EditUserForm>,
) -> impl IntoResponse {
    let display_name = form.display_name.trim();
    let new_role_str = form.role.trim();

    let target_row = match sqlx::query(
        "SELECT id, username, role, is_active FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch target user: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let target_username: String = target_row.get("username");
    let current_role_str: String = target_row.get("role");
    let current_is_active: bool = target_row.get("is_active");

    if display_name.is_empty() {
        return HtmlTemplate(EditUserTemplate {
            user,
            target_user_id: id,
            username: target_username,
            display_name: display_name.to_string(),
            role: new_role_str.to_string(),
            is_active: form.is_active,
            must_change_password: form.must_change_password,
            error: Some("Display Name is required.".to_string()),
        })
        .into_response();
    }

    if UserRole::from_str(new_role_str).is_err() {
        return HtmlTemplate(EditUserTemplate {
            user,
            target_user_id: id,
            username: target_username,
            display_name: display_name.to_string(),
            role: new_role_str.to_string(),
            is_active: form.is_active,
            must_change_password: form.must_change_password,
            error: Some("Invalid role selected.".to_string()),
        })
        .into_response();
    }

    // Protection of the last active admin
    if current_role_str == "admin" && current_is_active {
        let is_removing_admin = new_role_str != "admin" || !form.is_active;
        if is_removing_admin {
            let active_admins: i64 = match sqlx::query_scalar(
                "SELECT COUNT(*) FROM users WHERE role = 'admin' AND is_active = true",
            )
            .fetch_one(&state.db)
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Error counting active admins: {}", e);
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
                }
            };

            if active_admins <= 1 {
                return HtmlTemplate(EditUserTemplate {
                    user,
                    target_user_id: id,
                    username: target_username,
                    display_name: display_name.to_string(),
                    role: new_role_str.to_string(),
                    is_active: form.is_active,
                    must_change_password: form.must_change_password,
                    error: Some(
                        "Cannot deactivate or downgrade the only active admin user.".to_string(),
                    ),
                })
                .into_response();
            }
        }
    }

    if !form.is_active {
        // Revoke active sessions when user is deactivated
        if let Err(e) = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(id)
            .execute(&state.db)
            .await
        {
            tracing::error!("Failed to revoke sessions for deactivated user: {}", e);
        }
    }

    let update_res = sqlx::query(
        r#"
        UPDATE users
        SET display_name = $1, role = $2, is_active = $3, must_change_password = $4
        WHERE id = $5
        "#,
    )
    .bind(display_name)
    .bind(new_role_str)
    .bind(form.is_active)
    .bind(form.must_change_password)
    .bind(id)
    .execute(&state.db)
    .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to update user: {}", e);
        return HtmlTemplate(EditUserTemplate {
            user,
            target_user_id: id,
            username: target_username,
            display_name: display_name.to_string(),
            role: new_role_str.to_string(),
            is_active: form.is_active,
            must_change_password: form.must_change_password,
            error: Some("Failed to update user in database.".to_string()),
        })
        .into_response();
    }

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "update",
        "user",
        Some(id),
        Some(serde_json::json!({
            "display_name": display_name,
            "role": new_role_str,
            "is_active": form.is_active,
            "must_change_password": form.must_change_password,
        })),
    )
    .await;

    Redirect::to("/users").into_response()
}

#[tracing::instrument(skip(state, user))]
pub async fn post_deactivate_user_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let target_row = match sqlx::query("SELECT role, is_active FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch target user for deactivation: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let role_str: String = target_row.get("role");
    let is_active: bool = target_row.get("is_active");

    if role_str == "admin" && is_active {
        let active_admins: i64 = match sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE role = 'admin' AND is_active = true",
        )
        .fetch_one(&state.db)
        .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Error counting active admins: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
            }
        };

        if active_admins <= 1 {
            return (
                StatusCode::BAD_REQUEST,
                "Cannot deactivate the only active admin user.",
            )
                .into_response();
        }
    }

    let deact_res = sqlx::query("UPDATE users SET is_active = false WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    if let Err(e) = deact_res {
        tracing::error!("Failed to deactivate user: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    // Revoke sessions
    let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "deactivate",
        "user",
        Some(id),
        None,
    )
    .await;

    Redirect::to("/users").into_response()
}

#[tracing::instrument(skip(state, user))]
pub async fn get_reset_password_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
) -> impl IntoResponse {
    let row = match sqlx::query(
        "SELECT id, username, display_name FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch user for password reset: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    HtmlTemplate(ResetPasswordTemplate {
        user,
        target_user_id: row.get("id"),
        target_username: row.get("username"),
        target_display_name: row.get("display_name"),
        must_change_password: true,
        error: None,
    })
    .into_response()
}

#[tracing::instrument(skip(state, user, form))]
pub async fn post_reset_password_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    RequireAdmin(user): RequireAdmin,
    CsrfForm(form): CsrfForm<ResetPasswordForm>,
) -> impl IntoResponse {
    let row = match sqlx::query(
        "SELECT id, username, display_name FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to fetch target user: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let target_username: String = row.get("username");
    let target_display_name: String = row.get("display_name");

    if form.new_password != form.confirm_password {
        return HtmlTemplate(ResetPasswordTemplate {
            user,
            target_user_id: id,
            target_username,
            target_display_name,
            must_change_password: form.must_change_password,
            error: Some("Passwords do not match.".to_string()),
        })
        .into_response();
    }

    if form.new_password.len() < 8 {
        return HtmlTemplate(ResetPasswordTemplate {
            user,
            target_user_id: id,
            target_username,
            target_display_name,
            must_change_password: form.must_change_password,
            error: Some("Password must be at least 8 characters long.".to_string()),
        })
        .into_response();
    }

    let password_hash = match hash_password_async(
        state.rate_limiter.argon2_semaphore.clone(),
        form.new_password,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Password hashing error: {}", e);
            return HtmlTemplate(ResetPasswordTemplate {
                user,
                target_user_id: id,
                target_username,
                target_display_name,
                must_change_password: form.must_change_password,
                error: Some("Failed to hash new password.".to_string()),
            })
            .into_response();
        }
    };

    let update_res = sqlx::query(
        "UPDATE users SET password_hash = $1, must_change_password = $2 WHERE id = $3",
    )
    .bind(password_hash)
    .bind(form.must_change_password)
    .bind(id)
    .execute(&state.db)
    .await;

    if let Err(e) = update_res {
        tracing::error!("Failed to reset password: {}", e);
        return HtmlTemplate(ResetPasswordTemplate {
            user,
            target_user_id: id,
            target_username,
            target_display_name,
            must_change_password: form.must_change_password,
            error: Some("Failed to update password in database.".to_string()),
        })
        .into_response();
    }

    // Revoke target user's active sessions
    let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    let _ = log_audit(
        &state.db,
        Some(user.id),
        "reset_password",
        "user",
        Some(id),
        Some(serde_json::json!({
            "must_change_password": form.must_change_password,
        })),
    )
    .await;

    Redirect::to("/users").into_response()
}
