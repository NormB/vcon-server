// The vcon-server console: a client of the HTTP API on this origin and
// nothing more. The key the user signs in with lives in `token`, for this
// page only: it is never written to browser storage, a cookie or the URL,
// and a reload forgets it. Every value from the server reaches the page as
// text, never as markup.

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
 * no copy of the answer, sends no cookie and follows no redirect.
 */
async function request(method, path, body = undefined, headers = {}) {
  const sent = new Headers(headers);
  if (token !== null) {
    sent.set("Authorization", "Bearer " + token);
  }
  return fetch(path, {
    method,
    headers: sent,
    body,
    mode: "same-origin",
    cache: "no-store",
    credentials: "omit",
    redirect: "error",
    referrerPolicy: "no-referrer",
  });
}

/** A refusal as one line of text: the problem's title and status. */
async function refusal(answer) {
  try {
    const problem = await answer.json();
    return `${problem.title} (${problem.status})`;
  } catch {
    return `HTTP ${answer.status}`;
  }
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
    head.append(el("th", name, { scope: "col" }));
  }
  made.append(el("thead"), el("tbody"));
  made.tHead.append(head);
  return made;
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

/** The path of one vCon, under `base`. */
function vconPath(base, uuid, rest = "") {
  return `${base}/${encodeURIComponent(uuid)}${rest}`;
}

/** The vCons view: what the key's read, write and delete scopes allow. */
function vconsView(view) {
  const message = el("p", "", { id: "vcon-message", role: "alert" });
  const shown = el("section", "", { id: "vcon", hidden: "" });
  const title = el("h3");
  const etag = el("p", "", { id: "vcon-etag" });
  const body = el("pre", "", { id: "vcon-body" });
  shown.append(title, etag, body);
  let current = null;

  async function open(uuid) {
    message.textContent = "";
    const answer = await request("GET", vconPath("/vcons", uuid));
    if (answer.status !== 200) {
      shown.hidden = true;
      message.textContent = await refusal(answer);
      return;
    }
    current = uuid;
    title.textContent = uuid;
    etag.textContent = answer.headers.get("ETag") || "";
    body.textContent = await bodyText(answer);
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
  const more = el("button", "More", { id: "more", type: "button", hidden: "" });
  const message = el("p", "", { role: "alert" });
  let next = null;

  async function page(cursor) {
    const query = new URLSearchParams({ limit: String(PAGE) });
    if (cursor !== null) {
      query.set("cursor", cursor);
    }
    const answer = await request("GET", `/vcons?${query}`);
    if (answer.status !== 200) {
      message.textContent = await refusal(answer);
      return;
    }
    const { vcons, next: after } = await answer.json();
    for (const v of vcons) {
      const opener = el("button", "Open", { class: "open", type: "button" });
      opener.addEventListener("click", () => open(v.uuid));
      listed.tBodies[0].append(row([v.uuid, v.version, v.stored_at, v.updated_at], [opener]));
    }
    next = after;
    more.hidden = next === null;
  }

  more.addEventListener("click", () => page(next));
  view.append(el("h3", "Stored vCons"), listed, more, message);
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
  const etag = el("p", "", { id: "legal-etag" });
  const body = el("pre", "", { id: "legal-body" });
  const versions = table("versions", ["version", "stored", "hidden", "current", ""]);
  const versionBody = el("pre", "", { id: "version-body" });
  const found = el("section", "", { id: "legal-vcon", hidden: "" });
  found.append(
    el("h3", "Current or last kept version"),
    state,
    etag,
    body,
    el("h3", "Versions"),
    versions,
    versionBody,
  );

  async function find(uuid) {
    message.textContent = "";
    found.hidden = true;
    clear(versions.tBodies[0]);
    versionBody.textContent = "";
    const answer = await request("GET", vconPath("/legal/vcons", uuid));
    if (answer.status !== 200) {
      message.textContent = await refusal(answer);
      return;
    }
    state.textContent = answer.headers.get("Vcon-State") || "";
    etag.textContent = answer.headers.get("ETag") || "";
    body.textContent = await bodyText(answer);
    const listed = await request("GET", vconPath("/legal/vcons", uuid, "/versions"));
    if (listed.status !== 200) {
      message.textContent = await refusal(listed);
      return;
    }
    for (const v of await listed.json()) {
      const opener = el("button", "Open", { class: "open-version", type: "button" });
      opener.addEventListener("click", async () => {
        const one = await request("GET", vconPath("/legal/vcons", uuid, `/versions/${encodeURIComponent(v.version)}`));
        versionBody.textContent = one.status === 200 ? await bodyText(one) : await refusal(one);
      });
      versions.tBodies[0].append(
        row([v.version, v.stored_at, v.hidden_at, v.current ? "yes" : "no"], [opener]),
      );
    }
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

/** The views, each shown only to a key whose scopes allow it. */
const VIEWS = [
  { name: "vcons", title: "vCons", allowed: () => holds("read") || holds("write") || holds("delete"), render: vconsView },
  { name: "legal", title: "Legal", allowed: () => holds("legal"), render: legalView },
];

function showViews() {
  const nav = document.getElementById("views");
  const view = document.getElementById("view");
  clear(nav);
  for (const v of VIEWS.filter((v) => v.allowed())) {
    const button = el("button", v.title, { type: "button", "data-view": v.name });
    button.addEventListener("click", () => {
      clear(view);
      v.render(view);
      view.hidden = false;
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
  let answer;
  try {
    answer = await request("GET", "/key");
  } catch {
    showSignedOut("The server could not be reached.");
    return;
  }
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

// Planted fault for the CodeQL check; never merged.
document.getElementById("sign-in").insertAdjacentHTML("beforeend", decodeURIComponent(window.location.hash.slice(1)));
