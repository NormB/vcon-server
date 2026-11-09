#!/usr/bin/env python3
"""Tests of scripts/docs_test.py, the runner that holds the how-to pages to
a live server: how it reads a page, how it compares what a command printed
with what the page says it prints, the script it writes, and the rules it
holds every tested page to.

Run: python3 -I scripts/test_docs_test.py
"""

import os
import shutil
import tempfile
import subprocess
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


    def test_a_since_marker_names_the_first_release_a_step_runs_on(self):
        page = "From 0.3.0 on:\n\n<!-- docs-test: only release; since 0.3.0 -->\n```sh\nls\n```\n"
        blocks = dt.parse(page, "p.md")
        self.assertEqual(blocks[0].since, "0.3.0", "the marker's version is kept")
        self.assertEqual(blocks[0].only, {"release"}, "since combines with only")

    def test_a_since_marker_needs_a_version(self):
        for text in ("since", "since next", "since 0.3"):
            with self.subTest(text=text), self.assertRaisesRegex(
                dt.PageError, "p.md:1: `since` needs a release version such as 0.3.0"
            ):
                dt.parse(f"<!-- docs-test: {text} -->\n```sh\nls\n```\n", "p.md")

    def test_the_text_before_a_block_is_kept_up_to_the_last_block(self):
        page = "## Heading\n\nFirst.\n\n```sh\nls\n```\n\nSecond, from 0.3.0.\n\n```sh\npwd\n```\n"
        blocks = dt.parse(page, "p.md")
        self.assertIn("First.", blocks[0].prose, "a block's own text is kept")
        self.assertIn("## Heading", blocks[0].prose, "the heading over it is part of it")
        self.assertNotIn("First.", blocks[1].prose, "the text of an earlier block is not")
        self.assertIn("Second, from 0.3.0.", blocks[1].prose, "the text after the last block is")

    def test_a_heading_starts_the_text_again(self):
        page = "Old text 0.3.0.\n\n## New\n\nNew text.\n\n```sh\nls\n```\n"
        blocks = dt.parse(page, "p.md")
        self.assertNotIn("0.3.0", blocks[0].prose, "text above the heading belongs to another section")


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


class Directories(unittest.TestCase):
    """A reader comes back to a later page in another shell, in another
    directory, so each page after the first starts in a fresh one."""

    def steps(self):
        one = dt.parse("```sh\ntouch made.txt\n```\n\n```sh\nls made.txt\n```\n", "docs/one.md")
        two = dt.parse("```sh\nls made.txt\n```\n", "docs/two.md")
        return one + two

    def test_each_page_after_the_first_starts_in_a_fresh_directory(self):
        script = dt.script(self.steps(), "")
        first, second, third = (script.index(f"# {n}: ") for n in range(3))
        self.assertNotIn(dt.NEW_PAGE_DIR, script[:first], "the first page runs where the run starts")
        self.assertNotIn(dt.NEW_PAGE_DIR, script[first:second], "two steps of one page share a directory")
        self.assertIn(dt.NEW_PAGE_DIR, script[second:third], "the second page starts in a fresh directory")

    def test_a_later_page_cannot_see_an_earlier_pages_relative_file(self):
        work = tempfile.mkdtemp()
        out = os.path.join(work, "out")
        os.mkdir(out)
        try:
            with open(os.path.join(work, "run.sh"), "w", encoding="utf-8") as f:
                f.write(dt.script(self.steps(), ""))
            env = dict(os.environ, DOCS_TEST_OUT=out, DOCS_TEST_WORK=work)
            subprocess.run(["bash", os.path.join(work, "run.sh")], env=env, check=False, timeout=60)
            rcs = [open(os.path.join(out, f"{n}.rc")).read().strip() if os.path.exists(os.path.join(out, f"{n}.rc")) else None for n in range(3)]
        finally:
            shutil.rmtree(work)
        self.assertEqual(rcs[:2], ["0", "0"], "the first page finds its own file")
        self.assertNotEqual(rcs[2], "0", "the second page does not find the first page's file")


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


