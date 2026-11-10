// The vcon-server console: a client of the HTTP API on this origin and
// nothing more. The key the user signs in with lives in `token`, for this
// page only: it is never written to browser storage, a cookie or the URL,
// and a reload forgets it. Every value from the server reaches the page as
// text, never as markup.

import { utc, summary, refusalText } from "./format.js";

let token = null;
let key = null;

/** An element with the given text and attributes. Markup is never parsed. */
function el(tag, text = "", attributes = {}) {
  const made = document.createElement(tag);
  made.textContent = text;
  for (const [name, value] of Object.entries(attributes)) {
    made.setAttribute(name, value);
  }
  return made;
}

/** Empties an element. */
function clear(node) {
  while (node.firstChild) {
    node.removeChild(node.firstChild);
  }
}

/**
 * One API request on this origin, with the signed-in key. The browser keeps
 * no copy of the answer, sends no cookie and follows no redirect. A request
 * that gets no answer resolves to an error response of status 0.
 */
async function request(method, path, body = undefined, headers = {}) {
  const sent = new Headers(headers);
  if (token !== null) {
    sent.set("Authorization", "Bearer " + token);
  }
  try {
    return await fetch(path, {
      method,
      headers: sent,
      body,
      mode: "same-origin",
      cache: "no-store",
      credentials: "omit",
      redirect: "error",
      referrerPolicy: "no-referrer",
    });
  } catch {
    // No answer at all: status 0, which `refusal` reports as unreachable.
    return Response.error();
  }
}

/** A refusal as text: the problem's title, its status and what to do. */
async function refusal(answer) {
  let title;
  try {
    title = (await answer.json()).title;
  } catch {
    title = undefined;
  }
  return refusalText(answer.status, title);
}

function show(id, shown) {
  document.getElementById(id).hidden = !shown;
}

/** The scopes the signed-in key holds. */
function holds(scope) {
  return key !== null && key.scopes.includes(scope);
}

/** How many vCons a page of the list asks for. */
const PAGE = 25;

/** A form with an id, its children and a submit handler. */
function form(id, children, onSubmit) {
  const made = el("form", "", { id });
  made.append(...children);
  made.addEventListener("submit", (event) => {
    event.preventDefault();
    onSubmit(event.submitter);
  });
  return made;
}

/** A text field with its label. */
function field(id, label, attributes = {}) {
  return [
    el("label", label, { for: id }),
    el("input", "", { id, autocomplete: "off", spellcheck: "false", required: "", ...attributes }),
  ];
}

/** A table with these column names and an empty body. */
function table(id, columns) {
  const made = el("table", "", { id });
  const head = el("tr");
  for (const name of columns) {
    head.append(el("th", name === "" ? "actions" : name, { scope: "col" }));
  }
  made.append(el("thead"), el("tbody"));
  made.tHead.append(head);
  return made;
}

/**
 * A table inside a region that scrolls sideways by itself, so a narrow
 * window never scrolls the page, and that the keyboard can reach.
 */
function scrolling(made, label) {
  const region = el("div", "", { class: "scroll", role: "region", "aria-label": label, tabindex: "0" });
  region.append(made);
  return region;
}

/**
 * The line under a list that says it is loading or empty, announced
 * politely. A list that could not be read clears it: its refusal is shown
 * as an alert instead.
 */
function stateOf(made, noun) {
  const line = el("p", "", { id: `${made.id}-state`, class: "state", role: "status" });
  return {
    line,
    loading() {
      line.textContent = `Loading ${noun}…`;
    },
    loaded() {
      line.textContent = made.tBodies[0].rows.length === 0 ? `No ${noun} to show.` : "";
    },
    failed() {
      line.textContent = "";
    },
  };
}

/** What a masked vCon hides, and what Reveal does. */
const MASKED_NOTE =
  "Names, the subject, full numbers and addresses, and what was said are hidden. " +
  "Reveal reads it again, and the audit log records that read.";

