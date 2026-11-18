//! The retired Contributor License Agreement (decision (iv), 2026-11-18).
//!
//! From 2026-11-18 the `main` ruleset requires the `DCO` sign-off check and
//! no longer requires the CLA assistant's `license/cla`. The agreement is
//! kept, marked retired: `CLA.md` opens with a notice that gives the date
//! and says the contributions made before it were made under it, the text
//! below the notice is the text contributors signed (held by its SHA-256),
//! the site's `/cla/` page reproduces `CLA.md` under a title that says it is
//! retired, and the site footer names it the retired agreement. No page
//! asks a contributor to sign it any more; CONTRIBUTING.md states that the
//! DCO sign-off is required, and MAINTAINERS.md records the owner's three
//! steps of the switch as done.
//!
//! Each rule is a function of its text, so the tests drive it with planted
//! text as well as with the files in the tree.

use super::{read, section};

/// The day the agreement retired.
const RETIRED: &str = "2026-11-18";

/// SHA-256 of `CLA.md` as contributors signed it, before the notice.
const SIGNED_SHA256: &str = "2a65efde9d95fbea4c3f8d7c29ffdb4ed35b4e3560530356e849083a4b83ef01";

/// The `/cla/` page's title line.
const SITE_TITLE: &str = r#"title = "Contributor License Agreement (retired)""#;

/// The footer link's text.
const FOOTER_TEXT: &str = "Retired contributor agreement";

/// Where the notice sends a contributor.
const DCO_ANCHOR: &str = "CONTRIBUTING.md#signing-off-commits";

/// Words that ask for the agreement or for its check, compared without case.
const ASKING: [&str; 7] = [
    "license/cla",
    "sign the [contributor license agreement",
    "signs the [contributor license agreement",
    "the cla signed",
    "and the cla.",
    "cla-assistant.io",
    "when the bot asks",
];

/// The files that once asked for the agreement or its check. MAINTAINERS.md
/// is read apart: its record of the switch names the check it removed.
/// scripts/check_required_checks.py names it too, as the context it now
/// refuses; scripts/test_check_required_checks.py holds that.
const PAGES: [&str; 8] = [
    "README.md",
    "CONTRIBUTING.md",
    "GOVERNANCE.md",
    "website/content/docs/development.md",
    ".bestpractices.json",
    ".github/workflows/dco.yml",
    ".githooks/commit-msg",
    "scripts/check_dco.py",
];

/// The block quote right under `text`'s first-line `# ` title, its lines
/// joined by spaces, or None.
fn notice(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if !lines.next()?.starts_with("# ") {
        return None;
    }
    let quoted: Vec<&str> = lines
        .skip_while(|line| line.trim().is_empty())
        .map_while(|line| line.strip_prefix('>'))
        .map(str::trim)
        .collect();
    (!quoted.is_empty()).then(|| quoted.join(" "))
}

/// What the retired notice of `cla` lacks for the date `date`.
fn notice_findings(cla: &str, date: &str) -> Vec<String> {
    let Some(text) = notice(cla) else {
        return vec!["no block quote right under the title".to_owned()];
    };
    let mut out = Vec::new();
    let lead = format!("**Retired on {date}.**");
    if !text.starts_with(&lead) {
        out.push(format!("the notice does not start with {lead}"));
    }
    let before = format!("Contributions made before {date} were made under it.");
    if !text.contains(&before) {
        out.push(format!("the notice does not say {before:?}"));
    }
    if !text.contains(DCO_ANCHOR) {
        out.push(format!("the notice does not link {DCO_ANCHOR}"));
    }
    out
}

/// `cla` without its notice: the title line, the blank lines after it, and
/// the text after the quote and the one blank line that ends it.
fn without_notice(cla: &str) -> String {
    let lines: Vec<&str> = cla.split_inclusive('\n').collect();
    let Some(title) = lines.first() else {
        return String::new();
    };
    let mut out = (*title).to_owned();
    let mut i = 1;
    while i < lines.len() && lines[i].trim().is_empty() {
        out.push_str(lines[i]);
        i += 1;
    }
    while i < lines.len() && lines[i].starts_with('>') {
        i += 1;
    }
    // Blank only after a quote: the loop above stopped on a line that is
    // not blank.
    if i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    out.push_str(&lines[i..].concat());
    out
}

