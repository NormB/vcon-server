"""Tests scripts/codeql_pin.py: the CodeQL workflow analyzes with the one
pinned CodeQL bundle, fetched through the pinned-download rule, on the whole
tree; and a scheduled run fails while a newer bundle is released.

Run with: python3 -I scripts/test_codeql_pin.py
"""

import copy
import importlib.util
import io
import pathlib
import sys
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("codeql_pin", HERE / "codeql_pin.py")
pin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pin)

SHA = {"actions": "a" * 64, "javascript": "b" * 64, "rust": "c" * 64}
URL = (
    "https://github.com/github/codeql-action/releases/download/"
    "codeql-bundle-v${{ env.CODEQL_VERSION }}/codeql-bundle-${{ matrix.bundle }}-linux64.tar.zst"
)


def workflow(version="2.27.5"):
    """codeql.yml as it is shaped, parsed."""
    return {
        "env": {"CODEQL_VERSION": version, "CODEQL_ACTION_DIFF_INFORMED_QUERIES": "false"},
        "jobs": {
            "analyze": {
                "strategy": {
                    "matrix": {
                        "include": [
                            {"language": "actions", "bundle": "actions", "bundle_sha256": SHA["actions"]},
                            {"language": "javascript-typescript", "bundle": "javascript", "bundle_sha256": SHA["javascript"]},
                            {"language": "rust", "bundle": "rust", "bundle_sha256": SHA["rust"]},
                        ]
                    }
                },
                "steps": [
                    {"uses": "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"},
                    {
                        "name": "The pinned CodeQL bundle",
                        "id": "bundle",
                        "uses": "./.github/actions/pinned-download",
                        "with": {
                            "name": "codeql-bundle-${{ matrix.bundle }}",
                            "version": "${{ env.CODEQL_VERSION }}",
                            "sha256": "${{ matrix.bundle_sha256 }}",
                            "url": URL,
                            "file": "codeql-bundle-${{ matrix.bundle }}-linux64.tar.zst",
                        },
                    },
                    {
                        "name": "Initialize CodeQL",
                        "uses": "github/codeql-action/init@9b3254085494b55a0f7c4c2b3d2d52cc712ce370",
                        "with": {
                            "languages": "${{ matrix.language }}",
                            "build-mode": "none",
                            "tools": "${{ steps.bundle.outputs.path }}",
                        },
                    },
                    {"uses": "github/codeql-action/analyze@9b3254085494b55a0f7c4c2b3d2d52cc712ce370"},
                ],
            }
        },
    }


def steps(w):
    return w["jobs"]["analyze"]["steps"]


def release(version, prerelease=False, draft=False, digests=True):
    assets = []
    for bundle in ("actions", "javascript", "rust", "linux64"):
        name = f"codeql-bundle-{bundle}-linux64.tar.zst" if bundle != "linux64" else "codeql-bundle-linux64.tar.zst"
        asset = {"name": name}
        if digests:
            asset["digest"] = "sha256:" + {"actions": "1", "javascript": "2", "rust": "3", "linux64": "4"}[bundle] * 64
        assets.append(asset)
    return {"tag_name": f"codeql-bundle-v{version}", "prerelease": prerelease, "draft": draft, "assets": assets}


def run(argv, w=None, releases=None):
    out, err = io.StringIO(), io.StringIO()
    asked = []

    def api(path):
        asked.append(path)
        if isinstance(releases, Exception):
            raise releases
        return releases if releases is not None else []

    code = pin.main(argv, read_workflow=lambda: w if w is not None else workflow(), api=api, out=out, err=err)
    return code, out.getvalue(), err.getvalue(), asked


