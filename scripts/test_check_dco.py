"""Tests scripts/check_dco.py: the DCO sign-off rule for a pull request's
commits, and the commit-msg hook's warning.

Run with: python3 -I scripts/test_check_dco.py
"""

import contextlib
import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_dco", HERE / "check_dco.py")
dco = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dco)

OWNER = "NormB"
# Every address but the reserved ones is assembled at run time: the commit
# hook refuses a real-looking address in a tracked file, a GitHub noreply
# address included (scripts/scan_forbidden.py allows one only on a
# Signed-off-by line of a commit message).
AT = "@"
NOREPLY = "users.noreply.github.com"


def addr(local, domain=NOREPLY):
    return local + AT + domain


MAIL = addr("12345+contributor")
OLD_MAIL = addr("contributor")
OTHER = addr("9+other")
REAL = "contributor@example.com"
BOT_MAIL = addr("49699333+dependabot[bot]")


def commit(message="Change it", *, email=MAIL, login="contributor", kind="User",
           committer="web-flow", verified=True, parents=1, sha="a" * 40, name="A Contributor"):
    return dco.Commit(sha=sha, parents=parents, author_login=login, author_type=kind,
                      committer_login=committer, verified=verified, author_name=name,
                      author_email=email, message=message)


def signed(email=MAIL, name="A Contributor"):
    return f"Change it\n\nSigned-off-by: {name} <{email}>\n"


def api(message=None, *, email=MAIL, login="contributor", kind="User", committer="web-flow",
        verified=True, sha="b" * 40):
    return {
        "sha": sha,
        "parents": [{"sha": "c" * 40}],
        "author": None if login is None else {"login": login, "type": kind},
        "committer": None if committer is None else {"login": committer},
        "commit": {
            "author": {"name": "A Contributor", "email": email},
            "message": signed(email) if message is None else message,
            "verification": {"verified": verified},
        },
    }


def run_main(argv, text=None):
    """main's exit status and what it printed, with `text` as the FILE."""
    out, err = io.StringIO(), io.StringIO()
    with tempfile.TemporaryDirectory() as tmp:
        if text is not None:
            path = pathlib.Path(tmp, "commits.json")
            path.write_text(text, encoding="utf-8")
            argv = [path if a == "FILE" else a for a in argv]
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = dco.main([str(a) for a in argv])
    return rc, out.getvalue(), err.getvalue()


def run_warn(message, ident):
    """warn_local's exit status, standard output and standard error."""
    out, err = io.StringIO(), io.StringIO()
    with tempfile.TemporaryDirectory() as tmp:
        path = pathlib.Path(tmp, "COMMIT_EDITMSG")
        path.write_text(message, encoding="utf-8")
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = dco.main(["--warn-local", str(path), ident])
    return rc, out.getvalue(), err.getvalue()