/// The lowercase hex SHA-256 of `text`.
fn sha256_hex(text: &str) -> String {
    aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, text.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The site page's body: the lines after the second line that is exactly
/// `+++`, less the first of them, as `scripts/check-site.sh` read it
/// (`awk 'n >= 2' | tail -n +2`).
fn page_body(page: &str) -> Option<String> {
    let lines: Vec<&str> = page.split_inclusive('\n').collect();
    let second = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.strip_suffix('\n').unwrap_or(line) == "+++")
        .nth(1)?
        .0;
    Some(lines.get(second + 2..).unwrap_or_default().concat())
}

/// What the site page `page` lacks against `cla`.
fn site_findings(page: &str, cla: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !page.lines().any(|line| line == SITE_TITLE) {
        out.push(format!("the page's title is not {SITE_TITLE}"));
    }
    match page_body(page) {
        None => out.push("the page has no front matter between two `+++` lines".to_owned()),
        Some(body) if body != cla => out.push("the page does not reproduce CLA.md".to_owned()),
        Some(_) => {}
    }
    out
}

/// What the site footer template `base` lacks.
fn footer_findings(base: &str) -> Vec<String> {
    let mut out = Vec::new();
    let linked: Vec<&str> = base.lines().filter(|l| l.contains("@/cla.md")).collect();
    if linked.is_empty() {
        out.push("the footer has no link to the agreement's page".to_owned());
    }
    let text = format!(">{FOOTER_TEXT}</a>");
    for line in linked {
        if !line.contains(&text) {
            out.push(format!(
                "a link to the agreement's page is not named {FOOTER_TEXT:?}"
            ));
        }
    }
    if base.contains(">Contributor agreement</a>") {
        out.push("the footer still names the page \"Contributor agreement\"".to_owned());
    }
    out
}

/// The words of `ASKING` that `text` holds, compared without case.
fn asking(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    ASKING.into_iter().filter(|w| lower.contains(w)).collect()
}

/// What CONTRIBUTING.md lacks: the DCO stated as the requirement, no word
/// of the switch, and the retired agreement's record.
fn contributing_findings(text: &str, date: &str) -> Vec<String> {
    let mut out = Vec::new();
    let signing = section(text, "Signing off commits");
    if !signing.contains("`DCO` is required") {
        out.push("Signing off commits does not say \"`DCO` is required\"".to_owned());
    }
    for word in ["advisory", "both apply", "switches"] {
        if signing.to_lowercase().contains(word) {
            out.push(format!("Signing off commits still says {word:?}"));
        }
    }
    if text
        .lines()
        .any(|l| l.trim_end() == "## Contributor License Agreement")
    {
        out.push("the section that asks for the agreement is still there".to_owned());
    }
    let retired = section(text, "The retired Contributor License Agreement");
    if retired.trim().is_empty() {
        out.push("no section The retired Contributor License Agreement".to_owned());
    } else {
        let before = format!("Contributions made before {date} were made under it.");
        if !retired.contains(&before) {
            out.push(format!(
                "the retired agreement's section does not say {before:?}"
            ));
        }
        if !retired.contains("(CLA.md)") {
            out.push("the retired agreement's section does not link CLA.md".to_owned());
        }
    }
    out
}