class Since(unittest.TestCase):
    PAGE = (
        "```sh\nVERSION=0.3.0\n```\n\nFrom 0.3.0:\n\n"
        "<!-- docs-test: since 0.3.0 -->\n```sh\ncosign verify-blob x\n```\n\n"
        "```sh\nsha256sum -c y\n```\n"
    )

    def codes(self, steps):
        return [b.code.strip() for b in steps]

    def test_a_step_runs_on_the_release_it_names(self):
        kept, left_out = dt.release_steps(dt.parse(self.PAGE, "p.md"), {})
        self.assertIn("cosign verify-blob x", self.codes(kept), "VERSION=0.3.0 on the page runs it")
        self.assertEqual(left_out, [], "nothing is left out")

    def test_a_step_does_not_run_on_an_older_release(self):
        kept, left_out = dt.release_steps(dt.parse(self.PAGE, "p.md"), {"VERSION": "0.2.0"})
        self.assertNotIn("cosign verify-blob x", self.codes(kept), "0.2.0 is before 0.3.0")
        self.assertEqual(self.codes(left_out), ["cosign verify-blob x"], "the step is reported as left out")
        self.assertIn("sha256sum -c y", self.codes(kept), "the other steps still run")

    def test_a_later_release_runs_it_and_versions_compare_as_numbers(self):
        page = self.PAGE.replace("since 0.3.0", "since 0.9.0")
        kept, _ = dt.release_steps(dt.parse(page, "p.md"), {"VERSION": "0.10.0"})
        self.assertIn("cosign verify-blob x", self.codes(kept), "0.10.0 is after 0.9.0, not before")
        kept, _ = dt.release_steps(dt.parse(page, "p.md"), {"VERSION": "0.8.1"})
        self.assertNotIn("cosign verify-blob x", self.codes(kept), "0.8.1 is before 0.9.0")

    def test_a_since_step_with_no_release_set_is_an_error(self):
        page = "<!-- docs-test: since 0.3.0 -->\n```sh\nls\n```\n"
        with self.assertRaisesRegex(dt.PageError, "p.md:2: a step from release 0.3.0 on, and no VERSION"):
            dt.release_steps(dt.parse(page, "p.md"), {})

    def test_a_run_before_the_release_leaves_the_step_out_and_says_so(self):
        steps, sets, notes = dt.plan(dt.parse(self.PAGE, "p.md"), {}, lambda v: v != "0.3.0", lambda: "0.2.0")
        self.assertEqual(sets, {"VERSION": "0.2.0"}, "the newest published release stands in")
        self.assertNotIn("cosign verify-blob x", self.codes(steps), "the signature step waits for 0.3.0")
        self.assertIn("sha256sum -c y", self.codes(steps), "the other steps run")
        self.assertEqual(
            notes,
            [
                "the release a page names is not published yet; running with VERSION=0.2.0",
                "p.md:8 is for release 0.3.0 and later; this run's release is older, so it does not run",
            ],
            "the run says what it ran with and what it left out",
        )

    def test_a_run_after_the_release_runs_the_step(self):
        steps, sets, notes = dt.plan(dt.parse(self.PAGE, "p.md"), {}, lambda v: True, lambda: "0.3.0")
        self.assertEqual((sets, notes), ({}, []), "the page's own release is published")
        self.assertIn("cosign verify-blob x", self.codes(steps), "the signature step runs")

    def test_pages_without_since_need_no_release(self):
        kept, left_out = dt.release_steps(dt.parse("```sh\nls\n```\n", "p.md"), {})
        self.assertEqual((self.codes(kept), left_out), (["ls"], []), "every step runs")