/**
 * A vCon shown masked: its version and summary, and its whole text only
 * after Reveal, which reads it again with the function `show` was given.
 * The ids are `<name>-etag`, `<name>-summary`, `<name>-body` and `reveal`.
 */
function maskedVcon(name, reveal, label) {
  const etag = el("p", "", { id: `${name}-etag` });
  const masked = el("dl", "", { id: `${name}-summary` });
  const note = el("p", MASKED_NOTE, { class: "note" });
  const button = el("button", `Reveal ${label}`, { id: reveal, type: "button" });
  const body = el("pre", "", { id: `${name}-body`, hidden: "", tabindex: "0", role: "group", "aria-label": `${label}, as text` });
  let again = null;
  button.addEventListener("click", async () => {
    const got = await again();
    if (got === null) {
      return;
    }
    etag.textContent = got.etag;
    body.textContent = got.text;
    body.hidden = false;
    button.hidden = true;
    body.focus();
  });
  return {
    nodes: [etag, masked, note, button, body],
    /** Shows `got` masked; `read` reads the same vCon again for Reveal. */
    show(got, read) {
      again = read;
      etag.textContent = got.etag;
      body.textContent = "";
      body.hidden = true;
      showSummary(masked, summary(got.text));
      button.hidden = false;
    },
  };
}

/** A vCon answer as its version and text. */
async function versionAndText(answer) {
  return { etag: answer.headers.get("ETag") || "", text: await bodyText(answer) };
}

/** A row of text cells, and any controls in a last cell. */
function row(values, controls = []) {
  const made = el("tr");
  for (const value of values) {
    made.append(el("td", value === null ? "" : String(value)));
  }
  if (controls.length > 0) {
    const cell = el("td");
    cell.append(...controls);
    made.append(cell);
  }
  return made;
}

/** A vCon body as text: inflated first when it was stored gzipped. */
async function bodyText(answer) {
  const type = (answer.headers.get("Content-Type") || "").toLowerCase();
  let stream = answer.body;
  if (type.startsWith("application/vcon+gzip")) {
    stream = stream.pipeThrough(new DecompressionStream("gzip"));
  }
  return new Response(stream).text();
}

/** Fills a list of terms with a vCon's summary: counts and masked parties. */
function showSummary(list, s) {
  clear(list);
  const rows = [
    ["Form", s.form],
    ["Created", s.created === "" ? "not stated" : s.created],
    ["Subject", s.subject],
    ["Dialogs", String(s.dialogs)],
    ["Dialog seconds", String(s.seconds)],
    ["Attachments", String(s.attachments)],
    ["Analyses", String(s.analyses)],
  ];
  if (s.signature !== null) {
    for (const name of ["alg", "kid", "x5t"]) {
      rows.push([`Signature ${name}`, s.signature[name] === "" ? "not stated" : s.signature[name]]);
    }
  }
  for (const [name, value] of rows) {
    list.append(el("dt", name), el("dd", value));
  }
  const parties = el("dd");
  if (s.parties.length === 0) {
    parties.textContent = "none";
  } else {
    const lines = el("ul");
    for (const line of s.parties) {
      lines.append(el("li", line));
    }
    parties.append(lines);
  }
  list.append(el("dt", "Parties"), parties);
}

/** The path of one vCon, under `base`. */
function vconPath(base, uuid, rest = "") {
  return `${base}/${encodeURIComponent(uuid)}${rest}`;
}

