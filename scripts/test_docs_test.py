#!/usr/bin/env python3
"""Tests of scripts/docs_test.py, the runner that holds the how-to pages to
a live server: how it reads a page, how it compares what a command printed
with what the page says it prints, the script it writes, and the rules it
holds every tested page to.

Run: python3 -I scripts/test_docs_test.py
"""

import os
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
        code = "printf READY\nstty -echo -icanon\ndd bs=1 count=1 >/dev/null 2>&1\nexit 1\n"
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
        code = (
            "printf READY\n"
            "stty -echo -icanon -icrnl\n"
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
        self.assertIn("Content is not shown", steps[1].expect, "the detail view")
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

    def test_the_mirrors_that_answer_are_kept_in_their_listed_order(self):
        chosen = dt.choose_mirrors((UBUNTU,), lambda url: True)
        self.assertEqual(chosen, {UBUNTU: list(dt.APT_MIRRORS[UBUNTU])})

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
        dead = {UBUNTU, UBUNTU_SECURITY}
        said = io.StringIO()
        with unittest.mock.patch.object(dt, "probe_mirror", lambda url, timeout=5: url not in dead), \
                unittest.mock.patch.object(dt.platform, "machine", lambda: "x86_64"), \
                unittest.mock.patch.object(dt, "page_blocks", side_effect=Stop), \
                contextlib.redirect_stdout(said), self.assertRaises(Stop):
            dt.run(scenario, None, None)
        azure, kernel, princeton = dt.APT_MIRRORS[UBUNTU_SECURITY][1:]
        self.assertEqual(
            said.getvalue().splitlines(),
            [
                f"docs-test: packages from {UBUNTU} come from {azure}, then {kernel}, then {princeton}",
                f"docs-test: packages from {UBUNTU_SECURITY} come from {azure}, then {kernel}, then {princeton}",
            ],
            "the two hosts that did not answer on 2026-11-10 are in neither list",
        )

    def started(self, where, **patches):
        """What a run of an apt scenario with no steps says, up to the
        point where a patched function raises Stop. Only the Debian origin
        itself does not answer."""
        scenario = dt.Scenario("x", where, frozenset({"apt"}), (), binary=False)
        said = io.StringIO()
        with contextlib.ExitStack() as stack:
            stack.enter_context(unittest.mock.patch.object(dt, "probe_mirror", lambda url, timeout=5: url != DEBIAN))
            for name, replacement in patches.items():
                stack.enter_context(unittest.mock.patch.object(dt, name, replacement))
            stack.enter_context(contextlib.redirect_stdout(said))
            stack.enter_context(self.assertRaises(Stop))
            dt.run(scenario, None, None)
        return said.getvalue()

    def test_a_run_gives_the_chosen_mirrors_to_the_containers_setup(self):
        preludes = []

        def script(steps, prelude, sets=None):
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

    def serve(self, status):
        """A local HTTP server that answers every request with `status`;
        returns its base URL and the function that stops it."""
        import http.server
        import threading

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                self.send_response(status)
                self.send_header("Content-Length", "0")
                self.end_headers()

        httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        self.addCleanup(httpd.server_close)
        self.addCleanup(httpd.shutdown)
        return f"http://127.0.0.1:{httpd.server_address[1]}/debian"

    def test_a_mirror_that_serves_its_index_answers(self):
        self.assertTrue(dt.probe_mirror(self.serve(200)))

    # Failure.

    def test_a_mirror_that_does_not_answer_is_left_out(self):
        dead = dt.APT_MIRRORS[UBUNTU][0]
        chosen = dt.choose_mirrors((UBUNTU,), lambda url: url != dead)
        self.assertEqual(chosen[UBUNTU], list(dt.APT_MIRRORS[UBUNTU][1:]))

    def test_no_mirror_for_an_origin_is_an_unreachable_mirror_not_a_page_failure(self):
        with self.assertRaises(dt.MirrorError) as raised:
            dt.choose_mirrors((UBUNTU, UBUNTU_SECURITY), lambda url: False)
        said = str(raised.exception)
        self.assertIn("no package mirror answered for " + UBUNTU, said)
        self.assertIn("the package mirrors are unreachable from this machine", said)
        self.assertIn("not a failure of the pages", said)
        for mirror in dt.APT_MIRRORS[UBUNTU]:
            self.assertIn(mirror, said, "each mirror tried is named")

    def test_one_origin_without_a_mirror_fails_even_when_another_has_one(self):
        with self.assertRaisesRegex(dt.MirrorError, "no package mirror answered for " + UBUNTU_SECURITY):
            dt.choose_mirrors((UBUNTU, UBUNTU_SECURITY), lambda url: url == UBUNTU)

    def test_a_mirror_that_answers_404_does_not_answer(self):
        self.assertFalse(dt.probe_mirror(self.serve(404)))

    def test_a_closed_port_does_not_answer(self):
        import socket
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        self.assertFalse(dt.probe_mirror(f"http://127.0.0.1:{port}/debian"))

    def test_a_mirror_that_accepts_and_sends_nothing_is_given_up(self):
        import socket
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            s.listen(1)
            started = time.monotonic()
            self.assertFalse(dt.probe_mirror(f"http://127.0.0.1:{s.getsockname()[1]}/debian", timeout=1))
            self.assertLess(time.monotonic() - started, 5, "the wait is the timeout, not the scenario's 900 s")

    def test_a_run_with_no_mirror_ends_at_once_and_says_the_mirrors_are_unreachable(self):
        (scenario,) = [s for s in dt.SCENARIOS if s.name == "release-ubuntu"]
        said = io.StringIO()
        with unittest.mock.patch.object(dt, "probe_mirror", lambda url, timeout=5: False), \
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
            with unittest.mock.patch.object(dt, "probe_mirror", lambda url, timeout=5: asked.append(url)), \
                    unittest.mock.patch.object(dt, "script", side_effect=Stop), \
                    contextlib.redirect_stdout(said), self.assertRaises(Stop):
                dt.run(scenario, None, None)
            self.assertEqual((asked, said.getvalue()), ([], ""), where)

    def test_a_dnf_scenario_gets_no_apt_setup(self):
        self.assertEqual(dt.runner_prelude(self.scenario("fedora:44", ("dnf",)), (), {}), "")

    def test_the_probe_asks_for_the_mirrors_index_of_suites(self):
        asked = []
        import http.server
        import threading

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                asked.append(self.path)
                self.send_response(200)
                self.send_header("Content-Length", "0")
                self.end_headers()

        httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        self.addCleanup(httpd.server_close)
        self.addCleanup(httpd.shutdown)
        base = f"http://127.0.0.1:{httpd.server_address[1]}"
        dt.probe_mirror(base + "/debian")
        dt.probe_mirror(base + "/ubuntu/")
        self.assertEqual(asked, ["/debian/dists/", "/ubuntu/dists/"], "one slash, with or without the mirror's own")


if __name__ == "__main__":
    unittest.main()
