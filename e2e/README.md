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

## Formatting

```sh
# Verify formatting (Prettier, configured in .prettierrc.json)
npm run format:check

# Rewrite files in place
npm run format
```

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
- Deferred AccessKit activation via `WebAccessKitPlugin::new_lazy` (`/?lazy=1`), including focus restoration when the activation button is focused and removed.
- Real pointer clicks through the transparent overlay (`Increment counter`, `Enable notifications`), and keyboard `Space` on checkboxes and comboboxes.
- Dialog focus: keyboard Enter/Space opens it with focus inside, Tab/Shift+Tab wrap across all three dialog controls, Escape restores focus to the opener, Cancel/Confirm results reach the app, and a pointer-opened dialog leaves DOM focus on the canvas.
- Mirror geometry after a viewport resize (overlay matches the canvas rectangle).
- `WebAccessKitPlugin::teardown` (`/?teardown=1`): the mirror is removed and not re-mounted, with no errors.
- Every test fails on any `pageerror` or `console.error`, so wasm panics and logged errors can't pass silently.

The suite is browser-only, matching the crate: it runs against the wasm build of the demo, and the native build of the crate is covered by `cargo test` (mapper unit tests and the no-op plugin test).

## Manual screen reader checks

Automated tests verify the DOM and the accessibility tree, not what a screen reader speaks. Run this script before each release with a real screen reader, and record the result in the table. Use the demo at `/` (mirror on), or the published site.

Setup: NVDA with Chrome (Windows), or VoiceOver with Safari (macOS). Turn on speech viewer or speech history so every announcement is recorded.

| #   | Steps                                                                              | Expected (what to listen for)                                                                                              | Pass? |
| --- | ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- | ----- |
| 1   | Tab to `Password:`                                                                 | The field is announced as a password field. No characters are read. Typed characters are echoed as masked characters only. |       |
| 2   | Type two characters, then press Backspace three times                              | Each deleted character is announced once. The third Backspace is silent. No repeated "blank".                              |       |
| 3   | Tab to `Open dialog`, press Enter                                                  | Focus moves into `Confirmation dialog` (an alert dialog). Focus lands on `Close dialog`.                                   |       |
| 4   | In the dialog, press Tab repeatedly                                                | Focus cycles `Close dialog`, `Cancel`, `Confirm action`, then back. Focus never reaches the page behind the dialog.        |       |
| 5   | Press Escape                                                                       | The dialog closes and focus returns to `Open dialog`.                                                                      |       |
| 6   | Tab to `Search:`, then to `Note:`                                                  | `Search:` announces an autocomplete mode (set explicitly by the demo). `Note:` does not.                                   |       |
| 7   | Press the `Change section heading level` button, then navigate by headings (H key) | The section heading is announced at level 3 after the change. Focus stays on the button.                                   |       |
| 8   | Press `Turn screen reader mirror off`                                              | The page reloads without the mirror. Turn the mirror back on from a page opened without `?bridge=off`.                     |       |

Known limits found by this script's authors:

- The lazy activation control (`?lazy=1`) is 1px and `pointer-events: none`. It is meant for assistive technology and cannot be reached with a mouse.
- Headings are not focusable in egui, so the focus loss a recreated element could cause is not reachable from the demo. Focus stays on the control that changes a heading's level (covered by an e2e test).

## Investigations and open reports

- **"edit has autocomplete" announcement.** Not reproduced. The bridge only emits `aria-autocomplete` when an application sets an AccessKit autocomplete mode, and the demo sets it on `Search:` alone. A Chromium accessibility-tree check of plain text inputs (with and without `autocomplete="off"`, `aria-autocomplete="none"`, `role="textbox"`, `type="search"`) found no autocomplete property. The report still needs the exact field, markup or application code, and the screen reader version.
- **Password value exposure.** Fixed: the DOM mask is one ASCII `*` per character. Chromium exposes one glyph per UTF-8 byte, so the old U+2022 mask was read as three glyphs per character.
- **ComboBox keyboard activation on 0.35 and 0.36.** Not root-caused. The step is removed on those branches (see the root README).
