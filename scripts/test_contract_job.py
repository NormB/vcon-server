"""Tests scripts/contract_job.py, the advisory `Contract` job, and its
workflow, .github/workflows/contract.yml.

Run with: python3 -I scripts/test_contract_job.py
"""

import contextlib
import importlib.util
import io
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
WORKFLOW = ROOT / ".github/workflows/contract.yml"
STABILITY = ROOT / "website/content/docs/stability.md"
DEVELOPMENT = ROOT / "website/content/docs/development.md"


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


job = load("contract_job")
check = load("check_required_checks")
cd = job.cd

MARKED = json.dumps({"paths": {}, "components": {"schemas": {"Problem": {"properties": {
    "type": {"type": "string", cd.OPEN_ENUM: True}}}}}})
UNMARKED = json.dumps({"paths": {}})


class FakeGit:
    """A git reader that answers from a dict: (ref, path) -> text, and the
    tag list."""

    def __init__(self, tags, files=None, tag_rc=0):
        self.tags, self.files, self.tag_rc, self.calls = tags, files or {}, tag_rc, []

    def __call__(self, args):
        self.calls.append(args)
        if args[:2] == ["tag", "--list"]:
            return self.tag_rc, "".join(f"{t}\n" for t in self.tags)
        if args[:2] == ["rev-parse", "--verify"]:
            ref = args[2].split("^")[0]
            return (0, ref + "\n") if ref in self.tags else (128, "")
        if args[0] == "show":
            ref, path = args[1].split(":", 1)
            text = self.files.get((ref, path))
            return (0, text) if isinstance(text, str) else (128, "")
        if args[0] == "ls-tree":
            ref, path = args[2], args[4].rstrip("/")
            names = self.files.get((ref, path))
            return (0, "".join(f"{path}/{n}\n" for n in names)) if isinstance(names, list) else (0, "")
        raise AssertionError(f"unexpected git call {args}")


def tree(tmp, openapi=UNMARKED, problems=("a.md",)):
    root = pathlib.Path(tmp)
    p = root / cd.OPENAPI_PATH
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(openapi, encoding="utf-8")
    d = root / cd.PROBLEMS_PATH
    d.mkdir(parents=True, exist_ok=True)
    for name in problems:
        (d / name).write_text("x\n", encoding="utf-8")
    return root


def run_job(git, **tree_args):
    out, err = io.StringIO(), io.StringIO()
    with tempfile.TemporaryDirectory() as tmp:
        root = tree(tmp, **tree_args)
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = job.run(root, git)
    return rc, out.getvalue(), err.getvalue()


def baseline(tag="v1.0.0", openapi=UNMARKED, problems=("a.md",)):
    return {(tag, cd.OPENAPI_PATH): openapi, (tag, cd.PROBLEMS_PATH): list(problems)}


def clean_env():
    """The environment without the variables a hook sets for its own
    repository (GIT_DIR, GIT_INDEX_FILE), so git reads the tree it is run in:
    the commit hook runs these tests in a copy of the index with no .git."""
    import os
    return {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}


def clone_tags():
    """The `v*` tags of the clone this file is in, read as the job reads
    them; none outside a clone."""
    r = subprocess.run(["git", "tag", "--list", "v*"], cwd=ROOT, env=clean_env(), capture_output=True,
                       text=True, timeout=60)
    return r.stdout.splitlines() if r.returncode == 0 else []


def workflow():
    import yaml
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))


def steps():
    return workflow()["jobs"]["contract"]["steps"]


class TagPasses(unittest.TestCase):
    def test_the_one_release_tag_is_the_newest(self):
        self.assertEqual(job.newest_tag(["v0.4.10"]), "v0.4.10", "one tag")

    def test_tags_compare_as_numbers(self):
        self.assertEqual(job.newest_tag(["v0.4.9", "v0.4.10", "v0.4.2"]), "v0.4.10", "10 is past 9")

    def test_a_minor_and_a_major_step_count(self):
        self.assertEqual(job.newest_tag(["v0.4.10", "v0.5.0"]), "v0.5.0", "a minor step")
        self.assertEqual(job.newest_tag(["v0.9.9", "v1.0.0"]), "v1.0.0", "a major step")

    def test_the_order_of_the_list_does_not_matter(self):
        self.assertEqual(job.newest_tag(["v1.2.3", "v0.1.0"]), job.newest_tag(["v0.1.0", "v1.2.3"]), "order")

    def test_the_tags_of_this_clone_name_a_release(self):
        tags = clone_tags()
        if tags:
            self.assertRegex(job.newest_tag(tags), r"^v\d+\.\d+\.\d+$", tags)

    def test_a_marked_baseline_has_no_note(self):
        self.assertIsNone(job.open_enum_note("v0.5.0", MARKED), "the mark is there")


