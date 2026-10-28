"""Tests scripts/check_site_links.py: in the built site every internal
link names a built file, every #fragment names an id on its page, and the
problem types in openapi.json are exactly the built problems/ pages.

Run with: python3 -I scripts/test_check_site_links.py
"""

import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_site_links", HERE / "check_site_links.py")
check_site_links = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_site_links)

BASE = "https://vcon-server.com"
PROBLEMS = f"{BASE}/problems"


def page(body):
    return f"<!doctype html><html><head><title>t</title></head><body>{body}</body></html>"


class Site:
    """A built site in a temporary directory: a home page, a docs page
    with two headings, one problem page and the openapi.json naming it."""

    def __init__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        self.write("index.html", page('<a href="/docs/">Docs</a> <link rel="icon" href="https://vcon-server.com/favicon.ico">'))
        self.write("favicon.ico", "icon")
        self.write("docs/index.html", page('<h2 id="install">Install</h2><h2 id="run">Run</h2>'))
        self.write("problems/index.html", page('<a href="not-found/">not-found</a>'))
        self.write("problems/not-found/index.html", page("<h1>Not found</h1>"))
        self.openapi([f"{PROBLEMS}/not-found"])

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def openapi(self, types):
        doc = {"components": {"schemas": {"Problem": {"properties": {"type": {"type": "string", "enum": types}}}}}}
        self.write("openapi.json", json.dumps(doc))

    def problems(self):
        return check_site_links.check(self.root, BASE)

    def close(self):
        self.tmp.cleanup()


class SiteCase(unittest.TestCase):
    def setUp(self):
        self.site = Site()
        self.addCleanup(self.site.close)

    def assertOneProblem(self, *words):
        found = self.site.problems()
        self.assertEqual(len(found), 1, found)
        for word in words:
            self.assertIn(word, found[0])


class Links(SiteCase):
    def test_the_clean_site_has_no_problems(self):
        self.assertEqual(self.site.problems(), [])

    def test_a_link_to_a_missing_page_is_reported_with_its_page(self):
        self.site.write("docs/index.html", page('<a href="/docs/missing/">x</a>'))
        self.assertOneProblem("docs/index.html", "/docs/missing/")

    def test_a_relative_link_resolves_against_its_page(self):
        self.site.write("docs/a/index.html", page('<a href="../">up</a> <a href="../b/">b</a>'))
        self.site.write("docs/b/index.html", page(""))
        self.assertEqual(self.site.problems(), [])
        self.site.write("docs/a/index.html", page('<a href="../c/">c</a>'))
        self.assertOneProblem("docs/a/index.html", "../c/")

    def test_a_link_to_the_sites_own_domain_is_internal(self):
        self.site.write("index.html", page(f'<a href="{BASE}/nowhere/">x</a>'))
        self.assertOneProblem("index.html", f"{BASE}/nowhere/")

    def test_a_domain_that_starts_with_the_sites_name_is_another_site(self):
        # https://vcon-server.com.example.org is another site.
        self.site.write("index.html", page(f'<a href="{BASE}.example.org/x">x</a>'))
        self.assertEqual(self.site.problems(), [])

    def test_a_missing_script_or_image_is_reported(self):
        self.site.write("index.html", page('<script src="/js/gone.js"></script>'))
        self.assertOneProblem("index.html", "/js/gone.js")
        self.site.write("index.html", page('<img src="logo.png" alt="">'))
        self.assertOneProblem("index.html", "logo.png")

    def test_a_file_link_must_name_the_file(self):
        self.site.write("index.html", page('<a href="/favicon.ico">i</a> <a href="/openapi.json">o</a>'))
        self.assertEqual(self.site.problems(), [])
        self.site.write("index.html", page('<a href="/favicon.png">i</a>'))
        self.assertOneProblem("/favicon.png")

    def test_a_directory_without_an_index_is_missing(self):
        (self.site.root / "empty").mkdir()
        self.site.write("index.html", page('<a href="/empty/">e</a>'))
        self.assertOneProblem("/empty/")

    def test_a_link_cannot_leave_the_site(self):
        self.site.write("index.html", page('<a href="/../../etc/passwd">p</a>'))
        self.assertOneProblem("/../../etc/passwd")

    def test_the_query_is_not_part_of_the_path(self):
        self.site.write("index.html", page('<a href="/docs/?q=1">d</a>'))
        self.assertEqual(self.site.problems(), [])

    def test_a_percent_encoded_path_is_decoded(self):
        self.site.write("a b/index.html", page(""))
        self.site.write("index.html", page('<a href="/a%20b/">s</a>'))
        self.assertEqual(self.site.problems(), [])

    def test_other_sites_and_other_schemes_are_not_fetched_or_checked(self):
        links = [
            "https://example.com/missing",
            "http://example.org/",
            "//example.net/x",
            "mailto:security@example.com",
            "tel:+1-555-0100",
            "data:text/plain,x",
        ]
        self.site.write("index.html", page(" ".join(f'<a href="{h}">x</a>' for h in links)))
        self.assertEqual(self.site.problems(), [])

    def test_every_problem_on_a_page_is_reported(self):
        self.site.write("index.html", page('<a href="/a/">a</a><a href="/b/">b</a><a href="#c">c</a>'))
        self.assertEqual(len(self.site.problems()), 3)


