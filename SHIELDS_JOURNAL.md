# SHIELDS JOURNAL

## 2025-03-10 Task Run Formatting & Badge Class Edge Cases Coverage
Learning: The task run web UI structures (`TaskRunListItem`, `ExecutedStepItem`, `RunsListTemplate`) contain multiple conditional formatting paths (such as multi-minute/multi-hour duration formatting, unstarted vs running durations, partial RGB/XY coordinate captures, and template status filter matching) that can easily regress if untested.
Action: Test task run duration calculations, status badge classes, step result badge classes, and template filter methods directly in unit tests using edge-case inputs without requiring database fixtures.

## 2025-03-11 Automation UI Types Formatting & Badge Class Coverage
Learning: The automation UI data structures (`AutomationListItem`, `AutomationDetail`, `StepViewItem`, `StepOption`, and selection helper options) contain multiple conditional formatting paths (such as post-delay conversion from ms to seconds, status badge mappings for active/archived/draft/lost/cancelling, datetime formatting for missing vs present last runs, and label trimming) that are synchronous and best tested at the unit level.
Action: Write pure unit tests for domain DTO formatting and selection methods directly in `src/routes/automations/types.rs` without requiring database fixtures or network setup.

## 2025-03-12 Domain Enums Display, FromStr Parsing and Serde Edge Cases Coverage
Learning: Domain status enums (`WorkerStatus`, `TaskRunStatus`, `StepResultStatus`) use `sqlx(rename_all = ...)` for DB mapping and custom `FromStr`/`Display` implementations for case-insensitive, whitespace-trimmed string parsing and lowercase display output, while Serde default derivations preserve PascalCase variant names.
Action: In unit tests for domain status enums, test `Display`, case-insensitive and whitespace-tolerant `FromStr` parsing, and Serde JSON serialization/deserialization separately to ensure string parsing edge cases do not break.

## 2025-03-13 AppError HTTP Status Mapping & JSON Error Response Payload Coverage
Learning: Testing `AppError::into_response()` using `axum::body::to_bytes` and `serde_json::from_slice` allows verifying HTTP status code mappings and JSON error response formatting for all application error variants without requiring database or network fixtures.
Action: Test error handling response transformations directly via `into_response()` assertions in `src/error.rs` whenever new error variants or response fields are added.

## 2025-03-14 Worker Agent Version Comparison Edge Cases Coverage
Learning: Worker agent version checking (`is_agent_version_outdated`) parses dot-separated numeric version components using digit extraction. Key edge cases include double-digit components (e.g., `1.10.0` vs `1.9.0`), whitespace padding (`" 1.2.0 "`), pre-release suffixes (`-rc1`, `-beta`), and whitespace-only string representations for agent vs minimum required versions.
Action: Write pure unit tests asserting exact boolean outputs across double-digit semver components, pre-release suffixes, and whitespace variations to prevent version validation regressions.

## 2025-03-15 Deserialization Option Number Boundary and Error Edge Cases Coverage
Learning: `deserialize_option_number` converts empty or whitespace-only strings to `None` for optional form fields, but handles string trimming before numeric parsing. Key edge cases include whitespace-padded numeric strings in urlencoded forms or JSON (e.g. `" 123 "`, `"\t42\n"`), invalid non-numeric or float-to-integer string payloads (`"not_a_number"`, `"12.34"` into `i64`), and numeric overflow strings (e.g. `"99999999999"` into `i16`).
Action: Test both successful whitespace trimming and deserialization error failures (non-numeric, type mismatch, overflow) directly across `serde_urlencoded` and `serde_json` in `src/de.rs`.

## 2025-03-16 Automation Validation Helper Functions Unit Test Coverage
Learning: Internal validation helper routines (`check_coord_bounds`, `validate_mouse_click`, `validate_var_ref`, `validate_find_pixel`, `validate_find_bitmap`, `validate_output_var`, `validate_branch`) contain boundary condition checks (0..10,000 range), variable type compatibility validation, and optional target/bitmap existence rules that can be tested in isolation using lightweight in-memory data structures.
Action: Test validation helper functions directly with boundary coordinates (-1, 0, 10000, 10001), missing vs present variable/bitmap IDs, and type mismatch inputs in unit tests within `src/validation.rs`.

## 2025-03-17 Static Assets MIME Type & Conditional ETag Caching Coverage
Learning: Static asset handlers serving embedded `rust_embed::RustEmbed` assets construct SHA256 ETags and evaluate `If-None-Match` HTTP headers. Converting `static_asset_handler(...).await.into_response()` directly into `axum::response::Response` allows verifying `StatusCode::OK` vs `StatusCode::NOT_MODIFIED` vs `StatusCode::NOT_FOUND`, `Content-Type`, `Cache-Control`, and `ETag` headers at the unit test level without network listeners or database fixtures.
Action: Test static asset handlers and MIME type resolution functions directly using `into_response()` assertions across case variations, missing/unknown extensions, multi-dot paths, and ETag header lists.

## 2025-03-18 CSRF Origin Middleware Branch & Header Verification Coverage
Learning: Testing `csrf_origin_middleware` using `axum::Router`, `tower::ServiceExt::oneshot`, and `AppState` with `PgPool::connect_lazy` allows verifying CSRF origin checks (safe HTTP methods, exempt `/login`/`/api/*` paths, `Sec-Fetch-Site: cross-site` rejection, matching `Origin`, `Host` fallback, and mismatched `Origin` rejection) directly at the unit test level without database connections or external network listeners.
Action: Test web middleware handlers directly via `app.oneshot(req)` assertions in `src/auth/` without requiring live external dependencies.

