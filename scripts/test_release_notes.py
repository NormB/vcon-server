"""Tests scripts/release_notes.py: the text of a GitHub release, taken from
the changelog section of the version, with the line that says how to
verify a download.

Run with: python3 -I scripts/test_release_notes.py
"""

import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("release_notes", HERE / "release_notes.py")
release_notes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release_notes)

REPO = "example/vcon-server"

CHANGELOG = """# Changelog

All notable changes to this project are recorded here.

## [Unreleased]

### Fixed

- Not released yet.

## [0.4.4] - 2026-11-12

There is no downgrade from 0.4.4.

### Security

- **rusqlite 0.42.0.** For RUSTSEC-2026-0459.

### Added

- Key expiry. See the [upgrade page][upgrade].

## [0.4.3] - 2026-11-09

### Fixed

- A fix of 0.4.3.

## [0.4.10] - 2026-11-01

### Added

- A feature of 0.4.10.

## [0.1.0] - 2026-09-01

- The first release.

[Unreleased]: https://github.com/example/vcon-server/compare/v0.4.4...HEAD
[0.4.4]: https://github.com/example/vcon-server/releases/tag/v0.4.4
[upgrade]: https://example.com/docs/upgrading/
"""


def section(version, changelog=CHANGELOG):
    return release_notes.section(changelog, version)


def refusal(function, *arguments):
    """The message of the Refused that the call raises."""
    try:
        function(*arguments)
    except release_notes.Refused as refused:
        return str(refused)
    raise AssertionError("the call was not refused")


