#!/usr/bin/env python3
"""Tests of scripts/docs_test.py, the runner that holds the how-to pages to
a live server: how it reads a page, how it compares what a command printed
with what the page says it prints, the script it writes, and the rules it
holds every tested page to.

Run: python3 -I scripts/test_docs_test.py
"""

import os
import sys
import unittest

# No __pycache__ beside the scripts: nothing ignores it.
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import docs_test as dt  # noqa: E402

PAGE = """+++
title = "A page"
+++

Text.

```sh
echo one
```

```text
one
```

<!-- docs-test: only apt -->
```sh
apt-get install -y curl
```

<!-- docs-test: skip the reader picks a path -->
```sh
cd /somewhere/else
```

<!-- docs-test: file vcon-server.toml -->
```toml
listen = "0.0.0.0:8443"
```

<!-- docs-test: exit 1 -->
```sh
false
```

```ini
[Unit]
```
"""


class Parse(unittest.TestCase):
    def test_blocks_markers_and_outputs(self):
        blocks = dt.parse(PAGE, "page.md")
        kinds = [(b.kind, b.code.strip()) for b in blocks]
        self.assertEqual(
            kinds,
            [
                ("sh", "echo one"),
                ("sh", "apt-get install -y curl"),
                ("sh", "cd /somewhere/else"),
                ("file", 'listen = "0.0.0.0:8443"'),
                ("sh", "false"),
            ],
        )
        first = blocks[0]
        self.assertEqual(first.expect, "one\n")
        self.assertEqual(first.at, "page.md:7")
        self.assertEqual(blocks[1].only, {"apt"})
        self.assertEqual(blocks[2].skip, "the reader picks a path")
        self.assertEqual(blocks[3].path, "vcon-server.toml")
        self.assertEqual(blocks[4].exit, 1)
        # A block with no text block after it expects nothing in particular.
        self.assertIsNone(blocks[1].expect)

    def test_a_skip_needs_a_reason(self):
        with self.assertRaisesRegex(dt.PageError, "page.md:1: a skip needs a reason"):
            dt.parse("<!-- docs-test: skip -->\n```sh\nls\n```\n", "page.md")

    def test_an_unknown_marker_is_refused(self):
        with self.assertRaisesRegex(dt.PageError, "unknown docs-test marker `sometimes`"):
            dt.parse("<!-- docs-test: sometimes -->\n```sh\nls\n```\n", "page.md")

    def test_a_marker_must_sit_on_a_block(self):
        with self.assertRaisesRegex(dt.PageError, "page.md:1: a docs-test marker with no block"):
            dt.parse("<!-- docs-test: only apt -->\n\nText.\n", "page.md")

    def test_a_text_block_after_a_skipped_block_is_not_an_output(self):
        blocks = dt.parse(
            "<!-- docs-test: skip shown only -->\n```sh\nls\n```\n\n```text\nx\n```\n", "p.md"
        )
        self.assertEqual(len(blocks), 1)
        self.assertIsNone(blocks[0].expect)

    def test_prose_may_sit_between_but_a_heading_ends_it(self):
        blocks = dt.parse("```sh\nls\n```\n\nIt shows:\n\n```text\nx\n```\n", "p.md")
        self.assertEqual(blocks[0].expect, "x\n")
        blocks = dt.parse("```sh\nls\n```\n\n## Next\n\n```text\nx\n```\n", "p.md")
        self.assertIsNone(blocks[0].expect)


    def test_a_block_inside_a_list_item(self):
        page = "- Install:\n\n  <!-- docs-test: only apt -->\n  ```sh\n  apt-get update\n  ```\n\n  ```text\n  done\n  ```\n"
        blocks = dt.parse(page, "p.md")
        self.assertEqual(len(blocks), 1)
        self.assertEqual(blocks[0].code, "apt-get update\n")
        self.assertEqual(blocks[0].only, {"apt"})
        self.assertEqual(blocks[0].expect, "done\n")


    def test_indented_lines_of_a_block_keep_their_indent(self):
        blocks = dt.parse("```sh\nls\n```\n\n```text\nName:\n    value\n```\n", "p.md")
        self.assertEqual(blocks[0].expect, "Name:\n    value\n")