/** The vCons view: what the key's read, write and delete scopes allow. */
function vconsView(view) {
  const message = el("p", "", { id: "vcon-message", role: "alert" });
  const shown = el("section", "", { id: "vcon", hidden: "" });
  const title = el("h3");
  const panel = maskedVcon("vcon", "reveal", "the whole vCon");
  shown.append(title, ...panel.nodes);
  let current = null;

  /** Reads the vCon; a refusal is shown and answered with null. */
  async function read(uuid) {
    message.textContent = "";
    const answer = await request("GET", vconPath("/vcons", uuid));
    if (answer.status !== 200) {
      message.textContent = await refusal(answer);
      return null;
    }
    return versionAndText(answer);
  }

  /** Opens a vCon masked: its summary, and none of its content. */
  async function open(uuid) {
    const got = await read(uuid);
    if (got === null) {
      shown.hidden = true;
      return;
    }
    current = uuid;
    title.textContent = uuid;
    panel.show(got, () => read(uuid));
    shown.hidden = false;
  }

  async function remove(uuid) {
    if (!window.confirm(`Delete ${uuid}?`)) {
      return;
    }
    const answer = await request("DELETE", vconPath("/vcons", uuid));
    if (answer.status !== 204) {
      message.textContent = await refusal(answer);
      return;
    }
    message.textContent = `Deleted ${uuid}`;
    if (current === uuid) {
      shown.hidden = true;
      current = null;
    }
    if (list !== null) {
      await list.reload();
    }
  }

  view.append(el("h2", "vCons"));
  if (holds("write")) {
    const result = el("p", "", { id: "store-result", role: "status" });
    const file = el("input", "", { id: "store-file", type: "file", required: "" });
    const store = form(
      "store",
      [el("label", "Store a vCon from a file", { for: "store-file" }), file, el("button", "Store", { type: "submit" })],
      async () => {
        const chosen = file.files[0];
        result.textContent = "Storing…";
        const answer = await request("POST", "/vcons", await chosen.arrayBuffer());
        if (answer.status === 201 || answer.status === 200) {
          const location = answer.headers.get("Location") || "";
          const uuid = decodeURIComponent(location.split("/").pop());
          result.textContent = `Stored ${uuid}, version ${answer.headers.get("ETag")}`;
          if (list !== null) {
            await list.reload();
          }
        } else {
          result.textContent = await refusal(answer);
        }
      },
    );
    view.append(store, result);
  }
  if (holds("read") || holds("delete")) {
    const buttons = [];
    if (holds("read")) {
      buttons.push(el("button", "Open", { id: "open-uuid", type: "submit", value: "open" }));
    }
    if (holds("delete")) {
      buttons.push(el("button", "Delete", { id: "delete-uuid", type: "submit", value: "delete" }));
    }
    const byUuid = form("by-uuid", [...field("uuid", "vCon uuid"), ...buttons], (submitter) => {
      const uuid = document.getElementById("uuid").value.trim();
      return submitter.value === "delete" ? remove(uuid) : open(uuid);
    });
    view.append(byUuid);
  }
  view.append(message);
  let list = null;
  if (holds("read")) {
    list = vconList(view, open);
  }
  if (holds("delete")) {
    const del = el("button", "Delete", { id: "delete", type: "button" });
    del.addEventListener("click", () => remove(current));
    shown.append(del);
  }
  view.append(shown);
  if (list !== null) {
    list.reload();
  }
}

/** The tenant's live vCons, newest first, a page at a time. */
function vconList(view, open) {
  const listed = table("vcon-list", ["uuid", "version", "stored", "updated", ""]);
  const state = stateOf(listed, "vCons");
  const more = el("button", "More vCons", { id: "more", type: "button", hidden: "" });
  const message = el("p", "", { id: "vcon-list-message", role: "alert" });
  let next = null;

  async function page(cursor) {
    const query = new URLSearchParams({ limit: String(PAGE) });
    if (cursor !== null) {
      query.set("cursor", cursor);
    }
    state.loading();
    const answer = await request("GET", `/vcons?${query}`);
    if (answer.status !== 200) {
      state.failed();
      message.textContent = await refusal(answer);
      return;
    }
    const { vcons, next: after } = await answer.json();
    for (const v of vcons) {
      const opener = el("button", "Open", { class: "open", type: "button", "aria-label": `Open ${v.uuid}` });
      opener.addEventListener("click", () => open(v.uuid));
      listed.tBodies[0].append(row([v.uuid, v.version, utc(v.stored_at), utc(v.updated_at)], [opener]));
    }
    next = after;
    more.hidden = next === null;
    state.loaded();
  }

  more.addEventListener("click", () => page(next));
  view.append(el("h3", "Stored vCons"), scrolling(listed, "Stored vCons"), state.line, more, message);
  return {
    async reload() {
      clear(listed.tBodies[0]);
      message.textContent = "";
      await page(null);
    },
  };
}

