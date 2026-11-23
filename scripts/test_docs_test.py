#!/usr/bin/env python3
"""Tests of scripts/docs_test.py, the runner that holds the how-to pages to
a live server: how it reads a page, how it compares what a command printed
with what the page says it prints, the script it writes, and the rules it
holds every tested page to.

Run: python3 -I scripts/test_docs_test.py
"""

import os
import pathlib
import re
import shutil
import tempfile
import time
import subprocess
import sys
import contextlib
import io
import unittest
import unittest.mock

# No __pycache__ beside the scripts: nothing ignores it.
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import docs_test as dt  # noqa: E402


def stdin_of(data):
    """A standard input that holds `data` (text is written as UTF-8), with
    the byte stream under it that the commands read."""
    return io.TextIOWrapper(io.BytesIO(data.encode() if isinstance(data, str) else data), encoding="utf-8")

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


class RootOnlyDirectories(unittest.TestCase):
    """A directory one page makes for root alone, and a path under it that
    a page writes as the service account: found on 2026-11-10, when the
    upgrade page's `install -d -m 0700 /backup` left `vcon-server backup
    /backup/vcon-server/...` of the backup page refused."""

    ROOT_ONLY = "```sh\nsudo install -d -m 0700 /backup\n```\n"
    AS_VCON = "```sh\nsudo -u vcon vcon-server backup /backup/vcon-server/data-1 \\\n  --config $CONFIG\n```\n"

    def test_a_path_under_a_root_only_directory_written_as_vcon_is_found(self):
        problems = dt.root_only_directory_problems({"a.md": self.ROOT_ONLY, "b.md": self.AS_VCON})
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("a.md:2", problems[0])
        self.assertIn("b.md:2", problems[0])
        self.assertIn("/backup", problems[0])

    def test_it_is_found_on_one_page_too(self):
        self.assertEqual(len(dt.root_only_directory_problems({"a.md": self.ROOT_ONLY + self.AS_VCON})), 1)

    def test_a_variable_set_to_a_path_under_it_and_used_as_vcon_is_found(self):
        page = "```sh\nBACKUP=/backup/vcon-server/data-1\nsudo -u vcon vcon-server backup \"$BACKUP\" --config $CONFIG\n```\n"
        self.assertEqual(len(dt.root_only_directory_problems({"a.md": self.ROOT_ONLY, "b.md": page})), 1)

    def test_a_directory_made_for_vcon_is_not_root_only(self):
        made = "```sh\nsudo install -d -m 0700 -o vcon -g vcon /backup/vcon-server\n```\n"
        self.assertEqual(dt.root_only_directory_problems({"a.md": made, "b.md": self.AS_VCON}), [])

    def test_a_root_only_directory_beside_it_is_no_problem(self):
        beside = "```sh\nsudo install -d -m 0700 /backup/stopped\n```\n"
        self.assertEqual(dt.root_only_directory_problems({"a.md": beside, "b.md": self.AS_VCON}), [])

    def test_a_path_that_only_starts_with_the_same_letters_is_no_problem(self):
        other = "```sh\nsudo -u vcon vcon-server backup /backup2/data --config $CONFIG\n```\n"
        self.assertEqual(dt.root_only_directory_problems({"a.md": self.ROOT_ONLY, "b.md": other}), [])

    def test_a_command_run_as_root_under_it_is_no_problem(self):
        root = "```sh\nsudo tar -C /var/lib -cf /backup/copy.tar vcon-server\n```\n"
        self.assertEqual(dt.root_only_directory_problems({"a.md": self.ROOT_ONLY, "b.md": root}), [])

    def test_a_wider_mode_is_not_root_only(self):
        wide = "```sh\nsudo install -d -m 0755 /backup\n```\n"
        self.assertEqual(dt.root_only_directory_problems({"a.md": wide, "b.md": self.AS_VCON}), [])

    def test_text_outside_a_code_block_is_not_a_command(self):
        prose = "Do not run `sudo install -d -m 0700 /backup` here.\n"
        self.assertEqual(dt.root_only_directory_problems({"a.md": prose, "b.md": self.AS_VCON}), [])

    def test_the_pages_keep_the_rule(self):
        self.assertEqual(dt.root_only_directory_problems(dt.docs_texts()), [])

    def test_lint_reports_it(self):
        with unittest.mock.patch.object(dt, "docs_texts", lambda: {"a.md": self.ROOT_ONLY, "b.md": self.AS_VCON}):
            self.assertTrue(any("/backup" in p for p in dt.lint()))


class EveryGuideRuns(unittest.TestCase):
    """The how-to index says CI runs each guide. A guide is a page whose
    front matter names the group `start` or `how-to`; found on 2026-11-10:
    the PostgreSQL guide was in no scenario."""

    def test_the_group_reader_reads_the_front_matter(self):
        text = '+++\ntitle = "A"\nweight = 3\n\n[extra]\ngroup = "how-to"\n+++\n\ngroup = "start" in the body\n'
        self.assertEqual(dt.page_group(text), "how-to")
        self.assertEqual(dt.page_group("No front matter.\n"), "")
        in_the_body_only = '+++\ntitle = "A"\n+++\n\n```toml\n[extra]\ngroup = "how-to"\n```\n'
        self.assertEqual(dt.page_group(in_the_body_only), "", "a group a page only shows as an example is not its group")

    def test_a_guide_no_scenario_runs_is_named(self):
        texts = {"a.md": '+++\n[extra]\ngroup = "how-to"\n+++\n', "b.md": '+++\n[extra]\ngroup = "start"\n+++\n',
                 "c.md": '+++\n[extra]\ngroup = "reference"\n+++\n', "how-to.md": '+++\n[extra]\ngroup = "how-to"\n+++\n'}
        scenarios = [dt.Scenario("x", "debian:trixie-slim", frozenset(), ("a.md",))]
        self.assertEqual(dt.guides_not_run(texts, scenarios), ["b.md"])

    def test_every_guide_is_in_a_scenario(self):
        self.assertEqual(dt.guides_not_run(dt.docs_texts(), dt.SCENARIOS), [])

    def test_lint_reports_a_guide_no_scenario_runs(self):
        with unittest.mock.patch.object(dt, "guides_not_run", lambda texts, scenarios: ["new-guide.md"]):
            self.assertTrue(any("new-guide.md" in p for p in dt.lint()))


class Features(unittest.TestCase):
    """A scenario whose pages need a cargo feature the default build lacks
    names it, and CI gives that scenario a server built with it."""

    def by_name(self, name):
        return next(s for s in dt.SCENARIOS if s.name == name)

    def test_most_scenarios_need_no_feature(self):
        self.assertEqual(self.by_name("foreground").features, ())
        self.assertEqual(dt.artifact(self.by_name("foreground")), "docs-vcon-server")

    def test_the_postgres_scenario_needs_the_postgres_feature(self):
        s = self.by_name("postgres")
        self.assertEqual(s.features, ("postgres",))
        self.assertEqual(dt.artifact(s), "docs-vcon-server-postgres")

    def test_it_runs_the_guide_after_the_bucket_it_needs(self):
        s = self.by_name("postgres")
        self.assertEqual(s.pages, ("getting-started.md", "s3-storage.md", "postgres-catalog.md"))
        self.assertIn("rustfs", s.tags)
        self.assertEqual(s.where, "systemd")

    def test_the_artifact_command_prints_the_name(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(dt.main(["artifact", "postgres"]), 0)
        self.assertEqual(out.getvalue(), "docs-vcon-server-postgres\n")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(dt.main(["artifact", "systemd"]), 0)
        self.assertEqual(out.getvalue(), "docs-vcon-server\n")

    def test_the_artifact_command_refuses_a_name_that_is_no_scenario(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(dt.main(["artifact", "nope"]), 2)
        self.assertIn("nope", err.getvalue())

    def test_ci_builds_and_keeps_a_server_for_every_feature_set(self):
        with open(os.path.join(dt.ROOT, ".github", "workflows", "ci.yml"), encoding="utf-8") as f:
            workflow = f.read()
        for s in dt.SCENARIOS:
            name = dt.artifact(s)
            self.assertIn(f"name: {name}\n", workflow, f"ci.yml keeps no artifact {name}")
            if s.features:
                build = f"cargo build --locked -p vcon-server --features {','.join(s.features)}"
                self.assertIn(build, workflow, f"ci.yml does not run: {build}")
        self.assertIn('python3 -I scripts/docs_test.py artifact "$SCENARIO"', workflow,
                      "ci.yml does not ask the runner which server a scenario gets")


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

    # A spent budget: curl prints the answer's x-ratelimit-remaining and
    # x-ratelimit-reset before the status (scripts/gh_rate_limit.py's rule).
    SPENT = '{"message":"API rate limit exceeded for installation."}\n0 1000060 403'

    def answers(self, *stdouts):
        calls = []

        def run(cmd, **kw):
            calls.append(cmd)
            out = stdouts[min(len(calls), len(stdouts)) - 1]
            return subprocess.CompletedProcess(cmd, 0, out, "")

        return run, calls

    def test_curl_is_asked_for_the_rate_limit_headers(self):
        run, calls = self.answers("{}\n4999 1000060 200")
        self.assertEqual(dt.github_api("repos/o/r/releases/latest", None, run), (200, "{}"), "three fields on the last line")
        self.assertIn("%header{x-ratelimit-remaining}", " ".join(calls[0]), "curl prints the budget left")

    def test_a_spent_budget_is_waited_for_and_asked_again(self):
        run, calls = self.answers(self.SPENT, '{"tag_name":"v0.4.11"}\n4999 1003600 200')
        slept = []
        got = dt.github_api("repos/o/r/releases/latest", "t", run, clock=lambda: 1_000_000, sleep=slept.append, err=io.StringIO())
        self.assertEqual(got, (200, '{"tag_name":"v0.4.11"}'), "the answer after the reset")
        self.assertEqual(slept, [65], "to the reset plus 5 s")
        self.assertEqual(len(calls), 2, "asked twice")

    def test_a_refusal_with_budget_left_is_not_waited_for(self):
        run, calls = self.answers('{"message":"Resource not accessible"}\n12 1000060 403')
        slept = []
        self.assertEqual(dt.github_api("repos/o/r/x", "t", run, clock=lambda: 1_000_000, sleep=slept.append, err=io.StringIO())[0], 403, "returned")
        self.assertEqual((slept, len(calls)), ([], 1), "no wait, one request")

    def test_a_reset_too_far_off_is_not_waited_for(self):
        run, calls = self.answers('{"message":"API rate limit exceeded"}\n0 1009000 403')
        slept = []
        self.assertEqual(dt.github_api("repos/o/r/x", "t", run, clock=lambda: 1_000_000, sleep=slept.append, err=io.StringIO())[0], 403, "returned")
        self.assertEqual((slept, len(calls)), ([], 1), "no wait past 900 s")


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
# A step that waits for one log line and then shows the log. On 2026-11-15
# main's "how-to parquet" job failed at getting-started.md: the step waited
# for `serving` and showed the purge sweep's line after it. The server now
# logs `serving` before any background job starts, so the sweep's line comes
# after it, and may not be written yet when `serving` is: a step must wait
# for the last line it shows.
# ---------------------------------------------------------------------------

SERVING = "<time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443"
SWEEP = "<time>  INFO vcon_server::serve: purge sweep purged=0 failed=0"
OPERATOR = "<time>  INFO vcon_server::serve: the operator listener is bound operator_listen=127.0.0.1:8444"


def waiting_step(wait: str, *shown: str) -> str:
    """A page with one step that waits with `wait` and shows `shown`."""
    return (
        "```sh\n"
        f"timeout 30 sh -c 'until sudo journalctl -u vcon-server -o cat | {wait}; do sleep 1; done'\n"
        "sudo journalctl -u vcon-server -o cat | grep vcon_server\n"
        "```\n\n```text\n" + "".join(f"{l}\n" for l in shown) + "```\n"
    )


def wait_problem(at: str, word: str) -> str:
    return (
        f"{at}: the step waits for {word!r} and shows lines after that one, which may not be "
        "written yet; wait for a word of the last line it shows"
    )


def lint_of(page: str) -> list[str]:
    return dt.lint_page(dt.parse(page, "p.md"), [frozenset()])


class WaitsForTheLastLine(unittest.TestCase):
    # Success: what the rule leaves alone.
    def test_a_wait_for_the_one_line_shown_is_clean(self):
        self.assertEqual(lint_of(waiting_step("grep -q serving", SERVING)), [], "one line, waited for")

    def test_a_wait_for_the_last_of_two_lines_is_clean(self):
        page = waiting_step("grep -q 'purge sweep'", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [], "the last line is the one waited for")

    def test_a_wait_for_the_line_before_an_ellipsis_is_clean(self):
        page = waiting_step("grep -q serving", SERVING, "…")
        self.assertEqual(lint_of(page), [], "an ellipsis may stand for no line")

    def test_a_wait_for_a_word_the_text_does_not_show_is_clean(self):
        page = waiting_step("grep -q answering", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [], "the rule is about lines the text shows")

    def test_a_step_with_no_wait_is_clean(self):
        page = "```sh\nsudo journalctl -u vcon-server -o cat | grep vcon_server\n```\n\n```text\n"
        page += f"{SERVING}\n{SWEEP}\n```\n"
        self.assertEqual(lint_of(page), [], "no until, no wait")

    def test_a_wait_with_no_text_shown_is_clean(self):
        page = "```sh\ntimeout 30 sh -c 'until grep -q serving out; do sleep 1; done'\n```\n"
        self.assertEqual(lint_of(page), [], "nothing shown after it")

    def test_a_double_quoted_wait_for_the_last_line_is_clean(self):
        page = (
            "```sh\n"
            'timeout 30 sh -c "until docker logs vcon-server 2>&1 | grep -q \\"purge sweep\\"; do sleep 1; done"\n'
            "docker logs vcon-server 2>&1 | grep vcon_server\n"
            f"```\n\n```text\n{SERVING}\n{SWEEP}\n```\n"
        )
        self.assertEqual(lint_of(page), [], "the last line holds the quoted words")

    def test_a_docker_logs_wait_for_the_last_line_is_clean(self):
        page = (
            "```sh\n"
            "timeout 30 sh -c 'until docker logs vcon-server 2>&1 | grep -q \"purge sweep\"; do sleep 1; done'\n"
            "docker logs vcon-server 2>&1 | grep vcon_server\n"
            f"```\n\n```text\n{SERVING}\n{SWEEP}\n```\n"
        )
        self.assertEqual(lint_of(page), [], "the container page's shape, waiting for its last line")

    # Failure: a wait for a line that a later shown line follows.
    def test_a_wait_for_the_first_of_two_lines_is_found(self):
        page = waiting_step("grep -q serving", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "the sweep line may lag")

    def test_a_fixed_string_wait_is_found(self):
        page = waiting_step("grep -qF serving", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "-F is still a wait")

    def test_a_wait_after_a_double_dash_is_found(self):
        page = waiting_step("grep -q -- serving", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "-- ends the options")

    def test_a_quoted_wait_of_two_words_is_found(self):
        page = waiting_step("grep -q \"serving listen\"", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving listen")], "quoted words")

    def test_an_escaped_double_quoted_wait_is_found(self):
        page = (
            "```sh\n"
            'timeout 30 sh -c "until docker logs vcon-server 2>&1 | grep -q \\"serving listen\\"; do sleep 1; done"\n'
            "docker logs vcon-server 2>&1 | grep vcon_server\n"
            f"```\n\n```text\n{SERVING}\n{SWEEP}\n```\n"
        )
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving listen")], "quotes inside sh -c \"...\"")

    def test_a_wait_with_grouped_options_is_found(self):
        page = waiting_step("grep -Eq serving", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "-Eq is -E and -q")

    def test_a_wait_for_the_first_of_three_lines_is_found(self):
        page = waiting_step("grep -q serving", SERVING, SWEEP, "<time>  INFO vcon_server::serve: stopped")
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "two lines may lag")

    def test_a_wait_before_an_ellipsis_and_a_later_line_is_found(self):
        page = waiting_step("grep -q serving", SERVING, "…", SWEEP)
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "the line after the ellipsis lags")

    def test_the_last_of_two_waits_decides(self):
        page = (
            "```sh\n"
            "timeout 30 sh -c 'until grep -q \"purge sweep\" out; do sleep 1; done'\n"
            "timeout 30 sh -c 'until grep -q serving out; do sleep 1; done'\n"
            f"grep vcon_server out\n```\n\n```text\n{SERVING}\n{SWEEP}\n```\n"
        )
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "the wait just before the read")

    def test_the_getting_started_step_as_it_was_is_found(self):
        page = (
            "<!-- docs-test: only systemd -->\n```sh\n"
            "timeout 30 sh -c 'until sudo journalctl -u vcon-server -o cat | grep -q serving; do sleep 1; done'\n"
            "sudo journalctl -u vcon-server -o cat | grep vcon_server\n"
            f"```\n\n```text\n{SERVING}\n{SWEEP}\n```\n"
        )
        problems = dt.lint_page(dt.parse(page, "getting-started.md"), [frozenset({"systemd"})])
        self.assertEqual(problems, [wait_problem("getting-started.md:2", "serving")], "main's red step")

    def test_the_container_step_as_it_was_is_found(self):
        page = (
            "```sh\n"
            "timeout 30 sh -c 'until docker logs vcon-server 2>&1 | grep -q serving; do sleep 1; done'\n"
            "docker logs vcon-server 2>&1 | grep vcon_server\n"
            f"```\n\n```text\n{SERVING}\n{SWEEP}\n```\n"
        )
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "container.md's step")

    def test_a_wait_without_q_is_found(self):
        page = (
            "```sh\ntimeout 30 sh -c 'until grep serving out >/dev/null; do sleep 1; done'\n"
            f"grep vcon_server out\n```\n\n```text\n{SERVING}\n{SWEEP}\n```\n"
        )
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "serving")], "a grep in until waits")

    # Edge cases.
    def test_a_word_in_an_earlier_line_and_in_the_last_is_clean(self):
        page = waiting_step("grep -q vcon_server", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [], "the last line holds the word too")

    def test_ellipses_after_the_last_line_waited_for_are_clean(self):
        page = waiting_step("grep -q 'purge sweep'", SERVING, "…", SWEEP, "…")
        self.assertEqual(lint_of(page), [], "the last line that is not an ellipsis")

    def test_a_text_of_only_an_ellipsis_is_clean(self):
        page = waiting_step("grep -q serving", "…")
        self.assertEqual(lint_of(page), [], "no line shown")

    def test_a_word_with_pattern_characters_is_read_as_text(self):
        page = waiting_step("grep -qF listen=0.0.0.0", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [wait_problem("p.md:1", "listen=0.0.0.0")], "dots are text")

    def test_a_wait_with_no_grep_is_clean(self):
        page = (
            "```sh\ntimeout 30 sh -c 'until curl -s -o /dev/null https://example.com/health; do sleep 1; done'"
            " && echo answering\n```\n\n```text\nanswering\n```\n"
        )
        self.assertEqual(lint_of(page), [], "a loop on a request, not on a line")

    def test_a_grep_outside_the_wait_is_not_a_wait(self):
        page = "```sh\nsudo journalctl -o cat | grep -q serving && journalctl -o cat\n```\n\n```text\n"
        page += f"{SERVING}\n{SWEEP}\n```\n"
        self.assertEqual(lint_of(page), [], "only a grep inside until waits")

    def test_a_skipped_step_is_not_read(self):
        page = "<!-- docs-test: skip a reader's own log -->\n" + waiting_step("grep -q serving", SERVING, SWEEP)
        self.assertEqual(lint_of(page), [], "skipped steps are not run")

    def test_a_background_step_is_compared_after_the_run(self):
        page = (
            "<!-- docs-test: background serving -->\n```sh\nvcon-server serve\n```\n\n"
            f"```text\n{SERVING}\n{SWEEP}\n…\n<time>  INFO vcon_server::serve: stopped\n```\n"
        )
        self.assertEqual(lint_of(page), [], "its marker waits, and its text is read at the end")

    def test_an_indented_step_in_a_list_is_found(self):
        page = (
            "1. Start it:\n\n   ```sh\n"
            "   timeout 30 sh -c 'until grep -q serving out; do sleep 1; done'\n   grep vcon_server out\n"
            f"   ```\n\n   ```text\n   {SERVING}\n   {SWEEP}\n   ```\n"
        )
        self.assertEqual(lint_of(page), [wait_problem("p.md:3", "serving")], "a step in a list item")

    def test_the_waited_word_is_read_from_the_wait(self):
        self.assertEqual(dt.waited_for("until grep -q 'purge sweep' out; do"), "purge sweep", "quoted")
        self.assertEqual(dt.waited_for("until grep -qF -- serving out; do"), "serving", "options and --")
        self.assertEqual(dt.waited_for("journalctl | grep -q serving"), None, "not inside until")


# ---------------------------------------------------------------------------
# A step that waits for a pack run by watching its objects leave the bucket.
# On 2026-11-17 main's "how-to s3" job (run 44188522706) failed at
# parquet-storage.md:206, :221 and :326: the wait ended when no object was
# left, but a pack run removes the objects of each file it records before it
# merges the date's small files, writes the manifest and logs `pack run`
# (Store::pack, serve.rs pack_all). The listing missed the merged file, the
# log's last line was the run before, and the purge checked 2 files, not 3.
# A local run with an 8 s delay between the two showed all three each time.
# ---------------------------------------------------------------------------

LIST_OBJECTS = 'curl -sS --fail-with-body "${S3[@]}" "$LIST"'

OBJECTS_WAIT = (
    "for i in $(seq 30); do\n"
    f"  {LIST_OBJECTS} | grep -q '/objects/' || break\n"
    "  sleep 2\n"
    "done\n"
)

# parquet-storage.md's two waits as d09bccd had them, with their text.
OLD_PARQUET_WAITS = (
    "The pack run at start packs the objects.\n\n"
    "<!-- docs-test: only rustfs -->\n```sh\n" + OBJECTS_WAIT
    + 'BEFORE=$(curl -sS "${S3[@]}" "$FILES" | grep -o \'<Key>[^<]*\' | cut -c6-)\necho "$BEFORE"\n```\n\n'
    "```text\ntenants/<tenant id>/parquet/date=<yyyy-mm-dd>/part-<file id>.parquet\n```\n\n"
    "Wait for the next run:\n\n"
    "<!-- docs-test: only rustfs -->\n```sh\n" + OBJECTS_WAIT
    + 'AFTER=$(curl -sS "${S3[@]}" "$FILES" | grep -o \'<Key>[^<]*\' | cut -c6-)\necho "$AFTER" | wc -l\n```\n\n'
    "```text\n3\n```\n"
)


def objects_wait_problem(at: str) -> str:
    return (
        f"{at}: the step waits for the objects to leave the bucket, which a pack run does before "
        "it merges files, writes the manifest and logs `pack run`; wait for that run's `pack run` line"
    )


def step(code: str, shown: str | None = "3", marker: str = "") -> str:
    """A page with one sh step running `code`, and `shown` after it."""
    page = (marker + "\n" if marker else "") + "```sh\n" + code + "```\n"
    if shown is not None:
        page += "\n```text\n" + shown + "\n```\n"
    return page


def rustfs_lint(page: str) -> list[str]:
    return dt.lint_page(dt.parse(page, "p.md"), [frozenset({"rustfs", "debian"})])


