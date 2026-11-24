# egui-web-accesskit

A drop-in `egui::Plugin` that mirrors `egui`'s AccessKit tree to a real HTML/ARIA DOM overlay when running on `wasm32`, giving screen readers and browser accessibility tools visibility into `egui` widgets without patching `egui` or `eframe`.

## Version Compatibility

This repository mirrors `egui`'s minor version so there are no feature flags to juggle:

| `egui` / `eframe` | `egui-web-accesskit` | Branch / Tag |
| --- | --- | --- |
| `0.37` | `0.37` | `main` / `v0.37.0` |
| `0.36` | `0.36` | `egui-0.36` / `v0.36.0` |
| `0.35` | `0.35` | `egui-0.35` / `v0.35.0` |

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
When unmounting an `egui::Context` outside `eframe::WebRunner::destroy` (or on `egui 0.35`/`0.36`, which do not have `Plugin::on_exit`), call `WebAccessKitPlugin::teardown(&ctx)` to remove the DOM overlay and break the repaint-wakeup reference cycle.

On non-`wasm32` targets the plugin is a clean no-op so native builds can keep using `eframe` / `egui-winit`'s built-in desktop AccessKit adapters without `#[cfg]` clutter at the call site.

## How It Works

1. `Plugin::setup` enables AccessKit on the `egui::Context` (`ctx.enable_accesskit()`), mounts a visually transparent `<div data-egui-accesskit="true" role="application">` aligned over the target `<canvas>`, and registers passive `window` `"scroll"` / `"resize"` listeners so `position: fixed` elements stay aligned even while `egui` is idle.
2. `Plugin::output_hook` skips intermediate discarded layout passes, clones `output.platform_output.accesskit_update`, applies incremental changes with `accesskit_consumer::Tree`, and patches the mirrored HTML/ARIA elements — including `<textarea>` (`aria-multiline`), `aria-level`, `aria-modal`, `aria-live`, `placeholder`, `readonly`/`aria-readonly`, UTF-16 text selection sync, and `window.devicePixelRatio`-scaled bounds.
3. `Plugin::input_hook` keeps `RawInput::focused` true while a mirrored DOM element has focus and drains DOM events (`click`, `focus` + `ScrollIntoView`, `input`, `select`/navigation `keyup` selection updates, `PageUp`/`PageDown` scroll actions, and arrow keys on unstepped sliders) into `RawInput::events` as `egui::Event::AccessKitActionRequest`.
4. `Plugin::on_exit` / `WebAccessKitPlugin::teardown` tears down DOM event listeners (including capture-phase `window` listeners) and removes the overlay container.

## Running Tests & Demo

Run unit and round-trip tests natively:

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

Run the Playwright browser E2E suite:

```sh
cd examples/demo
trunk build
cd ../../e2e
npm install
npm test
```

## Known Limitations

- **`egui 0.35` / `0.36` upstream coverage:** On `egui 0.35` and `0.36`, `egui` itself does not yet handle `Action::SetValue` on `TextEdit`, emit `is_expanded` on `CollapsingHeader`, or emit `read_only` on `&str` `TextEdit`; text reading, focus, scroll actions, buttons, checkboxes, and sliders work on `0.35`/`0.36`, while remote DOM text editing, `readonly` from `&str`, and `aria-expanded` require `egui 0.37`.
- **IME composition:** `eframe`'s internal `TextAgent` IME flow is independent of the mirrored DOM text inputs.
- **Role coverage:** Core interactive controls (`Button`, `DisclosureTriangle`, `Tab`, `MenuItem`, `CheckBox` including mixed tri-state, `Switch`, `RadioButton`, `Slider` stepped and unstepped, single-line and multiline `TextEdit`, `PasswordInput`, `Label`, `Link`, `Window`, `Dialog`, `Alert`, headings with `aria-level`, lists, and regions) are mapped to semantic HTML/ARIA elements; complex grids/tables remain simplified.
