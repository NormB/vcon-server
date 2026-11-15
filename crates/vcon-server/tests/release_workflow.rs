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
    assert!(job.contains("vcon vcon-server"), "both crates, in order");
    assert!(
        job.contains("https://crates.io/api/v1/crates/${crate}/${version}"),
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
    assert!(
        job.contains("cargo publish --dry-run --locked -p vcon -p vcon-server"),
        "both crates in one dry run, so the server verifies against the packaged vcon"
    );
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
fn a_dry_run_names_each_crate_and_never_publishes() {
    let job = code(&job(&release_yml(), "crates-io"));
    let published = &job[job
        .find("200)")
        .expect("a branch for a version crates.io has")..];
    let published = &published[..published.find(";;").expect("end of the branch")];
    assert!(published.contains("version already published: verified package only"));
    assert!(
        !published.contains("cargo publish"),
        "nothing is published again"
    );
    let new = &job[job.find("404)").expect("a branch for a new version")..];
    let new = &new[..new.find(";;").expect("end of the branch")];
    let (dry, real) = new.split_once("else").expect("a dry and a real branch");
    assert!(dry.contains("new version: dry run passed"), "{dry}");
    assert!(
        !dry.contains("cargo publish"),
        "a dry run uploads nothing: {dry}"
    );
    assert!(
        real.contains("cargo publish --locked -p \"$crate\""),
        "{real}"
    );
}

#[test]
fn each_crate_is_looked_up_at_its_own_version() {
    // The crates keep their own versions (vcon 0.2.0 beside vcon-server
    // 0.1), so the workspace's version names neither for certain.
    let job = code(&job(&release_yml(), "crates-io"));
    assert!(
        job.contains("version=\"$(cargo pkgid -p \"$crate\" | sed 's/.*[#@]//')\""),
        "each crate's version is read from its own manifest"
    );
    assert!(
        !job.contains("${VERSION}"),
        "no one version stands for both crates"
    );
}

#[test]
fn job_reads_one_job_only() {
    let yml = "jobs:\n  a:\n    x: 1\n    # c\n  b:\n    y: 2\n";
    assert_eq!(job(yml, "a"), "    x: 1\n    # c");
    assert_eq!(job(yml, "b"), "    y: 2");
}

// Each release proves its own binaries: release.yml builds each target a
// second time, in another checkout with another CARGO_HOME, and compares
// the binary of the archive with that build before anything is attested
// or published. Until then no run compared a released binary: the
// reproducible-build workflow built x86_64 only and never on a tag.

fn workflow(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The jobs the job `name` names under `needs:`, written as a list on
/// one line or as one name; none when the job has no such line.
fn needs(yml: &str, name: &str) -> Vec<String> {
    code(&job(yml, name))
        .lines()
        .find_map(|line| line.strip_prefix("    needs:"))
        .map(|list| {
            list.trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|job| job.trim().to_owned())
                .filter(|job| !job.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The names of the jobs of a workflow.
fn jobs(yml: &str) -> Vec<String> {
    yml.lines()
        .skip_while(|line| *line != "jobs:")
        .skip(1)
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.trim().strip_suffix(':'))
        .map(str::to_owned)
        .collect()
}

/// Whether the job `later` cannot start before `earlier` has ended: it
/// needs it, or needs a job that does.
fn waits_for(yml: &str, later: &str, earlier: &str) -> bool {
    let known = jobs(yml);
    let mut seen: Vec<String> = Vec::new();
    let mut open = vec![later.to_owned()];
    while let Some(name) = open.pop() {
        if seen.contains(&name) || !known.contains(&name) {
            continue;
        }
        for needed in needs(yml, &name) {
            if needed == earlier {
                return true;
            }
            open.push(needed);
        }
        seen.push(name);
    }
    false
}

/// The `target` and `runner` pairs of a job's matrix.
fn targets(job: &str) -> Vec<(String, String)> {
    let code = code(job);
    let lines: Vec<&str> = code.lines().map(str::trim).collect();
    lines
        .windows(2)
        .filter_map(|pair| {
            let target = pair[0].strip_prefix("- target: ")?;
            let runner = pair[1].strip_prefix("runner: ")?;
            Some((target.to_owned(), runner.to_owned()))
        })
        .collect()
}

/// The container image a job runs in.
fn image(job: &str) -> Option<String> {
    code(job)
        .lines()
        .find_map(|line| line.trim().strip_prefix("image: "))
        .map(str::to_owned)
}

/// The events of a workflow's `on:` block.
fn events(yml: &str) -> Vec<String> {
    yml.lines()
        .skip_while(|line| *line != "on:")
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with(' '))
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.trim().split(':').next())
        .map(str::to_owned)
        .collect()
}

/// A workflow of the release's shape, for the readers above.
const SHAPE: &str = "on:\n  push:\n    tags: ['v*']\n  # pull_request:\n  workflow_dispatch:\n\n\
jobs:\n  version:\n    runs-on: x\n  build:\n    needs: version\n    container:\n      image: a:1@sha256:00\n    strategy:\n      matrix:\n        include:\n          - target: t-one\n            runner: r-one\n          - target: t-two\n            runner: r-two\n  rebuild:\n    needs: version\n  reproduced:\n    needs: [version, build, rebuild]\n  attest:\n    # needs: [nothing]\n    needs: [version, build, reproduced]\n  publish:\n    needs: [version, attest]\n";

#[test]
fn the_jobs_a_job_needs_are_read_from_a_list_and_from_one_name() {
    assert_eq!(
        needs(SHAPE, "reproduced"),
        ["version", "build", "rebuild"],
        "a list"
    );
    assert_eq!(needs(SHAPE, "build"), ["version"], "one name");
}

#[test]
fn a_job_that_names_no_other_job_needs_none() {
    assert_eq!(
        needs(SHAPE, "version"),
        Vec::<String>::new(),
        "no needs line"
    );
}

#[test]
fn a_needs_line_in_a_comment_is_not_read() {
    assert_eq!(
        needs(SHAPE, "attest"),
        ["version", "build", "reproduced"],
        "the comment above the line names another list"
    );
}

#[test]
fn the_jobs_of_a_workflow_are_its_names_under_jobs() {
    assert_eq!(
        jobs(SHAPE),
        [
            "version",
            "build",
            "rebuild",
            "reproduced",
            "attest",
            "publish"
        ],
        "each job once, and no key of the on block or of a job"
    );
}

#[test]
fn a_job_waits_for_a_job_it_needs_through_another() {
    assert!(
        waits_for(SHAPE, "publish", "rebuild"),
        "publish needs attest, attest needs reproduced, reproduced needs rebuild"
    );
    assert!(waits_for(SHAPE, "attest", "reproduced"), "named directly");
}

#[test]
fn a_job_does_not_wait_for_one_that_no_needs_line_leads_to() {
    assert!(
        !waits_for(SHAPE, "build", "rebuild"),
        "the two builds run side by side"
    );
    assert!(
        !waits_for(SHAPE, "rebuild", "publish"),
        "needs lines are followed one way"
    );
}

#[test]
fn a_workflow_whose_attest_job_skips_the_comparison_is_seen() {
    let skipped = SHAPE.replace(
        "    needs: [version, build, reproduced]",
        "    needs: [version, build]",
    );
    assert!(
        !waits_for(&skipped, "publish", "reproduced"),
        "nothing leads from publish to the comparison"
    );
}

#[test]
fn a_job_that_needs_itself_or_an_unknown_job_ends_the_walk() {
    let looped = "jobs:\n  a:\n    needs: [a, ghost]\n  b:\n    needs: a\n";
    assert!(!waits_for(looped, "b", "c"), "a loop is walked once");
    assert!(
        !waits_for(looped, "ghost", "a"),
        "a job the workflow does not have"
    );
}

#[test]
fn the_targets_of_a_matrix_are_read_with_their_runners() {
    assert_eq!(
        targets(&job(SHAPE, "build")),
        [
            ("t-one".to_owned(), "r-one".to_owned()),
            ("t-two".to_owned(), "r-two".to_owned())
        ],
        "each target and the runner on the line after it"
    );
    assert_eq!(
        targets(&job(SHAPE, "rebuild")),
        Vec::new(),
        "a job without a matrix"
    );
}

#[test]
fn the_image_of_a_job_is_read_and_a_job_without_one_has_none() {
    assert_eq!(
        image(&job(SHAPE, "build")).as_deref(),
        Some("a:1@sha256:00"),
        "the container's image"
    );
    assert_eq!(image(&job(SHAPE, "rebuild")), None, "no container");
}

#[test]
fn the_events_of_a_workflow_leave_out_comments_and_their_own_keys() {
    assert_eq!(
        events(SHAPE),
        ["push", "workflow_dispatch"],
        "pull_request is in a comment, tags is a key of push"
    );
}

#[test]
fn the_release_builds_each_target_a_second_time_in_the_same_container() {
    let yml = release_yml();
    let build = job(&yml, "build");
    let rebuild = job(&yml, "rebuild");
    assert_eq!(
        targets(&build).len(),
        2,
        "the release builds x86_64 and aarch64"
    );
    assert_eq!(
        targets(&rebuild),
        targets(&build),
        "the second build covers each target of the first, on the same runner"
    );
    assert!(image(&build).is_some(), "the build names its container");
    assert_eq!(
        image(&rebuild),
        image(&build),
        "the same image, digest for digest"
    );
}

#[test]
fn the_second_build_is_the_scripts_rebuild_and_not_a_copy_of_the_first() {
    let yml = release_yml();
    let rebuild = code(&job(&yml, "rebuild"));
    assert!(
        rebuild.contains(
            "bash scripts/reproducible-build.sh rebuild \"$TARGET\" \"$RUNNER_TEMP/rebuild\""
        ),
        "another checkout and another CARGO_HOME, made by the script"
    );
    assert!(
        rebuild.contains("bash scripts/release-build-env.sh"),
        "the container is prepared as the build's is"
    );
    assert!(
        !rebuild.contains("download-artifact"),
        "the second build takes nothing from the first"
    );
    assert!(
        !waits_for(&yml, "rebuild", "build"),
        "the two builds run side by side"
    );
}

#[test]
fn the_archives_binary_is_compared_with_the_second_build_for_each_target() {
    let yml = release_yml();
    let reproduced = code(&job(&yml, "reproduced"));
    for needed in ["build", "rebuild"] {
        assert!(
            needs(&yml, "reproduced").contains(&needed.to_owned()),
            "the comparison waits for {needed}"
        );
    }
    assert!(
        reproduced.contains("bash scripts/reproducible-build.sh compare"),
        "the script's verdict: identical with the digest, or the evidence and code 1"
    );
    for (target, _) in targets(&job(&yml, "build")) {
        assert!(
            reproduced.contains(&target),
            "the comparison names {target}"
        );
    }
    assert!(
        reproduced.contains("tar xzf"),
        "the first binary is the one in the archive that is released"
    );
    assert!(
        !reproduced.contains("continue-on-error") && !reproduced.contains("|| true"),
        "a difference fails the job"
    );
}

#[test]
fn nothing_is_attested_or_published_before_the_comparison() {
    let yml = release_yml();
    for later in ["attest", "publish", "crates-io"] {
        assert!(
            waits_for(&yml, later, "reproduced"),
            "{later} waits for the comparison"
        );
    }
    assert!(
        !code(&job(&yml, "reproduced")).contains("if:"),
        "the comparison runs on a tag, on a pull request and by hand"
    );
}

#[test]
fn the_reproducible_workflow_runs_weekly_and_by_hand_for_both_targets() {
    let yml = workflow("reproducible.yml");
    assert_eq!(
        events(&yml),
        ["schedule", "workflow_dispatch"],
        "release.yml compares both targets on a pull request that changes the build, so \
         this workflow does not build them again there"
    );
    let job = job(&yml, "reproducible");
    assert_eq!(
        targets(&job),
        targets(&super_build()),
        "the targets and runners of the release build"
    );
    assert_eq!(
        image(&job),
        image(&super_build()),
        "the release build's container"
    );
}

/// The release workflow's build job.
fn super_build() -> String {
    job(&release_yml(), "build")
}

#[test]
fn a_run_by_hand_can_name_the_commit_and_the_build_time() {
    let yml = workflow("reproducible.yml");
    let on = &yml[..yml.find("\njobs:").expect("jobs")];
    for input in ["ref:", "source_date_epoch:"] {
        assert!(on.contains(input), "the input {input}");
    }
    let job = code(&job(&yml, "reproducible"));
    assert_eq!(
        job.matches("ref: ${{ inputs.ref }}").count(),
        2,
        "both checkouts take the commit asked for"
    );
    assert!(
        job.contains("SOURCE_DATE_EPOCH: ${{ inputs.source_date_epoch }}"),
        "the build time is given to the script in the environment"
    );
    assert!(
        !job.contains("run: bash scripts/reproducible-build.sh check ${{"),
        "no input is written into a command line"
    );
}

/// Runs scripts/reproducible-build.sh with `args` and `epoch` as
/// SOURCE_DATE_EPOCH (unset when `None`); its code, stdout and stderr.
fn script(args: &[&str], epoch: Option<&str>) -> (Option<i32>, String, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = std::process::Command::new("bash");
    command
        .arg(root.join("scripts/reproducible-build.sh"))
        .args(args)
        .env_remove("SOURCE_DATE_EPOCH");
    if let Some(epoch) = epoch {
        command.env("SOURCE_DATE_EPOCH", epoch);
    }
    let output = command.output().expect("bash runs the script");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Two files in a new directory, with the bytes given.
fn pair(a: &[u8], b: &[u8]) -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let (one, two) = (dir.path().join("one"), dir.path().join("two"));
    std::fs::write(&one, a).expect("write one");
    std::fs::write(&two, b).expect("write two");
    let text = |path: &Path| path.to_str().expect("a UTF-8 path").to_owned();
    let (one, two) = (text(&one), text(&two));
    (dir, one, two)
}

#[test]
fn two_equal_files_compare_identical_with_their_digest() {
    let (_dir, one, two) = pair(b"abc", b"abc");
    let (status, out, _) = script(&["compare", &one, &two], None);
    assert_eq!(status, Some(0), "equal files: {out}");
    assert!(
        out.starts_with(
            "identical: ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  "
        ),
        "the SHA-256 of the bytes, for the reader to set beside a release's: {out}"
    );
}

#[test]
fn two_files_that_differ_by_one_byte_fail_with_the_evidence() {
    let (_dir, one, two) = pair(b"abcdefgh", b"abcdefgX");
    let (status, out, _) = script(&["compare", &one, &two], None);
    assert_eq!(status, Some(1), "a difference is code 1: {out}");
    assert!(out.starts_with("NOT reproducible: "), "the verdict: {out}");
    assert!(
        out.contains("differing bytes: 1 "),
        "how many bytes differ: {out}"
    );
    assert!(
        !out.contains("identical"),
        "no digest line of a pass: {out}"
    );
}

#[test]
fn a_file_that_is_a_prefix_of_the_other_is_not_identical() {
    let (_dir, one, two) = pair(b"abc", b"abcd");
    let (status, out, _) = script(&["compare", &one, &two], None);
    assert_eq!(status, Some(1), "another length: {out}");
    assert!(out.starts_with("NOT reproducible: "), "the verdict: {out}");
}

#[test]
fn two_empty_files_compare_identical() {
    let (_dir, one, two) = pair(b"", b"");
    let (status, out, _) = script(&["compare", &one, &two], None);
    assert_eq!(status, Some(0), "no bytes differ: {out}");
    assert!(
        out.starts_with(
            "identical: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  "
        ),
        "the SHA-256 of no bytes: {out}"
    );
}

#[test]
fn a_comparison_with_a_missing_file_is_refused() {
    let (dir, one, _) = pair(b"abc", b"abc");
    let missing = dir.path().join("three");
    let missing = missing.to_str().expect("a UTF-8 path");
    let (status, out, err) = script(&["compare", &one, missing], None);
    assert_eq!(status, Some(1), "no second file: {err}");
    assert!(err.contains("no such file: "), "the reason: {err}");
    assert!(out.is_empty(), "no verdict: {out}");
}

#[test]
fn a_comparison_of_one_file_is_refused() {
    let (_dir, one, _) = pair(b"abc", b"abc");
    let (status, _, err) = script(&["compare", &one], None);
    assert_eq!(status, Some(1), "one argument: {err}");
    assert!(err.contains("usage: "), "the usage: {err}");
}

#[test]
fn the_build_time_is_the_one_given() {
    let (status, out, err) = script(&["epoch"], Some("1794686421"));
    assert_eq!(status, Some(0), "a time in seconds: {err}");
    assert_eq!(out, "1794686421\n", "the time given, unchanged");
}

#[test]
fn the_build_time_is_the_commits_when_none_is_given() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let git = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["log", "-1", "--format=%ct", "HEAD"])
        .output()
        .expect("git runs");
    let commit_time = String::from_utf8_lossy(&git.stdout).into_owned();
    assert!(
        commit_time.trim().parse::<u64>().is_ok(),
        "the commit has a time: {commit_time:?}"
    );
    let (status, out, err) = script(&["epoch"], None);
    assert_eq!(status, Some(0), "from the checkout: {err}");
    assert_eq!(out, commit_time, "the time of the commit that is built");
}