class WaitsForThePackRunNotItsObjects(unittest.TestCase):
    # Success: waits on the run's own line, and other loops, are left alone.
    def test_a_wait_for_the_pack_run_line_is_clean(self):
        code = "timeout 60 sh -c 'until sudo journalctl -u vcon-server -o cat | grep -q \"pack run\"; do sleep 1; done'\n"
        self.assertEqual(rustfs_lint(step(code, None)), [], "the run's own line is the right signal")

    def test_a_count_of_pack_run_lines_is_clean(self):
        code = (
            "for i in $(seq 30); do\n"
            '  [ "$(sudo journalctl -o cat | grep \'pack run\' | grep -c -v \'versions=0\')" -ge 2 ] && break\n'
            "  sleep 2\ndone\n"
        )
        self.assertEqual(rustfs_lint(step(code)), [], "the page's new shape waits on the log")

    def test_a_one_time_count_of_objects_is_clean(self):
        code = f"{LIST_OBJECTS} | grep -c '/objects/'\n"
        self.assertEqual(rustfs_lint(step(code, "0")), [], "a count outside a loop is not a wait")

    def test_a_loop_over_keys_that_greps_a_uuid_is_clean(self):
        code = (
            "for key in $AFTER; do\n"
            '  if curl -sS "${S3[@]}" "https://objects.example.com:9000/vcon/$key" | grep -q -a "$UUID"; then\n'
            '    echo "$key"\n  fi\ndone\n'
        )
        self.assertEqual(rustfs_lint(step(code, "none")), [], "the purge page's check of each file")

    def test_a_loop_that_lists_the_parquet_keys_is_clean(self):
        code = "for i in 1 2; do\n  curl -sS \"$FILES\" | grep -q '/parquet/' && break\n  sleep 1\ndone\n"
        self.assertEqual(rustfs_lint(step(code)), [], "files, not objects")

    def test_a_while_loop_reading_lines_is_clean(self):
        code = "while read -r key; do\n  echo \"$key\"\ndone < keys.txt\n"
        self.assertEqual(rustfs_lint(step(code)), [], "no grep at all")

    def test_a_step_with_no_loop_is_clean(self):
        self.assertEqual(rustfs_lint(step("echo 3\n")), [], "nothing waits")

    def test_the_health_wait_is_clean(self):
        code = (
            "timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem "
            "https://vcon.example.com:8443/health; do sleep 1; done' && echo answering\n"
        )
        self.assertEqual(rustfs_lint(step(code, "answering")), [], "the pages' usual wait")

    def test_a_text_block_that_names_objects_is_clean(self):
        page = "```text\nfor i in 1; do grep -q '/objects/'; done\n```\n"
        self.assertEqual(rustfs_lint(page), [], "a text block is output, not a step")

    def test_prose_about_the_objects_is_clean(self):
        page = "Wait until no `/objects/` key is left: for i in 1; do grep /objects/; done.\n"
        self.assertEqual(rustfs_lint(page), [], "prose is not a step")

    def test_a_skipped_step_is_clean(self):
        page = step(OBJECTS_WAIT, marker="<!-- docs-test: skip the reader lists the bucket by hand -->")
        self.assertEqual(rustfs_lint(page), [], "a skipped step never runs")

    def test_a_file_block_that_names_objects_is_clean(self):
        page = "<!-- docs-test: file notes.toml -->\n```toml\nwait = \"for i in 1; do grep -q '/objects/'; done\"\n```\n"
        self.assertEqual(rustfs_lint(page), [], "a file the page writes is not run as a step")

    def test_a_head_loop_over_the_files_is_clean(self):
        code = (
            "for key in $AFTER; do\n"
            '  curl -sS -o /dev/null -w \'%{http_code}\\n\' "${S3[@]}" -I "https://objects.example.com:9000/vcon/$key"\n'
            "done\n"
        )
        self.assertEqual(rustfs_lint(step(code, "404")), [], "the purge step's HEAD of each file")

    def test_a_docker_logs_pack_run_wait_is_clean(self):
        code = "timeout 60 sh -c 'until docker logs vcon-server 2>&1 | grep -q \"pack run\"; do sleep 1; done'\n"
        self.assertEqual(rustfs_lint(step(code, None)), [], "the container's log is the signal")

    def test_an_objects_grep_only_in_a_comment_is_clean(self):
        code = "for i in $(seq 30); do\n  # grep -q '/objects/' would end mid-run\n  sleep 2\ndone\n"
        self.assertEqual(rustfs_lint(step(code)), [], "a comment runs nothing")

    def test_the_parquet_page_keeps_the_rule(self):
        problems = dt.lint_page(dt.page_blocks("parquet-storage.md"), [frozenset({"debian", "apt", "systemd", "rustfs"})])
        self.assertEqual([p for p in problems if "leave the bucket" in p], [], "the page waits on its pack runs' lines")

    # Failure: every shape of a wait on the objects is found.
    def test_the_old_parquet_waits_are_both_found(self):
        self.assertEqual(
            rustfs_lint(OLD_PARQUET_WAITS),
            [objects_wait_problem("p.md:4"), objects_wait_problem("p.md:20")],
            "d09bccd's two waits, the second of which failed main",
        )

    def test_a_single_quoted_objects_wait_is_found(self):
        self.assertEqual(rustfs_lint(step(OBJECTS_WAIT)), [objects_wait_problem("p.md:1")], "the page's shape")

    def test_a_double_quoted_objects_wait_is_found(self):
        code = OBJECTS_WAIT.replace("'/objects/'", '"/objects/"')
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "double quotes")

    def test_an_unquoted_objects_wait_is_found(self):
        code = OBJECTS_WAIT.replace("'/objects/'", "/objects/")
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "no quotes")

    def test_an_until_not_objects_wait_is_found(self):
        code = f"until ! {LIST_OBJECTS} | grep -q '/objects/'; do sleep 2; done\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "until !")

    def test_a_while_objects_wait_is_found(self):
        code = f"while {LIST_OBJECTS} | grep -q '/objects/'; do sleep 2; done\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "while")

    def test_a_timeout_sh_c_objects_wait_is_found(self):
        code = "timeout 60 sh -c 'until ! curl -s \"$U\" | grep -q /objects/; do sleep 1; done'\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "the pages' timeout shape")

    def test_a_fixed_string_objects_wait_is_found(self):
        code = OBJECTS_WAIT.replace("grep -q", "grep -qF")
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "-F is still a wait")

    def test_a_count_of_objects_in_a_loop_is_found(self):
        code = (
            "for i in $(seq 30); do\n"
            f"  [ \"$({LIST_OBJECTS} | grep -c '/objects/')\" = 0 ] && break\n"
            "  sleep 2\ndone\n"
        )
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "a count of 0 is the same end")

    def test_a_tenant_objects_prefix_wait_is_found(self):
        code = OBJECTS_WAIT.replace("'/objects/'", "\"tenants/$TENANT/objects/\"")
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "one tenant's prefix")

    def test_an_aws_cli_objects_wait_is_found(self):
        code = "until [ -z \"$(aws s3 ls s3://vcon/tenants/ --recursive | grep /objects/)\" ]; do sleep 2; done\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "another listing tool")

    def test_an_mc_objects_wait_is_found(self):
        code = "while mc ls --recursive local/vcon/tenants/ | grep -q objects/; do sleep 2; done\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "MinIO's client")

    def test_a_one_line_objects_wait_is_found(self):
        code = f"for i in $(seq 30); do {LIST_OBJECTS} | grep -q '/objects/' || break; sleep 2; done\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "on one line")

    def test_an_indented_objects_wait_in_a_list_is_found(self):
        body = "".join(f"   {l}\n" for l in OBJECTS_WAIT.splitlines())
        page = "1. Wait:\n\n   ```sh\n" + body + "   ```\n\n   ```text\n   3\n   ```\n"
        self.assertEqual(rustfs_lint(page), [objects_wait_problem("p.md:3")], "a step in a list item")

    def test_an_objects_wait_with_no_text_shown_is_found(self):
        self.assertEqual(rustfs_lint(step(OBJECTS_WAIT, None)), [objects_wait_problem("p.md:1")], "no output")

    def test_an_objects_wait_after_a_good_wait_is_found(self):
        code = "timeout 60 sh -c 'until sudo journalctl -o cat | grep -q \"pack run\"; do sleep 1; done'\n" + OBJECTS_WAIT
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "the bad wait still ends early")

    def test_an_extended_pattern_objects_wait_is_found(self):
        code = OBJECTS_WAIT.replace("grep -q '/objects/'", "grep -qE '/(objects)/'")
        self.assertEqual(rustfs_lint(step(code)), [], "a pattern the rule cannot read is not claimed")
        code = OBJECTS_WAIT.replace("grep -q", "grep -qE")
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "-E with a plain pattern")

    def test_an_objects_wait_under_an_only_marker_is_found(self):
        page = step(OBJECTS_WAIT, marker="<!-- docs-test: only rustfs -->")
        self.assertEqual(rustfs_lint(page), [objects_wait_problem("p.md:2")], "the marker moves the fence")

    def test_an_objects_wait_after_a_double_dash_is_found(self):
        code = OBJECTS_WAIT.replace("grep -q", "grep -q --")
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "-- ends the options")

    # Edge cases.
    def test_two_objects_waits_in_one_step_are_one_problem(self):
        self.assertEqual(rustfs_lint(step(OBJECTS_WAIT + OBJECTS_WAIT)), [objects_wait_problem("p.md:1")], "one per step")

    def test_two_steps_are_two_problems_in_page_order(self):
        page = step(OBJECTS_WAIT) + "\n" + step(OBJECTS_WAIT)
        self.assertEqual(
            rustfs_lint(page), [objects_wait_problem("p.md:1"), objects_wait_problem("p.md:12")], "each step, in order"
        )

    def test_the_problem_says_what_to_wait_for(self):
        (problem,) = rustfs_lint(step(OBJECTS_WAIT))
        self.assertIn("`pack run` line", problem, "it names the run's own line")
        self.assertIn("before it merges files, writes the manifest", problem, "and why the objects end too early")

    def test_objects_without_a_slash_are_not_claimed(self):
        code = OBJECTS_WAIT.replace("'/objects/'", "objects")
        self.assertEqual(rustfs_lint(step(code)), [], "the rule reads a key prefix, objects/")

    def test_a_grep_after_the_loop_is_not_in_it(self):
        code = f"for i in 1 2; do\n  sleep 1\ndone\n{LIST_OBJECTS} | grep -c '/objects/'\n"
        self.assertEqual(rustfs_lint(step(code, "0")), [], "the loop ends at its done")

    def test_a_grep_between_two_loops_is_in_neither(self):
        code = (
            f"for i in 1 2; do\n  sleep 1\ndone\n{LIST_OBJECTS} | grep -c '/objects/'\n"
            "for key in $AFTER; do\n  echo \"$key\"\ndone\n"
        )
        self.assertEqual(rustfs_lint(step(code, "0")), [], "each loop ends at its own done")

    def test_a_later_command_of_the_pipe_naming_objects_is_clean(self):
        code = "for k in $KEYS; do\n  curl -sS \"$k\" | grep -a \"$UUID\" | tee \"objects/$k.txt\"\ndone\n"
        self.assertEqual(rustfs_lint(step(code, "none")), [], "grep's pattern ends at the pipe")

    def test_a_nested_loop_objects_wait_is_found(self):
        code = "for t in a b; do\n" + "".join(f"  {l}\n" for l in OBJECTS_WAIT.splitlines()) + "done\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "a loop inside a loop")

    def test_a_continued_line_objects_wait_is_found(self):
        code = "for i in $(seq 30); do\n  curl -sS \"$LIST\" \\\n    | grep -q '/objects/' || break\n  sleep 2\ndone\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "a backslash continues the pipe")
        code = "for i in $(seq 30); do\n  curl -sS \"$LIST\" | grep -q \\\n    '/objects/' || break\n  sleep 2\ndone\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "the pattern on the next line")

    def test_a_trailing_comment_naming_objects_is_clean(self):
        code = "for i in $(seq 30); do\n  sleep 2  # grep -q /objects/ ends mid-run\ndone\n"
        self.assertEqual(rustfs_lint(step(code)), [], "a trailing comment runs nothing")

    def test_objects_in_the_url_not_the_pattern_is_clean(self):
        code = "for k in $KEYS; do\n  curl -sS \"https://objects.example.com:9000/vcon/tenants/t/objects/$k\" | grep -q \"$UUID\"\ndone\n"
        self.assertEqual(rustfs_lint(step(code, "none")), [], "objects/ in a URL is not what grep waits for")

    def test_a_pattern_in_a_variable_is_not_claimed(self):
        code = OBJECTS_WAIT.replace("'/objects/'", '"$PREFIX"')
        self.assertEqual(rustfs_lint(step(code)), [], "a pattern the rule cannot read is not claimed")

    def test_an_objects_wait_in_a_background_step_is_found(self):
        page = step(OBJECTS_WAIT, None, "<!-- docs-test: background packed -->")
        self.assertEqual(rustfs_lint(page), [objects_wait_problem("p.md:2")], "a background step waits too")

    def test_an_objects_wait_in_an_exit_1_step_is_found(self):
        page = step(OBJECTS_WAIT, None, "<!-- docs-test: exit 1 -->")
        self.assertEqual(rustfs_lint(page), [objects_wait_problem("p.md:2")], "an expected failure waits too")

    def test_undone_does_not_end_the_loop(self):
        code = "for i in $(seq 30); do\n  echo undone\n" + f"  {LIST_OBJECTS} | grep -q '/objects/' || break\ndone\n"
        self.assertEqual(rustfs_lint(step(code)), [objects_wait_problem("p.md:1")], "done is a word of its own")

    def test_the_predicate_reads_the_code(self):
        self.assertTrue(dt.waits_for_objects(OBJECTS_WAIT), "the page's old wait")
        self.assertFalse(dt.waits_for_objects("grep -q '/objects/' out\n"), "no loop")
        self.assertFalse(dt.waits_for_objects("for i in 1; do grep -q x; done\n"), "no objects")

    def test_lint_reports_the_old_parquet_waits(self):
        old = dt.parse(OLD_PARQUET_WAITS, "docs/parquet-storage.md")
        real = dt.page_blocks
        with unittest.mock.patch.object(dt, "page_blocks", lambda name: old if name == "parquet-storage.md" else real(name)):
            problems = [p for p in dt.lint() if "leave the bucket" in p]
        self.assertEqual(
            problems,
            [objects_wait_problem("docs/parquet-storage.md:4"), objects_wait_problem("docs/parquet-storage.md:20")],
            "lint holds every page a scenario runs to the rule",
        )

    def test_the_parquet_page_counts_its_pack_runs(self):
        waits = [b for b in dt.page_blocks("parquet-storage.md") if b.kind == "sh" and "packed_runs" in b.code]
        counts = [m for b in waits for m in re.findall(r'\[ "\$\(packed_runs\)" -ge (\d) \]', b.code)]
        self.assertEqual(counts, ["1", "2"], "the start's run, then the run that packs the new vCon")


def order_problem(at: str, line: str) -> str:
    return (
        f"{at}: the text shows {line!r} before the `serving` line; the server logs `serving` "
        "before any background job starts"
    )


def shown(*lines: str) -> str:
    """A page with a step that shows `lines`."""
    return "```sh\ngrep vcon_server out\n```\n\n```text\n" + "".join(f"{l}\n" for l in lines) + "```\n"


class ServeStartOrder(unittest.TestCase):
    # Success.
    def test_serving_then_the_sweep_is_clean(self):
        self.assertEqual(lint_of(shown(SERVING, SWEEP)), [], "the server's order")

    def test_the_operator_line_serving_and_the_sweep_are_clean(self):
        self.assertEqual(lint_of(shown(OPERATOR, SERVING, SWEEP)), [], "the operator line first")

    def test_serving_alone_is_clean(self):
        self.assertEqual(lint_of(shown(SERVING, "…")), [], "one start line")

    def test_a_background_line_with_no_serving_line_is_clean(self):
        line = "<time>  INFO vcon_server::serve: pack run files=1 versions=1 failed=0"
        self.assertEqual(lint_of(shown(line)), [], "a later read of the log")

    def test_a_request_line_before_serving_is_clean(self):
        request = "<time>  INFO vcon_server::api: request method=GET route=/health status=401"
        self.assertEqual(lint_of(shown(request, SERVING)), [], "not a background job's line")

    # Failure.
    def test_the_sweep_before_serving_is_found(self):
        self.assertEqual(lint_of(shown(SWEEP, SERVING)), [order_problem("p.md:1", SWEEP)], "the old race")

    def test_a_pack_run_before_serving_is_found(self):
        line = "<time>  INFO vcon_server::serve: pack run files=0 versions=0 failed=0"
        self.assertEqual(lint_of(shown(line, SERVING)), [order_problem("p.md:1", line)], "the packer")

    def test_a_rotation_warning_before_serving_is_found(self):
        line = "<time>  WARN vcon_server::serve: key rotation overdue: purged vCons stay readable"
        self.assertEqual(lint_of(shown(line, SERVING)), [order_problem("p.md:1", line)], "the warner")

    def test_an_unread_backlog_before_serving_is_found(self):
        line = "<time>  WARN vcon_server::serve: the key rotation backlog could not be read"
        self.assertEqual(lint_of(shown(line, SERVING)), [order_problem("p.md:1", line)], "the warner")

    def test_the_sweep_between_the_operator_line_and_serving_is_found(self):
        self.assertEqual(
            lint_of(shown(OPERATOR, SWEEP, SERVING)), [order_problem("p.md:1", SWEEP)], "between the two"
        )

    def test_each_background_line_before_serving_is_found(self):
        pack = "<time>  INFO vcon_server::serve: pack run files=0 versions=0 failed=0"
        self.assertEqual(
            lint_of(shown(SWEEP, pack, SERVING)),
            [order_problem("p.md:1", SWEEP), order_problem("p.md:1", pack)],
            "one problem per line",
        )

    # Edge cases.
    def test_a_background_line_after_an_ellipsis_before_serving_is_found(self):
        self.assertEqual(lint_of(shown(SWEEP, "…", SERVING)), [order_problem("p.md:1", SWEEP)], "still before")

    def test_an_indented_text_is_read(self):
        page = (
            "1. Start it:\n\n   ```sh\n   grep vcon_server out\n   ```\n\n"
            f"   ```text\n   {SWEEP}\n   {SERVING}\n   ```\n"
        )
        self.assertEqual(lint_of(page), [order_problem("p.md:3", SWEEP)], "a step in a list item")

    def test_a_second_start_after_a_stop_is_read_on_its_own(self):
        stopped = "<time>  INFO vcon_server::serve: stopped"
        page = shown(SERVING, SWEEP, stopped, SERVING, SWEEP)
        self.assertEqual(lint_of(page), [], "the sweep of the first start comes before the second serving")

    def test_a_sweep_named_in_another_module_is_not_a_background_line(self):
        line = "<time>  INFO vcon_server::store: purge sweep of one tenant"
        self.assertEqual(lint_of(shown(line, SERVING)), [], "only vcon_server::serve lines")

    def test_a_restart_with_no_stopped_line_is_clean(self):
        page = shown(SERVING, SWEEP, SERVING, SWEEP)
        self.assertEqual(lint_of(page), [], "the first start's sweep belongs to the first start")

    def test_a_line_before_serving_is_found_once_when_a_restart_follows(self):
        page = shown(SWEEP, SERVING, SERVING)
        self.assertEqual(lint_of(page), [order_problem("p.md:1", SWEEP)], "one problem for one line")

    def test_a_background_line_before_the_second_start_is_found(self):
        stopped = "<time>  INFO vcon_server::serve: stopped"
        page = shown(SERVING, stopped, SWEEP, SERVING)
        self.assertEqual(lint_of(page), [order_problem("p.md:1", SWEEP)], "after stopped, a new start")

    # BACKGROUND_LINES is checked against the messages serve.rs logs by
    # crates/vcon-server/tests/serve.rs, which has the source tree; the hook
    # tests run this file in a fixture that holds scripts/ only.

    def test_no_background_line_is_a_start_line(self):
        for m in dt.BACKGROUND_LINES:
            self.assertFalse(m.startswith("serving"), f"{m!r} is not the serving line")
            self.assertFalse(m.startswith("the operator listener"), f"{m!r} is not the operator line")



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


PREFETCH_PINS = {
    f"{REL}/v0.3.0/SHA256SUMS.txt": SHA_A,
    f"{REL}/v0.4.0/SHA256SUMS.txt": SHA_A,
    f"{REL}/v0.4.0/vcon-server-0.4.0-x86_64-unknown-linux-gnu.tar.gz": SHA_A,
    f"{REL}/v0.4.0/vcon-server-0.4.0-aarch64-unknown-linux-gnu.tar.gz": SHA_A,
    "https://github.com/sigstore/cosign/releases/download/v3.0.6/cosign_3.0.6_amd64.deb": SHA_B,
    "https://github.com/sigstore/cosign/releases/download/v3.0.6/cosign_3.0.6_arm64.deb": SHA_B,
    "https://github.com/minio/minio/releases/download/R1/minio.linux-amd64.R1": SHA_B,
    "https://github.com/minio/minio/releases/download/R1/minio.linux-arm64.R1": SHA_B,
}
INSTALL_STEP = (
    "VERSION=0.4.0\nARCHIVE=vcon-server-$VERSION-$(uname -m)-unknown-linux-gnu.tar.gz\n"
    "curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$VERSION/$ARCHIVE\n"
    "curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$VERSION/SHA256SUMS.txt\n"
)


class Prefetch(unittest.TestCase):
    """Before the first step, the runner fetches each pinned asset a step
    can name, so the mirror answers a step at once: on 2026-11-09 dnf gave
    up on the mirror after 30 s under 1000 bytes/s while github.com was
    slow (release-fedora, run of 18ff594)."""

    def test_the_release_and_cpu_a_step_names_are_resolved(self):
        urls = dt.prefetch_urls(page_with(INSTALL_STEP), {}, PREFETCH_PINS, "x86_64")
        self.assertEqual(urls, [f"{REL}/v0.4.0/SHA256SUMS.txt", f"{REL}/v0.4.0/vcon-server-0.4.0-x86_64-unknown-linux-gnu.tar.gz"])

    def test_the_scenario_s_release_wins_over_the_page_s(self):
        urls = dt.prefetch_urls(page_with(INSTALL_STEP), {"VERSION": "0.3.0"}, PREFETCH_PINS, "x86_64")
        self.assertEqual(urls, [f"{REL}/v0.3.0/SHA256SUMS.txt"])

    def test_dpkg_s_architecture_name_is_resolved(self):
        step = "COSIGN_DEB=cosign_3.0.6_$(dpkg --print-architecture).deb\ncurl -fsSLO https://github.com/sigstore/cosign/releases/download/v3.0.6/$COSIGN_DEB\n"
        self.assertEqual(dt.prefetch_urls(page_with(step), {}, PREFETCH_PINS, "aarch64"),
                         ["https://github.com/sigstore/cosign/releases/download/v3.0.6/cosign_3.0.6_arm64.deb"])

    def test_a_case_arm_for_this_cpu_sets_its_variables(self):
        step = ('MINIO_RELEASE=R1\ncase "$(uname -m)" in\n  x86_64) MINIO_ARCH=amd64\n    X=1 ;;\n  aarch64) MINIO_ARCH=arm64 ;;\nesac\n'
                'curl -fsSL -o minio "https://github.com/minio/minio/releases/download/$MINIO_RELEASE/minio.linux-$MINIO_ARCH.$MINIO_RELEASE"\n')
        self.assertEqual(dt.prefetch_urls(page_with(step), {}, PREFETCH_PINS, "x86_64"),
                         ["https://github.com/minio/minio/releases/download/R1/minio.linux-amd64.R1"])

    def test_a_variable_no_step_sets_matches_every_pin(self):
        step = 'curl -fsSL -o minio "https://github.com/minio/minio/releases/download/R1/minio.linux-$ARCH.R1"\n'
        self.assertEqual(len(dt.prefetch_urls(page_with(step), {}, PREFETCH_PINS, "x86_64")), 2)

    def test_steps_with_no_release_url_prefetch_nothing(self):
        self.assertEqual(dt.prefetch_urls(page_with("echo hello\n"), {}, PREFETCH_PINS, "x86_64"), [])

    def test_a_failed_prefetch_is_reported_and_does_not_stop_the_run(self):
        calls = []

        def fetch(url, sha, dest):
            calls.append(url)
            return 1
        m = dt.Mirror(PREFETCH_PINS, tempfile.mkdtemp(prefix="docs-test-prefetch-"), host="127.0.0.1", fetch=fetch)
        try:
            results = dt.prefetch(m, [f"{REL}/v0.4.0/SHA256SUMS.txt"])
        finally:
            shutil.rmtree(m.cache, ignore_errors=True)
        self.assertEqual(results, [(f"{REL}/v0.4.0/SHA256SUMS.txt", 504)])
        self.assertEqual(calls, [f"https://github.com/NormB/vcon-server/releases/download/v0.4.0/SHA256SUMS.txt"])


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
        self.assertIn(f"FROM {dt.pinned_image('debian:trixie-slim')}\n", dt.systemd_dockerfile({}))

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
        self.assertIn('Acquire::Retries "3";', dt.runner_prelude(self.scenario("debian:trixie-slim", {"apt"})))

    def test_a_host_scenario_changes_nothing_on_the_host(self):
        self.assertEqual(dt.runner_prelude(self.scenario("host", {"docker"})), "")

    def test_a_dnf_scenario_gets_no_apt_setting(self):
        self.assertEqual(dt.runner_prelude(self.scenario("fedora:44", {"dnf"})), "")

    def test_an_acme_scenario_gets_a_test_certificate_authority(self):
        prelude = dt.runner_prelude(self.scenario("systemd", {"debian", "apt", "systemd", "acme"}))
        self.assertIn("pebble", prelude, "Pebble stands in for Let's Encrypt")
        self.assertIn("server = https://localhost:14000/dir", prelude, "certbot is pointed at Pebble")
        self.assertIn("update-ca-certificates", prelude, "Pebble's roots are trusted as a public CA's are")
        self.assertIn(dt.ACME_TSIG_SECRET, prelude, "BIND takes the TSIG key the page shows")
        # The certbot package ships /etc/letsencrypt/cli.ini as a conffile:
        # one there already stops its install at dpkg's prompt.
        self.assertNotIn("/etc/letsencrypt/cli.ini", prelude, "the package's own cli.ini is left alone")
        self.assertIn("/root/.config/letsencrypt/cli.ini", prelude, "root's cli.ini names Pebble")

    def test_a_scenario_without_acme_gets_no_certificate_authority(self):
        prelude = dt.runner_prelude(self.scenario("systemd", {"debian", "apt", "systemd"}))
        self.assertNotIn("pebble", prelude, "only an acme scenario runs Pebble")


class LetsEncrypt(unittest.TestCase):
    """The Let's Encrypt page runs once per challenge, after getting started,
    under systemd, whose unit the deploy hook restarts."""

    def scenario(self, name):
        found = [s for s in dt.SCENARIOS if s.name == name]
        self.assertEqual(len(found), 1, f"one scenario named {name}")
        return found[0]

    def test_each_challenge_has_a_scenario(self):
        for name, challenge in (("lets-encrypt-http", "http-01"), ("lets-encrypt-dns", "dns-01")):
            s = self.scenario(name)
            self.assertEqual(s.where, "systemd", f"{name}: the deploy hook restarts a systemd unit")
            self.assertEqual(s.pages, ("getting-started.md", "lets-encrypt.md"), f"{name}: after getting started")
            self.assertTrue({"acme", challenge} <= s.tags, f"{name}: runs the {challenge} steps against Pebble")

    def test_the_page_shows_the_tsig_secret_bind_takes(self):
        page = open(os.path.join(dt.ROOT, "website", "content", "docs", "lets-encrypt.md")).read()
        self.assertIn(dt.ACME_TSIG_SECRET, page, "the page's sample credentials are the ones CI's DNS server takes")


TERMINAL_PAGE = """<!-- docs-test: terminal READY -->
```sh
vcon-server tui --config tui.toml
```

```text
a | b | connected
```
"""


