# SHIELDS JOURNAL

## 2025-03-10 Task Run Formatting & Badge Class Edge Cases Coverage
Learning: The task run web UI structures (`TaskRunListItem`, `ExecutedStepItem`, `RunsListTemplate`) contain multiple conditional formatting paths (such as multi-minute/multi-hour duration formatting, unstarted vs running durations, partial RGB/XY coordinate captures, and template status filter matching) that can easily regress if untested.
Action: Test task run duration calculations, status badge classes, step result badge classes, and template filter methods directly in unit tests using edge-case inputs without requiring database fixtures.