#[test]
fn an_empty_build_time_is_no_build_time() {
    let (_, unset, _) = script(&["epoch"], None);
    let (status, out, err) = script(&["epoch"], Some(""));
    assert_eq!(status, Some(0), "a run by hand that fills no input: {err}");
    assert_eq!(out, unset, "the commit's time, as with none given");
}

#[test]
fn a_build_time_that_is_not_a_number_of_seconds_is_refused() {
    for epoch in ["yesterday", "1794686421; id", "-5", "17 94", "1.5"] {
        let (status, out, err) = script(&["epoch"], Some(epoch));
        assert_eq!(status, Some(1), "{epoch:?} is refused: {out}");
        assert!(
            err.contains("SOURCE_DATE_EPOCH must be a whole number of seconds"),
            "{epoch:?}: the reason: {err}"
        );
        assert!(out.is_empty(), "{epoch:?}: no time is printed: {out}");
    }
}

#[test]
fn a_second_build_into_a_directory_that_exists_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let there = dir.path().to_str().expect("a UTF-8 path");
    // A target no toolchain has: were the refusal ever lost, cargo would
    // end at once instead of building a release binary inside a test.
    let (status, out, err) = script(&["rebuild", "no-such-target", there], None);
    assert_eq!(status, Some(1), "the directory is there: {err}");
    assert!(
        err.contains("exists; give rebuild a new directory"),
        "the reason: {err}"
    );
    assert!(out.is_empty(), "nothing was built: {out}");
}

