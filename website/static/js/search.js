// Search over the docs with the index Zola builds (search_index.en.js) and
// elasticlunr. The search box stays hidden until this script runs, so a
// browser without JavaScript never shows a box that does nothing.
(function () {
  "use strict";
  function start() {
    var box = document.getElementById("search");
    var input = document.getElementById("search-input");
    var list = document.getElementById("search-results");
    // A live region (role="status"): what the search found is announced
    // without moving focus from the box (WCAG 4.1.3).
    var status = document.getElementById("search-status");
    if (!box || !input || !list || !status || !window.elasticlunr || !window.searchIndex) {
      return;
    }
    var index = window.elasticlunr.Index.load(window.searchIndex);
    box.hidden = false;
    function show(term) {
      while (list.firstChild) {
        list.removeChild(list.firstChild);
      }
      if (term.trim().length < 2) {
        list.hidden = true;
        status.textContent = "";
        return;
      }
      var found = index.search(term, {
        bool: "AND",
        expand: true,
        fields: { title: { boost: 3 }, description: { boost: 2 }, body: { boost: 1 } }
      }).slice(0, 8);
      if (found.length === 0) {
        var none = document.createElement("li");
        none.className = "none";
        none.textContent = "No match";
        list.appendChild(none);
      }
      found.forEach(function (hit) {
        var doc = hit.doc || {};
        var item = document.createElement("li");
        var link = document.createElement("a");
        link.href = hit.ref;
        link.textContent = doc.title || hit.ref;
        item.appendChild(link);
        if (doc.description) {
          var desc = document.createElement("span");
          desc.textContent = doc.description;
          item.appendChild(desc);
        }
        list.appendChild(item);
      });
      list.hidden = false;
      status.textContent = found.length === 0 ? "No match for " + term.trim()
        : found.length + (found.length === 1 ? " result" : " results") + " for " + term.trim();
    }
    input.addEventListener("input", function () { show(input.value); });
    input.addEventListener("keydown", function (e) {
      if (e.key === "Escape") { input.value = ""; show(""); }
    });
    document.addEventListener("click", function (e) {
      if (!box.contains(e.target)) { list.hidden = true; }
    });
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();
