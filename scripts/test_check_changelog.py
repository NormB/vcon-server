"""Tests scripts/check_changelog.py: the shape of a changelog entry.

Run with: python3 -I scripts/test_check_changelog.py
"""

import contextlib
import importlib.util
import io
import pathlib
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_changelog", HERE / "check_changelog.py")
cc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cc)

HEAD = "# Changelog\n\nAll notable changes.\n\n"
GOOD = (
    "- **A server keeps its catalogs in PostgreSQL.**\n"
    "  - Effect: With `catalog.backend = \"postgres\"` the registry is in the\n"
    "    database.\n"
    "  - Do: Run `vcon-server migrate` first.\n"
    "  - Limit: One server serves a database.\n"
    "  - Docs: [keep the catalogs in\n"
    "    PostgreSQL](https://vcon-server.example.com/docs/postgres-catalog/#the-limits-of-the-preview)\n"
)


def log(entries, version="Unreleased", kind="Added", intro=""):
    """A changelog with one judged section holding `entries`, and an older
    section in the old prose form, which is not judged."""
    heading = f"## [{version}]" if version == "Unreleased" else f"## [{version}] - 2026-11-12"
    return (
        f"{HEAD}{heading}\n\n{intro}### {kind}\n\n{entries}\n"
        "## [0.1.0] - 2026-10-26\n\n### Added\n\n- Old prose, a paragraph of any\n  length. No lead.\n"
    )


def lead_of(count):
    """A lead of `count` words, wrapped onto a second line after the eighth."""
    return " ".join(["word"] * 8) + "\n  " + " ".join(["word"] * (count - 8))


def problems(entries, **how):
    return cc.problems(log(entries, **how), "Unreleased")


