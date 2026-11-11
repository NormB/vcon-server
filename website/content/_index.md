+++
title = "vcon-server"
description = "vcon-server stores vCons, the IETF format for conversation records, and returns them only to the keys allowed to see them. Each vCon is encrypted at rest under its own key, kept for its tenant's retention period, erased after it, and held past it when a legal hold says so. Every request is recorded."
[extra]
tagline = "encrypted, multi-tenant, audited storage for vCons"
headline = "Encrypted, multi-tenant, audited storage for vCons"
# The target of the hero's primary button.
install_link = "@/docs/install.md"

# The texts around "A first look", inline markdown.
first_look_intro = "Send a vCon with a key that holds the `write` scope, then read it back."
first_look_outro = "Sending the same bytes again answers `200` with the same version, so a producer can retry safely."
# The terminal UI section, inline markdown; its screen comes from
# data/tui-screens.json.
tui_intro = "`vcon-server tui` lists a tenant's vCons in a terminal, over the HTTP API, with one key that holds the `read` scope."
tui_label = "The vcon-server tui list view: the server, the tenant and connected on the first line, then a vCon on each line with its uuid and the times it was stored and updated, the third one selected, and the keys on the last line."
tui_outro = "It shows what `GET /vcons` lists and never asks for a vCon's content, and the server's scopes and audit log apply to it as to any client."
# The Install and How-to sections: their text is the docs' (see the how-to
# pages); the layout is the template's. Plain text, not markdown. Every plain
# key sits above the first [[...]] table, or TOML puts it inside that table.
install_intro = "vcon-server is one binary for Linux on x86_64 or aarch64. These are the release path's steps on Debian or Ubuntu. The install page also covers Fedora, cargo install, a build from source and a container, and what to do when a step fails. CI runs every one of these commands on each change."
howto_intro = "Each guide gives every command and what it prints, and CI runs each against a live server."

# "What it guarantees": one card each. `body` is inline markdown; `icon` names
# a drawing in templates/components.html.
[[extra.features]]
title = "Encrypted at rest, key by key"
icon = "lock"
body = "Each stored version is sealed with AES-256-GCM under its own data key, wrapped by its tenant's key, which is wrapped by the master key. The catalogs are SQLCipher databases."
link = "@/docs/security.md"
link_text = "Security"

[[extra.features]]
title = "Stored exactly as received"
icon = "file"
body = "Unsigned, signed (JWS) and encrypted (JWE) vCons, plain or gzipped, are checked and stored byte for byte. Nothing is rewritten, so a signature stays valid."
link = "@/docs/storing-vcons.md"
link_text = "Storing vCons"

[[extra.features]]
title = "Tenants kept apart"
icon = "tenants"
body = "Each tenant has its own key, catalog, files, retention period and audit log. A key belongs to one tenant and reaches nothing else."
link = "@/docs/tenants-and-keys.md"
link_text = "Tenants and keys"

[[extra.features]]
title = "Kept as long as required, then erased"
icon = "clock"
body = "A deleted vCon is hidden at once and purged once its tenant's retention period has passed since it was stored. A legal hold stops the purge."
link = "@/docs/retention-and-holds.md"
link_text = "Retention and legal holds"

[[extra.features]]
title = "Every request recorded"
icon = "log"
body = "Each tenant has an append-only audit log, and the server keeps one for its own administration. No log holds vCon content or a token."
link = "@/docs/audit-logs.md"
link_text = "Audit logs"

[[extra.features]]
title = "Errors you can act on"
icon = "alert"
body = "Every refusal is an RFC 9457 problem document whose `type` is a page on this site."
link = "@/problems/_index.md"
link_text = "Problem types"

# "A first look": one framed code block per step.

[[extra.first_look]]
title = "Store a vCon"
code = """
curl --fail-with-body https://vcon.example.com/vcons \\
  -H "Authorization: Bearer $VCON_KEY" \\
  -H "Content-Type: application/vcon" \\
  --data-binary @call.vcon
# 201 Created
# Location: /vcons/01a07da8-c2bb-83e5-b9a2-279e0d16bc46
# ETag: "…version id…""""

[[extra.first_look]]
title = "Read it back"
code = """
curl --fail-with-body https://vcon.example.com/vcons/01a07da8-c2bb-83e5-b9a2-279e0d16bc46 \\
  -H "Authorization: Bearer $VCON_KEY""""

[[extra.install]]
title = "Install what a minimal system lacks"
code = """
apt-get update -qq
apt-get install -y -qq sudo curl ca-certificates tar gzip >/dev/null"""
note = "As root, on Debian or Ubuntu. A minimal image has no sudo, curl or CA certificates."