class Section(unittest.TestCase):
    # What must work.

    def test_the_section_of_a_version_is_the_text_under_its_heading(self):
        self.assertEqual(
            section("0.4.3"),
            "### Fixed\n\n- A fix of 0.4.3.",
        )

    def test_the_section_ends_at_the_next_version(self):
        text = section("0.4.4")
        self.assertTrue(text.startswith("There is no downgrade from 0.4.4."), text)
        self.assertTrue(text.endswith("- Key expiry. See the [upgrade page](https://example.com/docs/upgrading/)."), text)
        self.assertNotIn("0.4.3", text)

    def test_the_subheadings_of_the_section_are_kept(self):
        text = section("0.4.4")
        self.assertIn("### Security\n", text)
        self.assertIn("### Added\n", text)

    def test_the_heading_of_the_version_is_left_out(self):
        self.assertNotIn("## [0.4.4]", section("0.4.4"))

    def test_the_last_section_ends_at_the_link_definitions(self):
        self.assertEqual(section("0.1.0"), "- The first release.")

    def test_a_version_is_matched_whole(self):
        self.assertEqual(section("0.4.10"), "### Added\n\n- A feature of 0.4.10.")

    def test_a_version_that_is_the_start_of_another_is_not_confused_with_it(self):
        changelog = "## [0.4.10] - 2026-11-01\n\n- Ten.\n\n## [0.4.1] - 2026-10-01\n\n- One.\n"
        self.assertEqual(section("0.4.1", changelog), "- One.")

    def test_the_unreleased_section_is_not_a_versions(self):
        self.assertNotIn("Not released yet", section("0.4.4"))

    def test_blank_lines_around_the_section_are_dropped(self):
        changelog = "## [1.0.0] - 2026-01-01\n\n\n\n- One.\n\n\n\n## [0.9.0] - 2025-12-01\n\n- Old.\n"
        self.assertEqual(section("1.0.0", changelog), "- One.")

    def test_a_blank_line_inside_the_section_is_kept(self):
        changelog = "## [1.0.0] - 2026-01-01\n\nFirst.\n\nSecond.\n"
        self.assertEqual(section("1.0.0", changelog), "First.\n\nSecond.")

    def test_windows_line_ends_are_read(self):
        changelog = "## [1.0.0] - 2026-01-01\r\n\r\n- One.\r\n\r\n## [0.9.0] - 2025-12-01\r\n\r\n- Old.\r\n"
        self.assertEqual(section("1.0.0", changelog), "- One.")

    def test_a_heading_shown_in_a_code_block_does_not_end_the_section(self):
        changelog = "## [1.0.0] - 2026-01-01\n\n```text\n## [0.9.0] - not a heading\n```\n\n- After.\n\n## [0.9.0] - 2025-12-01\n\n- Old.\n"
        self.assertEqual(
            section("1.0.0", changelog),
            "```text\n## [0.9.0] - not a heading\n```\n\n- After.",
        )

    def test_a_pre_release_version_has_a_section_of_its_own(self):
        changelog = "## [1.0.0-rc.1] - 2026-01-01\n\n- A candidate.\n\n## [0.9.0] - 2025-12-01\n\n- Old.\n"
        self.assertEqual(section("1.0.0-rc.1", changelog), "- A candidate.")

    # What must be refused.

    def test_a_version_with_no_section_is_refused(self):
        self.assertEqual(
            refusal(section, "9.9.9"),
            "CHANGELOG.md has no section `## [9.9.9] - <date>`",
        )

    def test_the_unreleased_section_is_refused_as_a_version(self):
        self.assertIn("is not a version", refusal(section, "Unreleased"))

    def test_a_version_with_a_v_in_front_is_refused(self):
        self.assertEqual(
            refusal(section, "v0.4.4"),
            "`v0.4.4` is not a version: give it as Cargo.toml has it, such as 0.4.4",
        )

    def test_an_empty_version_is_refused(self):
        self.assertIn("is not a version", refusal(section, ""))

    def test_a_version_that_is_a_pattern_is_refused(self):
        self.assertIn("is not a version", refusal(section, "0.4.."))
        self.assertIn("is not a version", refusal(section, ".*"))

    def test_a_development_version_is_refused(self):
        self.assertIn("has no section", refusal(section, "0.4.4-dev.0123456789ab"))

    def test_a_section_with_no_text_is_refused(self):
        changelog = "## [1.0.0] - 2026-01-01\n\n## [0.9.0] - 2025-12-01\n\n- Old.\n"
        self.assertEqual(
            refusal(section, "1.0.0", changelog),
            "the section `## [1.0.0]` of CHANGELOG.md is empty",
        )

    def test_a_section_with_only_subheadings_is_refused(self):
        changelog = "## [1.0.0] - 2026-01-01\n\n### Added\n\n### Fixed\n\n## [0.9.0] - 2025-12-01\n\n- Old.\n"
        self.assertIn("is empty", refusal(section, "1.0.0", changelog))

    def test_two_sections_for_one_version_are_refused(self):
        changelog = "## [1.0.0] - 2026-01-01\n\n- One.\n\n## [1.0.0] - 2026-01-02\n\n- Again.\n"
        self.assertEqual(
            refusal(section, "1.0.0", changelog),
            "CHANGELOG.md has 2 sections `## [1.0.0]`",
        )

    def test_a_heading_without_a_date_is_not_the_versions_section(self):
        changelog = "## [1.0.0]\n\n- One.\n"
        self.assertIn("has no section", refusal(section, "1.0.0", changelog))

    def test_a_heading_of_another_level_is_not_the_versions_section(self):
        changelog = "### [1.0.0] - 2026-01-01\n\n- One.\n"
        self.assertIn("has no section", refusal(section, "1.0.0", changelog))

    def test_an_empty_changelog_is_refused(self):
        self.assertIn("has no section", refusal(section, "1.0.0", ""))


