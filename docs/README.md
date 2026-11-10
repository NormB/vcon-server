# vcon-server documentation

The documentation is published at https://vcon-server.com/docs/. Its
source is the Markdown in [`website/content/docs/`](../website/content/docs/),
which the site is built from; this file lists every page of it. The links
below open each page's source in the repository.

- [Documentation](../website/content/docs/_index.md): Running vcon-server, sending vCons to it, and working on its code.

## Installing

- [Install](../website/content/docs/install.md): Put the vcon-server binary on a Linux host: download a release and check it, install it with cargo, build it from source, or run it in a container. Each path lists what to install first on Debian, Ubuntu and Fedora, what each step prints, and what to do when a step fails.
- [Getting started](../website/content/docs/getting-started.md): From an installed binary to a running server: the service account, memory locking, the master key, a certificate, the configuration, the first tenant and its keys, and the server started under systemd or in the foreground.
- [Run it in a container](../website/content/docs/container.md): Build a container image from a checked release, keep the configuration and the data in volumes, allow the server to lock its memory, make the first keys, run it, back it up, and stop it, with Docker or Docker Compose.
- [Upgrade](../website/content/docs/upgrading.md): Move a running server to a newer release: check the new binary, stop the server, copy the data directory, install the binary, check the data with it, start it, and go back to the old version if you need to.
- [How-to guides](../website/content/docs/how-to.md): Step-by-step guides for installing and running vcon-server. Each shows every command and its output, and CI runs each against a live server.

## Using

- [Store your first vCon](../website/content/docs/first-vcon.md): A first vCon end to end: store it, store it again, see a conflict refused, read it back byte for byte, list it, and delete it, each with a key that holds only the scope it needs, and every answer shown.
- [Storing vCons](../website/content/docs/storing-vcons.md): What vcon-server accepts, what it checks, how versions, deletes and replaces work, and what it never does to a vCon.
- [Producers](../website/content/docs/producers.md): Sending vCons to vcon-server from a recorder, a conserver or any other producer: keys, retries, and what each answer means.
- [Client examples](../website/content/docs/clients.md): Programs in curl, Python, TypeScript, Go and Rust that work through the HTTP API and MCP: store, gzip, list with a cursor, replace, legal hold, audit, admin and MCP calls, and delete.
- [HTTP API](../website/content/docs/http-api.md): Every route: the scope it needs, what it takes, and what it answers.
- [Console](../website/content/docs/console.md): Open the web console the server serves, sign in with an API key, and what the console does and does not keep.
- [Terminal UI](../website/content/docs/tui.md): Browse a tenant's vCons from a terminal with vcon-server tui: the build, the key and client file it needs, the keys, and what each message means.
- [MCP](../website/content/docs/mcp.md): Connecting an AI agent to vcon-server over MCP, and every tool with its parameters and an example.
- [Connect an AI agent over MCP](../website/content/docs/connect-mcp.md): Give an AI agent its own key, then check the MCP endpoint by hand: the handshake and the tool list over HTTPS, the refusals, and the same through vcon-server mcp.
- [Add a second tenant](../website/content/docs/second-tenant.md): Make a second tenant over the admin API while the server runs, give it a key, see that its data and acme's stay apart, change its retention period, and remove it.
- [The vcon library](../website/content/docs/library.md): Read, check and write vCons in your own Rust code with the vcon crate.
- [Command line](../website/content/docs/command-line.md): Every vcon-server command: what it does, what it prints, its exit codes, and which may run beside a serving server.
- [Configuration](../website/content/docs/configuration.md): Every key of the configuration file, its default, and why the default is what it is.

## Operating

- [Operations](../website/content/docs/operations.md): Running vcon-server day to day: the data directory, what may run beside the server, the purge sweep, logs, health checks and backups.
- [Back up and restore](../website/content/docs/backup-and-restore.md): Back up the master key and the data directory while the server runs, restore the data directory, check it, and see what a restore brings back, revoked keys included.
- [Keep vCons in an S3 bucket](../website/content/docs/s3-storage.md): Keep each version's sealed object in an S3-compatible bucket (RustFS, MinIO or AWS S3) instead of the data directory: the bucket and its keys on the storage side, the configuration, a check that the objects go there and that a purge removes them, what each refusal means, and how to clean up.
- [Keep vCons as Parquet files in a bucket](../website/content/docs/parquet-storage.md): Turn on Parquet storage over an S3 bucket: what the bucket must be, the configuration, a check that the server packs its objects into Parquet files and still reads each vCon back, a read of the files with DuckDB, what a purge removes from the bucket and from the storage's disks, and what each refusal means.
- [Keep the catalogs in PostgreSQL](../website/content/docs/postgres-catalog.md): Preview: reach a PostgreSQL server over TLS for the catalogs: the database's side, the roles, the schema and `migrate`, TLS and client certificates, the configuration, the secret files, and what each refusal at start means. This release migrates the schema, connects, checks the server and stops.
- [Export to Parquet](../website/content/docs/export-parquet.md): Write a copy of a tenant's vCons as Parquet files, to a directory or an S3 bucket, beside a running server: choose which clear columns the files carry, read them with DuckDB, check the export, and see what each refusal means.
- [Revoke and rotate keys](../website/content/docs/rotate-keys.md): Revoke an API key over HTTP or on the host, replace a key without a gap, and rotate a tenant's key and the master key, with every command and its output.
- [Tenants and keys](../website/content/docs/tenants-and-keys.md): How tenants keep their data apart, what each key scope allows, and how keys are made and revoked.
- [Retention and legal holds](../website/content/docs/retention-and-holds.md): How long a deleted vCon is kept, when it is erased, how a legal hold stops that, and what the legal routes can read and restore.
- [Audit logs](../website/content/docs/audit-logs.md): What vcon-server records about each request, where, and what it never records.

## Security

- [Security policy](../SECURITY.md): reporting a vulnerability, the response, supported versions, scope, verifying a release, and disclosure.
- [Security](../website/content/docs/security.md): The key hierarchy, what is encrypted with what, how the process protects keys in memory, and what an attacker gets from each thing they might steal.
- [Use keys and scopes](../website/content/docs/keys-and-scopes.md): Make a key for each scope, see what each one may do and what it is refused, and make a key with more than one scope. Every request and its answer is shown.
- [Assurance case](../website/content/docs/assurance-case.md): Why vcon-server's security requirements are met: the threat model, the trust boundaries, how the design principles are applied, and how the common weaknesses are countered.

## Developing

- [Development](../website/content/docs/development.md): The code's layout, how it is tested, the gates every commit passes, and how to work on this site.
- [What is built](../website/content/docs/status.md): Which parts of vcon-server work today, and which are being built.
- [Roadmap](../website/content/docs/roadmap.md): The next milestones for vcon-server.
