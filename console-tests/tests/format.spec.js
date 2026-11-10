// What the console works out before it shows anything: times in UTC, masked
// party values, a vCon's summary and what a refusal means. These run in
// Node, on the same file the server serves, with no browser and no server.
import { readFileSync } from "node:fs";
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
import * as format from "../../crates/vcon-server/console/format.js";

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
  test("a key on the listener that serves the other kind is told to use the other address", () => {
    expect(
      refusalText(403, "This listener does not serve this kind of key", "https://vcon-server.com/problems/wrong-listener"),
    ).toBe(
      "This listener does not serve this kind of key (403). " +
        "Open the console at the server's other address and sign in there: " +
        "one address serves tenant keys and the other serves operator keys.",
    );
  });
  test("a missing scope keeps its own advice when the problem type is forbidden", () => {
    expect(refusalText(403, "Forbidden", "https://vcon-server.com/problems/forbidden")).toBe(
      "Forbidden (403). Sign in with a key that holds the scope this needs.",
    );
  });
  test("a problem type that is not text, or is absent, changes nothing", () => {
    for (const type of [undefined, null, 7, {}, ["https://vcon-server.com/problems/wrong-listener"], ""]) {
      expect(refusalText(403, "Forbidden", type)).toBe("Forbidden (403). Sign in with a key that holds the scope this needs.");
    }
  });
  test("only a type that ends in the problem's name is the wrong listener", () => {
    for (const type of ["wrong-listener/other", "https://example.com/problems/wrong-listener-not", "WRONG-LISTENER"]) {
      expect(refusalText(403, "Forbidden", type)).toBe("Forbidden (403). Sign in with a key that holds the scope this needs.");
    }
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

test.describe("the reveal marker", () => {
  test("a reveal of a part reads the path with that part as its marker", () => {
    for (const part of ["parties", "subject", "dialog", "analysis", "links", "all"]) {
      expect(format.revealPath("/vcons/0199e5c6-0d7a-7c11-8000-000000000001", part)).toBe(
        `/vcons/0199e5c6-0d7a-7c11-8000-000000000001?reveal=${part}`,
      );
    }
  });
  test("a read that reveals nothing is the path with no marker", () => {
    expect(format.revealPath("/vcons/0199e5c6-0d7a-7c11-8000-000000000001", null)).toBe(
      "/vcons/0199e5c6-0d7a-7c11-8000-000000000001",
    );
  });
  test("a part the server does not know is never sent as a marker", () => {
    for (const unknown of ["everything", "Parties", "", "all&x=1", "audio/1", "video"]) {
      expect(() => format.revealPath("/vcons/x", unknown), unknown).toThrow(RangeError);
    }
  });
  test("a path that already has a query or a fragment is never given a marker", () => {
    for (const path of ["/vcons/x?limit=1", "/vcons/x?", "/vcons/x#top", "/vcons/x?reveal=all"]) {
      expect(() => format.revealPath(path, "all"), path).toThrow(RangeError);
    }
  });
  test("the parts the console can name are the parts the server's description lists", () => {
    const description = JSON.parse(
      readFileSync(new URL("../../crates/vcon-server/openapi.json", import.meta.url), "utf8"),
    );
    expect(format.REVEAL_PARTS).toEqual(description.components.parameters.reveal.schema.enum);
  });
  test("the list of parts cannot be changed by the page that reads it", () => {
    expect(() => format.REVEAL_PARTS.push("everything")).toThrow(TypeError);
    expect(format.REVEAL_PARTS).toHaveLength(7);
  });
});

/** Bytes that start as a WAV file does: RIFF, a length, WAVE. */
function wav(extra = 8) {
  return Uint8Array.from([...Buffer.from("RIFF"), 36, 0, 0, 0, ...Buffer.from("WAVEfmt "), ...new Array(extra).fill(0)]);
}

/** Bytes that start as an MP3 with an ID3 tag does. */
function mp3() {
  return Uint8Array.from([...Buffer.from("ID3"), 4, 0, 0, 0, 0, 0, 0, 0xff, 0xfb, 0x90, 0x00]);
}

/** Bytes that start as an Ogg stream does, with `head` as its first packet. */
function ogg(head = "\x01vorbis") {
  const page = new Uint8Array(28 + head.length);
  page.set(Buffer.from("OggS"), 0);
  page.set(Buffer.from(head, "latin1"), 28);
  return page;
}

/** An Ogg stream that holds Opus. */
function opus() {
  return ogg("OpusHead");
}

/** A vCon, as text, whose dialogs are these. */
function withDialogs(...dialog) {
  return JSON.stringify({ uuid: "0199e5c6-0d7a-7c11-8000-000000000001", parties: [], dialog });
}

/** A recording dialog that holds `bytes` inline, declared as `mediatype`. */
function inline(bytes, mediatype = "audio/wav", encoding = "base64url") {
  const body = Buffer.from(bytes).toString(encoding === "base64" ? "base64" : "base64url");
  return { type: "recording", start: "2026-10-23T12:00:00Z", parties: [0], mediatype, encoding, body };
}

test.describe("recordings", () => {
  test("bytes that start as a WAV file does are audio/wav", () => {
    expect(format.audioSignature(wav())).toBe("audio/wav");
  });
  test("bytes that are markup, text or nothing are the signature of no audio", () => {
    for (const text of ["<svg xmlns='http://www.w3.org/2000/svg'><script>1</script></svg>", "<!doctype html><html>", "{}", "hello there, this is text", "RIFF", ""]) {
      expect(format.audioSignature(Uint8Array.from(Buffer.from(text))), text).toBeNull();
    }
  });
  test("bytes that start with an ID3 tag or an MPEG audio frame are audio/mpeg", () => {
    expect(format.audioSignature(mp3())).toBe("audio/mpeg");
    expect(format.audioSignature(Uint8Array.from([0xff, 0xfb, 0x90, 0x00, 0, 0]))).toBe("audio/mpeg");
    expect(format.audioSignature(Uint8Array.from([0xff, 0xf3, 0x48, 0xc4, 0, 0]))).toBe("audio/mpeg");
  });
  test("bytes that only look like the start of an MPEG frame are not audio", () => {
    const notFrames = {
      "a reserved layer": [0xff, 0xe1, 0x90, 0x00],
      "a reserved version": [0xff, 0xea, 0x90, 0x00],
      "a bit rate that does not exist": [0xff, 0xfb, 0xf0, 0x00],
      "a sample rate that does not exist": [0xff, 0xfb, 0x9c, 0x00],
      "UTF-16 text": [0xff, 0xfe, 0x3c, 0x00, 0x73, 0x00],
      "two bytes": [0xff, 0xfb],
    };
    for (const [what, bytes] of Object.entries(notFrames)) {
      expect(format.audioSignature(Uint8Array.from(bytes)), what).toBeNull();
    }
  });
  test("bytes that start as an Ogg stream does are audio/ogg", () => {
    expect(format.audioSignature(ogg())).toBe("audio/ogg");
  });
  test("an Ogg stream whose first packet is an Opus header is audio/opus", () => {
    expect(format.audioSignature(opus())).toBe("audio/opus");
  });
  test("a dialog's inline WAV recording is decoded to its bytes and its type", () => {
    const got = format.recording(withDialogs(inline(wav())), 0);
    expect(got.ok).toBe(true);
    expect(got.type).toBe("audio/wav");
    expect(Array.from(got.bytes)).toEqual(Array.from(wav()));
  });
  test("a recording declared as a type the console does not play is refused, and the reason names the type", () => {
    for (const type of ["image/svg+xml", "text/html", "video/mp4", "audio/aac", "audio/flac", "application/octet-stream"]) {
      expect(format.recording(withDialogs(inline(wav(), type)), 0), type).toEqual({
        ok: false,
        reason:
          `Dialog 1's recording is declared as ${type}, which the console does not play. ` +
          "It plays audio/wav, audio/mpeg, audio/ogg and audio/opus. Download the vCon to use the recording another way.",
      });
    }
  });
  test("a recording whose bytes are not what it is declared as is refused", () => {
    const svg = Buffer.from("<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>");
    const html = Buffer.from("<!doctype html><script>alert(1)</script>");
    for (const [bytes, type] of [[svg, "audio/wav"], [html, "audio/mpeg"], [mp3(), "audio/wav"], [wav(), "audio/ogg"], [ogg(), "audio/mpeg"]]) {
      expect(format.recording(withDialogs(inline(bytes, type)), 0), type).toEqual({
        ok: false,
        reason:
          `Dialog 1's recording is declared as ${type}, but its bytes are not ${type}. ` +
          "The console does not play it. Download the vCon to examine the recording.",
      });
    }
  });
  test("Opus in an Ogg stream plays when it is declared as audio/ogg or as audio/opus", () => {
    expect(format.recording(withDialogs(inline(opus(), "audio/ogg")), 0).ok).toBe(true);
    expect(format.recording(withDialogs(inline(opus(), "audio/opus")), 0).ok).toBe(true);
    expect(format.recording(withDialogs(inline(ogg(), "audio/opus")), 0).ok).toBe(false);
  });
  test("a recording kept at a link outside the server is never played", () => {
    const linked = { type: "recording", mediatype: "audio/wav", url: "https://media.example.com/a.wav" };
    expect(format.recording(withDialogs(linked), 0)).toEqual({
      ok: false,
      reason: "Dialog 1's recording is at a link outside the server, which the console never loads.",
    });
  });
  test("a dialog with nothing inside it has no recording to play", () => {
    for (const dialog of [{ type: "recording", mediatype: "audio/wav" }, { type: "recording", mediatype: "audio/wav", encoding: "base64url", body: "" }, { type: "incomplete" }]) {
      expect(format.recording(withDialogs(dialog), 0)).toEqual({
        ok: false,
        reason: "Dialog 1 has no recording inside the vCon.",
      });
    }
  });
  test("a body that is not encoded as base64url or base64 is not a recording", () => {
    for (const encoding of ["none", "json", "hex", undefined, 7]) {
      const dialog = { ...inline(wav()), encoding };
      expect(format.recording(withDialogs(dialog), 0), String(encoding)).toEqual({
        ok: false,
        reason: "Dialog 1 holds text or data, not an encoded recording.",
      });
    }
  });
  test("a body that is not base64 cannot be decoded, and says so", () => {
    for (const body of ["****", "UklGR", "UklG RiQA", "UklGR=iQ"]) {
      const dialog = { ...inline(wav()), body };
      expect(format.recording(withDialogs(dialog), 0), body).toEqual({
        ok: false,
        reason: "Dialog 1's recording cannot be decoded: its body is not base64. Download the vCon to examine it.",
      });
    }
  });
  test("a recording larger than the limit is refused before it is decoded, with both sizes", () => {
    const big = inline(wav(100));
    const size = wav(100).length;
    expect(format.recording(withDialogs(big), 0, size - 1)).toEqual({
      ok: false,
      reason:
        `Dialog 1's recording is about ${size} bytes, and the console plays at most ${size - 1}. ` +
        "Download the vCon to hear it another way.",
    });
  });
  test("a recording of exactly the limit is played, and the limit is 100 MiB", () => {
    const size = wav(100).length;
    expect(format.recording(withDialogs(inline(wav(100))), 0, size).ok).toBe(true);
    expect(format.recording(withDialogs(inline(wav(100), "audio/wav", "base64")), 0, size).ok).toBe(true);
    expect(format.AUDIO_LIMIT).toBe(104857600);
  });
  test("a dialog the vCon does not have has no recording", () => {
    const text = withDialogs(inline(wav()), "not an object");
    for (const index of [2, -1, 0.5, "0", null, undefined, Number.NaN, 1]) {
      expect(format.recording(text, index), String(index)).toEqual({
        ok: false,
        reason: "The vCon has no such dialog to play.",
      });
    }
  });
  test("a vCon that cannot be read in parts has no recording the console can play", () => {
    const sealed = JSON.stringify({ protected: "eA", ciphertext: "Y2lwaGVy", iv: "aXY", tag: "dGFn" });
    for (const text of [sealed, "not json", "[]", "null"]) {
      expect(format.recording(text, 0), text).toEqual({
        ok: false,
        reason: "This vCon cannot be read in parts, so the console cannot play from it.",
      });
    }
  });
  test("a declared type is read without its case or its parameters", () => {
    for (const declared of ["Audio/WAV", "audio/wav; rate=8000", " audio/wav ", "AUDIO/WAV;codecs=1"]) {
      const got = format.recording(withDialogs(inline(wav(), declared)), 0);
      expect(got.ok, declared).toBe(true);
      expect(got.type, declared).toBe("audio/wav");
    }
  });
  test("a recording that does not say what type it is is refused", () => {
    for (const mediatype of [undefined, null, 7, "", " ", ";", {}]) {
      const dialog = { ...inline(wav()), mediatype };
      expect(format.recording(withDialogs(dialog), 0), String(mediatype)).toEqual({
        ok: false,
        reason: "Dialog 1's recording does not say what type it is, so the console does not play it. Download the vCon to examine it.",
      });
    }
  });
  test("a dialog that names its type the older way, as mimetype, is read the same", () => {
    const { mediatype, ...rest } = inline(mp3(), "audio/mpeg");
    const got = format.recording(withDialogs({ ...rest, mimetype: mediatype }), 0);
    expect(got.ok).toBe(true);
    expect(got.type).toBe("audio/mpeg");
    // When both are there, the current name is the one read.
    const both = format.recording(withDialogs({ ...rest, mimetype: "audio/mpeg", mediatype: "video/mp4" }), 0);
    expect(both.ok).toBe(false);
  });
  test("a long declared type is cut in the reason, and is text whatever it holds", () => {
    const hostile = `audio/<img src=x onerror=alert(1)>${"a".repeat(200)}`;
    const got = format.recording(withDialogs(inline(wav(), hostile)), 0);
    expect(got.ok).toBe(false);
    expect(got.reason).toContain(`is declared as ${hostile.slice(0, 80)}…, which the console does not play.`);
    expect(got.reason).not.toContain("a".repeat(100));
  });
  test("the dialogs that hold a recording the console plays are listed by place and type", () => {
    const text = withDialogs(inline(wav()), { type: "text", body: "hello", encoding: "none" }, inline(opus(), "audio/opus"));
    expect(format.recordings(text)).toEqual([
      { index: 0, type: "audio/wav" },
      { index: 2, type: "audio/opus" },
    ]);
  });
  test("a vCon that cannot be read, or has no dialog list, lists no recording", () => {
    const sealed = JSON.stringify({ protected: "eA", ciphertext: "Y2lwaGVy", iv: "aXY", tag: "dGFn" });
    for (const text of [sealed, "not json", "{}", JSON.stringify({ dialog: "x" }), JSON.stringify({ dialog: [null, 7, "x", []] })]) {
      expect(format.recordings(text), text).toEqual([]);
    }
  });
  test("a dialog the console would refuse to play is not listed as a recording", () => {
    const text = withDialogs(
      { type: "recording", mediatype: "audio/wav", url: "https://media.example.com/a.wav" },
      inline(wav(), "image/svg+xml"),
      inline(wav(), "video/mp4"),
      { ...inline(wav()), encoding: "none" },
      { ...inline(wav()), mediatype: undefined },
      inline(wav(200)),
      inline(mp3(), "audio/mpeg"),
    );
    expect(format.recordings(text, 100)).toEqual([{ index: 6, type: "audio/mpeg" }]);
  });
});

/** The message the console hands the player: a recording, its type and a line naming it. */
function handOver(bytes = wav(), type = "audio/wav", label = "Dialog 1 of 0199e5c6-0d7a-7c11-8000-000000000001") {
  return { kind: "recording", type, label, bytes: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) };
}

test.describe("what the player is handed", () => {
  test("a recording with its type and a line naming it is taken", () => {
    const got = format.handed(handOver());
    expect(got.type).toBe("audio/wav");
    expect(got.label).toBe("Dialog 1 of 0199e5c6-0d7a-7c11-8000-000000000001");
    expect(Array.from(new Uint8Array(got.bytes))).toEqual(Array.from(wav()));
  });
  test("a message that is not an object is not taken", () => {
    for (const data of [null, undefined, "recording", 7, true, [], [handOver()], () => handOver()]) {
      expect(format.handed(data), String(data)).toBeNull();
    }
  });
  test("a message of another kind is not taken", () => {
    for (const kind of [undefined, "", "player-ready", "Recording", "recording ", 1]) {
      expect(format.handed({ ...handOver(), kind }), String(kind)).toBeNull();
    }
  });
  test("bytes that are not one buffer of bytes are not taken", () => {
    for (const bytes of [wav(), Array.from(wav()), "UklGRg", undefined, null, { byteLength: 20 }, new Blob([wav()])]) {
      expect(format.handed({ ...handOver(), bytes }), Object.prototype.toString.call(bytes)).toBeNull();
    }
  });
  test("a recording with no bytes, or more than the limit, is not taken", () => {
    expect(format.handed({ ...handOver(), bytes: new ArrayBuffer(0) })).toBeNull();
    expect(format.handed(handOver(wav(100)), wav(100).length - 1)).toBeNull();
    expect(format.handed(handOver(wav(100)), wav(100).length)).not.toBeNull();
  });
  test("a recording of a type the console does not play is not taken", () => {
    for (const type of ["image/svg+xml", "text/html", "video/mp4", "audio/x-wav", "AUDIO/WAV", "audio/wav; x=1", "", undefined, 7]) {
      expect(format.handed({ ...handOver(), type }), String(type)).toBeNull();
    }
  });
  test("bytes that are not the type they are handed over as are not taken", () => {
    const svg = Uint8Array.from(Buffer.from("<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>"));
    const html = Uint8Array.from(Buffer.from("<!doctype html><script>alert(1)</script>"));
    expect(format.handed(handOver(svg, "audio/wav"))).toBeNull();
    expect(format.handed(handOver(html, "audio/mpeg"))).toBeNull();
    expect(format.handed(handOver(mp3(), "audio/wav"))).toBeNull();
    expect(format.handed(handOver(mp3(), "audio/mpeg"))).not.toBeNull();
  });
  test("a line naming the recording must be text of 1 to 200 characters, and is kept as it is", () => {
    for (const label of [undefined, null, 7, "", "x".repeat(201), ["Dialog 1"]]) {
      expect(format.handed({ ...handOver(), label }), String(label).slice(0, 20)).toBeNull();
    }
    expect(format.handed({ ...handOver(), label: "x".repeat(200) })).not.toBeNull();
    const markup = "<img src=x onerror=alert(1)>";
    expect(format.handed({ ...handOver(), label: markup }).label).toBe(markup);
  });
  test("what the console hands over for a recording is what the player takes", () => {
    const found = format.recording(withDialogs(inline(mp3(), "audio/mpeg"), inline(wav())), 1);
    const message = format.handOver(found, "0199e5c6-0d7a-7c11-8000-000000000001", 1);
    expect(Object.keys(message).sort()).toEqual(["bytes", "kind", "label", "type"]);
    const taken = format.handed(message);
    expect(taken.type).toBe("audio/wav");
    expect(taken.label).toBe("Dialog 2 of 0199e5c6-0d7a-7c11-8000-000000000001, audio/wav");
    expect(Array.from(new Uint8Array(taken.bytes))).toEqual(Array.from(wav()));
  });
});

test.describe("whose message is taken", () => {
  const origin = "https://vcon.example.com";
  const console_ = { name: "the console's window" };

  test("a message from the expected window on this origin is taken", () => {
    expect(format.accepts({ origin, source: console_ }, console_, origin)).toBe(true);
  });
  test("a message from another origin is not taken, however close its name", () => {
    for (const other of ["https://vcon.example.com:8443", "http://vcon.example.com", "https://vcon.example.com.evil.example", "https://evil.example", "null", ""]) {
      expect(format.accepts({ origin: other, source: console_ }, console_, origin), other).toBe(false);
    }
  });
  test("a message from another window on this origin is not taken", () => {
    expect(format.accepts({ origin, source: { name: "another tab" } }, console_, origin)).toBe(false);
    expect(format.accepts({ origin, source: null }, console_, origin)).toBe(false);
  });
  test("a page with no window to hear from takes no message, one with no source too", () => {
    expect(format.accepts({ origin, source: null }, null, origin)).toBe(false);
    expect(format.accepts({ origin, source: undefined }, undefined, origin)).toBe(false);
  });
});

test.describe("other names for the audio types", () => {
  /** Each other name the console knows, what it is a name for, and bytes of that type. */
  const ALIASES = [
    ["audio/x-wav", "audio/wav", wav],
    ["audio/wave", "audio/wav", wav],
    ["audio/vnd.wave", "audio/wav", wav],
    ["audio/x-mp3", "audio/mpeg", mp3],
    ["audio/mp3", "audio/mpeg", mp3],
    ["audio/x-ogg", "audio/ogg", ogg],
  ];

  for (const [alias, type, bytes] of ALIASES) {
    test(`a recording declared as ${alias} plays as ${type}`, () => {
      const got = format.recording(withDialogs(inline(bytes(), alias)), 0);
      expect(got.ok).toBe(true);
      expect(got.type).toBe(type);
      expect(Array.from(got.bytes)).toEqual(Array.from(bytes()));
    });
  }
  test("a recording under another name whose bytes are not that type is refused, and the reason gives both names", () => {
    const svg = Uint8Array.from(Buffer.from("<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>"));
    for (const [alias, type, bytes] of [["audio/x-wav", "audio/wav", mp3()], ["audio/x-mp3", "audio/mpeg", svg], ["audio/mp3", "audio/mpeg", wav()], ["audio/x-ogg", "audio/ogg", mp3()], ["audio/vnd.wave", "audio/wav", svg]]) {
      expect(format.recording(withDialogs(inline(bytes, alias)), 0), alias).toEqual({
        ok: false,
        reason:
          `Dialog 1's recording is declared as ${alias}, but its bytes are not ${type}. ` +
          "The console does not play it. Download the vCon to examine the recording.",
      });
    }
  });
  test("a name the console does not know is refused, an x- name too", () => {
    for (const type of ["audio/x-aiff", "audio/x-ms-wma", "audio/x-flac", "audio/basic", "audio/x-wave", "audio/x-mpeg", "audio/mpeg3", "application/ogg", "video/ogg", "audio/x-pn-realaudio"]) {
      const got = format.recording(withDialogs(inline(wav(), type)), 0);
      expect(got.ok, type).toBe(false);
      expect(got.reason, type).toContain(`is declared as ${type}, which the console does not play.`);
      expect(format.recordings(withDialogs(inline(wav(), type))), type).toEqual([]);
    }
  });
  test("the table of other names maps each to a type the console plays, and names none of those types", () => {
    const table = Object.entries(format.AUDIO_ALIASES);
    expect(table.map(([alias]) => alias).sort()).toEqual(ALIASES.map(([alias]) => alias).sort());
    for (const [alias, type] of table) {
      expect(format.AUDIO_TYPES, alias).toContain(type);
      expect(format.AUDIO_TYPES, alias).not.toContain(alias);
      expect(alias, alias).toBe(alias.toLowerCase());
    }
    expect(Object.isFrozen(format.AUDIO_ALIASES)).toBe(true);
  });
  test("a declared type that is the name of something every object has is not another name for audio", () => {
    for (const type of ["constructor", "tostring", "__proto__", "hasownproperty", "valueof"]) {
      const got = format.recording(withDialogs(inline(wav(), type)), 0);
      expect(got.ok, type).toBe(false);
      expect(got.reason, type).toContain(`is declared as ${type}, which the console does not play.`);
    }
  });
  test("another name is read without its case, and with its parameters set aside", () => {
    for (const declared of ["Audio/X-WAV; codecs=1", "AUDIO/X-WAV", "audio/x-wav;rate=8000", " audio/x-wav ; a=b ; c=d", 'audio/x-wav; note="a;b"', "audio/X-Wav\t;x=1"]) {
      const got = format.recording(withDialogs(inline(wav(), declared)), 0);
      expect(got.ok, declared).toBe(true);
      expect(got.type, declared).toBe("audio/wav");
    }
  });
  test("a declared type that is not written as a media type is refused", () => {
    for (const declared of ["audio/x-wav extra", "audio / x-wav", "audio/x-wav/x", "/x-wav", "audio/", "x-wav", "audio/x-wav,audio/wav", '"audio/x-wav"']) {
      expect(format.recording(withDialogs(inline(wav(), declared)), 0).ok, declared).toBe(false);
    }
  });
  test("the IETF example's recording dialog, with the recording inside it, plays as audio/mpeg", () => {
    const example = JSON.parse(readFileSync(new URL("../../crates/vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon", import.meta.url), "utf8"));
    expect(example.dialog[0].mediatype).toBe("audio/x-mp3");
    const { url, content_hash: hash, ...dialog } = example.dialog[0];
    expect(typeof url).toBe("string");
    example.dialog[0] = { ...dialog, encoding: "base64url", body: Buffer.from(mp3()).toString("base64url") };
    const text = JSON.stringify(example);
    expect(format.recordings(text)).toEqual([{ index: 0, type: "audio/mpeg" }]);
    const got = format.recording(text, 0);
    expect(got.ok).toBe(true);
    expect(got.type).toBe("audio/mpeg");
  });
  test("the IETF example as published keeps its recording at a link, which is never played", () => {
    const text = readFileSync(new URL("../../crates/vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon", import.meta.url), "utf8");
    expect(format.recordings(text)).toEqual([]);
    expect(format.recording(text, 0)).toEqual({
      ok: false,
      reason: "Dialog 1's recording is at a link outside the server, which the console never loads.",
    });
  });
  test("what is handed to the player names the type played, and the player takes no other name", () => {
    const found = format.recording(withDialogs(inline(mp3(), "audio/x-mp3")), 0);
    const message = format.handOver(found, "0199e5c6-0d7a-7c11-8000-000000000001", 0);
    expect(message.type).toBe("audio/mpeg");
    expect(message.label).toBe("Dialog 1 of 0199e5c6-0d7a-7c11-8000-000000000001, audio/mpeg");
    expect(format.handed(message)).not.toBeNull();
    expect(format.handed({ ...message, type: "audio/x-mp3" })).toBeNull();
    expect(format.handed({ ...message, type: "audio/mp3" })).toBeNull();
  });
});
