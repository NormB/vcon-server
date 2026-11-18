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

Each commit carries a DCO sign-off
([Signing off commits](CONTRIBUTING.md#signing-off-commits)), which the
required `DCO` check reads. A verified commit that the repository owner
authored and committed needs no sign-off, and neither does a verified
commit by a GitHub bot account. The
[Contributor License Agreement](CLA.md) retired on 2026-11-18;
contributions made before that day were made under it
([The retired Contributor License Agreement](CONTRIBUTING.md#the-retired-contributor-license-agreement)).

The owner made the switch from the agreement to the sign-off in this
order:

1. Done on 2026-11-18. The `main` ruleset requires `DCO` and no longer
   requires `license/cla`.
2. Done on 2026-11-18. The pull request that took `DCO` off `ADVISORY` in
   `scripts/check_required_checks.py`, and `license/cla` off `EXTERNAL`,
   landed. Both lists are empty now.
3. Done on 2026-11-18. CLA Assistant is off for the repository.

The commit hook refuses a commit without a sign-off. It cannot see
GitHub's signature verdict, which the owner exemption needs, so it reads
`git config dco.exempt`: the owner sets it to `true` in the owner's clone,
and the hook then skips the sign-off step. The personal-data scan of the
message still runs.
