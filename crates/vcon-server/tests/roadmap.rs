//! The roadmap (`website/content/docs/roadmap.md`) states what vcon-server
//! plans to do up to a horizon month, and what it does not plan to do.
//! OpenSSF's silver criterion `documentation_roadmap` asks for both, for at
//! least the next year. These tests hold the page to that shape:
//!
//! - one sentence names the horizon, "The plan on this page runs through
//!   <Month> <year>.", and the horizon has not passed;
//! - the page says that each month is a target, not a promise;
//! - the `Milestones` section lists each milestone as a bullet whose bold
//!   lead starts with its target month, in month order, none after the
//!   horizon, each with a `Status:` sub-bullet;
//! - the `Not planned` section lists at least one thing, each as a bullet
//!   with a bold lead;
//! - every link to a page of the site leads to a page, and its anchor to a
//!   heading of that page.
//!
//! The readers are pure functions over the page's text, today's month and
//! a [`Pages`] lookup, so their own tests drive them with pages, months and
//! sites made here. Only [`today`] reads the clock.

mod markdown;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use markdown::{heading_ids, is_relative, link_targets};

/// The page, from the repository root.
const PAGE: &str = "website/content/docs/roadmap.md";

/// The sentence that names the horizon, up to the month.
const HORIZON: &str = "The plan on this page runs through ";

/// The words that say a month is a target.
const TARGET: &str = "is a target, not a promise";

/// The section that lists the milestones.
const MILESTONES: &str = "Milestones";

/// The section that lists what the project does not plan to do.
const NOT_PLANNED: &str = "Not planned";

/// The names of the months, January first.
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A month of a year. The fields are in this order so that the derived
/// order is the calendar's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Month {
    year: i64,
    month: u32,
}

const fn month(year: i64, month: u32) -> Month {
    Month { year, month }
}

/// One milestone, as the page lists it.
#[derive(Debug, PartialEq, Eq)]
struct Milestone {
    month: Month,
    has_status: bool,
}

/// The pages of the site, by the path a link names after `@/`.
trait Pages {
    fn text(&self, path: &str) -> Option<String>;
}

/// The site in the checked-out tree.
struct Site(PathBuf);

impl Pages for Site {
    fn text(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join("website/content").join(path)).ok()
    }
}

/// A site made in a test.
struct Fake(BTreeMap<&'static str, &'static str>);

impl Pages for Fake {
    fn text(&self, path: &str) -> Option<String> {
        self.0.get(path).map(|text| (*text).to_string())
    }
}

/// The repository root, two levels above this crate.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The text with every run of white space made one space, so that a
/// sentence wrapped over several lines reads as one.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `<Month> <year>`, the month's full name and a four-digit year, and
/// nothing else.
fn parse_month(text: &str) -> Option<Month> {
    let (name, year) = text.split_once(' ')?;
    let index = MONTHS.iter().position(|m| *m == name)?;
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = u32::try_from(index + 1).ok()?;
    Some(month(year.parse().ok()?, number))
}

/// The horizon the page names, from its one horizon sentence.
fn horizon(page: &str) -> Result<Month, String> {
    let text = flat(page);
    let found: Vec<_> = text.match_indices(HORIZON).collect();
    let [(at, _)] = found.as_slice() else {
        return Err(format!(
            "the page has {} horizon sentences (`{HORIZON}<Month> <year>.`), not one",
            found.len()
        ));
    };
    let rest = &text[at + HORIZON.len()..];
    let end = rest
        .find('.')
        .ok_or("the horizon sentence has no full stop")?;
    parse_month(&rest[..end])
        .ok_or_else(|| format!("the horizon `{}` is not `<Month> <year>`", &rest[..end]))
}

/// The lines under the `## <heading>` line, up to the next `## ` line.
fn section<'a>(page: &'a str, heading: &str) -> Option<&'a str> {
    let line = format!("## {heading}");
    let mut offset = 0;
    for text in page.split_inclusive('\n') {
        offset += text.len();
        if text.trim_end() == line {
            let rest = &page[offset..];
            let end = rest
                .match_indices("\n## ")
                .next()
                .map_or(rest.len(), |(at, _)| at + 1);
            let end = if rest.starts_with("## ") { 0 } else { end };
            return Some(&rest[..end]);
        }
    }
    None
}

