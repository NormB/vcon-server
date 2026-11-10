// A dialog's inline recording plays in the player: a page of its own,
// opened in a new tab, whose policy lets a blob play and lets the page make
// no request. The console reads the vCon with the key, as a reveal of
// audio, checks the recording and hands its bytes to the player. A link in
// a vCon is never loaded, and bytes that are not the audio they are
// declared as are never handed over.
import { randomUUID } from "node:crypto";
import { test as base, expect, signIn, store, openView, auditLog, stored, nothingStored, wav, inline } from "./fixtures.js";

/** A vCon with these dialogs, as bytes. */
function call(uuid, dialog) {
  const body = { uuid, created_at: "2026-10-23T12:00:00Z", parties: [{ tel: "+15555550123", name: "Alice Example" }], dialog };
  return Buffer.from(JSON.stringify(body));
}

/** The text the player shows for a recording it was handed. */
function ready(uuid, dialog = 1, type = "audio/wav") {
  return `Ready to play: Dialog ${dialog} of ${uuid}, ${type}. It does not start by itself: use the controls below.`;
}

const WAITING = "Waiting for the console to hand over a recording.";
const NO_CONSOLE =
  "This page plays a recording the console hands it, and nothing else. Open a vCon in the console and choose Play.";

/**
 * Every content-security violation in any tab of the test, the player's
 * too: a test that expects none says so with `violations`.
 */
const test = base.extend({
  violations: async ({ context }, use) => {
    const seen = [];
    await context.exposeFunction("tabViolation", (v) => seen.push(v));
    await context.addInitScript(() => {
      // On the window, which outlives the empty document a new tab starts
      // with; once, however often this runs.
      if (window.tabViolationWatched !== true) {
        window.tabViolationWatched = true;
        window.addEventListener("securitypolicyviolation", (e) => window.tabViolation(`${e.violatedDirective} ${e.blockedURI}`));
      }
    });
    await use(seen);
  },
});

/** Stores a vCon with these dialogs and opens it masked in the vCons view. */
async function opened(page, request, tenant, dialog = [inline(wav())], scopes = ["read"]) {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), call(uuid, dialog));
  await signIn(page, await tenant.key(scopes));
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  await expect(page.locator("#vcon")).toBeVisible();
  return uuid;
}

/** Chooses a Play button and answers the player's tab. */
async function play(page, button = "#vcon-play-1") {
  const [player] = await Promise.all([page.waitForEvent("popup"), page.click(button)]);
  return player;
}

/** The actions the tenant's log holds for `uuid`, without the create. */
async function actions(request, tenant, uuid) {
  const log = await auditLog(request, await tenant.key(["audit"]), "/audit");
  return log.filter((r) => r.target === uuid && r.action !== "create").map((r) => [r.action, r.status]);
}

test("a dialog with a recording inside the vCon has a Play button that says it opens a new tab", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  await expect(page.locator("#vcon-play-1")).toHaveText("Play dialog 1 recording (opens the player in a new tab)");
  await expect(page.locator("#vcon-recordings button")).toHaveCount(1);
});

test("Play opens the player, which names the recording and shows the browser's controls", async ({ page, request, tenant, violations }) => {
  const uuid = await opened(page, request, tenant);
  const player = await play(page);
  await expect(player.locator("#playing")).toHaveText(ready(uuid));
  const audio = player.locator("#audio");
  await expect(audio).toBeVisible();
  await expect(audio).toHaveAttribute("controls", "");
  await expect(audio).toHaveAttribute("src", /^blob:https:\/\/localhost:\d+\//);
  await expect(page.locator("#vcon-recordings-state")).toHaveText("Dialog 1's recording was handed to the player's tab.");
  expect(violations).toEqual([]);
});

test("the recording the player holds is the one stored, and it does not start by itself", async ({ page, request, tenant }) => {
  await opened(page, request, tenant, [inline(wav(0.5))]);
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  const state = await player.locator("#audio").evaluate(
    (audio) =>
      new Promise((resolve) => {
        const answer = () => resolve({ duration: audio.duration, paused: audio.paused, autoplay: audio.autoplay, at: audio.currentTime });
        if (audio.readyState >= 1) {
          answer();
        } else {
          audio.addEventListener("loadedmetadata", answer, { once: true });
          audio.load();
        }
      }),
  );
  expect(state.duration).toBeCloseTo(0.5, 2);
  expect(state).toMatchObject({ paused: true, autoplay: false, at: 0 });
});

test("Play is recorded as a reveal of audio", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  expect(await actions(request, tenant, uuid)).toEqual([
    ["read", 200],
    ["reveal_audio", 200],
  ]);
});

