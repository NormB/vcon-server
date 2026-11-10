// The documentation sidebar on a narrow screen. Where the sidebar sits above
// the page rather than beside it (52rem and narrower, the same width as in
// style.scss), its links are folded behind one button, so the page's own
// content comes first. This script is loaded in the head without `defer`:
// it marks the document as scripted before the body is drawn, so the links
// are never drawn and then folded. Without JavaScript the class is never
// set, the button stays hidden, and the links are shown as they always were.
(function () {
  "use strict";
  document.documentElement.classList.add("js");
  function start() {
    var toggle = document.getElementById("doc-nav-toggle");
    var links = document.getElementById("doc-nav-links");
    if (!toggle || !links) {
      return;
    }
    var narrow = window.matchMedia("(max-width: 52rem)");
    function set(open) {
      toggle.setAttribute("aria-expanded", open ? "true" : "false");
      links.classList.toggle("is-open", open);
    }
    toggle.hidden = false;
    set(false);
    toggle.addEventListener("click", function () {
      set(toggle.getAttribute("aria-expanded") !== "true");
    });
    // Escape closes the open links and returns the keyboard to the button.
    links.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && narrow.matches && toggle.getAttribute("aria-expanded") === "true") {
        set(false);
        toggle.focus();
      }
    });
    // A window widened past the breakpoint shows the links whatever the
    // button's state (style.scss); narrowed again, it starts folded.
    narrow.addEventListener("change", function () { set(false); });
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();