## 2025-03-19 Login Rate Limiter Case Insensitivity, Clear & Eviction Coverage
Learning: `LoginRateLimiter` normalizes usernames to lowercase when storing and looking up attempts in its `HashMap<(IpAddr, String), Vec<Instant>>`, and opportunistically triggers an eviction sweep (`sweep_expired`) inside `record_failure` when the combined length of `user_attempts` and `ip_attempts` exceeds `EVICTION_THRESHOLD` (256 entries).
Action: Test username case insensitivity, selective user attempt resetting (`clear`), and `EVICTION_THRESHOLD` sweep triggering directly in `src/auth/rate_limit.rs` using short `Duration` windows and simulated attempt volumes.

## 2025-03-20 Dotenv Line Parsing Edge Cases Coverage
Learning: Zero-dependency `.env` parsing (`parse_dotenv_content`) handles complex string inputs (comments with leading whitespace, key/value surrounding whitespace, empty values, single/double empty quotes, mismatched quotes, single character quotes, and values with multiple equals signs).
Action: Test `.env` line parsing edge cases directly with multi-line string fixtures in `src/config.rs` unit tests without touching disk or process environment variables.

## 2025-03-21 URL Decoding & Flash Message Extraction Edge Cases Coverage
Learning: `url_decode` in `src/auth/flash.rs` returns `None` when encountering truncated percent escapes (e.g. `"hello%"`, `"hello%2"`), invalid hex escape digits (e.g. `"hello%2G"`), or invalid UTF-8 byte sequences (e.g. `"%FF%FE"`), while `extract_flash_message` defaults level to `FlashLevel::Info` when the `flash_level` cookie is absent or contains an unrecognized string value.
Action: Unit test `url_decode` boundary inputs, missing header options, default level fallbacks, and clear cookie formatting directly in `src/auth/flash.rs`.

## 2025-03-22 Session Cookie Parsing & Clearing Edge Cases Coverage
Learning: `extract_session_id` parses semicolon-separated cookie headers by stripping `__Host-session_id=` and `session_id=` prefixes. Key edge cases include invalid UUID values in `__Host-session_id=` gracefully falling back to subsequent valid `session_id=` pairs, precedence of `__Host-session_id` when valid, and avoiding false prefix matches (e.g. `not_session_id=`).
Action: Test session ID cookie extraction and cookie clearing exact attributes directly in unit tests in `src/auth/session.rs` without requiring database connection fixtures.

## 2025-03-23 Automation Step Helpers Sanitization, Summaries & Branch Validation Coverage
Learning: Step creation and editing in `src/routes/automations/steps.rs` relies on internal helper functions (`sanitize_label`, `seconds_to_post_delay_ms`, `generate_type_text_summary`, `sanitize_mouse_button`, `sanitize_click_type`, `validate_branch_form`) to clean form inputs, calculate post-delays, format summary labels, and validate branch targets before database mutations.
Action: Unit test label sanitization edge cases, delay float-to-int conversion clamping, 30-char text truncation formatting, mouse click option sanitization, and branch condition validation rules directly without requiring database fixtures.

## 2025-03-24 UserRole Permissions, Conversions & Parsing Edge Cases Coverage
Learning: `UserRole` parses string inputs using `s.to_lowercase().as_str()` without `.trim()`, meaning `FromStr` supports case-insensitivity (e.g. `"ADMIN"` -> `UserRole::Admin`) but rejects untrimmed whitespace surrounding role strings (e.g. `"  ADMIN  "` -> `Err("Unknown user role:   ADMIN  ")`).
Action: In unit tests for `UserRole`, test `as_str()`, `can_edit()`, `is_admin()`, case-insensitive `FromStr` without leading/trailing spaces, invalid role error handling, and Serde JSON serialization/deserialization.

## 2025-03-25 Cron Pattern Next After & Sunday Representation Edge Cases Coverage
Learning: `CronPattern` in `src/routes/schedules.rs` normalizes Sunday as `0`, `7`, or `"SUN"`, supports step-range syntax (e.g., `1-10/2`), handles year-end boundary transitions (Dec 31 to Jan 1), and applies OR-logic when both day-of-month and day-of-week fields are restricted (non-wildcards).
Action: Write pure unit tests asserting exact `CronPattern::next_after` outputs and bitmask matching across Sunday representations, step ranges, year rollover, and DOM/DOW OR-logic without requiring database or time-delay fixtures.

## 2025-03-26 CSS URL Escaping and Image Magnifier Style Generation Edge Cases Coverage
Learning: `escape_css_url` in `src/magnifier.rs` returns `Cow::Borrowed` for URLs lacking CSS-unsafe characters to prevent heap allocations, while escaping backslashes, single/double quotes, parentheses, and angle brackets, and stripping line breaks (`\n`, `\r`) and form feeds (`\x0C`) to neutralize CSS injection vectors in `ImageMagnifier` style templates (`normal_style`, `zoom400_style`, `grid_style`).
Action: Test `escape_css_url` zero-copy borrow returns, multi-character escaping/stripping, and CSS style generation formatting directly in unit tests in `src/magnifier.rs`.
## 2025-03-26 Window Control & Mouse Scroll Step Summaries & Branch Validation Coverage
Learning: Step summary formatting routines (`generate_window_control_summary`, `generate_mouse_scroll_summary`) and step branch form validation (`validate_branch_form`) for `window_state` conditions contain multiple conditional string formatting paths (e.g. action string case-insensitivity, 30-character pattern truncation, region vs coordinate target formatting, and missing pattern validation) that can be tested synchronously without database setup.
Action: Write pure unit tests in `src/routes/automations/steps.rs` covering window control actions, pattern truncation, mouse scroll coordinate modes, and window state branch validation rules.