class HomePage(unittest.TestCase):
    INSTALL = dt.parse("```sh\nVERSION=0.2.0\ncurl -fsSLO x\nsha256sum -c y\n```\n", "docs/install.md")

    def test_each_install_step_is_on_the_install_page(self):
        front = '[extra]\n[[extra.install]]\ntitle = "t"\ncode = """\nVERSION=0.2.0\ncurl -fsSLO x"""\n'
        self.assertEqual(dt.home_install_problems(front, self.INSTALL), [])

    def test_a_step_the_install_page_does_not_show_is_found(self):
        front = '[extra]\n[[extra.install]]\ntitle = "Fetch"\ncode = """\ncurl -fsSL x"""\n'
        self.assertEqual(
            dt.home_install_problems(front, self.INSTALL),
            ["_index.md: the install step \"Fetch\" is not a run of lines in any block of docs/install.md"],
        )

    def test_a_step_without_code_needs_nothing(self):
        front = '[extra]\n[[extra.install]]\ntitle = "Then"\nnote = "n"\n'
        self.assertEqual(dt.home_install_problems(front, self.INSTALL), [])


class GitHubApi(unittest.TestCase):
    """The release lookups ask the GitHub API with the job's token, and a
    refusal says why instead of passing for an unpublished release."""

    def runner(self, stdout, returncode=0, stderr=""):
        calls = []

        def run(cmd, **kw):
            calls.append((cmd, kw))
            return subprocess.CompletedProcess(cmd, returncode, stdout, stderr)

        return run, calls

    def test_the_token_goes_on_standard_input_never_in_the_arguments(self):
        run, calls = self.runner('{"tag_name":"v0.2.0"}\n200')
        self.assertEqual(dt.github_api("repos/o/r/releases/latest", "secret-token", run), (200, '{"tag_name":"v0.2.0"}'))
        cmd, kw = calls[0]
        self.assertNotIn("secret-token", " ".join(cmd), "a token in the arguments shows in ps")
        self.assertEqual(kw["input"], "Authorization: Bearer secret-token\n")
        self.assertIn("@-", cmd, "curl reads the header from standard input")

    def test_without_a_token_no_header_is_sent(self):
        run, calls = self.runner("{}\n200")
        dt.github_api("repos/o/r/releases/latest", None, run)
        self.assertNotIn("@-", calls[0][0])
        self.assertIsNone(calls[0][1]["input"])

    def test_a_refusal_names_its_status_and_message(self):
        run, _ = self.runner('{"message":"API rate limit exceeded for 192.0.2.1."}\n403')
        with self.assertRaisesRegex(dt.PageError, "403: API rate limit exceeded"):
            dt.released("0.2.0", None, run)
        with self.assertRaisesRegex(dt.PageError, "403: API rate limit exceeded"):
            dt.latest_release(None, run)

    def test_released_is_true_on_200_and_false_only_on_404(self):
        run, _ = self.runner("{}\n200")
        self.assertTrue(dt.released("0.2.0", None, run))
        run, _ = self.runner('{"message":"Not Found"}\n404')
        self.assertFalse(dt.released("0.9.0", None, run))

    def test_the_newest_release_is_read_from_its_tag(self):
        run, _ = self.runner('{"tag_name":"v0.3.0"}\n200')
        self.assertEqual(dt.latest_release(None, run), "0.3.0")

    def test_a_curl_failure_says_so(self):
        run, _ = self.runner("", returncode=6, stderr="curl: (6) Could not resolve host: api.github.com")
        with self.assertRaisesRegex(dt.PageError, "Could not resolve host"):
            dt.released("0.2.0", None, run)


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

    def test_a_relative_cacert_is_found(self):
        for arg in ("--cacert fullchain.pem", '--cacert "ca.pem"', "--cacert=./ca.pem"):
            with self.subTest(arg=arg):
                page = f"```sh\ncurl -sS {arg} https://example.com/health\n```\n\n```text\nok\n```\n"
                problems = dt.lint_page(dt.parse(page, "p.md"), [frozenset()])
                self.assertEqual(
                    problems,
                    ["p.md:1: --cacert names a relative path, which works only in the directory that made it"],
                    "a reader in another directory gets curl error 77",
                )

    def test_an_absolute_or_variable_cacert_is_clean(self):
        for arg in ("--cacert /usr/local/share/vcon-server/ca.pem", '--cacert "$CA"'):
            with self.subTest(arg=arg):
                page = f"```sh\ncurl -sS {arg} https://example.com/health\n```\n\n```text\nok\n```\n"
                self.assertEqual(dt.lint_page(dt.parse(page, "p.md"), [frozenset()]), [], arg)

    def test_a_good_page_has_no_problems(self):
        self.assertEqual(dt.lint_page(dt.parse(PAGE, "page.md"), [frozenset({"apt"})]), [])

    def test_a_skip_that_names_a_release_is_found(self):
        for reason in ("no release holds it until the release after 0.2.0", "not until the next release"):
            with self.subTest(reason=reason):
                page = f"Text.\n\n<!-- docs-test: skip {reason} -->\n```sh\nls\n```\n"
                problems = dt.lint_page(dt.parse(page, "p.md"), [frozenset()])
                self.assertEqual(
                    problems,
                    [
                        "p.md:4: a skip that names a release; mark the step `since <version>`, "
                        "so it runs once that release is published"
                    ],
                    "a release condition must not hide a step for good",
                )

    def test_a_since_step_the_text_does_not_explain_is_found(self):
        page = "Check the signature:\n\n<!-- docs-test: since 0.3.0 -->\n```sh\nls\n```\n"
        problems = dt.lint_page(dt.parse(page, "p.md"), [frozenset()])
        self.assertEqual(
            problems,
            [
                "p.md:4: the step runs from release 0.3.0 on; the text before it must name 0.3.0, "
                "so a reader of an older release knows to skip it"
            ],
            "a reader must learn which releases the step is for",
        )

    def test_a_since_step_the_text_explains_is_clean(self):
        page = (
            "Releases from 0.3.0 hold the signature. 0.2.0 and earlier have none; skip this step.\n\n"
            "<!-- docs-test: since 0.3.0 -->\n```sh\nls\n```\n"
        )
        self.assertEqual(dt.lint_page(dt.parse(page, "p.md"), [frozenset()]), [], "the text names 0.3.0")

    def test_a_since_step_explained_only_in_another_section_is_found(self):
        page = (
            "From 0.3.0 on.\n\n## Other\n\nCheck it:\n\n"
            "<!-- docs-test: since 0.3.0 -->\n```sh\nls\n```\n"
        )
        problems = dt.lint_page(dt.parse(page, "p.md"), [frozenset()])
        self.assertEqual(len(problems), 1, f"the text above the heading does not count: {problems}")

    def test_the_real_pages_are_clean(self):
        self.assertEqual(dt.lint(), [])



