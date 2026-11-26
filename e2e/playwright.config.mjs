import { defineConfig } from "@playwright/test";

// PW_BASE_URL switches the config to "live" mode: the smoke spec runs against a deployed site
// (for example https://<owner>.github.io/egui-web-accesskit/main/) and no local server is started.
const liveBaseUrl = process.env.PW_BASE_URL;

export default defineConfig({
  testDir: "./tests",
  testMatch: liveBaseUrl ? "smoke.spec.mjs" : "accesskit.spec.mjs",
  timeout: 30000,
  // Retries stay at 0 so flakes are visible instead of being masked.
  retries: 0,
  reporter: [["list"], ["html", { open: "never", outputFolder: "playwright-report" }]],
  use: {
    baseURL: liveBaseUrl ?? "http://127.0.0.1:4173",
    headless: true,
    trace: "off",
  },
  projects: [
    {
      name: "chrome",
      use: {
        // Set PW_CHROME_PATH to run against a specific Chromium binary instead of Chrome.
        ...(process.env.PW_CHROME_PATH
          ? {
              launchOptions: {
                executablePath: process.env.PW_CHROME_PATH,
                args: ["--use-gl=angle", "--use-angle=swiftshader", "--no-sandbox"],
              },
            }
          : {
              channel: "chrome",
              launchOptions: { args: ["--use-gl=angle", "--use-angle=swiftshader"] },
            }),
      },
    },
    {
      name: "firefox",
      use: {
        browserName: "firefox",
      },
    },
  ],
  webServer: liveBaseUrl
    ? undefined
    : {
        command: "node serve.mjs",
        url: "http://127.0.0.1:4173",
        reuseExistingServer: false,
      },
});
