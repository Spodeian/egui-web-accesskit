# egui-web-accesskit

A drop-in `egui::Plugin` that mirrors `egui`'s AccessKit tree to a real HTML/ARIA DOM overlay when running on `wasm32`, giving screen readers and browser accessibility tools visibility into `egui` widgets without patching `egui` or `eframe`.

## Version Compatibility

This repository mirrors `egui`'s minor version so there are no feature flags to juggle:

| `egui` / `eframe` | `egui-web-accesskit` | Branch / Tag |
| --- | --- | --- |
| `0.37` | `0.37` | `main` / `v0.37.4` |
| `0.36` | `0.36` | `egui-0.36` / `v0.36.4` |
| `0.35` | `0.35` | `egui-0.35` / `v0.35.4` |

## Quick Start

Add the crate version matching your `egui` / `eframe` dependency to `Cargo.toml` and register `WebAccessKitPlugin` once during app setup (for example inside `eframe`'s `CreationContext` callback):

```toml
[dependencies]
# For egui / eframe 0.37:
egui-web-accesskit = "0.37"

# Or pin a version-mirrored branch from git:
# egui-web-accesskit = { git = "https://github.com/Spodeian/egui-web-accesskit", branch = "egui-0.36" }
```

```rust
use egui_web_accesskit::WebAccessKitPlugin;

// Inside your eframe creation callback (`cc`):
cc.egui_ctx.add_plugin(WebAccessKitPlugin::new("the_canvas_id"));
```

For canvases hosted inside a `ShadowRoot` on `wasm32`, construct with `WebAccessKitPlugin::from_canvas(canvas)`.
To defer AccessKit tree building until a screen reader user activates the lightweight toggle control (or until you call `WebAccessKitPlugin::activate(&ctx)`), use `WebAccessKitPlugin::new_lazy("the_canvas_id")` or `WebAccessKitConfig::new("the_canvas_id").with_lazy(true)`.
When unmounting an `egui::Context` outside `eframe::WebRunner::destroy` (or on `egui 0.35`/`0.36`, which do not have `Plugin::on_exit`), call `WebAccessKitPlugin::teardown(&ctx)` to remove the DOM overlay and break the repaint-wakeup reference cycle.

This crate is intended only for `wasm32` deployment. On non-`wasm32` targets the plugin compiles to a clean no-op purely so shared code and native builds keep compiling without `#[cfg]` clutter at the call site; it does nothing there, so desktop apps should keep using `eframe` / `egui-winit`'s built-in AccessKit adapters.

## How It Works

1. `Plugin::setup` enables AccessKit on the `egui::Context` (unless `lazy: true`, in which case a single invisible activation `<button data-egui-accesskit-activate>` waits until focused/clicked or `WebAccessKitPlugin::activate(&ctx)` is called), mounts `<div data-egui-accesskit="true" role="application">` aligned over the target `<canvas>`'s CSS content box (`contain: strict`, `overflow: hidden`), attaches 9 container-level delegated DOM event handlers, and registers passive `window` `"scroll"` / `"resize"` listeners plus a canvas `ResizeObserver` so layout measurements only run when viewport geometry or DPR actually changes.
2. `Plugin::output_hook` skips intermediate discarded layout passes, takes ownership of `output.platform_output.accesskit_update`, diffs changes incrementally via `accesskit_consumer::Tree` (`TreeDelta`), hoists popups attached to leaf elements (`<button>`, `<input>`, `<textarea>`, `<a>`) with `aria-controls` / `aria-expanded`, maintains O(1) modal `inert` background trapping, moves focus into a dialog opened from the keyboard (and back to its opener on close), and patches only changed DOM attributes/bounds (`position: absolute` with scratch-buffer formatting so outer page scrolling is O(1)).
3. `Plugin::input_hook` keeps `RawInput::focused` true while a mirrored DOM element has focus and drains delegated DOM events (`click`, `Enter`/`Space` activation on custom roles such as `option`, `Escape` and single-line `Enter` key forwarding, `focus` + `ScrollIntoView`, `input` / `compositionend`, masked password range edits, `select` / navigation `keyup` selection updates, `PageUp`/`PageDown` scroll actions, and arrow keys on unstepped sliders) into `RawInput::events`. On `egui 0.35`/`0.36` where `TextEdit` does not handle `Action::SetValue` / `Action::ReplaceSelectedText`, DOM text edits fall back cleanly to focus-synchronized `Key::A` + `Backspace` + `Event::Paste` / `SetTextSelection` events, and overlay focus sync never steals DOM focus away from `<canvas>` or `eframe`'s `TextAgent`.
4. `Plugin::on_exit` / `WebAccessKitPlugin::teardown` disconnects the `ResizeObserver`, tears down DOM event listeners (including capture-phase `window` listeners), clears modal `inert` attributes, and removes the overlay container.

## Customizing AccessKit Semantics in `egui`

`egui`'s built-in widgets use conservative defaults in a few places (for example `ui.heading(...)` emits `Role::Label`, and `Slider` emits a numeric value without formatted units). You can enrich any widget's AccessKit node directly via `ui.ctx().accesskit_node_builder(resp.id, ...)`:

```rust
let heading = ui.heading("Settings");
ui.ctx().accesskit_node_builder(heading.id, |node| {
    node.set_role(egui::accesskit::Role::Heading);
    node.set_level(1);
});

let slider = ui.add(egui::Slider::new(&mut volume, 0.0..=100.0).text("Volume"));
ui.ctx().accesskit_node_builder(slider.id, |node| {
    node.set_value(format!("{}%", volume as i32));
});

let inc = ui.button("Increment counter");
ui.ctx().accesskit_node_builder(inc.id, |node| {
    node.set_description("Adds one to the click counter");
});

let docs = ui.hyperlink_to("egui docs", "https://docs.rs/egui");
ui.ctx().accesskit_node_builder(docs.id, |node| {
    node.set_url("https://docs.rs/egui");
});

// Dialogs and status banners can likewise set `node.set_modal()` or
// `node.set_live(egui::accesskit::Live::Polite)`.
```

## Layout, Shadow DOM, & Input Routing Notes

- **Canvas sizing, outer scroll, and zoom:** Mirrored children use `position: absolute` inside an `overflow: hidden; contain: strict` overlay synced to the canvas content box (via `getBoundingClientRect()`, `clientLeft`/`clientTop`/`clientWidth`/`clientHeight`, backing buffer ratio, `window` scroll/resize, and `ResizeObserver`), so outer page scrolling is O(1) (container `left`/`top` only), host CSS layout shifts stay aligned while `egui` is idle, and `ctx.set_zoom_factor(...)` scales accurately.
- **Shadow DOM:** `WebAccessKitPlugin::from_canvas(canvas)` attaches the overlay inside the canvas's parent `ShadowRoot` and resolves focus against `shadowRoot.activeElement`.
- **Password fields:** Plaintext is never written into `DomNodeSpec` or DOM attributes. The mirrored `<input type="password">` holds one ASCII `*` per character (Chromium exposes one mask glyph per character to assistive tech, so multi-byte bullets would be read as several glyphs each). User edits are translated via diffed UTF-16 `SetTextSelection` + `ReplaceSelectedText` (or fallback paste on `0.35`/`0.36`, and `SetValue` / select-all paste when replacing all text) without leaking plaintext.
- **Dialogs and keyboard focus:** Opening a modal marks the background `inert`, which would otherwise drop DOM focus to `document.body` when it was on the trigger button. For DOM-driven opens the bridge moves focus to the dialog's first control, traps `Tab` / `Shift+Tab` inside it, and returns focus to the opener when it closes.
- **Text field names and autocomplete:** `aria-label` is only rewritten when the accessible name changes, so typing and Backspace do not re-announce the field. `aria-autocomplete` is mirrored only when the application explicitly sets `Node::set_auto_complete`.
- **IME & focus coordination:** Clicking or typing directly on `<canvas>` leaves focus on `<canvas>` / `eframe::TextAgent`; when editing inside a mirrored DOM `<input>` / `<textarea>`, `InputEvent.isComposing` pauses `egui` value sync until `compositionend` commits.
- **Pointer events (`pointer-events: none`):** Mirrored DOM elements have `pointer-events: none` so mouse clicks, drags, hovers, and wheel events pass straight through to the underlying `<canvas>` with zero extra hit-test latency. Chromium AT (`kIgnorePointerEventsNone`) still resolves screen-reader touch exploration from DOM bounds; platforms that strictly hit-test by pointer coordinates (such as iOS VoiceOver touch-explore) use swipe/focus navigation instead.

## Running Tests & Demo

Native `cargo test` covers the platform-independent mapper (the DOM bridge only exists on `wasm32`). Run the checks natively and for WASM:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --target wasm32-unknown-unknown
```

See [examples/demo/README.md](examples/demo/README.md) to run the interactive `eframe` demo in the browser (it is browser-only), and [e2e/README.md](e2e/README.md) to run the Playwright + axe-core browser suite, which verifies the DOM bridge.

## Known Limitations

- **ComboBox keyboard activation on `egui 0.35` / `0.36`:** on a freshly loaded page, a `ComboBox` mirrored from these versions does not open from a keyboard `Space` press or an AccessKit click (the same `ComboBox` opens on `egui 0.37`). The E2E suite checks this step only on `0.37`. Pointer clicks on the canvas are unaffected.
- **`egui 0.35` / `0.36` upstream metadata:** On `egui 0.35` and `0.36`, `egui` itself does not emit `is_expanded` on `CollapsingHeader`, `read_only` on `&str` `TextEdit`, or `ListBox` roles on `ComboBox` popups (`TextEdit` DOM editing, focus, scroll actions, buttons, checkboxes, and sliders all work across `0.35`–`0.37`).
- **Role coverage:** Core interactive controls (`Button` including toggle `aria-pressed`, `DisclosureTriangle`, `Tab`/`TabList`/`TabPanel`, `Menu`/`MenuBar`/`MenuItem*`, `CheckBox` including mixed tri-state, `Switch`, `RadioButton`/`RadioGroup`, `ComboBox`/`EditableComboBox` with `ListBox`/`option`, `Table`/`Grid`/`TreeGrid` with `Row`/`Cell`/`GridCell`/`Header`, `Tree`/`TreeItem`, `Slider` stepped and unstepped, `SpinButton`/`DragValue`, `ProgressIndicator`, single-line, multiline, and password `TextEdit`, `Label`, `Image`, `Link`, landmarks, `Window`, `Dialog`/`AlertDialog` with `aria-modal` + `inert`, `Alert`/`Status` with `aria-live`, headings with `aria-level`, and lists) and rich ARIA metadata (`aria-required`, `aria-invalid`, `aria-errormessage`, `aria-describedby`, `aria-activedescendant`, `aria-orientation`, `aria-posinset`, `aria-setsize`, `aria-rowcount`/`aria-colcount`, `aria-rowindex`/`aria-colindex`) are mirrored with hierarchy context guards.

See [CHANGELOG.md](CHANGELOG.md) for release history.