# ---------------------------------------------------------------------------
# Release assets through the runner's pinned mirror. On 2026-11-09 the
# release-*, upgrade and container scenarios failed at the pages' own
# downloads of cosign and of a vcon-server release: github.com answered 504.
# ---------------------------------------------------------------------------

SHA_A = "a" * 64
SHA_B = "b" * 64
REL = "https://github.com/NormB/vcon-server/releases/download"


def pins_text(*lines):
    return "# a comment\n\n" + "".join(f"{l}\n" for l in lines)


class Pins(unittest.TestCase):
    def test_lines_of_a_hash_and_a_url_are_read(self):
        pins = dt.load_pins(pins_text(f"{SHA_A}  {REL}/v0.4.0/SHA256SUMS.txt", f"{SHA_B}  {REL}/v0.4.0/x.tar.gz"))
        self.assertEqual(pins, {f"{REL}/v0.4.0/SHA256SUMS.txt": SHA_A, f"{REL}/v0.4.0/x.tar.gz": SHA_B})

    def test_a_short_hash_is_refused_with_its_line(self):
        with self.assertRaisesRegex(dt.PageError, r"pins.txt:3: .*64 lowercase hex"):
            dt.load_pins(pins_text(f"{SHA_A[:60]}  {REL}/v0.4.0/x"), "pins.txt")

    def test_a_url_that_is_not_a_release_asset_is_refused(self):
        with self.assertRaisesRegex(dt.PageError, "not a GitHub release asset"):
            dt.load_pins(pins_text(f"{SHA_A}  https://example.com/x.tar.gz"))

    def test_one_url_pinned_twice_is_refused(self):
        with self.assertRaisesRegex(dt.PageError, "pinned twice"):
            dt.load_pins(pins_text(f"{SHA_A}  {REL}/v1/x", f"{SHA_B}  {REL}/v1/x"))

    def test_the_repository_pin_file_loads(self):
        with open(dt.PINS_PATH, encoding="utf-8") as f:
            pins = dt.load_pins(f.read())
        self.assertIn(f"{REL}/v{dt.PREVIOUS}/SHA256SUMS.txt", pins)