class Passes(unittest.TestCase):
    def test_a_noreply_sign_off_naming_the_author_passes(self):
        self.assertEqual(dco.verdict(commit(signed()), OWNER), (True, "signed off by A Contributor"))

    def test_the_noreply_form_without_an_id_passes(self):
        self.assertTrue(dco.verdict(commit(signed(OLD_MAIL), email=OLD_MAIL), OWNER)[0])

    def test_the_email_compares_without_case(self):
        self.assertTrue(dco.verdict(commit(signed(MAIL.upper())), OWNER)[0])

    def test_the_key_compares_without_case(self):
        self.assertTrue(dco.verdict(commit(f"x\n\nsigned-off-by: A <{MAIL}>\n"), OWNER)[0])

    def test_a_verified_bot_commit_needs_no_sign_off(self):
        found = dco.verdict(commit("bump", email=BOT_MAIL, login="dependabot[bot]", kind="Bot"), OWNER)
        self.assertEqual(found, (True, "bot account dependabot[bot], signature verified"))

    def test_a_verified_owner_commit_needs_no_sign_off(self):
        found = dco.verdict(commit("x", email="owner@example.com", login=OWNER, committer=OWNER), OWNER)
        self.assertEqual(found, (True, "owner NormB, signature verified"))

    def test_the_owner_login_compares_without_case(self):
        self.assertTrue(dco.verdict(commit("x", login="normb", committer="NORMB"), OWNER)[0])

    def test_one_matching_sign_off_among_several_passes(self):
        message = f"x\n\nSigned-off-by: Other <{OTHER}>\nSigned-off-by: A <{MAIL}>\n"
        self.assertTrue(dco.verdict(commit(message), OWNER)[0])

    def test_trailing_spaces_after_the_address_pass(self):
        self.assertTrue(dco.verdict(commit(f"x\n\nSigned-off-by: A <{MAIL}>   \n"), OWNER)[0])

    def test_a_crlf_sign_off_passes(self):
        self.assertTrue(dco.verdict(commit(f"x\r\n\r\nSigned-off-by: A <{MAIL}>\r\n"), OWNER)[0])

    def test_a_pull_request_of_passing_commits_exits_0(self):
        rc, lines = dco.check([commit(signed(), sha="1" * 40), commit(signed(), sha="2" * 40)], OWNER)
        self.assertEqual(rc, 0)
        self.assertEqual(lines[-1], "DCO: 2 commit(s), 0 without a sign-off")

    def test_arrays_written_one_after_another_are_read(self):
        self.assertEqual(dco.read_pages('[{"a": 1}]\n[{"a": 2}]\n'), [{"a": 1}, {"a": 2}])

    def test_one_page_of_several_commits_is_read(self):
        self.assertEqual(len(dco.read_pages('[{"a": 1}, {"a": 2}, {"a": 3}]')), 3)

    def test_an_api_commit_becomes_a_commit(self):
        found = dco.from_api(api())
        self.assertEqual((found.sha, found.parents, found.author_login, found.author_type,
                          found.committer_login, found.verified, found.author_email),
                         ("b" * 40, 1, "contributor", "User", "web-flow", True, MAIL))

    def test_main_exits_0_on_a_signed_off_pull_request(self):
        rc, out, _ = run_main(["--owner", OWNER, "FILE"], json.dumps([api()]))
        self.assertEqual(rc, 0, out)
        self.assertIn("OK bbbbbbbbb: signed off by A Contributor", out)

    def test_a_noreply_address_with_an_id_is_noreply(self):
        self.assertTrue(dco.is_noreply(MAIL))

    def test_a_bot_noreply_address_is_noreply(self):
        self.assertTrue(dco.is_noreply(BOT_MAIL))

    def test_no_warning_for_a_noreply_sign_off(self):
        self.assertIsNone(dco.local_warning(signed(), MAIL))

    def test_warn_local_is_silent_and_exits_0_when_signed_off(self):
        self.assertEqual(run_warn(signed(), f"A Contributor <{MAIL}> 1700000000 +0000"), (0, "", ""))

    def test_the_author_email_is_read_from_the_ident(self):
        self.assertEqual(dco.ident_email(f"A Contributor <{MAIL}> 1700000000 +0000"), MAIL)


