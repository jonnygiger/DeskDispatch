pub mod audit;
pub mod csrf;
pub mod flash;
pub mod rate_limit;
pub mod security_headers;
pub mod session;
pub mod user;
pub mod worker;

pub use audit::log_audit;
pub use csrf::{CsrfForm, csrf_middleware, generate_csrf_token, validate_csrf_token};
pub use flash::{
    FlashLevel, FlashMessage, build_flash_cookie, clear_flash_cookie, extract_flash_message,
};
pub use rate_limit::LoginRateLimiter;
pub use security_headers::security_headers_middleware;
pub use session::{
    clear_session_cookie, create_session, create_session_cookie, delete_session,
    purge_expired_sessions, revoke_user_sessions_except,
};
pub use user::{
    AuthUser, OptionalAuthUser, RequireAdmin, RequireEditor, UserRole, hash_password_async,
    verify_password_async,
};
pub use worker::{AuthWorker, WorkerAuth};