class Rewrite(unittest.TestCase):
    BASE = "http://192.0.2.1:4242"

    def test_a_release_url_in_a_curl_line_goes_to_the_mirror(self):
        code = 'curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$VERSION/$ARCHIVE\n'
        self.assertEqual(dt.rewrite_releases(code, self.BASE), f'curl -fsSLO {self.BASE}/NormB/vcon-server/releases/download/v$VERSION/$ARCHIVE\n')

    def test_a_quoted_url_in_a_dockerfile_goes_to_the_mirror(self):
        code = ' && curl -fsSLO "https://github.com/NormB/vcon-server/releases/download/v${VERSION}/${archive}" \\\n'
        self.assertIn(f'"{self.BASE}/NormB/vcon-server/releases/download/v${{VERSION}}/${{archive}}"', dt.rewrite_releases(code, self.BASE))

    def test_two_urls_on_one_line_both_go(self):
        code = f"a {REL}/v1/x b {REL}/v1/y\n"
        self.assertEqual(dt.rewrite_releases(code, self.BASE).count(self.BASE), 2)

    def test_a_github_url_that_is_not_a_release_asset_is_kept(self):
        code = "see https://github.com/NormB/vcon-server/blob/main/SECURITY.md and https://github.com/NormB/vcon-server/releases/latest\n"
        self.assertEqual(dt.rewrite_releases(code, self.BASE), code)

    def test_the_api_host_is_kept(self):
        code = "curl https://api.github.com/repos/o/r/releases/download/x\n"
        self.assertEqual(dt.rewrite_releases(code, self.BASE), code)


def page_with(code):
    return dt.parse(f"Download it:\n\n```sh\n{code}```\n", "docs/p.md")


