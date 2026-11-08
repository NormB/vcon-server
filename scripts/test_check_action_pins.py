"""Tests scripts/check_action_pins.py: every `uses:` in the workflows and
the composite actions is a local path in the repository, or an action or
reusable workflow pinned by its full 40-hex commit with a `# v<version>`
comment.

The classes group the cases as the pull request counts them: Success
(accepted), Failure (refused) and Edge (where the reading of the YAML
decides).

Run with: python3 -I scripts/test_check_action_pins.py
"""

import importlib.util
import pathlib
import re
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent
spec = importlib.util.spec_from_file_location("check_action_pins", HERE / "check_action_pins.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

SHA = "3d3c42e5aac5ba805825da76410c181273ba90b1"
PATH = ".github/workflows/x.yml"


def tree(files):
    """A temporary repository holding `files` ({relative path: text})."""
    tmp = tempfile.TemporaryDirectory()
    root = pathlib.Path(tmp.name)
    for relative, text in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="")
    return tmp, root


class Repo(unittest.TestCase):
    """A repository with one composite action and one reusable workflow,
    which local paths in the cases may name."""

    def setUp(self):
        self.tmp, self.root = tree(
            {
                ".github/actions/fuzz-setup/action.yml": "runs:\n  using: composite\n  steps: []\n",
                ".github/actions/empty/README": "not an action\n",
                ".github/workflows/build.yml": "on: workflow_call\n",
            }
        )

    def tearDown(self):
        self.tmp.cleanup()

    def found(self, text, path=PATH):
        return check.problems(path, text, self.root)

    def refused(self, text, reason):
        """`text` (one uses: line, at line 1) is refused for `reason`."""
        problems = self.found(text)
        self.assertEqual(len(problems), 1, problems)
        self.assertTrue(problems[0].startswith(f"{PATH}:1: "), problems)
        self.assertIn(reason, problems[0])