/** The legal view: a vCon as a legal key sees it, and its versions. */
function legalView(view) {
  const message = el("p", "", { id: "legal-message", role: "alert" });
  const state = el("p", "", { id: "legal-state" });
  const panel = maskedVcon("legal", "legal-reveal", "this version");
  const versions = table("versions", ["version", "stored", "hidden", "current", ""]);
  const versionsState = stateOf(versions, "versions");
  const versionTitle = el("h3", "", { id: "version-title", hidden: "" });
  const versionPanel = maskedVcon("version", "version-reveal", "the opened version");
  const opened = el("section", "", { id: "version", hidden: "" });
  opened.append(versionTitle, ...versionPanel.nodes);
  const found = el("section", "", { id: "legal-vcon", hidden: "" });
  const hold = el("button", "Hold", { id: "hold", type: "button" });
  const release = el("button", "Release hold", { id: "release", type: "button" });
  const restore = el("button", "Restore", { id: "restore", type: "button" });
  let current = null;

  /** One legal action on the vCon found; `done` says what it did. */
  async function act(method, rest, expected, done) {
    const answer = await request(method, vconPath("/legal/vcons", current, rest));
    if (answer.status !== expected) {
      message.textContent = await refusal(answer);
      return;
    }
    const said = done(answer);
    await find(current);
    message.textContent = said;
  }

  hold.addEventListener("click", () => act("PUT", "/hold", 204, () => `Held ${current}`));
  release.addEventListener("click", () => {
    if (confirmed(`Release the hold on ${current}? Retention can then purge it.`)) {
      act("DELETE", "/hold", 204, () => `Released ${current}`);
    }
  });
  restore.addEventListener("click", () =>
    act("POST", "/restore", 200, (a) => `Restored ${current}, version ${a.headers.get("ETag")}`),
  );
  found.append(
    hold,
    release,
    restore,
    el("h3", "Current or last kept version"),
    state,
    ...panel.nodes,
    el("h3", "Versions"),
    scrolling(versions, "Versions"),
    versionsState.line,
    opened,
  );

  /** Reads one legal path; a refusal is shown and answered with null. */
  async function read(path) {
    const answer = await request("GET", path);
    if (answer.status !== 200) {
      message.textContent = await refusal(answer);
      return null;
    }
    return { state: answer.headers.get("Vcon-State") || "", ...(await versionAndText(answer)) };
  }

  async function find(uuid) {
    current = uuid;
    message.textContent = "";
    found.hidden = true;
    clear(versions.tBodies[0]);
    opened.hidden = true;
    const path = vconPath("/legal/vcons", uuid);
    const got = await read(path);
    if (got === null) {
      return;
    }
    state.textContent = got.state;
    panel.show(got, () => read(path));
    versionsState.loading();
    const listed = await request("GET", vconPath("/legal/vcons", uuid, "/versions"));
    if (listed.status !== 200) {
      versionsState.failed();
      message.textContent = await refusal(listed);
      return;
    }
    for (const v of await listed.json()) {
      const opener = el("button", "Open", { class: "open-version", type: "button", "aria-label": `Open version ${v.version}` });
      opener.addEventListener("click", async () => {
        message.textContent = "";
        const one = vconPath("/legal/vcons", uuid, `/versions/${encodeURIComponent(v.version)}`);
        const kept = await read(one);
        if (kept === null) {
          opened.hidden = true;
          return;
        }
        versionTitle.textContent = `Version ${v.version}`;
        versionTitle.hidden = false;
        versionPanel.show(kept, () => read(one));
        opened.hidden = false;
      });
      versions.tBodies[0].append(
        row([v.version, utc(v.stored_at), utc(v.hidden_at), v.current ? "yes" : "no"], [opener]),
      );
    }
    versionsState.loaded();
    found.hidden = false;
  }

  view.append(
    el("h2", "Legal"),
    form("legal-find", [...field("legal-uuid", "vCon uuid"), el("button", "Find", { type: "submit" })], () =>
      find(document.getElementById("legal-uuid").value.trim()),
    ),
    message,
    found,
  );
}

