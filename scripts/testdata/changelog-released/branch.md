# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Library: a `KeyWrap` trait for the backend that wraps tenant keys.**
  - Effect: `vcon_server::key_wrap` holds `KeyWrap` and `FileWrap`, the
    master key file backend the server uses; stored data is unchanged.
  - Limit: It has one backend, the master key file; the roadmap plans the
    others.
- **On-disk format `key-wrap` version 1 records a key a key backend wrapped.**
  - Effect: A reader knows which backend and key opens a wrapped key before
    it asks any backend.
  - Limit: This release writes none; the other backends come in later
    releases.
  - Docs: [On-disk formats](https://vcon-server.com/docs/on-disk-format/)

### Changed

- **The roadmap gives each milestone a target month and lists what is not
  planned.**
  - Effect: The plan runs through December 2027, and each month is a target,
    not a promise.
  - Docs: [Roadmap](https://vcon-server.com/docs/roadmap/)
- **The roadmap and the 1.0 checklist plan six key wrapping backends.**
  - Effect: Azure Key Vault and Google Cloud KMS join HashiCorp Vault
    Transit, OpenBao, PKCS#11 and AWS KMS.
  - Docs: [1.0 checklist](https://vcon-server.com/docs/production-checklist/)

### Fixed

- **Readers on an older Parquet manifest no longer lose it early.**
  - Was: A start-up, a delete or a sweep removed every manifest but the
    newest, inside `storage.parquet.reader_grace`.
  - Effect: An older manifest stays for the reader grace unless a purge
    rewrites a pack file it names.
  - Docs: [Parquet storage](https://vcon-server.com/docs/parquet-storage/)

## [0.4.17] - 2026-11-25

### Changed

- **Upgrading from 0.4.16: copy the data directory first; it is the only way
  back.**
  - Effect: The catalogs in the data directory move to schema version 10 the
    first time 0.4.17 opens them; no `vcon-server migrate` is needed.
  - Limit: 0.4.16 refuses a data directory that 0.4.17 has opened; a
    PostgreSQL catalog stays at version 14, so 0.4.16 still opens it.
  - Docs: [From 0.4.16 to
    0.4.17](https://vcon-server.com/docs/upgrading/#from-0-4-16-to-0-4-17)
- **A tenant's SQLite catalog serves concurrent writes and reads two to
  three times faster.**
  - Effect: Commits made at once share one disk sync; measured 3.0 to 3.2
    times the writes a second and 2.3 to 3.0 times the reads.
  - Limit: After a failed disk sync, that tenant's requests are answered
    `internal` until the server is restarted.
  - Docs: [Durability](https://vcon-server.com/docs/durability/)

### Fixed

- **A count no longer changes the audit record of a single request.**
  - Was: A request recorded at the first instant of a minute could have a
    later count of the same kind added to its record.
  - Effect: A count adds only to a counted record; the catalogs move to
    schema version 10 when the server first opens them.
  - Limit: An older release refuses a data directory this build has
    opened, so copy it before the upgrade.
  - Docs: [Counted
    records](https://vcon-server.com/docs/audit-logs/#counted-records)

## [0.4.16] - 2026-11-25

### Changed

- **The upgrade from 0.4.15 is the binary alone.**
  - Effect: No stored data, configuration or schema changes; 0.4.15 opens a
    data directory or a PostgreSQL database that 0.4.16 has served.
  - Docs: [From 0.4.15 to
    0.4.16](https://vcon-server.com/docs/upgrading/#from-0-4-15-to-0-4-16)

### Fixed

- **Quitting the terminal UI no longer waits for a request under way.**
  - Was: Quitting while the server had not yet answered a request could
    hold the exit for up to the request's 15 s timeout.
  - Effect: The UI ends every request under way and exits at once.
  - Docs: [Use it](https://vcon-server.com/docs/tui/#use-it)

[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.4.17...HEAD
[0.4.17]: https://github.com/NormB/vcon-server/releases/tag/v0.4.17
[0.4.16]: https://github.com/NormB/vcon-server/releases/tag/v0.4.16
[0.4.15]: https://github.com/NormB/vcon-server/releases/tag/v0.4.15
[0.4.14]: https://github.com/NormB/vcon-server/releases/tag/v0.4.14
[0.4.13]: https://github.com/NormB/vcon-server/releases/tag/v0.4.13
[0.4.12]: https://github.com/NormB/vcon-server/releases/tag/v0.4.12
[0.4.11]: https://github.com/NormB/vcon-server/releases/tag/v0.4.11
[0.4.10]: https://github.com/NormB/vcon-server/releases/tag/v0.4.10
[0.4.9]: https://github.com/NormB/vcon-server/releases/tag/v0.4.9
[0.4.8]: https://github.com/NormB/vcon-server/releases/tag/v0.4.8
[0.4.7]: https://github.com/NormB/vcon-server/releases/tag/v0.4.7
[0.4.6]: https://github.com/NormB/vcon-server/releases/tag/v0.4.6
[0.4.5]: https://github.com/NormB/vcon-server/releases/tag/v0.4.5
[0.4.4]: https://github.com/NormB/vcon-server/releases/tag/v0.4.4
[0.4.3]: https://github.com/NormB/vcon-server/releases/tag/v0.4.3
[0.4.2]: https://github.com/NormB/vcon-server/releases/tag/v0.4.2
[0.4.1]: https://github.com/NormB/vcon-server/releases/tag/v0.4.1
[0.4.0]: https://github.com/NormB/vcon-server/releases/tag/v0.4.0
[0.3.0]: https://github.com/NormB/vcon-server/releases/tag/v0.3.0
[0.2.0]: https://github.com/NormB/vcon-server/releases/tag/v0.2.0
[0.1.0]: https://github.com/NormB/vcon-server/releases/tag/v0.1.0