class Fails(unittest.TestCase):
    def test_no_sign_off_fails(self):
        self.assertEqual(dco.verdict(commit("Change it"), OWNER), (False, "no Signed-off-by line"))

    def test_a_sign_off_naming_another_address_fails(self):
        ok, why = dco.verdict(commit(signed(OTHER)), OWNER)
        self.assertFalse(ok)
        self.assertIn(f"<{MAIL}>", why)

    def test_a_sign_off_without_a_name_fails(self):
        self.assertFalse(dco.verdict(commit(f"x\n\nSigned-off-by: <{MAIL}>\n"), OWNER)[0])

    def test_a_real_address_fails_even_when_it_is_the_authors(self):
        ok, why = dco.verdict(commit(signed(REAL), email=REAL), OWNER)
        self.assertFalse(ok)
        self.assertIn("not a GitHub noreply address", why)

    def test_an_unverified_bot_commit_fails(self):
        self.assertFalse(dco.verdict(commit("bump", login="dependabot[bot]", kind="Bot", verified=False), OWNER)[0])

    def test_an_unverified_owner_commit_fails(self):
        self.assertFalse(dco.verdict(commit("x", login=OWNER, committer=OWNER, verified=False), OWNER)[0])

    def test_an_owner_commit_another_account_committed_fails(self):
        self.assertFalse(dco.verdict(commit("x", login=OWNER, committer="web-flow"), OWNER)[0])

    def test_a_commit_the_owner_only_committed_fails(self):
        self.assertFalse(dco.verdict(commit("x", login="contributor", committer=OWNER), OWNER)[0])

    def test_one_failing_commit_fails_the_pull_request_with_the_fix(self):
        rc, lines = dco.check([commit(signed(), sha="1" * 40), commit("x", sha="2" * 40)], OWNER)
        self.assertEqual(rc, 1)
        self.assertIn("FAIL 222222222: no Signed-off-by line", lines)
        self.assertIn(dco.FIX, lines)
        self.assertEqual(lines[-1], "DCO: 2 commit(s), 1 without a sign-off")

    def test_no_commit_checks_nothing(self):
        self.assertEqual(dco.check([], OWNER)[0], 2)

    def test_main_without_its_arguments_is_a_usage_error(self):
        self.assertEqual(run_main(["--owner", OWNER])[0], 2)

    def test_main_with_a_missing_file_checks_nothing(self):
        rc, _, err = run_main(["--owner", OWNER, "/nonexistent/commits.json"])
        self.assertEqual(rc, 2)
        self.assertIn("nothing was checked", err)

    def test_main_with_broken_json_checks_nothing(self):
        self.assertEqual(run_main(["--owner", OWNER, "FILE"], "[{")[0], 2)

    def test_a_page_that_is_not_an_array_is_refused(self):
        with self.assertRaises(ValueError):
            dco.read_pages('{"sha": "x"}')

    def test_main_with_a_commit_missing_its_body_checks_nothing(self):
        self.assertEqual(run_main(["--owner", OWNER, "FILE"], '[{"sha": "x"}]')[0], 2)

    def test_main_exits_1_on_a_commit_without_a_sign_off(self):
        rc, out, _ = run_main(["--owner", OWNER, "FILE"], json.dumps([api("Change it")]))
        self.assertEqual(rc, 1)
        self.assertIn("FAIL bbbbbbbbb: no Signed-off-by line", out)

    def test_the_warning_names_the_author_email(self):
        found = dco.local_warning("Change it", MAIL)
        self.assertIsNotNone(found)
        self.assertIn(f"<{MAIL}>", found)

    def test_the_warning_asks_for_a_noreply_author_email(self):
        found = dco.local_warning(signed(REAL), REAL)
        self.assertIsNotNone(found)
        self.assertIn("noreply", found)

    def test_warn_local_warns_on_standard_error_and_exits_0(self):
        rc, out, err = run_warn("Change it", f"A <{MAIL}> 1700000000 +0000")
        self.assertEqual((rc, out), (0, ""))
        self.assertTrue(err.startswith("commit-msg: warning: "), err)

    def test_a_real_address_is_not_noreply(self):
        self.assertFalse(dco.is_noreply("someone@gm" "ail.com"))

    def test_a_lookalike_domain_is_not_noreply(self):
        self.assertFalse(dco.is_noreply("a@users.noreply.github.com.example.com"))
        self.assertFalse(dco.is_noreply(addr("a", "noreply.github.com")))

    def test_a_real_address_is_never_printed(self):
        # CI's log of a pull request is public.
        ok, why = dco.verdict(commit(signed(OTHER), email=REAL), OWNER)
        self.assertFalse(ok)
        self.assertNotIn(REAL, why)
        self.assertIn("not shown", why)

    def test_a_sign_off_inside_a_line_is_not_one(self):
        self.assertFalse(dco.verdict(commit(f"x\n\nSee Signed-off-by: A <{MAIL}>\n"), OWNER)[0])


