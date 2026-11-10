// Search over the docs with the index Zola builds (search_index.en.js) and
// elasticlunr. The search box stays hidden until this script runs, so a
// browser without JavaScript never shows a box that does nothing. The index
// is the site's largest file by far, so it and elasticlunr are fetched only
// when the box is first used: their addresses are the box's data-lunr and
// data-index attributes (templates/base.html).
(function () {
  "use strict";
  function start() {
    var box = document.getElementById("search");
    var input = document.getElementById("search-input");
    var list = document.getElementById("search-results");
    // A live region (role="status"): what the search found is announced
    // without moving focus from the box (WCAG 4.1.3).
    var status = document.getElementById("search-status");
    if (!box || !input || !list || !status) {
      return;
    }
    var files = [box.getAttribute("data-lunr"), box.getAttribute("data-index")];
    if (!files[0] || !files[1]) {
      return;
    }
    var index = null;
    var loading = false;
    box.hidden = false;
    // Adds one script to the page, then calls `done`. The page policy
    // allows scripts from this site only, and these are two of them.
    function add(src, done) {
      var script = document.createElement("script");
      script.src = src;
      script.addEventListener("load", done);
      script.addEventListener("error", function () {
        loading = false;
        script.remove();
        status.textContent = "Search could not be loaded";
      });
      document.head.appendChild(script);
    }
    // Fetches the library and the index, once, then searches for whatever
    // was typed while they were on their way. A failed fetch is announced,
    // and the next use of the box tries again.
    function load() {
      if (index || loading) {
        return;
      }
      loading = true;
      function ready() {
        index = window.elasticlunr.Index.load(window.searchIndex);
        loading = false;
        show(input.value);
      }
      if (window.elasticlunr) {
        add(files[1], ready);
      } else {
        add(files[0], function () { add(files[1], ready); });
      }
    }
    function show(term) {
      while (list.firstChild) {
        list.removeChild(list.firstChild);
      }
      if (term.trim().length < 2) {
        list.hidden = true;
        status.textContent = "";
        return;
      }
      if (!index) {
        list.hidden = true;
        status.textContent = "";
        load();
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
    input.addEventListener("focus", load);
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
