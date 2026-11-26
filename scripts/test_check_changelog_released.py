"""Tests scripts/check_changelog_released.py: a commit leaves the released
part of CHANGELOG.md as its base has it.

The three files of scripts/testdata/changelog-released/ are CHANGELOG.md of
three commits, with the sections [0.4.15] down to [0.1.0] cut out (each of
the three holds them unchanged, and the link definitions at the end are
kept): base.md is 823aca7, the merge base; main.md is 9dc303b, the 0.4.18
release on main; branch.md is 7bf77f0, a branch that added an [Unreleased]
entry. `git merge-file` of the three exits 0 and puts the branch's entry
under `## [0.4.18] - 2026-11-26`, which scripts/check_changelog.py accepts
(2026-11-26).

Run with: python3 -I scripts/test_check_changelog_released.py
"""

import contextlib
import hashlib
import importlib.util
import io
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "check_changelog_released.py"
spec = importlib.util.spec_from_file_location("check_changelog_released", SCRIPT)
ccr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ccr)
spec = importlib.util.spec_from_file_location("check_changelog", HERE / "check_changelog.py")
cc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cc)

DATA = HERE / "testdata" / "changelog-released"
PIN = ".config/changelog-released.sha256"

# A git hook runs with GIT_DIR and GIT_INDEX_FILE set to the repository the
# hook belongs to. Every git command of these tests runs without them, so
# it acts on the test's own directory only.
ENV = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
ENV["GIT_CONFIG_GLOBAL"] = os.devnull
ENV["GIT_CONFIG_SYSTEM"] = os.devnull
ENV["GIT_CEILING_DIRECTORIES"] = tempfile.gettempdir()
ENV["GIT_AUTHOR_NAME"] = ENV["GIT_COMMITTER_NAME"] = "Test User"
ENV["GIT_AUTHOR_EMAIL"] = ENV["GIT_COMMITTER_EMAIL"] = "test@example.com"

TOP = "# Changelog\n\nAll notable changes.\n\n## [Unreleased]\n\n"
ENTRY_A = "- **A server keeps its catalogs in PostgreSQL.**\n  - Effect: The registry is in the database.\n"
ENTRY_B = "- **A key backend wraps tenant keys.**\n  - Effect: A reader knows the backend.\n"
ENTRY_C = "- **Pack files are verified on read.**\n  - Effect: A bad pack is refused.\n"
DEFS_2 = (
    "[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.2.0...HEAD\n"
    "[0.2.0]: https://github.com/NormB/vcon-server/releases/tag/v0.2.0\n"
    "[0.1.0]: https://github.com/NormB/vcon-server/releases/tag/v0.1.0\n"
)
OLD = "## [0.1.0] - 2026-10-26\n\n### Added\n\n" + ENTRY_C + "\n"
RELEASED_2 = "## [0.2.0] - 2026-11-01\n\n### Added\n\n" + ENTRY_A + "\n" + OLD


def log(unreleased="", released=RELEASED_2, defs=DEFS_2):
    """A changelog: the [Unreleased] section's text, the released sections
    and the link definitions."""
    return TOP + unreleased + ("\n" if unreleased else "") + released + defs


def digest(text):
    return ccr.digest(text)


def fixture(name):
    return (DATA / name).read_text(encoding="utf-8")


def merged():
    """git merge-file of the fixtures: ours main.md, base base.md, theirs
    branch.md, as a rebase of the branch onto main merges them."""
    with tempfile.TemporaryDirectory() as tmp:
        for name in ("main.md", "base.md", "branch.md"):
            pathlib.Path(tmp, name).write_text(fixture(name), encoding="utf-8")
        run = subprocess.run(
            ["git", "merge-file", "-p", "main.md", "base.md", "branch.md"],
            cwd=tmp, env=ENV, capture_output=True, text=True, check=False,
        )
    return run.returncode, run.stdout