/// The numbered steps of `text` (`1. `, `2. `, ...), each with its
/// indented continuation lines joined by spaces.
fn steps(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let numbered = line
            .split_once(". ")
            .filter(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if let Some((_, rest)) = numbered {
            out.push(rest.trim().to_owned());
        } else if line.starts_with("   ")
            && !line.trim().is_empty()
            && let Some(last) = out.last_mut()
        {
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    out
}

/// What MAINTAINERS.md's "The contributor agreement" lacks: three steps,
/// each done on `date`, and the hook's exemption.
fn maintainers_findings(text: &str, date: &str) -> Vec<String> {
    let agreement = section(text, "The contributor agreement");
    let mut out = Vec::new();
    let found = steps(&agreement);
    if found.len() != 3 {
        out.push(format!("{} steps, not 3", found.len()));
    }
    let done = format!("Done on {date}.");
    for (i, step) in found.iter().enumerate() {
        if !step.starts_with(&done) {
            out.push(format!("step {} does not start with {done:?}", i + 1));
        }
    }
    if !agreement.contains("dco.exempt") {
        out.push("the section does not name `dco.exempt`".to_owned());
    }
    out
}

/// A CLA.md with a good notice over `body`.
fn retired_cla(body: &str) -> String {
    format!(
        "# The agreement\n\n> **Retired on {RETIRED}.** No contributor signs it now.\n> Contributions made before {RETIRED} were made under it. See\n> [Signing off commits](https://example.com/{DCO_ANCHOR}).\n\n{body}"
    )
}

/// A site page with `title` over `body`.
fn site_page(title: &str, body: &str) -> String {
    format!("+++\n{title}\ntemplate = \"page.html\"\n+++\n\n{body}")
}

const GOOD_CONTRIBUTING: &str = "## Signing off commits\n\n`DCO` is required.\n\n## The retired Contributor License Agreement\n\nSee [it](CLA.md). Contributions made before 2026-11-18 were made under it.\n";

const GOOD_MAINTAINERS: &str = "## The contributor agreement\n\n1. Done on 2026-11-18. One.\n2. Done on 2026-11-18. Two.\n3. Done on 2026-11-18. Three.\n\nSet `dco.exempt`.\n";

// Success: the files in the tree.

#[test]
fn cla_md_carries_the_retired_notice() {
    let findings = notice_findings(&read("CLA.md"), RETIRED);
    assert!(findings.is_empty(), "CLA.md: {findings:?}");
}

#[test]
fn the_agreement_below_the_notice_is_the_signed_text() {
    assert_eq!(
        sha256_hex(&without_notice(&read("CLA.md"))),
        SIGNED_SHA256,
        "CLA.md below the notice must be the text contributors signed, unchanged"
    );
}

#[test]
fn the_site_page_reproduces_cla_md_under_a_retired_title() {
    let findings = site_findings(&read("website/content/cla.md"), &read("CLA.md"));
    assert!(findings.is_empty(), "website/content/cla.md: {findings:?}");
}

#[test]
fn the_footer_names_the_retired_agreement() {
    let findings = footer_findings(&read("website/templates/base.html"));
    assert!(findings.is_empty(), "base.html: {findings:?}");
}

#[test]
fn no_page_asks_for_the_agreement() {
    for page in PAGES {
        let found = asking(&read(page));
        assert!(found.is_empty(), "{page} still holds {found:?}");
    }
}

#[test]
fn maintainers_asks_for_nothing_outside_its_record_of_the_switch() {
    let text = read("MAINTAINERS.md");
    let record = section(&text, "The contributor agreement");
    let rest = text.replace(&record, "");
    assert!(
        !record.is_empty(),
        "MAINTAINERS.md has no section The contributor agreement"
    );
    assert!(
        asking(&rest).is_empty(),
        "MAINTAINERS.md: {:?}",
        asking(&rest)
    );
}

#[test]
fn contributing_states_the_dco_as_the_requirement() {
    let findings = contributing_findings(&read("CONTRIBUTING.md"), RETIRED);
    assert!(findings.is_empty(), "CONTRIBUTING.md: {findings:?}");
}

#[test]
fn maintainers_records_the_three_steps_as_done() {
    let findings = maintainers_findings(&read("MAINTAINERS.md"), RETIRED);
    assert!(findings.is_empty(), "MAINTAINERS.md: {findings:?}");
}

#[test]
fn the_best_practices_answer_names_the_sign_off() {
    let text = read(".bestpractices.json");
    let line = text
        .lines()
        .find(|l| l.contains("\"contribution_justification\""))
        .expect(".bestpractices.json has a contribution_justification");
    assert!(
        line.contains("Signed-off-by"),
        "the answer does not name the sign-off: {line}"
    );
}

#[test]
fn the_pages_say_the_commit_hook_refuses_an_unsigned_message() {
    for page in ["README.md", "website/content/docs/development.md"] {
        let text = read(page).replace('\n', " ");
        assert!(
            !text.contains("without refusing, when it has no DCO sign-off")
                && !text.contains("never refuses, when the"),
            "{page} still says the hook only warns"
        );
    }
}

#[test]
fn the_readme_links_signing_off_commits() {
    let text = section(&read("README.md"), "Contributing");
    assert!(
        text.contains("CONTRIBUTING.md#signing-off-commits"),
        "README.md's Contributing does not link Signing off commits"
    );
}

#[test]
fn a_planted_good_set_has_no_findings() {
    let cla = retired_cla("Body.\n");
    assert!(notice_findings(&cla, RETIRED).is_empty(), "the notice");
    assert!(
        site_findings(&site_page(SITE_TITLE, &cla), &cla).is_empty(),
        "the page"
    );
    assert!(
        contributing_findings(GOOD_CONTRIBUTING, RETIRED).is_empty(),
        "CONTRIBUTING"
    );
    assert!(
        maintainers_findings(GOOD_MAINTAINERS, RETIRED).is_empty(),
        "MAINTAINERS"
    );
}

#[test]
fn the_notice_reads_across_its_wrapped_lines() {
    assert_eq!(
        notice("# T\n\n> a\n> b\n\nc\n").as_deref(),
        Some("a b"),
        "the quote's lines join with spaces"
    );
}

// Failure: planted text.

#[test]
fn a_cla_without_a_notice_is_named() {
    assert_eq!(
        notice_findings("# T\n\nBody.\n", RETIRED),
        ["no block quote right under the title"],
        "a CLA.md with no quote"
    );
}

#[test]
fn a_notice_with_another_date_is_named() {
    let cla =
        retired_cla("Body.\n").replace(&format!("Retired on {RETIRED}"), "Retired on 2026-11-19");
    assert_eq!(
        notice_findings(&cla, RETIRED),
        [format!(
            "the notice does not start with **Retired on {RETIRED}.**"
        )],
        "another date"
    );
}

#[test]
fn a_notice_without_the_before_sentence_is_named() {
    let cla = retired_cla("Body.\n").replace(" were made under it.", " are kept.");
    let findings = notice_findings(&cla, RETIRED);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].contains("were made under it"), "{findings:?}");
}