class Fragments(SiteCase):
    def test_a_fragment_that_names_an_id_resolves(self):
        self.site.write("index.html", page('<a href="/docs/#install">i</a> <a href="docs/#run">r</a>'))
        self.assertEqual(self.site.problems(), [])

    def test_a_fragment_on_another_page_must_name_an_id_there(self):
        self.site.write("index.html", page('<a href="/docs/#upgrade">u</a>'))
        self.assertOneProblem("index.html", "/docs/#upgrade", "upgrade")

    def test_a_fragment_on_the_same_page_must_name_an_id_there(self):
        self.site.write("docs/index.html", page('<h2 id="install">I</h2><a href="#install">i</a>'))
        self.assertEqual(self.site.problems(), [])
        self.site.write("docs/index.html", page('<h2 id="install">I</h2><a href="#instal">i</a>'))
        self.assertOneProblem("docs/index.html", "#instal")

    def test_an_anchor_name_counts_as_an_id(self):
        self.site.write("docs/index.html", page('<a name="old"></a><a href="#old">o</a>'))
        self.assertEqual(self.site.problems(), [])

    def test_an_empty_fragment_is_the_top_of_the_page(self):
        self.site.write("index.html", page('<a href="#">top</a> <a href="/docs/#">d</a>'))
        self.assertEqual(self.site.problems(), [])

    def test_a_percent_encoded_fragment_is_decoded(self):
        self.site.write("docs/index.html", page('<h2 id="a-b">x</h2><a href="#a%2Db">x</a>'))
        self.assertEqual(self.site.problems(), [])

    def test_a_fragment_on_a_file_that_is_not_a_page_is_reported(self):
        self.site.write("index.html", page('<a href="/openapi.json#x">o</a>'))
        self.assertOneProblem("/openapi.json#x")


class ProblemTypes(SiteCase):
    def test_a_type_with_no_built_page_is_reported(self):
        self.site.openapi([f"{PROBLEMS}/not-found", f"{PROBLEMS}/gone"])
        self.assertOneProblem("openapi.json", "gone")

    def test_a_problem_page_with_no_type_is_reported(self):
        self.site.write("problems/stale/index.html", page(""))
        self.assertOneProblem("problems/stale", "openapi.json")

    def test_a_type_outside_the_sites_problems_is_reported(self):
        self.site.openapi([f"{PROBLEMS}/not-found", "https://example.com/problems/not-found"])
        self.assertOneProblem("openapi.json", "https://example.com/problems/not-found")

    def test_a_type_with_a_trailing_slash_or_deeper_path_is_reported(self):
        self.site.openapi([f"{PROBLEMS}/not-found/"])
        self.assertTrue(any(f"{PROBLEMS}/not-found/" in p for p in self.site.problems()))

    def test_a_site_without_the_problem_type_list_is_reported(self):
        self.site.write("openapi.json", json.dumps({"components": {}}))
        self.assertOneProblem("openapi.json")

    def test_a_site_without_openapi_json_is_reported(self):
        (self.site.root / "openapi.json").unlink()
        self.assertOneProblem("openapi.json")


if __name__ == "__main__":
    unittest.main()