class Rule(unittest.TestCase):
    """problems(): the released part of ours against one base."""

    def ok(self, ours, base, ours_pin="", base_pin=""):
        self.assertEqual(ccr.problems(ours, base, ours_pin, base_pin, "the base"), [])

    def refused(self, ours, base, ours_pin="", base_pin=""):
        found = ccr.problems(ours, base, ours_pin, base_pin, "the base")
        self.assertEqual(len(found), 1, found)
        return found[0]

    # Success: what passes.

    def test_success_the_same_file(self):
        self.ok(log("### Added\n\n" + ENTRY_B), log("### Added\n\n" + ENTRY_B))

    def test_success_an_entry_added_to_unreleased(self):
        self.ok(log("### Added\n\n" + ENTRY_B), log())

    def test_success_an_unreleased_entry_removed(self):
        self.ok(log(), log("### Added\n\n" + ENTRY_B))

    def test_success_a_release_turns_unreleased_into_a_section(self):
        base = log("### Added\n\n" + ENTRY_B)
        ours = log(released="## [0.3.0] - 2026-11-20\n\n### Added\n\n" + ENTRY_B + "\n" + RELEASED_2,
                   defs="[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.3.0...HEAD\n"
                   "[0.3.0]: https://github.com/NormB/vcon-server/releases/tag/v0.3.0\n" + DEFS_2.split("\n", 1)[1])
        self.ok(ours, base)

    def test_success_two_releases_on_top_at_once(self):
        ours = log(released="## [0.4.0] - 2026-11-21\n\n### Fixed\n\n" + ENTRY_B + "\n"
                   "## [0.3.0] - 2026-11-20\n\n### Added\n\n" + ENTRY_C + "\n" + RELEASED_2)
        self.ok(ours, log())

    def test_success_the_real_0_4_18_release_against_the_merge_base(self):
        self.ok(fixture("main.md"), fixture("base.md"))

    def test_success_the_real_branch_against_the_merge_base(self):
        self.ok(fixture("branch.md"), fixture("base.md"))

    def test_success_a_new_definition_and_the_unreleased_link_moved(self):
        ours = log(defs="[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.3.0...HEAD\n"
                   "[0.3.0]: https://github.com/NormB/vcon-server/releases/tag/v0.3.0\n"
                   + DEFS_2.split("\n", 1)[1])
        self.ok(ours, log())

    def test_success_a_deliberate_edit_with_the_pin_moved_to_its_digest(self):
        ours = log(released=RELEASED_2.replace("in PostgreSQL", "in a PostgreSQL database"))
        self.ok(ours, log(), ours_pin=digest(ours), base_pin=digest(log()))

    def test_success_a_deliberate_edit_when_the_base_has_no_pin(self):
        ours = log(released=RELEASED_2.replace("### Added", "### Changed", 1))
        self.ok(ours, log(), ours_pin=digest(ours))

    def test_success_a_base_without_a_changelog(self):
        self.ok(log(), None)

    def test_success_the_first_release_of_a_changelog(self):
        base = TOP + "### Added\n\n" + ENTRY_C + "\n"
        self.ok(log(), base)

    def test_success_the_digest_the_message_prints_is_the_fix(self):
        # One rule: the digest the refusal names makes the same edit pass.
        ours = log(released=RELEASED_2.replace("registry", "catalog registry"))
        problem = self.refused(ours, log())
        printed = problem.split("writes ")[1].split(" ")[0]
        released = ccr.released(ours)
        self.assertEqual(printed, hashlib.sha256("".join(released.body + released.definitions).encode()).hexdigest())
        self.ok(ours, log(), ours_pin=printed)

    # Failure: what is refused.

    def test_failure_the_reproduction_names_the_line_and_its_heading(self):
        code, text = merged()
        self.assertEqual(code, 0, "git merge-file no longer merges the fixtures cleanly")
        problem = self.refused(text, fixture("main.md"))
        line = text.split("\n").index(
            "- **On-disk format `key-wrap` version 1 records a key a key backend wrapped.**") + 1
        self.assertEqual(line, 18)
        self.assertIn("line 18, under `## [0.4.18] - 2026-11-26`: "
                      "`- **On-disk format `key-wrap` version 1 records a key a key backend wrapped.**`", problem)

    def test_failure_an_unreleased_entry_moved_into_an_older_section(self):
        base = log("### Added\n\n" + ENTRY_B)
        ours = log(released=RELEASED_2.replace(ENTRY_C, ENTRY_C + ENTRY_B))
        problem = self.refused(ours, base)
        self.assertIn("under `## [0.1.0] - 2026-10-26`: `- **A key backend wraps tenant keys.**`", problem)

    def test_failure_a_released_line_reworded(self):
        problem = self.refused(log(released=RELEASED_2.replace("in the database", "in PostgreSQL")), log())
        self.assertIn("the base's line", problem)
        self.assertIn("in the database", problem)

    def test_failure_a_released_line_deleted(self):
        problem = self.refused(log(released=RELEASED_2.replace("  - Effect: A bad pack is refused.\n", "")), log())
        self.assertIn("under `## [0.1.0] - 2026-10-26`: `  - Effect: A bad pack is refused.`", problem)

    def test_failure_a_released_section_removed(self):
        self.refused(log(released=RELEASED_2.replace(OLD, "")), log())

    def test_failure_a_blank_line_added_inside_a_released_section(self):
        problem = self.refused(log(released=RELEASED_2.replace("### Added\n\n", "### Added\n\n\n", 1)), log())
        self.assertIn("(a blank line)", problem)

    def test_failure_a_new_section_between_released_sections(self):
        ours = log(released=RELEASED_2.replace(OLD, "## [0.1.5] - 2026-10-30\n\n### Fixed\n\n" + ENTRY_B + "\n" + OLD))
        self.refused(ours, log())

    def test_failure_a_released_definition_changed(self):
        ours = log(defs=DEFS_2.replace("tag/v0.1.0", "tag/v0.1.1"))
        problem = self.refused(ours, log())
        self.assertIn("`[0.1.0]: https://github.com/NormB/vcon-server/releases/tag/v0.1.0`", problem)

    def test_failure_a_released_definition_removed(self):
        self.refused(log(defs=DEFS_2.rsplit("[0.1.0]", 1)[0]), log())

    def test_failure_the_changelog_deleted(self):
        self.refused(None, log())

    def test_failure_a_pin_moved_to_another_digest(self):
        ours = log(released=RELEASED_2.replace("in PostgreSQL", "in a PostgreSQL database"))
        problem = self.refused(ours, log(), ours_pin=digest(log()), base_pin="")
        self.assertIn(digest(ours), problem)

    def test_failure_a_pin_that_did_not_move_excuses_nothing(self):
        ours = log(released=RELEASED_2.replace("in PostgreSQL", "in a PostgreSQL database"))
        self.refused(ours, log(), ours_pin=digest(ours), base_pin=digest(ours))

    def test_failure_a_moved_pin_that_is_not_the_digest_with_no_edit(self):
        problem = self.refused(log(), log(), ours_pin="0" * 64, base_pin="")
        self.assertIn("is not the sha256 of the released part", problem)

    # Edge cases.

    def test_edge_check_changelog_alone_accepts_the_reproduction(self):
        # The gap this script closes: the format gate passes the merged file.
        # The fixture's oldest section is [0.4.16]; the whole file's is
        # cc.MARKER, and the gate passed the whole merged file too.
        code, text = merged()
        self.assertEqual(code, 0)
        self.assertEqual(cc.problems(text, "0.4.16"), [])

    def test_edge_crlf_line_endings_are_a_change(self):
        self.refused(log().replace("\n", "\r\n"), log())

    def test_edge_crlf_files_compared_with_themselves_pass(self):
        self.ok(log().replace("\n", "\r\n"), log().replace("\n", "\r\n"))

    def test_edge_unicode_text_and_line_numbers(self):
        base = log(released=RELEASED_2.replace("database", "Datenbank für Schlüssel"))
        ours = base.replace("## [0.1.0]", "- **Moved.**\n  - Effect: x.\n\n## [0.1.0]")
        problem = self.refused(ours, base)
        number = ours.split("\n").index("- **Moved.**") + 1
        self.assertIn(f"line {number}, under `## [0.2.0] - 2026-11-01`: `- **Moved.**`", problem)

    def test_edge_at_most_shown_lines_and_a_count_of_the_rest(self):
        many = "".join(f"- **Entry {n}.**\n  - Effect: {n}.\n" for n in range(20))
        problem = self.refused(log(released=RELEASED_2.replace(ENTRY_C, ENTRY_C + many)), log())
        self.assertEqual(problem.count("\n  line "), ccr.SHOWN)
        self.assertIn(f"and {40 - ccr.SHOWN} more", problem)

    def test_edge_a_prerelease_heading_is_released(self):
        ours = log(released=RELEASED_2.replace("## [0.2.0]", "## [1.0.0-rc.1]"))
        self.assertTrue(ccr.released(ours).body[0].startswith("## [1.0.0-rc.1]"))
        self.refused(ours, log())

    def test_edge_unreleased_alone_has_no_released_part(self):
        text = TOP + "### Added\n\n" + ENTRY_A
        self.assertEqual(ccr.released(text).body, [])

    def test_edge_a_definition_inside_the_body_is_body(self):
        released = RELEASED_2.replace(ENTRY_A, ENTRY_A + "\n[note]: https://example.com/\n")
        self.assertIn("[note]: https://example.com/\n", ccr.released(log(released=released)).body)

    def test_edge_a_file_without_definitions(self):
        self.ok(log(defs=""), log(defs=""))
        self.refused(log(released=RELEASED_2 + "x\n", defs=""), log(defs=""))

    def test_edge_the_unreleased_definition_is_not_released(self):
        self.assertNotIn(DEFS_2.split("\n")[0] + "\n", ccr.released(log()).definitions)
        self.assertEqual(len(ccr.released(log()).definitions), 2)

    def test_edge_pin_of_skips_comments_and_blank_lines(self):
        self.assertEqual(ccr.pin_of("# The sha256.\n\n  abc def\n"), "abc")
        self.assertEqual(ccr.pin_of(None), "")
        self.assertEqual(ccr.pin_of("# only a comment\n"), "")