/** How many audit records a page of the audit view asks for. */
const AUDIT_PAGE = 50;

/** The scope names a tenant key may hold, in their fixed order. */
const TENANT_SCOPES = ["write", "read", "delete", "legal", "audit"];

/** Asks the user, through the browser's own dialog, before a change. */
function confirmed(question) {
  return window.confirm(question);
}

/** The audit view: the key's tenant log, or the admin log for an operator key. */
function auditView(view) {
  const admin = holds("admin");
  const path = admin ? "/admin/audit" : "/audit";
  const log = table("audit-log", ["seq", "at", "actor", "action", "target", "version", "status", "count"]);
  const state = stateOf(log, "records");
  const more = el("button", "More records", { id: "audit-more", type: "button", hidden: "" });
  const message = el("p", "", { id: "audit-message", role: "alert" });
  let after = 0;

  async function page() {
    const query = new URLSearchParams({ after: String(after), limit: String(AUDIT_PAGE) });
    state.loading();
    const answer = await request("GET", `${path}?${query}`);
    if (answer.status !== 200) {
      state.failed();
      message.textContent = await refusal(answer);
      return;
    }
    const lines = (await answer.text()).split("\n").filter((line) => line !== "");
    for (const line of lines) {
      const r = JSON.parse(line);
      log.tBodies[0].append(row([r.seq, utc(r.at), r.actor, r.action, r.target, r.version, r.status, r.count]));
      after = r.seq;
    }
    more.hidden = lines.length < AUDIT_PAGE;
    state.loaded();
  }

  more.addEventListener("click", () => page());
  view.append(el("h2", admin ? "Admin log" : "Audit log", { id: "audit-title" }), scrolling(log, "Audit records"), state.line, more, message);
  page();
}

