+++
title = "Stability"
description = "What a release keeps compatible with the release before it: the surfaces of the contract, which changes break a client and which do not, and how to compare the HTTP API, the command line, the configuration file, the MCP tools, the environment variables, the on-disk formats and the audit records of two releases, and the check CI runs on each pull request."
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
| Configuration file | every key of the [configuration file](@/docs/configuration.md), its type, whether a file must set it and its default, in [contract/config.json](https://github.com/NormB/vcon-server/blob/main/contract/config.json) | `contract_diff.py --kind config` |
| MCP tools | every tool of the [MCP server](@/docs/mcp.md), its argument schema, its hints and the scope a key needs to call it, in [contract/mcp.json](https://github.com/NormB/vcon-server/blob/main/contract/mcp.json) | `contract_diff.py --kind mcp` |
| Environment variables | each variable the [table below](#environment-variables) lists, who reads it and what its value means, in [contract/env.json](https://github.com/NormB/vcon-server/blob/main/contract/env.json) | `contract_diff.py --kind env` |
| On-disk formats | each [on-disk format](@/docs/on-disk-format.md), its version, whether the server reads it back, and the versions each release since 0.4.0 wrote, in [contract/formats.json](https://github.com/NormB/vcon-server/blob/main/contract/formats.json) | `contract_diff.py --kind formats` |
| Audit records | the members of an [audit record](@/docs/audit-log-records.md) and every action a record can name, in [contract/audit.json](https://github.com/NormB/vcon-server/blob/main/contract/audit.json) | `contract_diff.py --kind audit` |

[On-disk formats](@/docs/on-disk-format.md) also states the rule each new
build keeps for the data of an older one, and CI opens the data of each
release since 0.4.0 with every change.

## Environment variables

The code of vcon-server reads only the variables this table lists as read
by a command, and the AWS SDK it links reads the others. A test reads the
source of every crate and refuses a variable the code reads outside its
tests that the table does not list.

| Variable | Read by | Meaning |
|---|---|---|
| `NO_COLOR` | `vcon-server tui` | Set to anything but the empty string, it turns color off. |
| `TERM` | `vcon-server tui` | `dumb`, or not set, turns color off. |
| `AWS_ACCESS_KEY_ID` | the AWS SDK | The access key id of the S3 backend, when the configuration sets no `storage.s3.access_key_id`. |
| `AWS_SECRET_ACCESS_KEY` | the AWS SDK | The secret access key that goes with `AWS_ACCESS_KEY_ID`. |

The rows read by the AWS SDK are those of its default credential chain
that the [configuration page](@/docs/configuration.md) names. The chain
reads other variables too, such as those of a profile or a web identity:
the SDK documents them, and they are not part of this contract.

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

### The configuration file

Every table of the file refuses a key it does not know, so a file written
for the older release holds only that release's keys. A change is breaking
when such a file stops loading or acts otherwise:

- a key is removed;
- the type of a key changes, as `integer` to `string`;
- a key becomes required, or a new required key appears at the top of the
  file or in a table an older file may have;
- a key a file had to set only with its table, such as `storage.s3.region`
  under `[storage.s3]`, must now be set in every file;
- the default of a key a file may leave out changes or is removed.

A new optional key, a required key in a new table, a key that is no longer
required and a default stated where there was none are additive.

A key is required when the server refuses the file without it: always, as
`listen`, or with its table, as `storage.s3.region`. A key whose removal
breaks a rule between keys, such as one half of a credential pair, is not
required, since a file that sets neither half loads.

### The MCP tools

An MCP client calls a tool with arguments and acts on its hints, so a
change is breaking when a call the older release took fails, or a hint
the client trusted no longer holds:

- a tool is removed;
- the scope a key needs to call it changes, so a key that could call it may
  be refused;
- a tool is no longer read-only, becomes destructive, is no longer
  idempotent, or reaches outside the server;
- its arguments change in a way the writer rule above refuses, such as a
  new required argument, a removed one, a value of a list removed or a
  changed default.

A new tool, a hint that becomes safer and an argument change the writer
rule allows are additive. Each tool's `$ref`s are read in its own schema.

### The environment variables

A deployment sets variables for the server, so a change is breaking when
a variable it sets no longer acts as it did:

- a variable is no longer read;
- the meaning of its value changes;
- another program or command reads it in place of the one that did.

A new variable is additive. Space and letter case in the text of a meaning
are not compared.

### The on-disk formats

A new build opens the data an older release wrote, and programs such as
DuckDB read the files `export parquet` writes, so a change is breaking
when:

- a format is removed, or the server no longer reads it back;
- a version falls, or its name changes (`vcon-server.parquet-storage/1` to
  another name);
- the version of a format the server never reads, an export format, rises:
  a program that reads the older version may not read the newer one;
- the tests no longer open the data a release wrote, or the versions a
  release wrote are stated otherwise.

A new format, a new release with its data, and a higher version of a
format the server reads back are additive: the build migrates the older
data on its first open, and the tests open each release's data to hold
that.

### The audit records

A program reads the audit logs, so a change is breaking when a member of a
record or an action is removed. A new member and a new action are additive:
a program that reads the logs must pass over a member or an action it does
not know.

### What the script does not compare

`info` (whose `version` changes with each release), descriptions, summaries
and examples. A `oneOf`, `anyOf`, `allOf` or `not` that changes is reported
as breaking, since the script does not compare inside them.

On the command line, the text of `--help` and the names it gives arguments
and values. contract/cli.json is read from `--help`, so a flag counts as one
that may be given more than once only when `--help` shows `...` after its
value.

In the configuration file, the range of a value, the values a text key
takes (such as `local` and `s3` for `storage.objects`) and the rules
between keys: contract/config.json holds each key's TOML type only. The
[configuration page](@/docs/configuration.md) states those, and the server
names the key it refuses.

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
config: no baseline at v0.4.10
mcp: no baseline at v0.4.10
env: no baseline at v0.4.10
formats: no baseline at v0.4.10
audit: no baseline at v0.4.10
```

A surface the tag does not hold prints `no baseline at <tag>` and passes.
The six files under contract/ are first in the release after 0.4.10.

CI runs the same comparison on every pull request, as the job
`Contract`. It is advisory until 1.0: a pull request can merge while it is
red. Its script picks the newest release tag (`vMAJOR.MINOR.PATCH`; a tag
such as `v0.5.0-rc1` is not a release) and names it first:

```sh
python3 -I scripts/contract_job.py
```

While that tag's openapi.json lacks the open-enum mark, as releases up to
0.4.10 do, the job prints a note before the changes: `contract: note:`,
then "Each problem type added since then is reported as breaking for
every response that can carry a problem document, until the next release
carries the mark."

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

## Check a configuration file against a new release

Each release from the one after 0.4.10 holds contract/config.json. Set
`OLD` to the release a file was written for and `NEW` to the release to
upgrade to, write each one's file, then compare the files:

```sh
git show "$OLD:contract/config.json" > old-config.json
git show "$NEW:contract/config.json" > new-config.json
python3 -I scripts/contract_diff.py --kind config old-config.json new-config.json
```

Each line names a key, dotted with its table:

```text
breaking limits.body: default changed from '`16777216`' to '`8388608`'
additive storage.s3.sse: optional key added
contract_diff: 2 change(s), 1 breaking
```

A `breaking` line touches a file only through the key it names: a removed
or retyped key breaks a file that sets it, and a changed default changes a
file that leaves the key out. Look for each named key in the file, then
set it, remove it or change its value as the
[configuration page](@/docs/configuration.md) of the new release says.

contract/config.json is written from the configuration code's tests, never
by hand. After a change to a key, its type or its default, write it again
from the top of the clone and commit it with the change:

```sh
VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --lib config::tests::contract
git diff contract/config.json
```

## Check the MCP tools between two releases

Each release from the one after 0.4.10 holds contract/mcp.json. Set `OLD`
to the release an MCP client was written for and `NEW` to the release to
upgrade to, write each one's file, then compare the files:

```sh
git show "$OLD:contract/mcp.json" > old-mcp.json
git show "$NEW:contract/mcp.json" > new-mcp.json
python3 -I scripts/contract_diff.py --kind mcp old-mcp.json new-mcp.json
```

Each line names the tool, then a `$` path into its arguments:

```text
breaking admin_tenant_create: needs the scope 'tenant_admin' where it needed 'admin'
additive vcon_get $.view: optional property added
contract_diff: 2 change(s), 1 breaking
```

A `breaking` line names a tool a client of the older release may call:
check the client's calls of that tool, and the scopes of the key it uses,
against the [MCP page](@/docs/mcp.md) of the new release.

contract/mcp.json is written from the server's tool list, never by hand.
After a change to a tool, its arguments or its hints, write it again from
the top of the clone and commit it with the change:

```sh
VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --lib mcp::tests::contract
git diff contract/mcp.json
```

## Check the environment between two releases

Each release from the one after 0.4.10 holds contract/env.json. Set `OLD`
to the release a deployment was set up for and `NEW` to the release to
upgrade to, write each one's file, then compare the files:

```sh
git show "$OLD:contract/env.json" > old-env.json
git show "$NEW:contract/env.json" > new-env.json
python3 -I scripts/contract_diff.py --kind env old-env.json new-env.json
```

Each line names a variable:

```text
breaking TERM: variable no longer read, so a setting of it no longer acts
additive COLUMNS: variable added
contract_diff: 2 change(s), 1 breaking
```

A `breaking` line names a variable a deployment may set: look for it in
the service's unit file or container settings, and set it as the new
release's [table](#environment-variables) says.

contract/env.json is written from the table above, which a test holds to
the source, never by hand. After a change to a variable the code reads or
to the table, write it again from the top of the clone and commit it with
the change:

```sh
VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --test env_contract
git diff contract/env.json
```

## Check the on-disk formats between two releases

Each release from the one after 0.4.10 holds contract/formats.json. Set
`OLD` to the release that wrote the data and `NEW` to the release to
upgrade to, write each one's file, then compare the files:

```sh
git show "$OLD:contract/formats.json" > old-formats.json
git show "$NEW:contract/formats.json" > new-formats.json
python3 -I scripts/contract_diff.py --kind formats old-formats.json new-formats.json
```

Each line names a format, or a release and the versions it wrote:

```text
additive catalog: version raised from '9' to '10'; the build opens the older version's data
additive release 0.4.11: release added: the tests open its data
contract_diff: 2 change(s), 0 breaking
```

A higher version of a format the server reads back means the first open
of the older data migrates it, and an older binary cannot open it after
that: take a copy first, as [on-disk formats](@/docs/on-disk-format.md)
says. A `breaking` line on an export format names files a program such as
DuckDB reads: check that program against the new version.

contract/formats.json is written from the build's format list and
on-disk-format.md, never by hand. After a change to a format's version or
to the page's tables, write it again from the top of the clone and commit
it with the change:

```sh
VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --test formats contract_snapshot
git diff contract/formats.json
```

## Check the audit records between two releases

Each release from the one after 0.4.10 holds contract/audit.json. Set
`OLD` to the release a log reader was written for and `NEW` to the release
to upgrade to, write each one's file, then compare the files:

```sh
git show "$OLD:contract/audit.json" > old-audit.json
git show "$NEW:contract/audit.json" > new-audit.json
python3 -I scripts/contract_diff.py --kind audit old-audit.json new-audit.json
```

Each line names a member or an action:

```text
additive action role_cap_set: action added
breaking member count: member removed
contract_diff: 2 change(s), 1 breaking
```

A `breaking` line names a member or an action a log reader may look for:
check the reader against [audit log records](@/docs/audit-log-records.md)
of the new release.

contract/audit.json is written from a record the audit log writes and the
actions audit-log-records.md lists, never by hand. After a change to a
record's members or to an action, write it again from the top of the
clone and commit it with the change:

```sh
VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --test audit_contract
git diff contract/audit.json
```

## Read the result

Each line is one change: `breaking` or `additive`, where it is (the
method and path, then the parameter, the status and media type, and a
`$` path into the schema; the command and its flag, argument or exit
code; the dotted key of the configuration file; the MCP tool and a `$`
path into its arguments; the environment variable; the format or the
release; or the member or the action of an audit record), and what
changed. The script exits 0 when no change is breaking,
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
- **``a configuration snapshot has no `keys` object; nothing was decided``.**
  One of the files is not a contract/config.json: check the tag given to
  `git show`. A release up to 0.4.10 has no such file.
- **`contract/config.json is not what the configuration code reads`.** The
  lib test `config::tests::contract` (run by the commit hook and CI) found
  a key, a type or a default that differs from the snapshot, and names the
  first line that differs. Write the snapshot again with the command
  above, read the diff, and commit it.
- **``an MCP snapshot has no `tools` object; nothing was decided``.** One
  of the files is not a contract/mcp.json: check the tag given to
  `git show`. A release up to 0.4.10 has no such file.
- **`contract/mcp.json is not what tools/list shows`.** The lib test
  `mcp::tests::contract` (run by the commit hook and CI) found a tool, an
  argument schema, a hint or a scope that differs from the snapshot, and
  names the first line that differs. Write the snapshot again with the
  command above, read the diff, and commit it.
- **``an environment snapshot has no `variables` object; nothing was decided``.**
  One of the files is not a contract/env.json: check the tag given to
  `git show`. A release up to 0.4.10 has no such file.
- **`contract/env.json is not what the source and this page give`.** The
  test `env_contract` (run by the commit hook and CI) found a row of the
  table above that differs from the snapshot, and names the first line
  that differs. Write the snapshot again with the command above, read the
  diff, and commit it. The same test names each variable the code reads
  that the table does not list, and each read whose name it cannot see: a
  name that is not a literal, or `std::env::vars`.
- **``a formats snapshot has no `formats` or `releases` object; nothing was decided``.**
  One of the files is not a contract/formats.json: check the tag given to
  `git show`. A release up to 0.4.10 has no such file.
- **`contract/formats.json is not what this build and on-disk-format.md give`.**
  The test `formats` (run by the commit hook and CI) found a format, a
  version or a release that differs from the snapshot, and names the first
  line that differs. Write the snapshot again with the command above, read
  the diff, and commit it.
- **``an audit snapshot has no `members` or `actions` list; nothing was decided``.**
  One of the files is not a contract/audit.json: check the tag given to
  `git show`. A release up to 0.4.10 has no such file.
- **`contract/audit.json is not what the audit log writes and audit-log-records.md lists`.**
  The test `audit_contract` (run by the commit hook and CI) found a member
  or an action that differs from the snapshot, and names the first line
  that differs. Write the snapshot again with the command above, read the
  diff, and commit it.
- **`usage:`.** The arguments are not one of the nine forms the usage
  line shows.