class Message(unittest.TestCase):
    """message_problems(): a moved pin is named in the commit message."""

    def test_success_the_message_names_the_pin(self):
        self.assertEqual(ccr.message_problems(f"docs: reword 0.4.3\n\nMoves {PIN}.\n", True), [])

    def test_success_no_pin_moved_no_name_needed(self):
        self.assertEqual(ccr.message_problems("feat: a thing\n", False), [])

    def test_failure_a_moved_pin_the_message_does_not_name(self):
        found = ccr.message_problems("docs: reword 0.4.3\n", True)
        self.assertEqual(len(found), 1)
        self.assertIn(PIN, found[0])

    def test_failure_the_file_name_alone_is_not_the_path(self):
        self.assertEqual(len(ccr.message_problems("docs: changelog-released.sha256\n", True)), 1)

    def test_edge_a_comment_line_does_not_count(self):
        # git strips `#` lines from a message written in an editor.
        self.assertEqual(len(ccr.message_problems(f"docs: reword\n# {PIN}\n", True)), 1)


class Repository(unittest.TestCase):
    """The script's commands in a throwaway repository."""

    def setUp(self):
        self.root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory())).resolve()
        self.git("init", "-q", "-b", "main")
        self.git("config", "commit.gpgsign", "false")

    def git(self, *arguments):
        run = subprocess.run(["git", *arguments], cwd=self.root, env=ENV, capture_output=True, text=True,
                             check=False)
        self.assertEqual(run.returncode, 0, run.stderr)
        return run.stdout.strip()

    def write(self, text, pin=None):
        (self.root / "CHANGELOG.md").write_text(text, encoding="utf-8", newline="")
        self.git("add", "CHANGELOG.md")
        if pin is not None:
            (self.root / ".config").mkdir(exist_ok=True)
            (self.root / PIN).write_text(f"# The pin.\n{pin}\n", encoding="utf-8")
            self.git("add", PIN)

    def commit(self, text, message="change", pin=None):
        self.write(text, pin)
        self.git("commit", "-q", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")

    def run_script(self, *arguments):
        return subprocess.run([sys.executable, "-I", str(SCRIPT), *arguments], cwd=self.root, env=ENV,
                              capture_output=True, text=True, check=False, timeout=60)

    # Success.

    def test_success_index_with_an_unreleased_entry_added(self):
        self.git("update-ref", "refs/remotes/origin/main", self.commit(log()))
        self.write(log("### Added\n\n" + ENTRY_B))
        run = self.run_script("index")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertIn("check_changelog_released: ok", run.stdout)

    def test_success_commits_a_release_and_a_deliberate_edit(self):
        self.commit(log("### Added\n\n" + ENTRY_B))
        release = self.commit(log(released="## [0.3.0] - 2026-11-20\n\n### Added\n\n" + ENTRY_B + "\n" + RELEASED_2))
        edited = log(released="## [0.3.0] - 2026-11-20\n\n### Added\n\n" + ENTRY_B + "\n"
                     + RELEASED_2.replace("in PostgreSQL", "in a PostgreSQL database"))
        edit = self.commit(edited, f"docs: reword 0.2.0\n\nMoves {PIN}.", pin=digest(edited))
        run = self.run_script("commits", release, edit)
        self.assertEqual(run.returncode, 0, run.stderr)

    def test_success_message_with_a_moved_pin_named(self):
        self.commit(log())
        self.write(log(released=RELEASED_2.replace("in PostgreSQL", "in Postgres")), pin="abc")
        (self.root / "msg").write_text(f"docs: reword\n\nMoves {PIN}.\n", encoding="utf-8")
        run = self.run_script("message", str(self.root / "msg"))
        self.assertEqual(run.returncode, 0, run.stderr)

    # Failure.

    def test_failure_index_the_rebased_tip_against_the_merge_base(self):
        # After a rebase the index is HEAD: only the merge base with
        # origin/main shows the entry the rebase moved.
        main = self.commit(fixture("main.md"))
        self.git("update-ref", "refs/remotes/origin/main", main)
        self.commit(merged()[1])
        run = self.run_script("index")
        self.assertEqual(run.returncode, 1, run.stdout)
        self.assertIn("under `## [0.4.18] - 2026-11-26`", run.stderr)
        self.assertIn("the merge base of HEAD and origin/main", run.stderr)

    def test_failure_commits_the_rebased_commit_against_its_parent(self):
        self.commit(fixture("main.md"))
        bad = self.commit(merged()[1])
        run = self.run_script("commits", bad)
        self.assertEqual(run.returncode, 1, run.stdout)
        self.assertIn(f"parent of {bad[:12]}", run.stderr)
        self.assertIn("line 18, under `## [0.4.18] - 2026-11-26`", run.stderr)

    def test_failure_commits_a_moved_pin_the_message_does_not_name(self):
        self.commit(log())
        edited = log(released=RELEASED_2.replace("in PostgreSQL", "in Postgres"))
        edit = self.commit(edited, "docs: reword", pin=digest(edited))
        run = self.run_script("commits", edit)
        self.assertEqual(run.returncode, 1, run.stdout)
        self.assertIn(f"does not name {PIN}", run.stderr)

    def test_failure_message_with_a_moved_pin_not_named(self):
        self.commit(log())
        self.write(log(), pin=digest(log()))
        (self.root / "msg").write_text("docs: reword\n", encoding="utf-8")
        run = self.run_script("message", str(self.root / "msg"))
        self.assertEqual(run.returncode, 1, run.stdout)

    def test_failure_index_an_edit_of_released_text(self):
        self.commit(log())
        self.write(log(released=RELEASED_2.replace("registry", "list")))
        run = self.run_script("index")
        self.assertEqual(run.returncode, 1, run.stdout)
        self.assertIn("HEAD, the commit this one is built on", run.stderr)

    def test_failure_an_unknown_commit(self):
        self.commit(log())
        run = self.run_script("commits", "f" * 40)
        self.assertEqual(run.returncode, 2, run.stdout)

    def test_failure_usage(self):
        self.assertEqual(self.run_script().returncode, 2)
        self.assertEqual(self.run_script("nonsense").returncode, 2)

    # Edge cases.

    def test_edge_message_in_the_first_commit_moves_no_pin(self):
        # A first commit has no base whose released text it could change
        # (scripts/test-hooks.sh starts its fixture with a commit that holds
        # the pin file).
        self.write(log(), pin=digest(log()))
        (self.root / "msg").write_text("Start\n", encoding="utf-8")
        run = self.run_script("message", str(self.root / "msg"))
        self.assertEqual(run.returncode, 0, run.stderr)

    def test_edge_index_in_the_first_commit(self):
        self.write(log())
        run = self.run_script("index")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertIn("no HEAD", run.stdout)

    def test_edge_index_without_origin_main_names_what_it_compared(self):
        self.commit(log())
        self.write(log("### Added\n\n" + ENTRY_B))
        run = self.run_script("index")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertIn("no origin/main", run.stdout)

    def test_edge_commits_a_root_commit_has_nothing_to_judge(self):
        run = self.run_script("commits", self.commit(log()))
        self.assertEqual(run.returncode, 0, run.stderr)

    def test_edge_commits_without_a_changelog(self):
        (self.root / "a.txt").write_text("a\n", encoding="utf-8")
        self.git("add", "a.txt")
        self.git("commit", "-q", "-m", "a")
        (self.root / "a.txt").write_text("b\n", encoding="utf-8")
        self.git("commit", "-q", "-am", "b")
        run = self.run_script("commits", self.git("rev-parse", "HEAD"))
        self.assertEqual(run.returncode, 0, run.stderr)

    def test_edge_commits_a_merge_is_judged_against_each_parent(self):
        # A merge of main into a branch that keeps the branch's changelog
        # drops main's release: the first parent, the branch, sees nothing;
        # only the second, main, does.
        self.commit(log())
        self.git("checkout", "-q", "-b", "side")
        self.commit(log("### Added\n\n" + ENTRY_B))
        self.git("checkout", "-q", "main")
        main = self.commit(log(released="## [0.3.0] - 2026-11-20\n\n### Fixed\n\n" + ENTRY_C + "\n" + RELEASED_2))
        self.git("checkout", "-q", "side")
        self.git("merge", "-q", "--no-ff", "-s", "ours", "-m", "merge", "main")
        merge = self.git("rev-parse", "HEAD")
        run = self.run_script("commits", merge)
        self.assertEqual(run.returncode, 1, run.stdout)
        refusals = [line for line in run.stderr.splitlines() if line.startswith("check_changelog_released: refused: ")]
        self.assertEqual(len(refusals), 1, run.stderr)
        self.assertIn(f"{main[:12]}, parent of {merge[:12]}", refusals[0])

    def test_edge_index_reads_the_index_not_the_working_tree(self):
        self.commit(log())
        self.write(log())
        (self.root / "CHANGELOG.md").write_text(log(released=RELEASED_2.replace("registry", "list")),
                                                encoding="utf-8")
        run = self.run_script("index")
        self.assertEqual(run.returncode, 0, run.stderr)

    def test_edge_index_during_a_merge_judges_merge_head_too(self):
        self.commit(log())
        self.git("checkout", "-q", "-b", "side")
        side = self.commit(log(released="## [0.3.0] - 2026-11-20\n\n### Fixed\n\n" + ENTRY_C + "\n" + RELEASED_2))
        self.git("checkout", "-q", "main")
        self.git("update-ref", "MERGE_HEAD", side)
        self.write(log())
        run = self.run_script("index")
        self.assertEqual(run.returncode, 1, run.stdout)
        self.assertIn("MERGE_HEAD", run.stderr)


if __name__ == "__main__":
    unittest.main()
