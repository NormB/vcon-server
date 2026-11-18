# Maintainers

[GOVERNANCE.md](GOVERNANCE.md) says what a maintainer, a committer and a
contributor each do, and how the project decides.

## Maintainers

| GitHub account | Area |
|---|---|
| [@NormB](https://github.com/NormB) | All of the repository |

`.github/CODEOWNERS` names the same accounts, so GitHub asks them to review
every pull request. The `main` branch has no rule that requires a review.

To reach a maintainer, open an issue or a
[Discussion](https://github.com/NormB/vcon-server/discussions). A security
problem goes through [SECURITY.md](SECURITY.md) instead.

## Getting commit access

- [Becoming a committer](GOVERNANCE.md#becoming-a-committer) says who can
  ask and who decides.
- The GitHub account has two-factor authentication turned on.
- Every commit carries a signature that GitHub verifies: the `main` branch
  refuses an unsigned commit from anyone.
- The pull request that grants access adds the account to the table above
  and to `.github/CODEOWNERS`.
- Access ends by a pull request that removes the account from both files,
  and the maintainer then removes it in the repository's settings.

## Succession

No successor is named. [If the maintainer is unavailable](GOVERNANCE.md#if-the-maintainer-is-unavailable)
lists what a user can rely on without one: the license, release archives
that can be checked, and where each release is.

## The contributor agreement

A contributor signs the [Contributor License Agreement](CLA.md) before a
first pull request merges; [CONTRIBUTING.md](CONTRIBUTING.md#contributor-license-agreement)
gives the steps. Each commit also carries a DCO sign-off
([Signing off commits](CONTRIBUTING.md#signing-off-commits)), which the
advisory `DCO` check reads. A verified commit that the repository owner
authored and committed needs no sign-off, and neither does a verified
commit by a GitHub bot account.

The switch from the agreement to the sign-off is the owner's to make, in
this order:

1. Require `DCO` in the `main` ruleset and stop requiring `license/cla`.
2. Land the pull request that takes `DCO` off `ADVISORY` in
   `scripts/check_required_checks.py`, and `license/cla` off `EXTERNAL`.
   Until it lands, CI's required-checks step fails on every pull request.
3. Turn CLA Assistant off for the repository.

The commit hook's sign-off warning reads `git config dco.exempt`: a
maintainer whose commits the `DCO` check exempts sets it to `true` in the
clone, and the hook then prints nothing.
