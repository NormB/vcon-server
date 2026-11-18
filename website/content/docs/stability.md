+++
title = "Stability"
description = "What a release keeps compatible with the release before it: the surfaces of the contract, which changes break a client and which do not, and how to compare the HTTP API and the command line of two releases."
weight = 59

[extra]
group = "reference"
+++

This page says which parts of vcon-server a program or an operator can
build on, which changes to them break that program, and how to check two
releases against each other.

vcon-server is at 0.4. Until 1.0, a release may change any surface, and
the [changelog](https://github.com/NormB/vcon-server/blob/main/CHANGELOG.md)
names each change. The versions follow
[Semantic Versioning](https://semver.org/): from 1.0 on, a release that
breaks a surface of the contract is a new major version.

## The surfaces of the contract

| Surface | What it holds | The check |
|---|---|---|
| HTTP API | every path, operation, parameter, request body, response status, media type, response header and body schema in [openapi.json](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/openapi.json) | `contract_diff.py --kind openapi` |
| Problem types | the type of each [problem document](@/docs/http-api.md), one page each under `/problems/` | `contract_diff.py --kind problems` |
| Command line | every command, and each flag and argument it takes: whether it is required, whether it takes a value, its default and its possible values, in [contract/cli.json](https://github.com/NormB/vcon-server/blob/main/contract/cli.json) | `contract_diff.py --kind cli` |
| Exit codes | 0, 1 and 2, with the meaning [the command line page](@/docs/command-line.md#output-and-exit-codes) gives each, also in contract/cli.json | `contract_diff.py --kind cli` |

The configuration file, the MCP tools, the environment
variables the server reads, the on-disk formats and the audit records have
no check of this kind yet. [On-disk formats](@/docs/on-disk-format.md)
states the rule each new build keeps for the data of an older one, and CI
opens the data of each release since 0.4.0 with every change.

## What the contract leaves out

- The Rust API of the crates `vcon` and `vcon-server`.
- Text meant for people: log lines, `--help` prose, the `detail` of a
  problem document, and the pages of this site.
- A version prefix in the paths. The paths have none, such as `/v1`, and
  keep none: the contract holds each path as it is.

## Which changes break a client

A change breaks a client when a program written against the older release
can fail against the newer one. Every other change is additive. The script
`scripts/contract_diff.py` applies these rules and prints each change with
its kind.

### What the server sends

The client reads a response body, so the rule is the reader's. A
change is breaking when:

- a member disappears, or stops being required, so it may be absent;
- a value can take a type the older release never sent, `null` included;
- a closed list of values gains a value, or loses its list;
- a fixed value (`const`) changes;
- a response status, a media type of a response or a response header
  disappears.

A new member, status, media type or header is additive.

### What a client sends

The client writes a request body, a query parameter, a path parameter and
a request header, so the rule is the writer's. A change is
breaking when the server stops accepting a value the older release
accepted:

- a type is dropped, as `number` to `integer`;
- a value of a list is removed, or a list or a fixed value is set where
  there was none;
- a lower bound rises or an upper bound falls (`minimum`, `maximum`,
  `minLength`, `maxLength`, `minItems`, `maxItems` and the like);
- a `pattern` or a `format` changes;
- the items of an array must now be unique;
- members the schema does not name are now refused;
- a member is removed, or becomes required, or a new required member
  appears;
- a parameter is removed, becomes required, or a new required one appears;
- a request body becomes required, or one of its media types is removed;
- a default changes or is removed: a client that leaves the value out gets
  other behavior.

A new optional member or parameter, a new value of a list, a wider bound
and a new media type are additive.

### The operations

Removing an operation is breaking, and so is a security requirement an
older client met that the server no longer accepts, such as a route that
starts to need a key. A new operation is additive.

### Problem types and open lists

Removing a problem type is breaking: a client may act on the type. A new
problem type is additive. In openapi.json the problem `type` member carries
`"x-vcon-server-open-enum": true`, the mark of a list that may grow, so a
client must handle a type it does not know by the response's status code.
The mark counts when both releases carry it. Releases up to 0.4.10 do not,
so a comparison with one of them reports each new problem type as a
breaking change to every response that can carry a problem document.

### The command line

A script runs the binary, so a change is breaking when a command line the
older release took fails or acts otherwise on the newer one:

- a command, a flag, a flag's short name or an argument is removed;
- a flag or an argument becomes required, or a new required one appears;
- a flag that took a value stops taking one, or the other way round, or
  its value may no longer be left out;
- a default changes or is removed;
- a possible value is removed, or a list of possible values is set where
  there was none;
- a flag that could be given more than once no longer can;
- an exit code is removed, changes its meaning, or a new one appears: a
  script that tells the codes apart knows only the older release's.

A new command, a new optional flag or argument, a new possible value and a
default stated where there was none are additive. An argument is its place
on the command line: the first, the second.

### What the script does not compare

`info` (whose `version` changes with each release), descriptions, summaries
and examples. A `oneOf`, `anyOf`, `allOf` or `not` that changes is reported
as breaking, since the script does not compare inside them.

On the command line, the text of `--help` and the names it gives arguments
and values. contract/cli.json is read from `--help`, so a flag counts as one
that may be given more than once only when `--help` shows `...` after its
value.

## Before you compare two releases

- A clone of the repository with its tags. A clone made before the newer
  release needs `git fetch --tags`.
- Python 3. The script needs no package beyond the standard library.

```sh
git clone https://github.com/NormB/vcon-server
cd vcon-server
```

## Compare the work tree with a release

From the top of the clone, name the release tag:

```sh
python3 -I scripts/contract_diff.py --against v0.4.10
```

It reads each surface at the tag and in the work tree, and prints the
changes of each, then its count:

```text
openapi: 0 change(s), 0 breaking
problems: 0 change(s), 0 breaking
cli: no baseline at v0.4.10
```

A surface the tag does not hold prints `no baseline at <tag>` and passes.
contract/cli.json is first in the release after 0.4.10.

## Compare two releases

Write each release's openapi.json to a file, then compare the files:

```sh
git show v0.4.7:crates/vcon-server/openapi.json > old.json
git show v0.4.8:crates/vcon-server/openapi.json > new.json
python3 -I scripts/contract_diff.py --kind openapi old.json new.json
```

Between those two releases every change is additive:

```text
additive GET /legal/vcons/{uuid} query view: optional parameter added
additive GET /legal/vcons/{uuid} 200 application/json: media type added
additive GET /legal/vcons/{uuid}/versions/{version} query view: optional parameter added
additive GET /legal/vcons/{uuid}/versions/{version} 200 application/json: media type added
additive GET /vcons/{uuid} 200 application/json $.recordings: property added
contract_diff: 5 change(s), 0 breaking
```

For two directories of problem pages, `--kind problems OLD_DIR NEW_DIR`
compares the page names.

## Check the command line between two releases

Each release from the one after 0.4.10 holds contract/cli.json. Set `OLD`
and `NEW` to two such release tags, write each one's file, then compare
the files:

```sh
git show "$OLD:contract/cli.json" > old-cli.json
git show "$NEW:contract/cli.json" > new-cli.json
python3 -I scripts/contract_diff.py --kind cli old-cli.json new-cli.json
```

Each line names the command, then the flag, `<N>` for the command's Nth
argument, or `exit N` for an exit code:

```text
additive vcon-server key create --dry-run: optional flag added
breaking vcon-server tenant remove --force: flag removed
contract_diff: 2 change(s), 1 breaking
```

contract/cli.json is written from the binary, never by hand. After a
change to a command, its flags or the exit codes on the
[command line page](@/docs/command-line.md#output-and-exit-codes), write
it again from the top of the clone and commit it with the change:

```sh
VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --test cli_contract
git diff contract/cli.json
```

## Read the result

Each line is one change: `breaking` or `additive`, where it is (the
method and path, then the parameter, the status and media type, and a
`$` path into the schema; or the command and its flag, argument or exit
code), and what changed. The script exits 0 when no change is breaking,
1 when one is, and 2 when it could not decide.

## When the check fails

- **Exit 1.** Each `breaking` line names a change that can fail a client
  of the older release. In a pull request, make the change additive: add
  an optional parameter or member in place of a required one, and keep the
  old member beside a new one. From 1.0 on, a change that must break a
  client waits for a new major version.
- **`'v9' names no commit; nothing was decided`.** The clone has no such
  tag: run `git fetch --tags`, and check the name with `git tag`.
- **`is not local to the document; nothing was decided`.** A `$ref` names
  another file. The script follows only references inside the document,
  those that start with `#/`.
- **``an OpenAPI document has no `paths` object; nothing was decided``.**
  One of the files is not an OpenAPI document: check the path given to
  `git show`.
- **``a command line snapshot has no `commands` object; nothing was decided``.**
  One of the files is not a contract/cli.json: check the tag given to
  `git show`. A release up to 0.4.10 has no such file.
- **`contract/cli.json is not what the binary's --help shows`.** The test
  `cli_contract` (run by the commit hook and CI) found a command line that
  differs from the snapshot, and names the first line that differs. Write
  the snapshot again with the command above, read the diff, and commit it.
- **`usage:`.** The arguments are not one of the four forms the usage
  line shows.