/** The tenants view for an operator key: tenants, and each one's keys. */
function tenantsView(view) {
  const message = el("p", "", { id: "tenant-message", role: "alert" });
  const tenants = table("tenants", ["id", "name", "retention", ""]);
  const state = stateOf(tenants, "tenants");
  const keys = el("section", "", { id: "keys", hidden: "" });

  async function reload() {
    clear(tenants.tBodies[0]);
    state.loading();
    const answer = await request("GET", "/admin/tenants");
    if (answer.status !== 200) {
      state.failed();
      message.textContent = await refusal(answer);
      return;
    }
    for (const t of await answer.json()) {
      const showKeys = el("button", "Keys", { class: "keys", type: "button", "aria-label": `Keys of ${t.name}` });
      showKeys.addEventListener("click", () => keysOf(keys, t));
      const setRetention = el("button", "Retention", { class: "set-retention", type: "button", "aria-label": `Retention of ${t.name}` });
      setRetention.addEventListener("click", () => retention(t));
      const remove = el("button", "Remove", { class: "remove", type: "button", "aria-label": `Remove ${t.name}` });
      remove.addEventListener("click", () => removeTenant(t));
      tenants.tBodies[0].append(
        row([t.id, t.name, t.retention === null ? "server default" : t.retention], [showKeys, setRetention, remove]),
      );
    }
    state.loaded();
  }

  async function retention(t) {
    const period = window.prompt(`Retention period for ${t.name}, such as 7y, 18m or 90d; empty for the server default`, t.retention ?? "");
    if (period === null) {
      return;
    }
    const body = JSON.stringify({ retention: period.trim() === "" ? null : period.trim() });
    const answer = await request("PATCH", `/admin/tenants/${encodeURIComponent(t.id)}`, body, {
      "Content-Type": "application/json",
    });
    message.textContent = answer.status === 200 ? `Retention of ${t.name} set` : await refusal(answer);
    await reload();
  }

  async function removeTenant(t) {
    if (!confirmed(`Remove tenant ${t.name}?`)) {
      return;
    }
    const path = `/admin/tenants/${encodeURIComponent(t.id)}`;
    let answer = await request("DELETE", path);
    if (answer.status === 409) {
      const problem = await answer.json();
      if (problem.type.endsWith("/holds-vcons") && confirmed(`${t.name} still holds vCons. Remove them too?`)) {
        answer = await request("DELETE", `${path}?force=true`);
      } else {
        message.textContent = refusalText(problem.status, problem.title);
        return;
      }
    }
    message.textContent = answer.status === 204 ? `Removed ${t.name}` : await refusal(answer);
    keys.hidden = true;
    clear(keys);
    await reload();
  }

  const name = field("tenant-name", "Name");
  const period = el("input", "", { id: "tenant-retention", autocomplete: "off", spellcheck: "false" });
  const create = form(
    "tenant-create",
    [...name, el("label", "Retention (empty for the server default)", { for: "tenant-retention" }), period, el("button", "Create tenant", { type: "submit" })],
    async () => {
      const retention = period.value.trim();
      const body = JSON.stringify({ name: name[1].value, retention: retention === "" ? null : retention });
      const answer = await request("POST", "/admin/tenants", body, { "Content-Type": "application/json" });
      message.textContent = answer.status === 201 ? `Created ${name[1].value}` : await refusal(answer);
      await reload();
    },
  );
  view.append(el("h2", "Tenants"), create, message, scrolling(tenants, "Tenants"), state.line, keys);
  reload();
}

/** One tenant's keys: listed, issued (the token shown once) and revoked. */
async function keysOf(section, t) {
  clear(section);
  section.hidden = false;
  const message = el("p", "", { id: "key-message", role: "alert" });
  const token = el("p", "", { id: "new-token" });
  const listed = table("key-list", ["id", "scopes", "label", "created", "revoked", ""]);
  const state = stateOf(listed, "keys");
  const path = `/admin/tenants/${encodeURIComponent(t.id)}/keys`;

  async function reload() {
    clear(listed.tBodies[0]);
    state.loading();
    const answer = await request("GET", path);
    if (answer.status !== 200) {
      state.failed();
      message.textContent = await refusal(answer);
      return;
    }
    for (const k of await answer.json()) {
      const controls = [];
      if (k.revoked_at === null) {
        const revoke = el("button", "Revoke", { class: "revoke", type: "button", "aria-label": `Revoke key ${k.id}` });
        revoke.addEventListener("click", async () => {
          if (!confirmed(`Revoke key ${k.id}?`)) {
            return;
          }
          const gone = await request("DELETE", `/admin/keys/${encodeURIComponent(k.id)}`);
          message.textContent = gone.status === 204 ? `Revoked ${k.id}` : await refusal(gone);
          await reload();
        });
        controls.push(revoke);
      }
      listed.tBodies[0].append(row([k.id, k.scopes.join(", "), k.label, utc(k.created_at), utc(k.revoked_at)], controls));
    }
    state.loaded();
  }

  const choices = el("fieldset", "", { id: "key-scopes" });
  choices.append(el("legend", "Scopes"));
  for (const scope of TENANT_SCOPES) {
    const box = el("input", "", { type: "checkbox", name: "scope", value: scope, id: `scope-${scope}` });
    const choice = el("span", "", { class: "choice" });
    choice.append(box, el("label", scope, { for: `scope-${scope}` }));
    choices.append(choice);
  }
  const label = field("key-label", "Label");
  const create = form("key-create", [choices, ...label, el("button", "Issue key", { type: "submit" })], async () => {
    const scopes = TENANT_SCOPES.filter((s) => document.getElementById(`scope-${s}`).checked);
    const body = JSON.stringify({ scopes, label: label[1].value });
    const answer = await request("POST", path, body, { "Content-Type": "application/json" });
    if (answer.status === 201) {
      const made = await answer.json();
      token.textContent = made.token;
      message.textContent = `Issued ${made.id}. Copy the token now; it is not shown again.`;
    } else {
      token.textContent = "";
      message.textContent = await refusal(answer);
    }
    await reload();
  });
  section.append(el("h3", `Keys of ${t.name}`), create, message, token, scrolling(listed, `Keys of ${t.name}`), state.line);
  await reload();
}