class Check(unittest.TestCase):
    # Success.
    def test_the_workflow_as_shaped_passes(self):
        self.assertEqual(pin.problems(workflow()), [], "the shape the rule asks for")

    def test_the_pinned_version_is_read_from_the_workflow_env(self):
        self.assertEqual(pin.pinned_version(workflow("2.27.5")), "2.27.5", "env.CODEQL_VERSION")

    def test_check_exits_0_and_names_the_pin(self):
        code, out, err, _ = run(["check"])
        self.assertEqual(code, 0, err)
        self.assertIn("CodeQL 2.27.5", out, "names the pinned version")

    def test_check_reads_no_api(self):
        _, _, _, asked = run(["check"])
        self.assertEqual(asked, [], "check is offline, so the commit hook can run it")

    def test_a_diff_informed_false_on_the_job_passes(self):
        w = workflow()
        del w["env"]["CODEQL_ACTION_DIFF_INFORMED_QUERIES"]
        w["jobs"]["analyze"]["env"] = {"CODEQL_ACTION_DIFF_INFORMED_QUERIES": "false"}
        self.assertEqual(pin.problems(w), [], "job-level env counts as well as workflow-level")

    def test_the_real_codeql_workflow_passes(self):
        w = pin.load_workflow(HERE.parent / ".github" / "workflows" / "codeql.yml")
        self.assertEqual(pin.problems(w), [], "codeql.yml keeps the pin")

    def test_the_real_workflow_pins_every_analyzed_language(self):
        w = pin.load_workflow(HERE.parent / ".github" / "workflows" / "codeql.yml")
        languages = sorted(e["language"] for e in w["jobs"]["analyze"]["strategy"]["matrix"]["include"])
        self.assertEqual(languages, ["actions", "javascript-typescript", "rust"], "the three analyzed languages")

    # Failure.
    def test_an_init_step_without_tools_is_refused(self):
        w = workflow()
        del steps(w)[2]["with"]["tools"]
        self.assertTrue(any("tools" in p for p in pin.problems(w)), pin.problems(w))

    def test_tools_linked_is_refused(self):
        w = workflow()
        steps(w)[2]["with"]["tools"] = "linked"
        self.assertTrue(any("tools" in p for p in pin.problems(w)), pin.problems(w))

    def test_tools_as_a_bare_url_is_refused(self):
        w = workflow()
        steps(w)[2]["with"]["tools"] = "https://github.com/github/codeql-action/releases/download/codeql-bundle-v2.27.5/codeql-bundle-linux64.tar.zst"
        self.assertTrue(any("tools" in p for p in pin.problems(w)), pin.problems(w))

    def test_no_codeql_version_is_refused(self):
        w = workflow()
        del w["env"]["CODEQL_VERSION"]
        self.assertTrue(any("CODEQL_VERSION" in p for p in pin.problems(w)), pin.problems(w))

    def test_a_version_that_is_not_three_numbers_is_refused(self):
        self.assertTrue(any("CODEQL_VERSION" in p for p in pin.problems(workflow("latest"))), "latest")

    def test_diff_informed_analysis_left_on_is_refused(self):
        w = workflow()
        del w["env"]["CODEQL_ACTION_DIFF_INFORMED_QUERIES"]
        self.assertTrue(any("DIFF_INFORMED" in p for p in pin.problems(w)), pin.problems(w))

    def test_diff_informed_true_is_refused(self):
        w = workflow()
        w["env"]["CODEQL_ACTION_DIFF_INFORMED_QUERIES"] = "true"
        self.assertTrue(any("DIFF_INFORMED" in p for p in pin.problems(w)), pin.problems(w))

    def test_a_bundle_step_that_is_not_pinned_download_is_refused(self):
        w = workflow()
        steps(w)[1]["uses"] = "actions/cache@v4"
        self.assertTrue(any("pinned-download" in p for p in pin.problems(w)), pin.problems(w))

    def test_a_bundle_url_of_another_repository_is_refused(self):
        w = workflow()
        steps(w)[1]["with"]["url"] = URL.replace("github/codeql-action", "example/codeql-action")
        self.assertTrue(any("url" in p for p in pin.problems(w)), pin.problems(w))

    def test_a_bundle_version_other_than_the_env_is_refused(self):
        w = workflow()
        steps(w)[1]["with"]["version"] = "2.27.4"
        self.assertTrue(any("version" in p for p in pin.problems(w)), pin.problems(w))

    def test_a_matrix_entry_without_its_sha256_is_refused(self):
        w = workflow()
        del w["jobs"]["analyze"]["strategy"]["matrix"]["include"][2]["bundle_sha256"]
        self.assertTrue(any("rust" in p and "sha256" in p for p in pin.problems(w)), pin.problems(w))

    def test_a_short_sha256_is_refused(self):
        w = workflow()
        w["jobs"]["analyze"]["strategy"]["matrix"]["include"][0]["bundle_sha256"] = "a" * 63
        self.assertTrue(any("sha256" in p for p in pin.problems(w)), pin.problems(w))

    def test_check_exits_1_and_names_each_problem(self):
        w = workflow()
        del steps(w)[2]["with"]["tools"]
        del w["env"]["CODEQL_ACTION_DIFF_INFORMED_QUERIES"]
        code, _, err, _ = run(["check"], w=w)
        self.assertEqual(code, 1, "refused")
        self.assertIn("tools", err, "names the missing tools input")
        self.assertIn("DIFF_INFORMED", err, "names the diff-informed setting")

    # Edge.
    def test_a_language_without_its_bundle_is_refused(self):
        w = workflow()
        w["jobs"]["analyze"]["strategy"]["matrix"]["include"][1]["bundle"] = "rust"
        self.assertTrue(any("javascript-typescript" in p for p in pin.problems(w)), pin.problems(w))

    def test_a_language_list_without_include_is_refused(self):
        w = workflow()
        w["jobs"]["analyze"]["strategy"]["matrix"] = {"language": ["actions", "rust"]}
        self.assertTrue(any("include" in p for p in pin.problems(w)), pin.problems(w))

    def test_the_bundle_fetched_after_the_init_is_refused(self):
        w = workflow()
        s = steps(w)
        s[1], s[2] = s[2], s[1]
        self.assertTrue(any("before" in p for p in pin.problems(w)), pin.problems(w))

    def test_tools_naming_another_step_is_refused(self):
        w = workflow()
        steps(w)[2]["with"]["tools"] = "${{ steps.other.outputs.path }}"
        self.assertTrue(any("tools" in p for p in pin.problems(w)), pin.problems(w))

    def test_an_uppercase_sha256_is_refused(self):
        w = workflow()
        w["jobs"]["analyze"]["strategy"]["matrix"]["include"][0]["bundle_sha256"] = "A" * 64
        self.assertTrue(any("sha256" in p for p in pin.problems(w)), pin.problems(w))

    def test_diff_informed_as_a_yaml_boolean_false_passes(self):
        w = workflow()
        w["env"]["CODEQL_ACTION_DIFF_INFORMED_QUERIES"] = False
        self.assertEqual(pin.problems(w), [], "YAML false is the string 'false' in the environment")

    def test_a_workflow_with_no_analyze_job_is_refused(self):
        self.assertTrue(pin.problems({"jobs": {}}), "no job to check is a refusal, not a pass")

    def test_an_empty_workflow_file_is_refused(self):
        self.assertTrue(pin.problems(None), "an empty file parses to None")

    def test_a_second_init_step_without_tools_is_refused(self):
        w = workflow()
        steps(w).append({"uses": "github/codeql-action/init@9b3254085494b55a0f7c4c2b3d2d52cc712ce370", "with": {}})
        self.assertTrue(any("tools" in p for p in pin.problems(w)), pin.problems(w))

    def test_an_unknown_command_exits_2(self):
        code, _, err, _ = run(["bump"])
        self.assertEqual(code, 2, err)


