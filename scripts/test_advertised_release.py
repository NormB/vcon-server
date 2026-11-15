"""Tests scripts/advertised_release.py: which release the site may name,
when a deploy is held, what a read of the live site proves, and when a
held deploy has waited too long.

Run with: python3 -I scripts/test_advertised_release.py
"""

import importlib.util
import pathlib
import sys
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("advertised_release", HERE / "advertised_release.py")
ar = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ar)


def release(version, draft=False, prerelease=False, drop=(), state="uploaded", published="2026-11-14T22:32:39Z", extra=()):
    """A release as the GitHub API gives it, with the six assets of a
    vcon-server release unless `drop` names some."""
    names = [name.replace("<v>", version) for name in ar.ASSETS if name not in drop] + list(extra)
    return {
        "tag_name": f"v{version}",
        "draft": draft,
        "prerelease": prerelease,
        "published_at": published,
        "assets": [{"name": name, "state": state} for name in names],
    }


class VersionKey(unittest.TestCase):
    def test_a_version_of_three_numbers_is_read(self):
        self.assertEqual(ar.version_key("0.4.6"), (0, 4, 6))
        self.assertEqual(ar.version_key("10.20.30"), (10, 20, 30))

    def test_versions_compare_by_number_not_by_text(self):
        self.assertLess(ar.version_key("0.4.9"), ar.version_key("0.4.10"))
        self.assertLess(ar.version_key("0.9.0"), ar.version_key("0.10.0"))

    def test_what_is_not_three_numbers_is_no_version(self):
        for text in ["", "0.4", "0.4.6.beta","v0.4.6", "0.4.6-rc.1", "0.4.6-dev.abc", "0.4.x", " 0.4.6", "0.4.6\n", "00.4.6", "0.04.6", None, 46]:
            self.assertIsNone(ar.version_key(text), repr(text))


class Complete(unittest.TestCase):
    def test_a_published_release_with_its_six_assets_is_complete(self):
        self.assertEqual(ar.incomplete(release("0.4.6")), "")

    def test_an_asset_more_than_the_six_does_not_matter(self):
        self.assertEqual(ar.incomplete(release("0.4.6", extra=["notes.txt"])), "")

    def test_a_draft_is_not_complete(self):
        self.assertEqual(ar.incomplete(release("0.4.6", draft=True)), "v0.4.6 is a draft")

    def test_a_prerelease_is_not_complete(self):
        self.assertEqual(ar.incomplete(release("0.4.6", prerelease=True)), "v0.4.6 is a prerelease")

    def test_a_release_without_one_archive_is_not_complete(self):
        self.assertEqual(
            ar.incomplete(release("0.4.6", drop=["vcon-server-<v>-aarch64-unknown-linux-gnu.tar.gz"])),
            "v0.4.6 has no asset vcon-server-0.4.6-aarch64-unknown-linux-gnu.tar.gz",
        )

    def test_each_of_the_six_assets_is_required(self):
        self.assertEqual(len(ar.ASSETS), 6)
        for name in ar.ASSETS:
            self.assertIn("has no asset " + name.replace("<v>", "0.4.6"), ar.incomplete(release("0.4.6", drop=[name])), name)

    def test_an_asset_still_being_uploaded_is_not_there_yet(self):
        self.assertEqual(
            ar.incomplete(release("0.4.6", state="starter")),
            "v0.4.6: the asset SHA256SUMS.txt is not uploaded yet (state starter)",
        )

    def test_a_release_with_no_assets_is_not_complete(self):
        self.assertIn("has no asset", ar.incomplete({"tag_name": "v0.4.6", "draft": False, "prerelease": False, "assets": []}))
        self.assertIn("has no asset", ar.incomplete({"tag_name": "v0.4.6", "draft": False, "prerelease": False}))

    def test_a_tag_that_is_not_a_version_is_not_a_release_of_one(self):
        for tag in ["0.4.6", "v0.4", "v0.4.6-rc.1", "nightly", "", None]:
            found = ar.incomplete({"tag_name": tag, "draft": False, "prerelease": False, "assets": []})
            self.assertIn("is not a version tag", found, repr(tag))

    def test_another_versions_assets_do_not_complete_a_release(self):
        other = release("0.4.5")
        other["tag_name"] = "v0.4.6"
        self.assertIn("has no asset vcon-server-0.4.6-", ar.incomplete(other))


