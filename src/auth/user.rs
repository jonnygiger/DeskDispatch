use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::str::FromStr;
use uuid::Uuid;

use crate::AppState;

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

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub role: UserRole,
    pub session_id: Uuid,
    pub csrf_token: String,
}

#[derive(FromRow)]
struct SessionUserRow {
    id: i64,
    username: String,
    display_name: String,
    role: String,
    session_id: Uuid,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

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
            SELECT u.id, u.username, u.display_name, u.role, s.id as session_id
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

        let csrf_token = super::csrf::generate_csrf_token(record.session_id, &state.config.session_secret);

        Ok(AuthUser {
            id: record.id,
            username: record.username,
            display_name: record.display_name,
            role,
            session_id: record.session_id,
            csrf_token,
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
            Err((
                StatusCode::FORBIDDEN,
                "Forbidden: Admin permission required",
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
            Err((
                StatusCode::FORBIDDEN,
                "Forbidden: Editor permission required",
            )
                .into_response())
        }
    }
}
