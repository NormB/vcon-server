// A console page that breaks its content-security policy: the browser
// blocks what the page tried, reports it to the path the policy names, and
// the server counts the report in the admin audit log. The page here is a
// real one made to try a forbidden load; the console's own code breaks no
// rule, which every other test checks.
//
// The policies name the path with `report-uri` and not `report-to`. With
// `report-to csp` and a `Reporting-Endpoints` header in these answers,
// this browser sent no report to the test server within 100 seconds (seen
// on 2026-11-11, with the test certificate passed over and with its key
// trusted by flag); with `report-uri` alone it sends one within a second.
import { test, expect, auditLog } from "./fixtures.js";

const operator = process.env.VCON_OPERATOR_KEY;
const REPORT_PATH = "/console/csp-report";

/** How many reports the admin log counts under `status`. */
async function counted(request, status) {
  const log = await auditLog(request, operator, "/admin/audit");
  return log
    .filter((r) => r.action === "csp_report" && r.status === status)
    .reduce((sum, r) => sum + r.count, 0);
}

/** Makes the page try to load an image from another site. Resolves with the
 * directive the browser says was broken. Nothing is requested from the
 * other site: the browser refuses before it connects. */
function breakThePolicy(page) {
  return page.evaluate(
    () =>
      new Promise((resolve) => {
        document.addEventListener(
          "securitypolicyviolation",
          (e) => resolve(e.effectiveDirective),
          { once: true },
        );
        const image = new Image();
        image.src = "https://example.com/blocked.png";
        document.body.append(image);
      }),
  );
}

for (const [name, path] of [
  ["the console's page", "/console/"],
  ["the player's page", "/console/player.html"],
]) {
  test(`a violation on ${name} is reported to the server and counted`, async ({ context, request }) => {
    const before = await counted(request, 204);
    // The shared page fixture fails a test on any violation; this test
    // makes one on purpose, so it opens a page of its own.
    const page = await context.newPage();
    const answer = await page.goto(path);
    const policy = answer.headers()["content-security-policy"];
    expect(policy.endsWith(`; report-uri ${REPORT_PATH}`), policy).toBe(true);
    const report = page.waitForRequest((r) => new URL(r.url()).pathname === REPORT_PATH);
    expect(await breakThePolicy(page)).toBe("img-src");
    const sent = await report;
    expect(sent.method()).toBe("POST");
    expect(sent.headers()["content-type"]).toBe("application/csp-report");
    expect(sent.headers()["authorization"]).toBeUndefined();
    expect((await sent.response()).status()).toBe(204);
    // The server counts a report before it answers it.
    expect(await counted(request, 204)).toBe(before + 1);
    await page.close();
  });
}

test("a report the server refuses is counted under its status, without a key", async ({ playwright, baseURL }) => {
  // A context of its own, so that no Authorization header can come along.
  const anonymous = await playwright.request.newContext({ baseURL, ignoreHTTPSErrors: true });
  const keyed = await playwright.request.newContext({ baseURL, ignoreHTTPSErrors: true });
  const before = await counted(keyed, 415);
  const refused = await anonymous.post(REPORT_PATH, {
    headers: { "content-type": "text/plain" },
    data: "not a report",
  });
  expect(refused.status()).toBe(415);
  const taken = await anonymous.post(REPORT_PATH, {
    headers: { "content-type": "application/csp-report" },
    data: JSON.stringify({ "csp-report": { "violated-directive": "img-src" } }),
  });
  expect(taken.status()).toBe(204);
  expect(await counted(keyed, 415)).toBe(before + 1);
  await anonymous.dispose();
  await keyed.dispose();
});