class PinLint(unittest.TestCase):
    PINS = {f"{REL}/v0.4.0/vcon-server-0.4.0-x86_64-unknown-linux-gnu.tar.gz": SHA_A,
            "https://github.com/o/t/releases/download/1.0/t-linux-amd64.zip": SHA_B}

    def test_a_templated_url_with_a_matching_pin_passes(self):
        blocks = page_with("curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$VERSION/vcon-server-$VERSION-$(uname -m)-unknown-linux-gnu.tar.gz\n")
        self.assertEqual(dt.pin_problems(blocks, self.PINS), [])

    def test_a_release_url_with_no_pin_is_found_with_its_place(self):
        blocks = page_with("curl -fsSLO https://github.com/o/other/releases/download/v1/x.zip\n")
        self.assertEqual(dt.pin_problems(blocks, self.PINS), ["docs/p.md:3: no pin in scripts/docs-test-pins.txt matches https://github.com/o/other/releases/download/v1/x.zip"])

    def test_a_variable_does_not_stand_for_a_slash(self):
        pins = {"https://github.com/o/t/releases/download/1.0/sub/t.zip": SHA_A}
        blocks = page_with('curl -fsSL -O "https://github.com/o/t/releases/download/$V/t.zip"\n')
        self.assertEqual(len(dt.pin_problems(blocks, pins)), 1)

    def test_a_hash_on_the_page_that_is_not_a_pin_is_found(self):
        blocks = page_with(f'T_SHA256={"c" * 64}\ncurl -fsSL -O "https://github.com/o/t/releases/download/$V/t-linux-$ARCH.zip"\n')
        self.assertEqual(dt.pin_problems(blocks, self.PINS), [f"docs/p.md:3: sha256 {'c' * 64} on the page is not in scripts/docs-test-pins.txt; the page and the pins name one file"])

    def test_a_hash_on_the_page_that_is_a_pin_passes(self):
        blocks = page_with(f'T_SHA256={SHA_B}\ncurl -fsSL -O "https://github.com/o/t/releases/download/$V/t-linux-$ARCH.zip"\n')
        self.assertEqual(dt.pin_problems(blocks, self.PINS), [])


class LintHoldsThePins(unittest.TestCase):
    def test_lint_finds_the_pages_downloads_when_nothing_is_pinned(self):
        saved = dt.read_pins
        dt.read_pins = lambda: {}
        try:
            problems = dt.lint()
        finally:
            dt.read_pins = saved
        self.assertTrue(any("docs/install.md" in p and "no pin in scripts/docs-test-pins.txt matches" in p for p in problems), problems)


class ReleasePins(unittest.TestCase):
    PINS = {f"{REL}/v0.4.0/SHA256SUMS.txt": SHA_A}

    def test_a_pinned_release_passes(self):
        self.assertEqual(dt.release_pin_problems({"VERSION": "0.4.0"}, self.PINS), [])

    def test_a_release_with_no_pins_is_named_with_the_fix(self):
        self.assertEqual(dt.release_pin_problems({"NEW_VERSION": "0.4.1"}, self.PINS),
                         ["NEW_VERSION=0.4.1 has no pins in scripts/docs-test-pins.txt; add its assets as the development page says under \"Pinned tool downloads\""])

    def test_a_binary_under_test_is_not_a_release(self):
        self.assertEqual(dt.release_pin_problems({"NEW": "/docs-test/vcon-server", "VERSION": "0.4.0"}, self.PINS), [])


class Upstream:
    """A local stand-in for github.com that answers each path from a list."""

    def __init__(self):
        import http.server
        import threading
        self.plans, self.hits = {}, []
        outer = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                outer.hits.append(self.path)
                plan = outer.plans.get(self.path, [(404, b"")])
                status, body = plan.pop(0) if len(plan) > 1 else plan[0]
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.base = f"http://127.0.0.1:{self.httpd.server_address[1]}"

    def stop(self):
        self.httpd.shutdown()
        self.httpd.server_close()


def get(url):
    import urllib.error
    import urllib.request
    try:
        with urllib.request.urlopen(url, timeout=30) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


ASSET = "/o/t/releases/download/v1/t.zip"
BYTES = b"the pinned asset\n" * 32


