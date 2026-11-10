// What the console shows, worked out from what the server sent: times,
// masked party values, a vCon's summary and what a refusal means. Nothing
// here touches the page or the network, so each function is tested alone.

/** A time as `YYYY-MM-DD HH:MM:SS UTC+00:00`; other text as it was sent. */
export function utc(value) {
  if (value === null || value === undefined || value === "") {
    return "";
  }
  if (typeof value !== "string") {
    return String(value);
  }
  const at = new Date(value);
  if (Number.isNaN(at.getTime())) {
    return value;
  }
  const iso = at.toISOString();
  return `${iso.slice(0, 10)} ${iso.slice(11, 19)} UTC+00:00`;
}

/** The fewest digits a number needs before its last four may be shown. */
const MASKABLE_DIGITS = 8;

/** A telephone number as its last four digits, or hidden whole. */
export function maskTel(value) {
  if (typeof value !== "string") {
    return "number hidden";
  }
  const digits = value.replace(/\D/g, "");
  if (digits.length < MASKABLE_DIGITS) {
    return "number hidden";
  }
  return `number ending ${digits.slice(-4)}`;
}

/** An email address as its domain, or hidden whole. */
export function maskMail(value) {
  if (typeof value !== "string") {
    return "address hidden";
  }
  const at = value.lastIndexOf("@");
  const domain = at < 0 ? "" : value.slice(at + 1).trim().toLowerCase();
  if (at < 1 || domain === "") {
    return "address hidden";
  }
  return `address at ${domain}`;
}

/** Whether a value is a JSON object, not an array or null. */
function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** One party as a line that names what it holds and shows none of it. */
export function partyLine(party, index) {
  const label = `Party ${index + 1}`;
  if (!isObject(party)) {
    return `${label}: not an object`;
  }
  const parts = [];
  if ("tel" in party) {
    parts.push(maskTel(party.tel));
  }
  if ("mailto" in party) {
    parts.push(maskMail(party.mailto));
  }
  if ("name" in party) {
    parts.push("name hidden");
  }
  return parts.length === 0 ? `${label}: no number, address or name` : `${label}: ${parts.join("; ")}`;
}