class Behind(unittest.TestCase):
    # Success.
    def test_the_pin_at_the_newest_release_passes(self):
        code, out, err, asked = run(["behind"], releases=[release("2.27.5"), release("2.27.4")])
        self.assertEqual(code, 0, err)
        self.assertIn("2.27.5 is the newest", out, "says so")
        self.assertEqual(len(asked), 1, "one API read")

    def test_the_newest_is_found_among_other_tags(self):
        rels = [{"tag_name": "v4.39.4", "prerelease": False, "draft": False, "assets": []}, release("2.27.5")]
        self.assertEqual(pin.newest_bundle(rels)["tag_name"], "codeql-bundle-v2.27.5", "action tags are passed over")

    def test_versions_compare_as_numbers(self):
        self.assertEqual(pin.newest_bundle([release("2.9.9"), release("2.27.0")])["tag_name"], "codeql-bundle-v2.27.0", "27 > 9")

    def test_the_releases_are_read_from_codeql_action(self):
        _, _, _, asked = run(["behind"], releases=[release("2.27.5")])
        self.assertTrue(asked[0].startswith("repos/github/codeql-action/releases"), asked)

    def test_a_pin_newer_than_every_release_passes(self):
        code, _, err, _ = run(["behind"], releases=[release("2.27.4")])
        self.assertEqual(code, 0, err)

    def test_a_newer_release_prints_the_values_to_pin(self):
        code, _, err, _ = run(["behind"], releases=[release("2.27.6"), release("2.27.5")])
        self.assertEqual(code, 1, "a newer bundle fails the scheduled run")
        self.assertIn("CODEQL_VERSION: 2.27.6", err, "the new version")
        self.assertIn("rust: " + "3" * 64, err, "each language's sha256 from the release's digests")

    # Failure.
    def test_a_newer_major_release_fails(self):
        code, _, _, _ = run(["behind"], releases=[release("3.0.0")])
        self.assertEqual(code, 1, "3.0.0 is newer than 2.27.5")

    def test_no_bundle_release_at_all_exits_2(self):
        code, _, err, _ = run(["behind"], releases=[])
        self.assertEqual(code, 2, err)

    def test_an_api_failure_exits_2(self):
        code, _, err, _ = run(["behind"], releases=pin.PinError("gh api: HTTP 502"))
        self.assertEqual(code, 2, "the release list cannot be read")
        self.assertIn("HTTP 502", err, "names the failure")

    def test_a_newer_release_missing_a_digest_still_fails_and_says_so(self):
        code, _, err, _ = run(["behind"], releases=[release("2.27.6", digests=False)])
        self.assertEqual(code, 1, "still behind")
        self.assertIn("no sha256", err, "says the digest must be read by hand")

    def test_behind_on_a_workflow_without_a_pin_exits_1(self):
        w = workflow()
        del w["env"]["CODEQL_VERSION"]
        code, _, err, _ = run(["behind"], w=w, releases=[release("2.27.5")])
        self.assertEqual(code, 1, err)

    # Edge.
    def test_a_prerelease_is_passed_over(self):
        self.assertEqual(pin.newest_bundle([release("2.28.0", prerelease=True), release("2.27.5")])["tag_name"], "codeql-bundle-v2.27.5", "prerelease")

    def test_a_draft_is_passed_over(self):
        self.assertEqual(pin.newest_bundle([release("2.28.0", draft=True), release("2.27.5")])["tag_name"], "codeql-bundle-v2.27.5", "draft")

    def test_a_malformed_bundle_tag_is_passed_over(self):
        rels = [{"tag_name": "codeql-bundle-v2.28", "prerelease": False, "draft": False, "assets": []}, release("2.27.5")]
        self.assertEqual(pin.newest_bundle(rels)["tag_name"], "codeql-bundle-v2.27.5", "two numbers")

    def test_a_dated_nightly_tag_is_passed_over(self):
        rels = [{"tag_name": "codeql-bundle-20261120", "prerelease": False, "draft": False, "assets": []}, release("2.27.5")]
        self.assertEqual(pin.newest_bundle(rels)["tag_name"], "codeql-bundle-v2.27.5", "nightly")

    def test_an_answer_that_is_not_a_list_exits_2(self):
        code, _, _, _ = run(["behind"], releases={"message": "Not Found"})
        self.assertEqual(code, 2, "an error object is not a release list")