class TagFails(unittest.TestCase):
    def test_no_tag_is_no_release(self):
        self.assertIsNone(job.newest_tag([]), "no tags")

    def test_a_prerelease_is_not_a_release(self):
        self.assertEqual(job.newest_tag(["v0.4.10", "v0.5.0-rc1"]), "v0.4.10", "a suffix")

    def test_a_tag_without_the_v_is_not_a_release(self):
        self.assertIsNone(job.newest_tag(["0.4.10"]), "no v")

    def test_a_tag_of_two_or_four_parts_is_not_a_release(self):
        self.assertIsNone(job.newest_tag(["v0.4", "v10.20.300.4000"]), "three parts only")

    def test_other_tags_are_passed_over(self):
        self.assertEqual(job.newest_tag(["site-2026", "v0.4.1", "vnext"]), "v0.4.1", "names that are not versions")

    def test_an_unmarked_baseline_has_the_note(self):
        note = job.open_enum_note("v0.4.10", UNMARKED)
        self.assertIn("v0.4.10's openapi.json does not mark the problem `type` as an open list", note, note)
        self.assertIn("until the newest release tag carries the mark", note, note)


class TagEdges(unittest.TestCase):
    def test_space_around_a_tag_is_left_out(self):
        self.assertEqual(job.newest_tag(["  v0.4.10  ", "v0.4.9"]), "v0.4.10", "trimmed")

    def test_a_tag_listed_twice_is_one(self):
        self.assertEqual(job.newest_tag(["v0.4.10", "v0.4.10"]), "v0.4.10", "twice")

    def test_leading_zeros_compare_as_numbers(self):
        self.assertEqual(job.newest_tag(["v0.4.010", "v0.4.9"]), "v0.4.010", "010 is ten")

    def test_an_empty_line_is_passed_over(self):
        self.assertEqual(job.newest_tag(["", "v0.1.0", ""]), "v0.1.0", "blank lines")

    def test_a_baseline_without_openapi_has_no_note(self):
        self.assertIsNone(job.open_enum_note("v0.1.0", None), "no file, no note")

    def test_the_note_names_the_tag_it_compares(self):
        self.assertIn("v0.4.7's", job.open_enum_note("v0.4.7", UNMARKED), "the tag")


class RunPasses(unittest.TestCase):
    def test_no_tag_compares_nothing_and_passes(self):
        rc, out, err = run_job(FakeGit([]))
        self.assertEqual(rc, 0, err)
        self.assertIn("contract: no release tag (vMAJOR.MINOR.PATCH); nothing to compare", out, out)

    def test_an_unchanged_tree_passes_and_names_the_tag(self):
        rc, out, err = run_job(FakeGit(["v0.9.0", "v1.0.0"], baseline()))
        self.assertEqual(rc, 0, out + err)
        self.assertIn("contract: comparing the work tree with v1.0.0, the newest release tag", out, out)
        self.assertIn("openapi: 0 change(s), 0 breaking", out, out)

    def test_an_additive_change_passes(self):
        rc, out, _ = run_job(FakeGit(["v1.0.0"], baseline()), problems=("a.md", "b.md"))
        self.assertEqual(rc, 0, out)
        self.assertIn("problems: additive problems/b: problem type added", out, out)

    def test_surfaces_the_tag_lacks_have_no_baseline_and_pass(self):
        rc, out, _ = run_job(FakeGit(["v1.0.0"], baseline()))
        for label in ("cli", "config", "mcp", "env", "formats", "audit"):
            self.assertIn(f"{label}: no baseline at v1.0.0", out, out)
        self.assertEqual(rc, 0, out)

    def test_an_unmarked_baseline_prints_the_note_before_the_changes(self):
        _, out, _ = run_job(FakeGit(["v0.4.10"], baseline("v0.4.10")))
        self.assertLess(out.index("contract: note:"), out.index("openapi:"), out)

    def test_a_marked_baseline_prints_no_note(self):
        _, out, _ = run_job(FakeGit(["v1.0.0"], baseline(openapi=MARKED)), openapi=MARKED)
        self.assertNotIn("contract: note:", out, out)