class Terminal(unittest.TestCase):
    """A step the page runs in a terminal: a full-screen command, which the
    runner starts in a pseudo-terminal, ends with q, and whose screen it
    compares with the page."""

    # Success.

    def test_the_marker_names_the_text_that_says_the_screen_is_ready(self):
        (block,) = dt.parse(TERMINAL_PAGE, "p.md")
        self.assertEqual(block.terminal, "READY")

    # Failure.

    def test_a_terminal_marker_without_its_text_is_refused(self):
        with self.assertRaisesRegex(dt.PageError, "p.md:1: `terminal` needs the text"):
            dt.parse("<!-- docs-test: terminal -->\n```sh\ntop\n```\n", "p.md")


    def test_a_ready_text_of_two_words_is_refused(self):
        # The runner looks for the text in what the command writes, where a
        # space the screen already held is a cursor movement, not a space.
        with self.assertRaisesRegex(dt.PageError, "p.md:1: `terminal` takes one word"):
            dt.parse("<!-- docs-test: terminal two words -->\n```sh\ntop\n```\n", "p.md")


    def test_a_terminal_step_cannot_also_run_in_the_background(self):
        with self.assertRaisesRegex(dt.PageError, "p.md:1: a `terminal` step cannot be a `background` step"):
            dt.parse("<!-- docs-test: terminal READY; background serving -->\n```sh\ntop\n```\n", "p.md")


    def step(self, page=TERMINAL_PAGE):
        return dt.script(dt.parse(page, "p.md"), "")

    def test_a_terminal_step_runs_in_a_pseudo_terminal_of_80_by_24(self):
        script = self.step()
        self.assertIn("script -qefc", script, "util-linux script gives the command a terminal")
        self.assertIn("stty rows 24 cols 80", script, "the size the page's screen was taken at")


    def run_steps(self, page, env=None):
        """Runs the page's script with bash and returns each step's exit
        record and output."""
        work = tempfile.mkdtemp()
        out = os.path.join(work, "out")
        os.mkdir(out)
        try:
            with open(os.path.join(work, "run.sh"), "w", encoding="utf-8") as f:
                f.write(self.step(page))
            env = dict(os.environ, DOCS_TEST_OUT=out, DOCS_TEST_WORK=work, **(env or {}))
            subprocess.run(["bash", os.path.join(work, "run.sh")], env=env, check=False, timeout=120)
            def read(name):
                path = os.path.join(out, name)
                if not os.path.exists(path):
                    return None
                with open(path, encoding="utf-8", errors="replace") as f:
                    return f.read()
            n = len(dt.parse(page, "p.md"))
            return [((read(f"{i}.rc") or "").strip() or None, read(f"{i}.out")) for i in range(n)]
        finally:
            shutil.rmtree(work)

    # A full-screen command in bash: it draws on the alternate screen,
    # waits for one key without echo, and ends 0 on q, 3 on anything else.
    FULL_SCREEN = (
        "printf '\\033[?1049h\\033[1;1Ha | b | connected READY'\n"
        "stty -echo -icanon\n"
        "key=$(dd bs=1 count=1 2>/dev/null)\n"
        "printf '\\033[?1049l'\n"
        "[ \"$key\" = q ] || exit 3\n"
    )

    def test_q_is_typed_once_the_ready_text_is_written_and_the_command_ends_0(self):
        page = "<!-- docs-test: terminal READY -->\n```sh\n" + self.FULL_SCREEN + "```\n"
        ((rc, out),) = self.run_steps(page)
        self.assertEqual(rc, "0", f"the command read q and ended: {out!r}")


    def test_a_command_that_never_writes_the_ready_text_is_recorded_as_never(self):
        page = "<!-- docs-test: terminal NEVER-WRITTEN -->\n```sh\n" + self.FULL_SCREEN + "```\n"
        with unittest.mock.patch.object(dt, "READY_SECONDS", 1):
            ((rc, _),) = self.run_steps(page)
        self.assertEqual(rc, "never")


    def terminal_page(self, code, ready="READY", more=""):
        return f"<!-- docs-test: terminal {ready} -->\n```sh\n{code}```\n{more}"

    def test_what_the_command_wrote_is_the_steps_output(self):
        ((_, out),) = self.run_steps(self.terminal_page(self.FULL_SCREEN))
        self.assertIn("a | b | connected READY", out)

    def test_the_step_after_a_terminal_step_runs(self):
        rcs = [rc for rc, _ in self.run_steps(self.terminal_page(self.FULL_SCREEN, more="\n```sh\necho after\n```\n"))]
        self.assertEqual(rcs, ["0", "0"])

    def test_a_command_that_ends_before_its_screen_is_ready_is_not_waited_for(self):
        started = time.monotonic()
        ((rc, _),) = self.run_steps(self.terminal_page("exit 1\n"))
        self.assertEqual(rc, "never")
        self.assertLess(time.monotonic() - started, 20, "the wait for the ready text ended with the command")

    def test_a_command_that_ends_1_after_its_screen_is_recorded_and_stops_the_run(self):
        # The terminal is set before the ready text, as a full-screen
        # program sets it before it draws: the key typed at READY must
        # find it set.
        code = "stty -echo -icanon\nprintf READY\ndd bs=1 count=1 >/dev/null 2>&1\nexit 1\n"
        rcs = [rc for rc, _ in self.run_steps(self.terminal_page(code, more="\n```sh\necho after\n```\n"))]
        self.assertEqual(rcs, ["1", None], "the exit is recorded and the next step does not run")

    def test_a_failing_command_in_the_block_ends_the_block(self):
        ((rc, out),) = self.run_steps(self.terminal_page("false\nprintf READY\n"))
        self.assertEqual(rc, "never", f"nothing after the failure ran: {out!r}")

    def test_a_command_that_does_not_end_on_q_is_ended_at_the_time_limit(self):
        with unittest.mock.patch.object(dt, "TERMINAL_SECONDS", 2):
            ((rc, _),) = self.run_steps(self.terminal_page("printf READY\nsleep 30\n"))
        self.assertEqual(rc, "124", "timeout's status for a command it ended")

    def test_the_terminal_is_80_columns_by_24_lines(self):
        code = "printf 'size=%s READY' \"$(stty size | tr ' ' x)\"\nstty -echo -icanon\ndd bs=1 count=1 >/dev/null 2>&1\n"
        ((_, out),) = self.run_steps(self.terminal_page(code))
        self.assertIn("size=24x80", out)

    # The keys a step types: `keys`, after `terminal`.

    def keys_page(self, keys, code=None):
        marker = f"<!-- docs-test: terminal READY; keys {keys} -->" if keys is not None else "<!-- docs-test: terminal READY -->"
        return f"{marker}\n```sh\n{code or 'top'}\n```\n"

    def typed(self, keys, count):
        """The bytes a command read, as hex, when the step's marker names
        `keys`: the command reads `count` bytes as a full-screen program
        does, each as it is typed and none changed by the terminal."""
        # stty before READY: the runner types the keys once READY is
        # written, and an Enter typed while ICRNL is still on reads back as
        # 0a, not 0d (seen once in repeated runs on 2026-11-15).
        code = (
            "stty -echo -icanon -icrnl\n"
            "printf READY\n"
            f"typed=$(for _ in $(seq {count}); do dd bs=1 count=1 2>/dev/null; done | od -An -tx1 | tr -d ' \\n')\n"
            'printf "typed=%s." "$typed"'
        )
        ((rc, out),) = self.run_steps(self.keys_page(keys, code))
        self.assertEqual(rc, "0", out)
        return re.search(r"typed=([0-9a-f]*)\.", out).group(1)

    # Success.

    def test_a_step_with_no_keys_marker_types_q(self):
        (block,) = dt.parse(self.keys_page(None), "p.md")
        self.assertEqual(block.keys, ("q",))

    def test_the_keys_marker_names_the_keys_in_order(self):
        (block,) = dt.parse(self.keys_page("Enter q"), "p.md")
        self.assertEqual(block.keys, ("Enter", "q"))

    def test_the_keys_are_typed_in_order(self):
        self.assertEqual(self.typed("j q", 2), "6a71", "j, then q")

    def test_enter_is_typed_as_a_carriage_return(self):
        self.assertEqual(self.typed("Enter q", 2), "0d71", "what a terminal sends for Enter, then q")

    def test_a_question_mark_is_typed_as_it_is(self):
        self.assertEqual(self.typed("? q", 2), "3f71")

    def test_one_key_is_typed_once(self):
        self.assertEqual(self.typed("q", 1), "71")

    # Failure.

    def test_keys_without_terminal_is_refused(self):
        with self.assertRaisesRegex(dt.PageError, "p.md:1: `keys` is for a `terminal` step"):
            dt.parse("<!-- docs-test: keys q -->\n```sh\ntop\n```\n", "p.md")

    def test_a_keys_marker_without_a_key_is_refused(self):
        with self.assertRaisesRegex(dt.PageError, "p.md:1: `keys` needs the keys to type"):
            dt.parse("<!-- docs-test: terminal READY; keys -->\n```sh\ntop\n```\n", "p.md")

    def test_a_key_name_the_runner_does_not_know_is_refused(self):
        with self.assertRaisesRegex(dt.PageError, "p.md:1: `keys`: `Escape` is not one character or one of Enter"):
            dt.parse(self.keys_page("Escape q"), "p.md")

    def test_two_characters_are_not_one_key(self):
        with self.assertRaisesRegex(dt.PageError, "p.md:1: `keys`: `jq` is not one character"):
            dt.parse(self.keys_page("jq"), "p.md")

    def test_keys_that_do_not_end_the_command_end_at_the_time_limit(self):
        code = "printf READY\nstty -echo -icanon\ndd bs=1 count=1 >/dev/null 2>&1\nsleep 30"
        with unittest.mock.patch.object(dt, "TERMINAL_SECONDS", 2):
            ((rc, _),) = self.run_steps(self.keys_page("j", code))
        self.assertEqual(rc, "124", "the command read j and went on; nothing typed q")

    # Edge.

    def test_a_quote_is_typed_as_it_is(self):
        self.assertEqual(self.typed("' q", 2), "2771", "the shell's own quote character")

    def test_a_percent_sign_is_typed_as_it_is(self):
        self.assertEqual(self.typed("% q", 2), "2571", "printf's own format character")

    def test_a_backslash_is_typed_as_it_is(self):
        self.assertEqual(self.typed("\\ q", 2), "5c71", "printf's own escape character")

    def test_spaces_between_keys_are_not_keys(self):
        (block,) = dt.parse(self.keys_page("Enter    q"), "p.md")
        self.assertEqual(block.keys, ("Enter", "q"))

    def test_a_key_name_is_matched_with_its_capital(self):
        with self.assertRaisesRegex(dt.PageError, "`enter` is not one character"):
            dt.parse(self.keys_page("enter q"), "p.md")

    def test_quotes_and_dollars_in_the_block_reach_the_command_as_written(self):
        code = "X=inner\nprintf '%s READY' \"it's $X\"\nstty -echo -icanon\ndd bs=1 count=1 >/dev/null 2>&1\n"
        ((_, out),) = self.run_steps(self.terminal_page(code))
        self.assertIn("it's inner READY", out)

    def test_the_block_sees_exported_variables_only(self):
        setup = "```sh\nexport SHOWN=exported\nHIDDEN=plain\n```\n\n"
        code = "printf '%s READY' \"[$SHOWN][${HIDDEN:-}]\"\nstty -echo -icanon\ndd bs=1 count=1 >/dev/null 2>&1\n"
        (_, (_, out)) = self.run_steps(setup + self.terminal_page(code))
        self.assertIn("[exported][] READY", out)

    def test_a_ready_text_with_shell_characters_is_looked_for_as_written(self):
        code = "printf '%s' 'UTC+00:00$x*'\nstty -echo -icanon\ndd bs=1 count=1 >/dev/null 2>&1\n"
        ((rc, out),) = self.run_steps(self.terminal_page(code, ready="UTC+00:00$x*"))
        self.assertEqual(rc, "0", out)


class Screen(unittest.TestCase):
    """What a terminal of 80 columns by 24 lines shows after the bytes a
    full-screen command wrote: the text the page's screen is compared with."""

    # Success.

    def test_text_lands_where_the_cursor_is(self):
        self.assertEqual(dt.screen("hello"), "hello\n")


    def test_a_cursor_position_moves_to_its_line_and_column(self):
        self.assertEqual(dt.screen("\x1b[2;3Hxy"), "\n  xy\n")


    def test_colors_and_other_attributes_are_not_text(self):
        self.assertEqual(dt.screen("\x1b[1m\x1b[32mconnected\x1b[0m"), "connected\n")


    def test_the_screen_is_the_alternate_screen_as_the_command_left_it(self):
        written = "$ tui\x1b[?1049h\x1b[1;1Ha | b | connected\x1b[?1049l$ "
        self.assertEqual(dt.screen(written), "a | b | connected\n")


    def test_clearing_the_screen_empties_it(self):
        self.assertEqual(dt.screen("old\x1b[2J\x1b[1;5Hnew"), "    new\n")


    # Edge.

    def test_a_carriage_return_and_a_line_feed_start_the_next_line(self):
        self.assertEqual(dt.screen("one\r\ntwo"), "one\ntwo\n")


    def test_a_position_without_numbers_is_the_first_cell(self):
        self.assertEqual(dt.screen("\x1b[5;5Hx\x1b[Hy"), "y\n\n\n\n    x\n")


    def test_a_position_past_the_screen_is_its_last_cell(self):
        shown = dt.screen("\x1b[99;999Hx").split("\n")
        self.assertEqual(len(shown) - 1, 24, "the last line")
        self.assertEqual(shown[23], " " * 79 + "x", "the last column")


    def test_text_past_the_last_column_goes_on_at_the_next_line(self):
        self.assertEqual(dt.screen("\x1b[1;79Habcd"), " " * 78 + "ab\ncd\n")


    def test_a_line_feed_on_the_last_line_moves_every_line_up(self):
        written = "first\r\n" + "\r\n" * 22 + "last\r\nnew"
        shown = dt.screen(written).split("\n")
        self.assertEqual(shown[22:24], ["last", "new"], "the screen moved up one line")
        self.assertNotIn("first", shown, "the first line left the screen")


    def test_erasing_to_the_end_of_the_line_keeps_what_is_before_the_cursor(self):
        self.assertEqual(dt.screen("abcdef\r\nsecond\x1b[1;3H\x1b[K"), "ab\nsecond\n")


    def test_a_window_title_is_not_text(self):
        self.assertEqual(dt.screen("\x1b]0;vcon-server\x07shown"), "shown\n")


    def test_control_characters_and_short_escapes_are_dropped(self):
        # ESC = (keypad mode) and a bell: neither is a cell.
        self.assertEqual(dt.screen("\x1b=a\x07b"), "ab\n")


    def test_a_command_that_never_left_an_alternate_screen_shows_the_main_one(self):
        self.assertEqual(dt.screen("plain output\r\n"), "plain output\n")

    def test_of_two_visits_to_the_alternate_screen_the_last_is_shown(self):
        written = "\x1b[?1049hfirst\x1b[?1049l\x1b[?1049hsecond\x1b[?1049l"
        self.assertEqual(dt.screen(written), "second\n")

    def test_a_character_of_several_bytes_is_one_cell(self):
        self.assertEqual(dt.screen("\u2026|"), "\u2026|\n")



class TerminalReport(unittest.TestCase):
    """The verdict on a terminal step: its screen against the page's."""

    WROTE = "\x1b[?1049h\x1b[1;1Ha | b | \x1b[1mconnected\x1b[0m\x1b[24;1Hq quit\x1b[?1049l"

    def verdict(self, page, wrote, rc="0"):
        """How many steps passed, and what the report printed."""
        steps = dt.parse(page, "p.md")
        outdir = tempfile.mkdtemp()
        try:
            with open(os.path.join(outdir, "0.out"), "w", encoding="utf-8") as f:
                f.write(wrote)
            with open(os.path.join(outdir, "0.rc"), "w", encoding="utf-8") as f:
                f.write(rc + "\n")
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                passed = dt.report(steps, outdir)
            return passed, printed.getvalue()
        finally:
            shutil.rmtree(outdir)

    # Success.

    def test_the_screen_is_compared_with_the_pages_text_block(self):
        page = TERMINAL_PAGE.replace("a | b | connected\n", "a | b | connected\n\u2026\nq quit\n")
        passed, printed = self.verdict(page, self.WROTE)
        self.assertEqual(passed, 1, printed)


    # Failure.

    def test_a_screen_that_differs_fails_and_is_shown_as_a_screen(self):
        passed, printed = self.verdict(TERMINAL_PAGE, self.WROTE.replace("connected", "refused"))
        self.assertEqual(passed, 0, printed)
        self.assertIn("line 1: the page says 'a | b | connected', it printed 'a | b | refused'", printed)
        self.assertNotIn("\x1b", printed, "the report shows the screen, not the escape sequences")

    def test_a_ready_text_never_written_is_named(self):
        passed, printed = self.verdict(TERMINAL_PAGE, self.WROTE, rc="never")
        self.assertEqual(passed, 0, printed)
        self.assertIn("it never printed 'READY'", printed)


def run_script(text, env=None, timeout=60):
    """Runs a script the runner wrote with bash in a fresh directory and
    returns the files of its output directory, by name."""
    work = tempfile.mkdtemp()
    out = os.path.join(work, "out")
    os.mkdir(out)
    try:
        with open(os.path.join(work, "run.sh"), "w", encoding="utf-8") as f:
            f.write(text)
        env = dict(os.environ, DOCS_TEST_OUT=out, DOCS_TEST_WORK=work, **(env or {}))
        subprocess.run(["bash", os.path.join(work, "run.sh")], env=env, check=False, timeout=timeout)
        files = {}
        for name in os.listdir(out):
            with open(os.path.join(out, name), encoding="utf-8", errors="replace") as f:
                files[name] = f.read()
        return files
    finally:
        shutil.rmtree(work)


def report_of(steps, files):
    """How many steps passed, and what the report printed, for an output
    directory that holds `files` (name: text)."""
    outdir = tempfile.mkdtemp()
    try:
        for name, text in files.items():
            with open(os.path.join(outdir, name), "w", encoding="utf-8") as f:
                f.write(text)
        printed = io.StringIO()
        with contextlib.redirect_stdout(printed):
            passed = dt.report(steps, outdir)
        return passed, printed.getvalue()
    finally:
        shutil.rmtree(outdir)


TWO_STEPS = "```sh\necho one\n```\n\n```sh\necho two\n```\n"


class StepTimes(unittest.TestCase):
    """Each step's start, from the first step's, and how long it ran: a
    failure that depends on the pace of the run (systemd's start limit
    counts the starts of the last 10 s) shows that pace in the report."""

    # Success.

    def test_each_step_records_when_it_started_and_the_run_when_it_ended(self):
        files = run_script(dt.script(dt.parse(TWO_STEPS, "p.md"), ""))
        for name in ("0.start", "1.start", "end", "finished"):
            self.assertIn(name, files, f"the run writes {name}: {sorted(files)}")
        first, second, end = (float(files[n]) for n in ("0.start", "1.start", "end"))
        self.assertLessEqual(first, second, "the second step starts after the first")
        self.assertLessEqual(second, end, "the run ends after its last step starts")

    def test_the_report_gives_each_step_its_start_from_the_first_and_its_length(self):
        steps = dt.parse(TWO_STEPS, "p.md")
        files = {"0.rc": "0\n", "1.rc": "0\n", "0.start": "100.0\n", "1.start": "101.5\n", "end": "104.25\n"}
        passed, printed = report_of(steps, files)
        self.assertEqual(passed, 2, printed)
        self.assertIn("ok       p.md:1 (at 0.00 s, for 1.50 s)\n", printed, "the first step's time")
        self.assertIn("ok       p.md:5 (at 1.50 s, for 2.75 s)\n", printed, "the last step runs to the end")

    def test_a_step_that_runs_last_by_rule_is_timed_to_the_end_of_those(self):
        page = "```sh\necho one\n```\n\n<!-- docs-test: always -->\n```sh\necho last\n```\n"
        steps = dt.parse(page, "p.md")
        self.assertTrue(steps[1].always, "the second step runs last by its marker")
        files = {"0.rc": "0\n", "1.rc": "0\n", "0.start": "10\n", "end": "11\n", "1.start": "11.5\n", "finished": "12\n"}
        passed, printed = report_of(steps, files)
        self.assertEqual(passed, 2, printed)
        self.assertIn("ok       p.md:1 (at 0.00 s, for 1.00 s)\n", printed, "the main steps end at `end`")
        self.assertIn("ok       p.md:6 (at 1.50 s, for 0.50 s)\n", printed, "the last-by-rule step ends at `finished`")

    def test_a_step_that_runs_after_a_slow_one_shows_the_wait(self):
        steps = dt.parse(TWO_STEPS, "p.md")
        files = {"0.rc": "0\n", "1.rc": "0\n", "0.start": "5\n", "1.start": "17\n", "end": "17.25\n"}
        _, printed = report_of(steps, files)
        self.assertIn("ok       p.md:1 (at 0.00 s, for 12.00 s)\n", printed, "the slow step's length")
        self.assertIn("ok       p.md:5 (at 12.00 s, for 0.25 s)\n", printed, "the next step's start")

    # Failure.

    def test_a_step_that_stopped_the_run_is_timed_to_the_end_of_the_run(self):
        steps = dt.parse("```sh\necho one\n```\n\n```sh\nfalse\n```\n\n```sh\necho three\n```\n", "p.md")
        files = {"0.rc": "0\n", "0.start": "1\n", "1.start": "2\n", "end": "4.5\n"}
        passed, printed = report_of(steps, files)
        self.assertEqual(passed, 1, printed)
        self.assertIn(
            "FAIL     p.md:5: it stopped the run: a command in it failed (at 1.00 s, for 2.50 s)\n",
            printed,
            "the failed step's time ends its verdict line",
        )
        self.assertIn("not run  p.md:9\n", printed, "a step that did not run has no time")

    def test_a_step_that_failed_in_a_real_run_has_a_start_and_no_rc(self):
        files = run_script(dt.script(dt.parse("```sh\necho one\n```\n\n```sh\nfalse\n```\n", "p.md"), ""))
        self.assertIn("1.start", files, "the failed step recorded its start")
        self.assertNotIn("1.rc", files, "and no exit record, which is how the report knows it stopped the run")
        self.assertIn("end", files, "the end is written when the run stops")

    def test_a_differing_output_keeps_its_time_after_the_reason(self):
        steps = dt.parse("```sh\necho one\n```\n\n```text\ntwo\n```\n", "p.md")
        files = {"0.rc": "0\n", "0.out": "one\n", "0.start": "3\n", "end": "3.5\n"}
        passed, printed = report_of(steps, files)
        self.assertEqual(passed, 0, printed)
        self.assertRegex(printed, r"FAIL     p\.md:1: .* \(at 0\.00 s, for 0\.50 s\)\n", "the reason, then the time")

    # Edge.

    def test_an_output_directory_with_no_times_reports_as_before(self):
        passed, printed = report_of(dt.parse(TWO_STEPS, "p.md"), {"0.rc": "0\n", "1.rc": "0\n"})
        self.assertEqual(passed, 2, printed)
        self.assertEqual(printed, "ok       p.md:1\nok       p.md:5\n", "no time, no brackets")

    def test_a_start_with_no_later_time_has_a_start_and_no_length(self):
        steps = dt.parse(TWO_STEPS, "p.md")
        files = {"0.rc": "0\n", "1.rc": "0\n", "0.start": "1\n", "1.start": "2\n"}
        _, printed = report_of(steps, files)
        self.assertIn("ok       p.md:5 (at 1.00 s)\n", printed, "nothing after it says when it ended")

    def test_an_unreadable_time_is_left_out(self):
        steps = dt.parse(TWO_STEPS, "p.md")
        files = {"0.rc": "0\n", "1.rc": "0\n", "0.start": "N\n", "1.start": "2\n", "end": "3\n"}
        passed, printed = report_of(steps, files)
        self.assertEqual(passed, 2, printed)
        self.assertIn("ok       p.md:1\n", printed, "a start that is not a number gives no time")
        self.assertIn("ok       p.md:5 (at 0.00 s, for 1.00 s)\n", printed, "the first readable start is 0")

    def test_the_times_are_read_with_the_clock_bash_gives_every_step(self):
        script = dt.script(dt.parse(TWO_STEPS, "p.md"), "")
        self.assertIn('date +%s.%N > "$DOCS_TEST_OUT/0.start"', script, "the start of step 0")
        self.assertLess(script.index('"$DOCS_TEST_OUT/0.start"'), script.index("echo one"), "before the block runs")

    def test_a_file_step_and_a_background_step_are_timed_too(self):
        page = (
            "<!-- docs-test: file a.toml -->\n```toml\nx = 1\n```\n\n"
            "<!-- docs-test: background up -->\n```sh\necho up; sleep 1\n```\n"
        )
        files = run_script(dt.script(dt.parse(page, "p.md"), ""))
        self.assertIn("0.start", files, "the file step's start")
        self.assertIn("1.start", files, "the background step's start")


SYSTEMD_PAGE = (
    "```sh\nsudo systemctl enable --now vcon-server\n```\n\n"
    "```sh\nsudo systemctl restart vcon-server.service\nsystemctl start postgresql\n```\n"
)


def stub_bin(where, names):
    """Commands that print their name and arguments, in `where`, for a run
    that must not reach the host's systemd."""
    for name in names:
        path = os.path.join(where, name)
        with open(path, "w", encoding="utf-8") as f:
            f.write(f'#!/bin/sh\necho "{name} $*"\n')
        os.chmod(path, 0o755)


class SystemdUnits(unittest.TestCase):
    """Under systemd, the runner records each unit the steps start or stop,
    as systemd saw it when the run ended, and prints that when a step
    failed: a start refused, a crash, a restart loop."""

    # Success.

    def test_the_units_are_the_ones_the_steps_start_stop_or_restart(self):
        steps = dt.parse(SYSTEMD_PAGE, "p.md")
        self.assertEqual(dt.systemd_units(steps), ("vcon-server", "postgresql"), "in order, once each, no .service")

    def test_a_timer_keeps_its_suffix(self):
        steps = dt.parse("```sh\nsudo systemctl enable --now certbot-renew.timer\n```\n", "p.md")
        self.assertEqual(dt.systemd_units(steps), ("certbot-renew.timer",), "a unit that is not a service")

    def test_the_systemd_scenarios_units_are_read_from_its_pages(self):
        (scenario,) = [s for s in dt.SCENARIOS if s.name == "systemd"]
        steps = dt.select(sum((dt.page_blocks(p) for p in scenario.pages), []), scenario)
        self.assertIn("vcon-server", dt.systemd_units(steps), "the server's unit")

    def test_the_script_records_each_units_status_journal_and_starts(self):
        script = dt.script(dt.parse(SYSTEMD_PAGE, "p.md"), "", units=("vcon-server", "certbot-renew.timer"))
        for command in (
            "systemctl status --no-pager -l vcon-server",
            "systemctl show -p Result -p NRestarts -p StartLimitBurst -p StartLimitIntervalUSec vcon-server",
            "journalctl -u vcon-server --no-pager -n 200 -o short-precise",
            "journalctl _PID=1 UNIT=vcon-server.service --no-pager -o short-precise",
            "journalctl _PID=1 UNIT=certbot-renew.timer --no-pager -o short-precise",
        ):
            self.assertIn(command, script, f"the script runs {command}")
        self.assertIn('> "$DOCS_TEST_OUT/systemd.txt" 2>&1', script, "into one file of the output directory")

    def test_a_run_that_fails_records_the_units_after_the_failure(self):
        bin_dir = tempfile.mkdtemp()
        try:
            stub_bin(bin_dir, ("systemctl", "journalctl"))
            page = "```sh\necho one\n```\n\n```sh\nfalse\n```\n"
            files = run_script(
                dt.script(dt.parse(page, "p.md"), "", units=("vcon-server",)),
                env={"PATH": bin_dir + os.pathsep + os.environ["PATH"]},
            )
        finally:
            shutil.rmtree(bin_dir)
        self.assertIn("systemd.txt", files, sorted(files))
        text = files["systemd.txt"]
        self.assertIn("systemctl status --no-pager -l vcon-server", text, "the status command ran")
        self.assertIn("journalctl -u vcon-server --no-pager -n 200 -o short-precise", text, "the journal command ran")
        self.assertIn("journalctl _PID=1 UNIT=vcon-server.service", text, "systemd's own lines on the unit")

    def test_the_report_prints_the_units_when_a_step_failed(self):
        steps = dt.parse("```sh\nfalse\n```\n", "p.md")
        record = "== vcon-server\nActive: failed (Result: start-limit-hit)\n"
        passed, printed = report_of(steps, {"systemd.txt": record})
        self.assertEqual(passed, 0, printed)
        self.assertIn("the units, as systemd saw them when the run ended:\n", printed, "a heading")
        self.assertIn("    Active: failed (Result: start-limit-hit)\n", printed, "the record, indented")
        self.assertGreater(printed.index("Active:"), printed.index("FAIL "), "after the step lines")

    # Failure.

    def test_a_page_with_no_systemctl_has_no_units(self):
        self.assertEqual(dt.systemd_units(dt.parse(TWO_STEPS, "p.md")), (), "nothing to record")

    def test_a_systemctl_that_names_no_unit_is_not_a_unit(self):
        page = "```sh\nsudo systemctl daemon-reload\nsystemctl is-system-running\nsystemctl --failed\n```\n"
        self.assertEqual(dt.systemd_units(dt.parse(page, "p.md")), (), "daemon-reload and the rest name no unit")

    def test_a_run_that_passes_does_not_print_the_units(self):
        steps = dt.parse(TWO_STEPS, "p.md")
        files = {"0.rc": "0\n", "1.rc": "0\n", "systemd.txt": "== vcon-server\nActive: active\n"}
        passed, printed = report_of(steps, files)
        self.assertEqual(passed, 2, printed)
        self.assertNotIn("Active:", printed, "only a failed run prints them; --keep keeps the file")

    # Edge.

    def test_without_units_the_script_runs_no_systemctl(self):
        script = dt.script(dt.parse(TWO_STEPS, "p.md"), "")
        self.assertNotIn("systemctl", script, "a run with no systemd has nothing to ask")
        self.assertNotIn("systemd.txt", script, "and writes no record")

    def test_a_failed_run_without_a_record_prints_no_heading(self):
        passed, printed = report_of(dt.parse("```sh\nfalse\n```\n", "p.md"), {})
        self.assertEqual(passed, 0, printed)
        self.assertNotIn("as systemd saw them", printed, "no record, no heading")

    def test_the_record_is_written_even_when_the_run_passes(self):
        bin_dir = tempfile.mkdtemp()
        try:
            stub_bin(bin_dir, ("systemctl", "journalctl"))
            files = run_script(
                dt.script(dt.parse(TWO_STEPS, "p.md"), "", units=("vcon-server",)),
                env={"PATH": bin_dir + os.pathsep + os.environ["PATH"]},
            )
        finally:
            shutil.rmtree(bin_dir)
        self.assertIn("systemd.txt", files, "a kept run (--keep) holds it for reading")
        self.assertEqual(files.get("exit", "").strip(), "0", "recording the units does not change the run's exit")

    def test_the_run_function_passes_the_units_only_under_systemd(self):
        source = pathlib.Path(dt.__file__).read_text(encoding="utf-8")
        self.assertIn(
            'units=systemd_units(steps) if scenario.where == "systemd" else ()',
            source,
            "run() hands script() the units of a systemd scenario and none elsewhere",
        )


PAGE_ONE = "```sh\nsudo systemctl enable --now vcon-server\n```\n\n```sh\necho one\n```\n"
PAGE_TWO = "```sh\nsudo systemctl restart vcon-server\n```\n"
PAGE_THREE = "```sh\nsudo systemctl restart vcon-server\n```\n"