class Notes(unittest.TestCase):
    def notes(self, version="0.4.4", changelog=CHANGELOG, repo=REPO):
        return release_notes.notes(changelog, version, repo)

    # What must work.

    def test_the_notes_are_the_section_then_how_to_verify_a_download(self):
        self.assertEqual(
            self.notes("0.4.3"),
            "### Fixed\n\n- A fix of 0.4.3.\n\n"
            "## Verify a download\n\n"
            "Verify a download with: gh attestation verify <file> --repo example/vcon-server. "
            "SHA256SUMS.txt.sigstore.json signs SHA256SUMS.txt; SECURITY.md has the cosign command.\n",
        )

    def test_the_verify_line_names_the_repository_given(self):
        self.assertIn("--repo other/fork.", self.notes(repo="other/fork"))

    def test_the_verify_line_is_last(self):
        self.assertTrue(self.notes().rstrip("\n").endswith("SECURITY.md has the cosign command."))

    def test_the_notes_end_with_one_newline(self):
        text = self.notes()
        self.assertTrue(text.endswith("\n") and not text.endswith("\n\n"), repr(text[-5:]))

    def test_a_link_by_reference_is_written_in_full(self):
        # A release's text does not hold the changelog's link definitions.
        self.assertIn("See the [upgrade page](https://example.com/docs/upgrading/).", self.notes())

    def test_a_reference_in_two_brackets_and_a_shortcut_are_both_resolved(self):
        changelog = (
            "## [1.0.0] - 2026-01-01\n\n- See [the page][Page] and [page].\n\n"
            "[page]: https://example.com/page\n"
        )
        self.assertEqual(
            release_notes.section(changelog, "1.0.0"),
            "- See [the page](https://example.com/page) and [page](https://example.com/page).",
        )

    def test_brackets_with_no_definition_are_left_as_they_are(self):
        changelog = "## [1.0.0] - 2026-01-01\n\n- An array `[1, 2]` and [words] and [a][b].\n"
        self.assertEqual(
            release_notes.section(changelog, "1.0.0"),
            "- An array `[1, 2]` and [words] and [a][b].",
        )

    def test_an_inline_link_is_left_as_it_is(self):
        changelog = (
            "## [1.0.0] - 2026-01-01\n\n- The [page](https://example.com/a).\n\n"
            "[page]: https://example.com/b\n"
        )
        self.assertEqual(release_notes.section(changelog, "1.0.0"), "- The [page](https://example.com/a).")

    def test_code_is_not_rewritten(self):
        changelog = (
            "## [1.0.0] - 2026-01-01\n\n- Write `[page]` or:\n\n```text\n[page]\n```\n\n"
            "[page]: https://example.com/b\n"
        )
        self.assertEqual(
            release_notes.section(changelog, "1.0.0"),
            "- Write `[page]` or:\n\n```text\n[page]\n```",
        )

    # What must be refused.

    def test_a_repository_that_is_not_owner_and_name_is_refused(self):
        for repo in ["", "vcon-server", "a/b/c", "a b/c", "a/b --notes x", "https://github.com/a/b"]:
            with self.subTest(repo=repo):
                self.assertEqual(
                    refusal(release_notes.notes, CHANGELOG, "0.4.4", repo),
                    f"`{repo}` is not a repository: give it as owner/name",
                )

    def test_notes_longer_than_a_release_may_hold_are_refused(self):
        changelog = "## [1.0.0] - 2026-01-01\n\n" + "- A line of the section.\n" * 6000
        said = refusal(release_notes.notes, changelog, "1.0.0", REPO)
        self.assertIn("a GitHub release holds at most 125000", said)

    def test_notes_at_the_limit_pass(self):
        fixed = len(release_notes.notes("## [1.0.0] - 2026-01-01\n\nx\n", "1.0.0", REPO)) - 1
        changelog = "## [1.0.0] - 2026-01-01\n\n" + "x" * (release_notes.LIMIT - fixed) + "\n"
        self.assertEqual(len(release_notes.notes(changelog, "1.0.0", REPO)), release_notes.LIMIT)
        over = "## [1.0.0] - 2026-01-01\n\n" + "x" * (release_notes.LIMIT - fixed + 1) + "\n"
        self.assertIn("at most", refusal(release_notes.notes, over, "1.0.0", REPO))


