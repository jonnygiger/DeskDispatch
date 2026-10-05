# Palette's Journal - Critical Learnings

## 2025-05-18 - Top Nav and Skip Link Accessibility Pattern
**Learning:** In Askama-rendered server-side Rust web apps without heavy client JS frameworks, landmark navigation (`<main id="main-content">` with a `.skip-link`) and `aria-current="page"` on macro-rendered top navigation links provide immediate keyboard focus navigation and screen reader semantics across all pages without layout overhead.
**Action:** Always include `aria-current="page"` on active navigation tabs rendered by `templates/macros.html` and ensure layout templates contain an accessible skip-to-content target.