/** Base64url JSON as an object, or null when it is anything else. */
function decoded(text) {
  if (typeof text !== "string" || !/^[A-Za-z0-9_-]+$/.test(text)) {
    return null;
  }
  try {
    const bytes = Uint8Array.from(atob(text.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
    const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
    return isObject(value) ? value : null;
  } catch {
    return null;
  }
}

/** A value's items when it is an array, or none. */
function items(value) {
  return Array.isArray(value) ? value : [];
}

/** The dialog types the vCon format names; any other is not shown. */
const DIALOG_TYPES = ["recording", "text", "transfer", "incomplete"];

/** Whether a value is a party's place in the parties list. */
function isIndex(value) {
  return Number.isInteger(value) && value >= 0;
}

/** A dialog's parties as their numbers, counted from one. */
function partiesOf(parties) {
  if (isIndex(parties)) {
    return `party ${parties + 1}`;
  }
  if (Array.isArray(parties) && parties.length > 0 && parties.every(isIndex)) {
    return `parties ${parties.map((p) => p + 1).join(", ")}`;
  }
  return "no parties";
}

/**
 * One dialog as a line with its type, start, length and parties, and
 * whether it has content, never the content or where a link points.
 */
export function dialogLine(dialog, index) {
  const label = `Dialog ${index + 1}`;
  if (!isObject(dialog)) {
    return `${label}: not an object`;
  }
  const type = DIALOG_TYPES.includes(dialog.type) ? dialog.type : "other type";
  const at = typeof dialog.start === "string" ? utc(dialog.start) : "";
  const start = at.endsWith("UTC+00:00") ? at : "no start";
  const length = typeof dialog.duration === "number" && dialog.duration > 0 ? `${dialog.duration} s` : "no length";
  let content = "no content";
  if (typeof dialog.body === "string" && dialog.body !== "") {
    content = "content hidden";
  } else if (typeof dialog.url === "string" && dialog.url !== "") {
    content = "content at a link outside the server";
  }
  return `${label}: ${type}, ${start}, ${length}, ${partiesOf(dialog.parties)}; ${content}`;
}

/** The parts of a vCon that may hold a link, and what one item is called. */
const LINKED = [
  ["dialog", "dialog"],
  ["attachments", "attachment"],
  ["analysis", "analysis"],
];

/** Every link in a vCon object, with where it is. */
function linksOf(vcon) {
  const found = [];
  for (const [part, noun] of LINKED) {
    items(vcon[part]).forEach((item, index) => {
      if (isObject(item) && typeof item.url === "string" && item.url !== "") {
        found.push({ where: `${noun} ${index + 1}`, url: item.url });
      }
    });
  }
  return found;
}

/**
 * The vCon object a body holds, for reading its parts: the body itself, or
 * a signed vCon's payload. Null for an encrypted vCon and for anything
 * that is not a JSON object.
 */
function readable(text) {
  let vcon;
  try {
    vcon = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObject(vcon) || "ciphertext" in vcon) {
    return null;
  }
  if ("payload" in vcon && "signatures" in vcon) {
    return decoded(vcon.payload);
  }
  return vcon;
}

/** Every link in a vCon body, with where it is; none when it cannot be read. */
export function links(text) {
  const vcon = readable(text);
  return vcon === null ? [] : linksOf(vcon);
}

/** The most characters of a vCon the console shows at once: 2 MiB. */
export const DISPLAY_LIMIT = 2 * 1024 * 1024;

/** Text cut at the limit, and how many characters were left out. */
export function shown(text, limit = DISPLAY_LIMIT) {
  if (text.length <= limit) {
    return { text, left: 0 };
  }
  return { text: text.slice(0, limit), left: text.length - limit };
}

/** The encodings under which a body is media, not text to read. */
const MEDIA_ENCODINGS = ["base64url", "base64"];

/** Items as indented JSON, with inline media named by its size, not shown. */
function listed(value) {
  const made = items(value);
  if (made.length === 0) {
    return "none";
  }
  const readableItems = made.map((item) => {
    if (isObject(item) && MEDIA_ENCODINGS.includes(item.encoding) && typeof item.body === "string") {
      return { ...item, body: `[${item.body.length} characters of ${item.encoding}, not shown]` };
    }
    return item;
  });
  return shown(JSON.stringify(readableItems, null, 2)).text;
}

/** What each part Reveal can show is made from. */
const SECTIONS = new Map([
  ["parties", (vcon) => listed(vcon.parties)],
  ["subject", (vcon) => (typeof vcon.subject === "string" && vcon.subject !== "" ? vcon.subject : "none")],
  ["dialog", (vcon) => listed(vcon.dialog)],
  ["analysis", (vcon) => listed(vcon.analysis)],
]);

/**
 * One part of a vCon body as text: `parties`, `subject`, `dialog` or
 * `analysis`. Null for any other name and for a body that cannot be read
 * in parts.
 */
export function section(text, name) {
  const make = SECTIONS.get(name);
  const vcon = readable(text);
  return make === undefined || vcon === null ? null : make(vcon);
}

/** A summary of nothing, in the given form. */
function empty(form, subject) {
  return {
    form,
    created: "",
    subject,
    parties: [],
    dialogs: 0,
    dialog: [],
    seconds: 0,
    attachments: 0,
    analyses: 0,
    links: 0,
    signature: null,
  };
}

/** The counts and masked parties of an unsigned vCon object. */
function counted(vcon, form) {
  const dialog = items(vcon.dialog);
  let seconds = 0;
  for (const d of dialog) {
    if (isObject(d) && typeof d.duration === "number" && d.duration > 0) {
      seconds += d.duration;
    }
  }
  return {
    form,
    created: typeof vcon.created_at === "string" ? utc(vcon.created_at) : "",
    subject: typeof vcon.subject === "string" && vcon.subject !== "" ? "present, hidden" : "none",
    parties: items(vcon.parties).map(partyLine),
    dialogs: dialog.length,
    dialog: dialog.map(dialogLine),
    seconds,
    attachments: items(vcon.attachments).length,
    analyses: items(vcon.analysis).length,
    links: linksOf(vcon).length,
    signature: null,
  };
}

/** A header field when it is text, or nothing. */
function textField(header, name) {
  return header !== null && typeof header[name] === "string" ? header[name] : "";
}

/**
 * What a vCon body holds, without any name, subject, full number or
 * address: its form, counts and masked parties. A signed vCon's payload is
 * read; an encrypted one is not decoded.
 */
export function summary(text) {
  let vcon;
  try {
    vcon = JSON.parse(text);
  } catch {
    return empty("not JSON", "not readable");
  }
  if (!isObject(vcon)) {
    return empty("not a JSON object", "not readable");
  }
  if ("ciphertext" in vcon) {
    return empty("encrypted (JWE)", "not readable");
  }
  if ("payload" in vcon && "signatures" in vcon) {
    const first = items(vcon.signatures)[0];
    const header = isObject(first) ? decoded(first.protected) : null;
    const payload = decoded(vcon.payload);
    const made = payload === null ? empty("signed (JWS)", "not readable") : counted(payload, "signed (JWS)");
    made.signature = {
      alg: textField(header, "alg"),
      kid: textField(header, "kid"),
      x5t: textField(header, "x5t"),
    };
    return made;
  }
  return counted(vcon, "unsigned");
}

/** What to do about a refusal, by its status. */
function advice(status) {
  if (status === 401) {
    return "Sign in again with a key the server accepts.";
  }
  if (status === 403) {
    return "Sign in with a key that holds the scope this needs.";
  }
  if (status === 404) {
    return "Check the uuid or id: it may be deleted or belong to another tenant.";
  }
  if (status === 410) {
    return "It was deleted: a key with the legal scope can still find it in the Legal view.";
  }
  if (status === 409) {
    return "Reload the view and check its state before trying again.";
  }
  if (status === 412) {
    return "It changed since you opened it: open it again, then repeat the change.";
  }
  if (status === 413) {
    return "Send a smaller file.";
  }
  if (status === 400 || status === 415 || status === 422) {
    return "Correct what was sent and try again.";
  }
  if (status === 429) {
    return "Wait a minute, then try again.";
  }
  if (status >= 500) {
    return "Try again; if it repeats, tell the server's operator.";
  }
  return "Try again.";
}

/** A refusal as text: what happened, its status and what to do. */
export function refusalText(status, title) {
  if (status === 0) {
    return "The server could not be reached. Check the connection and try again.";
  }
  const what = typeof title === "string" && title !== "" ? `${title} (${status})` : `HTTP ${status}`;
  return `${what}. ${advice(status)}`;
}
