import { expect, test } from "@playwright/test";

// Post-deploy smoke test. Runs against a live site, so every path is relative to the deployed
// base URL (for example .../egui-web-accesskit/main/), not the origin root.
test.describe("deployed demo smoke test", () => {
  test("canvas renders, the mirror is mounted, and a click reaches the app", async ({ page }) => {
    const errors = [];
    page.on("pageerror", (err) => errors.push(err.message));
    page.on("console", (msg) => {
      if (msg.type() === "error") errors.push(msg.text());
    });

    await page.goto("./");
    await expect(page.locator("#the_canvas_id")).toBeVisible({ timeout: 30000 });

    const overlay = page.locator('[data-egui-accesskit="true"]');
    await expect(overlay).toBeAttached({ timeout: 30000 });

    await overlay.getByRole("button", { name: "Increment counter" }).evaluate((el) => el.click());
    await expect(overlay.getByText("Button clicked 1 times")).toBeAttached();

    expect(errors).toEqual([]);
  });
});