class Newest(unittest.TestCase):
    def test_the_newest_complete_release_is_advertised(self):
        self.assertEqual(ar.newest_to_advertise([release("0.4.5"), release("0.4.6"), release("0.4.4")], "0.4.6"), "0.4.6")

    def test_the_order_of_the_list_does_not_matter(self):
        self.assertEqual(ar.newest_to_advertise([release("0.4.6"), release("0.4.10"), release("0.4.9")], "0.4.10"), "0.4.10")

    def test_a_tree_ahead_of_the_releases_advertises_the_last_release(self):
        self.assertEqual(ar.newest_to_advertise([release("0.4.5"), release("0.4.6")], "0.4.7"), "0.4.6")

    def test_a_release_newer_than_the_tree_is_not_advertised_by_it(self):
        self.assertEqual(ar.newest_to_advertise([release("0.4.5"), release("0.4.6")], "0.4.5"), "0.4.5")

    def test_a_draft_of_the_trees_version_leaves_the_release_before_it(self):
        self.assertEqual(ar.newest_to_advertise([release("0.4.6"), release("0.4.7", draft=True)], "0.4.7"), "0.4.6")

    def test_a_release_whose_assets_are_still_arriving_leaves_the_release_before_it(self):
        self.assertEqual(ar.newest_to_advertise([release("0.4.6"), release("0.4.7", state="starter")], "0.4.7"), "0.4.6")

    def test_a_release_withdrawn_as_a_prerelease_leaves_the_release_before_it(self):
        self.assertEqual(ar.newest_to_advertise([release("0.4.6"), release("0.4.7", prerelease=True)], "0.4.7"), "0.4.6")

    def test_no_release_advertises_nothing(self):
        self.assertIsNone(ar.newest_to_advertise([], "0.4.6"))

    def test_only_incomplete_releases_advertise_nothing(self):
        self.assertIsNone(ar.newest_to_advertise([release("0.4.6", draft=True), {"tag_name": "nightly"}], "0.4.6"))

    def test_a_tree_older_than_every_release_advertises_nothing(self):
        self.assertIsNone(ar.newest_to_advertise([release("0.4.6")], "0.4.5"))

    def test_a_tree_version_that_is_no_version_is_refused(self):
        for tree in ["", "0.4", "0.4.7-dev.abc", None]:
            with self.assertRaises(ar.Refused, msg=repr(tree)) as caught:
                ar.newest_to_advertise([release("0.4.6")], tree)
            self.assertIn("is not a version", str(caught.exception))

    def test_a_list_that_is_not_a_list_of_releases_is_refused(self):
        for answer in [None, {"message": "Not Found"}, "[]", [release("0.4.6"), "v0.4.5"]]:
            with self.assertRaises(ar.Refused, msg=repr(answer)):
                ar.newest_to_advertise(answer, "0.4.6")


class Decide(unittest.TestCase):
    def test_a_tree_at_the_newest_release_is_deployed(self):
        self.assertEqual(ar.decide([release("0.4.5"), release("0.4.6")], "0.4.6"), ("deploy", "0.4.6"))

    def test_a_release_commit_before_its_release_is_held(self):
        self.assertEqual(ar.decide([release("0.4.5"), release("0.4.6")], "0.4.7"), ("held", "0.4.6"))

    def test_a_release_commit_whose_release_is_a_draft_is_held(self):
        self.assertEqual(ar.decide([release("0.4.6"), release("0.4.7", draft=True)], "0.4.7"), ("held", "0.4.6"))

    def test_a_tree_two_versions_ahead_is_held_at_the_last_release(self):
        self.assertEqual(ar.decide([release("0.4.6"), release("0.4.7")], "0.4.9"), ("held", "0.4.7"))

    def test_a_repository_without_a_release_is_held_with_none(self):
        self.assertEqual(ar.decide([], "0.1.0"), ("held", None))

    def test_the_notice_of_a_held_deploy_is_one_line_with_both_versions(self):
        line = ar.held_notice("0.4.7", "0.4.6")
        self.assertEqual(
            line,
            "Site deploy held: main is at 0.4.7, which has no published release; "
            "the site keeps showing 0.4.6 until the release workflow publishes 0.4.7 and deploys the site.",
        )
        self.assertNotIn("\n", line)

    def test_the_notice_without_any_release_says_so(self):
        self.assertEqual(
            ar.held_notice("0.1.0", None),
            "Site deploy held: main is at 0.1.0, which has no published release; "
            "no release is published yet, so the site is not deployed.",
        )

    def test_an_expected_version_that_is_deployed_passes(self):
        self.assertEqual(ar.expectation("deploy", "0.4.7", "0.4.7", "0.4.7"), "")

    def test_no_expectation_is_never_a_refusal(self):
        self.assertEqual(ar.expectation("held", "0.4.7", "0.4.6", ""), "")
        self.assertEqual(ar.expectation("deploy", "0.4.7", "0.4.7", None), "")

    def test_an_expected_release_that_is_not_complete_is_a_refusal(self):
        self.assertEqual(
            ar.expectation("held", "0.4.7", "0.4.6", "0.4.7"),
            "expected to deploy the site for 0.4.7, but main is at 0.4.7 and the newest complete release is 0.4.6: "
            "the site is not deployed",
        )

    def test_a_main_that_has_moved_to_the_next_version_is_a_refusal(self):
        self.assertEqual(
            ar.expectation("held", "0.4.8", "0.4.7", "0.4.7"),
            "expected to deploy the site for 0.4.7, but main is at 0.4.8 and the newest complete release is 0.4.7: "
            "the site is not deployed",
        )

    def test_a_deploy_of_another_version_than_expected_is_a_refusal(self):
        self.assertIn("expected to deploy the site for 0.4.7, but main is at 0.4.6", ar.expectation("deploy", "0.4.6", "0.4.6", "0.4.7"))