#[test]
fn a_second_build_without_a_directory_is_refused() {
    let (status, _, err) = script(&["rebuild", "no-such-target"], None);
    assert_eq!(status, Some(1), "one argument: {err}");
    assert!(
        err.contains("usage: ") && err.contains("rebuild <target-triple> <workdir>"),
        "the usage: {err}"
    );
}

#[test]
fn the_build_says_which_commit_and_build_time_it_used() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/reproducible-build.sh"),
    )
    .expect("the script");
    let build = &script[script.find("cmd_build() {").expect("cmd_build")..];
    let build = &build[..build.find("\n}\n").expect("the function's end")];
    assert!(
        build.contains("SOURCE_DATE_EPOCH=$(build_epoch)"),
        "the build and `epoch` take the time from one function"
    );
    assert!(
        build.contains("printf 'reproducible-build: commit %s, SOURCE_DATE_EPOCH=%s\\n'"),
        "a log shows the input that makes a pull request's digest differ from a release's"
    );
}

// The website names a release only once it exists. The pages name the
// version of the tree they are built from, and a release commit changes
// it before its tag exists: at 0.4.6 the site said "0.4.6 is released",
// with that release's download commands, for 22 minutes 9 seconds before
// the release was published. pages.yml now deploys a commit only when
// scripts/advertised_release.py finds a complete release of its version,
// and release.yml deploys the site, from main, after it has published.