[[extra.install]]
title = "Download the release and check it"
code = """
VERSION=0.4.3
ARCHIVE=vcon-server-$VERSION-$(uname -m)-unknown-linux-gnu.tar.gz
curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$VERSION/$ARCHIVE
curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$VERSION/SHA256SUMS.txt
sha256sum -c SHA256SUMS.txt --ignore-missing"""
note = "sha256sum prints the archive's name and OK. The 0.4.3 binaries need glibc 2.34 or later, such as Debian 12 or 13, Ubuntu 22.04 or 24.04, or Fedora 44; on an older system, install with cargo or build from source."

[[extra.install]]
title = "Unpack it"
code = """
tar -xzf "$ARCHIVE"
ls "${ARCHIVE%.tar.gz}""""
note = "The archive holds vcon-server, its README and the two license files."

[[extra.install]]
title = "Install it"
code = """
sudo install -m 0755 "${ARCHIVE%.tar.gz}/vcon-server" /usr/local/bin/vcon-server
vcon-server --version"""
note = "It prints vcon-server and its version. The install page also checks the archive's build provenance with gh attestation verify."

[[extra.install]]
title = "Set it up and start it"
note = "Getting started makes the service account, the master key, a certificate, the configuration and the first keys, then starts the server under systemd or in the foreground."

[[extra.howto]]
title = "Get a Let's Encrypt certificate"
link = "@/docs/lets-encrypt.md"
summary = "A free, publicly trusted certificate by HTTP-01 or DNS-01 with certbot, renewed automatically, so clients need no CA file."

[[extra.howto]]
title = "Store your first vCon"
link = "@/docs/first-vcon.md"
summary = "Store it, store it again, see a conflict refused, read it back, list it and delete it, with a key for each scope."

[[extra.howto]]
title = "Use keys and scopes"
link = "@/docs/keys-and-scopes.md"
summary = "A key for each scope, what each one may do, and what each one is refused."

[[extra.howto]]
title = "Add a second tenant"
link = "@/docs/second-tenant.md"
summary = "A tenant made over the admin API, its key, and its data kept apart from the first."

[[extra.howto]]
title = "Revoke and rotate keys"
link = "@/docs/rotate-keys.md"
summary = "Revoke an API key, replace one without a gap, and rotate the tenant key and the master key."

[[extra.howto]]
title = "Back up and restore"
link = "@/docs/backup-and-restore.md"
summary = "The master key and the data directory, a restore, and what the restore brings back."

[[extra.howto]]
title = "Run it in a container"
link = "@/docs/container.md"
summary = "An image built from a checked release, named volumes, and the memory-lock limit a container needs."

[[extra.howto]]
title = "Upgrade"
link = "@/docs/upgrading.md"
summary = "A new binary over a running server, the data checked with it, and the way back."

[[extra.howto]]
title = "Connect an AI agent over MCP"
link = "@/docs/connect-mcp.md"
summary = "A key for the agent, the handshake and the tool list over HTTPS and through vcon-server mcp."

[[extra.howto]]
title = "All the guides"
link = "@/docs/how-to.md"
summary = "Every how-to guide, with the install and getting-started pages."
+++

> **Status: 0.4.3 released, not yet for production.** The store, the HTTP API
> with its client routes and OpenAPI description, tenants and their keys,
> retention and the purge sweep, legal holds, the audit logs, the console, the
> terminal UI, the command line, key rotation, `check` and the MCP server
> with its tools are built and tested. From 0.4.0 the sealed objects can be kept in an S3 bucket
> (RustFS, MinIO or AWS S3) and packed there into Parquet files, and a
> tenant's vCons can be exported as Parquet files. See
> [what is built](@/docs/status.md).

## What it is for

A vCon holds a conversation: who took part, the dialog itself (text, or a
recording inline or by URL), and what was learned from it (transcripts,
summaries, analysis). That is some of the most sensitive data a business keeps,
and much of it must be kept for a fixed time and then destroyed, unless a
court says otherwise.

vcon-server is the place a producer sends vCons to and the place they are read
back from: an HTTP API over an encrypted store, with tenants kept apart, every
request recorded, and retention periods and legal holds applied to each vCon.

## Who it is for

- **Operators** who run the server on Linux: [install](@/docs/install.md)
  it, follow [getting started](@/docs/getting-started.md) to a running
  server, [store a first vCon](@/docs/first-vcon.md), then read
  [operations](@/docs/operations.md).
- **Developers** of the recorders, conservers and applications that send
  vCons and read them back: [producers](@/docs/producers.md), the
  [HTTP API](@/docs/http-api.md) and the
  [client examples](@/docs/clients.md) in five languages.
- **Compliance and legal staff** who set how long a conversation is kept
  and who may see it: [retention and legal holds](@/docs/retention-and-holds.md),
  [audit logs](@/docs/audit-logs.md) and [security](@/docs/security.md).
- **AI agents**, and the people who connect them:
  [connect an AI agent over MCP](@/docs/connect-mcp.md).