class Outputs(unittest.TestCase):
    def test_a_deploy_gives_its_three_outputs(self):
        self.assertEqual(ar.outputs("deploy", "0.4.7", "0.4.7"), "verdict=deploy\ntree=0.4.7\nadvertised=0.4.7\n")

    def test_a_held_run_with_no_release_gives_an_empty_advertised(self):
        self.assertEqual(ar.outputs("held", "0.1.0", None), "verdict=held\ntree=0.1.0\nadvertised=\n")

    def test_the_outputs_hold_no_workflow_command(self):
        text = ar.outputs("held", "0.4.7", "0.4.6")
        self.assertNotIn("::", text)
        self.assertTrue(all(line.count("=") == 1 for line in text.splitlines()), text)


class Live(unittest.TestCase):
    def test_the_file_with_the_version_is_verified(self):
        self.assertEqual(ar.live_verdict(200, {}, "0.4.6\n", "0.4.6"), ("verified", "the site names 0.4.6"))

    def test_the_file_without_a_last_newline_is_verified(self):
        self.assertEqual(ar.live_verdict(200, {"Content-Type": "text/plain"}, "0.4.6", "0.4.6")[0], "verified")

    def test_another_version_differs(self):
        self.assertEqual(ar.live_verdict(200, {}, "0.4.5\n", "0.4.6"), ("differs", "the site names 0.4.5, not 0.4.6"))

    def test_a_challenge_is_not_verified_and_never_a_pass(self):
        verdict, why = ar.live_verdict(403, {"cf-mitigated": "challenge"}, "<html>Just a moment...</html>", "0.4.6")
        self.assertEqual(verdict, "not verified")
        self.assertIn("challenge", why)

    def test_a_challenge_header_in_any_case_is_seen(self):
        self.assertEqual(ar.live_verdict(403, {"CF-Mitigated": "Challenge"}, "", "0.4.6")[0], "not verified")

    def test_a_challenge_page_answered_with_200_is_not_verified(self):
        verdict, why = ar.live_verdict(200, {"cf-mitigated": "challenge"}, "0.4.6\n", "0.4.6")
        self.assertEqual(verdict, "not verified")
        self.assertIn("challenge", why)

    def test_a_page_of_html_is_not_a_version(self):
        verdict, why = ar.live_verdict(200, {}, "<!doctype html><title>Just a moment</title> 0.4.6", "0.4.6")
        self.assertEqual(verdict, "not verified")
        self.assertIn("not a version", why)

    def test_a_missing_file_is_not_verified(self):
        self.assertEqual(ar.live_verdict(404, {}, "Not Found", "0.4.6"), ("not verified", "the site answered 404"))

    def test_a_server_error_and_no_answer_are_not_verified(self):
        self.assertEqual(ar.live_verdict(503, {}, "", "0.4.6")[0], "not verified")
        self.assertEqual(ar.live_verdict(None, {}, "timed out", "0.4.6"), ("not verified", "no answer: timed out"))

    def test_an_empty_file_is_not_verified(self):
        self.assertEqual(ar.live_verdict(200, {}, "", "0.4.6")[0], "not verified")