class Matches(unittest.TestCase):
    def test_exact_lines(self):
        self.assertIsNone(dt.mismatch("HTTP/2 201\n", "HTTP/2 201 \r\n"))
        self.assertIn("line 1", dt.mismatch("HTTP/2 201\n", "HTTP/2 200 \r\n"))

    def test_placeholders_match_any_text_on_their_line(self):
        want = 'etag: "<version>"\ndate: <date>\n'
        self.assertIsNone(dt.mismatch(want, 'etag: "e351"\ndate: Tue, 27 Oct 2026\n'))
        self.assertIsNotNone(dt.mismatch(want, "etag: e351\ndate: x\n"))
        # A placeholder never matches an empty value.
        self.assertIsNotNone(dt.mismatch(want, 'etag: ""\ndate: x\n'))

    def test_an_ellipsis_line_matches_any_lines(self):
        want = "serving\n…\n"
        self.assertIsNone(dt.mismatch(want, "serving\n"))
        self.assertIsNone(dt.mismatch(want, "serving\nrequest 1\nrequest 2\n"))
        self.assertIsNotNone(dt.mismatch(want, "starting\nserving\n"))
        self.assertIsNone(dt.mismatch("…\nstopped\n", "a\nb\nstopped\n"))

    def test_an_ellipsis_inside_a_line_is_literal(self):
        self.assertIsNotNone(dt.mismatch('{"id":"…"}\n', '{"id":"3f0c"}\n'))

    def test_a_missing_or_extra_line_is_a_mismatch(self):
        self.assertIsNotNone(dt.mismatch("a\nb\n", "a\n"))
        self.assertIsNotNone(dt.mismatch("a\n", "a\nb\n"))

    def test_blank_lines_at_either_end_do_not_count(self):
        self.assertIsNone(dt.mismatch("a\n", "\n\na\n\n"))

    def test_regex_characters_are_literal(self):
        self.assertIsNone(dt.mismatch("a.b [c] (d)*\n", "a.b [c] (d)*\n"))
        self.assertIsNotNone(dt.mismatch("a.b\n", "axb\n"))


class Script(unittest.TestCase):
    def scenario(self, tags, sets=None):
        return dt.Scenario(name="t", where="debian:trixie-slim", tags=frozenset(tags), pages=(), sets=sets or {})

    def test_only_the_scenario_tags_run(self):
        blocks = dt.parse(PAGE, "page.md")
        steps = dt.select(blocks, self.scenario({"apt"}))
        self.assertEqual([b.code.strip() for b in steps][:2], ["echo one", "apt-get install -y curl"])
        steps = dt.select(blocks, self.scenario({"dnf"}))
        self.assertNotIn("apt-get install -y curl", [b.code.strip() for b in steps])
        # Skipped blocks never run.
        self.assertNotIn("cd /somewhere/else", [b.code.strip() for b in steps])

    def test_every_listed_tag_is_needed(self):
        blocks = dt.parse("<!-- docs-test: only apt release -->\n```sh\nls\n```\n", "p.md")
        self.assertEqual(dt.select(blocks, self.scenario({"apt"})), [])
        self.assertEqual(len(dt.select(blocks, self.scenario({"apt", "release"}))), 1)

    def test_a_scenario_can_set_a_variable_the_page_sets(self):
        blocks = dt.parse("```sh\nVERSION=0.2.0\necho $VERSION\n```\n", "p.md")
        script = dt.script(dt.select(blocks, self.scenario(set())), "", {"VERSION": "0.1.0"})
        self.assertIn("VERSION=0.1.0\n", script)
        self.assertNotIn("VERSION=0.2.0", script)

    def test_a_variable_the_page_does_not_set_is_an_error(self):
        blocks = dt.parse("```sh\necho hi\n```\n", "p.md")
        with self.assertRaisesRegex(dt.PageError, "no page block sets VERSION"):
            dt.script(dt.select(blocks, self.scenario(set())), "", {"VERSION": "0.1.0"})

    def test_a_variable_two_blocks_set_is_an_error(self):
        blocks = dt.parse("```sh\nNEW=a\n```\n\n```sh\nNEW=$(make)\n```\n", "p.md")
        with self.assertRaisesRegex(dt.PageError, "p.md:5: NEW is set again"):
            dt.script(dt.select(blocks, self.scenario(set())), "", {"NEW": "b"})

    def test_the_script_runs_each_block_and_records_it(self):
        blocks = dt.parse(PAGE, "page.md")
        script = dt.script(dt.select(blocks, self.scenario({"apt"})), "echo prelude\n")
        self.assertIn("echo prelude", script)
        self.assertIn("echo one", script)
        self.assertIn("cat > 'vcon-server.toml' <<'DOCS_TEST_EOF'", script)
        self.assertIn('> "$DOCS_TEST_OUT/0.out" 2>&1', script)
        # The block expected to exit 1 runs without errexit.
        self.assertIn("set +e", script)


