// What the console works out before it shows anything: times in UTC, masked
// party values, a vCon's summary and what a refusal means. These run in
// Node, on the same file the server serves, with no browser and no server.
import { test, expect } from "@playwright/test";
import {
  utc,
  maskTel,
  maskMail,
  partyLine,
  summary,
  refusalText,
  dialogLine,
  section,
  links,
  shown,
  DISPLAY_LIMIT,
} from "../../crates/vcon-server/console/format.js";

/** A value as base64url, as a JWS carries its header and payload. */
function b64url(value) {
  return Buffer.from(JSON.stringify(value)).toString("base64url");
}

test.describe("times", () => {
  test("a UTC time is shown to the second with its offset stated", () => {
    expect(utc("2026-10-23T12:00:00Z")).toBe("2026-10-23 12:00:00 UTC+00:00");
  });
  test("a time with another offset is converted to UTC", () => {
    expect(utc("2026-10-23T14:30:05+02:00")).toBe("2026-10-23 12:30:05 UTC+00:00");
  });
  test("fractions of a second are dropped, not rounded up", () => {
    expect(utc("2026-10-23T12:00:00.999999Z")).toBe("2026-10-23 12:00:00 UTC+00:00");
  });
  test("no time is shown as nothing", () => {
    expect(utc(null)).toBe("");
    expect(utc(undefined)).toBe("");
    expect(utc("")).toBe("");
  });
  test("text that is not a time is shown as it was sent", () => {
    expect(utc("yesterday")).toBe("yesterday");
  });
  test("a value that is not text is shown as text, never read as a clock time", () => {
    expect(utc(1761220800)).toBe("1761220800");
  });
});

test.describe("masking", () => {
  test("a telephone number shows its last four digits only", () => {
    expect(maskTel("tel:+15555550123")).toBe("number ending 0123");
  });
  test("punctuation in a number is not counted as digits", () => {
    expect(maskTel("+1 (555) 555-0199")).toBe("number ending 0199");
  });
  test("a number too short to mask is hidden whole", () => {
    expect(maskTel("tel:5550123")).toBe("number hidden");
    expect(maskTel("")).toBe("number hidden");
  });
  test("a number that is not text is hidden whole", () => {
    expect(maskTel(15555550123)).toBe("number hidden");
    expect(maskTel(null)).toBe("number hidden");
  });
  test("an email address shows its domain only", () => {
    expect(maskMail("mailto:alice@example.com")).toBe("address at example.com");
    expect(maskMail("bob@example.com")).toBe("address at example.com");
  });
  test("the domain is the part after the last at sign, in lower case", () => {
    expect(maskMail('mailto:"a@b"@EXAMPLE.com')).toBe("address at example.com");
  });
  test("an address with no local part is hidden whole", () => {
    expect(maskMail("@example.com")).toBe("address hidden");
  });
  test("an address with no domain is hidden whole", () => {
    expect(maskMail("mailto:alice")).toBe("address hidden");
    expect(maskMail("alice@")).toBe("address hidden");
    expect(maskMail(null)).toBe("address hidden");
  });
  test("a party line names what the party holds and shows none of it", () => {
    const line = partyLine({ tel: "+15555550123", mailto: "alice@example.com", name: "Alice Example" }, 0);
    expect(line).toBe("Party 1: number ending 0123; address at example.com; name hidden");
    expect(line).not.toContain("Alice");
    expect(line).not.toContain("555555");
  });
  test("a party with nothing to mask says so", () => {
    expect(partyLine({}, 1)).toBe("Party 2: no number, address or name");
  });
  test("a party that is not an object is named and not read", () => {
    expect(partyLine("<b>Alice</b>", 2)).toBe("Party 3: not an object");
    expect(partyLine(null, 0)).toBe("Party 1: not an object");
    expect(partyLine(["x"], 0)).toBe("Party 1: not an object");
  });
});