class RunFails(unittest.TestCase):
    def test_a_breaking_change_exits_1(self):
        rc, out, _ = run_job(FakeGit(["v1.0.0"], baseline(problems=("a.md", "b.md"))))
        self.assertEqual(rc, 1, out)
        self.assertIn("problems: breaking problems/b: problem type removed", out, out)

    def test_a_tag_list_git_refuses_is_an_error(self):
        rc, _, err = run_job(FakeGit(["v1.0.0"], tag_rc=128))
        self.assertEqual(rc, 2, err)
        self.assertIn("git could not list the tags; nothing was decided", err, err)

    def test_a_baseline_that_is_not_a_snapshot_is_an_error(self):
        files = {**baseline(), ("v1.0.0", cd.ENV_PATH): json.dumps({"tools": {}})}
        with tempfile.TemporaryDirectory() as tmp:
            root = tree(tmp)
            (root / "contract").mkdir()
            (root / cd.ENV_PATH).write_text(json.dumps({"variables": {}}), encoding="utf-8")
            err = io.StringIO()
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
                rc = job.run(root, FakeGit(["v1.0.0"], files))
        self.assertEqual(rc, 2, err.getvalue())

    def test_the_newest_tag_is_compared_not_an_older_one(self):
        git = FakeGit(["v0.9.0", "v1.0.0"], {**baseline("v1.0.0"), **baseline("v0.9.0", problems=("z.md",))})
        rc, out, _ = run_job(git)
        self.assertEqual(rc, 0, out)
        self.assertNotIn("problems/z", out, out)

    def test_a_removed_problem_type_with_the_note_still_exits_1(self):
        rc, out, _ = run_job(FakeGit(["v0.4.10"], baseline("v0.4.10", problems=("a.md", "gone.md"))))
        self.assertEqual(rc, 1, out)
        self.assertIn("contract: note:", out, out)

    def test_main_refuses_arguments(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            rc = job.main(["--against", "v1"])
        self.assertEqual((rc, err.getvalue()), (2, "usage: contract_job.py\n"), "it takes none")


class RunEdges(unittest.TestCase):
    def test_the_job_asks_git_only(self):
        git = FakeGit(["v1.0.0"], baseline())
        run_job(git)
        self.assertEqual({c[0] for c in git.calls}, {"tag", "rev-parse", "show", "ls-tree"}, git.calls)

    def test_only_prereleases_compare_nothing(self):
        rc, out, _ = run_job(FakeGit(["v1.0.0-rc1"]))
        self.assertEqual(rc, 0, out)
        self.assertIn("no release tag", out, out)

    def test_the_tag_list_asks_for_v_tags(self):
        git = FakeGit([])
        run_job(git)
        self.assertEqual(git.calls[0], ["tag", "--list", "v*"], "one call")

    def test_the_note_counts_on_the_tag_not_the_work_tree(self):
        _, out, _ = run_job(FakeGit(["v0.4.10"], baseline("v0.4.10")), openapi=MARKED)
        self.assertIn("contract: note:", out, "the tree's mark does not count while the tag lacks it")

    def test_the_real_tree_passes_against_its_newest_tag(self):
        if not clone_tags():
            self.skipTest("no clone with tags here (the commit hook's copy of the index, the hook fixture)")
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            rc = job.run(ROOT, cd.git_runner(ROOT, clean_env()))
        self.assertIn(rc, (0, 1), out.getvalue())
        self.assertIn("contract: comparing the work tree with v", out.getvalue(), out.getvalue())


class Workflow(unittest.TestCase):
    def test_the_job_is_named_contract_and_runs_on_every_pull_request(self):
        w = workflow()
        self.assertEqual(w["jobs"]["contract"]["name"], "Contract", "the check's name")
        triggers = w[True]
        self.assertEqual(list(triggers), ["pull_request"], triggers)
        self.assertIsNone(triggers["pull_request"], "no path filter")

    def test_it_runs_on_a_github_hosted_runner(self):
        self.assertEqual(workflow()["jobs"]["contract"]["runs-on"], "ubuntu-24.04", "not self-hosted")

    def test_every_action_is_pinned_by_sha(self):
        uses = [s["uses"] for s in steps() if "uses" in s]
        self.assertTrue(uses, "the checkout")
        for u in uses:
            self.assertRegex(u, r"^[^@]+@[0-9a-f]{40}$", u)

    def test_the_checkout_keeps_no_credential_and_fetches_the_tags(self):
        checkout = steps()[0]["with"]
        self.assertIs(checkout["persist-credentials"], False, checkout)
        self.assertEqual(checkout["fetch-depth"], 0, "the whole history")
        self.assertIs(checkout["fetch-tags"], True, "the tags")

    def test_it_runs_the_job_script_and_calls_no_github_api(self):
        runs = [s["run"] for s in steps() if "run" in s]
        self.assertEqual(runs, ["python3 -I scripts/contract_job.py"], runs)
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertNotIn("GH_TOKEN", text, "no token for the API")
        self.assertNotRegex(text, r"\bgh (api|run|pr)\b", "no gh call")

    def test_it_may_only_read_and_is_bounded(self):
        w = workflow()
        self.assertEqual(w["permissions"], {"contents": "read"}, w["permissions"])
        self.assertLessEqual(w["jobs"]["contract"]["timeout-minutes"], 15, "bounded")

    def test_the_script_calls_no_github_api(self):
        text = (HERE / "contract_job.py").read_text(encoding="utf-8")
        self.assertNotRegex(text, r"\b(urllib|http\.client|requests)\b|\"gh\"", "git only")


class Advisory(unittest.TestCase):
    def test_contract_is_the_one_advisory_check(self):
        self.assertEqual(check.ADVISORY, frozenset({"Contract"}), check.ADVISORY)

    def test_this_repository_s_contract_job_runs_on_every_pr(self):
        real = check.read_workflows(ROOT)
        names = {n for file, _, ns in check.every_pr_jobs(real) if file == "contract.yml" for n in (ns or [])}
        self.assertEqual(names, {"Contract"}, names)

    def test_without_advisory_the_contract_job_is_refused(self):
        found = check.problems(check.read_workflows(ROOT), ["CI success", "DCO"], frozenset())
        self.assertIn("contract.yml: 'Contract' runs on every pull request, but no required check needs it", found)

    def test_a_ruleset_that_requires_contract_while_it_is_advisory_is_refused(self):
        found = check.problems(check.read_workflows(ROOT), ["Contract"], check.ADVISORY)
        self.assertIn("the ruleset requires 'Contract', which ADVISORY in scripts/check_required_checks.py still "
                      "lists: remove it from ADVISORY", found)


class Docs(unittest.TestCase):
    def test_the_stability_page_names_the_job_and_the_note(self):
        text = " ".join(STABILITY.read_text(encoding="utf-8").split())
        self.assertIn("`Contract`", text, "the job")
        self.assertIn("python3 -I scripts/contract_job.py", text, "how to run it")
        self.assertIn("until the newest release tag carries the mark", text, "the note")

    def test_the_development_page_lists_the_job_under_the_gates(self):
        text = DEVELOPMENT.read_text(encoding="utf-8")
        self.assertIn("contract.yml", text, "the workflow")
        self.assertIn("scripts/contract_job.py", text, "the script")

    def test_the_note_on_the_page_is_the_one_the_script_prints(self):
        page = " ".join(STABILITY.read_text(encoding="utf-8").split())
        note = job.open_enum_note("v0.4.10", UNMARKED)
        tail = note.split("Each problem type", 1)[1]
        self.assertIn("Each problem type" + tail, page, "the page quotes the script")

    def test_the_note_holds_no_phrase_a_release_refuses_on_a_page(self):
        # The stability page quotes the note, and site_status.rs refuses each
        # phrase of .config/unreleased-phrases.txt on any page once the
        # changelog's [Unreleased] section is empty, as it is in every
        # release commit. On 2026-11-19 the note said "until the next
        # release", so the 0.4.11 release commit could not pass both tests.
        lines = (ROOT / ".config/unreleased-phrases.txt").read_text(encoding="utf-8").splitlines()
        phrases = [line for line in lines if line and not line.startswith("#")]
        self.assertGreaterEqual(len(phrases), 7, phrases)
        note = " ".join(job.open_enum_note("v0.4.10", UNMARKED).split()).lower()
        self.assertEqual([p for p in phrases if p in note], [], note)


if __name__ == "__main__":
    unittest.main()