class StartLimitPerPage(unittest.TestCase):
    """systemd refuses a unit's sixth start within 10 s (DefaultStartLimit
    Burst=5, IntervalSec=10s). An operator runs each how-to on its own
    occasion; the runner runs the pages one after the other in one
    container, so the starts of earlier pages count against a later one's.
    Under systemd the runner resets each unit's start counter
    (`systemctl reset-failed`) where a page begins, so each page meets the
    limit as an operator does: its own starts count, no other page's."""

    def pages(self, *texts):
        steps = []
        for n, text in enumerate(texts):
            steps += dt.parse(text, f"docs/p{n}.md")
        return steps

    def run_logged(self, steps, units, stub_exit=0):
        """Runs the script with a systemctl that appends its arguments to a
        log, and returns the log's lines and the step exit records."""
        bin_dir = tempfile.mkdtemp()
        log = os.path.join(bin_dir, "calls")
        try:
            path = os.path.join(bin_dir, "systemctl")
            with open(path, "w", encoding="utf-8") as f:
                # `stub_exit` is the exit of a reset only; every other call ends 0.
                f.write(f'#!/bin/sh\necho "$*" >> "{log}"\n[ "$1" = reset-failed ] && exit {stub_exit}\nexit 0\n')
            os.chmod(path, 0o755)
            stub_bin(bin_dir, ("journalctl", "sudo"))
            with open(os.path.join(bin_dir, "sudo"), "w", encoding="utf-8") as f:
                f.write('#!/bin/sh\nexec "$@"\n')
            files = run_script(dt.script(steps, "", units=units), env={"PATH": bin_dir + os.pathsep + os.environ["PATH"]})
            with open(log, encoding="utf-8") as f:
                calls = [l.strip() for l in f]
        finally:
            shutil.rmtree(bin_dir)
        return calls, [files.get(f"{n}.rc", "").strip() or None for n in range(len(steps))]

    # Success.

    def test_each_page_after_the_first_starts_with_the_units_start_counter_reset(self):
        script = dt.script(self.pages(PAGE_ONE, PAGE_TWO), "", units=("vcon-server",))
        first, second, third = (script.index(f"# {n}: ") for n in range(3))
        reset = dt.start_limit_reset(("vcon-server",))
        self.assertNotIn(reset, script[:second], "the first page meets the limit as it stands")
        self.assertIn(reset, script[second:third], "the second page starts with its own counter")
        self.assertEqual(script.count(reset), 1, "once per page boundary")

    def test_the_reset_runs_between_the_pages_in_a_real_run(self):
        calls, rcs = self.run_logged(self.pages(PAGE_ONE, PAGE_TWO), ("vcon-server",))
        self.assertEqual(rcs, ["0", "0", "0"], "every step ran")
        self.assertEqual(
            [c for c in calls if not c.startswith(("status", "show"))],
            ["enable --now vcon-server", "reset-failed vcon-server", "restart vcon-server"],
            "the reset falls after the first page's start and before the second page's",
        )

    def test_three_pages_get_two_resets(self):
        script = dt.script(self.pages(PAGE_ONE, PAGE_TWO, PAGE_THREE), "", units=("vcon-server",))
        self.assertEqual(script.count(dt.start_limit_reset(("vcon-server",))), 2, "one before each later page")

    def test_every_unit_is_reset(self):
        reset = dt.start_limit_reset(("vcon-server", "postgresql"))
        self.assertIn("systemctl reset-failed vcon-server", reset, "the server")
        self.assertIn("systemctl reset-failed postgresql", reset, "and each other unit the steps start")

    def test_the_systemd_scenario_resets_before_each_of_its_later_pages(self):
        (scenario,) = [s for s in dt.SCENARIOS if s.name == "systemd"]
        steps = dt.select(sum((dt.page_blocks(p) for p in scenario.pages), []), scenario)
        units = dt.systemd_units(steps)
        script = dt.script(steps, "", units=units)
        pages = len({dt._page(s) for s in steps if not s.always})
        self.assertEqual(script.count(dt.start_limit_reset(units)), pages - 1, "every page after the first")

    # Failure.

    def test_without_units_nothing_is_reset(self):
        script = dt.script(self.pages(PAGE_ONE, PAGE_TWO), "")
        self.assertNotIn("reset-failed", script, "a run with no systemd has no start limit")

    def test_one_pages_starts_are_not_reset(self):
        # Five starts of one page in 10 s is what an operator would meet:
        # the runner must not hide it.
        page = "".join(f"```sh\nsudo systemctl restart vcon-server\n```\n\n" for _ in range(6))
        script = dt.script(dt.parse(page, "docs/p.md"), "", units=("vcon-server",))
        self.assertNotIn("reset-failed", script, "within a page the limit holds as it does for an operator")

    def test_a_reset_that_fails_does_not_stop_the_run(self):
        calls, rcs = self.run_logged(self.pages(PAGE_ONE, PAGE_TWO), ("vcon-server",), stub_exit=1)
        self.assertIn("reset-failed vcon-server", calls, "the reset was asked for")
        self.assertEqual(rcs[1], "0", "the first page's last step ran")
        self.assertIsNotNone(rcs[2], "a unit systemd does not know yet does not end the run")

    # Edge.

    def test_a_page_that_comes_back_after_another_is_a_new_page(self):
        steps = self.pages(PAGE_ONE, PAGE_TWO) + dt.parse(PAGE_THREE, "docs/p0.md")
        script = dt.script(steps, "", units=("vcon-server",))
        self.assertEqual(script.count("reset-failed vcon-server"), 2, "each change of page is a boundary")

    def test_a_last_by_rule_step_of_a_later_page_gets_no_reset(self):
        steps = self.pages(PAGE_ONE, "<!-- docs-test: always -->\n```sh\necho cleanup\n```\n")
        self.assertTrue(steps[-1].always, "the second page's only step runs last by its marker")
        script = dt.script(steps, "", units=("vcon-server",))
        self.assertNotIn("reset-failed", script, "the always steps run in the exit handler, not as a page")

    def test_the_reset_reads_no_input_and_writes_no_step_output(self):
        reset = dt.start_limit_reset(("vcon-server",))
        self.assertIn("< /dev/null", reset, "it cannot read a step's input")
        self.assertIn("2>/dev/null", reset, "an unknown unit's message is not shown as a step's")

    def test_the_reset_runs_as_the_run_does_without_sudo(self):
        reset = dt.start_limit_reset(("vcon-server",))
        self.assertNotIn("sudo", reset, "the runner's own commands run as root in the container")


class TerminalRules(unittest.TestCase):
    """What the runner holds a page with a terminal step to, and what it
    sets up for one."""

    def scenario(self, where, tags):
        return dt.Scenario("x", where, frozenset(tags), ("tui.md",))

    # Failure.

    def test_a_terminal_step_with_no_screen_after_it_is_a_problem(self):
        blocks = dt.parse("<!-- docs-test: terminal READY -->\n```sh\ntop\n```\n", "p.md")
        self.assertEqual(
            dt.lint_page(blocks, [frozenset()]),
            ["p.md:2: a terminal step with no screen shown after it"],
        )

    # Success.

    def test_a_dnf_scenario_with_a_terminal_step_installs_script(self):
        steps = dt.parse(TERMINAL_PAGE, "p.md")
        self.assertIn(
            "dnf install -y -q util-linux-script",
            dt.runner_prelude(self.scenario("fedora:44", {"dnf"}), steps),
            "Fedora's image has no script(1)",
        )

    def test_every_how_to_scenario_runs_the_terminal_ui_page(self):
        for name in ("foreground", "foreground-fedora", "systemd"):
            (scenario,) = [s for s in dt.SCENARIOS if s.name == name]
            self.assertIn("tui.md", scenario.pages, name)

    def test_the_terminal_ui_page_starts_the_ui_in_a_terminal_and_shows_its_screen(self):
        steps = self.ui_steps()
        self.assertEqual(len(steps), 4, "the list, the detail view, help, and a revoked key")
        for step in steps:
            self.assertIn("vcon-server tui --config", step.code)
            self.assertIn("connected", step.expect or "", "the screen the page shows")

    def ui_steps(self):
        return [b for b in dt.page_blocks("tui.md") if b.terminal is not None]

    def test_the_terminal_ui_page_opens_each_view_on_the_live_server(self):
        steps = self.ui_steps()
        self.assertEqual(
            [step.keys for step in steps],
            [("q",), ("Enter", "q"), ("?", "q"), ("q",)],
            "the list as it starts, a vCon opened with Enter, help with ?, and the list again",
        )
        self.assertIn("Enter open", steps[0].expect, "the list's own keys")
        self.assertIn("Last updated  <updated>", steps[1].expect, "the detail view")
        self.assertIn("Show or close this help", steps[2].expect, "the help view")

    def test_the_views_are_opened_only_once_the_list_is_drawn(self):
        # `connected` is written before the first page arrives; Enter then
        # opens nothing. The header of the list is written with its rows.
        for step in self.ui_steps()[:3]:
            self.assertEqual(step.terminal, "UUID")

    def test_the_terminal_ui_page_starts_the_ui_with_a_revoked_key(self):
        blocks = dt.page_blocks("tui.md")
        last = self.ui_steps()[-1]
        before = blocks[blocks.index(last) - 1]
        self.assertIn("key revoke", before.code, "the step before it revokes the key")
        self.assertIn("the server refused the key (401)", last.expect)
        self.assertIn("not connected", last.expect)
        self.assertEqual(last.exit, 0, "q still ends it with code 0")

    def test_every_screen_the_terminal_ui_page_expects_ends_with_how_to_quit(self):
        for step in self.ui_steps():
            self.assertTrue(step.expect.rstrip().endswith("q quit"), step.at)

    # Edge.

    def test_a_dnf_scenario_without_a_terminal_step_installs_nothing(self):
        steps = dt.parse("```sh\nls\n```\n", "p.md")
        self.assertEqual(dt.runner_prelude(self.scenario("fedora:44", {"dnf"}), steps), "")

    def test_an_apt_scenario_with_a_terminal_step_installs_nothing_more(self):
        # Debian's and Ubuntu's script(1) is in bsdutils, which every image has.
        steps = dt.parse(TERMINAL_PAGE, "p.md")
        self.assertEqual(dt.runner_prelude(self.scenario("debian:trixie-slim", {"apt"}), steps), dt.APT_RETRIES)

    def test_a_host_scenario_with_a_terminal_step_installs_nothing(self):
        steps = dt.parse(TERMINAL_PAGE, "p.md")
        self.assertEqual(dt.runner_prelude(self.scenario("host", {"dnf"}), steps), "")



UBUNTU = "http://archive.ubuntu.com/ubuntu/"
UBUNTU_SECURITY = "http://security.ubuntu.com/ubuntu/"
UBUNTU_PORTS = "http://ports.ubuntu.com/ubuntu-ports/"
DEBIAN = "http://deb.debian.org/debian"
DEBIAN_SECURITY = "http://deb.debian.org/debian-security"


class Stop(Exception):
    """Raised by a test's stand-in to end a run at the point under test."""


class MirrorTable(unittest.TestCase):
    """The table of mirrors is one file, scripts/apt-mirrors.txt, read here
    and by scripts/apt-mirror-setup.sh, which the release's smoke containers
    run: two tables would name different hosts one day."""

    A = "http://a.example.com/debian"
    B = "http://b.example.com/debian"

    def refused(self, text, said):
        with self.assertRaises(dt.PageError) as caught:
            dt.load_apt_mirrors(text, "scripts/apt-mirrors.txt")
        self.assertIn(said, str(caught.exception), "what the refusal says")

    def test_a_line_is_an_origin_and_its_mirrors_in_their_order(self):
        self.assertEqual(dt.load_apt_mirrors(f"{self.A} {self.B} {self.A}\n", "t"), {self.A: (self.B, self.A)}, "one line")

    def test_the_origins_keep_the_order_of_the_lines(self):
        table = dt.load_apt_mirrors(f"{self.B} {self.B} {self.A}\n{self.A} {self.A} {self.B}\n", "t")
        self.assertEqual(list(table), [self.B, self.A], "the order of the origins")

    def test_comments_empty_lines_and_runs_of_spaces_are_passed_over(self):
        table = dt.load_apt_mirrors(f"# why\n\n  # indented\n{self.A}   {self.A}\t{self.B}  \n", "t")
        self.assertEqual(table, {self.A: (self.A, self.B)}, "the table")

    def test_the_real_file_is_the_runner_s_table(self):
        with open(os.path.join(dt.ROOT, dt.APT_MIRRORS_NAME), encoding="utf-8") as f:
            table = dt.load_apt_mirrors(f.read(), dt.APT_MIRRORS_NAME)
        self.assertEqual(dt.APT_MIRRORS_NAME, "scripts/apt-mirrors.txt", "the file's name")
        self.assertEqual(table, dt.APT_MIRRORS, "the file and APT_MIRRORS")
        self.assertEqual(list(table), [UBUNTU, UBUNTU_SECURITY, UBUNTU_PORTS, DEBIAN, DEBIAN_SECURITY], "the origins")

    def test_the_runner_holds_no_second_table(self):
        with open(dt.__file__, encoding="utf-8") as f:
            source = f.read()
        for host in ("azure.archive.ubuntu.com", "ftp.us.debian.org", "mirrors.edge.kernel.org", "security.debian.org"):
            self.assertNotIn(host, source, f"{host} is written in docs_test.py and in the table")

    def test_an_origin_with_no_mirror_is_refused_with_its_line(self):
        self.refused(f"# one\n{self.A}\n", f"scripts/apt-mirrors.txt:2: {self.A} has no mirror")

    def test_an_origin_written_twice_is_refused(self):
        self.refused(f"{self.A} {self.A} {self.B}\n{self.A} {self.B} {self.A}\n", f"scripts/apt-mirrors.txt:2: {self.A} is written twice")

    def test_an_origin_that_is_not_one_of_its_mirrors_is_refused(self):
        self.refused(f"{self.A} {self.B} http://c.example.com/debian\n", f"scripts/apt-mirrors.txt:1: {self.A} is not among its own mirrors")

    def test_an_origin_on_one_host_is_refused(self):
        self.refused(f"{self.A} {self.A} {self.A}\n", f"scripts/apt-mirrors.txt:1: {self.A} has one host")

    def test_a_mirror_that_is_not_an_http_address_is_refused(self):
        self.refused(f"{self.A} {self.A} https://b.example.com/debian\n", "scripts/apt-mirrors.txt:1: https://b.example.com/debian is not an http:// address")

    def test_a_table_with_no_origin_is_refused(self):
        self.refused("# nothing yet\n", "scripts/apt-mirrors.txt: no origin")


class PackageMirrors(unittest.TestCase):
    """A scenario's container must not depend on one outside host for its
    packages: the runner gives apt a list of mirrors that answer now, and
    says so when none does. On 2026-11-10 archive.ubuntu.com and
    security.ubuntu.com did not answer on port 80, `apt-get update` in the
    release-ubuntu scenario hung for the scenario's 900 s, twice, and the
    failure read as the install page's."""

    def scenario(self, where, tags=("apt",)):
        return dt.Scenario("x", where, frozenset(tags), ("install.md",), binary=False)

    # Success.

    def test_ubuntu_on_x86_64_reads_the_archive_and_the_security_hosts(self):
        self.assertEqual(dt.apt_origins("ubuntu:24.04", "x86_64"), (UBUNTU, UBUNTU_SECURITY))

    def test_debian_and_the_systemd_image_read_debian_and_its_security_archive(self):
        self.assertEqual(dt.apt_origins("debian:trixie-slim", "x86_64"), (DEBIAN, DEBIAN_SECURITY))
        self.assertEqual(dt.apt_origins("systemd", "aarch64"), (DEBIAN, DEBIAN_SECURITY))

    def test_the_mirror_githubs_runners_use_is_asked_first_for_the_ubuntu_archive(self):
        self.assertEqual(dt.APT_MIRRORS[UBUNTU][:2], ("http://azure.archive.ubuntu.com/ubuntu/", UBUNTU))

    def test_ubuntu_on_x86_64_reads_its_suites_from_the_archive_and_the_security_hosts(self):
        self.assertEqual(
            dt.apt_suites("ubuntu:24.04", "x86_64"),
            {UBUNTU: ("noble", "noble-updates", "noble-backports"), UBUNTU_SECURITY: ("noble-security",)},
        )

    def test_ubuntu_on_arm_reads_every_suite_from_the_ports_host(self):
        self.assertEqual(dt.apt_suites("ubuntu:24.04", "aarch64"), {UBUNTU_PORTS: ("noble", "noble-updates", "noble-backports", "noble-security")})

    def test_debian_and_the_systemd_image_read_the_trixie_suites(self):
        expected = {DEBIAN: ("trixie", "trixie-updates"), DEBIAN_SECURITY: ("trixie-security",)}
        self.assertEqual(dt.apt_suites("debian:trixie-slim", "x86_64"), expected)
        self.assertEqual(dt.apt_suites("systemd", "aarch64"), expected)

    def test_the_origins_are_those_of_the_suites(self):
        for where in ("ubuntu:24.04", "debian:trixie-slim", "systemd", "fedora:44", "host"):
            for machine in ("x86_64", "aarch64"):
                self.assertEqual(dt.apt_origins(where, machine), tuple(dt.apt_suites(where, machine)), (where, machine))

    def said(self, chosen, notes=()):
        """What scripts/apt-mirror-setup.sh prints for `chosen`, after the
        lines `notes`."""
        return "".join(f"apt-mirror-setup: {n}\n" for n in notes) + "".join(
            f"apt-mirror-setup: packages from {o} come from {', then '.join(m)}\n" for o, m in chosen.items()
        )

    def test_the_mirrors_the_script_keeps_are_kept_in_its_order(self):
        chosen = {UBUNTU: ["http://b.example.com/ubuntu/", UBUNTU], UBUNTU_SECURITY: [UBUNTU_SECURITY]}
        got = dt.choose_mirrors({UBUNTU: ("noble",), UBUNTU_SECURITY: ("noble-security",)}, ask=lambda s: (0, self.said(chosen), ""))
        self.assertEqual(got, (chosen, []))

    def test_the_script_s_other_lines_are_the_run_s_notes(self):
        chosen = {DEBIAN: [DEBIAN]}
        out = self.said(chosen, ["left out http://b.example.com/debian: its InRelease for trixie is not the one http://deb.debian.org/debian serves (a mirror in the middle of a sync)"])
        _, notes = dt.choose_mirrors({DEBIAN: ("trixie",)}, ask=lambda s: (0, out, ""))
        self.assertEqual(notes, ["docs-test: left out http://b.example.com/debian: its InRelease for trixie is not the one http://deb.debian.org/debian serves (a mirror in the middle of a sync)"])

    def test_the_script_is_asked_about_the_suites_of_the_run(self):
        asked = []
        suites = {DEBIAN: ("trixie", "trixie-updates")}
        dt.choose_mirrors(suites, ask=lambda s: asked.append(s) or (0, self.said({DEBIAN: [DEBIAN]}), ""))
        self.assertEqual(asked, [suites])

    def test_the_question_is_a_dry_run_of_the_script_on_a_system_of_those_suites(self):
        seen = {}

        def run(command, **options):
            root = command[command.index("--root") + 1]
            with open(os.path.join(root, "etc/apt/sources.list"), encoding="utf-8") as f:
                seen["sources"] = f.read()
            seen["command"] = command
            return subprocess.CompletedProcess(command, 0, "", "")

        with unittest.mock.patch.object(dt.subprocess, "run", run):
            dt.ask_mirrors({DEBIAN: ("trixie", "trixie-updates"), DEBIAN_SECURITY: ("trixie-security",)}, timeout=7)
        self.assertEqual(
            seen["sources"],
            f"deb {DEBIAN} trixie main\ndeb {DEBIAN} trixie-updates main\ndeb {DEBIAN_SECURITY} trixie-security main\n",
        )
        command = seen["command"]
        self.assertEqual(command[:2], ["bash", os.path.join(dt.ROOT, "scripts/apt-mirror-setup.sh")])
        self.assertIn("--dry-run", command)
        self.assertEqual(command[command.index("--timeout") + 1], "7")

    def test_the_setup_writes_one_list_of_mirrors_for_each_origin(self):
        setup = dt.apt_mirror_setup({DEBIAN: ["http://a.example.com/debian"], DEBIAN_SECURITY: ["http://b.example.com/debian-security"]})
        written = [c.rsplit(" > ", 1) for c in setup if c.startswith("printf ")]
        self.assertEqual([where for _, where in written], ["/etc/apt/docs-test-mirrors-0.list", "/etc/apt/docs-test-mirrors-1.list"])
        self.assertIn("'http://a.example.com/debian'", written[0][0])
        self.assertIn("'http://b.example.com/debian-security'", written[1][0])

    def rewritten(self, chosen, name, text):
        """The file `name` under /etc/apt after the setup ran on a copy of it."""
        root = tempfile.mkdtemp()
        try:
            path = os.path.join(root, "etc/apt", name)
            os.makedirs(os.path.dirname(path))
            os.makedirs(os.path.join(root, "etc/apt/sources.list.d"), exist_ok=True)
            os.makedirs(os.path.join(root, "etc/apt/apt.conf.d"), exist_ok=True)
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            script = "set -e\n" + "\n".join(dt.apt_mirror_setup(chosen, root=root)) + "\n"
            done = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=30)
            self.assertEqual(done.returncode, 0, done.stderr)
            with open(path, encoding="utf-8") as f:
                return f.read()
        finally:
            shutil.rmtree(root)

    def listed(self, mirrors):
        """The list of mirrors the setup writes for one origin, as apt reads it."""
        root = tempfile.mkdtemp()
        try:
            os.makedirs(os.path.join(root, "etc/apt/sources.list.d"))
            script = "set -e\n" + "\n".join(dt.apt_mirror_setup({DEBIAN: mirrors}, root=root)) + "\n"
            # sh, not bash: the same commands are a Dockerfile's RUN.
            done = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
            self.assertEqual(done.returncode, 0, done.stderr)
            with open(os.path.join(root, "etc/apt/docs-test-mirrors-0.list"), encoding="utf-8") as f:
                return f.read()
        finally:
            shutil.rmtree(root)

    def test_the_list_gives_each_mirror_its_place_as_a_priority(self):
        # apt's mirror method does not read a list in the order written: of
        # two mirrors with no priority it asked the second for every file
        # (measured on 2026-11-10 with the second one unreachable; the
        # update did not end in 80 s). A lower priority is asked first.
        self.assertEqual(
            self.listed(["http://a.example.com/debian", "http://b.example.com/debian"]),
            "http://a.example.com/debian\tpriority:1\nhttp://b.example.com/debian\tpriority:2\n",
        )

    def test_a_deb822_source_is_pointed_at_the_list_of_mirrors(self):
        after = self.rewritten(
            {UBUNTU_PORTS: ["http://a.example.com/ubuntu-ports/"]},
            "sources.list.d/ubuntu.sources",
            "Types: deb\nURIs: http://ports.ubuntu.com/ubuntu-ports/\nSuites: noble\n",
        )
        self.assertIn("URIs: mirror+file:", after)
        self.assertIn("/etc/apt/docs-test-mirrors-0.list\nSuites: noble\n", after)

    def test_the_run_says_which_mirror_each_origin_gets(self):
        notes = dt.mirror_notes({UBUNTU: ["http://a.example.com/ubuntu/", "http://b.example.com/ubuntu/"]})
        self.assertEqual(
            notes,
            ["docs-test: packages from " + UBUNTU + " come from http://a.example.com/ubuntu/, then http://b.example.com/ubuntu/"],
        )

    def test_a_run_names_the_mirrors_it_chose_before_it_reads_a_page(self):
        (scenario,) = [s for s in dt.SCENARIOS if s.name == "release-ubuntu"]
        azure, kernel, princeton = dt.APT_MIRRORS[UBUNTU_SECURITY][1:]
        chosen = {UBUNTU: [azure, kernel], UBUNTU_SECURITY: [azure, kernel]}
        note = f"left out {princeton}: its InRelease for noble-security is older than another mirror's"
        asked = []
        said = io.StringIO()
        with unittest.mock.patch.object(dt, "ask_mirrors", lambda suites: asked.append(suites) or (0, self.said(chosen, [note]), "")), \
                unittest.mock.patch.object(dt.platform, "machine", lambda: "x86_64"), \
                unittest.mock.patch.object(dt, "page_blocks", side_effect=Stop), \
                contextlib.redirect_stdout(said), self.assertRaises(Stop):
            dt.run(scenario, None, None)
        self.assertEqual(asked, [dt.apt_suites("ubuntu:24.04", "x86_64")], "the suites asked about")
        self.assertEqual(
            said.getvalue().splitlines(),
            [
                f"docs-test: {note}",
                f"docs-test: packages from {UBUNTU} come from {azure}, then {kernel}",
                f"docs-test: packages from {UBUNTU_SECURITY} come from {azure}, then {kernel}",
            ],
            "the script's notes, then the mirrors of each origin",
        )

    def started(self, where, **patches):
        """What a run of an apt scenario with no steps says, up to the
        point where a patched function raises Stop. The script keeps every
        Debian mirror but the origin itself."""
        scenario = dt.Scenario("x", where, frozenset({"apt"}), (), binary=False)
        chosen = {DEBIAN: list(dt.APT_MIRRORS[DEBIAN][1:]), DEBIAN_SECURITY: list(dt.APT_MIRRORS[DEBIAN_SECURITY])}
        said = io.StringIO()
        with contextlib.ExitStack() as stack:
            stack.enter_context(unittest.mock.patch.object(dt, "ask_mirrors", lambda suites: (0, self.said(chosen), "")))
            for name, replacement in patches.items():
                stack.enter_context(unittest.mock.patch.object(dt, name, replacement))
            stack.enter_context(contextlib.redirect_stdout(said))
            stack.enter_context(self.assertRaises(Stop))
            dt.run(scenario, None, None)
        return said.getvalue()

    def test_a_run_gives_the_chosen_mirrors_to_the_containers_setup(self):
        preludes = []

        def script(steps, prelude, sets=None, units=()):
            preludes.append(prelude)
            raise Stop

        self.started("debian:trixie-slim", script=script)
        (prelude,) = preludes
        self.assertIn("printf '%s\\tpriority:%s\\n' 'http://ftp.us.debian.org/debian' 1 'http://ftp.debian.org/debian' 2 > /etc/apt/docs-test-mirrors-0.list\n", prelude)

    def test_a_systemd_run_builds_its_image_through_the_chosen_mirrors(self):
        dockerfiles = []

        def docker(*args, **options):
            if args[0] == "build":
                dockerfiles.append(options["input"].decode())
                raise Stop
            raise AssertionError(f"docker {args[0]} before the image is built")

        self.started("systemd", _docker=docker, pull_with_retry=lambda name: name)
        (dockerfile,) = dockerfiles
        self.assertIn("RUN printf '%s\\tpriority:%s\\n' 'http://ftp.us.debian.org/debian' 1 'http://ftp.debian.org/debian' 2 > /etc/apt/docs-test-mirrors-0.list", dockerfile)

    def test_an_apt_container_gets_the_mirror_setup_before_the_pages(self):
        prelude = dt.runner_prelude(self.scenario("debian:trixie-slim"), (), {DEBIAN: ["http://a.example.com/debian"]})
        self.assertIn("docs-test-mirrors-0.list", prelude)
        self.assertIn('Acquire::Retries "3";', prelude)

    def test_the_systemd_image_is_built_through_the_mirrors_too(self):
        dockerfile = dt.systemd_dockerfile({DEBIAN: ["http://a.example.com/debian"]})
        self.assertLess(
            dockerfile.index("docs-test-mirrors-0.list"),
            dockerfile.index("apt-get update"),
            "the mirrors are set before the image's first download",
        )

    # Failure. How the script tells mirrors apart is tested in
    # scripts/test_apt_mirror_setup.py, against a server that stands for them.

    def unreachable(self, origin):
        return (3, "", f"apt-mirror-setup: no package mirror answered for {origin} (tried {', '.join(dt.APT_MIRRORS[origin])}): "
                "the package mirrors are unreachable from this machine\n")

    def test_no_mirror_for_an_origin_is_an_unreachable_mirror_not_a_page_failure(self):
        with self.assertRaises(dt.MirrorError) as raised:
            dt.choose_mirrors({UBUNTU: ("noble",), UBUNTU_SECURITY: ("noble-security",)}, ask=lambda s: self.unreachable(UBUNTU))
        said = str(raised.exception)
        self.assertIn("no package mirror answered for " + UBUNTU, said)
        self.assertIn("the package mirrors are unreachable from this machine", said)
        self.assertIn("not a failure of the pages", said)
        self.assertNotIn("apt-mirror-setup:", said, "the runner's words, without the script's name")
        for mirror in dt.APT_MIRRORS[UBUNTU]:
            self.assertIn(mirror, said, "each mirror tried is named")

    def test_no_mirror_with_the_newest_copy_is_an_unreachable_mirror_too(self):
        err = f"apt-mirror-setup: no mirror of {DEBIAN} holds the newest InRelease of every suite (tried {DEBIAN}): apt would read some of its files from an older copy\n"
        with self.assertRaisesRegex(dt.MirrorError, "holds the newest InRelease of every suite .*; this is not a failure of the pages"):
            dt.choose_mirrors({DEBIAN: ("trixie",)}, ask=lambda s: (3, "", err))

    def test_a_table_the_script_refuses_is_a_page_error_that_names_its_exit_code(self):
        err = "apt-mirror-setup: scripts/apt-mirrors.txt:3: http://x is not among its own mirrors\n"
        with self.assertRaises(dt.PageError) as raised:
            dt.choose_mirrors({DEBIAN: ("trixie",)}, ask=lambda s: (2, "", err))
        self.assertNotIsInstance(raised.exception, dt.MirrorError, "a wrong table is not an outage")
        self.assertIn("scripts/apt-mirror-setup.sh exited 2", str(raised.exception))
        self.assertIn("is not among its own mirrors", str(raised.exception))

    def test_a_system_the_table_does_not_know_is_a_page_error(self):
        with self.assertRaisesRegex(dt.PageError, "exited 4"):
            dt.choose_mirrors({DEBIAN: ("trixie",)}, ask=lambda s: (4, "", "apt-mirror-setup: no package source of this system is in the table\n"))

    def test_an_origin_the_script_names_no_mirror_for_is_a_page_error(self):
        # Exit 0 with an origin missing from the log would leave that
        # origin's source on one host.
        with self.assertRaisesRegex(dt.PageError, f"named no mirror for {re.escape(UBUNTU_SECURITY)}"):
            dt.choose_mirrors({UBUNTU: ("noble",), UBUNTU_SECURITY: ("noble-security",)}, ask=lambda s: (0, self.said({UBUNTU: [UBUNTU]}), ""))

    def test_a_script_that_does_not_end_in_time_is_a_mirror_error(self):
        def run(command, **options):
            raise subprocess.TimeoutExpired(command, options.get("timeout"))

        with unittest.mock.patch.object(dt.subprocess, "run", run), self.assertRaisesRegex(dt.MirrorError, "did not end within"):
            dt.choose_mirrors({DEBIAN: ("trixie",)})

    def test_an_image_without_a_known_release_name_is_a_page_error(self):
        with self.assertRaisesRegex(dt.PageError, "ubuntu:20.04"):
            dt.apt_suites("ubuntu:20.04", "x86_64")
        with self.assertRaisesRegex(dt.PageError, "debian:12"):
            dt.apt_suites("debian:12", "x86_64")

    def test_a_run_with_no_mirror_ends_at_once_and_says_the_mirrors_are_unreachable(self):
        (scenario,) = [s for s in dt.SCENARIOS if s.name == "release-ubuntu"]
        said = io.StringIO()
        with unittest.mock.patch.object(dt, "ask_mirrors", lambda suites: self.unreachable(UBUNTU)), \
                unittest.mock.patch.object(dt, "_docker", side_effect=AssertionError("no container is started")), \
                unittest.mock.patch.object(dt, "released", side_effect=AssertionError("nothing else is asked")), \
                contextlib.redirect_stdout(said), contextlib.redirect_stderr(said):
            code = dt.run(scenario, None, None)
        self.assertEqual(code, 3, "its own exit code")
        self.assertIn("docs-test: release-ubuntu: no package mirror answered", said.getvalue())

    def test_apt_gives_up_a_download_that_stalls(self):
        # Without a timeout apt waits on a mirror that accepts a connection
        # and sends nothing, and its retries never start. Both numbers are
        # small because apt spends them on every file before it asks the
        # next mirror.
        prelude = dt.runner_prelude(self.scenario("ubuntu:24.04"), (), {})
        self.assertIn('Acquire::http::Timeout "10";', prelude)
        self.assertIn('Acquire::https::Timeout "10";', prelude)
        self.assertIn('Acquire::Retries "3";', prelude)

    def test_the_systemd_image_is_built_with_the_same_two_limits(self):
        dockerfile = dt.systemd_dockerfile({})
        for command in ("apt-get update", "apt-get install"):
            self.assertIn(f"{command} -qq -o Acquire::Retries=3 -o Acquire::http::Timeout=10 ", dockerfile.replace("install -y", "install"))

    # Edge.

    def test_ubuntu_on_arm_reads_the_ports_host(self):
        self.assertEqual(dt.apt_origins("ubuntu:24.04", "aarch64"), (UBUNTU_PORTS,))

    def test_fedora_and_the_host_have_no_apt_origin(self):
        self.assertEqual(dt.apt_origins("fedora:44", "x86_64"), ())
        self.assertEqual(dt.apt_origins("host", "x86_64"), ())

    def test_every_origin_is_one_of_its_own_mirrors_and_has_another(self):
        for origin, mirrors in dt.APT_MIRRORS.items():
            self.assertIn(origin, mirrors, origin)
            self.assertGreaterEqual(len(set(mirrors)), 2, f"{origin} depends on one host")

    def test_the_debian_origin_does_not_take_the_security_archives_line(self):
        after = self.rewritten(
            {DEBIAN: ["http://a.example.com/debian"]},
            "sources.list.d/debian.sources",
            "URIs: http://deb.debian.org/debian\nSuites: trixie\n\nURIs: http://deb.debian.org/debian-security\nSuites: trixie-security\n",
        )
        self.assertIn("URIs: http://deb.debian.org/debian-security\n", after, "one origin is a prefix of the other")
        self.assertEqual(after.count("mirror+file:"), 1)

    def test_the_setup_ends_0_as_the_runner_runs_it_on_an_image_with_no_sources_list(self):
        # debian:trixie-slim has /etc/apt/sources.list.d/debian.sources and
        # no /etc/apt/sources.list. The runner's setup runs under
        # `set -eo pipefail` (see script), where one failing command ends
        # the run before the first page.
        root = tempfile.mkdtemp()
        try:
            source = os.path.join(root, "etc/apt/sources.list.d/debian.sources")
            os.makedirs(os.path.dirname(source))
            with open(source, "w", encoding="utf-8") as f:
                f.write("URIs: http://deb.debian.org/debian\nSuites: trixie\n")
            setup = "".join(c + "\n" for c in dt.apt_mirror_setup({DEBIAN: ["http://a.example.com/debian"]}, root=root))
            done = subprocess.run(["bash", "-c", "set -eo pipefail\n" + setup + "echo reached\n"], capture_output=True, text=True, timeout=30)
            self.assertEqual((done.returncode, done.stdout, done.stderr), (0, "reached\n", ""))
            with open(source, encoding="utf-8") as f:
                self.assertEqual(f.read(), f"URIs: mirror+file:{root}/etc/apt/docs-test-mirrors-0.list\nSuites: trixie\n")
        finally:
            shutil.rmtree(root)

    def test_the_setup_does_not_rewrite_its_own_list_of_mirrors(self):
        # The origin is one of its own mirrors, so its address is in the list.
        self.assertEqual(self.listed(["http://deb.debian.org/debian"]), "http://deb.debian.org/debian\tpriority:1\n")

    def test_the_security_archive_is_pointed_at_its_own_list(self):
        after = self.rewritten(
            {DEBIAN: ["http://a.example.com/debian"], DEBIAN_SECURITY: ["http://b.example.com/debian-security"]},
            "sources.list.d/debian.sources",
            "URIs: http://deb.debian.org/debian\nSuites: trixie\n\nURIs: http://deb.debian.org/debian-security\nSuites: trixie-security\n",
        )
        self.assertRegex(after, r"^URIs: mirror\+file:\S+-0\.list\nSuites: trixie\n\nURIs: mirror\+file:\S+-1\.list\nSuites: trixie-security\n$")

    def test_a_one_line_source_is_pointed_at_the_list_of_mirrors(self):
        after = self.rewritten(
            {UBUNTU: ["http://a.example.com/ubuntu/"]},
            "sources.list",
            "deb http://archive.ubuntu.com/ubuntu/ jammy main restricted\n",
        )
        self.assertRegex(after, r"^deb mirror\+file:\S+/etc/apt/docs-test-mirrors-0\.list jammy main restricted\n$")

    def test_a_source_on_another_host_is_left_as_it_is(self):
        text = "deb [signed-by=/usr/share/keyrings/x.gpg] https://cli.github.com/packages stable main\n"
        self.assertEqual(self.rewritten({UBUNTU: ["http://a.example.com/ubuntu/"]}, "sources.list.d/github-cli.list", text), text)

    def test_an_origin_written_without_its_last_slash_is_found(self):
        after = self.rewritten(
            {UBUNTU: ["http://a.example.com/ubuntu/"]},
            "sources.list",
            "deb http://archive.ubuntu.com/ubuntu jammy main\n",
        )
        self.assertIn("mirror+file:", after)

    def test_a_host_scenario_gets_no_mirror_setup(self):
        self.assertEqual(dt.runner_prelude(self.scenario("host", ("docker",)), (), {DEBIAN: ["http://a.example.com/debian"]}), "")

    def test_a_run_on_a_system_without_apt_asks_no_mirror(self):
        for where, tags in (("fedora:44", {"dnf"}), ("host", {"docker"})):
            scenario = dt.Scenario("x", where, frozenset(tags), (), binary=False)
            asked = []
            said = io.StringIO()
            with unittest.mock.patch.object(dt, "ask_mirrors", lambda suites: asked.append(suites)), \
                    unittest.mock.patch.object(dt, "script", side_effect=Stop), \
                    contextlib.redirect_stdout(said), self.assertRaises(Stop):
                dt.run(scenario, None, None)
            self.assertEqual((asked, said.getvalue()), ([], ""), where)

    def test_a_dnf_scenario_gets_no_apt_setup(self):
        self.assertEqual(dt.runner_prelude(self.scenario("fedora:44", ("dnf",)), (), {}), "")

    def test_a_system_without_apt_has_no_suite(self):
        self.assertEqual(dt.apt_suites("fedora:44", "x86_64"), {})
        self.assertEqual(dt.apt_suites("host", "aarch64"), {})

    def test_no_suite_asks_the_script_nothing(self):
        asked = []
        self.assertEqual(dt.choose_mirrors({}, ask=lambda s: asked.append(s)), ({}, []))
        self.assertEqual(asked, [], "a system without apt runs no script")

    def test_a_mirror_address_with_a_comma_in_its_path_is_read_whole(self):
        odd = "http://a.example.com/x,y/debian"
        chosen, _ = dt.choose_mirrors({DEBIAN: ("trixie",)}, ask=lambda s: (0, self.said({DEBIAN: [odd, DEBIAN]}), ""))
        self.assertEqual(chosen, {DEBIAN: [odd, DEBIAN]})

    def test_the_script_s_stderr_on_success_is_passed_on_as_notes(self):
        _, notes = dt.choose_mirrors({DEBIAN: ("trixie",)}, ask=lambda s: (0, self.said({DEBIAN: [DEBIAN]}), "apt-mirror-setup: a warning\n"))
        self.assertEqual(notes, ["docs-test: a warning"])


