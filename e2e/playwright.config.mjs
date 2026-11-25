import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  timeout: 30000,
  use: {
    baseURL: "http://127.0.0.1:4173",
    headless: true,
  },
  projects: [
    {
      name: "chrome",
      use: {
        // Set PW_CHROME_PATH to run against a specific Chromium binary instead of Chrome.
        ...(process.env.PW_CHROME_PATH
          ? { launchOptions: { executablePath: process.env.PW_CHROME_PATH, args: ["--use-gl=angle", "--use-angle=swiftshader", "--no-sandbox"] } }
          : { channel: "chrome", launchOptions: { args: ["--use-gl=angle", "--use-angle=swiftshader"] } }),
      },
    },
    {
      name: "firefox",
      use: {
        browserName: "firefox",
      },
    },
  ],
  webServer: {
    command: "node serve.mjs",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
  },
});