test.describe("summary", () => {
  const plain = {
    uuid: "0198a1b2-0000-8000-8000-000000000001",
    created_at: "2026-10-23T12:00:00Z",
    subject: "Billing question",
    parties: [{ tel: "+15555550123", name: "Alice Example" }, { mailto: "bob@example.com" }],
    dialog: [{ type: "text", duration: 30 }, { type: "text", duration: 12.5 }],
    attachments: [{}],
    analysis: [{}, {}, {}],
  };

  test("an unsigned vCon is summarized by counts and masked parties", () => {
    expect(summary(JSON.stringify(plain))).toEqual({
      form: "unsigned",
      created: "2026-10-23 12:00:00 UTC+00:00",
      subject: "present, hidden",
      parties: ["Party 1: number ending 0123; name hidden", "Party 2: address at example.com"],
      dialogs: 2,
      dialog: [
        "Dialog 1: text, no start, 30 s, no parties; no content",
        "Dialog 2: text, no start, 12.5 s, no parties; no content",
      ],
      seconds: 42.5,
      attachments: 1,
      analyses: 3,
      links: 0,
      signature: null,
    });
  });
  test("the summary holds no name, subject, full number or address", () => {
    const text = JSON.stringify(summary(JSON.stringify(plain)));
    for (const hidden of ["Alice", "Billing", "5555550123", "bob@"]) {
      expect(text).not.toContain(hidden);
    }
  });
  test("a vCon with no optional parts counts zero of each", () => {
    expect(summary('{"uuid":"u"}')).toEqual({
      form: "unsigned",
      created: "",
      subject: "none",
      parties: [],
      dialogs: 0,
      dialog: [],
      seconds: 0,
      attachments: 0,
      analyses: 0,
      links: 0,
      signature: null,
    });
  });
  test("parts of the wrong type count as none instead of failing", () => {
    const odd = summary(JSON.stringify({ parties: "x", dialog: { a: 1 }, attachments: 7, analysis: null, subject: 5 }));
    expect(odd.parties).toEqual([]);
    expect(odd.dialogs).toBe(0);
    expect(odd.attachments).toBe(0);
    expect(odd.analyses).toBe(0);
    expect(odd.subject).toBe("none");
  });
  test("a duration that is not a positive number adds nothing", () => {
    const body = { dialog: [{ duration: "90" }, { duration: -5 }, { duration: 7 }, {}, null] };
    expect(summary(JSON.stringify(body)).seconds).toBe(7);
    expect(summary(JSON.stringify(body)).dialogs).toBe(5);
  });
  test("a signed vCon shows its header and the summary of its payload", () => {
    const signed = {
      payload: b64url(plain),
      signatures: [{ protected: b64url({ alg: "RS256", kid: "key-1", x5t: "abc" }), signature: "c2ln" }],
    };
    const s = summary(JSON.stringify(signed));
    expect(s.form).toBe("signed (JWS)");
    expect(s.signature).toEqual({ alg: "RS256", kid: "key-1", x5t: "abc" });
    expect(s.dialogs).toBe(2);
    expect(s.parties).toHaveLength(2);
  });
  test("a signed vCon whose header cannot be read still shows as signed", () => {
    const signed = { payload: "%%%", signatures: [{ protected: "%%%" }] };
    const s = summary(JSON.stringify(signed));
    expect(s.form).toBe("signed (JWS)");
    expect(s.signature).toEqual({ alg: "", kid: "", x5t: "" });
    expect(s.dialogs).toBe(0);
  });
  test("a header field that is not text is not shown", () => {
    const signed = { payload: b64url({}), signatures: [{ protected: b64url({ alg: { a: 1 }, kid: 7 }) }] };
    expect(summary(JSON.stringify(signed)).signature).toEqual({ alg: "", kid: "", x5t: "" });
  });
  test("an encrypted vCon shows as encrypted and nothing is decoded", () => {
    const s = summary(JSON.stringify({ protected: "eA", ciphertext: b64url(plain), iv: "aXY", tag: "dGFn" }));
    expect(s.form).toBe("encrypted (JWE)");
    expect(s.parties).toEqual([]);
    expect(s.dialogs).toBe(0);
    expect(s.subject).toBe("not readable");
  });
  test("a body that is not JSON is named as such", () => {
    expect(summary("<html>").form).toBe("not JSON");
    expect(summary("").form).toBe("not JSON");
  });
  test("a JSON body that is not an object is named as such", () => {
    expect(summary("[1,2]").form).toBe("not a JSON object");
    expect(summary("null").form).toBe("not a JSON object");
    expect(summary('"text"').form).toBe("not a JSON object");
  });
});

