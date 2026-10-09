use askama::Template;
use serde::Deserialize;
use crate::auth::AuthUser;
use crate::de::deserialize_option_number;

#[derive(Debug, Deserialize)]
pub struct AutomationsListQuery {
    pub status: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct AutomationListItem {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub step_count: i64,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub last_run_id: Option<i64>,
    pub last_run_status: Option<String>,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AutomationListItem {
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "active" => "badge-success",
            "archived" => "badge-neutral",
            "draft" => "badge-warning",
            _ => "badge-neutral",
        }
    }

    pub fn last_run_status_badge_class(&self) -> &'static str {
        match self.last_run_status.as_deref() {
            Some("succeeded") => "badge-success",
            Some("failed") | Some("lost") => "badge-danger",
            Some("running") | Some("cancelling") => "badge-warning",
            _ => "badge-neutral",
        }
    }

    pub fn formatted_updated_at(&self) -> String {
        self.updated_at.format("%Y-%m-%d %H:%M:%S").to_string()
    }

    pub fn formatted_last_run_at(&self) -> String {
        match self.last_run_at {
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => "-".to_string(),
        }
    }
}

#[derive(Template)]
#[template(path = "automations/list.html")]
pub struct AutomationsListTemplate {
    pub user: AuthUser,
    pub automations: Vec<AutomationListItem>,
    pub status_filter: String,
    pub search_query: String,
}

#[derive(Template)]
#[template(path = "automations/new.html")]
pub struct AutomationNewTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub error: Option<String>,
    pub name: String,
    pub description: String,
}

#[derive(Deserialize)]
pub struct CreateAutomationForm {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AutomationDetail {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub created_by: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl AutomationDetail {
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "active" => "badge-success",
            "archived" => "badge-neutral",
            "draft" => "badge-warning",
            _ => "badge-neutral",
        }
    }
}

#[derive(Debug)]
pub struct StepViewItem {
    pub id: i64,
    pub step_number: usize,
    pub step_type: String,
    pub label: Option<String>,
    pub post_delay_ms: i32,
    pub position: f64,
    pub description: String,
}

impl StepViewItem {
    pub fn formatted_post_delay(&self) -> String {
        if self.post_delay_ms > 0 {
            format!(" · then wait {:.1}s", self.post_delay_ms as f64 / 1000.0)
        } else {
            String::new()
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, sqlx::FromRow)]
pub struct WorkerGroupOption {
    pub id: i64,
    pub name: String,
}

impl WorkerGroupOption {
    pub fn is_selected(&self, selected_id: &Option<i64>) -> bool {
        *selected_id == Some(self.id)
    }
}

#[derive(Template)]
#[template(path = "automations/detail.html")]
pub struct AutomationDetailTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation: AutomationDetail,
    pub steps: Vec<StepViewItem>,
    pub worker_groups: Vec<WorkerGroupOption>,
    pub parameters: Vec<AutomationParameterItem>,
    pub active_tab: String,
    pub error: Option<String>,
    pub success: Option<String>,
}

#[derive(Deserialize)]
pub struct EditAutomationForm {
    pub name: String,
    pub description: Option<String>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct RunNowForm {
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub worker_group_id: Option<i64>,
    #[serde(default)]
    pub parameters: std::collections::HashMap<String, String>,
}

#[derive(Template)]
#[template(path = "automations/delete.html")]
pub struct AutomationDeleteTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation: AutomationDetail,
}

#[derive(Template)]
#[template(path = "automations/step_type_picker.html")]
pub struct StepTypePickerTemplate {
    pub user: AuthUser,
    pub automation_id: i64,
}

#[derive(Debug, Clone, serde::Deserialize, sqlx::FromRow)]
pub struct AutomationVariableItem {
    pub id: i64,
    pub automation_id: i64,
    pub name: String,
    pub var_type: String,
    pub description: String,
}

#[derive(Template)]
#[template(path = "automations/variables.html")]
pub struct AutomationVariablesTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation: AutomationDetail,
    pub variables: Vec<AutomationVariableItem>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct VariableForm {
    pub name: String,
    pub var_type: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, sqlx::FromRow)]
pub struct AutomationParameterItem {
    pub id: i64,
    pub automation_id: i64,
    pub name: String,
    pub param_type: String,
    pub default_value: String,
    pub description: String,
}

#[derive(Template)]
#[template(path = "automations/parameters.html")]
pub struct AutomationParametersTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation: AutomationDetail,
    pub parameters: Vec<AutomationParameterItem>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct ParameterForm {
    pub name: String,
    pub param_type: String,
    pub default_value: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, sqlx::FromRow)]
