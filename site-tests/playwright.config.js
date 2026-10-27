// Run by scripts/test-site.sh, which builds the website, serves it on a free
// local port and passes its address in SITE_URL.
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  forbidOnly: true,
  retries: 0,
  workers: 1,
  reporter: "list",
  // The script points this into its temporary directory, which it removes.
  outputDir: process.env.SITE_TEST_OUTPUT ?? "test-results",
  use: {
    baseURL: process.env.SITE_URL,
    browserName: "chromium",
    headless: true,
  },
});