test("Play in the Legal view is recorded as a legal reveal of audio", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), call(uuid, [inline(wav())]));
  await signIn(page, await tenant.key(["legal"]));
  await openView(page, "legal");
  await page.fill("#legal-uuid", uuid);
  await page.click("#legal-find button[type=submit]");
  const player = await play(page, "#legal-play-1");
  await expect(player.locator("#playing")).toHaveText(ready(uuid));
  expect(await actions(request, tenant, uuid)).toContainEqual(["legal_reveal_audio", 200]);
});

test("the player asks the server for its own files and for nothing else", async ({ page, request, tenant, context }) => {
  await opened(page, request, tenant);
  const asked = [];
  context.on("request", (r) => {
    let fromConsole;
    try {
      fromConsole = r.frame().page() === page;
    } catch {
      // The request that opens the player's tab has no frame yet.
      fromConsole = false;
    }
    if (!fromConsole) {
      asked.push(new URL(r.url()).protocol === "blob:" ? "blob:" : new URL(r.url()).pathname);
    }
  });
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  await player.locator("#audio").evaluate((audio) => audio.load());
  expect([...new Set(asked)].sort()).toEqual(["/console/console.css", "/console/format.js", "/console/player.html", "/console/player.js", "blob:"]);
});

test("the player's page can make no request: its policy refuses it", async ({ page, request, tenant, violations }) => {
  await opened(page, request, tenant);
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  const outcome = await player.evaluate(() =>
    fetch("/health").then(
      () => "answered",
      () => "refused",
    ),
  );
  expect(outcome).toBe("refused");
  await expect.poll(() => violations.filter((v) => v.startsWith("connect-src")).length).toBe(1);
});

test("the console's own page still can play nothing: its policy is as it was", async ({ context }) => {
  // A page of the test's own, not the fixture's, which fails a test on any
  // violation: here the violation is what is looked for.
  const page = await context.newPage();
  await page.goto("/console/");
  const outcome = await page.evaluate(
    () =>
      new Promise((resolve) => {
        document.addEventListener("securitypolicyviolation", (e) => resolve(e.violatedDirective), { once: true });
        const audio = new Audio();
        audio.addEventListener("loadedmetadata", () => resolve("loaded"));
        audio.src = URL.createObjectURL(new Blob([new Uint8Array(4)], { type: "audio/wav" }));
      }),
  );
  expect(outcome).toMatch(/^(media-src|default-src)$/);
});

test("a recording at a link has no Play button, and nothing asks for the link", async ({ page, request, tenant, context }) => {
  const asked = [];
  context.on("request", (r) => asked.push(r.url()));
  await opened(page, request, tenant, [{ type: "recording", start: "2026-10-23T12:00:00Z", parties: [0], mediatype: "audio/wav", url: "https://media.example.com/a.wav" }]);
  await expect(page.locator("#vcon-summary")).toContainText("content at a link outside the server");
  await expect(page.locator("#vcon-recordings button")).toHaveCount(0);
  expect(asked.filter((u) => !u.startsWith("https://localhost:"))).toEqual([]);
});

test("a body declared as markup or video has no Play button, whatever it holds", async ({ page, request, tenant }) => {
  const svg = Buffer.from("<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>");
  await opened(page, request, tenant, [inline(svg, "image/svg+xml"), inline(Buffer.from("<!doctype html><script>alert(1)</script>"), "text/html"), inline(wav(), "video/mp4")]);
  await expect(page.locator("#vcon-summary")).toContainText("Dialog 3:");
  await expect(page.locator("#vcon-recordings button")).toHaveCount(0);
});

