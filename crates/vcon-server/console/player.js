// The recording player of the vcon-server console. The console hands this
// page one recording as bytes, in a message; the page plays it from a blob
// it makes itself. It makes no request, keeps nothing in the browser and
// takes a message only from the console that opened it, on its own origin.

import { accepts, handed, PLAYER_READY, PLAYER_CLOSED } from "./format.js";

/** The one origin a message may come from or go to: this page's own. */
const ORIGIN = location.origin;

/** The console's window, or null when this page was opened by itself. */
const opener = window.opener;

const playing = document.getElementById("playing");
const audio = document.getElementById("audio");
const error = document.getElementById("player-error");

/** The blob URL of the recording held, or null when none is. */
let held = null;
/** Whether a recording was taken: the page takes one in its life. */
let taken = false;

/** Lets go of the recording: its URL no longer names it, and nothing plays. */
function release() {
  if (held !== null) {
    URL.revokeObjectURL(held);
    held = null;
  }
  audio.removeAttribute("src");
  audio.hidden = true;
}

/** Tells the console something, on this origin only. */
function tell(what) {
  if (opener !== null) {
    opener.postMessage(what, ORIGIN);
  }
}

window.addEventListener("message", (event) => {
  if (taken || !accepts(event, opener, ORIGIN)) {
    return;
  }
  const got = handed(event.data);
  if (got === null) {
    return;
  }
  taken = true;
  held = URL.createObjectURL(new Blob([got.bytes], { type: got.type }));
  audio.src = held;
  audio.hidden = false;
  playing.textContent = `Ready to play: ${got.label}. It does not start by itself: use the controls below.`;
  audio.focus();
});

audio.addEventListener("ended", () => {
  release();
  playing.textContent =
    "Playback ended, and the recording was released from this page. Choose Play in the console to hear it again.";
  document.getElementById("close").focus();
});

audio.addEventListener("error", () => {
  if (held === null) {
    return;
  }
  release();
  playing.textContent = "";
  error.textContent = "The browser could not play this recording. Download the vCon in the console to hear it another way.";
});

document.getElementById("close").addEventListener("click", () => window.close());

window.addEventListener("pagehide", () => {
  release();
  tell(PLAYER_CLOSED);
});

if (opener === null) {
  playing.textContent =
    "This page plays a recording the console hands it, and nothing else. Open a vCon in the console and choose Play.";
} else {
  tell(PLAYER_READY);
}
