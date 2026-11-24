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

    // 6. Zoomed coordinate alignment with real mouse click on <canvas>
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
