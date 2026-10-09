# SHIELDS JOURNAL

## 2025-03-10 Task Run Formatting & Badge Class Edge Cases Coverage
Learning: The task run web UI structures (`TaskRunListItem`, `ExecutedStepItem`, `RunsListTemplate`) contain multiple conditional formatting paths (such as multi-minute/multi-hour duration formatting, unstarted vs running durations, partial RGB/XY coordinate captures, and template status filter matching) that can easily regress if untested.
Action: Test task run duration calculations, status badge classes, step result badge classes, and template filter methods directly in unit tests using edge-case inputs without requiring database fixtures.

## 2025-03-11 Automation UI Types Formatting & Badge Class Coverage
Learning: The automation UI data structures (`AutomationListItem`, `AutomationDetail`, `StepViewItem`, `StepOption`, and selection helper options) contain multiple conditional formatting paths (such as post-delay conversion from ms to seconds, status badge mappings for active/archived/draft/lost/cancelling, datetime formatting for missing vs present last runs, and label trimming) that are synchronous and best tested at the unit level.
Action: Write pure unit tests for domain DTO formatting and selection methods directly in `src/routes/automations/types.rs` without requiring database fixtures or network setup.