pub struct VariableOption {
    pub id: i64,
    pub name: String,
    pub var_type: String,
}

impl VariableOption {
    pub fn is_selected_x(&self, x_var_id: &Option<i64>) -> bool {
        *x_var_id == Some(self.id)
    }

    pub fn is_selected_y(&self, y_var_id: &Option<i64>) -> bool {
        *y_var_id == Some(self.id)
    }

    pub fn is_selected_output(&self, output_var_id: &Option<i64>) -> bool {
        *output_var_id == Some(self.id)
    }

    pub fn is_selected_found(&self, found_var_id: &Option<i64>) -> bool {
        *found_var_id == Some(self.id)
    }
}

#[derive(Debug, Clone, serde::Deserialize, sqlx::FromRow)]
pub struct BitmapOption {
    pub id: i64,
    pub name: String,
    pub width: i32,
    pub height: i32,
}

impl BitmapOption {
    pub fn is_selected(&self, selected_id: &Option<i64>) -> bool {
        *selected_id == Some(self.id)
    }
}

#[derive(Debug, Clone, serde::Deserialize, sqlx::FromRow)]
pub struct StepOption {
    pub id: i64,
    pub step_type: String,
    pub label: Option<String>,
    pub position: f64,
    pub display_number: usize,
}

impl StepOption {
    pub fn is_selected_match(&self, match_id: &Option<i64>) -> bool {
        *match_id == Some(self.id)
    }

    pub fn is_selected_no_match(&self, no_match_id: &Option<i64>) -> bool {
        *no_match_id == Some(self.id)
    }

    pub fn display_name(&self) -> String {
        match &self.label {
            Some(lbl) if !lbl.trim().is_empty() => format!("Step {} ({})", self.display_number, lbl.trim()),
            _ => format!("Step {} ({})", self.display_number, self.step_type),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct NewMouseClickQuery {
    pub x: Option<i32>,
    pub y: Option<i32>,
}

#[derive(Template)]
#[template(path = "automations/step_mouse_click.html")]
pub struct StepMouseClickTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation_id: i64,
    pub step_id: Option<i64>,
    pub label: String,
    pub post_delay_seconds: f64,
    pub x_mode: String,
    pub x: Option<i32>,
    pub x_variable_id: Option<i64>,
    pub y_mode: String,
    pub y: Option<i32>,
    pub y_variable_id: Option<i64>,
    pub button: String,
    pub click_type: String,
    pub variables: Vec<VariableOption>,
    pub error: Option<String>,
    pub is_edit: bool,
}

#[derive(Template)]
#[template(path = "automations/step_key_press.html")]
pub struct StepKeyPressTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation_id: i64,
    pub step_id: Option<i64>,
    pub label: String,
    pub post_delay_seconds: f64,
    pub key_combo: String,
    pub error: Option<String>,
    pub is_edit: bool,
}

#[derive(Deserialize)]
pub struct KeyPressStepForm {
    pub label: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub post_delay_seconds: Option<f64>,
    pub key_combo: String,
}

#[derive(Debug, Deserialize)]
pub struct NewFindPixelRgbQuery {
    pub x: Option<i32>,
    pub y: Option<i32>,
}

#[derive(Template)]
#[template(path = "automations/step_find_pixel_rgb.html")]
pub struct StepFindPixelRgbTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation_id: i64,
    pub step_id: Option<i64>,
    pub label: String,
    pub post_delay_seconds: f64,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub output_variable_id: Option<i64>,
    pub variables: Vec<VariableOption>,
    pub error: Option<String>,
    pub is_edit: bool,
}

#[derive(Debug, Deserialize)]
pub struct NewFindBitmapQuery {
    pub reference_bitmap_id: Option<i64>,
    pub search_x: Option<i32>,
    pub search_y: Option<i32>,
    pub search_width: Option<i32>,
    pub search_height: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct NewBranchQuery {
    pub condition_type: Option<String>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub reference_bitmap_id: Option<i64>,
    pub search_x: Option<i32>,
    pub search_y: Option<i32>,
    pub search_width: Option<i32>,
    pub search_height: Option<i32>,
}

#[derive(Template)]
#[template(path = "automations/step_branch.html")]
pub struct StepBranchTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation_id: i64,
    pub step_id: Option<i64>,
    pub label: String,
    pub post_delay_seconds: f64,
    pub condition_type: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub expected_r: Option<i16>,
    pub expected_g: Option<i16>,
    pub expected_b: Option<i16>,
    pub tolerance: Option<i16>,
    pub reference_bitmap_id: Option<i64>,
    pub search_x: Option<i32>,
    pub search_y: Option<i32>,
    pub search_width: Option<i32>,
    pub search_height: Option<i32>,
    pub match_threshold: Option<f32>,
    pub on_match_step_id: Option<i64>,
    pub on_no_match_step_id: Option<i64>,
    pub steps: Vec<StepOption>,
    pub bitmaps: Vec<BitmapOption>,
    pub error: Option<String>,
    pub is_edit: bool,
}

