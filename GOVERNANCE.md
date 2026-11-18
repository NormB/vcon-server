# Governance

## Roles

| Role | Who | What they do |
|---|---|---|
| Maintainer | The people listed in [MAINTAINERS.md](MAINTAINERS.md): today the repository owner, NormB on GitHub | Reviews and merges pull requests, publishes releases, sets the [roadmap](https://vcon-server.com/docs/roadmap/), handles security reports as [SECURITY.md](SECURITY.md) describes, and enforces the [Code of Conduct](CODE_OF_CONDUCT.md) |
| Committer | No one besides the maintainer today | Merges reviewed pull requests to `main` |
| Contributor | Anyone whose pull request merges | Follows [CONTRIBUTING.md](CONTRIBUTING.md): the test written first, the hooks turned on, reserved test data only; signs the [Contributor License Agreement](CLA.md) before a first pull request merges; follows the [Code of Conduct](CODE_OF_CONDUCT.md) |

Anyone may open an issue, a pull request or a
[Discussion](https://github.com/NormB/vcon-server/discussions).

## Becoming a committer

A contributor with several merged, reviewed pull requests can ask for commit
access in an issue or a Discussion. The maintainer decides. Access follows
the rules in [MAINTAINERS.md](MAINTAINERS.md#getting-commit-access): the
GitHub account has two-factor authentication turned on, and every commit
carries a signature that GitHub verifies, because the `main` branch refuses
any other. The pull request that grants access adds the person to
MAINTAINERS.md and to `.github/CODEOWNERS`, and changes
[How the project decides](#how-the-project-decides) to say how the
committers and the maintainer decide together.

## How the project decides

- A change is a pull request against `main`, discussed on the pull request.
- A change to the structure of the project (a new crate, a new storage
  backend, a change to what the 1.0 release keeps compatible) starts as an
  issue, so the discussion comes before the code.
- The [roadmap](https://vcon-server.com/docs/roadmap/) lists what comes next.
- While the project has one maintainer, the maintainer decides, in the open
  on the issue or the pull request.
- Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## If the maintainer is unavailable

These are facts about the project as it stands, not plans.

- The project has one maintainer and no named successor.
- The code is licensed under the [MIT License](LICENSE-MIT) or the
  [Apache License, Version 2.0](LICENSE-APACHE), at your option. Anyone may
  fork it, build it and publish the fork under those terms; a fork needs no
  permission.
- Each release archive can be checked against the build that made it, with
  the [GitHub CLI](https://cli.github.com/), as SECURITY.md's
  [Verifying a release](SECURITY.md#verifying-a-release) describes:

  ```sh
  gh attestation verify vcon-server-$VERSION-x86_64-unknown-linux-gnu.tar.gz \
    --bundle vcon-server-$VERSION.intoto.jsonl --repo NormB/vcon-server
  ```

- A release goes to three places, each published by a job of
  `.github/workflows/release.yml`, and each controlled by the maintainer's
  accounts:

  | Place | Release job | What is there |
  |---|---|---|
  | [GitHub Releases](https://github.com/NormB/vcon-server/releases) | `publish` | The archives, the checksums, their signature and the build provenance |
  | [crates.io](https://crates.io/crates/vcon-server) | `crates-io` | The crates `vcon` and `vcon-server` |
  | [vcon-server.com](https://vcon-server.com/) | `site` | The documentation, deployed to GitHub Pages |

- Only the newest release receives fixes
  ([Supported versions](SECURITY.md#supported-versions)). A user who needs
  to stay on one version can pin it and keep a copy of its archive or of its
  source.
- The [development page](https://vcon-server.com/docs/development/) explains
  the layout of the code, how to build and test it, and every gate.

## Reporting a vulnerability

Report a security problem in private, never in an issue, as
[SECURITY.md](SECURITY.md#reporting-a-vulnerability) describes.

## Changing this document

This document changes by pull request, like any other file.
