# Playwright E2E Suite

End-to-end browser verification for `egui-web-accesskit` using Playwright, headless Chrome with SwiftShader WebGL, and `@axe-core/playwright`.

## Prerequisites

1. Build the demo app's WASM distribution bundle first:
   ```sh
   cd ../examples/demo
   trunk build
   ```
2. Install Node dependencies in `e2e/`:
   ```sh
   cd ../../e2e
   npm install
   ```
   (Ensure Google Chrome is installed or run `npx playwright install chrome`.)

## How It Works

- [serve.mjs](serve.mjs) serves `../examples/demo/dist` on `http://127.0.0.1:4173`.
- [playwright.config.mjs](playwright.config.mjs) starts `serve.mjs` automatically via `webServer`, runs against `channel: "chrome"` with `--use-gl=angle --use-angle=swiftshader` for software WebGL rendering, and enforces a 30s per-test timeout.

## Running Tests

```sh
# Headless Chrome (default)
npm test

# Headless Firefox (requires `npx playwright install firefox`)
npm run test:firefox

# Interactive headed Chrome window for debugging
npx playwright test --project=chrome --headed
```

## Coverage

[tests/accesskit.spec.mjs](tests/accesskit.spec.mjs) verifies:

- Semantic DOM mirroring, `aria-level`, `aria-description`, `aria-valuetext`, `aria-pressed`, radio groups (`aria-checked`), disabled controls, spinbuttons, progress bars, `aria-live="polite"`, link `href`, `aria-checked="mixed"`, `aria-expanded` / `aria-controls`, modal `inert` background trapping + `Escape` dismissal, stepped/unstepped `<input type="range">`, and zero `@axe-core/playwright` violations inside `[data-egui-accesskit="true"]` (including while `ComboBox` and modal `AlertDialog` popups are open).
- Idle repaint wakeup from DOM clicks, toggle buttons, disclosure toggles, `ComboBox` option selection (`Enter`), modal `AlertDialog`, and keyboard slider steps.
- DOM focus preservation across sibling insertion, single-line `Enter` submit forwarding, IME `compositionstart`/`compositionend` handling, canvas click focus non-interference, multiline `<textarea>` editing (`placeholder`, `aria-multiline`, newlines), and UTF-16 selection sync.
- Password masking safety (no plaintext in the DOM) and `ReplaceSelectedText` / `SetValue` edits.
- `ScrollArea` `PageDown` / `PageUp` scroll actions and `ScrollIntoView` on DOM focus, idle outer-page scroll/offset realignment, and `devicePixelRatio` zoom alignment against real `<canvas>` pointer clicks.
- Shadow DOM hosting via `WebAccessKitPlugin::from_canvas` (`/?shadow=1`), including `shadowRoot.activeElement` focus retention.
- Deferred AccessKit activation via `WebAccessKitPlugin::new_lazy` (`/?lazy=1`).
