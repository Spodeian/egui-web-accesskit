# Changelog

All notable changes to `egui-web-accesskit` are documented here.
The crate versions mirror `egui`'s minor version:

- `0.37.x` (`main` branch) targets `egui` / `eframe` `0.37`
- `0.36.x` (`egui-0.36` branch) targets `egui` / `eframe` `0.36`
- `0.35.x` (`egui-0.35` branch) targets `egui` / `eframe` `0.35`

## Unreleased

### Added
- Demo: a reload-based screen reader mirror toggle (`?bridge=off`, and a button that reloads the page with the opposite setting). egui keeps one plugin per type, so a torn-down bridge cannot be re-added in place.
- Demo: a section heading whose level toggles between 2 and 3, with an end-to-end test that focus stays on the control that changes it.
- Unit tests for the wasm32-only helpers in `src/dom.rs` (`wasm-bindgen-test`), run by CI.
- Demo build script (`examples/demo/build-dist.sh`) with a release profile, the wasm32 baseline target features, explicit `wasm-opt -Oz` and HTML minification.
- CI: per-branch GitHub Pages publishing, a post-deploy smoke test, formatting, licence (`cargo-deny`) and Prettier gates. Firefox runs headed under Xvfb as a non-required job.

### Changed
- Demo wasm: 5.69 MB to 4.23 MB raw, and 1.76 MB to 1.56 MB gzipped. The JS glue is not minified, because trunk's subresource-integrity hash would break.

### Fixed
- Keyboard activation of a dialog trigger (Enter or Space) no longer drops DOM focus to `document.body`. The modal's `inert` background is applied to the focused trigger, so the bridge now moves focus into the dialog, traps `Tab` / `Shift+Tab` inside it, and restores focus to the trigger on close.
- Typing and Backspace in text fields no longer rewrite `aria-label` on every keystroke, which screen readers announced again (for example as "blank" once the field was empty).
- Password fields no longer expose three mask glyphs per character. The DOM mask is now ASCII `*`, which Chromium exposes as one glyph per character.
- `aria-autocomplete` is no longer emitted by default. It is mirrored only when the application sets `Node::set_auto_complete`.

### Testing & Demo
- Added Playwright regression tests for keyboard dialog focus, Backspace attribute churn, password mask exposure, and `aria-autocomplete`. `PW_CHROME_PATH` selects a specific Chromium binary for `npm test`.
- Demo gained an explicitly autocompleted `Search:` field.

## 0.37.4 / 0.36.4 / 0.35.4

### Performance
- Replaced per-node event listeners with container-level event delegation across 9 shared handlers on the overlay element, eliminating per-widget `Closure` allocations and attach/detach DOM churn.
- Replaced per-frame `TreeBridge` intermediate `Vec<BridgeOp>` allocation with structured `TreeDelta` vectors and eliminated heap `Vec` allocation inside `has_descendant`.
- Added dirty-flagged `sync_viewport()` gating (`scroll`, `resize`, `ResizeObserver`, DPR, or canvas backing-buffer size changes) so steady 60fps `egui` frames incur zero forced DOM layout reads (`getBoundingClientRect`, `client*`, `offset*`).
- Tracked active modal nodes in a `HashSet<NodeId>` with a `modal_dirty` guard for O(1) modal `inert` synchronization when zero or one modal is active.
- Reused a scratch `String` buffer in `apply_bounds` and avoided redundant `width`/`height`/`position` style writes when only coordinates changed.
- Added opt-in lazy AccessKit activation via `WebAccessKitConfig::with_lazy(true)`, `WebAccessKitPlugin::new_lazy(...)`, and `WebAccessKitPlugin::activate(&ctx)`.

### Accessibility & Keyboard
- Forwarded `Escape` (when not composing) and single-line `Enter` (non-composing, unmodified `<input>`) keypresses into `egui::RawInput::events` so modals, popups, and `TextEdit::singleline` submit handlers work when DOM overlay elements hold focus.
- Mirrored additional WAI-ARIA metadata on `DomNodeSpec` and DOM elements: `aria-required`, `aria-invalid`, `aria-errormessage`, `aria-describedby`, `aria-activedescendant`, `aria-orientation`, 1-based `aria-posinset`, `aria-setsize`, `aria-rowcount`, `aria-colcount`, 1-based `aria-rowindex`, and 1-based `aria-colindex`.

### Demo & Testing
- Added `/?lazy=1` query mode, `Escape` modal dismissal, and single-line `Enter` submission counter to `examples/demo`.
- Added Firefox (`--project=firefox` / `npm run test:firefox`) alongside headless Chrome in `e2e/playwright.config.mjs`.

## 0.37.3 / 0.36.3 / 0.35.3

- Updated Playwright E2E and documentation for release readiness; clarified `pointer-events: none` behavior and unified `WebAccessKitPlugin::setup` / `teardown` lifecycle notes.

## 0.37.2 / 0.36.2 / 0.35.2

- Fixed canvas border/padding CSS transform alignment (`clientLeft`/`clientTop`/`offsetWidth`), modal `inert` background trapping, hoisted leaf popups (`aria-controls` / `aria-expanded`), and added Shadow DOM hosting via `WebAccessKitPlugin::from_canvas`.

## 0.37.1 / 0.36.1 / 0.35.1

- Added `WebAccessKitPlugin::teardown`, idle viewport listeners (`ResizeObserver`, capture scroll, resize), toggle/radio/checkbox/combobox/grid/tree role mapping with hierarchy guards, IME composition deferral, and UTF-16 password diffing.

## 0.37.0 / 0.36.0 / 0.35.0

- Initial release mirroring `egui`'s AccessKit tree to an overlaid HTML/ARIA DOM with two-way action, focus, scroll, slider, and text/selection synchronization.