# ---------------------------------------------------------------------------
# The scenarios a push runs (`docs_test.py push`, from .githooks/pre-push).
# On #215 a change to what the terminal UI records passed every local gate
# and failed three how-to jobs in CI: the scenarios ran only there.
# ---------------------------------------------------------------------------

def fake(name, pages=("a.md",), **kw):
    return dt.Scenario(name, "debian:trixie-slim", frozenset({"debian"}), tuple(pages), **kw)


FAKES = [
    fake("one", ("a.md", "b.md")),
    fake("two", ("b.md",), features=("postgres",)),
    fake("pages-only", ("c.md",), binary=False),
    fake("tokened", ("d.md",)),
    fake("long", ("e.md",), binary=False, timeout=3600),
]


def fake_steps(s):
    """Steps for FAKES: `tokened` calls gh attestation verify."""
    code = "gh attestation verify x --repo NormB/vcon-server\n" if s.name == "tokened" else "ls\n"
    return [dt.Block("sh", code, f"docs/{s.pages[0]}:1")]


def needed_of(changed, token=False, scenarios=FAKES):
    run, skipped = dt.needed(changed, scenarios, token, fake_steps)
    return [n for n, _ in run], [n for n, _ in skipped]


class Needed(unittest.TestCase):
    # Success: what a push runs.
    def test_a_page_change_runs_the_scenarios_that_show_it(self):
        self.assertEqual(needed_of(["website/content/docs/b.md"]), (["one", "two"], []), "both show b.md")

    def test_a_page_of_one_scenario_runs_that_one(self):
        self.assertEqual(needed_of(["website/content/docs/c.md"]), (["pages-only"], []), "c.md alone")

    def test_a_server_source_change_runs_every_scenario_with_the_server_under_test(self):
        run, skipped = needed_of(["crates/vcon-server/src/serve.rs"])
        self.assertEqual(run, ["one", "two"], "the scenarios given the pushed server")
        self.assertEqual(skipped, ["tokened"], "the one that needs a token is named")

    def test_the_lock_file_is_server_source(self):
        self.assertEqual(needed_of(["Cargo.lock"])[0], ["one", "two"], "dependencies build the server")

    def test_the_workspace_manifest_and_toolchain_are_server_source(self):
        for path in ("Cargo.toml", "rust-toolchain.toml", "crates/vcon/Cargo.toml", ".cargo/config.toml"):
            with self.subTest(path=path):
                self.assertEqual(needed_of([path])[0], ["one", "two"], path)

    def test_the_vcon_library_is_server_source(self):
        self.assertEqual(needed_of(["crates/vcon/src/read.rs"])[0], ["one", "two"], "the server links it")

    def test_the_console_assets_are_server_source(self):
        self.assertEqual(needed_of(["crates/vcon-server/console/console.js"])[0], ["one", "two"], "embedded")

    def test_a_runner_change_runs_every_scenario_it_can(self):
        for path in ("scripts/docs_test.py", "scripts/docs-test-pins.txt", "scripts/fetch-pinned.sh", "scripts/apt-mirrors.txt"):
            with self.subTest(path=path):
                self.assertEqual(needed_of([path]), (["one", "two", "pages-only"], ["tokened", "long"]), path)

    def test_with_a_token_the_scenarios_that_call_gh_run(self):
        self.assertEqual(needed_of(["website/content/docs/d.md"], token=True), (["tokened"], []), "GH_TOKEN set")

    def test_each_scenario_runs_once_in_the_order_listed(self):
        changed = ["website/content/docs/b.md", "crates/vcon-server/src/api.rs", "website/content/docs/a.md"]
        self.assertEqual(needed_of(changed)[0], ["one", "two"], "once each, SCENARIOS order")

    def test_the_reason_names_the_page(self):
        run, _ = dt.needed(["website/content/docs/b.md"], FAKES, False, fake_steps)
        self.assertEqual(dict(run)["two"], "the push changes docs/b.md, which it runs", "the page")

    def test_the_reason_names_the_source_file(self):
        run, _ = dt.needed(["Cargo.lock"], FAKES, False, fake_steps)
        self.assertEqual(
            dict(run)["one"], "the push changes Cargo.lock, which the server under test is built from", "the file"
        )

    def test_the_real_scenarios_for_a_server_change(self):
        run, _ = dt.needed(["crates/vcon-server/src/serve.rs"], dt.SCENARIOS, True)
        names = [n for n, _ in run]
        for name in ("foreground", "foreground-fedora", "systemd", "upgrade", "s3", "s3-minio", "postgres", "parquet"):
            self.assertIn(name, names, f"{name} gets the server under test")
        for name in ("container", "release-debian", "cargo-debian", "source-fedora"):
            self.assertNotIn(name, names, f"{name} runs a published release or builds its own")

    # Failure: what a push does not run.
    def test_a_test_only_change_runs_none(self):
        self.assertEqual(needed_of(["crates/vcon-server/tests/serve.rs"]), ([], []), "tests are not the server")

    def test_a_test_fixture_change_runs_none(self):
        self.assertEqual(needed_of(["crates/vcon/tests/fixtures/ietf/a.vcon"]), ([], []), "fixtures are not built in")

    def test_an_unrelated_script_runs_none(self):
        self.assertEqual(needed_of(["scripts/check_changelog.py"]), ([], []), "not the runner")

    def test_the_readme_and_changelog_run_none(self):
        self.assertEqual(needed_of(["README.md", "CHANGELOG.md"]), ([], []), "no scenario shows them")

    def test_a_page_no_scenario_runs_runs_none(self):
        self.assertEqual(needed_of(["website/content/docs/z.md"]), ([], []), "z.md is in no scenario")

    def test_a_page_of_the_same_name_elsewhere_runs_none(self):
        self.assertEqual(needed_of(["docs/b.md", "website/content/b.md"]), ([], []), "only the docs section")

    def test_a_path_that_only_starts_like_the_source_runs_none(self):
        self.assertEqual(needed_of(["crates-notes.md", "Cargo.toml.orig"]), ([], []), "a name, not a directory")

    def test_the_fuzz_package_runs_none(self):
        self.assertEqual(needed_of(["fuzz/Cargo.lock", "fuzz/fuzz_targets/read.rs"]), ([], []), "not the server")

    def test_no_change_runs_none(self):
        self.assertEqual(needed_of([]), ([], []), "nothing pushed")

    def test_without_a_token_a_scenario_that_calls_gh_is_skipped(self):
        run, skipped = dt.needed(["website/content/docs/d.md"], FAKES, False, fake_steps)
        self.assertEqual(run, [], "it cannot verify without a token")
        self.assertEqual(
            skipped,
            [("tokened", "its steps call gh, which needs GH_TOKEN here; CI runs it")],
            "the reason says what to set",
        )

    def test_a_long_scenario_is_skipped_even_with_a_token(self):
        run, skipped = dt.needed(["website/content/docs/e.md"], FAKES, True, fake_steps)
        self.assertEqual(run, [], "an hour is no push hook")
        self.assertEqual(skipped, [("long", "it may take up to 60 min; CI runs it")], "the bound is named")

    def test_the_real_release_scenarios_need_a_token(self):
        run, skipped = dt.needed(["website/content/docs/install.md"], dt.SCENARIOS, False)
        names = dict(skipped)
        for name in ("release-debian", "release-ubuntu", "release-fedora", "upgrade"):
            self.assertIn(name, names, f"{name} verifies with gh")
        self.assertEqual(run, [], f"nothing runs without a token: {run}")

    # Edge cases.
    def test_blank_lines_and_spaces_are_ignored(self):
        self.assertEqual(needed_of(["", "  website/content/docs/c.md  ", "\t"]), (["pages-only"], []), "trimmed")

    def test_the_page_reason_comes_before_the_source_reason(self):
        run, _ = dt.needed(["Cargo.lock", "website/content/docs/a.md"], FAKES, False, fake_steps)
        self.assertEqual(dict(run)["one"], "the push changes docs/a.md, which it runs", "the page is named first")

    def test_a_skipped_scenario_is_named_once(self):
        _, skipped = needed_of(["website/content/docs/d.md", "Cargo.lock", "scripts/docs_test.py"])
        self.assertEqual(skipped, ["tokened", "long"], "once each")

    def test_gh_that_needs_no_token_does_not_skip(self):
        steps = lambda s: [dt.Block("sh", "gh attestation --help | sed -n 1p\ngh --version\n", "docs/d.md:1")]
        run, skipped = dt.needed(["website/content/docs/d.md"], FAKES, False, steps)
        self.assertEqual(([n for n, _ in run], skipped), (["tokened"], []), "help and version read nothing")

    def test_gh_api_and_release_need_a_token(self):
        for code in ("gh api repos/NormB/vcon-server/releases\n", "gh release download v0.4.6\n"):
            with self.subTest(code=code):
                steps = lambda s, code=code: [dt.Block("sh", code, "docs/d.md:1")]
                _, skipped = dt.needed(["website/content/docs/d.md"], FAKES, False, steps)
                self.assertEqual([n for n, _ in skipped], ["tokened"], code)

    def test_a_skipped_step_that_calls_gh_does_not_skip(self):
        steps = lambda s: [dt.Block("sh", "gh api x\n", "docs/d.md:1", skip="a reader's own")]
        run, _ = dt.needed(["website/content/docs/d.md"], FAKES, False, steps)
        self.assertEqual([n for n, _ in run], ["tokened"], "a skipped step never runs")

    def test_a_build_script_is_server_source(self):
        self.assertEqual(needed_of(["crates/vcon-server/build.rs"])[0], ["one", "two"], "a build script builds it")

    def test_a_crate_named_tests_is_still_source(self):
        self.assertEqual(needed_of(["crates/vcon-server/src/tests.rs"])[0], ["one", "two"], "src/, not tests/")


class FakeRun:
    """Stands in for subprocess.run: answers each command from `answers`
    by its first words and records every command."""

    def __init__(self, answers=None):
        self.calls = []
        self.answers = answers or {}

    def __call__(self, cmd, **kw):
        self.calls.append(list(cmd))
        key = " ".join(cmd[:2]) if cmd[0] != sys.executable else f"run {cmd[cmd.index('run') + 1]}"
        code, out = self.answers.get(key, (0, ""))
        if key == "git rev-parse" and key not in self.answers:
            code, out = 0, "abc123\n"
        return subprocess.CompletedProcess(cmd, code, out, "")

    def names(self, first):
        return [c for c in self.calls if c[0] == first]


def push_of(changed, run, shas=("abc123",), env=None, scenarios=FAKES, stamped=None, stamps=None, commits=None, delta=None,
            test_only=None):
    out = io.StringIO()
    stamps = stamps or {}
    made = []
    stamped = stamped if stamped is not None else []

    def tempdir():
        d = tempfile.mkdtemp(prefix="docs-push-test-")
        made.append(d)
        return d

    code = dt.push(
        list(shas),
        changed,
        run=run,
        env=env or {},
        scenarios=scenarios,
        steps=fake_steps,
        copy=lambda a, b: open(b, "w").close(),
        tempdir=tempdir,
        out=out,
        clock=iter(range(0, 1000, 7)).__next__,
        stamp=lambda sha, entries: stamped.append((sha, dict(entries))),
        read=lambda sha: stamps.get(sha),
        commits=commits,
        delta=delta,
        test_only=test_only,
    )
    return code, out.getvalue(), made


