import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.describe("egui-web-accesskit E2E", () => {
  test("mirrors widgets, passes axe-core ARIA checks, routes DOM/keyboard/password/scroll interactions, preserves focus, and tracks zoom", async ({
    page,
  }) => {
    const errors = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await page.goto("/");

    const overlay = page.locator('[data-egui-accesskit="true"]');
    await expect(overlay).toBeAttached({ timeout: 15000 });

    // 1. Initial accessibility mirror & ARIA metadata
    const heading = overlay.getByRole("heading", {
      name: "egui-web-accesskit demo",
      level: 1,
    });
    const moreHeading = overlay.getByRole("heading", {
      name: "More controls",
      level: 2,
    });
    const liveStatus = overlay.locator('[aria-live="polite"]');
    const incBtn = overlay.getByRole("button", { name: "Increment counter" });
    const notifyCheckbox = overlay.getByRole("checkbox", {
      name: "Enable notifications",
    });
    const triCheckbox = overlay.getByRole("checkbox", {
      name: "Tri-state option",
    });
    const opacitySlider = overlay.getByRole("slider", { name: "Opacity" });
    const volumeSlider = overlay.getByRole("slider", { name: "Volume" });
    const docsLink = overlay.getByRole("link", { name: "egui docs" });
    const noteBox = overlay.getByRole("textbox", { name: "Note:" });
    const detailsBtn = overlay.getByRole("button", { name: "Advanced details" });
    const muteBtn = overlay.getByRole("button", { name: "Mute audio" });
    const compactRadio = overlay.getByRole("radio", { name: "Compact" });
    const spaciousRadio = overlay.getByRole("radio", { name: "Spacious" });
    const lockedBtn = overlay.getByRole("button", { name: "Locked action" });
    const retriesSpin = overlay.getByRole("spinbutton", { name: "Retries:" });
    const progressBar = overlay.getByRole("progressbar", {
      name: "Sync progress",
    });
    const themeCombo = overlay.getByRole("combobox", { name: "Theme" });

    await expect(heading).toHaveAttribute("aria-level", "1");
    await expect(moreHeading).toHaveAttribute("aria-level", "2");
    await expect(liveStatus).toHaveText("Audio active");
    await expect(incBtn).toBeAttached();
    await expect(incBtn).toHaveAttribute(
      "aria-description",
      "Adds one to the click counter"
    );
    await expect(notifyCheckbox).toHaveAttribute("aria-checked", "false");
    await expect(triCheckbox).toHaveAttribute("aria-checked", "mixed");
    await expect(opacitySlider).toHaveAttribute("step", "any");
    await expect(opacitySlider).toHaveValue("0.35");
    await expect(volumeSlider).toHaveAttribute("step", "5");
    await expect(volumeSlider).toHaveValue("50");
    await expect(volumeSlider).toHaveAttribute("aria-valuetext", "50%");
    await expect(docsLink).toHaveAttribute("href", "https://docs.rs/egui");
    await expect(noteBox).toHaveValue("Type here");
    await expect(detailsBtn).toHaveAttribute("aria-expanded", "false");
    await expect(muteBtn).toHaveAttribute("aria-pressed", "false");
    await expect(compactRadio).toHaveAttribute("aria-checked", "true");
    await expect(spaciousRadio).toHaveAttribute("aria-checked", "false");
    await expect(lockedBtn).toBeDisabled();
    await expect(lockedBtn).toHaveAttribute("aria-disabled", "true");
    await expect(retriesSpin).toHaveAttribute("min", "0");
    await expect(retriesSpin).toHaveAttribute("max", "10");
    await expect(retriesSpin).toHaveAttribute("step", "1");
    await expect(retriesSpin).toHaveValue("3");
    await expect(progressBar).toHaveAttribute("aria-valuenow", "75");
    await expect(themeCombo).toHaveAttribute("aria-expanded", "false");
    await expect(themeCombo).toHaveText("Dark");

    const axeResults = await new AxeBuilder({ page })
      .include('[data-egui-accesskit="true"]')
      .disableRules(["color-contrast"])
      .analyze();
    expect(axeResults.violations).toEqual([]);

    // 2. Idle wakeup & click round-trip + toggle button, live region, radio group, spinbutton
    await incBtn.evaluate((el) => el.click());
    await expect(overlay.getByText("Button clicked 1 times")).toBeAttached();

    await muteBtn.evaluate((el) => el.click());
    await expect(muteBtn).toHaveAttribute("aria-pressed", "true");
    await expect(liveStatus).toHaveText("Audio muted");

    await spaciousRadio.evaluate((el) => el.click());
    await expect(spaciousRadio).toHaveAttribute("aria-checked", "true");
    await expect(compactRadio).toHaveAttribute("aria-checked", "false");

    await retriesSpin.evaluate((el) => {
      el.focus();
      el.value = "7";
      el.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(retriesSpin).toHaveValue("7");

    // 2b. ComboBox open popup, hoisted options, axe-core check while open, keyboard Enter, and close
    await themeCombo.evaluate((el) => el.click());
    await expect(themeCombo).toHaveAttribute("aria-expanded", "true");
    const controlsId = await themeCombo.getAttribute("aria-controls");
    expect(controlsId).toBeTruthy();
    const listbox = overlay.getByRole("listbox");
    await expect(listbox).toBeAttached();
    const lightOpt = overlay.getByRole("option", { name: "Light" });
    await expect(lightOpt).toBeAttached();
    await expect(lightOpt).toHaveAttribute("aria-selected", "false");

    const comboAxe = await new AxeBuilder({ page })
      .include('[data-egui-accesskit="true"]')
      .disableRules(["color-contrast"])
      .analyze();
    expect(comboAxe.violations).toEqual([]);

    await lightOpt.focus();
    await page.keyboard.press("Enter");
    await expect(themeCombo).toHaveText("Light");
    await expect(lightOpt).toHaveAttribute("aria-selected", "true");
    await themeCombo.evaluate((el) => el.click());
    await expect(themeCombo).toHaveAttribute("aria-expanded", "false");

    // 2c. Modal AlertDialog + background inert trapping + axe-core check while open + close
    await overlay.getByRole("button", { name: "Open dialog" }).evaluate((el) => el.click());
    const modal = overlay.getByRole("alertdialog", { name: "Confirmation dialog" });
    await expect(modal).toBeAttached();
    await expect(modal).toHaveAttribute("aria-modal", "true");
    expect(await incBtn.evaluate((el) => el.closest("[inert]") !== null)).toBe(true);
    expect(await modal.evaluate((el) => el.closest("[inert]") !== null)).toBe(false);

    const modalAxe = await new AxeBuilder({ page })
      .include('[data-egui-accesskit="true"]')
      .disableRules(["color-contrast"])
      .analyze();
    expect(modalAxe.violations).toEqual([]);

    await overlay.getByRole("button", { name: "Close dialog" }).evaluate((el) => el.click());
    await expect(modal).toHaveCount(0);
    expect(await incBtn.evaluate((el) => el.closest("[inert]") !== null)).toBe(false);

    // 3. Disclosure click & aria-expanded update
    await detailsBtn.evaluate((el) => el.click());
    await expect(detailsBtn).toHaveAttribute("aria-expanded", "true");
    await expect(overlay.getByText("Hidden detail text")).toBeAttached();

    // 4. Unstepped float slider preserves 0.35 and steps on ArrowRight
    await opacitySlider.focus();
    await page.keyboard.press("ArrowRight");
    await expect
      .poll(async () => Number(await opacitySlider.inputValue()))
      .toBeGreaterThan(0.35);

    // 5. Sibling insertion above focused textbox keeps DOM focus, and clear resets value
    await noteBox.evaluate((el) => {
      el.focus();
      el.setSelectionRange(0, el.value.length);
    });
    await page.keyboard.type("!hello");
    await expect(overlay.getByText("Warning banner")).toBeAttached();
    await expect(noteBox).toHaveValue("!hello");
    const stillFocused = await noteBox.evaluate(
      (el) => el === document.activeElement
    );
    expect(stillFocused).toBe(true);

    await overlay.getByRole("button", { name: "Clear note" }).evaluate((el) =>
      el.click()
    );
    await expect(noteBox).toHaveValue("");

    // 5b. IME composition defers sync until compositionend commits
    await noteBox.evaluate((el) => {
      el.focus();
      el.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
      el.value = "ka";
      el.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          inputType: "insertCompositionText",
          data: "ka",
          isComposing: true,
        })
      );
    });
    const toggleWarningBtn = overlay.getByRole("button", { name: "Toggle warning" });
    await toggleWarningBtn.evaluate((el) => el.click());
    await expect(overlay.getByText("Warning banner")).toBeAttached();
    await expect(noteBox).toHaveValue("ka");
    await noteBox.evaluate((el) => {
      el.value = "漢";
      el.dispatchEvent(
        new CompositionEvent("compositionend", { bubbles: true, data: "漢" })
      );
    });
    await expect(noteBox).toHaveValue("漢");
    await toggleWarningBtn.evaluate((el) => el.click());
    await overlay.getByRole("button", { name: "Clear note" }).evaluate((el) =>
      el.click()
    );
    await expect(noteBox).toHaveValue("");

    // 6. Multiline <textarea>, placeholder, selection sync, read-only input, and ScrollIntoView
    const bioBox = overlay.getByLabel("Bio:");
    await expect(bioBox).toHaveJSProperty("tagName", "TEXTAREA");
    await expect(bioBox).toHaveAttribute("placeholder", "Write bio");
    await expect(bioBox).toHaveAttribute("aria-multiline", "true");
    await expect(bioBox).toHaveValue("Line 1");

    await bioBox.focus();
    await page.keyboard.press("End");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Line 2");
    await expect(bioBox).toHaveValue("Line 1\nLine 2");

    await bioBox.evaluate((el) => {
      el.setSelectionRange(0, 6);
      el.dispatchEvent(new Event("select"));
    });
    await page.waitForTimeout(150);
    const selRange = await bioBox.evaluate((el) => [
      el.selectionStart,
      el.selectionEnd,
    ]);
    expect(selRange).toEqual([0, 6]);

    const tokenBox = overlay.getByLabel("Token:");
    await expect(tokenBox).toHaveAttribute("readonly", "");
    await expect(tokenBox).toHaveAttribute("aria-readonly", "true");
    await expect(tokenBox).toHaveValue("RO-123");

    // 7. Password masking safety & ReplaceSelectedText / SetValue editing
    const pwBox = overlay.getByLabel("Password:");
    await expect(pwBox).toHaveAttribute("type", "password");
    await expect(overlay.getByText("Secret length: 6")).toBeAttached();
    const overlayHtml = await overlay.innerHTML();
    expect(overlayHtml).not.toContain("s3cr3t");
    const pwInitialValue = await pwBox.inputValue();
    expect(pwInitialValue).not.toContain("s3cr3t");
    expect(pwInitialValue.length).toBe(6);

    await pwBox.focus();
    await page.keyboard.press("End");
    await page.keyboard.type("!");
    await expect(overlay.getByText("Secret length: 7")).toBeAttached();

    await pwBox.evaluate((el) => {
      el.focus();
      el.setSelectionRange(0, el.value.length);
    });
    await page.keyboard.type("newpass12");
    await expect(overlay.getByText("Secret length: 9")).toBeAttached();

    // 8. ScrollArea ScrollIntoView on DOM focus + PageDown/PageUp scroll actions
    const topItemBtn = overlay.getByRole("button", { name: "Top item" });
    const topBefore = await topItemBtn.boundingBox();
    expect(topBefore).not.toBeNull();
    await topItemBtn.focus();
    await page.keyboard.press("PageDown");
    await expect
      .poll(async () => {
        const b = await topItemBtn.boundingBox();
        return b ? b.y : topBefore.y;
      })
      .toBeLessThan(topBefore.y - 20);

    const bottomBtn = overlay.getByRole("button", { name: "Bottom action" });
    const bottomBefore = await bottomBtn.boundingBox();
    expect(bottomBefore).not.toBeNull();
    await bottomBtn.focus();
    await expect
      .poll(async () => {
        const b = await bottomBtn.boundingBox();
        return b ? b.y : bottomBefore.y;
      })
      .toBeLessThan(bottomBefore.y - 20);

    const bottomCentered = await bottomBtn.boundingBox();
    await page.keyboard.press("PageUp");
    await expect
      .poll(async () => {
        const b = await bottomBtn.boundingBox();
        return b ? b.y : bottomCentered.y;
      })
      .toBeGreaterThan(bottomCentered.y + 20);

    // 9. Idle outer-page scroll/offset realigns overlay without waking egui
    const incBeforeScroll = await incBtn.boundingBox();
    expect(incBeforeScroll).not.toBeNull();
    await page.evaluate(() => {
      const canvas = document.getElementById("the_canvas_id");
      if (canvas) canvas.style.marginTop = "120px";
      window.dispatchEvent(new Event("scroll"));
    });
    await expect
      .poll(async () => {
        const b = await incBtn.boundingBox();
        return b ? b.y - incBeforeScroll.y : 0;
      })
      .toBeGreaterThan(100);
    await page.evaluate(() => {
      const canvas = document.getElementById("the_canvas_id");
      if (canvas) canvas.style.marginTop = "0px";
      window.dispatchEvent(new Event("scroll"));
    });
    await expect
      .poll(async () => {
        const b = await incBtn.boundingBox();
        return b ? Math.abs(b.y - incBeforeScroll.y) : 999;
      })
      .toBeLessThan(2);

    // 10. Zoomed coordinate alignment with real mouse click on <canvas>
    const boxBefore = await incBtn.boundingBox();
    expect(boxBefore).not.toBeNull();

    await overlay.getByRole("button", { name: "Zoom 1.5x" }).evaluate((el) =>
      el.click()
    );

    await expect
      .poll(async () => {
        const b = await incBtn.boundingBox();
        return b ? b.width / boxBefore.width : 0;
      })
      .toBeGreaterThan(1.4);

    const boxAfter = await incBtn.boundingBox();
    expect(boxAfter).not.toBeNull();
    await page.mouse.click(
      boxAfter.x + boxAfter.width / 2,
      boxAfter.y + boxAfter.height / 2
    );
    await expect(overlay.getByText("Button clicked 2 times")).toBeAttached();
    const activeInsideOverlay = await overlay.evaluate((el) =>
      el.contains(document.activeElement)
    );
    expect(activeInsideOverlay).toBe(false);

    expect(errors).toEqual([]);
  });

  test("supports Shadow DOM via WebAccessKitPlugin::from_canvas", async ({
    page,
  }) => {
    const errors = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await page.goto("/?shadow=1");

    const shadowOverlay = page.locator('#shadow-host [data-egui-accesskit="true"]');
    await expect(shadowOverlay).toBeAttached({ timeout: 15000 });

    const domPlacement = await page.evaluate(() => {
      const host = document.getElementById("shadow-host");
      const root = host?.shadowRoot;
      return {
        lightDomOverlay: document.querySelector('[data-egui-accesskit="true"]') !== null,
        shadowHasCanvas: root?.querySelector("#the_canvas_id") !== null,
        shadowHasOverlay: root?.querySelector('[data-egui-accesskit="true"]') !== null,
      };
    });
    expect(domPlacement).toEqual({
      lightDomOverlay: false,
      shadowHasCanvas: true,
      shadowHasOverlay: true,
    });

    await shadowOverlay
      .getByRole("button", { name: "Increment counter" })
      .evaluate((el) => el.click());
    await expect(
      shadowOverlay.getByText("Button clicked 1 times")
    ).toBeAttached();

    const noteBox = shadowOverlay.getByRole("textbox", { name: "Note:" });
    await noteBox.evaluate((el) => {
      el.focus();
      el.setSelectionRange(0, el.value.length);
    });
    await page.keyboard.type("!shadow");
    await expect(shadowOverlay.getByText("Warning banner")).toBeAttached();
    const activeInShadow = await noteBox.evaluate((el) => {
      const host = document.getElementById("shadow-host");
      return host?.shadowRoot?.activeElement === el;
    });
    expect(activeInShadow).toBe(true);

    expect(errors).toEqual([]);
  });
});
