// What the console shows, worked out from what the server sent: times,
// the server's masked view of a vCon, the parts a Reveal shows, a
// recording to play and what a refusal means. The masked view is the
// server's (src/masked.rs); the console makes no summary of its own.
// Nothing here touches the page or the network, so each function is
// tested alone.

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

/** Whether a value is a JSON object, not an array or null. */
function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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

/** Whether a value is a place in a list, counted from zero. */
function isIndex(value) {
  return Number.isInteger(value) && value >= 0;
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

/** The fields of a masked view that are text. */
const VIEW_TEXTS = ["form", "created", "subject"];

/** The fields of a masked view that are counts. */
const VIEW_COUNTS = ["dialogs", "attachments", "analyses", "links"];

/** The fields of a masked view that are lists of lines. */
const VIEW_LINES = ["parties", "dialog"];

/** The fields of a signature's header a masked view names. */
const VIEW_SIGNATURE = ["alg", "kid", "x5t"];

/**
 * The server's masked view of a vCon (`?view=masked`, made by
 * src/masked.rs), read from the text of its answer and checked to hold
 * every field the console shows, or null when it does not. An older server
 * sends no `recordings`; that is read as none. The console makes no
 * summary of its own: the server holds the one rule.
 */
export function maskedView(text) {
  let view;
  try {
    view = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObject(view)) {
    return null;
  }
  const isText = (value) => typeof value === "string";
  const recordings = "recordings" in view ? view.recordings : [];
  const signature = view.signature;
  const whole =
    VIEW_TEXTS.every((name) => isText(view[name])) &&
    VIEW_COUNTS.every((name) => Number.isInteger(view[name]) && view[name] >= 0) &&
    typeof view.seconds === "number" &&
    VIEW_LINES.every((name) => Array.isArray(view[name]) && view[name].every(isText)) &&
    (signature === null || (isObject(signature) && VIEW_SIGNATURE.every((name) => isText(signature[name])))) &&
    Array.isArray(recordings) &&
    recordings.every((r) => isObject(r) && Number.isInteger(r.dialog) && r.dialog >= 1 && isText(r.type));
  if (!whole) {
    return null;
  }
  return {
    form: view.form,
    created: view.created,
    subject: view.subject,
    parties: view.parties,
    dialogs: view.dialogs,
    dialog: view.dialog,
    seconds: view.seconds,
    attachments: view.attachments,
    analyses: view.analyses,
    links: view.links,
    signature: signature === null ? null : { alg: signature.alg, kid: signature.kid, x5t: signature.x5t },
    recordings: recordings.map((r) => ({ dialog: r.dialog, type: r.type })),
  };
}

/** The end of the problem type a listener answers a key of the other kind with. */
const WRONG_LISTENER = "/problems/wrong-listener";

/** What to do about a refusal, by its problem type when that says more, else its status. */
function advice(status, type) {
  if (typeof type === "string" && type.endsWith(WRONG_LISTENER)) {
    return (
      "Open the console at the server's other address and sign in there: " +
      "one address serves tenant keys and the other serves operator keys."
    );
  }
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

/**
 * A refusal as text: what happened, its status and what to do. `type` is
 * the problem document's type, when the answer had one.
 */
export function refusalText(status, title, type = undefined) {
  if (status === 0) {
    return "The server could not be reached. Check the connection and try again.";
  }
  const what = typeof title === "string" && title !== "" ? `${title} (${status})` : `HTTP ${status}`;
  return `${what}. ${advice(status, type)}`;
}

/** The parts a reveal can name, as the server's `reveal` parameter lists them. */
export const REVEAL_PARTS = Object.freeze(["parties", "subject", "dialog", "analysis", "links", "all", "audio"]);

/**
 * The path a reveal of `part` reads: `path` with the marker the server
 * records the read by. A null part is a read that reveals nothing, and has
 * no marker. A part the server does not list is never sent.
 */
export function revealPath(path, part) {
  if (part === null) {
    return path;
  }
  if (!REVEAL_PARTS.includes(part)) {
    throw new RangeError(`not a part a reveal can name: ${part}`);
  }
  if (path.includes("?") || path.includes("#")) {
    throw new RangeError("a reveal is the only parameter of its read");
  }
  return `${path}?reveal=${part}`;
}

/**
 * The path of a read asking for the server's masked view of the vCon at
 * `path`. The view is the only parameter of its read.
 */
export function maskedPath(path) {
  if (path.includes("?") || path.includes("#")) {
    throw new RangeError("the masked view is the only parameter of its read");
  }
  return `${path}?view=masked`;
}

/** What the console says when the server's masked view cannot be read. */
export const VIEW_UNREADABLE =
  "The server's masked view of this vCon could not be read. Open it again; if this repeats, tell the server's operator.";

/**
 * The audio type a recording's first bytes are the signature of, or null
 * when they are the signature of none the console plays.
 */
export function audioSignature(bytes) {
  if (startsWith(bytes, "RIFF") && startsWith(bytes, "WAVE", 8)) {
    return "audio/wav";
  }
  if (startsWith(bytes, "ID3") || isMpegFrame(bytes)) {
    return "audio/mpeg";
  }
  if (startsWith(bytes, "OggS")) {
    // The first page's header is 27 bytes and one segment length; the
    // first packet, which names the codec, follows.
    return startsWith(bytes, "OpusHead", 28) ? "audio/opus" : "audio/ogg";
  }
  return null;
}

/**
 * Whether `bytes` starts with the header of an MPEG audio frame: the
 * eleven sync bits, then a version, a layer, a bit rate and a sample rate
 * that exist.
 */
function isMpegFrame(bytes) {
  return (
    bytes.length >= 4 &&
    bytes[0] === 0xff &&
    (bytes[1] & 0xe0) === 0xe0 &&
    (bytes[1] & 0x18) !== 0x08 &&
    (bytes[1] & 0x06) !== 0x00 &&
    (bytes[2] & 0xf0) !== 0xf0 &&
    (bytes[2] & 0x0c) !== 0x0c
  );
}

/** Whether `bytes` holds the ASCII `text` at `offset`. */
function startsWith(bytes, text, offset = 0) {
  if (bytes.length < offset + text.length) {
    return false;
  }
  for (let i = 0; i < text.length; i++) {
    if (bytes[offset + i] !== text.charCodeAt(i)) {
      return false;
    }
  }
  return true;
}

/** The characters of base64 and of base64url, with any padding last. */
const BASE64 = /^(?:[A-Za-z0-9+/]*|[A-Za-z0-9_-]*)={0,2}$/;

/** A base64 or base64url body as bytes, or null when it is neither. */
function bytesOf(body) {
  if (!BASE64.test(body)) {
    return null;
  }
  let text;
  try {
    text = atob(body.replaceAll("-", "+").replaceAll("_", "/"));
  } catch {
    return null;
  }
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) {
    bytes[i] = text.charCodeAt(i);
  }
  return bytes;
}

/**
 * What a dialog says it holds, read without decoding it: `{ ok: true,
 * type, declared }` for an inline recording of a type and size the
 * console plays, with the type it plays as and the name it was declared
 * under, or `{ ok: false, reason }`.
 */
function offered(dialog, index, limit) {
  const what = `Dialog ${index + 1}'s recording`;
  if (typeof dialog.body !== "string" || dialog.body === "") {
    if (typeof dialog.url === "string" && dialog.url !== "") {
      return refused(`${what} is at a link outside the server, which the console never loads.`);
    }
    return refused(`Dialog ${index + 1} has no recording inside the vCon.`);
  }
  if (!MEDIA_ENCODINGS.includes(dialog.encoding)) {
    return refused(`Dialog ${index + 1} holds text or data, not an encoded recording.`);
  }
  const type = declaredType(dialog);
  if (type === "") {
    return refused(`${what} does not say what type it is, so the console does not play it. Download the vCon to examine it.`);
  }
  let played;
  if (AUDIO_TYPES.includes(type)) {
    played = type;
  } else if (Object.hasOwn(AUDIO_ALIASES, type)) {
    played = AUDIO_ALIASES[type];
  }
  if (played === undefined) {
    const named = type.length > TYPE_SHOWN ? `${type.slice(0, TYPE_SHOWN)}…` : type;
    return refused(
      `${what} is declared as ${named}, which the console does not play. ` +
        `It plays ${AUDIO_TYPES.slice(0, -1).join(", ")} and ${AUDIO_TYPES.at(-1)}. ` +
        "Download the vCon to use the recording another way.",
    );
  }
  const size = Math.floor((dialog.body.replace(/=+$/, "").length * 3) / 4);
  if (size > limit) {
    return refused(
      `${what} is about ${size} bytes, and the console plays at most ${limit}. ` +
        "Download the vCon to hear it another way.",
    );
  }
  return { ok: true, type: played, declared: type };
}

/**
 * The recording inside dialog `index` of a vCon body, to hand to the
 * player: `{ ok: true, type, bytes }`, or `{ ok: false, reason }` with
 * what is wrong and what to do. Its bytes must carry the signature of the
 * type it is declared as.
 */
export function recording(text, index, limit = AUDIO_LIMIT) {
  const vcon = readable(text);
  if (vcon === null) {
    return refused("This vCon cannot be read in parts, so the console cannot play from it.");
  }
  const dialog = isIndex(index) ? items(vcon.dialog)[index] : undefined;
  if (!isObject(dialog)) {
    return refused("The vCon has no such dialog to play.");
  }
  const offer = offered(dialog, index, limit);
  if (!offer.ok) {
    return offer;
  }
  const what = `Dialog ${index + 1}'s recording`;
  const bytes = bytesOf(dialog.body);
  if (bytes === null) {
    return refused(`${what} cannot be decoded: its body is not base64. Download the vCon to examine it.`);
  }
  if (!isDeclared(bytes, offer.type)) {
    return refused(
      `${what} is declared as ${offer.declared}, but its bytes are not ${offer.type}. ` +
        "The console does not play it. Download the vCon to examine the recording.",
    );
  }
  return { ok: true, type: offer.type, bytes };
}

/**
 * Whether `bytes` carries the signature of the type it is declared as. An
 * Ogg stream that holds Opus is also what `audio/ogg` declares.
 */
export function isDeclared(bytes, type) {
  const found = audioSignature(bytes);
  return found !== null && (found === type || (found === "audio/opus" && type === "audio/ogg"));
}

/**
 * The media type a dialog declares, without its parameters, in lower
 * case: its `mediatype`, or `mimetype`, the name older vCons use, when it
 * has no `mediatype`. Empty when it declares none.
 */
function declaredType(dialog) {
  const declared = "mediatype" in dialog ? dialog.mediatype : dialog.mimetype;
  return typeof declared === "string" ? declared.split(";")[0].trim().toLowerCase() : "";
}

/** The most characters of a declared type a refusal shows. */
const TYPE_SHOWN = 80;

/** A recording the console will not play, and why. */
function refused(reason) {
  return { ok: false, reason };
}

/**
 * The most bytes of one recording the console plays: 100 MiB, the server's
 * default limit on a whole vCon once inflated.
 */
export const AUDIO_LIMIT = 100 * 1024 * 1024;

/**
 * Other names producers give the types the console plays, each with the
 * type it is a name for. A recording declared under one is checked and
 * played as that type; a name not here is not played.
 */
export const AUDIO_ALIASES = Object.freeze({
  "audio/x-wav": "audio/wav",
  "audio/wave": "audio/wav",
  "audio/vnd.wave": "audio/wav",
  "audio/x-mp3": "audio/mpeg",
  "audio/mp3": "audio/mpeg",
  "audio/x-ogg": "audio/ogg",
});

/** The audio types the console plays, and no other. */
export const AUDIO_TYPES = Object.freeze(["audio/wav", "audio/mpeg", "audio/ogg", "audio/opus"]);

/** The most characters of the line that names a recording. */
const LABEL_LIMIT = 200;

/** The kind of message that hands the player a recording. */
export const HAND_OVER = "recording";

/**
 * What the player takes from a message: `{ type, label, bytes }` when the
 * message is one recording as the console hands it over, and null for
 * anything else, which the player ignores.
 */
export function handed(data, limit = AUDIO_LIMIT) {
  if (!isObject(data) || data.kind !== HAND_OVER) {
    return null;
  }
  if (!(data.bytes instanceof ArrayBuffer) || data.bytes.byteLength > limit) {
    return null;
  }
  if (!AUDIO_TYPES.includes(data.type) || !isDeclared(new Uint8Array(data.bytes), data.type)) {
    return null;
  }
  if (typeof data.label !== "string" || data.label === "" || data.label.length > LABEL_LIMIT) {
    return null;
  }
  return { type: data.type, label: data.label, bytes: data.bytes };
}

/**
 * The message that hands the player the recording `found` (what
 * `recording` answered) of dialog `index` of the vCon `uuid`: its bytes as
 * one buffer, its type and the line that names it.
 */
export function handOver(found, uuid, index) {
  const { bytes } = found;
  const whole = bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength;
  return {
    kind: HAND_OVER,
    type: found.type,
    label: `Dialog ${index + 1} of ${uuid}, ${found.type}`,
    bytes: whole ? bytes.buffer : bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
  };
}

/**
 * Whether a message event is one to read: it came from `sender`, the one
 * window this page exchanges messages with, on `origin`, this page's own.
 */
export function accepts(event, sender, origin) {
  return sender !== null && sender !== undefined && event.origin === origin && event.source === sender;
}

/** What the player tells the console: that it can take a recording, and that it closed. */
export const PLAYER_READY = "player-ready";
export const PLAYER_CLOSED = "player-closed";
