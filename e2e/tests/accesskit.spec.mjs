import { expect, test } from "@playwright/test";

test.describe("egui-web-accesskit E2E", () => {
  test("mirrors widgets, routes DOM/keyboard interactions, preserves focus, and tracks zoom", async ({
    page,
  }) => {
    const errors = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await page.goto("/");

    const overlay = page.locator('[data-egui-accesskit="true"]');
    await expect(overlay).toBeAttached({ timeout: 15000 });

    // 1. Initial accessibility mirror
    const heading = overlay.getByRole("heading", {
      name: "egui-web-accesskit demo",
      level: 1,
    });
    const incBtn = overlay.getByRole("button", { name: "Increment counter" });
    const notifyCheckbox = overlay.getByRole("checkbox", {
      name: "Enable notifications",
    });
    const triCheckbox = overlay.getByRole("checkbox", {
      name: "Tri-state option",
    });
    const opacitySlider = overlay.getByRole("slider", { name: "Opacity" });
    const volumeSlider = overlay.getByRole("slider", { name: "Volume" });
    const noteBox = overlay.getByRole("textbox", { name: "Note:" });
    const detailsBtn = overlay.getByRole("button", { name: "Advanced details" });

    await expect(heading).toHaveAttribute("aria-level", "1");
    await expect(incBtn).toBeAttached();
    await expect(notifyCheckbox).toHaveAttribute("aria-checked", "false");
    await expect(triCheckbox).toHaveAttribute("aria-checked", "mixed");
    await expect(opacitySlider).toHaveAttribute("step", "any");
    await expect(opacitySlider).toHaveValue("0.35");
    await expect(volumeSlider).toHaveAttribute("step", "5");
    await expect(volumeSlider).toHaveValue("50");
    await expect(noteBox).toHaveValue("Type here");
    await expect(detailsBtn).toHaveAttribute("aria-expanded", "false");

    // 2. Idle wakeup & click round-trip
    await incBtn.evaluate((el) => el.click());
    await expect(overlay.getByText("Button clicked 1 times")).toBeAttached();

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

    // 7. Idle outer-page scroll/offset realigns overlay without waking egui
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

    // 8. Zoomed coordinate alignment with real mouse click on <canvas>
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

    expect(errors).toEqual([]);
  });
});