class Success(Repo):
    def test_a_step_pinned_by_commit_with_a_version_comment_passes(self):
        self.assertEqual(self.found(f"      - uses: actions/checkout@{SHA} # v7\n"), [])

    def test_a_job_level_reusable_workflow_pinned_by_commit_passes(self):
        text = f"  call:\n    uses: octo-org/ci/.github/workflows/build.yml@{SHA} # v1.0.0\n"
        self.assertEqual(self.found(text), [])

    def test_an_action_in_a_subdirectory_of_its_repository_passes(self):
        self.assertEqual(self.found(f"        uses: github/codeql-action/init@{SHA} # v4.39.1\n"), [])

    def test_an_owner_and_repository_with_hyphens_digits_and_dots_pass(self):
        text = f"      - uses: rust-lang/crates-io-auth-action@{SHA} # v1.0.6\n      - uses: octo9/setup.js_x@{SHA} # v2\n"
        self.assertEqual(self.found(text), [])

    def test_a_local_composite_action_needs_no_commit_or_comment(self):
        self.assertEqual(self.found("      - uses: ./.github/actions/fuzz-setup\n"), [])

    def test_a_local_reusable_workflow_passes(self):
        self.assertEqual(self.found("    uses: ./.github/workflows/build.yml\n"), [])

    def test_a_double_quoted_value_with_the_comment_after_the_quote_passes(self):
        self.assertEqual(self.found(f'      - uses: "actions/checkout@{SHA}" # v7\n'), [])

    def test_a_single_quoted_value_passes(self):
        self.assertEqual(self.found(f"      - uses: 'actions/checkout@{SHA}' # v7\n"), [])

    def test_an_anchored_value_is_read_after_its_anchor(self):
        self.assertEqual(self.found(f"      - uses: &checkout actions/checkout@{SHA} # v7\n"), [])

    def test_a_prerelease_version_comment_passes(self):
        self.assertEqual(self.found(f"      - uses: a/b@{SHA} # v5.0.0-beta.1\n"), [])

    def test_a_version_comment_may_carry_more_text_after_the_version(self):
        self.assertEqual(self.found(f"      - uses: a/b@{SHA} # v2.5.0 pinned for the scorecard\n"), [])

    def test_extra_spaces_around_the_value_and_comment_pass(self):
        self.assertEqual(self.found(f"      - uses:    a/b@{SHA}    #    v1   \n"), [])

    def test_a_space_before_the_colon_still_names_the_key(self):
        self.assertEqual(self.found(f"      - uses : a/b@{SHA} # v1\n"), [])

    def test_a_pinned_composite_action_in_the_tree_passes(self):
        tmp, root = tree(
            {
                ".github/workflows/ci.yml": "      - uses: ./.github/actions/setup\n",
                ".github/actions/setup/action.yml": f"    - uses: a/b@{SHA} # v3\n",
            }
        )
        with tmp:
            self.assertEqual(check.check(root), [])

    def test_this_repository_passes(self):
        self.assertEqual(check.check(REPO), [])

    def test_ci_runs_the_tests_and_the_check_in_the_scan_job(self):
        ci = (REPO / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        scan = ci.split("\n  scan:\n", 1)[1].split("\n\n  ", 1)[0]
        self.assertIn("python3 -I scripts/test_check_action_pins.py", scan)
        self.assertIn("python3 -I scripts/check_action_pins.py", scan)

    def test_the_commit_hook_runs_the_tests_and_the_check(self):
        hook = (REPO / ".githooks/pre-commit").read_text(encoding="utf-8")
        self.assertIn("python3 -I scripts/test_check_action_pins.py\n", hook)
        self.assertIn("python3 -I scripts/check_action_pins.py\n", hook)

    def test_the_development_page_says_how_to_add_an_action(self):
        page = (REPO / "website/content/docs/development.md").read_text(encoding="utf-8")
        section = next(s for s in page.split("\n## ") if s.startswith("Adding a GitHub Action\n"))
        self.assertIn("`scripts/check_action_pins.py`", section)
        self.assertIn("owner/repo@<40-hex commit> # v<version>", section)
        self.assertIn("./.github/actions/", section)


class Failure(Repo):
    def test_a_tag_is_refused(self):
        self.refused("      - uses: actions/checkout@v4\n", "@v4 is not a commit")

    def test_a_branch_is_refused(self):
        self.refused("      - uses: actions/checkout@main\n", "@main is not a commit")

    def test_a_short_commit_is_refused(self):
        self.refused("      - uses: actions/checkout@3d3c42e # v7\n", "@3d3c42e is not a commit")

    def test_a_commit_in_upper_case_is_refused(self):
        self.refused(f"      - uses: actions/checkout@{SHA.upper()} # v7\n", "is not a commit")

    def test_an_action_without_a_ref_is_refused(self):
        self.refused("      - uses: actions/checkout\n", "no @<commit>")

    def test_a_reusable_workflow_by_tag_is_refused(self):
        self.refused("    uses: octo-org/ci/.github/workflows/build.yml@v1\n", "@v1 is not a commit")

    def test_a_commit_without_a_version_comment_is_refused(self):
        self.refused(f"      - uses: actions/checkout@{SHA}\n", "no # v<version> comment")

    def test_a_comment_that_is_not_a_version_is_refused(self):
        self.refused(f"      - uses: actions/checkout@{SHA} # latest\n", "no # v<version> comment")

    def test_a_version_comment_without_the_v_is_refused(self):
        self.refused(f"      - uses: actions/checkout@{SHA} # 4.1.2\n", "no # v<version> comment")

    def test_an_empty_comment_is_refused(self):
        self.refused(f"      - uses: actions/checkout@{SHA} #\n", "no # v<version> comment")

    def test_a_docker_image_by_tag_is_refused(self):
        self.refused("      - uses: docker://alpine:3.20\n", "a docker:// image is not pinned by commit")

    def test_a_docker_image_by_digest_is_refused(self):
        self.refused("      - uses: docker://alpine@sha256:" + "a" * 64 + "\n", "a docker:// image is not pinned by commit")

    def test_a_local_path_with_a_ref_is_refused(self):
        self.refused(f"      - uses: ./.github/actions/fuzz-setup@{SHA}\n", "a local path takes no @ref")

    def test_a_local_path_not_in_the_repository_is_refused(self):
        self.refused("      - uses: ./.github/actions/gone\n", "./.github/actions/gone is not an action")

    def test_a_parent_relative_path_is_refused(self):
        self.refused("      - uses: ../shared/action\n", "not ./<path> or owner/repo")

    def test_a_name_without_an_owner_is_refused(self):
        self.refused(f"      - uses: checkout@{SHA} # v7\n", "not ./<path> or owner/repo")

    def test_an_alias_is_refused(self):
        self.refused("      - uses: *checkout\n", "an alias")

    def test_a_folded_value_on_the_next_lines_is_refused(self):
        self.refused(f"      - uses: >-\n          a/b@{SHA}\n", "not on the uses: line")

    def test_a_value_on_the_next_line_is_refused(self):
        self.refused(f"      - uses:\n          a/b@{SHA} # v1\n", "not on the uses: line")

    def test_a_flow_mapping_is_refused(self):
        self.refused(f"      - {{ name: x, uses: a/b@{SHA} }} # v1\n", "flow mapping")

    def test_a_comment_inside_the_quotes_is_part_of_the_value(self):
        self.refused(f'      - uses: "a/b@{SHA} # v1"\n', "is not a commit")

    def test_a_hash_without_a_space_is_part_of_the_value(self):
        self.refused(f"      - uses: a/b@{SHA}#v1\n", "is not a commit")

    def test_an_unclosed_quote_is_refused(self):
        self.refused(f'      - uses: "a/b@{SHA} # v1\n', "no closing quote")

    def test_text_after_the_closing_quote_is_refused(self):
        self.refused(f'      - uses: "a/b@{SHA}" v1\n', "after the closing quote")

    def test_a_yaml_tag_on_the_value_is_refused(self):
        self.refused(f"      - uses: !!str a/b@{SHA} # v1\n", "not ./<path> or owner/repo")

    def test_an_unpinned_composite_action_in_the_tree_is_found(self):
        tmp, root = tree(
            {
                ".github/workflows/ci.yml": "      - uses: ./.github/actions/setup\n",
                ".github/actions/setup/action.yml": "    - uses: a/b@v3\n",
            }
        )
        with tmp:
            self.assertEqual(check.check(root), [".github/actions/setup/action.yml:1: @v3 is not a commit: pin the full 40-hex commit id, with # v<version>"])

    def test_a_nested_composite_action_is_read(self):
        tmp, root = tree(
            {
                ".github/workflows/ci.yml": f"      - uses: a/b@{SHA} # v1\n",
                ".github/actions/group/inner/action.yaml": "    - uses: a/b@main\n",
            }
        )
        with tmp:
            self.assertEqual(len(check.check(root)), 1)
            self.assertTrue(check.check(root)[0].startswith(".github/actions/group/inner/action.yaml:1: "))

    def test_a_workflow_named_yaml_is_read(self):
        tmp, root = tree({".github/workflows/release.yaml": "      - uses: a/b@v1\n"})
        with tmp:
            self.assertEqual(len(check.check(root)), 1)


class Edge(Repo):
    def test_a_commented_out_step_is_not_read(self):
        self.assertEqual(self.found("      # - uses: actions/checkout@v4\n"), [])

    def test_a_commented_out_flow_mapping_is_not_read(self):
        self.assertEqual(self.found("      # - { uses: actions/checkout@v4 }\n"), [])

    def test_prose_naming_uses_in_a_comment_is_not_read(self):
        self.assertEqual(self.found("    # because a job that `uses:` a workflow cannot carry timeout-minutes.\n"), [])

    def test_uses_inside_a_literal_run_block_is_not_read(self):
        text = "      - run: |\n          echo 'uses: actions/checkout@v4'\n          uses: a/b@main\n"
        self.assertEqual(self.found(text), [])

    def test_uses_inside_a_folded_block_with_indicators_is_not_read(self):
        text = "      - run: >-\n          uses: a/b@main\n      - run: |2+ # keep\n          uses: a/b@main\n"
        self.assertEqual(self.found(text), [])

    def test_a_block_goes_on_across_blank_lines(self):
        text = "      - run: |\n          true\n\n          uses: a/b@main\n"
        self.assertEqual(self.found(text), [])

    def test_a_uses_after_a_block_ends_is_read_with_its_line_number(self):
        text = "      - run: |\n          uses: a/b@main\n\n      - uses: a/b@v1\n"
        self.assertEqual(self.found(text), [f"{PATH}:4: @v1 is not a commit: pin the full 40-hex commit id, with # v<version>"])

    def test_a_sibling_key_at_the_block_parents_column_ends_the_block(self):
        text = "      - run: |\n          true\n        uses: a/b@v1\n"
        self.assertEqual(len(self.found(text)), 1)

    def test_a_key_that_only_ends_in_uses_is_not_read(self):
        self.assertEqual(self.found("      reuses: a/b@main\n      uses_count: 3\n"), [])

    def test_uses_in_the_text_of_another_value_is_not_read(self):
        self.assertEqual(self.found("      - name: the step that uses: a/b@main\n"), [])

    def test_a_commit_one_digit_short_is_refused(self):
        self.refused(f"      - uses: a/b@{SHA[:-1]} # v1\n", "is not a commit")

    def test_a_commit_one_digit_long_is_refused(self):
        self.refused(f"      - uses: a/b@{SHA}0 # v1\n", "is not a commit")

    def test_a_sha256_object_id_is_refused(self):
        self.refused("      - uses: a/b@" + "0123456789abcdef" * 4 + " # v1\n", "is not a commit")

    def test_an_empty_ref_is_refused(self):
        self.refused("      - uses: a/b@ # v1\n", "@ is not a commit")

    def test_every_problem_in_a_file_is_reported(self):
        text = "      - uses: a/b@v1\n      - uses: c/d@main\n      - uses: e/f\n"
        self.assertEqual([p.split(":")[1] for p in self.found(text)], ["1", "2", "3"])

    def test_crlf_line_ends_are_not_part_of_the_value(self):
        self.assertEqual(self.found("      - uses: ./.github/actions/fuzz-setup\r\n"), [])

    def test_a_directory_without_an_action_file_is_not_an_action(self):
        self.refused("      - uses: ./.github/actions/empty\n", "is not an action")

    def test_a_local_path_that_leaves_the_repository_is_refused(self):
        # The action outside exists, so only the repository's bounds refuse it.
        tmp, top = tree({"repo/.github/workflows/ci.yml": "", "outside/action.yml": "runs: {}\n"})
        with tmp:
            problems = check.problems(PATH, "      - uses: ./../outside\n", top / "repo")
            self.assertEqual(problems, [f"{PATH}:1: ./../outside is not an action or workflow file in the repository"])

    def test_an_anchor_with_its_value_on_the_next_line_is_refused(self):
        self.refused(f"      - uses: &pin\n          a/b@{SHA} # v1\n", "not on the uses: line")

    def test_a_tree_with_no_uses_fails(self):
        tmp, root = tree({".github/workflows/ci.yml": "on: push\n"})
        with tmp:
            self.assertEqual(check.check(root), ["no uses: found in .github/workflows or .github/actions"])

    def test_the_check_reads_every_uses_line_of_this_repository(self):
        # A reading that loses lines would pass a tree it never looked at.
        key = re.compile(r"^\s*(?:-\s+)?uses\s*:\s")
        lines = sum(1 for path in check.files(REPO) for line in path.read_text(encoding="utf-8").splitlines() if key.match(line))
        self.assertGreater(lines, 0)
        self.assertEqual(sum(len(check.scan(path.read_text(encoding="utf-8"))) for path in check.files(REPO)), lines)

    def test_main_names_each_problem_and_exits_one(self):
        tmp, root = tree({".github/workflows/ci.yml": "      - uses: a/b@v1\n"})
        with tmp:
            self.assertEqual(check.report(root), (1, ["check_action_pins: .github/workflows/ci.yml:1: @v1 is not a commit: pin the full 40-hex commit id, with # v<version>"]))

    def test_main_counts_what_it_read_when_all_pass(self):
        tmp, root = tree({".github/workflows/ci.yml": f"      - uses: a/b@{SHA} # v1\n      - uses: c/d@{SHA} # v2\n"})
        with tmp:
            self.assertEqual(check.report(root), (0, ["check_action_pins: ok (2 uses in 1 files)"]))


if __name__ == "__main__":
    unittest.main()
