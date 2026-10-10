use argon2::{Argon2, PasswordHasher, PasswordVerifier, password_hash::phc::PasswordHash};
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::AppState;

pub async fn verify_password_async(
    semaphore: Arc<Semaphore>,
    password: String,
    password_hash: String,
) -> Result<bool, String> {
    let permit = semaphore
        .acquire_owned()
        .await
        .map_err(|e| format!("Failed to acquire argon2 semaphore permit: {}", e))?;

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let parsed_hash = match PasswordHash::new(&password_hash) {
            Ok(h) => h,
            Err(e) => return Err(format!("Invalid password hash format: {}", e)),
        };
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    })
    .await
    .map_err(|e| format!("Password verification task panicked: {}", e))?
}

pub async fn hash_password_async(
    semaphore: Arc<Semaphore>,
    password: String,
) -> Result<String, String> {
    let permit = semaphore
        .acquire_owned()
        .await
        .map_err(|e| format!("Failed to acquire argon2 semaphore permit: {}", e))?;

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| format!("Password hashing error: {}", e))
    })
    .await
    .map_err(|e| format!("Password hashing task panicked: {}", e))?
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    Editor,
    Viewer,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Admin => "admin",
            UserRole::Editor => "editor",
            UserRole::Viewer => "viewer",
        }
    }

    pub fn can_edit(&self) -> bool {
        matches!(self, UserRole::Admin | UserRole::Editor)
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, UserRole::Admin)
    }
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for UserRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "admin" => Ok(UserRole::Admin),
            "editor" => Ok(UserRole::Editor),
            "viewer" => Ok(UserRole::Viewer),
            _ => Err(format!("Unknown user role: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_async_password_hashing_and_verification() {
        let sem = Arc::new(Semaphore::new(2));
        let password = "AsyncPassword123!".to_string();

        let hash = hash_password_async(sem.clone(), password.clone())
            .await
            .unwrap();

        assert!(!hash.is_empty());

        let is_valid = verify_password_async(sem.clone(), password, hash.clone())
            .await
            .unwrap();
        assert!(is_valid);

        let is_wrong_valid = verify_password_async(sem.clone(), "WrongPassword!".to_string(), hash)
            .await
            .unwrap();
        assert!(!is_wrong_valid);
    }
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub role: UserRole,
    pub session_id: Uuid,
    pub csrf_token: String,
    pub must_change_password: bool,
}

#[derive(Debug, Clone)]
pub struct OptionalAuthUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for OptionalAuthUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match AuthUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(OptionalAuthUser(Some(user))),
            Err(_) => Ok(OptionalAuthUser(None)),
        }
    }
}

#[derive(FromRow)]
struct SessionUserRow {
    id: i64,
    username: String,
    display_name: String,
    role: String,
    session_id: Uuid,
    must_change_password: bool,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    #[tracing::instrument(skip(parts, state))]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let cookie_header = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|h| h.to_str().ok());

        let session_id = cookie_header
            .and_then(super::session::extract_session_id)
            .ok_or_else(|| Redirect::to("/login").into_response())?;

        let record = sqlx::query_as::<_, SessionUserRow>(
            r#"
            SELECT u.id, u.username, u.display_name, u.role, s.id as session_id, u.must_change_password
            FROM sessions s
            JOIN users u ON s.user_id = u.id
            WHERE s.id = $1 AND s.expires_at > now() AND u.is_active = true
            "#,
        )
        .bind(session_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Database query error in AuthUser extractor: {}", e);
            Redirect::to("/login").into_response()
        })?
        .ok_or_else(|| Redirect::to("/login").into_response())?;

        let role = UserRole::from_str(&record.role).map_err(|e| {
            tracing::error!("Invalid role in database: {}", e);
            Redirect::to("/login").into_response()
        })?;

        let csrf_token = super::csrf::generate_csrf_token(
            record.session_id,
            state.config.session_secret.expose_secret(),
        );

        if record.must_change_password
            && parts.uri.path() != "/account/password"
            && parts.uri.path() != "/logout"
        {
            return Err(Redirect::to("/account/password").into_response());
        }

        Ok(AuthUser {
            id: record.id,
            username: record.username,
            display_name: record.display_name,
            role,
            session_id: record.session_id,
            csrf_token,
            must_change_password: record.must_change_password,
        })
    }
}

pub struct RequireAdmin(pub AuthUser);

impl FromRequestParts<AppState> for RequireAdmin {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role.is_admin() {
            Ok(RequireAdmin(user))
        } else {
            let body = crate::routes::errors::ForbiddenTemplate {
                message: Some("Admin permission required".to_string()),
                user: Some(user),
            };
            Err((
                StatusCode::FORBIDDEN,
                crate::routes::auth::HtmlTemplate(body),
            )
                .into_response())
        }
    }
}

pub struct RequireEditor(pub AuthUser);

impl FromRequestParts<AppState> for RequireEditor {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role.can_edit() {
            Ok(RequireEditor(user))
        } else {
            let body = crate::routes::errors::ForbiddenTemplate {
                message: Some("Editor or Admin permission required".to_string()),
                user: Some(user),
            };
            Err((
                StatusCode::FORBIDDEN,
                crate::routes::auth::HtmlTemplate(body),
            )
                .into_response())
        }
    }
}
