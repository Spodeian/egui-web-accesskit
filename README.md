# egui-web-accesskit

A drop-in `egui::Plugin` that mirrors `egui`'s AccessKit tree to a real HTML/ARIA DOM overlay when running on `wasm32`, giving screen readers and browser accessibility tools visibility into `egui` widgets without patching `egui` or `eframe`.

## Version Compatibility

This repository mirrors `egui`'s minor version so there are no feature flags to juggle:

| `egui` / `eframe` | `egui-web-accesskit` | Branch / Tag |
| --- | --- | --- |
| `0.37` | `0.37` | `main` / `v0.37.1` |
| `0.36` | `0.36` | `egui-0.36` / `v0.36.1` |
| `0.35` | `0.35` | `egui-0.35` / `v0.35.1` |

## Quick Start

Add the crate version matching your `egui` / `eframe` dependency to `Cargo.toml` and register `WebAccessKitPlugin` once during app setup (for example inside `eframe`'s `CreationContext` callback):

```toml
[dependencies]
# For egui / eframe 0.35:
egui-web-accesskit = "0.35"

# Or pin this branch from git:
# egui-web-accesskit = { git = "https://github.com/Spodeian/egui-web-accesskit", branch = "egui-0.35" }
```

```rust
use egui_web_accesskit::WebAccessKitPlugin;

// Inside your eframe creation callback (`cc`):
cc.egui_ctx.add_plugin(WebAccessKitPlugin::new("the_canvas_id"));
```

For canvases hosted inside a `ShadowRoot` on `wasm32`, construct with `WebAccessKitPlugin::from_canvas(canvas)`.
When unmounting an `egui::Context` outside `eframe::WebRunner::destroy` (or on `egui 0.35`/`0.36`, which do not have `Plugin::on_exit`), call `WebAccessKitPlugin::teardown(&ctx)` to remove the DOM overlay and break the repaint-wakeup reference cycle.

On non-`wasm32` targets the plugin is a clean no-op so native builds can keep using `eframe` / `egui-winit`'s built-in desktop AccessKit adapters without `#[cfg]` clutter at the call site.

## How It Works

1. `Plugin::setup` enables AccessKit on the `egui::Context` (`ctx.enable_accesskit()`), mounts a visually transparent `<div data-egui-accesskit="true" role="application">` aligned over the target `<canvas>`, and registers passive `window` `"scroll"` / `"resize"` listeners so `position: fixed` elements stay aligned even while `egui` is idle.
2. `Plugin::output_hook` skips intermediate discarded layout passes, clones `output.platform_output.accesskit_update`, applies incremental changes with `accesskit_consumer::Tree`, and patches the mirrored HTML/ARIA elements — including `<textarea>` (`aria-multiline`), native heading elements (`<h1>`–`<h6>`) + `aria-level`, `aria-modal`, `aria-live`, `aria-description`, `aria-valuetext`, `placeholder`, `readonly`/`aria-readonly`, safe link `href`s, UTF-16 text selection sync, and `window.devicePixelRatio`-scaled bounds.
3. `Plugin::input_hook` keeps `RawInput::focused` true while a mirrored DOM element has focus and drains DOM events (`click`, `focus` + `ScrollIntoView`, `input`, masked password `ReplaceSelectedText` edits, `select`/navigation `keyup` selection updates, `PageUp`/`PageDown` scroll actions, and arrow keys on unstepped sliders) into `RawInput::events` as `egui::Event::AccessKitActionRequest`.
4. `Plugin::on_exit` / `WebAccessKitPlugin::teardown` tears down DOM event listeners (including capture-phase `window` listeners) and removes the overlay container.

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

- **Canvas sizing, outer scroll, and zoom:** Overlay elements are positioned with `position: fixed` using `canvas.get_bounding_client_rect()` and `window.devicePixelRatio`, so normal scrolling pages, CSS flex/grid layouts, and `ctx.set_zoom_factor(...)` remain aligned even when `egui` is idle.
- **Shadow DOM:** `WebAccessKitPlugin::from_canvas(canvas)` attaches the overlay inside the canvas's parent `ShadowRoot` and resolves focus against `shadowRoot.activeElement`.
- **Password fields:** Plaintext is never written into `DomNodeSpec` or DOM attributes; user edits inside a masked `<input type="password">` are translated via diffed UTF-16 `SetTextSelection` + `ReplaceSelectedText` (or `SetValue` when replacing all text) without leaking plaintext.
- **Pointer events (`pointer-events: none`):** Mirrored DOM elements have `pointer-events: none` so mouse clicks, drags, hovers, and wheel events pass straight through to the underlying `<canvas>`. The tradeoff is that touch-explore accessibility modes on mobile screen readers (which hit-test DOM nodes by touch coordinates) cannot focus elements directly without screen-reader swipe navigation.

## Running Tests & Demo

Run unit tests, doctests, Clippy, and rustdoc natively and for WASM:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --target wasm32-unknown-unknown
```

See [examples/demo/README.md](examples/demo/README.md) to run the interactive `eframe` demo in the browser or natively, and [e2e/README.md](e2e/README.md) to run the Playwright + axe-core browser suite. (Note: `examples/demo` and `e2e/` track `main` / `egui 0.37`.)

## Known Limitations

- **`egui 0.35` / `0.36` upstream coverage:** On `egui 0.35` and `0.36`, `egui` itself does not yet handle `Action::SetValue` / `Action::ReplaceSelectedText` on `TextEdit`, emit `is_expanded` on `CollapsingHeader`, or emit `read_only` on `&str` `TextEdit`; text reading, focus, scroll actions, buttons, checkboxes, and sliders work on `0.35`/`0.36`, while remote DOM text editing, `readonly` from `&str`, and `aria-expanded` require `egui 0.37`.
- **IME composition:** `eframe`'s internal `TextAgent` IME flow is independent of the mirrored DOM text inputs.
- **Role coverage:** Core interactive controls (`Button`, `DisclosureTriangle`, `Tab`, `MenuItem`, `CheckBox` including mixed tri-state, `Switch`, `RadioButton`, `Slider` stepped and unstepped, `SpinButton` / `DragValue`, single-line, multiline, and password `TextEdit`, `Label`, `Link`, `Window`, `Dialog`, `Alert`, headings with `aria-level`, lists, and regions) are mapped to semantic HTML/ARIA elements; complex grids/tables remain simplified.
