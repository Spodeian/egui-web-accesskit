# egui-web-accesskit

A drop-in `egui::Plugin` that mirrors `egui`'s AccessKit tree to a real HTML/ARIA DOM overlay when running on `wasm32`, giving screen readers and browser accessibility tools visibility into `egui` widgets without patching `egui` or `eframe`.

## Quick Start

Add the crate to your `Cargo.toml` and register `WebAccessKitPlugin` once during app setup (for example inside `eframe`'s `CreationContext` callback):

```rust
use egui_web_accesskit::WebAccessKitPlugin;

// Inside your eframe creation callback (`cc`):
cc.egui_ctx.add_plugin(WebAccessKitPlugin::new("the_canvas_id"));
```

On non-`wasm32` targets the plugin is a clean no-op so native builds can keep using `eframe` / `egui-winit`'s built-in desktop AccessKit adapters without `#[cfg]` clutter at the call site.

## How It Works

1. `Plugin::setup` enables AccessKit on the `egui::Context` (`ctx.enable_accesskit()`) and mounts a visually transparent `<div data-egui-accesskit="true" role="application">` aligned over the target `<canvas>`.
2. `Plugin::output_hook` reads `output.platform_output.accesskit_update`, applies incremental changes with `accesskit_consumer::Tree`, and patches the mirrored HTML/ARIA elements (including bounding boxes in logical CSS pixels).
3. `Plugin::input_hook` drains DOM events (`click`, `focus`, `input`, `change`) into `RawInput::events` as `egui::Event::AccessKitActionRequest`.
4. `Plugin::on_exit` tears down DOM event listeners and removes the overlay container.

## Running Tests & Demo

Run unit and integration tests natively:

```sh
cargo test
```

Verify the library compiles for WASM:

```sh
cargo check --target wasm32-unknown-unknown
```

Run the standalone `eframe` demo in the browser using [Trunk](https://trunkrs.dev/):

```sh
cd examples/demo
trunk serve
```

## Known Limitations

- **Focus & IME coordination with `eframe::web::TextAgent`:** `eframe`'s web runner uses its own hidden `<input>` and global window keyboard listeners for text input and IME. While this plugin calls `stop_propagation()` on mirrored DOM events, multiline selection, caret synchronization, and mobile virtual keyboard focus transitions remain limited compared to native desktop adapters.
- **Role coverage:** Core interactive controls (`Button`, `CheckBox`, `RadioButton`, `Slider`, `TextInput`, `MultilineTextInput`, `PasswordInput`, `Label`, `Link`, `Window`, `Dialog`, headings, lists, and regions) are mapped to semantic HTML/ARIA elements; complex grids/tables and rich text selection ranges are simplified.
