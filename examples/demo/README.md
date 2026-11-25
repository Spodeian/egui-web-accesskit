# egui-web-accesskit Demo App

Standalone two-column `eframe` showcase exercising the widgets, ARIA metadata, focus/scroll sync, popups (`ComboBox`), modal `AlertDialog` (keyboard-openable via Enter/Space, with focus trapping and restoration), polite `aria-live` status announcements, password masking, and Shadow DOM support mirrored by `egui-web-accesskit` (available and verified across all three version branches: `main` / `0.37`, `egui-0.36` / `0.36`, and `egui-0.35` / `0.35`).

## Prerequisites

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
```

## Running

- **In the browser (with live rebuilds):**
  ```sh
  trunk serve
  ```
  Open `http://127.0.0.1:8080/` for the standard light-DOM `<canvas>` setup, `http://127.0.0.1:8080/?shadow=1` to mount `<canvas>` inside an open `ShadowRoot` using `WebAccessKitPlugin::from_canvas`, or `http://127.0.0.1:8080/?lazy=1` to exercise deferred activation via `WebAccessKitPlugin::new_lazy`.

- **Build static dist bundle (used by the E2E tests):**
  ```sh
  trunk build
  ```

- **Teardown mode (`?teardown=1`):** unmounts the mirror with `WebAccessKitPlugin::teardown` a few frames after startup, used by the E2E suite.

> The demo is browser-only. Its egui and bridge dependencies are restricted to `wasm32`, so `cargo run` on a desktop target only prints an explanation and exits. The bridge itself is a no-op on native targets, so there is nothing to see there.

## Inspecting the Mirrored Accessibility Tree

1. Open Chrome DevTools and inspect `<div data-egui-accesskit="true">` (inside `#shadow-host`'s shadow root when `?shadow=1` is set).
2. Switch to the **Accessibility** tab in the Elements panel to inspect computed roles, names (`aria-label`), descriptions (`aria-description`), slider `aria-valuetext`, `aria-expanded`, heading levels, and live selection ranges while interacting with the UI.
3. Test with a screen reader (NVDA, Narrator, or VoiceOver) using keyboard navigation (`Tab`, `ArrowRight`/`ArrowLeft` on sliders, `Enter` inside `Bio:`, focus into the `ScrollArea`).