test.describe("refusals", () => {
  test("a refusal names the problem, its status and what to do", () => {
    expect(refusalText(404, "No such vCon")).toBe(
      "No such vCon (404). Check the uuid or id: it may be deleted or belong to another tenant.",
    );
  });
  test("a deleted vCon is told where it can still be found", () => {
    expect(refusalText(410, "Gone")).toBe(
      "Gone (410). It was deleted: a key with the legal scope can still find it in the Legal view.",
    );
  });
  test("a key that is no longer accepted is told to sign in again", () => {
    expect(refusalText(401, "Unauthorized")).toBe("Unauthorized (401). Sign in again with a key the server accepts.");
  });
  test("a missing scope is told which kind of key is needed", () => {
    expect(refusalText(403, "Forbidden")).toBe("Forbidden (403). Sign in with a key that holds the scope this needs.");
  });
  test("a changed version is told to open it again", () => {
    expect(refusalText(412, "Precondition failed")).toBe(
      "Precondition failed (412). It changed since you opened it: open it again, then repeat the change.",
    );
  });
  test("refused content is told to correct the file", () => {
    for (const status of [400, 415, 422]) {
      expect(refusalText(status, "Refused")).toBe(`Refused (${status}). Correct what was sent and try again.`);
    }
  });
  test("a body past the limit is told to send less", () => {
    expect(refusalText(413, "Too large")).toBe("Too large (413). Send a smaller file.");
  });
  test("a conflict is told to check the current state", () => {
    expect(refusalText(409, "Not deleted")).toBe("Not deleted (409). Reload the view and check its state before trying again.");
  });
  test("a rate limit is told to wait", () => {
    expect(refusalText(429, "Too many requests")).toBe("Too many requests (429). Wait a minute, then try again.");
  });
  test("a server failure is told to retry and whom to tell", () => {
    for (const status of [500, 503]) {
      expect(refusalText(status, "Failed")).toBe(
        `Failed (${status}). Try again; if it repeats, tell the server's operator.`,
      );
    }
  });
  test("a refusal with no readable problem shows the status alone", () => {
    expect(refusalText(418, undefined)).toBe("HTTP 418. Try again.");
    expect(refusalText(404, 7)).toBe("HTTP 404. Check the uuid or id: it may be deleted or belong to another tenant.");
  });
  test("no answer at all is told to check the connection", () => {
    expect(refusalText(0, undefined)).toBe("The server could not be reached. Check the connection and try again.");
  });
});

test.describe("dialog lines", () => {
  test("a dialog line shows its type, start, length and parties, and no content", () => {
    const d = { type: "text", start: "2026-10-23T12:00:00Z", duration: 30, parties: [0, 1], body: "hello there" };
    expect(dialogLine(d, 0)).toBe("Dialog 1: text, 2026-10-23 12:00:00 UTC+00:00, 30 s, parties 1, 2; content hidden");
  });
  test("a recording kept elsewhere says its content is at a link", () => {
    const d = { type: "recording", parties: 1, url: "https://media.example.com/a.wav" };
    const line = dialogLine(d, 2);
    expect(line).toBe("Dialog 3: recording, no start, no length, party 2; content at a link outside the server");
    expect(line).not.toContain("media.example.com");
  });
  test("a type the vCon format does not name is not shown", () => {
    expect(dialogLine({ type: "Alice said hello" }, 0)).toBe("Dialog 1: other type, no start, no length, no parties; no content");
  });
  test("each type the vCon format names is shown", () => {
    for (const type of ["recording", "text", "transfer", "incomplete"]) {
      expect(dialogLine({ type }, 0)).toContain(`Dialog 1: ${type},`);
    }
  });
  test("parties that are not whole numbers from zero are not shown", () => {
    for (const parties of [-1, 1.5, "0", [0, "x"], [], null, { a: 1 }]) {
      expect(dialogLine({ type: "text", parties }, 0), JSON.stringify(parties)).toContain(", no parties;");
    }
  });
  test("a start that is not a time is not shown", () => {
    expect(dialogLine({ type: "text", start: "Alice called" }, 0)).toContain(", no start,");
    expect(dialogLine({ type: "text", start: 7 }, 0)).toContain(", no start,");
  });
  test("a dialog that is not an object is named and not read", () => {
    expect(dialogLine("hello", 1)).toBe("Dialog 2: not an object");
    expect(dialogLine(null, 0)).toBe("Dialog 1: not an object");
  });
  test("the summary lists a line for each dialog", () => {
    const body = { dialog: [{ type: "text", body: "secret words" }, { type: "recording", url: "https://media.example.com/a" }] };
    const s = summary(JSON.stringify(body));
    expect(s.dialog).toEqual([
      "Dialog 1: text, no start, no length, no parties; content hidden",
      "Dialog 2: recording, no start, no length, no parties; content at a link outside the server",
    ]);
    expect(s.links).toBe(1);
  });
});

