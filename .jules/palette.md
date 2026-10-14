# Palette's Journal - Critical Learnings

## 2025-05-18 - Top Nav and Skip Link Accessibility Pattern
**Learning:** In Askama-rendered server-side Rust web apps without heavy client JS frameworks, landmark navigation (`<main id="main-content">` with a `.skip-link`) and `aria-current="page"` on macro-rendered top navigation links provide immediate keyboard focus navigation and screen reader semantics across all pages without layout overhead.
**Action:** Always include `aria-current="page"` on active navigation tabs rendered by `templates/macros.html` and ensure layout templates contain an accessible skip-to-content target.

## 2025-05-18 - Table Row Action Button Accessibility Pattern
**Learning:** Table action cells containing generic button text (e.g. "Disable", "Edit", "Delete") across repetitive loop iterations create ambiguity for screen reader users. Dynamic template expressions like `aria-label="Edit schedule {{ s.name }}"` give instant contextual semantics without layout or visual changes.
**Action:** Always qualify generic table action buttons/links with item-specific `aria-label`s in Askama HTML table templates.

## 2025-05-18 - Form Input Helper Text Accessibility Pattern
**Learning:** Form helper texts (`<small>` or `<span class="text-muted">`) placed beneath `<input>` or `<select>` elements are bypassed by screen readers unless explicitly linked via `aria-describedby="[id]"` matching the helper element's `id`.
**Action:** Always link form controls with descriptive text or syntax guidance using `aria-describedby` and matching `id` attributes on Askama form templates.