/// The step of `job` named `name`, to the next step.
fn step(job: &str, name: &str) -> String {
    let start = job
        .find(&format!("- name: {name}\n"))
        .unwrap_or_else(|| panic!("no step {name}"));
    let rest = &job[start..];
    let end = rest[2..].find("\n      - ").map_or(rest.len(), |at| at + 3);
    rest[..end].to_owned()
}

#[test]
fn a_step_is_read_to_the_next_step() {
    let job = "    steps:\n      - name: A\n        run: one\n      - uses: x\n      - name: B\n        run: two\n";
    assert_eq!(
        step(job, "A"),
        "- name: A\n        run: one\n",
        "to the next step"
    );
    assert_eq!(
        step(job, "B"),
        "- name: B\n        run: two\n",
        "the last step"
    );
}

#[test]
fn pages_decides_before_it_builds_and_builds_only_a_released_version() {
    let yml = workflow("pages.yml");
    assert!(
        waits_for(&yml, "build", "decide") && waits_for(&yml, "deploy", "decide"),
        "the build and the deploy wait for the decision"
    );
    let build = code(&job(&yml, "build"));
    assert!(
        build.contains("if: needs.decide.outputs.verdict == 'deploy'"),
        "a held commit is not built, so it is not deployed"
    );
    let decide = code(&job(&yml, "decide"));
    assert!(
        decide.contains(
            "python3 -I scripts/advertised_release.py decide --repo \"$GITHUB_REPOSITORY\" \
             --expect \"$EXPECT\" --github-output \"$GITHUB_OUTPUT\""
        ),
        "the one rule decides, and its verdict is the job's output"
    );
    for output in ["verdict:", "tree:", "sha:", "id:"] {
        assert!(decide.contains(output), "the decision's output {output}");
    }
}