class Edges(unittest.TestCase):
    def test_no_space_after_the_key_passes(self):
        self.assertTrue(dco.verdict(commit(f"x\n\nSigned-off-by:A <{MAIL}>\n"), OWNER)[0])

    def test_tabs_around_the_name_pass(self):
        self.assertTrue(dco.verdict(commit(f"x\n\nSigned-off-by:\tA\t<{MAIL}>\n"), OWNER)[0])

    def test_a_name_in_any_script_passes(self):
        self.assertEqual(dco.verdict(commit(signed(name="Zoë Ünal")), OWNER), (True, "signed off by Zoë Ünal"))

    def test_an_address_with_a_space_is_no_sign_off(self):
        self.assertEqual(dco.sign_offs(f"Signed-off-by: A <{addr('a b')}>"), [])

    def test_text_after_the_address_is_no_sign_off(self):
        self.assertEqual(dco.sign_offs(f"Signed-off-by: A <{MAIL}> (work)"), [])

    def test_a_merge_commit_follows_the_same_rule(self):
        self.assertFalse(dco.verdict(commit("Merge main", parents=2), OWNER)[0])
        self.assertTrue(dco.verdict(commit(signed(), parents=2), OWNER)[0])

    def test_an_author_without_a_github_account_can_sign_off(self):
        found = dco.from_api(api(login=None))
        self.assertIsNone(found.author_login)
        self.assertTrue(dco.verdict(found, OWNER)[0])

    def test_a_verified_user_that_is_no_bot_needs_a_sign_off(self):
        self.assertFalse(dco.verdict(commit("x", kind="User", verified=True), OWNER)[0])

    def test_a_bot_needs_no_sign_off_even_with_a_wrong_one(self):
        message = "bump\n\nSigned-off-by: dependabot[bot] <support@github.com>\n"
        self.assertTrue(dco.verdict(commit(message, login="dependabot[bot]", kind="Bot"), OWNER)[0])

    def test_an_owner_commit_without_a_committer_account_fails(self):
        self.assertFalse(dco.verdict(dco.from_api(api("x", login=OWNER, committer=None)), OWNER)[0])

    def test_an_empty_owner_is_a_usage_error(self):
        self.assertEqual(run_main(["--owner", "", "FILE"], json.dumps([api()]))[0], 2)

    def test_a_file_of_white_space_holds_no_commit(self):
        rc, out, _ = run_main(["--owner", OWNER, "FILE"], "\n \n")
        self.assertEqual(rc, 2)
        self.assertIn("nothing was checked", out)

    def test_an_empty_author_email_fails(self):
        self.assertFalse(dco.verdict(commit(f"x\n\nSigned-off-by: A <>\n", email=""), OWNER)[0])

    def test_a_missing_verification_is_unverified(self):
        raw = api("x", login=OWNER, committer=OWNER)
        del raw["commit"]["verification"]
        self.assertFalse(dco.from_api(raw).verified)

    def test_the_noreply_domain_compares_without_case(self):
        self.assertTrue(dco.is_noreply(addr("12345+Contributor", "Users.NoReply.GitHub.com")))

    def test_a_login_cannot_start_with_a_hyphen(self):
        self.assertTrue(dco.is_noreply(addr("12345+a-b")))
        self.assertFalse(dco.is_noreply(addr("12345+-ab")))

    def test_an_id_without_its_plus_is_no_address(self):
        self.assertFalse(dco.is_noreply(addr("12345", NOREPLY + "+x")))

    def test_an_unreadable_message_is_not_checked_and_exits_0(self):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = dco.main(["--warn-local", "/nonexistent/COMMIT_EDITMSG", f"A <{MAIL}>"])
        self.assertEqual(rc, 0)
        self.assertIn("not checked", err.getvalue())

    def test_an_ident_without_brackets_is_the_email(self):
        self.assertEqual(dco.ident_email(f"  {MAIL} "), MAIL)

    def test_an_empty_owner_matches_no_commit(self):
        self.assertFalse(dco.verdict(commit("x", login=None, committer=None), "")[0])

    def test_a_merge_commit_from_the_api_keeps_its_parents(self):
        raw = api()
        raw["parents"].append({"sha": "d" * 40})
        self.assertEqual(dco.from_api(raw).parents, 2)

    def test_a_nameless_sign_off_warns_locally(self):
        self.assertIsNotNone(dco.local_warning(f"x\n\nSigned-off-by: <{MAIL}>\n", MAIL))

    def test_the_output_names_each_commit_by_nine_characters(self):
        _, lines = dco.check([commit(signed(), sha="0123456789abcdef" + "0" * 24)], OWNER)
        self.assertEqual(lines[0], "OK 012345678: signed off by A Contributor")


if __name__ == "__main__":
    unittest.main()