class Command(unittest.TestCase):
    def run_in(self, arguments, changelog=CHANGELOG):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            if changelog is not None:
                (root / "CHANGELOG.md").write_text(changelog, encoding="utf-8")
            return subprocess.run(
                [sys.executable, "-I", str(HERE / "release_notes.py"), *arguments],
                cwd=root,
                capture_output=True,
                text=True,
            )

    def test_the_command_prints_the_notes(self):
        result = self.run_in(["0.4.3", "--repo", REPO])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, release_notes.notes(CHANGELOG, "0.4.3", REPO))
        self.assertEqual(result.stderr, "", "nothing else is written")

    def test_the_command_reads_the_changelog_it_is_given(self):
        with tempfile.TemporaryDirectory() as d:
            other = pathlib.Path(d) / "other.md"
            other.write_text("## [2.0.0] - 2027-01-01\n\n- Two.\n", encoding="utf-8")
            result = self.run_in(["2.0.0", "--repo", REPO, "--changelog", str(other)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith("- Two.\n"), result.stdout)

    def test_a_version_with_no_section_fails_and_prints_no_notes(self):
        result = self.run_in(["9.9.9", "--repo", REPO])
        self.assertEqual(result.returncode, 1, "no section, no release text")
        self.assertEqual(result.stdout, "", "nothing a release could be made from")
        self.assertEqual(result.stderr, "release_notes: CHANGELOG.md has no section `## [9.9.9] - <date>`\n")

    def test_a_missing_changelog_fails(self):
        result = self.run_in(["0.4.4", "--repo", REPO], changelog=None)
        self.assertEqual(result.returncode, 1, "no file to read")
        self.assertEqual(result.stdout, "", "no notes")
        self.assertIn("release_notes: CHANGELOG.md:", result.stderr)

    def test_a_missing_argument_is_a_usage_error(self):
        for arguments in [[], ["0.4.4"], ["--repo", REPO], ["0.4.4", "--repo"], ["0.4.4", "0.4.3", "--repo", REPO]]:
            with self.subTest(arguments=arguments):
                result = self.run_in(arguments)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(result.stdout, "", "no notes")
                self.assertIn("usage: release_notes.py VERSION --repo OWNER/NAME [--changelog PATH]", result.stderr)

    def test_an_option_given_twice_is_a_usage_error(self):
        result = self.run_in(["0.4.3", "--repo", REPO, "--repo", "other/fork"])
        self.assertEqual(result.returncode, 2, "which repository is meant is not guessed")
        self.assertEqual(result.stdout, "", "no notes")

    def test_an_unknown_option_is_a_usage_error(self):
        result = self.run_in(["0.4.4", "--repo", REPO, "--draft"])
        self.assertEqual(result.returncode, 2, result.stderr)

    def test_a_changelog_that_is_not_utf_8_fails(self):
        with tempfile.TemporaryDirectory() as d:
            other = pathlib.Path(d) / "other.md"
            other.write_bytes(b"## [2.0.0] - 2027-01-01\n\n- \xff\n")
            result = self.run_in(["2.0.0", "--repo", REPO, "--changelog", str(other)])
        self.assertEqual(result.returncode, 1, "not text")
        self.assertEqual(result.stdout, "", "no notes")


class Repository(unittest.TestCase):
    """The repository's own changelog and release workflow."""

    def test_every_released_version_of_the_changelog_has_notes(self):
        changelog = (HERE.parent / "CHANGELOG.md").read_text(encoding="utf-8")
        versions = release_notes.versions(changelog)
        self.assertGreaterEqual(len(versions), 8, versions)
        for version in versions:
            with self.subTest(version=version):
                text = release_notes.notes(changelog, version, "NormB/vcon-server")
                self.assertGreater(len(text), 200, "a section and the verify line")

    def test_the_release_workflow_takes_its_notes_from_the_script(self):
        workflow = (HERE.parent / ".github/workflows/release.yml").read_text(encoding="utf-8")
        publish = workflow.split("\n  publish:\n", 1)[1].split("\n  crates-io:\n", 1)[0]
        self.assertIn(
            'python3 -I scripts/release_notes.py "$VERSION" --repo "$GITHUB_REPOSITORY" > "$RUNNER_TEMP/release-notes.md"',
            publish,
        )
        self.assertIn('--notes-file "$RUNNER_TEMP/release-notes.md"', publish)
        self.assertNotIn("--notes ", publish, "no fixed text beside the file")
        self.assertNotIn("--generate-notes", publish, "the changelog is the text, not the commit list")
        self.assertLess(
            publish.index("scripts/release_notes.py"),
            publish.index("gh release create"),
            "the notes are written before the release is made",
        )
        self.assertLess(
            publish.index("actions/checkout@"),
            publish.index("actions/download-artifact@"),
            "a checkout empties the directory, so it comes before the download",
        )

    def test_a_tag_with_no_changelog_section_stops_the_release_before_the_builds(self):
        workflow = (HERE.parent / ".github/workflows/release.yml").read_text(encoding="utf-8")
        version = workflow.split("\n  version:\n", 1)[1].split("\n  build:\n", 1)[0]
        self.assertIn('python3 -I scripts/release_notes.py "$version" --repo "$GITHUB_REPOSITORY" > /dev/null', version)
        tagged = version.split('if [ "${GITHUB_REF_TYPE}" = tag ]; then\n', 1)[1].split("          else\n", 1)[0]
        self.assertIn(
            "python3 -I scripts/release_notes.py",
            tagged,
            "only a tag is a release: a run by hand has a development version with no section",
        )
        self.assertEqual(version.count("python3 -I scripts/release_notes.py"), 1, "and only there")


if __name__ == "__main__":
    unittest.main()