/// The top-level bullets of a section, each with its wrapped lines and
/// sub-bullets.
fn bullets(section: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in section.lines() {
        if let Some(text) = line.strip_prefix("- ") {
            found.push(text.to_string());
        } else if line.starts_with(' ')
            && let Some(last) = found.last_mut()
        {
            last.push('\n');
            last.push_str(line);
        }
    }
    found
}

/// The milestones of the `Milestones` section, each read from its bold
/// lead `**<Month> <year>: ...**` and its `Status:` sub-bullet.
fn milestones(section: &str) -> Result<Vec<Milestone>, String> {
    let mut found = Vec::new();
    for bullet in bullets(section) {
        let first = bullet.lines().next().unwrap_or_default();
        let lead = first
            .strip_prefix("**")
            .and_then(|rest| rest.split_once(':'))
            .map(|(when, _)| when);
        let Some(when) = lead.and_then(parse_month) else {
            return Err(format!(
                "the milestone `{first}` does not start with `**<Month> <year>:`"
            ));
        };
        let has_status = bullet
            .lines()
            .skip(1)
            .any(|line| line.trim_start().starts_with("- Status:"));
        found.push(Milestone {
            month: when,
            has_status,
        });
    }
    Ok(found)
}

/// What the page breaks of the rules above, on `today`'s month.
fn problems(page: &str, today: Month) -> Vec<String> {
    let mut found = Vec::new();
    let until = match horizon(page) {
        Ok(until) => {
            if until < today {
                found.push(format!(
                    "the horizon {until:?} has passed ({today:?}): move it at least a year ahead"
                ));
            }
            Some(until)
        }
        Err(problem) => {
            found.push(problem);
            None
        }
    };
    if !flat(page).contains(TARGET) {
        found.push(format!("the page does not say each month `{TARGET}`"));
    }
    match section(page, MILESTONES).map(milestones) {
        None => found.push(format!("the page has no `## {MILESTONES}` section")),
        Some(Err(problem)) => found.push(problem),
        Some(Ok(list)) if list.is_empty() => {
            found.push(format!("the `{MILESTONES}` section lists no milestone"));
        }
        Some(Ok(list)) => {
            for pair in list.windows(2) {
                if pair[1].month < pair[0].month {
                    found.push(format!(
                        "{:?} is listed after {:?}: the milestones are in month order",
                        pair[1].month, pair[0].month
                    ));
                }
            }
            for milestone in &list {
                if until.is_some_and(|until| milestone.month > until) {
                    found.push(format!(
                        "the milestone of {:?} is after the horizon",
                        milestone.month
                    ));
                }
                if !milestone.has_status {
                    found.push(format!(
                        "the milestone of {:?} has no `Status:` sub-bullet",
                        milestone.month
                    ));
                }
            }
        }
    }
    match section(page, NOT_PLANNED).map(bullets) {
        None => found.push(format!("the page has no `## {NOT_PLANNED}` section")),
        Some(list) if list.is_empty() => {
            found.push(format!("the `{NOT_PLANNED}` section lists nothing"));
        }
        Some(list) => {
            for bullet in list {
                if !bullet.starts_with("**") {
                    found.push(format!(
                        "the `{NOT_PLANNED}` entry `{bullet}` has no bold lead"
                    ));
                }
            }
        }
    }
    found
}

/// The links of `page` to the site that lead nowhere: an `@/` page the
/// site does not have, or an anchor that is not a heading of its page.
fn broken_links(page: &str, site: &dyn Pages) -> Vec<String> {
    let mut found = Vec::new();
    for target in link_targets(page) {
        let (path, anchor) = target.split_once('#').unwrap_or((&target, ""));
        let text = if path.is_empty() {
            Some(page.to_string())
        } else if let Some(path) = path.strip_prefix("@/") {
            site.text(path)
        } else if is_relative(&target) {
            found.push(format!(
                "{target}: a relative link; the site links its pages as `@/docs/<page>.md`"
            ));
            continue;
        } else {
            continue;
        };
        match text {
            None => found.push(format!("{target}: the site has no such page")),
            Some(text) => {
                if !anchor.is_empty() && !heading_ids(&text).contains(anchor) {
                    found.push(format!("{target}: the page has no such heading"));
                }
            }
        }
    }
    found
}