/** The views, each shown only to a key whose scopes allow it. */
const VIEWS = [
  { name: "vcons", title: "vCons", allowed: () => holds("read") || holds("write") || holds("delete"), render: vconsView },
  { name: "legal", title: "Legal", allowed: () => holds("legal"), render: legalView },
  { name: "audit", title: "Audit log", allowed: () => holds("audit") || holds("admin"), render: auditView },
  { name: "tenants", title: "Tenants", allowed: () => holds("admin"), render: tenantsView },
];

function showViews() {
  const nav = document.getElementById("views");
  const view = document.getElementById("view");
  clear(nav);
  for (const v of VIEWS.filter((v) => v.allowed())) {
    const button = el("button", v.title, { type: "button", "data-view": v.name });
    button.addEventListener("click", () => {
      for (const other of nav.children) {
        other.removeAttribute("aria-current");
      }
      button.setAttribute("aria-current", "page");
      clear(view);
      v.render(view);
      view.hidden = false;
      // Focus goes to the view's heading, so a keyboard or screen-reader
      // user lands on what the button opened.
      const heading = view.querySelector("h2");
      heading.setAttribute("tabindex", "-1");
      heading.focus();
    });
    nav.append(button);
  }
  nav.hidden = nav.childElementCount === 0;
}

function showSignedIn() {
  document.getElementById("who").textContent =
    `Signed in (${key.scopes.join(", ")})`;
  const fields = document.getElementById("key-fields");
  clear(fields);
  const rows = [
    ["Key id", key.id],
    ["Tenant", key.tenant === null ? "none (operator key)" : key.tenant],
    ["Scopes", key.scopes.join(", ")],
  ];
  for (const [name, value] of rows) {
    fields.append(el("dt", name), el("dd", value));
  }
  show("sign-in", false);
  show("signed-in", true);
  show("key-info", true);
  showViews();
}

function showSignedOut(message = "") {
  token = null;
  key = null;
  clear(document.getElementById("key-fields"));
  clear(document.getElementById("views"));
  clear(document.getElementById("view"));
  document.getElementById("who").textContent = "";
  document.getElementById("sign-in-error").textContent = message;
  show("signed-in", false);
  show("key-info", false);
  show("views", false);
  show("view", false);
  show("sign-in", true);
}

async function signIn(event) {
  event.preventDefault();
  const field = document.getElementById("key");
  const candidate = field.value.trim();
  field.value = "";
  token = candidate;
  const answer = await request("GET", "/key");
  if (answer.status !== 200) {
    const why = answer.status === 401 ? "The key was not accepted." : await refusal(answer);
    showSignedOut(why);
    return;
  }
  key = await answer.json();
  showSignedIn();
}

document.getElementById("sign-in").addEventListener("submit", signIn);
document.getElementById("sign-out").addEventListener("click", () => showSignedOut());

export { el, clear, request, refusal, holds };
