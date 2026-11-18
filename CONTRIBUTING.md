# Contributing to vcon-server

## Orientation

Start with the [development page](https://vcon-server.com/docs/development/):
the layout of the workspace, how to build and test, how the code is tested,
the commit hooks and CI. The [security page](https://vcon-server.com/docs/security/)
says what the server guarantees; a change must keep every guarantee there.

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).
[GOVERNANCE.md](GOVERNANCE.md) says who decides and what a contributor and the
maintainer each do; [MAINTAINERS.md](MAINTAINERS.md) lists the maintainers.
For a first pull request, see [Your first change](#your-first-change).

Each change comes with the test that encodes it, written first and seen
failing for the right reason. Test data uses reserved values only:
`example.com` addresses, 555-01xx numbers, RFC 5737 and RFC 3849 addresses.

Before your first commit, turn on the hooks; they run the same checks as CI:

```sh
git config core.hooksPath .githooks
```

## Reporting a bug or asking for a feature

Open an issue at <https://github.com/NormB/vcon-server/issues>. For a bug,
give:

- the output of `vcon-server --version` and the commit you built,
- what you did, as a command or a request,
- what you expected and what happened instead,
- the server's log lines for that request, with every token removed.

Issues and comments are in English. A security problem is never an issue:
see [Reporting a security problem](#reporting-a-security-problem).

## Proposing a change

Send it as a pull request against `main`. CI runs the same checks as the
hooks on every pull request. The check `CI success` passes only when every
job of `.github/workflows/ci.yml` succeeded, and a pull request merges only
after it passes.

## Your first change

**Purpose:** a first pull request, from a starter task to a merge.

**Prerequisites:**

- A GitHub account and git.
- Rust: rustup installs the toolchain that `rust-toolchain.toml` names on
  the first build. Python 3 runs the script checks of the commit hook.
- An SSH key registered on GitHub as a signing key. The `main` branch
  refuses a commit without a signature that GitHub verifies, and the
  pre-push hook refuses one that git cannot verify.
- Nothing changes on any other system.

**Steps:**

1. Pick an issue opened from the
   [starter-task form](.github/ISSUE_TEMPLATE/starter_task.yml), or open
   one with it at <https://github.com/NormB/vcon-server/issues/new?template=starter_task.yml>.
   Say on the issue that you are taking it.
2. Fork the repository, clone your fork, and turn on the hooks:

   ```sh
   git config core.hooksPath .githooks
   ```

3. Sign your commits with your SSH key, and let git verify them:

   ```sh
   git config gpg.format ssh
   git config user.signingkey ~/.ssh/id_ed25519.pub
   git config commit.gpgsign true
   git config gpg.ssh.allowedSignersFile ~/.config/git/allowed_signers
   ```

   The file `~/.config/git/allowed_signers` holds one line: your
   `user.email`, a space, and the contents of `~/.ssh/id_ed25519.pub`.
4. Make a branch from `main`. Write the test the issue's "How to test it
   first" describes, run it, and read the failure: it must fail because the
   behavior is missing, not because the test is wrong.
5. Make the change, run the test until it passes, and commit. The commit
   hook runs the same checks as CI.
6. Push the branch to your fork and open a pull request against `main` that
   names the issue. Sign the
   [Contributor License Agreement](#contributor-license-agreement) when the
   bot asks.

**Verify:** `git log -1 --show-signature` reports a good signature; on the
pull request, `CI success` and `license/cla` are green.

**Troubleshoot:**

- `pre-push: refused: <sha> has no good signature from an allowed signer`:
  step 3 is not complete. Check the allowed signers line, then sign the
  branch's commits again with
  `git rebase --exec 'git commit --amend --no-edit -S' origin/main`.
- The pre-push hook refuses a commit whose author or committer is not your
  configured `user.name` and `user.email`: set them, and sign again as
  above.
- The commit hook refuses personal data: test data uses reserved values only
  (`example.com` addresses, 555-01xx numbers, RFC 5737 and RFC 3849
  addresses).
- A CI job fails: its log names the gate. The
  [development page](https://vcon-server.com/docs/development/#gates)
  describes each one and how to run it before a push.

## Dependency versions

A new dependency, or a new version of one, is taken only once that version
has been public for at least 14 days, so that a regression or a compromised
release has time to come to light first. The rule covers every ecosystem the
repository uses: crates, npm and pip packages, GitHub Actions and container
images. Dependabot waits the same 14 days (`cooldown` in
`.github/dependabot.yml`), and `scripts/check_dependabot.py` fails on an
entry that waits less. Dependabot applies no cooldown to its security
updates.

There is one exception: a version that fixes a published security advisory
against a dependency the repository already uses is taken as soon as it is
out. The change that takes it writes one comment line in the `Cargo.toml`
that requires the crate, above the requirement (in the workspace
`Cargo.toml` for a crate that only a `Cargo.lock` names):

```toml
# dependency-age-exception: <crate> <fixed version> <advisory id> published <YYYY-MM-DD>
```

- `<fixed version>` is the version the change takes: the one the advisory
  names as fixed.
- `<advisory id>` is the advisory's RUSTSEC, GHSA or CVE id, for example
  `RUSTSEC-2026-0459`.
- `<YYYY-MM-DD>` is the UTC day crates.io published the fixed version.

A crate that the fixed version requires, at a version that is itself under
14 days old, gets a line of its own with the same advisory id, and its own
version and day. The line stays while a `Cargo.lock` holds the crate.
`scripts/check_dependency_age.py` reads these lines. In the commit hook it
uses no network: it fails on a line that does not have this form, on a
crate and version excepted twice, and on a line whose crate no `Cargo.lock`
holds at the fixed version or later. In CI it runs with `--registry` and
also asks crates.io for publish times: it fails on a line whose date is not
the registry's, and on a crate version new to a `Cargo.lock` that has been
public for less than the 14 days and that no line names. A lockfile does not
record publish times, so only the CI run can tell a version's age. To run
the same check before a push:

```sh
python3 -I scripts/check_dependency_age.py --registry --base origin/main
```

The check covers crates. For npm and pip packages, GitHub Actions and
container images, a security fix inside the 14 days is stated in the pull
request with the advisory's id.

## Signing off commits

**Purpose:** each commit carries a Developer Certificate of Origin
sign-off, a `Signed-off-by:` line that states you have the right to submit
it under the project's licenses (<https://developercertificate.org/>). The
`DCO` check reads every commit of a pull request. While the project
switches from the Contributor License Agreement, both apply: sign the
[agreement](#contributor-license-agreement) once and sign off each commit.
`DCO` is advisory today: the `main` branch does not require it, but a red
`DCO` still holds up a merge.

**Prerequisites:**

- GitHub's "Keep my email addresses private" turned on, under Settings,
  Emails. GitHub then shows your noreply address, of the form
  `<ID>+<login>@users.noreply.github.com`. A sign-off can name only that
  address: any other address in a commit message is personal data, which
  the commit hook and CI refuse.
- The signing setup of [Your first change](#your-first-change), step 3, with
  the same address in the allowed signers line.
- Nothing changes on any other system.

**Steps:**

1. Use the noreply address for your commits in this clone:

   ```sh
   git config user.email '<ID>+<login>@users.noreply.github.com'
   ```

2. Commit with `-s`, which adds `Signed-off-by: <user.name> <user.email>`:

   ```sh
   git commit -s
   ```

3. Commits made before, on your branch, get the line and the address
   again, each signed again:

   ```sh
   git rebase --exec 'git commit --amend --no-edit --reset-author -s' origin/main
   ```

4. Push the branch to your fork.

**Verify:** `git log -1 --format=%B` ends with the `Signed-off-by:` line;
the commit hook printed no `commit-msg: warning:` line; on the pull
request, `DCO` is green.

**Troubleshoot:** `DCO` prints one line per commit, and the commit hook the
same rule as a warning.

| Message | Fix |
|---|---|
| `no Signed-off-by line` | Step 3, or `git commit --amend --no-edit -s` for the last commit only. |
| `no Signed-off-by line names the author email ... with a name` | The line names another address than the commit's author, or no name: set `user.name` and `user.email`, then step 3. |
| `the author email is not a GitHub noreply address` | Step 1, then step 3: `--reset-author` puts the new address on each commit. |
| `<commit message>: email address:` from the commit hook | The sign-off names a real address; use the noreply address (step 1). |
| `DCO: cannot read ...; nothing was checked` | GitHub's API did not answer; run the job again. |

## Contributor License Agreement

Sign the vcon-server [Contributor License Agreement](CLA.md) before your first
pull request merges.

- It is a one-time step covering all of your contributions.
- You keep full ownership of your work.

The text exists in three copies:

- `CLA.md` holds the text.
- <https://vcon-server.com/cla/> republishes it; `scripts/check-site.sh` fails
  if the two differ by a single character.
- [CLA Assistant](https://cla-assistant.io/NormB/vcon-server) shows the same
  words to signers, from a gist that matches `CLA.md`.

Open your pull request as normal. If anyone who committed to it has not signed,
the `CLAassistant` bot comments within a minute or so. You have two ways to
answer it:

- follow its link and authorize CLA Assistant with your GitHub account,
- or post this exact sentence as a pull request comment.

> I have read the CLA Document and I hereby sign the CLA

Post it verbatim: the bot matches the whole sentence, so a reworded version
reads as an ordinary comment and signs nothing. The bot then reports a
`license/cla` status on the pull request, and turns it green once everyone who
committed to the branch has signed.

## Reporting a security problem

Do not open a public issue. Use GitHub's private vulnerability reporting on
this repository: the Security tab, then "Report a vulnerability".