#[test]
fn a_notice_without_the_dco_link_is_named() {
    let cla = retired_cla("Body.\n").replace(DCO_ANCHOR, "CONTRIBUTING.md");
    assert_eq!(
        notice_findings(&cla, RETIRED),
        [format!("the notice does not link {DCO_ANCHOR}")],
        "no link"
    );
}

#[test]
fn one_changed_word_of_the_agreement_changes_its_hash() {
    let cla = read("CLA.md").replacen("Agreement", "Agreemant", 1);
    assert_ne!(
        sha256_hex(&without_notice(&cla)),
        SIGNED_SHA256,
        "a changed word must not match the signed text"
    );
}

#[test]
fn a_site_page_that_differs_by_one_character_is_named() {
    let cla = retired_cla("Body.\n");
    let page = site_page(SITE_TITLE, &cla.replace("Body.", "Body!"));
    assert_eq!(
        site_findings(&page, &cla),
        ["the page does not reproduce CLA.md"],
        "one character"
    );
}

#[test]
fn a_site_page_with_the_old_title_is_named() {
    let cla = retired_cla("Body.\n");
    let page = site_page(r#"title = "Contributor License Agreement""#, &cla);
    assert_eq!(
        site_findings(&page, &cla),
        [format!("the page's title is not {SITE_TITLE}")],
        "the old title"
    );
}

#[test]
fn a_site_page_without_front_matter_is_named() {
    let cla = retired_cla("Body.\n");
    let findings = site_findings(&format!("{SITE_TITLE}\n\n{cla}"), &cla);
    assert_eq!(
        findings,
        ["the page has no front matter between two `+++` lines"],
        "no front matter"
    );
}

#[test]
fn the_old_footer_text_is_named() {
    let base = r#"<li><a href="{{ get_url(path='@/cla.md') }}">Contributor agreement</a></li>"#;
    let findings = footer_findings(base);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(findings[1].contains("still names"), "{findings:?}");
}

#[test]
fn a_footer_without_the_agreement_link_is_named() {
    assert_eq!(
        footer_findings("<li><a href=\"/\">Home</a></li>"),
        ["the footer has no link to the agreement's page"],
        "no link"
    );
}

#[test]
fn each_asking_word_is_named() {
    for word in ASKING {
        assert_eq!(asking(&format!("Text {word} text.")), [word], "{word:?}");
    }
}

#[test]
fn contributing_that_still_calls_the_dco_advisory_is_named() {
    let text = GOOD_CONTRIBUTING.replace("`DCO` is required.", "`DCO` is advisory today.");
    let findings = contributing_findings(&text, RETIRED);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(findings[1].contains("advisory"), "{findings:?}");
}

#[test]
fn contributing_with_the_old_heading_is_named() {
    let text = format!("{GOOD_CONTRIBUTING}\n## Contributor License Agreement\n\nSign it.\n");
    assert_eq!(
        contributing_findings(&text, RETIRED),
        ["the section that asks for the agreement is still there"],
        "the old section"
    );
}

#[test]
fn contributing_without_the_retired_section_is_named() {
    let text = "## Signing off commits\n\n`DCO` is required.\n";
    assert_eq!(
        contributing_findings(text, RETIRED),
        ["no section The retired Contributor License Agreement"],
        "no retired section"
    );
}

#[test]
fn a_step_not_done_is_named() {
    let text = GOOD_MAINTAINERS.replace("2. Done on 2026-11-18. Two.", "2. Two.");
    assert_eq!(
        maintainers_findings(&text, RETIRED),
        [format!("step 2 does not start with \"Done on {RETIRED}.\"")],
        "step 2 not done"
    );
}

#[test]
fn a_step_done_on_another_day_is_named() {
    let text = GOOD_MAINTAINERS.replace("3. Done on 2026-11-18.", "3. Done on 2026-11-17.");
    assert_eq!(
        maintainers_findings(&text, RETIRED),
        [format!("step 3 does not start with \"Done on {RETIRED}.\"")],
        "another day"
    );
}

#[test]
fn a_notice_whose_lead_is_not_first_is_named() {
    let cla = retired_cla("Body.\n").replace(
        &format!("> **Retired on {RETIRED}.**"),
        &format!("> Note. **Retired on {RETIRED}.**"),
    );
    assert_eq!(
        notice_findings(&cla, RETIRED),
        [format!(
            "the notice does not start with **Retired on {RETIRED}.**"
        )],
        "the lead comes first"
    );
}

#[test]
fn two_steps_are_named() {
    let text = GOOD_MAINTAINERS.replace("3. Done on 2026-11-18. Three.\n", "");
    assert_eq!(
        maintainers_findings(&text, RETIRED),
        ["2 steps, not 3"],
        "two steps"
    );
}

#[test]
fn a_record_without_the_exemption_is_named() {
    let text = GOOD_MAINTAINERS.replace("Set `dco.exempt`.", "");
    assert_eq!(
        maintainers_findings(&text, RETIRED),
        ["the section does not name `dco.exempt`"],
        "no exemption"
    );
}

// Edge cases.

#[test]
fn a_quote_after_a_paragraph_is_not_the_notice() {
    let cla = format!("# T\n\nText.\n\n> **Retired on {RETIRED}.**\n");
    assert_eq!(notice(&cla), None, "a quote further down is not the notice");
}

#[test]
fn a_title_that_is_not_the_first_line_has_no_notice() {
    assert_eq!(notice("\n# T\n\n> a\n"), None, "a blank first line");
}

#[test]
fn blank_lines_before_the_notice_are_skipped() {
    assert_eq!(
        notice("# T\n\n\n> a\n").as_deref(),
        Some("a"),
        "two blank lines"
    );
}

#[test]
fn asking_words_compare_without_case() {
    assert_eq!(
        asking("Wait for LICENSE/CLA."),
        ["license/cla"],
        "upper case"
    );
}

#[test]
fn a_crlf_delimiter_is_not_front_matter() {
    let cla = retired_cla("Body.\n");
    let page = format!("+++\r\n{SITE_TITLE}\r\n+++\r\n\r\n{cla}");
    assert!(
        site_findings(&page, &cla)
            .contains(&"the page has no front matter between two `+++` lines".to_owned()),
        "`+++\\r` is not a delimiter"
    );
}

#[test]
fn a_cla_without_its_last_newline_does_not_match_the_page() {
    let cla = retired_cla("Body.");
    let page = site_page(SITE_TITLE, &format!("{cla}\n"));
    assert_eq!(
        site_findings(&page, &cla),
        ["the page does not reproduce CLA.md"],
        "byte for byte"
    );
}

#[test]
fn a_text_without_a_notice_is_kept_whole() {
    let text = "# T\n\nBody.\n\nMore.\n";
    assert_eq!(without_notice(text), text, "nothing to remove");
}

#[test]
fn the_notice_and_its_blank_line_are_removed() {
    assert_eq!(
        without_notice(&retired_cla("Body.\n\nMore.\n")),
        "# The agreement\n\nBody.\n\nMore.\n",
        "the text before the notice was added"
    );
}

#[test]
fn a_step_wrapped_on_indented_lines_is_one_step() {
    let text = "## The contributor agreement\n\n1. Done on 2026-11-18.\n   More.\n2. Done on 2026-11-18. Two.\n3. Done on\n   2026-11-18. Three.\n\n`dco.exempt`\n";
    assert_eq!(
        steps(&section(text, "The contributor agreement")),
        [
            "Done on 2026-11-18. More.",
            "Done on 2026-11-18. Two.",
            "Done on 2026-11-18. Three."
        ],
        "continuation lines join"
    );
    assert!(
        maintainers_findings(text, RETIRED).is_empty(),
        "a wrapped date still counts"
    );
}

#[test]
fn a_dated_line_that_is_not_a_step_is_not_counted() {
    assert_eq!(
        steps("Version 1. Done.\n- 2. Done.\n"),
        Vec::<String>::new(),
        "not numbered steps"
    );
}

#[test]
fn the_dco_requirement_elsewhere_does_not_count() {
    let text = GOOD_CONTRIBUTING.replace("`DCO` is required.", "Add the line.")
        + "\n## Other\n\n`DCO` is required.\n";
    assert_eq!(
        contributing_findings(&text, RETIRED),
        ["Signing off commits does not say \"`DCO` is required\""],
        "the requirement belongs in Signing off commits"
    );
}

#[test]
fn a_footer_link_with_other_words_is_named() {
    let base =
        r#"<li><a href="{{ get_url(path='@/cla.md') }}">Contributor agreement (retired)</a></li>"#;
    assert_eq!(
        footer_findings(base),
        [format!(
            "a link to the agreement's page is not named {FOOTER_TEXT:?}"
        )],
        "other words"
    );
}

#[test]
fn an_empty_text_has_no_notice_and_nothing_to_remove() {
    assert_eq!(notice(""), None, "empty");
    assert_eq!(without_notice(""), "", "empty");
}
