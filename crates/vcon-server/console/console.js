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