class Daily(unittest.TestCase):
    NOW = 1_800_000_000
    HOUR = 3600

    def test_a_site_that_names_the_newest_release_passes(self):
        self.assertEqual(ar.daily_problems("deploy", "0.4.6", "0.4.6", None, ("verified", ""), self.NOW, None), [])

    def test_a_deploy_held_for_less_than_the_bound_passes(self):
        since = self.NOW - 2 * self.HOUR
        self.assertEqual(ar.daily_problems("held", "0.4.7", "0.4.6", since, ("verified", ""), self.NOW, None), [])

    def test_a_deploy_held_for_exactly_the_bound_passes(self):
        since = self.NOW - ar.HELD_BOUND_HOURS * self.HOUR
        self.assertEqual(ar.daily_problems("held", "0.4.7", "0.4.6", since, ("verified", ""), self.NOW, None), [])

    def test_a_deploy_held_longer_than_the_bound_fails_with_both_versions_and_the_remedy(self):
        since = self.NOW - 7 * self.HOUR
        found = ar.daily_problems("held", "0.4.7", "0.4.6", since, ("verified", ""), self.NOW, None)
        self.assertEqual(len(found), 1, found)
        self.assertEqual(
            found[0],
            "main has been at 0.4.7 for 7 hours and release 0.4.7 is not published, so the site is held at 0.4.6 "
            "and no change to the pages is deployed (the bound is 6 hours). Remedy: finish the release (push the tag "
            "v0.4.7, or re-run the failed release workflow), or set the version in Cargo.toml back to 0.4.6.",
        )

    def test_the_bound_is_six_hours(self):
        self.assertEqual(ar.HELD_BOUND_HOURS, 6)

    def test_a_held_deploy_whose_start_is_not_known_fails(self):
        found = ar.daily_problems("held", "0.4.7", "0.4.6", None, ("verified", ""), self.NOW, None)
        self.assertEqual(len(found), 1, found)
        self.assertIn("since when is not known", found[0])

    def test_a_site_that_names_an_older_release_fails(self):
        found = ar.daily_problems("deploy", "0.4.6", "0.4.6", None, ("differs", "the site names 0.4.5, not 0.4.6"), self.NOW, self.NOW - 30 * self.HOUR)
        self.assertEqual(found, ["the live site does not name the newest complete release: the site names 0.4.5, not 0.4.6"])

    def test_a_site_behind_a_release_published_within_the_bound_passes(self):
        found = ar.daily_problems("deploy", "0.4.6", "0.4.6", None, ("differs", "the site names 0.4.5, not 0.4.6"), self.NOW, self.NOW - self.HOUR)
        self.assertEqual(found, [])

    def test_a_read_that_was_challenged_fails_as_not_verified(self):
        found = ar.daily_problems("deploy", "0.4.6", "0.4.6", None, ("not verified", "the site answered 403 with a challenge"), self.NOW, None)
        self.assertEqual(found, ["the live site was not verified: the site answered 403 with a challenge"])

    def test_a_held_deploy_past_the_bound_and_a_stale_site_are_two_problems(self):
        since = self.NOW - 30 * self.HOUR
        found = ar.daily_problems("held", "0.4.7", "0.4.6", since, ("differs", "the site names 0.4.5, not 0.4.6"), self.NOW, self.NOW - 90 * self.HOUR)
        self.assertEqual(len(found), 2, found)

    def test_a_time_as_github_writes_it_is_read_in_seconds(self):
        self.assertEqual(ar.seconds("2026-11-14T22:32:39Z"), 1794695559)
        self.assertIsNone(ar.seconds(None))
        self.assertIsNone(ar.seconds("yesterday"))


class TreeVersion(unittest.TestCase):
    def test_the_workspace_version_is_the_first_version_line(self):
        manifest = '[workspace.package]\nversion = "0.4.6"\nedition = "2024"\n\n[workspace.dependencies]\nserde = { version = "1.0.0" }\n'
        self.assertEqual(ar.tree_version(manifest), "0.4.6")

    def test_a_manifest_without_a_version_is_refused(self):
        with self.assertRaises(ar.Refused):
            ar.tree_version("[workspace]\nmembers = []\n")

    def test_the_manifest_of_the_tree_the_script_is_in_has_the_version_it_reads(self):
        # The root Cargo.toml next to scripts/: the repository's, or the
        # hook test's fixture (scripts/test-hooks.sh), which is shaped like it.
        text = (HERE.parent / "Cargo.toml").read_text(encoding="utf-8")
        self.assertIsNotNone(ar.version_key(ar.tree_version(text)))


if __name__ == "__main__":
    unittest.main()