#[derive(Template)]
#[template(path = "automations/step_find_bitmap.html")]
pub struct StepFindBitmapTemplate {
    pub user: AuthUser,
    pub csrf_token: String,
    pub automation_id: i64,
    pub step_id: Option<i64>,
    pub label: String,
    pub post_delay_seconds: f64,
    pub reference_bitmap_id: Option<i64>,
    pub search_x: Option<i32>,
    pub search_y: Option<i32>,
    pub search_width: Option<i32>,
    pub search_height: Option<i32>,
    pub match_threshold: f32,
    pub output_found_variable_id: Option<i64>,
    pub output_x_variable_id: Option<i64>,
    pub output_y_variable_id: Option<i64>,
    pub bitmaps: Vec<BitmapOption>,
    pub variables: Vec<VariableOption>,
    pub error: Option<String>,
    pub is_edit: bool,
}

#[derive(Deserialize)]
pub struct MouseClickStepForm {
    pub step_type: Option<String>,
    pub label: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub post_delay_seconds: Option<f64>,
    pub x_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub x: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub x_variable_id: Option<i64>,
    pub y_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub y: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub y_variable_id: Option<i64>,
    pub button: Option<String>,
    pub click_type: Option<String>,
    pub key_combo: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub output_variable_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub reference_bitmap_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub search_x: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub search_y: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub search_width: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub search_height: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub match_threshold: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub output_found_variable_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub output_x_variable_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub output_y_variable_id: Option<i64>,
    pub condition_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub expected_r: Option<i16>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub expected_g: Option<i16>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub expected_b: Option<i16>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub tolerance: Option<i16>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub on_match_step_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_option_number")]
    pub on_no_match_step_id: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_automation_list_item_status_badge_class() {
        let item = |status: &str| AutomationListItem {
            id: 1,
            name: "Test".to_string(),
            description: "".to_string(),
            status: status.to_string(),
            step_count: 5,
            updated_at: Utc::now(),
            last_run_id: None,
            last_run_status: None,
            last_run_at: None,
        };

        assert_eq!(item("active").status_badge_class(), "badge-success");
        assert_eq!(item("archived").status_badge_class(), "badge-neutral");
        assert_eq!(item("draft").status_badge_class(), "badge-warning");
        assert_eq!(item("unknown").status_badge_class(), "badge-neutral");
        assert_eq!(item("").status_badge_class(), "badge-neutral");
    }

    #[test]
    fn test_automation_list_item_last_run_status_badge_class() {
        let item = |last_status: Option<&str>| AutomationListItem {
            id: 1,
            name: "Test".to_string(),
            description: "".to_string(),
            status: "active".to_string(),
            step_count: 5,
            updated_at: Utc::now(),
            last_run_id: Some(10),
            last_run_status: last_status.map(|s| s.to_string()),
            last_run_at: None,
        };

        assert_eq!(item(Some("succeeded")).last_run_status_badge_class(), "badge-success");
        assert_eq!(item(Some("failed")).last_run_status_badge_class(), "badge-danger");
        assert_eq!(item(Some("lost")).last_run_status_badge_class(), "badge-danger");
        assert_eq!(item(Some("running")).last_run_status_badge_class(), "badge-warning");
        assert_eq!(item(Some("cancelling")).last_run_status_badge_class(), "badge-warning");
        assert_eq!(item(Some("other")).last_run_status_badge_class(), "badge-neutral");
        assert_eq!(item(None).last_run_status_badge_class(), "badge-neutral");
    }