class More(unittest.TestCase):
    # Success.
    def test_the_bundle_file_is_the_last_part_of_its_url(self):
        self.assertTrue(pin.PINNED_DOWNLOAD["url"].endswith("/" + pin.PINNED_DOWNLOAD["file"]), "one name for the asset")

    def test_an_older_release_listed_first_does_not_hide_a_newer_one(self):
        rels = [release("2.27.4"), release("2.27.6"), release("2.27.5")]
        self.assertEqual(pin.newest_bundle(rels)["tag_name"], "codeql-bundle-v2.27.6", "order of the list does not matter")

    # Edge.
    def test_a_matrix_entry_of_an_unknown_language_is_refused(self):
        w = workflow()
        w["jobs"]["analyze"]["strategy"]["matrix"]["include"].append({"language": "python", "bundle": "python", "bundle_sha256": "d" * 64})
        self.assertTrue(any("python" in p for p in pin.problems(w)), pin.problems(w))

    def test_a_two_digit_patch_is_newer_than_a_one_digit_one(self):
        code, _, _, _ = run(["behind"], w=workflow("2.27.9"), releases=[release("2.27.10")])
        self.assertEqual(code, 1, "2.27.10 > 2.27.9")


class Unchanged(unittest.TestCase):
    def test_problems_does_not_change_its_input(self):
        w = workflow()
        before = copy.deepcopy(w)
        pin.problems(w)
        self.assertEqual(w, before, "a check reads, never writes")


if __name__ == "__main__":
    unittest.main()