test("bytes that are not the audio they are declared as are never handed to the player", async ({ page, request, tenant, context }) => {
  const svg = Buffer.from("<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>");
  await opened(page, request, tenant, [inline(svg, "audio/wav")]);
  const player = await play(page);
  await expect(page.locator("#vcon-recordings-state")).toHaveText(
    "Dialog 1's recording is declared as audio/wav, but its bytes are not audio/wav. The console does not play it. Download the vCon to examine the recording.",
  );
  await expect.poll(() => player.isClosed()).toBe(true);
  expect(context.pages()).toHaveLength(1);
});

test("a Play the server refuses says why and leaves no player open", async ({ page, request, tenant, context }) => {
  const uuid = await opened(page, request, tenant);
  await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${await tenant.key(["delete"])}` } });
  const player = await play(page);
  await expect(page.locator("#vcon-message")).toContainText("(410)");
  await expect.poll(() => player.isClosed()).toBe(true);
  expect(context.pages()).toHaveLength(1);
  expect(await actions(request, tenant, uuid)).toContainEqual(["reveal_audio", 410]);
});

test("a blocked tab is said, with what to do, and nothing is read", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  await page.evaluate(() => {
    window.open = () => null;
  });
  await page.click("#vcon-play-1");
  await expect(page.locator("#vcon-recordings-state")).toHaveText(
    "The browser blocked the player's tab. Allow pop-ups for this site, then choose Play again.",
  );
  expect(await actions(request, tenant, uuid)).toEqual([["read", 200]]);
});

test("the player opened by itself says it plays only what the console hands it", async ({ page }) => {
  await page.goto("/console/player.html");
  await expect(page.locator("#playing")).toHaveText(NO_CONSOLE);
  await expect(page.locator("#audio")).toBeHidden();
  expect(await page.locator("#audio").getAttribute("src")).toBeNull();
});

/** Opens the player from the console's page without choosing Play, and answers its tab. */
async function bare(page) {
  const [player] = await Promise.all([
    page.waitForEvent("popup"),
    page.evaluate(() => {
      // The player says when it can take a message; one posted before
      // that is lost.
      window.addEventListener("message", (event) => {
        if (event.data === "player-ready") {
          window.testPlayerReady = true;
        }
      });
      window.testPlayer = window.open("player.html", "_blank");
    }),
  ]);
  await page.waitForFunction(() => window.testPlayerReady === true);
  await expect(player.locator("#playing")).toHaveText(WAITING);
  return player;
}

/** In the console's page: posts `message` (with `bytes` as its buffer) to the player `bare` opened. */
async function post(page, message, bytes) {
  await page.evaluate(
    ([m, b]) => {
      const data = b === null ? m : { ...m, bytes: Uint8Array.from(b).buffer };
      window.testPlayer.postMessage(data, location.origin);
    },
    [message, bytes === null ? null : Array.from(bytes)],
  );
}

test("a message that is not a recording is ignored, and the next good one is taken", async ({ page }) => {
  await page.goto("/console/");
  const player = await bare(page);
  const svg = Buffer.from("<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>");
  await post(page, "play this", null);
  await post(page, { kind: "recording", type: "audio/wav", label: "no bytes" }, null);
  await post(page, { kind: "recording", type: "image/svg+xml", label: "markup" }, svg);
  await post(page, { kind: "recording", type: "audio/wav", label: "markup declared as audio" }, svg);
  await post(page, { kind: "other", type: "audio/wav", label: "another kind" }, wav());
  await post(page, { kind: "recording", type: "audio/wav", label: "x".repeat(201) }, wav());
  await post(page, { kind: "recording", type: "audio/wav", label: "the good one" }, wav());
  await expect(player.locator("#playing")).toHaveText("Ready to play: the good one. It does not start by itself: use the controls below.");
  await expect(player.locator("#audio")).toHaveAttribute("src", /^blob:/);
});

test("a message from a window that is not the console that opened the player is ignored", async ({ page }) => {
  await page.goto("/console/");
  const player = await bare(page);
  await player.evaluate((b) => {
    const data = { kind: "recording", type: "audio/wav", label: "from the player itself", bytes: Uint8Array.from(b).buffer };
    window.postMessage(data, location.origin);
  }, Array.from(wav()));
  // A round trip through the page, so the message has been delivered.
  await player.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)));
  await expect(player.locator("#playing")).toHaveText(WAITING);
  await expect(player.locator("#audio")).toBeHidden();
});

test("a page of another origin that opens the player can hand it nothing", async ({ page, baseURL, context }) => {
  // The same server by its address is another origin to the browser.
  const other = baseURL.replace("localhost", "127.0.0.1");
  await page.goto(`${other}/console/`);
  const [player] = await Promise.all([
    context.waitForEvent("page"),
    page.evaluate((url) => {
      window.testPlayer = window.open(url, "_blank");
    }, `${baseURL}/console/player.html`),
  ]);
  await expect(player.locator("#playing")).toHaveText(NO_CONSOLE);
  await page.evaluate((b) => {
    const data = { kind: "recording", type: "audio/wav", label: "from another origin", bytes: Uint8Array.from(b).buffer };
    try {
      window.testPlayer.postMessage(data, "*");
    } catch {
      // A window cut off from its opener takes no message.
    }
  }, Array.from(wav()));
  await player.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)));
  await expect(player.locator("#playing")).toHaveText(NO_CONSOLE);
  await expect(player.locator("#audio")).toBeHidden();
});

test("a second recording is not taken by a player that already holds one", async ({ page }) => {
  await page.goto("/console/");
  const player = await bare(page);
  await post(page, { kind: "recording", type: "audio/wav", label: "the first" }, wav());
  await expect(player.locator("#playing")).toContainText("the first");
  const first = await player.locator("#audio").getAttribute("src");
  await post(page, { kind: "recording", type: "audio/wav", label: "the second" }, wav(0.3));
  await player.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)));
  await expect(player.locator("#playing")).toContainText("the first");
  expect(await player.locator("#audio").getAttribute("src")).toBe(first);
});

test("when playback ends the player releases the recording and says so", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  const url = await player.locator("#audio").getAttribute("src");
  await player.locator("#audio").evaluate((audio) => audio.dispatchEvent(new Event("ended")));
  await expect(player.locator("#playing")).toHaveText(
    "Playback ended, and the recording was released from this page. Choose Play in the console to hear it again.",
  );
  await expect(player.locator("#audio")).toBeHidden();
  expect(await player.locator("#audio").getAttribute("src")).toBeNull();
  const again = await player.evaluate(
    (old) =>
      new Promise((resolve) => {
        const probe = new Audio();
        probe.addEventListener("loadedmetadata", () => resolve("still there"));
        probe.addEventListener("error", () => resolve("released"));
        probe.src = old;
      }),
    url,
  );
  expect(again).toBe("released");
});

test("a player that is closing releases the recording before it goes", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  const url = await player.locator("#audio").getAttribute("src");
  // The page's own leaving event, sent without closing it, so that what it
  // did can be looked at.
  await player.evaluate(() => window.dispatchEvent(new Event("pagehide")));
  expect(await player.locator("#audio").getAttribute("src")).toBeNull();
  const again = await player.evaluate(
    (old) =>
      new Promise((resolve) => {
        const probe = new Audio();
        probe.addEventListener("loadedmetadata", () => resolve("still there"));
        probe.addEventListener("error", () => resolve("released"));
        probe.src = old;
      }),
    url,
  );
  expect(again).toBe("released");
  await expect(page.locator("#vcon-recordings-state")).toHaveText("The player was closed.");
});

test("a recording the browser cannot play says so and what to do", async ({ page }) => {
  await page.goto("/console/");
  const player = await bare(page);
  // The signature of a WAV file, and nothing a decoder can use after it.
  const broken = Buffer.concat([Buffer.from("RIFF\x24\x00\x00\x00WAVE", "latin1"), Buffer.alloc(64, 0x7f)]);
  await post(page, { kind: "recording", type: "audio/wav", label: "broken" }, broken);
  await expect(player.locator("#player-error")).toHaveText(
    "The browser could not play this recording. Download the vCon in the console to hear it another way.",
  );
  await expect(player.locator("#audio")).toBeHidden();
});

test("closing the player returns focus to the Play button and says the player was closed", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  // Focus is put somewhere else in the console first, so that it is the
  // console that returns it and not the browser that left it there.
  await page.evaluate(() => document.getElementById("uuid").focus());
  // The click closes the tab it is made in, so the click itself may end
  // with "the page has been closed"; that the tab closed is what is waited
  // for, and what the console then shows is what is checked.
  await Promise.all([player.waitForEvent("close"), player.click("#close").catch(() => {})]);
  await expect(page.locator("#vcon-recordings-state")).toHaveText("The player was closed.");
  await expect(page.locator("#vcon-play-1")).toBeFocused();
});

test("signing out closes the player the console opened", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  await page.click("#sign-out");
  await expect.poll(() => player.isClosed()).toBe(true);
});

test("the player keeps nothing in the browser, and its address names no vCon", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const player = await play(page);
  await expect(player.locator("#audio")).toBeVisible();
  expect(await stored(player)).toEqual(nothingStored);
  const url = new URL(player.url());
  expect([url.pathname, url.search, url.hash]).toEqual(["/console/player.html", "", ""]);
  expect(await stored(page)).toEqual(nothingStored);
});

test("each recording has a button of its own, and the player names the one chosen", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant, [inline(wav()), { type: "text", start: "2026-10-23T12:00:00Z", parties: [0], mediatype: "text/plain", encoding: "none", body: "hello there" }, inline(wav(0.3))]);
  await expect(page.locator("#vcon-recordings button")).toHaveText([
    "Play dialog 1 recording (opens the player in a new tab)",
    "Play dialog 3 recording (opens the player in a new tab)",
  ]);
  const player = await play(page, "#vcon-play-3");
  await expect(player.locator("#playing")).toHaveText(ready(uuid, 3));
  // The third dialog's recording is 0.3 s long and the first's 0.2 s.
  await expect.poll(() => player.locator("#audio").evaluate((audio) => audio.duration)).toBeCloseTo(0.3, 2);
});

test("opening another vCon replaces the Play buttons with its own", async ({ page, request, tenant }) => {
  const first = await opened(page, request, tenant, [inline(wav()), inline(wav())]);
  await expect(page.locator("#vcon-recordings button")).toHaveCount(2);
  const second = randomUUID();
  await store(request, await tenant.key(["write"]), call(second, [{ type: "text", start: "2026-10-23T12:00:00Z", parties: [0], mediatype: "text/plain", encoding: "none", body: "hello" }]));
  await page.fill("#uuid", second);
  await page.click("#open-uuid");
  await expect(page.locator("#vcon h3")).toHaveText(second);
  await expect(page.locator("#vcon-recordings button")).toHaveCount(0);
  expect(first).not.toBe(second);
});

test("Play and the player need only the keyboard, and focus goes to the player's controls", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  await page.locator("#vcon-play-1").focus();
  const [player] = await Promise.all([page.waitForEvent("popup"), page.keyboard.press("Enter")]);
  await expect(player.locator("#audio")).toBeFocused();
  await player.keyboard.press("Tab");
  // The audio element's own controls take Tab stops first; Close follows.
  for (let i = 0; i < 12 && !(await player.locator("#close").evaluate((b) => b === document.activeElement)); i++) {
    await player.keyboard.press("Tab");
  }
  await expect(player.locator("#close")).toBeFocused();
  // Enter going down is the click, which closes the tab the key is pressed
  // in, so the press itself may end with "the page has been closed".
  await Promise.all([player.waitForEvent("close"), player.keyboard.down("Enter").catch(() => {})]);
  expect(player.isClosed()).toBe(true);
});