    #[test]
    fn test_automation_list_item_formatted_dates() {
        let dt = Utc.with_ymd_and_hms(2025, 3, 10, 14, 30, 45).unwrap();
        let item = AutomationListItem {
            id: 1,
            name: "Test".to_string(),
            description: "".to_string(),
            status: "active".to_string(),
            step_count: 5,
            updated_at: dt,
            last_run_id: Some(10),
            last_run_status: Some("succeeded".to_string()),
            last_run_at: Some(dt),
        };

        assert_eq!(item.formatted_updated_at(), "2025-03-10 14:30:45");
        assert_eq!(item.formatted_last_run_at(), "2025-03-10 14:30:45");

        let item_no_last_run = AutomationListItem {
            id: 1,
            name: "Test".to_string(),
            description: "".to_string(),
            status: "active".to_string(),
            step_count: 0,
            updated_at: dt,
            last_run_id: None,
            last_run_status: None,
            last_run_at: None,
        };
        assert_eq!(item_no_last_run.formatted_last_run_at(), "-");
    }

    #[test]
    fn test_automation_detail_status_badge_class() {
        let detail = |status: &str| AutomationDetail {
            id: 1,
            name: "Test".to_string(),
            description: "".to_string(),
            status: status.to_string(),
            created_by: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert_eq!(detail("active").status_badge_class(), "badge-success");
        assert_eq!(detail("archived").status_badge_class(), "badge-neutral");
        assert_eq!(detail("draft").status_badge_class(), "badge-warning");
        assert_eq!(detail("custom").status_badge_class(), "badge-neutral");
    }

    #[test]
    fn test_step_view_item_formatted_post_delay() {
        let step1 = StepViewItem {
            id: 1,
            step_number: 1,
            step_type: "mouse_click".to_string(),
            label: None,
            post_delay_ms: 1500,
            position: 10.0,
            description: "Click".to_string(),
        };
        assert_eq!(step1.formatted_post_delay(), " · then wait 1.5s");

        let step2 = StepViewItem {
            id: 2,
            step_number: 2,
            step_type: "mouse_click".to_string(),
            label: None,
            post_delay_ms: 0,
            position: 20.0,
            description: "Click".to_string(),
        };
        assert_eq!(step2.formatted_post_delay(), "");

        let step3 = StepViewItem {
            id: 3,
            step_number: 3,
            step_type: "mouse_click".to_string(),
            label: None,
            post_delay_ms: -500,
            position: 30.0,
            description: "Click".to_string(),
        };
        assert_eq!(step3.formatted_post_delay(), "");
    }

    #[test]
    fn test_worker_group_option_selection() {
        let group = WorkerGroupOption {
            id: 42,
            name: "Default Group".to_string(),
        };

        assert!(group.is_selected(&Some(42)));
        assert!(!group.is_selected(&Some(99)));
        assert!(!group.is_selected(&None));
    }

    #[test]
    fn test_variable_option_selections() {
        let var = VariableOption {
            id: 15,
            name: "var1".to_string(),
            var_type: "int".to_string(),
        };

        assert!(var.is_selected_x(&Some(15)));
        assert!(!var.is_selected_x(&Some(10)));
        assert!(!var.is_selected_x(&None));

        assert!(var.is_selected_y(&Some(15)));
        assert!(!var.is_selected_y(&None));

        assert!(var.is_selected_output(&Some(15)));
        assert!(!var.is_selected_output(&None));

        assert!(var.is_selected_found(&Some(15)));
        assert!(!var.is_selected_found(&None));
    }

    #[test]
    fn test_bitmap_option_selection() {
        let bitmap = BitmapOption {
            id: 7,
            name: "btn.png".to_string(),
            width: 100,
            height: 50,
        };

        assert!(bitmap.is_selected(&Some(7)));
        assert!(!bitmap.is_selected(&Some(8)));
        assert!(!bitmap.is_selected(&None));
    }

    #[test]
    fn test_step_option_display_name_and_selections() {
        let step_with_label = StepOption {
            id: 101,
            step_type: "mouse_click".to_string(),
            label: Some("  Submit Form  ".to_string()),
            position: 10.0,
            display_number: 1,
        };
        assert_eq!(step_with_label.display_name(), "Step 1 (Submit Form)");
        assert!(step_with_label.is_selected_match(&Some(101)));
        assert!(!step_with_label.is_selected_match(&Some(200)));
        assert!(step_with_label.is_selected_no_match(&Some(101)));

        let step_with_whitespace_label = StepOption {
            id: 102,
            step_type: "find_bitmap".to_string(),
            label: Some("   ".to_string()),
            position: 20.0,
            display_number: 2,
        };
        assert_eq!(step_with_whitespace_label.display_name(), "Step 2 (find_bitmap)");

        let step_without_label = StepOption {
            id: 103,
            step_type: "key_press".to_string(),
            label: None,
            position: 30.0,
            display_number: 3,
        };
        assert_eq!(step_without_label.display_name(), "Step 3 (key_press)");
    }
}
