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
gives the steps.
