// Run by scripts/test-console.sh, which starts the real server and passes
// its address and an operator key in VCON_URL and VCON_OPERATOR_KEY.
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  forbidOnly: true,
  retries: 0,
  workers: 1,
  reporter: "list",
  // The script points this into its temporary directory, which it removes.
  outputDir: process.env.VCON_TEST_OUTPUT ?? "test-results",
  use: {
    baseURL: process.env.VCON_URL,
    // The test server's certificate is made for the run and trusted by
    // nothing; the tests check the console, not the certificate.
    ignoreHTTPSErrors: true,
    browserName: "chromium",
    headless: true,
  },
});