test.describe("sections", () => {
  const body = {
    subject: "Billing question",
    parties: [{ name: "Alice Example", tel: "+15555550123" }],
    dialog: [
      { type: "text", body: "hello there" },
      { type: "recording", encoding: "base64url", body: "QUJD".repeat(100), url: "https://media.example.com/a.wav" },
    ],
    analysis: [{ type: "transcript", body: "Alice: hello there" }],
    attachments: [{ url: "https://files.example.com/doc" }, { url: 7 }, "x"],
  };
  const text = JSON.stringify(body);

  test("the parties section is the parties and nothing else", () => {
    const shownText = section(text, "parties");
    expect(JSON.parse(shownText)).toEqual(body.parties);
    expect(shownText).not.toContain("hello there");
    expect(shownText).not.toContain("Billing");
  });
  test("the subject section is the subject alone", () => {
    expect(section(text, "subject")).toBe("Billing question");
  });
  test("the dialog section holds what was said and not the parties", () => {
    const shownText = section(text, "dialog");
    expect(shownText).toContain("hello there");
    expect(shownText).not.toContain("Alice Example");
  });
  test("the analysis section holds the transcript", () => {
    expect(section(text, "analysis")).toContain("Alice: hello there");
  });
  test("inline media is named by its size and not shown", () => {
    const shownText = section(text, "dialog");
    expect(shownText).not.toContain("QUJDQUJD");
    expect(shownText).toContain("400 characters of base64url, not shown");
  });
  test("a part the vCon does not have is shown as none", () => {
    for (const name of ["parties", "subject", "dialog", "analysis"]) {
      expect(section("{}", name), name).toBe("none");
    }
    expect(section('{"parties":[],"subject":""}', "parties")).toBe("none");
    expect(section('{"parties":[],"subject":""}', "subject")).toBe("none");
  });
  test("a signed vCon's sections come from its payload", () => {
    const signed = { payload: b64url(body), signatures: [{ protected: b64url({ alg: "RS256" }) }] };
    expect(section(JSON.stringify(signed), "subject")).toBe("Billing question");
  });
  test("a vCon that cannot be read has no sections", () => {
    expect(section("<html>", "parties")).toBeNull();
    expect(section("[1]", "dialog")).toBeNull();
    expect(section(JSON.stringify({ ciphertext: "eA" }), "subject")).toBeNull();
    expect(section(JSON.stringify({ payload: "%%", signatures: [] }), "subject")).toBeNull();
  });
  test("a section the console does not have is refused", () => {
    expect(section(text, "uuid")).toBeNull();
    expect(section(text, "__proto__")).toBeNull();
  });
  test("links are listed with where they are, as text", () => {
    expect(links(text)).toEqual([
      { where: "dialog 2", url: "https://media.example.com/a.wav" },
      { where: "attachment 1", url: "https://files.example.com/doc" },
    ]);
  });
  test("a link of any scheme is listed as the text it is", () => {
    const hostile = { dialog: [{ url: "javascript:alert(1)" }], analysis: [{ url: "data:text/html,<script>1</script>" }] };
    expect(links(JSON.stringify(hostile))).toEqual([
      { where: "dialog 1", url: "javascript:alert(1)" },
      { where: "analysis 1", url: "data:text/html,<script>1</script>" },
    ]);
  });
  test("a vCon with no links, or one that cannot be read, lists none", () => {
    expect(links("{}")).toEqual([]);
    expect(links("not json")).toEqual([]);
    expect(links(JSON.stringify({ ciphertext: "eA", dialog: [{ url: "https://x.example.com" }] }))).toEqual([]);
  });
});

test.describe("the display limit", () => {
  test("the limit is 2 MiB of characters", () => {
    expect(DISPLAY_LIMIT).toBe(2 * 1024 * 1024);
  });
  test("text within the limit is shown whole", () => {
    expect(shown("abc", 3)).toEqual({ text: "abc", left: 0 });
    expect(shown("", 3)).toEqual({ text: "", left: 0 });
  });
  test("text past the limit is cut at it and the rest counted", () => {
    expect(shown("abcdef", 4)).toEqual({ text: "abcd", left: 2 });
  });
  test("text is cut at the default limit when none is given", () => {
    const big = "x".repeat(DISPLAY_LIMIT + 5);
    const cut = shown(big);
    expect(cut.text).toHaveLength(DISPLAY_LIMIT);
    expect(cut.left).toBe(5);
  });
});
