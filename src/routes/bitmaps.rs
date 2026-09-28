use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use sqlx::Row;

use super::auth::HtmlTemplate;
use crate::auth::AuthUser;
use crate::magnifier::ImageMagnifier;
use crate::AppState;

#[derive(Debug, Clone)]
pub struct BitmapListItem {
    pub id: i64,
    pub automation_id: Option<i64>,
    pub automation_name: Option<String>,
    pub name: String,
    pub object_storage_key: String,
    pub width: i32,
    pub height: i32,
    pub created_by: i64,
    pub created_by_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub magnifier: ImageMagnifier,
}

impl BitmapListItem {
    pub fn formatted_created_at(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

#[derive(Template)]
#[template(path = "bitmaps/list.html")]
pub struct BitmapsListTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub bitmaps: Vec<BitmapListItem>,
    pub automation_id: Option<i64>,
    pub automation_name: Option<String>,
}

/// GET /bitmaps
/// Renders the list page for all reference bitmaps in the system.
pub async fn get_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let rows = match sqlx::query(
        r#"
        SELECT
            b.id,
            b.automation_id,
            a.name AS automation_name,
            b.name,
            b.object_storage_key,
            b.width,
            b.height,
            b.created_by,
            COALESCE(u.display_name, u.username, 'Unknown') AS created_by_name,
            b.created_at
        FROM bitmaps b
        LEFT JOIN automations a ON b.automation_id = a.id
        LEFT JOIN users u ON b.created_by = u.id
        ORDER BY b.created_at DESC, b.id DESC
        "#,
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Error fetching bitmaps: {}", e);
            Vec::new()
        }
    };

    let bitmaps = rows
        .into_iter()
        .map(|row| {
            let id: i64 = row.get("id");
            let width: i32 = row.get("width");
            let height: i32 = row.get("height");

            let magnifier = ImageMagnifier::new(
                format!("/media/bitmaps/{}", id),
                width.max(1) as u32,
                height.max(1) as u32,
                None,
                None,
            );

            BitmapListItem {
                id,
                automation_id: row.get("automation_id"),
                automation_name: row.get("automation_name"),
                name: row.get("name"),
                object_storage_key: row.get("object_storage_key"),
                width,
                height,
                created_by: row.get("created_by"),
                created_by_name: row.get("created_by_name"),
                created_at: row.get("created_at"),
                magnifier,
            }
        })
        .collect();

    HtmlTemplate(BitmapsListTemplate {
        user,
        csrf_token,
        bitmaps,
        automation_id: None,
        automation_name: None,
    })
}

/// GET /automations/{id}/bitmaps
/// Renders the reference bitmaps list page scoped to a specific automation.
pub async fn get_automation_bitmaps_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let csrf_token = user.csrf_token.clone();

    let auto_row = match sqlx::query("SELECT name FROM automations WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        _ => return Redirect::to("/automations").into_response(),
    };
    let automation_name: String = auto_row.get("name");

    let rows = match sqlx::query(
        r#"
        SELECT
            b.id,
            b.automation_id,
            a.name AS automation_name,
            b.name,
            b.object_storage_key,
            b.width,
            b.height,
            b.created_by,
            COALESCE(u.display_name, u.username, 'Unknown') AS created_by_name,
            b.created_at
        FROM bitmaps b
        LEFT JOIN automations a ON b.automation_id = a.id
        LEFT JOIN users u ON b.created_by = u.id
        WHERE b.automation_id = $1 OR b.automation_id IS NULL
        ORDER BY b.created_at DESC, b.id DESC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Error fetching automation bitmaps: {}", e);
            Vec::new()
        }
    };

    let bitmaps = rows
        .into_iter()
        .map(|row| {
            let bitmap_id: i64 = row.get("id");
            let width: i32 = row.get("width");
            let height: i32 = row.get("height");

            let magnifier = ImageMagnifier::new(
                format!("/media/bitmaps/{}", bitmap_id),
                width.max(1) as u32,
                height.max(1) as u32,
                None,
                None,
            );

            BitmapListItem {
                id: bitmap_id,
                automation_id: row.get("automation_id"),
                automation_name: row.get("automation_name"),
                name: row.get("name"),
                object_storage_key: row.get("object_storage_key"),
                width,
                height,
                created_by: row.get("created_by"),
                created_by_name: row.get("created_by_name"),
                created_at: row.get("created_at"),
                magnifier,
            }
        })
        .collect();

    HtmlTemplate(BitmapsListTemplate {
        user,
        csrf_token,
        bitmaps,
        automation_id: Some(id),
        automation_name: Some(automation_name),
    })
    .into_response()
}
