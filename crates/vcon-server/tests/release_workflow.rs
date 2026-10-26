//! The crates.io job in .github/workflows/release.yml: what it may publish,
//! when, and with which credential. crates.io versions can never be
//! replaced, so these hold the parts a careless edit could break.

use std::path::Path;

fn release_yml() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/release.yml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The lines of the job `name` under `jobs:`, up to the next job.
fn job(yml: &str, name: &str) -> String {
    let header = format!("  {name}:");
    let mut lines = yml.lines().skip_while(|line| *line != header);
    assert!(lines.next().is_some(), "no job {name}");
    lines
        .take_while(|line| {
            let next_job = line.starts_with("  ")
                && !line.starts_with("   ")
                && !line.trim_start().starts_with('#');
            !next_job
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The non-comment lines of `text`, so a comment cannot satisfy a check.
fn code(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_crates_io_job_authenticates_by_trusted_publishing_only() {
    let job = code(&job(&release_yml(), "crates-io"));
    assert!(
        job.contains("id-token: write"),
        "OIDC is needed for the token exchange"
    );
    assert!(job.contains("contents: read"));
    assert!(
        job.contains(
            "uses: rust-lang/crates-io-auth-action@3268eb413133e2960fa4ae0e963ca76380c54cd9"
        ),
        "the auth action, pinned by the commit of v1.0.6"
    );
    assert!(
        job.contains("CARGO_REGISTRY_TOKEN: ${{ steps.auth.outputs.token }}"),
        "the only token is the one the exchange returns"
    );
    assert!(!job.contains("secrets."), "no stored crates.io secret");
}

#[test]
fn the_crates_io_job_matches_the_trusted_publisher_on_crates_io() {
    // crates.io's trusted publisher for both crates names release.yml and no
    // environment; a job with an environment would be refused.
    let job = job(&release_yml(), "crates-io");
    assert!(!code(&job).contains("environment:"));
    assert!(
        job.contains("crates-io") && job.contains("environment"),
        "a comment says the environment to set once crates.io names it"
    );
}

#[test]
fn a_tag_publishes_only_after_the_github_release_and_never_twice() {
    let job = code(&job(&release_yml(), "crates-io"));
    assert!(
        job.contains("needs: [version, publish]"),
        "after the GitHub release"
    );
    assert!(
        job.contains("needs.publish.result == 'success'"),
        "a tag publishes only when the GitHub release succeeded"
    );
    let vcon = job.find("vcon vcon-server").expect("both crates, in order");
    assert!(vcon > 0);
    assert!(
        job.contains("https://crates.io/api/v1/crates/${crate}/${VERSION}"),
        "each version is looked up on crates.io before an upload"
    );
    assert!(
        job.contains("already on crates.io"),
        "a version there is skipped"
    );
    assert!(job.contains("cargo publish --locked -p \"$crate\""));
}

#[test]
fn a_run_by_hand_is_always_a_dry_run() {
    let yml = release_yml();
    let on = &yml[..yml.find("\njobs:").expect("jobs")];
    let input = &on[on
        .find("crates_io_dry_run:")
        .expect("a crates_io_dry_run input")..];
    assert!(
        input.contains("default: true"),
        "a run by hand includes the dry run"
    );
    let job = code(&job(&yml, "crates-io"));
    assert!(
        job.contains("github.event_name == 'workflow_dispatch' && inputs.crates_io_dry_run"),
        "by hand, the job runs only when asked"
    );
    assert!(
        job.contains("DRY_RUN: ${{ github.event_name != 'push' }}"),
        "only a tag push uploads"
    );
    assert!(job.contains("cargo publish --dry-run --locked -p \"$crate\""));
}

#[test]
fn the_token_exchange_is_checked_and_the_token_never_printed() {
    let job = code(&job(&release_yml(), "crates-io"));
    let check = &job[job
        .find("Check the token exchange")
        .expect("a step checks the exchange")..];
    assert!(
        check.contains("TOKEN: ${{ steps.auth.outputs.token }}"),
        "the step reads the token the exchange returned"
    );
    assert!(
        check.contains("[ -n \"$TOKEN\" ]"),
        "an empty token fails the job"
    );
    assert!(
        check.contains("${#TOKEN}"),
        "the log says only how long it is"
    );
    for leak in [
        "echo \"$TOKEN\"",
        "echo $TOKEN",
        "${TOKEN}",
        "echo \"$CARGO_REGISTRY_TOKEN\"",
    ] {
        assert!(!job.contains(leak), "the token is never printed: {leak}");
    }
    assert!(
        job.find("Check the token exchange") < job.find("Publish what crates.io does not have yet"),
        "the exchange is checked before anything is published"
    );
}

#[test]
fn a_dry_run_of_a_published_version_verifies_its_package_only() {
    let job = code(&job(&release_yml(), "crates-io"));
    let published = &job[job
        .find("200)")
        .expect("a branch for a version crates.io has")..];
    let published = &published[..published.find(";;").expect("end of the branch")];
    assert!(published.contains("cargo package --locked -p \"$crate\""));
    assert!(published.contains("version already published: verified package only"));
    assert!(
        !published.contains("cargo publish"),
        "nothing is published again"
    );
    let new = &job[job.find("404)").expect("a branch for a new version")..];
    let new = &new[..new.find(";;").expect("end of the branch")];
    assert!(new.contains("cargo publish --dry-run --locked -p \"$crate\""));
    assert!(new.contains("cargo publish --locked -p \"$crate\""));
}

#[test]
fn job_reads_one_job_only() {
    let yml = "jobs:\n  a:\n    x: 1\n    # c\n  b:\n    y: 2\n";
    assert_eq!(job(yml, "a"), "    x: 1\n    # c");
    assert_eq!(job(yml, "b"), "    y: 2");
}