class Push(unittest.TestCase):
    # Success.
    def test_nothing_needed_runs_nothing(self):
        run = FakeRun()
        code, out, _ = push_of(["README.md"], run)
        self.assertEqual(code, 0, "nothing to run is a pass")
        self.assertEqual(run.calls, [], "not even git")
        self.assertIn("docs scenarios: not needed", out, "it says so")

    def test_a_page_change_builds_once_and_runs_its_scenario(self):
        run = FakeRun()
        code, out, _ = push_of(["website/content/docs/c.md"], run)
        self.assertEqual(code, 0, out)
        self.assertEqual(run.names("cargo"), [["cargo", "build", "--locked", "-p", "vcon-server"]], "one build")
        runs = run.names(sys.executable)
        self.assertEqual([c[c.index("run") + 1] for c in runs], ["pages-only"], "its scenario")

    def test_each_scenario_gets_the_build_with_its_features(self):
        run = FakeRun()
        code, out, _ = push_of(["website/content/docs/b.md"], run)
        self.assertEqual(code, 0, out)
        self.assertEqual(
            run.names("cargo"),
            [["cargo", "build", "--locked", "-p", "vcon-server"],
             ["cargo", "build", "--locked", "-p", "vcon-server", "--features", "postgres"]],
            "one build per feature set",
        )
        binaries = {c[c.index("run") + 1]: c[c.index("--binary") + 1] for c in run.names(sys.executable)}
        self.assertNotEqual(binaries["one"], binaries["two"], "two builds, two binaries")
        self.assertTrue(binaries["two"].endswith("postgres/vcon-server"), binaries)

    def test_scenarios_with_the_same_features_share_one_build(self):
        run = FakeRun()
        code, out, _ = push_of(["scripts/docs_test.py"], run)
        self.assertEqual(code, 0, out)
        self.assertEqual(len(run.names("cargo")), 2, "one and pages-only share the default build")
        self.assertEqual(len(run.names(sys.executable)), 3, "three scenarios ran")

    def test_each_result_is_printed_with_its_time(self):
        code, out, _ = push_of(["website/content/docs/b.md"], FakeRun())
        self.assertRegex(out, r"docs scenario one: ok in \d+ s \(the push changes docs/b\.md, which it runs\)", out)
        self.assertIn("docs scenarios: 2 of 2 ran as their pages say", out, "the total")

    def test_skipped_scenarios_are_named_beside_the_run(self):
        code, out, _ = push_of(["Cargo.lock"], FakeRun())
        self.assertEqual(code, 0, out)
        self.assertIn(
            "docs scenarios: skipped tokened: its steps call gh, which needs GH_TOKEN here; CI runs it", out, out
        )

    def test_a_token_in_the_environment_runs_the_gh_scenario(self):
        run = FakeRun()
        code, out, _ = push_of(["website/content/docs/d.md"], run, env={"GH_TOKEN": "x"})
        self.assertEqual(code, 0, out)
        self.assertEqual([c[c.index("run") + 1] for c in run.names(sys.executable)], ["tokened"], out)

    def test_the_scenarios_run_from_the_repository_root(self):
        calls = []

        def run(cmd, **kw):
            calls.append(kw.get("cwd"))
            return FakeRun()(cmd, **kw)

        push_of(["website/content/docs/c.md"], run)
        self.assertEqual(set(calls), {dt.ROOT}, "every command in the root")

    # Failure.
    def test_a_failing_scenario_refuses_the_push_and_is_named(self):
        run = FakeRun({"run one": (1, "FAIL     docs/b.md:3: line 1 differs\n")})
        code, out, _ = push_of(["website/content/docs/b.md"], run)
        self.assertEqual(code, 1, "refused")
        self.assertIn("docs scenario one: FAILED in", out, out)
        self.assertIn("FAIL     docs/b.md:3: line 1 differs", out, "its report is shown")
        self.assertIn("docs scenarios: 1 of 2 ran as their pages say", out, "the rest still ran")

    def test_a_scenario_that_cannot_start_refuses(self):
        run = FakeRun({"run two": (2, "docs-test: no scenario two; see `list`\n")})
        code, out, _ = push_of(["website/content/docs/b.md"], run)
        self.assertEqual(code, 1, "exit 2 is a failure too")
        self.assertIn("docs scenario two: FAILED", out, out)

    def test_a_head_other_than_the_pushed_commit_refuses_before_building(self):
        run = FakeRun({"git rev-parse": (0, "fff999\n")})
        code, out, _ = push_of(["website/content/docs/c.md"], run)
        self.assertEqual(code, 1, "refused")
        self.assertEqual(run.names("cargo"), [], "no build of another commit")
        self.assertIn("the checkout is at fff999, not the pushed abc123", out, out)

    def test_uncommitted_changes_refuse_before_building(self):
        run = FakeRun({"git status": (0, " M crates/vcon-server/src/serve.rs\n")})
        code, out, _ = push_of(["website/content/docs/c.md"], run)
        self.assertEqual(code, 1, "refused")
        self.assertEqual(run.names("cargo"), [], "the tree is not the commit")
        self.assertIn("uncommitted changes", out, out)

    def test_two_pushed_commits_that_differ_refuse(self):
        run = FakeRun()
        code, out, _ = push_of(["website/content/docs/c.md"], run, shas=("abc123", "def456"))
        self.assertEqual(code, 1, "one checkout cannot be both")
        self.assertIn("push one branch at a time", out, out)

    def test_a_failed_build_refuses_and_runs_no_scenario(self):
        run = FakeRun({"cargo build": (101, "error[E0425]: cannot find value\n")})
        code, out, _ = push_of(["website/content/docs/c.md"], run)
        self.assertEqual(code, 1, "refused")
        self.assertEqual(run.names(sys.executable), [], "nothing to run it with")
        self.assertIn("error[E0425]", out, "the build's error is shown")

    def test_a_git_that_fails_refuses(self):
        run = FakeRun({"git rev-parse": (128, "")})
        code, out, _ = push_of(["website/content/docs/c.md"], run)
        self.assertEqual(code, 1, "cannot tell which commit is checked out")
        self.assertIn("git cannot read which commit is checked out", out, "the reason, not a mismatch")

    # Edge cases.
    def test_the_builds_are_removed_after_a_pass(self):
        _, _, made = push_of(["website/content/docs/b.md"], FakeRun())
        self.assertTrue(made and not any(os.path.exists(d) for d in made), f"removed: {made}")

    def test_the_builds_are_removed_after_a_failure(self):
        _, _, made = push_of(["website/content/docs/b.md"], FakeRun({"run one": (1, "x\n")}))
        self.assertTrue(made and not any(os.path.exists(d) for d in made), f"removed: {made}")

    def test_a_failing_report_shows_only_its_last_lines(self):
        long = "".join(f"line {i}\n" for i in range(200))
        code, out, _ = push_of(["website/content/docs/c.md"], FakeRun({"run pages-only": (1, long)}))
        self.assertIn("line 199", out, "the end of the report")
        self.assertNotIn("line 100\n", out, "not the whole report")

    def test_the_same_commit_pushed_to_two_branches_runs_once(self):
        run = FakeRun()
        code, out, _ = push_of(["website/content/docs/c.md"], run, shas=("abc123", "abc123"))
        self.assertEqual(code, 0, out)
        self.assertEqual(len(run.names(sys.executable)), 1, "one run")

    def test_only_skipped_scenarios_is_a_pass_that_says_so(self):
        run = FakeRun()
        code, out, _ = push_of(["website/content/docs/e.md"], run)
        self.assertEqual(code, 0, "nothing this host runs")
        self.assertEqual(run.calls, [], "no build for nothing")
        self.assertIn("docs scenarios: skipped long", out, out)

    def test_the_command_reads_the_changed_paths_on_stdin(self):
        with unittest.mock.patch.object(dt, "push", return_value=0) as push, \
                unittest.mock.patch("sys.stdin", stdin_of("README.md\nCargo.lock\n")):
            self.assertEqual(dt.main(["push", "abc123"]), 0, "its code")
        self.assertEqual(push.call_args.args, (["abc123"], ["README.md", "Cargo.lock"]), "shas and paths")

    def test_push_with_no_commit_is_a_usage_error(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(dt.main(["push"]), 2, "it needs the pushed commits")

    # The pass stamp: the hook checks it, so the scenarios run before
    # `git push`, whose connection to GitHub would not wait for them.
    def test_a_pass_stamps_the_commit_with_the_scenarios_that_ran(self):
        stamped = []
        code, out, _ = push_of(["website/content/docs/b.md"], FakeRun(), stamped=stamped)
        self.assertEqual(code, 0, out)
        self.assertEqual(stamped, [("abc123", {"one": "", "two": ""})], "the commit and every scenario that passed")

    def test_a_failure_stamps_nothing(self):
        stamped = []
        push_of(["website/content/docs/b.md"], FakeRun({"run two": (1, "x\n")}), stamped=stamped)
        self.assertEqual(stamped, [], "one failed: no stamp")

    def test_a_failed_build_stamps_nothing(self):
        stamped = []
        push_of(["website/content/docs/c.md"], FakeRun({"cargo build": (101, "e\n")}), stamped=stamped)
        self.assertEqual(stamped, [], "nothing ran")

    def test_a_refused_checkout_stamps_nothing(self):
        stamped = []
        push_of(["website/content/docs/c.md"], FakeRun({"git rev-parse": (0, "fff999\n")}), stamped=stamped)
        self.assertEqual(stamped, [], "another commit's server")

    def test_push_since_a_revision_reads_the_changed_paths_from_git(self):
        with unittest.mock.patch.object(dt, "push", return_value=0) as push, \
                unittest.mock.patch.object(dt, "changed_since", return_value=["Cargo.lock"]) as since, \
                unittest.mock.patch.object(dt, "range_commits", return_value=["abc123"]):
            self.assertEqual(dt.main(["push", "abc123", "--since", "origin/main"]), 0, "its code")
        since.assert_called_once_with("origin/main", "abc123")
        self.assertEqual(push.call_args.args, (["abc123"], ["Cargo.lock"]), "the paths git names")

    def test_the_stamp_file_holds_one_scenario_a_line(self):
        d = tempfile.mkdtemp(prefix="docs-stamp-test-")
        try:
            dt.write_stamp("abc123", ["one", "two"], git_dir=d)
            self.assertEqual(dt.read_stamp("abc123", git_dir=d), {"one": "", "two": ""}, "read back")
            self.assertIsNone(dt.read_stamp("def456", git_dir=d), "another commit has none")
            self.assertTrue(os.path.exists(os.path.join(d, "docs-scenarios-abc123.pass")), "named by the commit")
        finally:
            shutil.rmtree(d)


class FakeGit:
    """Stands in for docs_test.Git over a line of commits on top of the
    remote's main, "m0": `chain` lists (sha, changed paths), oldest first.
    A ref's range is the commits after its remote sha (all of them for a
    new ref or one whose remote sha is not in the line) up to its local sha.
    `outside` names shas that are not ancestors of anything."""

    def __init__(self, chain, outside=(), fail=None):
        self.chain = list(chain)
        self.order = ["m0"] + [sha for sha, _ in self.chain]
        self.paths = dict(self.chain)
        self.outside = set(outside)
        self.fail = fail
        self.diffs = []

    def _after(self, base, tip):
        if self.fail:
            raise dt.PageError(self.fail)
        if tip not in self.order:
            raise dt.PageError(f"unknown commit {tip}")
        end = self.order.index(tip)
        start = self.order.index(base) + 1 if base in self.order[: end + 1] else 1
        return self.order[start : end + 1]

    def commits(self, ref, remote):
        return list(reversed(self._after(ref.remote_sha, ref.local_sha)))

    def changed(self, ref, remote):
        return [p for sha in self._after(ref.remote_sha, ref.local_sha) for p in self.paths[sha]]

    def diff(self, a, b):
        self.diffs.append((a, b))
        return [p for sha in self._after(a, b) for p in self.paths[sha]]

    def show(self, sha, path):
        return None

    def includes(self, sha):
        return None

    def is_ancestor(self, a, b):
        if self.fail:
            raise dt.PageError(self.fail)
        if a in self.outside:
            return False
        return a in self.order and b in self.order and self.order.index(a) <= self.order.index(b)


def ref_of(local, remote="m0", name="refs/heads/topic", remote_ref=None):
    return dt.Ref(name, local, remote_ref or name, remote)


def no_ci(tip, names, *, pull_requests_ok):
    return {}, {n: f"CI has run no how-to job for it on {tip}" for n in names}


def ci_must_not_be_asked(tip, names, *, pull_requests_ok):
    raise AssertionError(f"CI was asked about {names} on {tip}")


def range_check(git, refs, stamps, env=None, ci=no_ci, scenarios=FAKES):
    out = io.StringIO()
    code = dt.check(
        list(refs),
        remote="origin",
        env=env or {},
        scenarios=scenarios,
        steps=fake_steps,
        read=lambda sha: stamps.get(sha),
        git=git,
        ci=ci,
        out=out,
    )
    return code, out.getvalue()


def check_of(changed, stamps, shas=("abc123",), env=None, scenarios=FAKES, ci=no_ci):
    """One commit per pushed sha, each changing `changed`, each pushed to a
    branch of its own."""
    git = FakeGit([(sha, list(changed)) for sha in dict.fromkeys(shas)])
    refs = [ref_of(sha, name=f"refs/heads/b{i}") for i, sha in enumerate(shas)]
    return range_check(git, refs, stamps, env=env, ci=ci, scenarios=scenarios)


class Check(unittest.TestCase):
    # Success.
    def test_nothing_needed_passes(self):
        code, out = check_of(["README.md"], {})
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenarios: not needed", out, out)

    def test_a_stamp_with_every_needed_scenario_passes(self):
        code, out = check_of(["website/content/docs/b.md"], {"abc123": {"one", "two"}})
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenario one: passed on abc123 before this push", out, out)
        self.assertIn("docs scenario two: passed on abc123 before this push", out, out)

    def test_a_stamp_with_more_scenarios_passes(self):
        code, out = check_of(["website/content/docs/c.md"], {"abc123": {"one", "two", "pages-only"}})
        self.assertEqual(code, 0, "a wider run covers this push")

    def test_the_same_commit_on_two_branches_passes(self):
        code, out = check_of(["website/content/docs/c.md"], {"abc123": {"pages-only"}}, shas=("abc123", "abc123"))
        self.assertEqual(code, 0, out)

    def test_only_skipped_scenarios_pass_and_are_named(self):
        code, out = check_of(["website/content/docs/e.md"], {})
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenarios: skipped long", out, out)

    def test_a_token_counts_the_gh_scenario_as_needed(self):
        code, out = check_of(["website/content/docs/d.md"], {"abc123": {"tokened"}}, env={"GH_TOKEN": "x"})
        self.assertEqual(code, 0, out)

    # Failure.
    def test_no_stamp_refuses_and_names_the_command(self):
        code, out = check_of(["website/content/docs/b.md"], {})
        self.assertEqual(code, 1, "refused")
        self.assertIn("one, two have not passed on abc123", out, out)
        self.assertIn("python3 -I scripts/docs_test.py push abc123 --since origin/main", out, "what to run")

    def test_a_stamp_missing_one_scenario_refuses_and_names_it(self):
        code, out = check_of(["website/content/docs/b.md"], {"abc123": {"one"}})
        self.assertEqual(code, 1, "two is missing")
        self.assertIn("two have not passed on abc123", out, out)
        self.assertNotIn("one, two", out, "only the missing one")

    def test_a_stamp_of_another_commit_refuses(self):
        code, out = check_of(["website/content/docs/c.md"], {"def456": {"pages-only"}})
        self.assertEqual(code, 1, "the stamp is for another commit")

    def test_a_second_branch_without_its_stamp_refuses(self):
        code, out = check_of(["website/content/docs/c.md"], {"abc123": {"pages-only"}}, shas=("abc123", "def456"))
        self.assertEqual(code, 1, out)
        self.assertIn("pages-only have not passed on def456", out, "the branch whose tip has no stamp")

    def test_an_empty_stamp_refuses(self):
        code, out = check_of(["website/content/docs/c.md"], {"abc123": set()})
        self.assertEqual(code, 1, "it names no scenario")

    def test_a_server_change_needs_every_scenario_in_the_stamp(self):
        code, out = check_of(["Cargo.lock"], {"abc123": {"one"}})
        self.assertEqual(code, 1, "two gets the server too")

    # Edge cases.
    def test_check_runs_nothing(self):
        with unittest.mock.patch.object(dt.subprocess, "run", side_effect=AssertionError("ran")):
            code, _ = check_of(["website/content/docs/b.md"], {"abc123": {"one", "two"}})
        self.assertEqual(code, 0, "a stamp is read, nothing is built or run")

    def test_a_skipped_scenario_is_not_asked_of_the_stamp(self):
        code, out = check_of(["Cargo.lock"], {"abc123": {"one", "two"}})
        self.assertEqual(code, 0, "tokened is skipped without a token, so not needed")

    def test_the_command_reads_the_pushed_refs_on_stdin(self):
        lines = "refs/heads/a abc123 refs/heads/a 000\nrefs/heads/b def456 refs/heads/main m0\n"
        with unittest.mock.patch.object(dt, "check", return_value=1) as check, \
                unittest.mock.patch("sys.stdin", stdin_of(lines)):
            self.assertEqual(dt.main(["check", "--remote", "upstream"]), 1, "its code")
        self.assertEqual(
            check.call_args.args,
            ([dt.Ref("refs/heads/a", "abc123", "refs/heads/a", "000"), dt.Ref("refs/heads/b", "def456", "refs/heads/main", "m0")],),
            "the refs, in the order the hook read them",
        )
        self.assertEqual(check.call_args.kwargs["remote"], "upstream", "the remote the hook pushes to")

    def test_check_with_no_ref_is_a_usage_error(self):
        with contextlib.redirect_stderr(io.StringIO()), unittest.mock.patch("sys.stdin", stdin_of("")):
            self.assertEqual(dt.main(["check", "--remote", "origin"]), 2, "it needs the pushed refs")

    def test_a_stamp_line_with_spaces_is_read(self):
        d = tempfile.mkdtemp(prefix="docs-stamp-test-")
        try:
            path = os.path.join(d, "docs-scenarios-abc123.pass")
            with open(path, "w", encoding="utf-8") as f:
                f.write(" one \n\ntwo\n")
            self.assertEqual(dt.read_stamp("abc123", git_dir=d), {"one": "", "two": ""}, "trimmed, blank lines dropped")
        finally:
            shutil.rmtree(d)


SRC = "crates/vcon-server/src/serve.rs"
PAGE_B = "website/content/docs/b.md"
PAGE_C = "website/content/docs/c.md"


def carried_of(git, tip, stamps, names=("one", "two", "pages-only")):
    commits = list(reversed(git._after("m0", tip)))
    return dt.carried(
        list(names), tip, commits, lambda sha: stamps.get(sha), lambda sha: git.diff(sha, tip), FAKES, fake_steps
    )


class Carried(unittest.TestCase):
    """A scenario that passed on a commit of the pushed range still holds
    for the tip when the commits after it change nothing that scenario
    runs: the tip's tree is the tree it was tested on, as far as it reads."""

    # Success.
    def test_the_tips_own_stamp_holds(self):
        git = FakeGit([("c1", [PAGE_B])])
        self.assertEqual(carried_of(git, "c1", {"c1": {"one", "two"}}), {"one": "c1", "two": "c1"}, "the tip")

    def test_an_earlier_stamp_holds_when_nothing_after_it_is_read(self):
        git = FakeGit([("c1", [SRC]), ("c2", ["README.md"])])
        self.assertEqual(carried_of(git, "c2", {"c1": {"one", "two"}}), {"one": "c1", "two": "c1"}, "README")

    def test_two_stamps_together_hold_the_range(self):
        git = FakeGit([("c1", [SRC]), ("c2", [PAGE_C])])
        got = carried_of(git, "c2", {"c1": {"one", "two"}, "c2": {"pages-only"}})
        self.assertEqual(got, {"one": "c1", "two": "c1", "pages-only": "c2"}, "each from its own commit")

    def test_the_newest_stamp_is_named(self):
        git = FakeGit([("c1", [PAGE_B]), ("c2", ["README.md"])])
        self.assertEqual(carried_of(git, "c2", {"c1": {"one"}, "c2": {"one"}}), {"one": "c2"}, "the tip first")

    def test_a_change_and_its_undoing_leave_the_stamp_holding(self):
        git = FakeGit([("c1", [SRC]), ("c2", [])])
        self.assertEqual(carried_of(git, "c2", {"c1": {"one", "two"}}), {"one": "c1", "two": "c1"}, "same tree")

    # Failure.
    def test_a_later_change_to_its_page_voids_it(self):
        git = FakeGit([("c1", [SRC]), ("c2", [PAGE_B])])
        self.assertEqual(carried_of(git, "c2", {"c1": {"one", "two"}}), {}, "b.md is run by both")

    def test_a_later_server_change_voids_every_server_scenario(self):
        git = FakeGit([("c1", [PAGE_B]), ("c2", [SRC])])
        self.assertEqual(carried_of(git, "c2", {"c1": {"one", "two"}}), {}, "the server they run changed")

    def test_a_later_runner_change_voids_every_scenario(self):
        git = FakeGit([("c1", [PAGE_C]), ("c2", ["scripts/docs_test.py"])])
        self.assertEqual(carried_of(git, "c2", {"c1": {"pages-only"}}), {}, "the runner runs every scenario")

    def test_a_stamp_outside_the_range_is_not_read(self):
        git = FakeGit([("c1", [PAGE_C])])
        self.assertEqual(carried_of(git, "c1", {"m0": {"pages-only"}}), {}, "m0 is not pushed")

    def test_no_stamp_holds_nothing(self):
        git = FakeGit([("c1", [SRC]), ("c2", [])])
        self.assertEqual(carried_of(git, "c2", {}), {}, "nothing ran")

    # Edge cases.
    def test_a_later_page_change_voids_only_the_scenario_that_runs_it(self):
        git = FakeGit([("c1", [SRC, PAGE_C]), ("c2", [PAGE_C])])
        got = carried_of(git, "c2", {"c1": {"one", "two", "pages-only"}})
        self.assertEqual(got, {"one": "c1", "two": "c1"}, "c.md is pages-only's alone")

    def test_a_stamped_scenario_the_push_does_not_need_is_left_out(self):
        git = FakeGit([("c1", [PAGE_C])])
        got = carried_of(git, "c1", {"c1": {"pages-only", "one"}}, names=("pages-only",))
        self.assertEqual(got, {"pages-only": "c1"}, "only what was asked")

    def test_an_unknown_name_in_a_stamp_is_ignored(self):
        git = FakeGit([("c1", [PAGE_C])])
        self.assertEqual(carried_of(git, "c1", {"c1": {"retired", "pages-only"}}), {"pages-only": "c1"}, "ignored")

    def test_the_tip_is_not_compared_with_itself(self):
        git = FakeGit([("c1", [PAGE_B])])
        carried_of(git, "c1", {"c1": {"one"}})
        self.assertEqual(git.diffs, [], "no diff of the tip with the tip")

    def test_a_commit_with_no_stamp_costs_no_diff(self):
        git = FakeGit([("c1", [SRC]), ("c2", [PAGE_C]), ("c3", [])])
        carried_of(git, "c3", {"c1": {"one", "two"}})
        self.assertEqual(git.diffs, [("c1", "c3")], "only a stamped commit is compared")


class FakeApi:
    """Stands in for gh_api: answers GitHub's runs and jobs listings from
    `runs` (each with its jobs) and records each path. The runs listing
    answers every run, whatever head_sha the path asks for, so a run of
    another commit in the answer is tested too."""

    def __init__(self, runs, fail=None, split=False):
        self.runs = runs
        self.fail = fail
        self.split = split
        self.paths = []

    def __call__(self, path):
        self.paths.append(path)
        if self.fail:
            raise dt.PageError(self.fail)
        if re.fullmatch(r"repos/NormB/vcon-server/actions/runs\?head_sha=\w+&per_page=100", path):
            runs = [{k: v for k, v in r.items() if k != "jobs"} for r in self.runs]
            return [{"total_count": len(runs), "workflow_runs": runs}]
        m = re.fullmatch(r"repos/NormB/vcon-server/actions/runs/(\d+)/jobs\?per_page=100", path)
        if m:
            jobs = next(r["jobs"] for r in self.runs if r["id"] == int(m.group(1)))
            if self.split:
                return [{"total_count": len(jobs), "jobs": jobs[:1]}, {"total_count": len(jobs), "jobs": jobs[1:]}]
            return [{"total_count": len(jobs), "jobs": jobs}]
        raise AssertionError(f"unexpected path {path}")


def ci_run(run_id, jobs, event="pull_request", sha="tip", path=".github/workflows/ci.yml", created="2026-11-16T01:00:00Z"):
    return {"id": run_id, "event": event, "head_sha": sha, "path": path, "created_at": created, "jobs": jobs}


def ci_job(name, conclusion="success", sha="tip", status="completed"):
    return {"name": f"how-to {name} against a live server", "conclusion": conclusion, "status": status, "head_sha": sha}


def ci_of(api, names=("one", "two"), tip="tip", pull_requests_ok=True):
    return dt.ci_results(tip, list(names), pull_requests_ok=pull_requests_ok, api=api)


def ci_from(api):
    return lambda tip, names, *, pull_requests_ok: dt.ci_results(
        tip, names, pull_requests_ok=pull_requests_ok, api=api
    )


class CiResults(unittest.TestCase):
    """CI's how-to jobs on the pushed commit itself stand for a local run."""

    # Success.
    def test_green_jobs_on_the_commit_pass(self):
        passed, problems = ci_of(FakeApi([ci_run(7, [ci_job("one"), ci_job("two")])]))
        self.assertEqual(sorted(passed), ["one", "two"], problems)
        self.assertEqual(problems, {}, "nothing missing")

    def test_the_evidence_names_the_job_and_the_run(self):
        passed, _ = ci_of(FakeApi([ci_run(7, [ci_job("one")])]), names=("one",))
        self.assertEqual(passed["one"], 'CI\'s job "how-to one against a live server" passed on tip (run 7)', passed)

    def test_a_push_run_counts_without_the_fast_forward(self):
        passed, problems = ci_of(FakeApi([ci_run(7, [ci_job("one")], event="push")]), names=("one",), pull_requests_ok=False)
        self.assertEqual(list(passed), ["one"], problems)

    def test_a_dispatched_run_counts_without_the_fast_forward(self):
        api = FakeApi([ci_run(7, [ci_job("one")], event="workflow_dispatch")])
        passed, problems = ci_of(api, names=("one",), pull_requests_ok=False)
        self.assertEqual(list(passed), ["one"], problems)

    def test_jobs_on_two_pages_are_read(self):
        passed, problems = ci_of(FakeApi([ci_run(7, [ci_job("one"), ci_job("two")])], split=True))
        self.assertEqual(sorted(passed), ["one", "two"], problems)

    def test_the_newest_run_decides_when_it_is_green(self):
        api = FakeApi([
            ci_run(5, [ci_job("one", "failure")], created="2026-11-16T01:00:00Z"),
            ci_run(9, [ci_job("one")], created="2026-11-16T02:00:00Z"),
        ])
        passed, problems = ci_of(api, names=("one",))
        self.assertIn("(run 9)", passed.get("one", ""), problems)

    # Failure.
    def test_a_failed_job_is_named_with_its_conclusion(self):
        passed, problems = ci_of(FakeApi([ci_run(7, [ci_job("one", "failure"), ci_job("two")])]))
        self.assertEqual(list(passed), ["two"], "two still passed")
        self.assertEqual(problems["one"], "CI's newest job for it on tip (run 7) is failure", problems)

    def test_a_job_of_another_commit_is_refused(self):
        passed, problems = ci_of(FakeApi([ci_run(7, [ci_job("one", sha="other")])]), names=("one",))
        self.assertEqual(passed, {}, "another commit's job proves nothing about this one")
        self.assertEqual(problems["one"], "CI's job for it in run 7 ran on other, not tip", problems)

    def test_a_run_of_another_commit_is_not_read(self):
        api = FakeApi([ci_run(7, [ci_job("one")], sha="other")])
        passed, problems = ci_of(api, names=("one",))
        self.assertEqual(passed, {}, "the run is for other")
        self.assertEqual(problems["one"], "CI has run no how-to job for it on tip", problems)
        self.assertFalse(any("/jobs" in p for p in api.paths), "its jobs are not even read")

    def test_no_run_at_all_is_refused(self):
        passed, problems = ci_of(FakeApi([]))
        self.assertEqual(passed, {}, "unknown is not green")
        self.assertEqual(problems["two"], "CI has run no how-to job for it on tip", problems)

    def test_an_unreadable_answer_refuses_every_name(self):
        passed, problems = ci_of(FakeApi([], fail="gh: HTTP 502"))
        self.assertEqual(passed, {}, "nothing known")
        self.assertEqual(problems["one"], "CI's results for tip cannot be read: gh: HTTP 502", problems)

    def test_a_running_job_is_not_green(self):
        api = FakeApi([ci_run(7, [ci_job("one", None, status="in_progress")])])
        _, problems = ci_of(api, names=("one",))
        self.assertEqual(problems["one"], "CI's newest job for it on tip (run 7) is in_progress", problems)

    def test_a_skipped_job_is_not_green(self):
        _, problems = ci_of(FakeApi([ci_run(7, [ci_job("one", "skipped")])]), names=("one",))
        self.assertEqual(problems["one"], "CI's newest job for it on tip (run 7) is skipped", problems)

    def test_a_newer_failure_beats_an_older_pass(self):
        api = FakeApi([
            ci_run(9, [ci_job("one", "failure")], created="2026-11-16T02:00:00Z"),
            ci_run(5, [ci_job("one")], created="2026-11-16T01:00:00Z"),
        ])
        passed, problems = ci_of(api, names=("one",))
        self.assertEqual(passed, {}, "the newest result stands")
        self.assertIn("(run 9) is failure", problems["one"], problems)

    def test_a_pull_request_run_does_not_count_without_the_fast_forward(self):
        passed, problems = ci_of(FakeApi([ci_run(7, [ci_job("one")])]), names=("one",), pull_requests_ok=False)
        self.assertEqual(passed, {}, "a pull request run tests the merge with main")
        self.assertIn("only for a pull request", problems["one"], problems)

    def test_another_workflow_with_the_same_job_name_does_not_count(self):
        api = FakeApi([ci_run(7, [ci_job("one")], path=".github/workflows/sanitizers.yml")])
        passed, problems = ci_of(api, names=("one",))
        self.assertEqual(passed, {}, "only CI's how-to jobs")
        self.assertEqual(problems["one"], "CI has run no how-to job for it on tip", problems)

    # Edge cases.
    def test_a_run_without_the_job_falls_through_to_an_older_run(self):
        api = FakeApi([
            ci_run(9, [ci_job("two")], created="2026-11-16T02:00:00Z"),
            ci_run(5, [ci_job("one")], created="2026-11-16T01:00:00Z"),
        ])
        passed, problems = ci_of(api)
        self.assertEqual((passed["one"][-7:], passed["two"][-7:]), ("(run 5)", "(run 9)"), problems)

    def test_runs_stop_being_read_once_each_name_is_decided(self):
        api = FakeApi([
            ci_run(9, [ci_job("one")], created="2026-11-16T02:00:00Z"),
            ci_run(5, [ci_job("one")], created="2026-11-16T01:00:00Z"),
        ])
        ci_of(api, names=("one",))
        self.assertEqual([p for p in api.paths if "/jobs" in p], ["repos/NormB/vcon-server/actions/runs/9/jobs?per_page=100"], api.paths)

    def test_the_runs_are_asked_for_by_the_commit(self):
        api = FakeApi([])
        ci_of(api, tip="abc123")
        self.assertEqual(api.paths, ["repos/NormB/vcon-server/actions/runs?head_sha=abc123&per_page=100"], api.paths)

    def test_no_name_asks_nothing(self):
        api = FakeApi([])
        self.assertEqual(ci_of(api, names=()), ({}, {}), "nothing to ask")
        self.assertEqual(api.paths, [], "no request")

    def test_the_job_name_is_cis(self):
        with open(os.path.join(dt.ROOT, ".github", "workflows", "ci.yml"), encoding="utf-8") as f:
            workflow = f.read()
        self.assertIn("name: how-to ${{ matrix.scenario }} against a live server", workflow, "ci.yml's job name")
        self.assertEqual(dt.ci_job_name("s3"), "how-to s3 against a live server", "the same name")
        self.assertEqual(dt.CI_WORKFLOW, ".github/workflows/ci.yml", "the workflow read")

    def test_a_job_with_no_conclusion_and_no_status_is_named_unknown(self):
        api = FakeApi([ci_run(7, [{"name": "how-to one against a live server", "head_sha": "tip"}])])
        _, problems = ci_of(api, names=("one",))
        self.assertEqual(problems["one"], "CI's newest job for it on tip (run 7) is unknown", problems)

    def test_gh_reads_every_page(self):
        run = FakeRun({"gh api": (0, '[{"total_count": 0, "workflow_runs": []}]')})
        self.assertEqual(dt.gh_api("repos/x", run=run), [{"total_count": 0, "workflow_runs": []}], "the pages")
        self.assertEqual(run.calls, [["gh", "api", "--paginate", "--slurp", "repos/x"]], "every page, as one list")

    def test_a_failing_gh_is_an_error_with_its_message(self):
        run = FakeRun({"gh api": (1, "")})
        with self.assertRaises(dt.PageError):
            dt.gh_api("repos/x", run=run)

    def test_a_missing_gh_is_an_error(self):
        def run(cmd, **kw):
            raise FileNotFoundError("gh")

        with self.assertRaisesRegex(dt.PageError, "gh"):
            dt.gh_api("repos/x", run=run)

    def test_an_answer_that_is_not_json_is_an_error(self):
        with self.assertRaises(dt.PageError):
            dt.gh_api("repos/x", run=FakeRun({"gh api": (0, "<html>")}))

    def test_a_spent_budget_is_waited_for_and_the_listing_read_again(self):
        calls = []

        def run(cmd, **kw):
            calls.append(cmd)
            if cmd == ["gh", "api", "rate_limit"]:
                return subprocess.CompletedProcess(cmd, 0, '{"resources":{"core":{"limit":5000,"remaining":0,"reset":1000010}}}', "")
            if len(calls) == 1:
                return subprocess.CompletedProcess(cmd, 1, "", "gh: API rate limit exceeded for user ID 1.")
            return subprocess.CompletedProcess(cmd, 0, "[[]]", "")

        slept = []
        self.assertEqual(dt.gh_api("repos/x", run=run, clock=lambda: 1_000_000, sleep=slept.append, err=io.StringIO()), [[]], "read after the reset")
        self.assertEqual(slept, [15], "to the reset plus 5 s")


class CheckRange(unittest.TestCase):
    """What pre-push asks, for each pushed ref: the scenarios its range
    needs have passed on the tip, on a commit of the range whose later
    commits change nothing they run, or in CI on the tip itself."""

    # Success.
    def test_two_commits_that_both_need_scenarios_pass_on_one_stamp_of_the_tip(self):
        git = FakeGit([("c1", [SRC]), ("c2", [SRC, PAGE_C])])
        code, out = range_check(git, [ref_of("c2")], {"c2": {"one", "two", "pages-only"}})
        self.assertEqual(code, 0, out)
        self.assertNotIn("one branch at a time", out, "not refused for its commit count")

    def test_each_commit_stamped_for_its_own_change_passes(self):
        git = FakeGit([("c1", [SRC]), ("c2", [PAGE_C])])
        code, out = range_check(git, [ref_of("c2")], {"c1": {"one", "two"}, "c2": {"pages-only"}})
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenario one: passed on c1, and the commits after it change nothing it runs", out, out)
        self.assertIn("docs scenario pages-only: passed on c2 before this push", out, out)

    def test_two_branches_each_with_its_stamp_pass(self):
        git = FakeGit([("a1", [PAGE_C]), ("b1", [PAGE_C])])
        refs = [ref_of("a1", name="refs/heads/a"), ref_of("b1", remote="a1", name="refs/heads/b")]
        code, out = range_check(git, refs, {"a1": {"pages-only"}, "b1": {"pages-only"}})
        self.assertEqual(code, 0, out)

    def test_a_landing_push_passes_on_cis_green_jobs_for_its_head(self):
        git = FakeGit([("c1", [SRC])])
        api = FakeApi([ci_run(7, [ci_job("one", sha="c1"), ci_job("two", sha="c1")], sha="c1")])
        ref = ref_of("c1", remote="m0", name="refs/heads/topic", remote_ref="refs/heads/main")
        code, out = range_check(git, [ref], {}, ci=ci_from(api))
        self.assertEqual(code, 0, out)
        self.assertIn('docs scenario one: CI\'s job "how-to one against a live server" passed on c1 (run 7)', out, out)

    def test_a_push_run_on_the_tip_passes_a_topic_branch(self):
        git = FakeGit([("c1", [PAGE_C])])
        api = FakeApi([ci_run(7, [ci_job("pages-only", sha="c1")], sha="c1", event="push")])
        code, out = range_check(git, [ref_of("c1")], {}, ci=ci_from(api))
        self.assertEqual(code, 0, out)

    def test_stamps_and_ci_together_pass(self):
        git = FakeGit([("c1", [SRC])])
        api = FakeApi([ci_run(7, [ci_job("two", sha="c1")], sha="c1")])
        ref = ref_of("c1", remote_ref="refs/heads/main")
        code, out = range_check(git, [ref], {"c1": {"one"}}, ci=ci_from(api))
        self.assertEqual(code, 0, out)
        self.assertEqual([p for p in api.paths if "/jobs" in p], ["repos/NormB/vcon-server/actions/runs/7/jobs?per_page=100"], "CI asked once")

    def test_ci_is_not_asked_when_the_stamps_hold(self):
        git = FakeGit([("c1", [SRC])])
        code, out = range_check(git, [ref_of("c1")], {"c1": {"one", "two"}}, ci=ci_must_not_be_asked)
        self.assertEqual(code, 0, out)

    def test_a_ref_with_no_new_commit_is_not_checked(self):
        git = FakeGit([("c1", [SRC])])
        code, out = range_check(git, [ref_of("c1", remote="c1")], {}, ci=ci_must_not_be_asked)
        self.assertEqual(code, 0, out)
        self.assertEqual(out, "", "nothing to say about a ref that sends nothing")

    def test_ci_is_told_a_fast_forward_of_main(self):
        seen = []
        git = FakeGit([("c1", [PAGE_C])])
        ci = lambda tip, names, *, pull_requests_ok: (seen.append(pull_requests_ok), no_ci(tip, names, pull_requests_ok=pull_requests_ok))[1]
        range_check(git, [ref_of("c1", remote_ref="refs/heads/main")], {}, ci=ci)
        range_check(git, [ref_of("c1")], {}, ci=ci)
        self.assertEqual(seen, [True, False], "main, then a topic branch")

    # Failure.
    def test_a_stamp_voided_by_a_later_commit_refuses_and_names_it(self):
        git = FakeGit([("c1", [SRC]), ("c2", [PAGE_C, PAGE_B])])
        code, out = range_check(git, [ref_of("c2")], {"c1": {"one", "two", "pages-only"}, "c2": {"pages-only"}})
        self.assertEqual(code, 1, out)
        self.assertIn("docs scenarios: one, two have not passed on c2", out, out)
        self.assertIn("python3 -I scripts/docs_test.py push c2 --since origin/main", out, "what to run")

    def test_cis_failed_job_refuses_with_its_reason(self):
        git = FakeGit([("c1", [PAGE_C])])
        api = FakeApi([ci_run(7, [ci_job("pages-only", "failure", sha="c1")], sha="c1", event="push")])
        code, out = range_check(git, [ref_of("c1")], {}, ci=ci_from(api))
        self.assertEqual(code, 1, out)
        self.assertIn("docs scenario pages-only: CI's newest job for it on c1 (run 7) is failure", out, out)

    def test_cis_green_job_on_another_commit_refuses(self):
        git = FakeGit([("c1", [PAGE_C]), ("c2", [PAGE_C])])
        api = FakeApi([ci_run(7, [ci_job("pages-only", sha="c1")], sha="c1", event="push")])
        code, out = range_check(git, [ref_of("c2", remote="c1")], {}, ci=ci_from(api))
        self.assertEqual(code, 1, "c1's job is not c2's")
        self.assertIn("pages-only have not passed on c2", out, out)

    def test_a_pull_request_run_does_not_pass_a_topic_branch(self):
        git = FakeGit([("c1", [PAGE_C])])
        api = FakeApi([ci_run(7, [ci_job("pages-only", sha="c1")], sha="c1")])
        code, out = range_check(git, [ref_of("c1")], {}, ci=ci_from(api))
        self.assertEqual(code, 1, out)
        self.assertIn("only for a pull request", out, out)

    def test_a_pull_request_run_does_not_pass_a_push_to_main_that_is_not_a_fast_forward(self):
        git = FakeGit([("c1", [PAGE_C])], outside={"x9"})
        api = FakeApi([ci_run(7, [ci_job("pages-only", sha="c1")], sha="c1")])
        code, out = range_check(git, [ref_of("c1", remote="x9", remote_ref="refs/heads/main")], {}, ci=ci_from(api))
        self.assertEqual(code, 1, "main moved past the run's base")

    def test_unreadable_ci_refuses(self):
        git = FakeGit([("c1", [PAGE_C])])
        code, out = range_check(git, [ref_of("c1")], {}, ci=ci_from(FakeApi([], fail="gh: no network")))
        self.assertEqual(code, 1, out)
        self.assertIn("cannot be read: gh: no network", out, out)

    def test_a_git_failure_refuses(self):
        git = FakeGit([("c1", [PAGE_C])], fail="fatal: bad revision")
        code, out = range_check(git, [ref_of("c1")], {})
        self.assertEqual(code, 1, out)
        self.assertIn("docs scenarios: refused: cannot read the commits pushed to refs/heads/topic: fatal: bad revision", out, out)

    def test_one_refused_branch_refuses_the_push(self):
        git = FakeGit([("a1", [PAGE_C]), ("b1", [PAGE_C])])
        refs = [ref_of("a1", name="refs/heads/a"), ref_of("b1", remote="a1", name="refs/heads/b")]
        code, out = range_check(git, refs, {"a1": {"pages-only"}})
        self.assertEqual(code, 1, out)
        self.assertIn("pages-only have not passed on b1", out, out)

    def test_a_ci_failure_on_one_scenario_refuses_though_the_rest_passed(self):
        git = FakeGit([("c1", [SRC])])
        api = FakeApi([ci_run(7, [ci_job("one", sha="c1"), ci_job("two", "cancelled", sha="c1")], sha="c1", event="push")])
        code, out = range_check(git, [ref_of("c1")], {}, ci=ci_from(api))
        self.assertEqual(code, 1, out)
        self.assertIn("docs scenarios: two have not passed on c1", out, out)

    # Edge cases.
    def test_a_page_changed_and_changed_back_still_needs_its_scenario_on_a_stamp(self):
        git = FakeGit([("c1", [PAGE_C]), ("c2", [])])
        code, out = range_check(git, [ref_of("c2")], {"c1": {"pages-only"}})
        self.assertEqual(code, 0, "c1 ran it and nothing after it changed c.md")

    def test_the_same_tip_on_two_branches_reads_one_stamp(self):
        git = FakeGit([("c1", [PAGE_C])])
        refs = [ref_of("c1", name="refs/heads/a"), ref_of("c1", name="refs/heads/b")]
        code, out = range_check(git, refs, {"c1": {"pages-only"}})
        self.assertEqual(code, 0, out)

    def test_only_skipped_scenarios_need_no_ci(self):
        git = FakeGit([("c1", ["website/content/docs/e.md"])])
        code, out = range_check(git, [ref_of("c1")], {}, ci=ci_must_not_be_asked)
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenarios: skipped long", out, out)

    def test_a_ref_line_with_too_few_fields_is_a_usage_error(self):
        with contextlib.redirect_stderr(io.StringIO()) as err, unittest.mock.patch("sys.stdin", stdin_of("refs/heads/a abc123\n")):
            self.assertEqual(dt.main(["check", "--remote", "origin"]), 2, "the hook's lines have four fields")
        self.assertIn("refs/heads/a abc123", err.getvalue(), "the line is named")

    def test_an_unknown_check_argument_is_a_usage_error(self):
        with contextlib.redirect_stderr(io.StringIO()), unittest.mock.patch("sys.stdin", stdin_of("a b c d\n")):
            self.assertEqual(dt.main(["check", "abc123"]), 2, "the commits come on stdin now")

    def test_blank_ref_lines_are_ignored(self):
        self.assertEqual(dt.parse_refs(["", "a b c d", "  "]), [dt.Ref("a", "b", "c", "d")], "one ref")

    def test_the_default_remote_is_origin(self):
        with unittest.mock.patch.object(dt, "check", return_value=0) as check, \
                unittest.mock.patch("sys.stdin", stdin_of("a b c d\n")):
            dt.main(["check"])
        self.assertEqual(check.call_args.kwargs["remote"], "origin", "as the hook pushes")

    # Standard input is read as bytes and decoded as git's paths are (UTF-8,
    # each byte that is not kept as a surrogate escape), so a path or a ref
    # that is not UTF-8 is no crash.
    def test_a_ref_that_is_not_utf8_is_read(self):
        with unittest.mock.patch.object(dt, "check", return_value=0) as check, \
                unittest.mock.patch("sys.stdin", stdin_of(b"refs/heads/caf\xe9 abc refs/heads/caf\xe9 000\n")):
            self.assertEqual(dt.main(["check"]), 0, "its code")
        self.assertEqual(check.call_args.args[0][0].local_ref, "refs/heads/caf\udce9", "the byte, kept")

    def test_a_short_ref_line_that_is_not_utf8_is_a_usage_error_that_names_it(self):
        with contextlib.redirect_stderr(io.StringIO()) as err, unittest.mock.patch("sys.stdin", stdin_of(b"refs/heads/\xff abc\n")):
            self.assertEqual(dt.main(["check"]), 2, "a usage error, not a crash")
        self.assertIn("refs/heads/\udcff abc", err.getvalue(), "the line is named")

    def test_a_changed_path_that_is_not_utf8_is_read(self):
        with unittest.mock.patch.object(dt, "push", return_value=0) as push, \
                unittest.mock.patch.object(dt, "test_only_at", return_value=None), \
                unittest.mock.patch("sys.stdin", stdin_of(b"docs/caf\xe9.md\nCargo.lock\n")):
            self.assertEqual(dt.main(["push", "abc123"]), 0, "its code")
        self.assertEqual(push.call_args.args[1], ["docs/caf\udce9.md", "Cargo.lock"], "both paths, in order")

    def test_a_changed_path_in_utf8_is_read_as_it_is(self):
        with unittest.mock.patch.object(dt, "push", return_value=0) as push, \
                unittest.mock.patch.object(dt, "test_only_at", return_value=None), \
                unittest.mock.patch("sys.stdin", stdin_of("docs/café.md\n")):
            dt.main(["push", "abc123"])
        self.assertEqual(push.call_args.args[1], ["docs/café.md"], "no escape for UTF-8")

    def test_the_lines_split_where_they_did(self):
        with unittest.mock.patch("sys.stdin", stdin_of(b"a\r\nb\n\nc")):
            self.assertEqual(dt.stdin_lines(), ["a", "b", "", "c"], "as str.splitlines")

    def test_the_command_does_not_crash_on_a_ref_that_is_not_utf8(self):
        done = subprocess.run(
            [sys.executable, "-I", os.path.join(os.path.dirname(os.path.abspath(__file__)), "docs_test.py"), "check"],
            input=b"refs/heads/\xff abc\n", capture_output=True, timeout=60, check=False,
        )
        self.assertNotIn(b"Traceback", done.stderr, done.stderr)
        self.assertEqual(done.returncode, 2, done.stderr)
        self.assertIn(b"refs/heads/\\udcff abc", done.stderr, "the byte is shown, escaped")

    def test_the_command_names_a_ref_that_is_not_utf8_on_stdout(self):
        # No such commit: check says it cannot read the commits pushed to
        # the ref, on stdout, with the ref's name.
        ref = b"refs/heads/a 1111111111111111111111111111111111111111 refs/heads/\xff 0000000000000000000000000000000000000000\n"
        done = subprocess.run(
            [sys.executable, "-I", os.path.join(os.path.dirname(os.path.abspath(__file__)), "docs_test.py"), "check"],
            input=ref, capture_output=True, timeout=60, check=False,
        )
        self.assertNotIn(b"Traceback", done.stderr, done.stderr)
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn(b"cannot read the commits pushed to refs/heads/\\udcff", done.stdout, done.stdout)

    def test_a_new_branch_counts_from_what_the_remote_lacks(self):
        git = FakeGit([("c1", [SRC]), ("c2", [PAGE_C])])
        code, out = range_check(git, [ref_of("c2", remote=dt.ZERO_SHA)], {"c2": {"pages-only"}})
        self.assertEqual(code, 1, "c1's server change is in the range of a new branch too")
        self.assertIn("one, two have not passed on c2", out, out)


def clean_git_env():
    """The caller's environment without GIT_* (the commit hook sets GIT_DIR
    and GIT_INDEX_FILE) and without the user's git configuration."""
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    return env


class RealGit(unittest.TestCase):
    """docs_test.Git against a throwaway repository: the ranges the hook
    asks about are the ones git names."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="docs-git-test-")
        self.env = clean_git_env()
        self.git = dt.Git(cwd=self.dir, env=self.env)
        self.sh("init", "-q", "-b", "main")
        self.shas = {}
        for name, path, text in (("c0", "crates/x.rs", "0"), ("c1", "crates/x.rs", "1"), ("c2", "crates/x.rs", "0"), ("c3", "b.txt", "3")):
            os.makedirs(os.path.dirname(os.path.join(self.dir, path)) or self.dir, exist_ok=True)
            with open(os.path.join(self.dir, path), "w", encoding="utf-8") as f:
                f.write(text)
            self.sh("add", path)
            self.sh("-c", "user.name=T", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false",
                    "commit", "-q", "-m", name)
            self.shas[name] = self.sh("rev-parse", "HEAD").strip()
        self.sh("update-ref", "refs/remotes/origin/main", self.shas["c0"])

    def tearDown(self):
        shutil.rmtree(self.dir)

    def sh(self, *args):
        return subprocess.run(["git", *args], cwd=self.dir, env=self.env, capture_output=True, text=True, check=True).stdout

    def ref(self, local, remote):
        return dt.Ref("refs/heads/main", self.shas[local], "refs/heads/main", self.shas.get(remote, remote))

    # Success.
    def test_the_commits_of_a_ref_are_the_new_ones_newest_first(self):
        got = self.git.commits(self.ref("c3", "c0"), "origin")
        self.assertEqual(got, [self.shas["c3"], self.shas["c2"], self.shas["c1"]], "after c0")

    def test_the_paths_of_a_range_are_every_commits(self):
        self.assertEqual(sorted(set(self.git.changed(self.ref("c3", "c0"), "origin"))), ["b.txt", "crates/x.rs"], "both")

    def test_a_change_and_its_undoing_diff_to_nothing(self):
        self.assertEqual(self.git.diff(self.shas["c0"], self.shas["c2"]), [], "the same tree")

    def real_check(self, stamps):
        out = io.StringIO()
        code = dt.check(
            [self.ref("c3", "c0")], remote="origin", env={}, scenarios=FAKES, steps=fake_steps,
            read=lambda sha: stamps.get(sha), git=self.git, ci=no_ci, out=out,
        )
        return code, out.getvalue()

    def test_check_over_a_real_range_carries_a_stamp_whose_tree_holds(self):
        code, out = self.real_check({self.shas["c2"]: {"one", "two"}})
        self.assertEqual(code, 0, out)
        self.assertIn(f"docs scenario one: passed on {self.shas['c2']}, and the commits after it change nothing it runs", out, out)

    def test_an_ancestor_is_named(self):
        self.assertTrue(self.git.is_ancestor(self.shas["c0"], self.shas["c3"]), "c0 is under c3")
        self.assertFalse(self.git.is_ancestor(self.shas["c3"], self.shas["c0"]), "c3 is not under c0")

    # Failure.
    def test_an_unknown_commit_is_an_error(self):
        with self.assertRaises(dt.PageError):
            self.git.commits(self.ref("c3", "f" * 40), "origin")

    def test_check_over_a_real_range_refuses_a_stamp_whose_server_changed_after_it(self):
        code, out = self.real_check({self.shas["c1"]: {"one", "two"}})
        self.assertEqual(code, 1, "c2 changed crates/x.rs after c1 ran them")
        self.assertIn(f"one, two have not passed on {self.shas['c3']}", out, out)

    def test_an_unknown_ancestor_is_an_error(self):
        with self.assertRaises(dt.PageError):
            self.git.is_ancestor("f" * 40, self.shas["c3"])

    # Edge cases.
    def test_a_new_ref_counts_only_what_the_remote_lacks(self):
        got = self.git.commits(self.ref("c3", dt.ZERO_SHA), "origin")
        self.assertEqual(got, [self.shas["c3"], self.shas["c2"], self.shas["c1"]], "origin/main has c0")

    def test_a_diff_compares_the_two_trees_not_their_merge_base(self):
        self.sh("checkout", "-q", "-b", "side", self.shas["c0"])
        with open(os.path.join(self.dir, "side.txt"), "w", encoding="utf-8") as f:
            f.write("s")
        self.sh("add", "side.txt")
        self.sh("-c", "user.name=T", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "side")
        side = self.sh("rev-parse", "HEAD").strip()
        self.assertEqual(self.git.diff(self.shas["c3"], side), ["b.txt", "side.txt"], "what differs between the two")

    def test_a_diff_names_each_path_once(self):
        self.assertEqual(self.git.diff(self.shas["c0"], self.shas["c3"]), ["b.txt"], "x.rs is back as it was")


TREE = {
    "crates/vcon-server/src/lib.rs": "pub mod serve;\n#[cfg(test)]\nmod lib_tests;\npub mod mixed;\n",
    "crates/vcon-server/src/main.rs": "fn main() {}\n",
    "crates/vcon-server/src/serve.rs": (
        "mod listen;\n"
        "#[cfg(test)]\n"
        "mod start_tests;\n"
        "#[cfg(all(test, feature = \"postgres\"))]\n"
        "mod pg_tests;\n"
        "/// Docs.\n"
        "#[cfg(test)]\n"
        "// a note\n"
        "pub(crate) mod helper_tests;\n"
        "#[cfg(feature = \"s3\")]\n"
        "mod s3;\n"
        "#[cfg(not(test))]\n"
        "mod real;\n"
        "#[cfg(test)]\n"
        "#[path = \"elsewhere.rs\"]\n"
        "mod moved;\n"
    ),
    "crates/vcon-server/src/serve/start_tests.rs": "mod cases;\n",
    "crates/vcon-server/src/serve/start_tests/cases.rs": "#[test]\nfn t() {}\n",
    "crates/vcon-server/src/serve/listen.rs": "pub fn listen() {}\n",
    "crates/vcon-server/src/mixed.rs": "pub fn f() {}\n#[cfg(test)]\nmod tests {}\n",
    "crates/vcon-server/src/lib_tests/mod.rs": "mod deep;\n",
    "crates/vcon-server/src/lib_tests/deep.rs": "",
    "crates/vcon/src/lib.rs": "mod read;\n#[cfg(test)]\nmod read_tests;\n",
}


def test_only(path, tree=None):
    tree = TREE if tree is None else tree
    return dt.test_only(path, tree.get)


class TestOnlySource(unittest.TestCase):
    """A file that only `#[cfg(test)] mod` declarations reach is not built
    into the server the scenarios run; any other file under src/ is."""

    # Success: test-only files.
    def test_a_cfg_test_module_is_test_only(self):
        self.assertTrue(test_only("crates/vcon-server/src/serve/start_tests.rs"), "#[cfg(test)] mod start_tests;")

    def test_a_module_inside_a_test_module_is_test_only(self):
        self.assertTrue(test_only("crates/vcon-server/src/serve/start_tests/cases.rs"), "its parent is test-only")

    def test_cfg_all_with_test_is_test_only(self):
        self.assertTrue(test_only("crates/vcon-server/src/serve/pg_tests.rs"), "cfg(all(test, ...))")

    def test_doc_and_comment_lines_between_do_not_hide_the_attribute(self):
        self.assertTrue(test_only("crates/vcon-server/src/serve/helper_tests.rs"), "/// and // skipped, pub(crate)")

    def test_a_mod_rs_module_is_read_from_its_parent(self):
        self.assertTrue(test_only("crates/vcon-server/src/lib_tests/mod.rs"), "lib_tests/mod.rs, declared in lib.rs")

    def test_a_module_of_a_mod_rs_test_module_is_test_only(self):
        self.assertTrue(test_only("crates/vcon-server/src/lib_tests/deep.rs"), "declared in lib_tests/mod.rs")

    def test_the_library_crate_follows_the_same_rule(self):
        self.assertTrue(test_only("crates/vcon/src/read_tests.rs"), "crates/vcon too")

    def test_a_test_only_change_needs_no_scenario(self):
        run, skipped = dt.needed(["crates/vcon-server/src/serve/start_tests.rs"], FAKES, False, fake_steps,
                                 test_only=lambda p: test_only(p))
        self.assertEqual((run, skipped), ([], []), "nothing built into the server changed")

    def test_a_test_only_change_beside_a_page_runs_the_pages_scenario_only(self):
        run, _ = dt.needed(["crates/vcon-server/src/serve/start_tests.rs", PAGE_C], FAKES, False, fake_steps,
                           test_only=lambda p: test_only(p))
        self.assertEqual([n for n, _ in run], ["pages-only"], "the page still needs its scenario")

    def test_check_reads_the_declarations_at_the_tip(self):
        git = FakeGit([("c1", ["crates/vcon-server/src/serve/start_tests.rs"])])
        git.show = lambda sha, path: TREE.get(path) if sha == "c1" else None
        code, out = range_check(git, [ref_of("c1")], {}, ci=ci_must_not_be_asked)
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenarios: not needed", out, out)

    def test_a_later_test_only_change_does_not_void_a_stamp(self):
        git = FakeGit([("c1", [SRC]), ("c2", ["crates/vcon-server/src/serve/start_tests.rs"])])
        git.show = lambda sha, path: TREE.get(path)
        code, out = range_check(git, [ref_of("c2")], {"c1": {"one", "two"}}, ci=ci_must_not_be_asked)
        self.assertEqual(code, 0, out)

    def test_push_runs_nothing_for_a_test_only_change(self):
        run = FakeRun()
        code, out, _ = push_of(["crates/vcon-server/src/serve/start_tests.rs"], run, test_only=lambda p: test_only(p))
        self.assertEqual(code, 0, out)
        self.assertEqual(run.calls, [], "no build, no run")

    # Failure: still server source.
    def test_a_plain_module_is_server_source(self):
        self.assertFalse(test_only("crates/vcon-server/src/serve/listen.rs"), "mod listen;")

    def test_a_mixed_file_is_server_source(self):
        self.assertFalse(test_only("crates/vcon-server/src/mixed.rs"), "code beside its test module")

    def test_a_feature_cfg_is_server_source(self):
        self.assertFalse(test_only("crates/vcon-server/src/serve/s3.rs"), "cfg(feature)")

    def test_cfg_not_test_is_server_source(self):
        self.assertFalse(test_only("crates/vcon-server/src/serve/real.rs"), "built only outside tests")

    def test_a_path_attribute_is_server_source(self):
        self.assertFalse(test_only("crates/vcon-server/src/serve/moved.rs"), "#[path] moves the file")

    def test_a_crate_root_is_server_source(self):
        for path in ("crates/vcon-server/src/lib.rs", "crates/vcon-server/src/main.rs"):
            with self.subTest(path=path):
                self.assertFalse(test_only(path), "the crate itself")

    def test_an_undeclared_file_is_server_source(self):
        self.assertFalse(test_only("crates/vcon-server/src/serve/nowhere.rs"), "no declaration found")

    def test_an_unreadable_parent_is_server_source(self):
        tree = {k: v for k, v in TREE.items() if k != "crates/vcon-server/src/serve.rs"}
        self.assertFalse(test_only("crates/vcon-server/src/serve/start_tests.rs", tree), "cannot read serve.rs")

    def test_a_file_that_is_not_rust_is_server_source(self):
        self.assertFalse(test_only("crates/vcon-server/src/backend/postgres/schema_v13.sql"), "include_str!")

    def test_a_module_under_a_plain_module_is_server_source(self):
        tree = dict(TREE, **{"crates/vcon-server/src/serve/listen/inner.rs": "", "crates/vcon-server/src/serve/listen.rs": "mod inner;\n"})
        self.assertFalse(test_only("crates/vcon-server/src/serve/listen/inner.rs", tree), "listen is built in")

    def test_a_test_module_of_a_mixed_file_needs_every_scenario_when_the_file_changes(self):
        run, _ = dt.needed(["crates/vcon-server/src/mixed.rs"], FAKES, False, fake_steps, test_only=lambda p: test_only(p))
        self.assertEqual([n for n, _ in run], ["one", "two"], "mixed files count")

    def test_a_declaration_in_lib_and_main_counts_unless_both_are_test_only(self):
        tree = dict(TREE, **{"crates/vcon-server/src/main.rs": "mod both;\n",
                             "crates/vcon-server/src/lib.rs": "#[cfg(test)]\nmod both;\n",
                             "crates/vcon-server/src/both.rs": ""})
        self.assertFalse(test_only("crates/vcon-server/src/both.rs", tree), "the binary builds it")

    # Edge cases.
    def test_a_declaration_in_both_roots_under_cfg_test_is_test_only(self):
        tree = dict(TREE, **{"crates/vcon-server/src/main.rs": "#[cfg(test)]\nmod both;\n",
                             "crates/vcon-server/src/lib.rs": "#[cfg(test)]\nmod both;\n",
                             "crates/vcon-server/src/both.rs": ""})
        self.assertTrue(test_only("crates/vcon-server/src/both.rs", tree), "neither crate builds it outside tests")

    def test_a_blank_line_ends_the_attributes(self):
        tree = dict(TREE, **{"crates/vcon-server/src/lib.rs": "#[cfg(test)]\n\nmod gap;\n", "crates/vcon-server/src/gap.rs": ""})
        self.assertFalse(test_only("crates/vcon-server/src/gap.rs", tree), "rustc applies it, but this rule reads only adjacent lines")

    def test_a_similar_module_name_is_not_its_declaration(self):
        tree = dict(TREE, **{"crates/vcon-server/src/lib.rs": "#[cfg(test)]\nmod start_tests;\n"})
        self.assertFalse(test_only("crates/vcon-server/src/start.rs", tree), "start is not declared; start_tests is")

    def test_a_crate_root_declared_under_cfg_test_elsewhere_is_still_server_source(self):
        tree = dict(TREE, **{"crates/vcon-server/src/main.rs": "#[cfg(test)]\nmod lib;\n"})
        self.assertFalse(test_only("crates/vcon-server/src/lib.rs", tree), "lib.rs is the library crate's root")

    def test_an_asset_named_like_a_test_module_is_server_source(self):
        self.assertFalse(test_only("crates/vcon-server/src/lib_tests.js"), "not Rust, whatever its stem")

    def test_a_commented_out_declaration_does_not_count(self):
        tree = dict(TREE, **{"crates/vcon-server/src/lib.rs": "// #[cfg(test)]\n// mod old;\n"})
        self.assertFalse(test_only("crates/vcon-server/src/old.rs", tree), "no declaration")

    def test_a_raw_identifier_module_is_server_source(self):
        tree = dict(TREE, **{"crates/vcon-server/src/lib.rs": "#[cfg(test)]\nmod r#type;\n"})
        self.assertFalse(test_only("crates/vcon-server/src/type.rs", tree), "not read: counts")

    def test_cfg_test_with_spaces_is_read(self):
        tree = dict(TREE, **{"crates/vcon-server/src/lib.rs": "#[cfg( test )]\nmod spaced;\n"})
        self.assertTrue(test_only("crates/vcon-server/src/spaced.rs", tree), "spaces inside")

    def test_cfg_any_with_test_is_server_source(self):
        tree = dict(TREE, **{"crates/vcon-server/src/lib.rs": "#[cfg(any(test, feature = \"x\"))]\nmod either;\n"})
        self.assertFalse(test_only("crates/vcon-server/src/either.rs", tree), "built with the feature too")

    def test_a_tests_directory_path_is_not_asked(self):
        asked = []
        dt.needed(["crates/vcon-server/tests/serve.rs"], FAKES, False, fake_steps, test_only=lambda p: asked.append(p))
        self.assertEqual(asked, [], "a crate's tests/ is never server source")

    def test_a_page_change_never_asks(self):
        asked = []
        dt.needed([PAGE_B], FAKES, False, fake_steps, test_only=lambda p: asked.append(p))
        self.assertEqual(asked, [], "only files under src/")

    def test_without_a_reader_every_src_file_counts(self):
        self.assertEqual(needed_of(["crates/vcon-server/src/serve/start_tests.rs"])[0], ["one", "two"], "no reader")

    def test_git_show_reads_a_file_at_a_commit(self):
        run = FakeRun({"git show": (0, "mod x;\n")})
        self.assertEqual(dt.Git(run=run).show("abc", "crates/a/src/lib.rs"), "mod x;\n", "the text")
        self.assertEqual(run.calls[0][-1], "abc:crates/a/src/lib.rs", "sha:path")

    def test_git_show_of_a_missing_file_is_none(self):
        self.assertIsNone(dt.Git(run=FakeRun({"git show": (128, "")})).show("abc", "x.rs"), "unreadable")

    def test_each_module_file_is_read_once_per_commit(self):
        asked = []
        at = dt.test_only_at("c1", lambda sha, path: asked.append((sha, path)) or TREE.get(path))
        at("crates/vcon-server/src/serve/start_tests.rs")
        at("crates/vcon-server/src/serve/listen.rs")
        self.assertEqual(asked.count(("c1", "crates/vcon-server/src/serve.rs")), 1, asked)
        self.assertTrue(all(sha == "c1" for sha, _ in asked), "the commit pushed")

    def test_a_deep_chain_stops_at_the_crate_root(self):
        tree = {"crates/c/src/lib.rs": "#[cfg(test)]\nmod a;\n", "crates/c/src/a.rs": "mod b;\n", "crates/c/src/a/b.rs": "mod c;\n"}
        self.assertTrue(test_only("crates/c/src/a/b/c.rs", tree), "three levels under a test module")


SERVER_README = "crates/vcon-server/README.md"
LIB_README = "crates/vcon/README.md"
MANIFESTS = {
    "crates/vcon-server/Cargo.toml": '[package]\nname = "vcon-server"\nreadme = "README.md"\n',
    "crates/vcon/Cargo.toml": '[package]\nname = "vcon"\nreadme = "README.md"\n',
}
# The include lines of the crates as on main: the library's README is its
# crate documentation; the server includes SQL, not its README.
INCLUDES = [
    ("crates/vcon/src/lib.rs", '#![doc = include_str!("../README.md")]'),
    ("crates/vcon-server/src/backend/postgres/schema.rs", 'const V1: &str = include_str!("schema_v1.sql");'),
]


def readme(path, tree=None, includes=INCLUDES):
    tree = MANIFESTS if tree is None else tree
    return dt.crate_readme(path, tree.get, lambda: includes)


def at_readme(tree=None, includes=INCLUDES):
    """`needed`'s test_only for a commit with `tree` and `includes`."""
    tree = MANIFESTS if tree is None else tree
    return dt.test_only_at("c1", lambda sha, path: tree.get(path), lambda sha: includes)


def tree_includes(root):
    """The include lines of the real crates, as Git.includes reads them."""
    found = []
    for path in sorted(pathlib.Path(root, "crates").rglob("*.rs")):
        rel = path.relative_to(root).as_posix()
        for line in path.read_text(encoding="utf-8").splitlines():
            if dt.INCLUDE_MACRO.search(line):
                found.append((rel, line))
    return found


class CrateReadme(unittest.TestCase):
    """The file a crate's manifest names as its readme is packaged by cargo
    and read by no build, unless an include macro reads it: a change to it
    alone runs no scenario (check_package.py still checks the package)."""

    # Success: the server crate's README is not server source.
    def test_the_server_readme_is_no_server_source(self):
        self.assertTrue(readme(SERVER_README), "no include reads it")

    def test_a_server_readme_change_needs_no_scenario(self):
        run, skipped = dt.needed([SERVER_README], FAKES, False, fake_steps, test_only=at_readme())
        self.assertEqual((run, skipped), ([], []), "nothing a build reads changed")

    def test_a_server_readme_change_beside_a_page_runs_the_pages_scenario_only(self):
        run, _ = dt.needed([SERVER_README, PAGE_C], FAKES, False, fake_steps, test_only=at_readme())
        self.assertEqual([n for n, _ in run], ["pages-only"], "the page still needs its scenario")

    def test_a_server_readme_change_beside_server_source_runs_every_scenario(self):
        run, _ = dt.needed([SERVER_README, SRC], FAKES, False, fake_steps, test_only=at_readme())
        self.assertEqual([n for n, _ in run], ["one", "two"], "the source still counts")

    def test_push_runs_nothing_for_a_server_readme_change(self):
        run = FakeRun()
        code, out, _ = push_of([SERVER_README], run, test_only=at_readme())
        self.assertEqual(code, 0, out)
        self.assertEqual(run.calls, [], "no build, no run")

    def test_check_reads_the_manifest_and_the_includes_at_the_tip(self):
        git = FakeGit([("c1", [SERVER_README])])
        git.show = lambda sha, path: MANIFESTS.get(path) if sha == "c1" else None
        git.includes = lambda sha: INCLUDES if sha == "c1" else None
        code, out = range_check(git, [ref_of("c1")], {}, ci=ci_must_not_be_asked)
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenarios: not needed", out, out)

    def test_a_later_server_readme_change_does_not_void_a_stamp(self):
        git = FakeGit([("c1", [SRC]), ("c2", [SERVER_README])])
        git.show = lambda sha, path: MANIFESTS.get(path)
        git.includes = lambda sha: INCLUDES
        code, out = range_check(git, [ref_of("c2")], {"c1": {"one", "two"}}, ci=ci_must_not_be_asked)
        self.assertEqual(code, 0, out)

    def test_a_readme_in_a_subdirectory_is_the_one_named(self):
        tree = {"crates/x/Cargo.toml": '[package]\nname = "x"\nreadme = "docs/README.md"\n'}
        self.assertTrue(readme("crates/x/docs/README.md", tree), "the manifest names it")

    def test_a_readme_named_with_a_dot_slash_is_the_same_file(self):
        tree = {"crates/x/Cargo.toml": '[package]\nname = "x"\nreadme = "./README.md"\n'}
        self.assertTrue(readme("crates/x/README.md", tree), "the path, normalized")

    def test_an_include_of_another_file_does_not_count(self):
        self.assertTrue(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", 'include_str!("notes.md")')]), "notes.md is not it")

    def test_the_library_readme_include_does_not_count_for_the_server(self):
        self.assertTrue(readme(SERVER_README, includes=INCLUDES[:1]), "crates/vcon's README is another file")

    def test_the_real_server_readme_is_no_server_source(self):
        root = pathlib.Path(dt.ROOT)
        read = lambda p: (root / p).read_text(encoding="utf-8") if (root / p).is_file() else None  # noqa: E731
        self.assertTrue(dt.crate_readme(SERVER_README, read, lambda: tree_includes(root)), "as the tree is")

    def test_git_includes_reads_each_include_line_at_a_commit(self):
        run = FakeRun({"git grep": (0, 'abc:crates/vcon/src/lib.rs\x003\x00#![doc = include_str!("../README.md")]\n')})
        self.assertEqual(dt.Git(run=run).includes("abc"), [("crates/vcon/src/lib.rs", '#![doc = include_str!("../README.md")]')])

    def test_git_includes_with_no_match_is_empty(self):
        self.assertEqual(dt.Git(run=FakeRun({"git grep": (1, "")})).includes("abc"), [], "git grep exits 1: no line")

    def test_git_includes_leaves_out_a_file_that_is_not_rust(self):
        run = FakeRun({"git grep": (0, 'abc:crates/vcon/notes.md\x001\x00use include_str!("../README.md")\n')})
        self.assertEqual(dt.Git(run=run).includes("abc"), [], "a page that shows an include is no source")

    def test_both_push_forms_read_the_includes(self):
        seen = []
        with unittest.mock.patch.object(dt, "push", return_value=0), \
                unittest.mock.patch.object(dt, "test_only_at", lambda *a: seen.append(len(a))), \
                unittest.mock.patch.object(dt, "changed_since", return_value=[]), \
                unittest.mock.patch.object(dt, "range_commits", return_value=[]), \
                unittest.mock.patch("sys.stdin", stdin_of("")):
            dt.main(["push", "abc123"])
            dt.main(["push", "abc123", "--since", "origin/main"])
        self.assertEqual(seen, [3, 3], "show and includes, for each form")

    def test_git_includes_asks_git_grep_at_the_commit_under_crates(self):
        run = FakeRun({"git grep": (1, "")})
        dt.Git(run=run).includes("abc")
        self.assertIn("abc", run.calls[0], run.calls[0])
        self.assertEqual(run.calls[0][-2:], ["--", "crates/"], run.calls[0])

    # Failure: still server source.
    def test_the_library_readme_is_server_source(self):
        self.assertFalse(readme(LIB_README), "lib.rs includes it as the crate's documentation")

    def test_a_library_readme_change_needs_every_scenario(self):
        run, _ = dt.needed([LIB_README], FAKES, False, fake_steps, test_only=at_readme())
        self.assertEqual([n for n, _ in run], ["one", "two"], "a build reads it")

    def test_the_real_library_readme_is_server_source(self):
        root = pathlib.Path(dt.ROOT)
        read = lambda p: (root / p).read_text(encoding="utf-8") if (root / p).is_file() else None  # noqa: E731
        self.assertFalse(dt.crate_readme(LIB_README, read, lambda: tree_includes(root)), "include_str! in lib.rs")

    def test_a_manifest_with_no_readme_field_is_server_source(self):
        self.assertFalse(readme("crates/x/README.md", {"crates/x/Cargo.toml": '[package]\nname = "x"\n'}), "not named")

    def test_readme_false_is_server_source(self):
        self.assertFalse(readme("crates/x/README.md", {"crates/x/Cargo.toml": '[package]\nreadme = false\n'}), "none named")

    def test_a_workspace_inherited_readme_is_server_source(self):
        self.assertFalse(readme("crates/x/README.md", {"crates/x/Cargo.toml": '[package]\nreadme.workspace = true\n'}), "not read here")

    def test_an_unreadable_manifest_is_server_source(self):
        self.assertFalse(readme(SERVER_README, {}), "no manifest at the commit")

    def test_a_manifest_that_is_not_toml_is_server_source(self):
        self.assertFalse(readme(SERVER_README, {"crates/vcon-server/Cargo.toml": "[package\nreadme = "}), "cannot read it")

    def test_includes_that_cannot_be_read_are_server_source(self):
        self.assertFalse(readme(SERVER_README, includes=None), "cannot tell")

    def test_an_include_that_is_not_a_plain_literal_is_server_source(self):
        line = '#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))]'
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/lib.rs", line)]), "it may read it")

    def test_include_bytes_of_the_readme_is_server_source(self):
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", 'include_bytes!("../README.md")')]), "bytes")

    def test_include_of_the_readme_is_server_source(self):
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", 'include!("../README.md");')]), "include!")

    def test_a_build_script_that_names_the_readme_is_server_source(self):
        tree = dict(MANIFESTS, **{"crates/vcon-server/build.rs": 'fn main() { std::fs::read("README.md").unwrap(); }\n'})
        self.assertFalse(readme(SERVER_README, tree), "build.rs reads it")

    def test_a_named_build_script_that_names_the_readme_is_server_source(self):
        tree = {"crates/x/Cargo.toml": '[package]\nreadme = "README.md"\nbuild = "tools/gen.rs"\n', "crates/x/tools/gen.rs": '// README.md\n'}
        self.assertFalse(readme("crates/x/README.md", tree), "the manifest's build script reads it")

    def test_a_file_that_is_not_the_named_readme_is_server_source(self):
        tree = {"crates/x/Cargo.toml": '[package]\nreadme = "docs/intro.md"\n'}
        self.assertFalse(readme("crates/x/README.md", tree), "the readme is docs/intro.md")

    def test_the_crate_manifest_is_server_source(self):
        run, _ = dt.needed(["crates/vcon-server/Cargo.toml"], FAKES, False, fake_steps, test_only=at_readme())
        self.assertEqual([n for n, _ in run], ["one", "two"], "the manifest builds the server")

    def test_a_raw_string_include_is_server_source(self):
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", 'include_str!(r"../README.md")')]), "not a plain literal")

    def test_an_include_with_an_escape_is_server_source(self):
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", 'include_str!("..\\x2fREADME.md")')]), "not read")

    def test_git_includes_that_fail_are_none(self):
        self.assertIsNone(dt.Git(run=FakeRun({"git grep": (128, "")})).includes("abc"), "cannot tell")

    def test_a_readme_named_with_another_case_is_server_source(self):
        self.assertFalse(readme("crates/vcon-server/readme.md"), "README.md is named")

    # Edge cases.
    def test_a_readme_outside_the_crates_is_never_asked(self):
        asked = []
        dt.needed(["README.md"], FAKES, False, fake_steps, test_only=lambda p: asked.append(p))
        self.assertEqual(asked, [], "not under crates/")

    def test_a_path_with_no_crate_is_not_a_readme(self):
        self.assertFalse(readme("crates/README.md", {"crates/Cargo.toml": '[package]\nreadme = "README.md"\n'}), "no crate")

    def test_an_include_with_spaces_is_read(self):
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", 'include_str!( "../README.md" )')]), "resolved")

    def test_an_include_from_a_nested_module_is_resolved_from_its_file(self):
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a/b.rs", 'include_str!("../../README.md")')]), "two up")

    def test_an_include_of_a_readme_of_the_same_name_elsewhere_does_not_count(self):
        self.assertTrue(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", 'include_str!("README.md")')]), "src/README.md")

    def test_the_second_include_on_a_line_counts(self):
        line = 'const A: &str = include_str!("a.sql"); const B: &str = include_str!("../README.md");'
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", line)]), "both are read")

    def test_an_include_from_another_crate_counts(self):
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon/src/a.rs", 'include_str!("../../vcon-server/README.md")')]), "across crates")

    def test_a_commented_out_include_still_counts(self):
        # Comments are not parsed: counting is the safe side.
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", '// include_str!("../README.md")')]), "safe side")

    def test_a_readme_under_src_that_is_named_and_not_included_is_no_source(self):
        tree = {"crates/x/Cargo.toml": '[package]\nreadme = "src/README.md"\n'}
        self.assertTrue(readme("crates/x/src/README.md", tree), "not Rust, not included")

    def test_without_a_reader_the_readme_counts(self):
        self.assertEqual(needed_of([SERVER_README])[0], ["one", "two"], "no reader")

    def test_the_includes_are_read_once_per_commit(self):
        asked = []
        at = dt.test_only_at("c1", lambda sha, path: MANIFESTS.get(path), lambda sha: asked.append(sha) or INCLUDES)
        at(SERVER_README)
        at(LIB_README)
        self.assertEqual(asked, ["c1"], "one git grep for the commit")

    def test_a_source_change_reads_no_includes(self):
        asked = []
        at = dt.test_only_at("c1", lambda sha, path: TREE.get(path), lambda sha: asked.append(sha) or INCLUDES)
        at("crates/vcon-server/src/serve/listen.rs")
        self.assertEqual(asked, [], "only a readme asks")

    def test_a_readme_value_that_is_not_text_is_server_source(self):
        self.assertFalse(readme("crates/x/README.md", {"crates/x/Cargo.toml": '[package]\nreadme = 5\n'}), "not a path")

    def test_a_manifest_with_no_package_table_is_server_source(self):
        self.assertFalse(readme("crates/x/README.md", {"crates/x/Cargo.toml": '[workspace]\nmembers = []\n'}), "no [package]")

    def test_a_dot_slash_include_is_resolved(self):
        self.assertFalse(readme(SERVER_README, includes=[("crates/vcon-server/src/a.rs", 'include_str!("./../README.md")')]), "normalized")

    def test_without_a_grep_the_readme_counts(self):
        at = dt.test_only_at("c1", lambda sha, path: MANIFESTS.get(path))
        self.assertFalse(at(SERVER_README), "no include list: cannot tell")


def run_output(note):
    return f"docs-test: {note}\nok       docs/c.md:3\n"


class SubstitutedRelease(unittest.TestCase):
    """The summary line and the stamp name the release a scenario really
    ran when the page's release is not published yet."""

    NOTE = "the release a page names is not published yet; running with VERSION=0.4.6"

    # Success.
    def test_the_note_is_the_one_plan_prints(self):
        steps = [dt.Block("sh", "VERSION=0.4.7\necho $VERSION\n", "docs/c.md:1")]
        _, _, notes = dt.plan(steps, {}, lambda v: False, lambda: "0.4.6")
        self.assertEqual(notes, [self.NOTE], "plan's note")
        self.assertEqual(dt.substituted(run_output(notes[0])), ["VERSION=0.4.6"], "read back by the same rule")

    def test_the_summary_names_the_release_run(self):
        run = FakeRun({"run pages-only": (0, run_output(self.NOTE))})
        code, out, _ = push_of([PAGE_C], run)
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenario pages-only: ok in ", out, out)
        self.assertIn("; it ran VERSION=0.4.6 in place of the unpublished release its page names", out, out)

    def test_the_stamp_names_the_release_run(self):
        stamped = []
        push_of([PAGE_C], FakeRun({"run pages-only": (0, run_output(self.NOTE))}), stamped=stamped)
        self.assertEqual(stamped, [("abc123", {"pages-only": "VERSION=0.4.6"})], "kept beside the name")

    def test_a_run_of_the_named_release_has_no_note(self):
        stamped = []
        code, out, _ = push_of([PAGE_C], FakeRun(), stamped=stamped)
        self.assertNotIn("in place of", out, out)
        self.assertEqual(stamped, [("abc123", {"pages-only": ""})], "nothing substituted")

    def test_two_substitutions_are_both_named(self):
        text = run_output(self.NOTE) + "docs-test: the release a page names is not published yet; running with NEW_VERSION=0.4.6\n"
        self.assertEqual(dt.substituted(text), ["VERSION=0.4.6", "NEW_VERSION=0.4.6"], "both")

    def test_the_stamp_file_keeps_the_release(self):
        d = tempfile.mkdtemp(prefix="docs-stamp-test-")
        try:
            dt.write_stamp("abc123", {"container": "VERSION=0.4.6", "s3": ""}, git_dir=d)
            self.assertEqual(dt.read_stamp("abc123", git_dir=d), {"container": "VERSION=0.4.6", "s3": ""}, "read back")
            with open(os.path.join(d, "docs-scenarios-abc123.pass"), encoding="utf-8") as f:
                self.assertEqual(f.read(), "container VERSION=0.4.6\ns3\n", "one scenario a line, the release after it")
        finally:
            shutil.rmtree(d)

    def test_check_repeats_the_release_of_a_stamp(self):
        git = FakeGit([("c1", [PAGE_C])])
        code, out = range_check(git, [ref_of("c1")], {"c1": {"pages-only": "VERSION=0.4.6"}})
        self.assertEqual(code, 0, out)
        self.assertIn("docs scenario pages-only: passed on c1 before this push; it ran VERSION=0.4.6 in place of", out, out)

    # Failure.
    def test_a_failed_run_with_a_substitution_stamps_nothing(self):
        stamped = []
        code, out, _ = push_of([PAGE_C], FakeRun({"run pages-only": (1, run_output(self.NOTE))}), stamped=stamped)
        self.assertEqual((code, stamped), (1, []), out)

    def test_a_since_note_is_not_a_substitution(self):
        text = "docs-test: docs/c.md:3 is for release 0.4.7 and later; this run's release is older, so it does not run\n"
        self.assertEqual(dt.substituted(text), [], "a step left out names no release run")

    def test_a_note_in_a_steps_output_is_not_read(self):
        text = "ok       docs/c.md:3\n    the release a page names is not published yet; running with VERSION=9\n"
        self.assertEqual(dt.substituted(text), [], "only the runner's own docs-test: lines")

    def test_the_note_inside_another_line_is_not_read(self):
        text = "FAIL     docs/c.md:3: line 1 differs: docs-test: the release a page names is not published yet; running with VERSION=9\n"
        self.assertEqual(dt.substituted(text), [], "only a line of the runner's own")

    def test_a_note_without_a_value_is_not_read(self):
        self.assertEqual(dt.substituted("docs-test: the release a page names is not published yet; running with VERSION=\n"), [], "no value")

    def test_an_unknown_variable_is_not_read(self):
        text = "docs-test: the release a page names is not published yet; running with PATH=/x\n"
        self.assertEqual(dt.substituted(text), [], "only the release variables")

    def test_a_note_cut_short_is_not_read(self):
        self.assertEqual(dt.substituted("docs-test: the release a page names is not published yet\n"), [], "partial")

    # Edge cases.
    def test_a_stamp_line_without_a_release_reads_as_none_given(self):
        d = tempfile.mkdtemp(prefix="docs-stamp-test-")
        try:
            with open(os.path.join(d, "docs-scenarios-abc123.pass"), "w", encoding="utf-8") as f:
                f.write("one\ntwo VERSION=0.4.6\n")
            self.assertEqual(dt.read_stamp("abc123", git_dir=d), {"one": "", "two": "VERSION=0.4.6"}, "old and new lines")
        finally:
            shutil.rmtree(d)

    def test_a_second_run_keeps_the_first_runs_release(self):
        stamped = []
        push_of([PAGE_B, PAGE_C], FakeRun(), stamps={"abc123": {"pages-only": "VERSION=0.4.6"}}, stamped=stamped)
        self.assertEqual(stamped, [("abc123", {"one": "", "pages-only": "VERSION=0.4.6", "two": ""})], "merged")

    def test_a_carried_scenario_names_its_release(self):
        run = FakeRun()
        code, out, _ = push_of([PAGE_C], run, stamps={"abc123": {"pages-only": "VERSION=0.4.6"}})
        self.assertIn("passed on abc123 before; it ran VERSION=0.4.6 in place of", out, out)

    def test_a_set_stamp_still_reads(self):
        git = FakeGit([("c1", [PAGE_C])])
        code, out = range_check(git, [ref_of("c1")], {"c1": {"pages-only"}})
        self.assertEqual(code, 0, out)
        self.assertNotIn("in place of", out, "no release named")

    def test_the_same_substitution_twice_is_named_once(self):
        self.assertEqual(dt.substituted(run_output(self.NOTE) * 2), ["VERSION=0.4.6"], "once")

    def test_the_note_format_has_one_place(self):
        self.assertEqual(dt.SUBSTITUTED_NOTE.format(name="VERSION", value="1.2.3"),
                         "the release a page names is not published yet; running with VERSION=1.2.3", "the format")

    def test_the_summary_without_a_why_still_names_it(self):
        self.assertEqual(dt.release_words("VERSION=0.4.6"), "it ran VERSION=0.4.6 in place of the unpublished release its page names", "the words")
        self.assertEqual(dt.release_words(""), "", "nothing to say")

    def test_two_releases_join_with_and(self):
        self.assertEqual(dt.release_words("VERSION=0.4.6,NEW_VERSION=0.4.6"),
                         "it ran VERSION=0.4.6 and NEW_VERSION=0.4.6 in place of the unpublished releases its page names", "plural")


class PushCarries(unittest.TestCase):
    """`push --since` runs only what no stamp of its range already holds."""

    # Success.
    def test_a_scenario_an_earlier_stamp_holds_is_not_run_again(self):
        run, stamped = FakeRun(), []
        code, out, _ = push_of([SRC, PAGE_C], run, stamps={"c1": {"one", "two"}}, commits=["abc123", "c1"],
                               delta=lambda sha: [PAGE_C], stamped=stamped)
        self.assertEqual(code, 0, out)
        self.assertEqual([c[c.index("run") + 1] for c in run.names(sys.executable)], ["pages-only"], out)
        self.assertEqual(stamped, [("abc123", {"pages-only": ""})], "the tip records what ran on it")
        self.assertIn("docs scenario one: passed on c1, and the commits after it change nothing it runs; not run again", out, out)

    def test_nothing_left_to_run_builds_nothing(self):
        run, stamped = FakeRun(), []
        code, out, _ = push_of([SRC], run, stamps={"c1": {"one", "two"}}, commits=["abc123", "c1"],
                               delta=lambda sha: ["README.md"], stamped=stamped)
        self.assertEqual(code, 0, out)
        self.assertEqual(run.calls, [], "no git, no build, no run")
        self.assertEqual(stamped, [], "nothing new ran")
        self.assertIn("docs scenarios: each one this push needs has passed already", out, out)

    def test_a_second_run_on_the_tip_keeps_its_first_stamp(self):
        stamped = []
        code, out, _ = push_of([PAGE_B, PAGE_C], FakeRun(), stamps={"abc123": {"pages-only"}}, stamped=stamped)
        self.assertEqual(code, 0, out)
        self.assertEqual(stamped, [("abc123", {"one": "", "pages-only": "", "two": ""})], "both runs' scenarios")

    def test_push_since_passes_the_range_to_push(self):
        with unittest.mock.patch.object(dt, "push", return_value=0) as push, \
                unittest.mock.patch.object(dt, "changed_since", return_value=["Cargo.lock"]), \
                unittest.mock.patch.object(dt, "range_commits", return_value=["abc123", "c1"]) as commits:
            self.assertEqual(dt.main(["push", "abc123", "--since", "origin/main"]), 0, "its code")
        commits.assert_called_once_with("origin/main", "abc123")
        self.assertEqual(push.call_args.kwargs["commits"], ["abc123", "c1"], "the range's commits")
        self.assertTrue(callable(push.call_args.kwargs["delta"]), "and how to compare one with the tip")

    # Failure.
    def test_a_stamp_a_later_commit_voids_runs_again(self):
        run = FakeRun()
        code, out, _ = push_of([SRC], run, stamps={"c1": {"one", "two"}}, commits=["abc123", "c1"],
                               delta=lambda sha: [SRC])
        self.assertEqual(code, 0, out)
        self.assertEqual([c[c.index("run") + 1] for c in run.names(sys.executable)], ["one", "two"], out)

    def test_a_carried_scenario_does_not_hide_a_failure(self):
        stamped = []
        code, out, _ = push_of([SRC, PAGE_C], FakeRun({"run pages-only": (1, "x\n")}), stamps={"c1": {"one", "two"}},
                               commits=["abc123", "c1"], delta=lambda sha: [PAGE_C], stamped=stamped)
        self.assertEqual(code, 1, out)
        self.assertEqual(stamped, [], "a failure stamps nothing")

    # Edge cases.
    def test_without_a_range_only_the_tips_own_stamp_is_read(self):
        run = FakeRun()
        code, out, _ = push_of([PAGE_C], run, stamps={"c1": {"pages-only"}})
        self.assertEqual(code, 0, out)
        self.assertEqual(len(run.names(sys.executable)), 1, "c1 is not in a range nobody gave")

    def test_everything_carried_needs_no_checkout_of_the_commit(self):
        run = FakeRun({"git rev-parse": (0, "fff999\n")})
        code, out, _ = push_of([PAGE_C], run, stamps={"abc123": {"pages-only"}})
        self.assertEqual(code, 0, "nothing is built, so the checkout does not matter")


if __name__ == "__main__":
    unittest.main()
