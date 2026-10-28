# Contributing to vcon-server

## Orientation

Start with the [development page](https://vcon-server.com/docs/development/):
the layout of the workspace, how to build and test, how the code is tested,
the commit hooks and CI. The [security page](https://vcon-server.com/docs/security/)
says what the server guarantees; a change must keep every guarantee there.

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).
[GOVERNANCE.md](GOVERNANCE.md) says who decides and what a contributor and the
maintainer each do.

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
hooks on every pull request.

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
