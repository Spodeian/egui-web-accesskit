# egui-web-accesskit

A drop-in `egui::Plugin` that mirrors `egui`'s AccessKit tree to a real HTML/ARIA DOM overlay when running on `wasm32`, giving screen readers and browser accessibility tools visibility into `egui` widgets without patching `egui` or `eframe`.

## Quick Start

Add the crate to your `Cargo.toml` and register `WebAccessKitPlugin` once during app setup (for example inside `eframe`'s `CreationContext` callback):

```toml
# Default: egui 0.37
egui-web-accesskit = "0.1"

# For egui 0.36 apps:
# egui-web-accesskit = { version = "0.1", default-features = false, features = ["egui-036"] }
```

```rust
use egui_web_accesskit::WebAccessKitPlugin;

// Inside your eframe creation callback (`cc`):
cc.egui_ctx.add_plugin(WebAccessKitPlugin::new("the_canvas_id"));
```

For canvases hosted inside a `ShadowRoot` on `wasm32`, construct with `WebAccessKitPlugin::from_canvas(canvas)`.
When unmounting an `egui::Context` outside `eframe::WebRunner::destroy` (or on `egui 0.36`, which does not have `Plugin::on_exit`), call `WebAccessKitPlugin::teardown(&ctx)` to remove the DOM overlay and break the repaint-wakeup reference cycle.

On non-`wasm32` targets the plugin is a clean no-op so native builds can keep using `eframe` / `egui-winit`'s built-in desktop AccessKit adapters without `#[cfg]` clutter at the call site.

## How It Works

1. `Plugin::setup` enables AccessKit on the `egui::Context` (`ctx.enable_accesskit()`) and mounts a visually transparent `<div data-egui-accesskit="true" role="application">` aligned over the target `<canvas>`.
2. `Plugin::output_hook` skips intermediate discarded layout passes, clones `output.platform_output.accesskit_update`, applies incremental changes with `accesskit_consumer::Tree`, and patches the mirrored HTML/ARIA elements (using `window.devicePixelRatio` so `egui` zoom stays aligned in CSS logical pixels).
3. `Plugin::input_hook` keeps `RawInput::focused` true while a mirrored DOM element has focus and drains DOM events (`click`, `focus`, `input`, arrow keys on unstepped sliders) into `RawInput::events` as `egui::Event::AccessKitActionRequest`.
4. `Plugin::on_exit` / `WebAccessKitPlugin::teardown` tears down DOM event listeners and removes the overlay container.

## Running Tests & Demo

Run unit and round-trip tests natively (for both `egui 0.37` and `egui 0.36`):

```sh
cargo test
cargo test --no-default-features --features egui-036
```

Verify the library compiles for WASM:

```sh
cargo check --target wasm32-unknown-unknown
cargo check --no-default-features --features egui-036 --target wasm32-unknown-unknown
```

Run the standalone `eframe` demo in the browser using [Trunk](https://trunkrs.dev/):

```sh
cd examples/demo
trunk serve
```

Run the Playwright browser E2E suite:

```sh
cd examples/demo
trunk build
cd ../../e2e
npm install
npm test
```

## Known Limitations

- **`egui 0.36` upstream coverage:** On `egui 0.36`, `egui` itself does not yet handle `Action::SetValue` on `TextEdit` or emit `is_expanded` on `CollapsingHeader`; text reading, focus, buttons, checkboxes, and sliders work on `0.36`, while remote DOM text editing and `aria-expanded` require `egui 0.37`.
- **Multiline text selection & IME composition:** `Role::MultilineTextInput` currently mirrors as `<input type="text">` (`aria-multiline` / full caret selection ranges are not synced), and `eframe`'s `TextAgent` IME flow is independent of the mirrored DOM `<input>`.
- **Role coverage:** Core interactive controls (`Button`, `DisclosureTriangle`, `Tab`, `MenuItem`, `CheckBox` including mixed tri-state, `Switch`, `RadioButton`, `Slider` stepped and unstepped, `TextInput`, `PasswordInput`, `Label`, `Link`, `Window`, `Dialog`, headings, lists, and regions) are mapped to semantic HTML/ARIA elements; complex grids/tables and rich text selection ranges are simplified.