/// The civil date of a day counted from 1970-01-01 (day 0), as
/// (year, month, day), by Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).expect("a month is 1 to 12"),
        u32::try_from(day).expect("a day is 1 to 31"),
    )
}

/// This month, in UTC, from the clock.
fn today() -> Month {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs();
    let days = i64::try_from(seconds / 86_400).expect("days fit an i64");
    let (year, number, _) = civil_from_days(days);
    month(year, number)
}

/// A page that keeps every rule, for the readers' own tests to change.
const GOOD: &str = "+++
title = \"Roadmap\"
+++

The plan on this page runs through December
2027. Each month below is a target, not a
promise.

## Milestones

- **January 2027: the first.**
  - Work: one thing.
  - Status: [a page](@/docs/a.md#a-heading).
- **January 2027: the second.**
  - Status: none yet.
- **September 2027: the last.**
  - Work: a thing that wraps
    over two lines.
  - Status: [this page](#not-planned).

## Not planned

- **A new name.** The name stays.
- **Windows builds.** None.
";

fn good_site() -> Fake {
    Fake(BTreeMap::from([("docs/a.md", "# Title\n\n## A heading\n")]))
}

const NOW: Month = month(2026, 11);

fn read_page() -> String {
    std::fs::read_to_string(root().join(PAGE)).expect("the roadmap page exists")
}

// The page itself.

#[test]
fn the_roadmap_keeps_every_rule_this_month() {
    let found = problems(&read_page(), today());
    assert!(found.is_empty(), "{PAGE}: {found:#?}");
}

#[test]
fn every_site_link_of_the_roadmap_leads_to_a_page_and_heading() {
    let found = broken_links(&read_page(), &Site(root()));
    assert!(found.is_empty(), "{PAGE}: {found:#?}");
}

#[test]
fn the_roadmap_horizon_is_at_least_a_year_after_the_plan_was_written() {
    // The plan of 2026-11-15 is the page's source; OpenSSF asks for at
    // least the next year from when the page is entered (2026-11).
    let until = horizon(&read_page()).expect("the page names its horizon");
    assert!(
        until >= month(2027, 11),
        "the horizon {until:?} is less than a year after 2026-11"
    );
}

// Success.

#[test]
fn a_page_with_every_rule_has_no_problem() {
    assert_eq!(
        problems(GOOD, NOW),
        Vec::<String>::new(),
        "GOOD keeps every rule"
    );
}

#[test]
fn a_page_with_every_rule_has_no_broken_link() {
    assert_eq!(
        broken_links(GOOD, &good_site()),
        Vec::<String>::new(),
        "GOOD's links lead to the fake site's pages"
    );
}

#[test]
fn parse_month_reads_january() {
    assert_eq!(
        parse_month("January 2027"),
        Some(month(2027, 1)),
        "January is 1"
    );
}

#[test]
fn parse_month_reads_december() {
    assert_eq!(
        parse_month("December 2027"),
        Some(month(2027, 12)),
        "December is 12"
    );
}

#[test]
fn parse_month_reads_every_month_name_in_order() {
    for (index, name) in MONTHS.iter().enumerate() {
        let number = u32::try_from(index + 1).expect("a small index");
        assert_eq!(
            parse_month(&format!("{name} 2030")),
            Some(month(2030, number)),
            "{name} is month {number}"
        );
    }
}

#[test]
fn horizon_reads_a_sentence_wrapped_over_lines() {
    assert_eq!(
        horizon(GOOD),
        Ok(month(2027, 12)),
        "the month and year are on two lines"
    );
}

#[test]
fn milestones_reads_each_month_and_status() {
    let section = section(GOOD, MILESTONES).expect("GOOD has the section");
    assert_eq!(
        milestones(section),
        Ok(vec![
            Milestone {
                month: month(2027, 1),
                has_status: true
            },
            Milestone {
                month: month(2027, 1),
                has_status: true
            },
            Milestone {
                month: month(2027, 9),
                has_status: true
            },
        ]),
        "three milestones, each with a status"
    );
}

#[test]
fn section_stops_at_the_next_section() {
    let text = section(GOOD, MILESTONES).expect("GOOD has the section");
    assert!(
        !text.contains("Windows"),
        "the Milestones section ends at Not planned"
    );
    assert!(
        text.contains("the last"),
        "the Milestones section holds its last bullet"
    );
}

#[test]
fn the_last_section_runs_to_the_end_of_the_page() {
    let text = section(GOOD, NOT_PLANNED).expect("GOOD has the section");
    assert_eq!(bullets(text).len(), 2, "both Not planned entries are read");
}

#[test]
fn months_order_by_year_before_month() {
    assert!(
        month(2027, 1) > month(2026, 12),
        "January 2027 is after December 2026"
    );
    assert!(month(2027, 2) > month(2027, 1), "February is after January");
}

#[test]
fn civil_from_days_reads_the_epoch() {
    assert_eq!(civil_from_days(0), (1970, 1, 1), "day 0 is 1970-01-01");
}

#[test]
fn civil_from_days_reads_the_day_the_page_was_dated() {
    // `date -u -d 2026-11-24 +%s` / 86400 = 20781.
    assert_eq!(
        civil_from_days(20_781),
        (2026, 11, 24),
        "day 20781 is 2026-11-24"
    );
}

#[test]
fn civil_from_days_reads_a_day_after_a_century_leap_year() {
    // 2000 is a leap year by the 400-year rule; 2000-03-01 is day 11017.
    assert_eq!(
        civil_from_days(11_017),
        (2000, 3, 1),
        "day 11017 is 2000-03-01"
    );
}

#[test]
fn today_is_a_month_of_this_century() {
    let now = today();
    assert!(
        (1..=12).contains(&now.month),
        "the month {now:?} is 1 to 12"
    );
    assert!(now.year >= 2026, "the year {now:?} is not before this page");
}

#[test]
fn an_external_link_is_not_checked_against_the_site() {
    let page = "[the history](https://github.com/NormB/vcon-server/commits/main)";
    assert_eq!(
        broken_links(page, &good_site()),
        Vec::<String>::new(),
        "an https link is not a page of the site"
    );
}

// Failure.

#[test]
fn a_page_without_a_horizon_is_refused() {
    let page = GOOD.replace(HORIZON, "The plan runs through ");
    let found = problems(&page, NOW);
    assert!(
        found.iter().any(|p| p.contains("0 horizon sentences")),
        "a missing horizon is named: {found:#?}"
    );
}

#[test]
fn a_page_with_two_horizons_is_refused() {
    let page = format!("{GOOD}\n{HORIZON}May 2028.\n");
    assert!(
        horizon(&page).is_err_and(|p| p.contains("2 horizon sentences")),
        "two horizon sentences are refused"
    );
}

#[test]
fn a_horizon_that_has_passed_is_refused() {
    let found = problems(GOOD, month(2028, 3));
    assert!(
        found.iter().any(|p| p.contains("has passed")),
        "a past horizon is named: {found:#?}"
    );
}

#[test]
fn a_horizon_that_is_not_a_month_is_refused() {
    let page = GOOD.replace("December\n2027", "late 2027");
    assert!(
        horizon(&page).is_err_and(|p| p.contains("is not `<Month> <year>`")),
        "a horizon that is not a month is refused"
    );
}

#[test]
fn a_horizon_sentence_without_a_full_stop_is_refused() {
    let page = "The plan on this page runs through December 2027";
    assert!(
        horizon(page).is_err_and(|p| p.contains("no full stop")),
        "a horizon sentence that never ends is refused"
    );
}

#[test]
fn a_page_that_does_not_call_the_months_targets_is_refused() {
    let page = GOOD.replace("a target, not a\npromise", "the date");
    let found = problems(&page, NOW);
    assert!(
        found.iter().any(|p| p.contains(TARGET)),
        "the missing target sentence is named: {found:#?}"
    );
}

#[test]
fn a_page_without_milestones_is_refused() {
    let page = GOOD.replace("## Milestones", "## Plans");
    let found = problems(&page, NOW);
    assert!(
        found
            .iter()
            .any(|p| p.contains("no `## Milestones` section")),
        "a missing Milestones section is named: {found:#?}"
    );
}

#[test]
fn a_milestone_without_a_month_is_refused() {
    let page = GOOD.replace("**September 2027: the last.**", "**The last.**");
    let found = problems(&page, NOW);
    assert!(
        found
            .iter()
            .any(|p| p.contains("does not start with `**<Month> <year>:`")),
        "a milestone with no month is named: {found:#?}"
    );
}

#[test]
fn milestones_out_of_month_order_are_refused() {
    let page = GOOD.replace(
        "**January 2027: the second.**",
        "**October 2027: the second.**",
    );
    let found = problems(&page, NOW);
    assert!(
        found.iter().any(|p| p.contains("are in month order")),
        "an out-of-order milestone is named: {found:#?}"
    );
}

#[test]
fn a_milestone_after_the_horizon_is_refused() {
    let page = GOOD.replace("December\n2027", "August\n2027");
    let found = problems(&page, NOW);
    assert!(
        found.iter().any(|p| p.contains("is after the horizon")),
        "a milestone past the horizon is named: {found:#?}"
    );
}

#[test]
fn a_milestone_without_a_status_is_refused() {
    let page = GOOD.replace("  - Status: none yet.\n", "");
    let found = problems(&page, NOW);
    assert!(
        found
            .iter()
            .any(|p| p.contains("has no `Status:` sub-bullet")),
        "a milestone with no status is named: {found:#?}"
    );
}

#[test]
fn a_page_without_a_not_planned_section_is_refused() {
    let page = GOOD.replace("## Not planned", "## Later");
    let found = problems(&page, NOW);
    assert!(
        found
            .iter()
            .any(|p| p.contains("no `## Not planned` section")),
        "a missing Not planned section is named: {found:#?}"
    );
}

#[test]
fn an_empty_not_planned_section_is_refused() {
    let at = GOOD.find("- **A new name.**").expect("GOOD has the entry");
    let page = &GOOD[..at];
    let found = problems(page, NOW);
    assert!(
        found.iter().any(|p| p.contains("section lists nothing")),
        "an empty Not planned section is named: {found:#?}"
    );
}

#[test]
fn a_not_planned_entry_without_a_bold_lead_is_refused() {
    let page = GOOD.replace("- **Windows builds.** None.", "- Windows builds.");
    let found = problems(&page, NOW);
    assert!(
        found.iter().any(|p| p.contains("has no bold lead")),
        "an entry with no lead is named: {found:#?}"
    );
}

#[test]
fn a_link_to_a_page_the_site_lacks_is_refused() {
    let found = broken_links("[gone](@/docs/gone.md)", &good_site());
    assert_eq!(
        found,
        vec!["@/docs/gone.md: the site has no such page".to_string()],
        "a missing page is named"
    );
}

#[test]
fn a_link_to_a_heading_the_page_lacks_is_refused() {
    let found = broken_links("[gone](@/docs/a.md#gone)", &good_site());
    assert_eq!(
        found,
        vec!["@/docs/a.md#gone: the page has no such heading".to_string()],
        "a missing heading is named"
    );
}

#[test]
fn a_relative_link_is_refused() {
    let found = broken_links("[a](status.md)", &good_site());
    assert_eq!(
        found,
        vec![
            "status.md: a relative link; the site links its pages as `@/docs/<page>.md`"
                .to_string()
        ],
        "a relative link is named"
    );
}

#[test]
fn a_misspelled_month_is_not_read() {
    assert_eq!(
        parse_month("Febuary 2027"),
        None,
        "a misspelled name is not a month"
    );
}

#[test]
fn a_two_digit_year_is_not_read() {
    assert_eq!(parse_month("January 27"), None, "a year has four digits");
}

#[test]
fn a_short_month_name_is_not_read() {
    assert_eq!(
        parse_month("Jan 2027"),
        None,
        "the page names months in full"
    );
}

// Edge.

#[test]
fn a_horizon_in_this_month_has_not_passed() {
    let found = problems(GOOD, month(2027, 12));
    assert!(
        found.is_empty(),
        "the horizon month itself is still covered: {found:#?}"
    );
}

#[test]
fn a_horizon_in_last_december_has_passed_in_january() {
    let found = problems(GOOD, month(2028, 1));
    assert!(
        found.iter().any(|p| p.contains("has passed")),
        "the year boundary is crossed: {found:#?}"
    );
}

#[test]
fn a_milestone_in_the_horizon_month_is_allowed() {
    let page = GOOD.replace("December\n2027", "September\n2027");
    let found = problems(&page, NOW);
    assert!(
        found.is_empty(),
        "a milestone in the horizon month is inside it: {found:#?}"
    );
}

#[test]
fn two_milestones_in_one_month_keep_their_order() {
    let section = section(GOOD, MILESTONES).expect("GOOD has the section");
    let list = milestones(section).expect("GOOD's milestones read");
    assert_eq!(
        list[0].month, list[1].month,
        "two milestones share January 2027"
    );
    assert!(problems(GOOD, NOW).is_empty(), "a shared month is in order");
}

#[test]
fn a_status_of_the_next_milestone_is_not_counted_for_this_one() {
    let page = GOOD.replace("  - Status: [a page](@/docs/a.md#a-heading).\n", "");
    let section = section(&page, MILESTONES).expect("the page has the section");
    let list = milestones(section).expect("the milestones read");
    assert!(!list[0].has_status, "the first milestone lost its status");
    assert!(list[1].has_status, "the second keeps its own");
}

#[test]
fn a_wrapped_line_of_a_milestone_is_not_a_milestone() {
    let section = section(GOOD, MILESTONES).expect("GOOD has the section");
    assert_eq!(
        milestones(section).map(|list| list.len()),
        Ok(3),
        "the wrapped `over two lines.` line belongs to the last milestone"
    );
}

#[test]
fn a_subheading_does_not_end_a_section() {
    let page = GOOD.replace("- **Windows builds.**", "### More\n\n- **Windows builds.**");
    let text = section(&page, NOT_PLANNED).expect("the page has the section");
    assert_eq!(
        bullets(text).len(),
        2,
        "a ### heading stays inside the section"
    );
}

#[test]
fn a_heading_with_trailing_spaces_is_found() {
    let page = GOOD.replace("## Not planned\n", "## Not planned  \n");
    assert!(
        section(&page, NOT_PLANNED).is_some(),
        "trailing spaces are ignored"
    );
}

#[test]
fn a_heading_that_only_starts_with_the_name_is_not_the_section() {
    let page = GOOD.replace("## Not planned\n", "## Not planned yet\n");
    assert!(
        section(&page, NOT_PLANNED).is_none(),
        "the heading must match whole"
    );
}

#[test]
fn an_anchor_on_this_page_is_checked_against_its_own_headings() {
    assert_eq!(
        broken_links("[up](#milestones)\n\n## Milestones\n", &good_site()),
        Vec::<String>::new(),
        "an anchor of this page's heading leads somewhere"
    );
    assert_eq!(
        broken_links("[up](#gone)\n", &good_site()),
        vec!["#gone: the page has no such heading".to_string()],
        "an anchor this page lacks is named"
    );
}

#[test]
fn a_page_link_without_an_anchor_needs_only_the_page() {
    assert_eq!(
        broken_links("[a](@/docs/a.md)", &good_site()),
        Vec::<String>::new(),
        "a page that exists is enough"
    );
}

#[test]
fn civil_from_days_reads_a_leap_day() {
    // 2024-02-29 is day 19782.
    assert_eq!(
        civil_from_days(19_782),
        (2024, 2, 29),
        "day 19782 is 2024-02-29"
    );
}

#[test]
fn civil_from_days_crosses_a_year_end() {
    // 2026-12-31 is day 20818 and 2027-01-01 day 20819.
    assert_eq!(
        civil_from_days(20_818),
        (2026, 12, 31),
        "the last day of 2026"
    );
    assert_eq!(
        civil_from_days(20_819),
        (2027, 1, 1),
        "the first day of 2027"
    );
}

#[test]
fn civil_from_days_reads_a_day_before_the_epoch() {
    assert_eq!(civil_from_days(-1), (1969, 12, 31), "day -1 is 1969-12-31");
}

#[test]
fn a_month_followed_by_more_words_is_not_read() {
    assert_eq!(
        parse_month("January 2027 or later"),
        None,
        "only `<Month> <year>`"
    );
}

#[test]
fn a_lower_case_month_is_not_read() {
    assert_eq!(
        parse_month("january 2027"),
        None,
        "month names are capitalized"
    );
}