#[test]
fn the_decision_reads_the_tree_that_is_built() {
    let yml = workflow("pages.yml");
    let decide = code(&job(&yml, "decide"));
    let build = code(&job(&yml, "build"));
    assert!(
        decide.contains("ref: ${{ steps.ci.outputs.sha }}"),
        "the decision reads Cargo.toml of the commit CI ran"
    );
    assert!(
        build.contains("ref: ${{ needs.decide.outputs.sha }}"),
        "the build checks out that same commit"
    );
    assert!(
        build.contains("run-id: ${{ needs.decide.outputs.id }}"),
        "and takes its numbers from that commit's CI run"
    );
}

#[test]
fn a_run_by_hand_builds_mains_newest_green_commit_and_never_the_tag() {
    let yml = workflow("pages.yml");
    let decide = code(&job(&yml, "decide"));
    let find = step(&decide, "Find CI's run of the commit to deploy");
    assert!(
        find.contains("--workflow ci.yml --branch main"),
        "a run by hand takes CI's newest passing run of main, so a docs fix that landed while a \
         deploy was held is deployed with the release"
    );
    assert!(find.contains("--event push --status success"), "{find}");
    assert!(
        !yml.contains("refs/tags") && !yml.contains("github.ref_name"),
        "nothing in pages.yml builds a tag"
    );
}