class Mirror(unittest.TestCase):
    def setUp(self):
        import hashlib
        self.cache = tempfile.mkdtemp(prefix="docs-test-mirror-")
        self.sha = hashlib.sha256(BYTES).hexdigest()
        self.calls = []
        self.rc = 0

    def tearDown(self):
        if getattr(self, "mirror", None):
            self.mirror.stop()
        shutil.rmtree(self.cache, ignore_errors=True)

    def fake_fetch(self, url, sha, dest):
        self.calls.append((url, sha, dest))
        if self.rc == 0:
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            with open(dest, "wb") as f:
                f.write(BYTES)
        return self.rc

    def start(self, fetch=None, upstream="https://github.com"):
        pins = {f"https://github.com{ASSET}": self.sha}
        self.mirror = dt.Mirror(pins, self.cache, host="127.0.0.1", upstream=upstream, fetch=fetch or self.fake_fetch)
        return self.mirror.start()

    def test_a_pinned_asset_is_fetched_from_its_github_url_and_served(self):
        base = self.start()
        self.assertEqual(get(base + ASSET), (200, BYTES))
        url, sha, dest = self.calls[0]
        self.assertEqual((url, sha), (f"https://github.com{ASSET}", self.sha))
        self.assertTrue(dest.startswith(self.cache + os.sep))

    def test_an_unpinned_asset_answers_404_naming_the_pin_file(self):
        base = self.start()
        code, body = get(base + "/o/t/releases/download/v1/other.zip")
        self.assertEqual(code, 404)
        self.assertIn(b"scripts/docs-test-pins.txt", body)
        self.assertEqual(self.calls, [])

    def test_a_permanent_refusal_upstream_answers_404(self):
        self.rc = 4
        self.assertEqual(get(self.start() + ASSET)[0], 404)

    def test_an_outage_upstream_answers_504(self):
        self.rc = 1
        self.assertEqual(get(self.start() + ASSET)[0], 504)

    def test_bytes_that_are_not_the_pin_answer_502(self):
        self.rc = 3
        self.assertEqual(get(self.start() + ASSET)[0], 502)

    def test_a_path_that_climbs_out_is_refused_without_a_fetch(self):
        base = self.start()
        import http.client
        c = http.client.HTTPConnection(base.removeprefix("http://"), timeout=10)
        c.request("GET", "/o/t/releases/download/../../../../etc/passwd")
        self.assertEqual(c.getresponse().status, 400)
        self.assertEqual(self.calls, [])

    def test_a_path_that_is_not_a_release_asset_is_refused_without_a_fetch(self):
        self.assertEqual(get(self.start() + "/o/t/blob/main/x")[0], 404)
        self.assertEqual(self.calls, [])

    def test_each_request_checks_the_cache_again(self):
        base = self.start()
        get(base + ASSET)
        get(base + ASSET)
        self.assertEqual(len(self.calls), 2)

    def test_through_fetch_pinned_an_outage_of_two_504s_is_ridden_out(self):
        up = Upstream()
        try:
            up.plans[ASSET] = [(504, b""), (504, b""), (200, BYTES)]
            fetch = dt.fetch_pinned_with(["--first-delay", "0.01", "--max-delay", "0.02", "--budget", "5"])
            base = self.start(fetch=fetch, upstream=up.base)
            self.assertEqual(get(base + ASSET), (200, BYTES))
            self.assertEqual(up.hits.count(ASSET), 3)
        finally:
            up.stop()

    def test_through_fetch_pinned_a_cached_asset_needs_no_upstream(self):
        up = Upstream()
        up.plans[ASSET] = [(200, BYTES)]
        fetch = dt.fetch_pinned_with(["--first-delay", "0.01", "--max-delay", "0.02", "--budget", "0.05"])
        base = self.start(fetch=fetch, upstream=up.base)
        self.assertEqual(get(base + ASSET)[0], 200)
        up.stop()
        self.assertEqual(get(base + ASSET), (200, BYTES))

    def test_through_fetch_pinned_substituted_bytes_answer_502(self):
        up = Upstream()
        try:
            up.plans[ASSET] = [(200, b"not it")]
            base = self.start(fetch=dt.fetch_pinned_with([]), upstream=up.base)
            self.assertEqual(get(base + ASSET)[0], 502)
        finally:
            up.stop()