class WhatMustPass(unittest.TestCase):
    def test_an_entry_with_a_lead_and_labeled_sub_bullets_passes(self):
        self.assertEqual(problems(GOOD), [])

    def test_a_fix_needs_was_and_no_effect(self):
        entry = "- **`migrate` frees the database before it returns.**\n  - Was: A server started at once could be refused.\n"
        self.assertEqual(problems(entry, kind="Fixed"), [])

    def test_a_lead_may_wrap_onto_a_second_line(self):
        entry = "- **The catalogs can be kept in PostgreSQL, a preview that the\n  release binaries hold.**\n  - Effect: One.\n"
        self.assertEqual(problems(entry), [])

    def test_a_table_of_six_rows_stands_in_place_of_the_sub_bullets(self):
        rows = "".join(f"  | `t{n}` | 1 | 2 |\n" for n in range(6))
        entry = "- **The release binaries are larger.**\n\n  | Target | Before | After |\n  |---|---|---|\n" + rows
        self.assertEqual(problems(entry), [])

    def test_a_section_may_open_with_a_paragraph_before_its_first_heading(self):
        intro = "The upgrade from 0.4.4 is the binary alone. No stored data\nchanges.\n\n"
        self.assertEqual(problems(GOOD, intro=intro), [])

    def test_a_line_that_is_one_long_link_is_not_too_long(self):
        long = "https://vcon-server.example.com/docs/" + "x" * 80
        entry = f"- **A lead.**\n  - Effect: One.\n  - Docs: [the\n    page]({long})\n"
        self.assertEqual(problems(entry), [])

    def test_a_link_counts_as_the_words_a_reader_sees(self):
        words = " ".join(["word"] * 11)
        entry = f"- **A lead.**\n  - Effect: {words}\n    {words}\n    [one\n    word](https://vcon-server.example.com/a/very/long/address/that/is/one/word).\n"
        self.assertEqual(problems(entry), [])

    def test_sections_below_the_marker_are_not_judged(self):
        text = log(GOOD, version="0.4.5")
        self.assertEqual(cc.problems(text, "0.4.5"), [])
        self.assertEqual(
            [p for p in cc.problems(text, "0.1.0") if "[0.1.0]" in p][:1],
            ["[0.1.0]: `Old prose, a paragraph of any`: the entry does not start with a bold lead (`- **Lead.**`)"],
        )

    def test_every_section_from_the_top_to_the_marker_is_judged(self):
        text = (
            f"{HEAD}## [Unreleased]\n\n### Added\n\n{GOOD}\n"
            "## [0.4.5] - 2026-11-12\n\n### Fixed\n\n- No lead here.\n\n"
            "## [0.4.4] - 2026-11-12\n\n- Old prose.\n"
        )
        found = cc.problems(text, "0.4.5")
        self.assertEqual(len(found), 1, found)
        self.assertIn("[0.4.5]", found[0])

    def test_the_repositorys_changelog_passes(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = cc.main(HERE.parent)
        self.assertEqual(code, 0, out.getvalue())

    def test_the_marker_names_a_section_of_the_repositorys_changelog(self):
        text = (HERE.parent / "CHANGELOG.md").read_text(encoding="utf-8")
        headings = [heading for heading, _ in cc.sections(text)]
        self.assertTrue(any(h.startswith(f"## [{cc.MARKER}]") for h in headings), headings[:4])
        self.assertTrue(headings[0].startswith("## [Unreleased]"), headings[:1])

    def test_the_contributor_page_states_the_rule_the_gate_holds(self):
        page = (HERE.parent / "website/content/docs/development.md").read_text(encoding="utf-8")
        flat = " ".join(page.split())
        self.assertIn("scripts/check_changelog.py", flat)
        for label in cc.LABELS:
            self.assertIn(f"`{label}:`", flat, f"the page names the label {label}")
        self.assertIn(f"{cc.LEAD_WORDS} words", flat)
        self.assertIn(f"{cc.BULLET_WORDS} words", flat)


class WhatMustBeRefused(unittest.TestCase):
    def one(self, entries, rule, **how):
        found = problems(entries, **how)
        self.assertEqual(len(found), 1, found)
        self.assertIn(rule, found[0])
        self.assertTrue(found[0].startswith("[Unreleased]: `"), found[0])
        return found[0]

    def test_a_bullet_with_no_bold_lead(self):
        self.one("- A server keeps its catalogs.\n  - Effect: One.\n", "does not start with a bold lead")

    def test_prose_after_the_lead_on_the_bullet(self):
        self.one("- **A lead.** And a sentence of prose after it.\n  - Effect: One.\n", "the lead is the whole bullet")

    def test_a_lead_of_sixteen_words(self):
        said = self.one(f"- **{lead_of(16)}.**\n  - Effect: One.\n", "the lead has 16 words, at most 15")
        self.assertIn("`word word", said)

    def test_an_entry_with_no_sub_bullet(self):
        self.one("- **A lead.**\n", "has no sub-bullet and no table")

    def test_an_entry_with_six_sub_bullets(self):
        entry = "- **A lead.**\n  - Effect: One.\n  - Do: Two.\n  - Limit: Three.\n  - Docs: Four.\n  - Was: Five.\n  - Effect: Six.\n"
        found = problems(entry)
        self.assertTrue(any("has 6 sub-bullets, at most 5" in p for p in found), found)

    def test_a_sub_bullet_without_a_label(self):
        self.one("- **A lead.**\n  - Effect: One.\n  - One server serves a database.\n", "does not start with one of Effect:, Do:, Limit:, Docs:, Was:")

    def test_a_label_the_format_does_not_have(self):
        self.one("- **A lead.**\n  - Effect: One.\n  - Note: Two.\n", "does not start with one of")

    def test_a_label_used_twice(self):
        self.one("- **A lead.**\n  - Effect: One.\n  - Effect: Two.\n", "the label Effect: is used twice")

    def test_an_entry_with_neither_effect_nor_was(self):
        self.one("- **A lead.**\n  - Do: Run it.\n  - Docs: [page](https://vcon-server.example.com/).\n", "has neither Effect: nor Was:")

    def test_a_sub_bullet_of_twenty_six_words(self):
        words = " ".join(["word"] * 13)
        self.one(f"- **A lead.**\n  - Effect: {words}\n    {words}\n", "the Effect: sub-bullet has 26 words, at most 25")

    def test_a_third_level(self):
        found = problems("- **A lead.**\n  - Effect: One.\n    - Deeper.\n")
        self.assertTrue(any("a third level of nesting" in p for p in found), found)

    def test_a_paragraph_under_the_bullet(self):
        found = problems("- **A lead.**\n  - Effect: One.\n\n  A paragraph that goes on.\n")
        self.assertTrue(any("a paragraph under the bullet" in p for p in found), found)

    def test_a_paragraph_between_the_lead_and_the_sub_bullets(self):
        found = problems("- **A lead.**\n\n  Prose first.\n\n  - Effect: One.\n")
        self.assertTrue(any("a paragraph under the bullet" in p for p in found), found)

    def test_a_line_longer_than_the_wrap(self):
        line = "  - Effect: " + " ".join(["word"] * 16)
        self.assertGreater(len(line), cc.WRAP)
        found = problems(f"- **A lead.**\n{line}\n")
        self.assertTrue(any(f"a line of {len(line)} characters, at most {cc.WRAP}" in p for p in found), found)

    def test_a_table_of_seven_rows(self):
        rows = "".join(f"  | `t{n}` | 1 | 2 |\n" for n in range(7))
        entry = "- **A lead.**\n\n  | Target | Before | After |\n  |---|---|---|\n" + rows
        self.one(entry, "the table has 7 rows, at most 6")

    def test_a_table_and_sub_bullets_together(self):
        entry = "- **A lead.**\n  - Effect: One.\n\n  | A | B |\n  |---|---|\n  | 1 | 2 |\n"
        self.one(entry, "has a table and sub-bullets; a table stands in place of them")

    def test_prose_under_a_heading_outside_any_entry(self):
        found = problems("Loose prose under the heading.\n\n" + GOOD)
        self.assertEqual(len(found), 1, found)
        self.assertIn("text outside an entry", found[0])

    def test_a_heading_the_format_does_not_have(self):
        found = problems(GOOD, kind="Notes")
        self.assertEqual(len(found), 1, found)
        self.assertIn("`### Notes` is not one of", found[0])

    def test_a_marker_the_changelog_does_not_have(self):
        with self.assertRaises(ValueError) as raised:
            cc.problems(log(GOOD), "9.9.9")
        self.assertIn("9.9.9", str(raised.exception))

    def test_main_prints_the_section_the_lead_and_the_rule_and_exits_1(self):
        with tempfile.TemporaryDirectory() as root:
            root = pathlib.Path(root)
            (root / "CHANGELOG.md").write_text(
                log("- **A lead.**\n", version=cc.MARKER), encoding="utf-8"
            )
            out = io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
                code = cc.main(root)
        self.assertEqual(code, 1)
        self.assertIn(f"CHANGELOG.md: [{cc.MARKER}]: `A lead.`: the entry has no sub-bullet and no table", out.getvalue())


class TheEdges(unittest.TestCase):
    def test_a_lead_of_fifteen_words_and_a_sub_bullet_of_twenty_five_pass(self):
        words = " ".join(["word"] * 13)
        entry = f"- **{lead_of(15)}.**\n  - Effect: {words}\n    {' '.join(['word'] * 12)}\n"
        self.assertEqual(problems(entry), [])

    def test_five_sub_bullets_pass(self):
        entry = "- **A lead.**\n  - Was: One.\n  - Effect: Two.\n  - Do: Three.\n  - Limit: Four.\n  - Docs: Five.\n"
        self.assertEqual(problems(entry), [])

    def test_a_line_of_exactly_the_wrap_passes(self):
        line = "  - Effect: " + "x" * 30 + " " + "y" * (cc.WRAP - 43)
        self.assertEqual(len(line), cc.WRAP)
        self.assertEqual(problems(f"- **A lead.**\n{line}\n"), [])

    def test_words_are_counted_without_the_label_and_code_is_a_word(self):
        self.assertEqual(cc.words("Run `vcon-server migrate` [as shown](https://vcon-server.example.com/x) first."), 6)
        self.assertEqual(cc.words(""), 0)

    def test_an_empty_section_passes(self):
        text = f"{HEAD}## [Unreleased]\n\n## [0.4.5] - 2026-11-12\n\n### Fixed\n\n{GOOD.replace('Effect', 'Was')}"
        self.assertEqual(cc.problems(text, "0.4.5"), [])

    def test_sections_are_split_at_release_headings_only(self):
        text = f"{HEAD}## [Unreleased]\n\n### Added\n\n{GOOD}\n## [0.4.5] - 2026-11-12\n\n### Fixed\n"
        self.assertEqual([h for h, _ in cc.sections(text)], ["## [Unreleased]", "## [0.4.5] - 2026-11-12"])

    def test_every_problem_of_an_entry_is_reported_not_only_the_first(self):
        found = problems(f"- **{lead_of(16)}.**\n  - Note: One.\n")
        self.assertEqual(len(found), 3, found)


if __name__ == "__main__":
    unittest.main()