#[test]
fn a_run_by_hand_can_name_the_release_it_must_deploy() {
    let yml = workflow("pages.yml");
    assert_eq!(
        events(&yml),
        ["workflow_run", "workflow_dispatch"],
        "after CI on main, and by hand"
    );
    let on = &yml[..yml.find("\njobs:").expect("jobs")];
    assert!(on.contains("expect:"), "the input expect");
    let decide = code(&job(&yml, "decide"));
    assert!(
        decide.contains("EXPECT: ${{ inputs.expect }}"),
        "the input reaches the script through the environment"
    );
}

#[test]
fn the_site_holds_the_version_it_names_in_a_static_file() {
    let yml = workflow("pages.yml");
    let build = code(&job(&yml, "build"));
    let write = step(&build, "Record the release the site names");
    assert!(
        write.contains("website/public/advertised-release.txt"),
        "a file beside the pages, read by the release workflow and the daily check"
    );
    assert!(
        write.contains("TREE: ${{ needs.decide.outputs.tree }}"),
        "the version the decision deployed"
    );
    let built = build.find("- name: Build site").expect("the build step");
    let written = build
        .find("- name: Record the release the site names")
        .expect("the step");
    let uploaded = build.find("upload-pages-artifact").expect("the upload");
    assert!(
        built < written && written < uploaded,
        "after zola build, which empties website/public, and before the upload"
    );
}