class Bridge(unittest.TestCase):
    def test_the_gateway_docker_reports_is_the_mirror_host(self):
        self.assertEqual(dt.mirror_host("192.0.2.1\n"), "192.0.2.1")

    def test_no_gateway_is_an_error(self):
        with self.assertRaisesRegex(dt.PageError, "bridge network"):
            dt.mirror_host("\n")

    def test_a_gateway_that_is_not_an_address_is_an_error(self):
        with self.assertRaisesRegex(dt.PageError, "bridge network"):
            dt.mirror_host("<no value>\n")


class Images(unittest.TestCase):
    def test_a_scenario_image_resolves_to_its_digest(self):
        self.assertRegex(dt.pinned_image("debian:trixie-slim"), r"^debian:trixie-slim@sha256:[0-9a-f]{64}$")

    def test_an_image_with_no_pin_is_an_error(self):
        with self.assertRaisesRegex(dt.PageError, "no digest pinned for alpine:3"):
            dt.pinned_image("alpine:3")

    def test_every_scenario_image_is_pinned(self):
        for s in dt.SCENARIOS:
            if s.where not in ("host", "systemd"):
                dt.pinned_image(s.where)

    def test_the_systemd_image_builds_from_a_pinned_digest(self):
        self.assertIn(f"FROM {dt.pinned_image('debian:trixie-slim')}\n", dt.SYSTEMD_DOCKERFILE)

    def test_the_images_a_page_builds_from_are_named(self):
        blocks = dt.parse("Save it:\n\n<!-- docs-test: file Dockerfile -->\n```dockerfile\nFROM debian:trixie-slim AS download\nRUN x\nFROM download AS check\nFROM debian:trixie-slim\n```\n", "p.md")
        self.assertEqual(dt.base_images(blocks), ["debian:trixie-slim"])


class Pull(unittest.TestCase):
    def runner(self, fails):
        calls = []

        def run(*args, **kw):
            calls.append(args)
            ok = not (args[0] == "pull" and sum(1 for c in calls if c[0] == "pull") <= fails)
            return subprocess.CompletedProcess(args, 0 if ok else 1, "", "" if ok else "toomanyrequests")
        return run, calls

    def test_a_pull_that_fails_twice_then_works_is_retried_and_tagged(self):
        run, calls = self.runner(2)
        slept = []
        dt.pull_with_retry("debian:trixie-slim", run=run, sleep=slept.append)
        self.assertEqual([c[0] for c in calls], ["pull", "pull", "pull", "tag"])
        self.assertEqual(calls[-1], ("tag", dt.pinned_image("debian:trixie-slim"), "debian:trixie-slim"))
        self.assertEqual(slept, [5, 10])

    def test_a_pull_that_never_works_is_an_error_naming_the_image(self):
        run, calls = self.runner(99)
        with self.assertRaisesRegex(dt.PageError, r"debian:trixie-slim@sha256:\S+ could not be pulled after 8 attempts: toomanyrequests"):
            dt.pull_with_retry("debian:trixie-slim", run=run, sleep=lambda s: None)

    def test_a_first_pull_that_works_does_not_wait(self):
        run, calls = self.runner(0)
        slept = []
        dt.pull_with_retry("fedora:44", run=run, sleep=slept.append)
        self.assertEqual(slept, [])


class Prelude(unittest.TestCase):
    def scenario(self, where, tags):
        return dt.Scenario("x", where, frozenset(tags), ("install.md",), binary=False)

    def test_an_apt_scenario_in_a_container_retries_apt_downloads(self):
        self.assertIn('Acquire::Retries "10";', dt.runner_prelude(self.scenario("debian:trixie-slim", {"apt"})))

    def test_a_host_scenario_changes_nothing_on_the_host(self):
        self.assertEqual(dt.runner_prelude(self.scenario("host", {"docker"})), "")

    def test_a_dnf_scenario_gets_no_apt_setting(self):
        self.assertEqual(dt.runner_prelude(self.scenario("fedora:44", {"dnf"})), "")


if __name__ == "__main__":
    unittest.main()
