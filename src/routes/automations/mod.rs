pub mod detail;
pub mod list;
pub mod parameters;
pub mod steps;
pub mod types;
pub mod variables;

pub use detail::*;
pub use list::*;
pub use parameters::*;
pub use steps::*;
pub use types::*;
pub use variables::*;

use axum::routing::{get, post};
use axum::Router;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/automations", get(get_automations_handler).post(post_automations_handler))
        .route("/automations/new", get(get_new_automation_handler))
        .route("/automations/{id}", get(get_automation_detail_handler).post(post_automation_edit_handler))
        .route("/automations/{id}/run-now", post(post_run_now_automation_handler))
        .route("/automations/{id}/delete", get(get_automation_delete_handler).post(post_automation_delete_handler))
        .route("/automations/{id}/variables", get(get_automation_variables_handler).post(post_create_automation_variable_handler))
        .route("/automations/{id}/variables/{vid}", post(post_update_automation_variable_handler))
        .route("/automations/{id}/variables/{vid}/delete", post(post_delete_automation_variable_handler))
        .route("/automations/{id}/parameters", get(get_automation_parameters_handler).post(post_create_automation_parameter_handler))
        .route("/automations/{id}/parameters/{pid}", post(post_update_automation_parameter_handler))
        .route("/automations/{id}/parameters/{pid}/delete", post(post_delete_automation_parameter_handler))
        .route("/automations/{id}/steps/new", get(get_step_type_picker_handler))
        .route("/automations/{id}/steps/new/key_press", get(get_new_key_press_step_handler))
        .route("/automations/{id}/steps/new/mouse_click", get(get_new_mouse_click_step_handler))
        .route("/automations/{id}/steps/new/find_pixel_rgb", get(get_new_find_pixel_rgb_step_handler))
        .route("/automations/{id}/steps/new/find_bitmap", get(get_new_find_bitmap_step_handler))
        .route("/automations/{id}/steps/new/branch", get(get_new_branch_step_handler))
        .route("/automations/{id}/steps", post(post_create_step_handler))
        .route("/automations/{id}/steps/{sid}/edit", get(get_edit_step_handler))
        .route("/automations/{id}/steps/{sid}", post(post_edit_step_handler))
        .route("/automations/{id}/steps/{sid}/move-up", post(post_move_step_up_handler))
        .route("/automations/{id}/steps/{sid}/move-down", post(post_move_step_down_handler))
        .route("/automations/{id}/steps/{sid}/delete", post(post_delete_step_handler))
}