#[test]
fn the_release_deploys_the_site_only_after_it_is_published() {
    let yml = release_yml();
    for earlier in ["publish", "crates-io"] {
        assert!(
            needs(&yml, "site").contains(&earlier.to_owned()),
            "the site job needs {earlier}"
        );
    }
    let site = code(&job(&yml, "site"));
    for condition in [
        "github.ref_type == 'tag'",
        "needs.publish.result == 'success'",
        "needs.crates-io.result == 'success'",
    ] {
        assert!(site.contains(condition), "{condition}");
    }
    assert!(
        site.contains("actions: write"),
        "dispatching a workflow needs it"
    );
    assert!(
        !site.contains("contents: write"),
        "the job changes nothing in the repository"
    );
}

#[test]
fn the_release_deploys_main_and_waits_for_that_run() {
    let site = code(&job(&release_yml(), "site"));
    assert!(
        site.contains(
            "gh workflow run pages.yml --repo \"$GITHUB_REPOSITORY\" --ref main -f expect=\"$VERSION\""
        ),
        "the deploy builds main, not the tag, and must deploy this version"
    );
    assert!(
        site.contains("--event workflow_dispatch --created \">=$since\""),
        "the run it waits for is the one it started"
    );
    assert!(
        site.contains("gh run watch \"$run\" --repo \"$GITHUB_REPOSITORY\" --exit-status"),
        "a failed deploy fails the release run"
    );
}

#[test]
fn the_release_checks_the_live_static_file_and_a_challenge_is_no_pass() {
    let site = code(&job(&release_yml(), "site"));
    assert!(
        site.contains(
            "python3 -I scripts/advertised_release.py live --url \
             https://vcon-server.com/advertised-release.txt --expect \"$VERSION\""
        ),
        "the static file, read by the script whose verdict counts only a match as a pass"
    );
    assert!(
        !site.contains("|| true") && !site.contains("continue-on-error"),
        "a read that is not verified fails the job"
    );
}

#[test]
fn the_daily_check_reads_main_with_its_history_and_the_static_file() {
    let yml = workflow("site-release.yml");
    assert_eq!(
        events(&yml),
        ["schedule", "workflow_dispatch"],
        "daily, and by hand"
    );
    let check = code(&job(&yml, "site-release"));
    assert!(check.contains("ref: main"), "main's Cargo.toml");
    assert!(
        check.contains("fetch-depth: 0"),
        "git tells since when main's version has had no release"
    );
    assert!(
        check.contains("python3 -I scripts/advertised_release.py daily"),
        "the script"
    );
    assert!(
        check.contains("--url https://vcon-server.com/advertised-release.txt"),
        "a static file, which the network in front of the site answers to a runner"
    );
}

#[test]
fn the_site_url_is_the_sites_base_url() {
    let config = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../website/config.toml"),
    )
    .expect("the site's config");
    assert!(
        config
            .lines()
            .any(|line| line == "base_url = \"https://vcon-server.com\""),
        "the address the workflows read the file from"
    );
}
