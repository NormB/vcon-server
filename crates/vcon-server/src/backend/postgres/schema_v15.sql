-- Schema version 15: the registry records its tenant creates under way,
-- and each tenant its pack files under way; the registry holds the leases
-- of the work one server does for every server.
--
-- A tenant's create makes its space (the tenant's lock row and its
-- prefix in the bucket) and then, in a transaction of its own, the
-- tenant's row in vcon.tenant. Start-up recovery removes every space
-- that no tenant row names, as a leftover of a create cut off. With
-- several servers on one database, the space of another server's create
-- under way would look the same.
--
-- From this version a create adds a row to vcon.pending_tenant, with the
-- time it began in nanoseconds, in the transaction before it makes the
-- space, and deletes it in the transaction that adds the tenant's row.
-- Recovery keeps the space of a create whose row is younger than the
-- write lease, and deletes a row it takes for a leftover before it
-- removes the space; a create whose row is gone at its last transaction
-- adds no tenant row. The tenant id is random and in the clear in
-- vcon.tenant already, and the time is a time: both are in the clear.
--
-- The rows are the registry's: read and written in its transactions
-- only. The application role may read the rows, add them and delete
-- them. It has no reason to change one: a create begins once.
--
-- No row of another table is read or changed. The table starts empty:
-- migrate holds the database's lock, which a serving server holds too,
-- so no create is under way while it runs.
CREATE TABLE vcon.pending_tenant (
    id bytea PRIMARY KEY,
    started_at bigint NOT NULL
);
ALTER TABLE vcon.pending_tenant ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.pending_tenant FORCE ROW LEVEL SECURITY;
CREATE POLICY registry_rows ON vcon.pending_tenant
    USING (vcon.in_registry() = (id IS NOT NULL)) WITH CHECK (vcon.in_registry());
GRANT SELECT, INSERT, DELETE ON vcon.pending_tenant TO vcon_app;

-- A pack run and a purge's rewrite put a new pack file and then, in a
-- transaction of their own, add the file's row to vcon.parquet_file.
-- Recovery removes every pack file no row names, so with several servers
-- on one database the file of another server's run would look the same.
--
-- From this version a run adds a row to vcon.pending_file, with the
-- time its writing began in nanoseconds, in a transaction before it puts
-- the file, and deletes it in the transaction that records the file.
-- Recovery keeps a file whose row is younger than the write lease, and
-- deletes a row it takes for a leftover before it removes the file; a
-- run whose row is gone at its last transaction records nothing. The
-- file id is random and in the clear in vcon.parquet_file already.
--
-- The rows are a tenant's, under the policy of vcon.pending_write. The
-- application role may read the rows, add them and delete them: a file's
-- writing begins once. The table starts empty, as vcon.pending_tenant
-- does.
CREATE TABLE vcon.pending_file (
    tenant bytea NOT NULL,
    file_id bytea NOT NULL,
    started_at bigint NOT NULL,
    PRIMARY KEY (tenant, file_id)
);
ALTER TABLE vcon.pending_file ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.pending_file FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_rows ON vcon.pending_file
    USING (tenant = vcon.tenant()) WITH CHECK (tenant = vcon.tenant());
GRANT SELECT, INSERT, DELETE ON vcon.pending_file TO vcon_app;

-- From this version the time of a row of vcon.pending_write,
-- vcon.pending_tenant and vcon.pending_file is the database's now(), in
-- nanoseconds, written by the statement that adds the row, and recovery
-- compares it with the database's now(): servers whose host clocks differ
-- by more than the write lease agree on which write is under way. No
-- statement here changes a row: a row a release before this one wrote
-- has the time of its server's clock, and is a leftover once that time
-- is the write lease or more from the database's now().

-- Several servers on one database share two jobs that one of them does
-- for all: the purge sweep, with the recovery that runs before it, and
-- the pack run. From this version vcon.lease holds a row per job while a
-- server holds it: the job's name, the holder (random bits that name one
-- connect of one process, then its host and process id, for an
-- operator), the time it was taken and the time it ends.
--
-- Both times are by the database's clock, now() in the one statement
-- that takes or renews a lease, and no server's own clock is read: two
-- servers whose clocks disagree agree on who holds a job and until when.
-- A holder's lease is renewed by its holder, taken by another only once
-- it has ended, and given up by deleting the row.
--
-- The rows are the registry's: read and written in its transactions
-- only. The application role may read the rows, add them, change them
-- (a renewal, or the take of a lease that ended) and delete them. The
-- table starts empty, as vcon.pending_tenant does.
CREATE TABLE vcon.lease (
    name text PRIMARY KEY,
    holder text NOT NULL,
    taken_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL
);
ALTER TABLE vcon.lease ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.lease FORCE ROW LEVEL SECURITY;
CREATE POLICY registry_rows ON vcon.lease
    USING (vcon.in_registry() = (name IS NOT NULL)) WITH CHECK (vcon.in_registry());
GRANT SELECT, INSERT, UPDATE, DELETE ON vcon.lease TO vcon_app;