class Releases(unittest.TestCase):
    def blocks(self):
        return dt.parse("```sh\nVERSION=0.3.0\n```\n\n```sh\nNEW_VERSION=0.3.0\n```\n", "p.md")

    def test_a_published_release_runs_as_the_page_names_it(self):
        self.assertEqual(dt.release_sets(self.blocks(), {}, lambda v: True, lambda: "0.2.0"), {})

    def test_an_unpublished_release_runs_as_the_newest_published_one(self):
        got = dt.release_sets(self.blocks(), {}, lambda v: v != "0.3.0", lambda: "0.2.0")
        self.assertEqual(got, {"VERSION": "0.2.0", "NEW_VERSION": "0.2.0"})

    def test_a_version_the_scenario_sets_is_left_alone(self):
        got = dt.release_sets(self.blocks(), {"VERSION": "0.1.0"}, lambda v: False, lambda: "0.2.0")
        self.assertEqual(got, {"VERSION": "0.1.0", "NEW_VERSION": "0.2.0"})

    def test_no_newest_release_is_an_error(self):
        with self.assertRaisesRegex(dt.PageError, "cannot learn the newest release"):
            dt.release_sets(self.blocks(), {}, lambda v: False, lambda: None)


class Lint(unittest.TestCase):
    def test_a_request_needs_its_answer_on_the_page(self):
        page = "```sh\ncurl -sS https://vcon.example.com:8443/health\n```\n"
        problems = dt.lint_page(dt.parse(page, "p.md"), [frozenset()])
        self.assertEqual(problems, ["p.md:1: a request to the server with no output shown after it"])

    def test_a_block_no_scenario_runs_is_found(self):
        page = "<!-- docs-test: only nowhere -->\n```sh\nls\n```\n"
        problems = dt.lint_page(dt.parse(page, "p.md"), [frozenset({"apt"})])
        self.assertEqual(problems, ["p.md:2: no scenario runs this block (only nowhere)"])

    def test_a_release_variable_set_twice_is_found(self):
        page = "```sh\nVERSION=0.2.0\n```\n\n```sh\nVERSION=$VERSION docker compose up\n```\n"
        problems = dt.lint_page(dt.parse(page, "p.md"), [frozenset()])
        self.assertEqual(
            problems, ["p.md:5: VERSION is set again; a scenario that sets it would rewrite both"]
        )

    def test_a_good_page_has_no_problems(self):
        self.assertEqual(dt.lint_page(dt.parse(PAGE, "page.md"), [frozenset({"apt"})]), [])

    def test_the_real_pages_are_clean(self):
        self.assertEqual(dt.lint(), [])


if __name__ == "__main__":
    unittest.main()
