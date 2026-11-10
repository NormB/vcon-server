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

/** A summary of nothing, in the given form. */
function empty(form, subject) {
  return {
    form,
    created: "",
    subject,
    parties: [],
    dialogs: 0,
    seconds: 0,
    attachments: 0,
    analyses: 0,
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
    seconds,
    attachments: items(vcon.attachments).length,
    analyses: items(vcon.analysis).length,
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
