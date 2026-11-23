-- Schema version 14: a tenant's catalog records its writes under way.
--
-- A create or a replace seals the new version and writes its object to
-- the bucket between two transactions, so for that time the bucket holds
-- an object that no row of vcon.version names. Start-up recovery removes
-- every such object as a leftover of a write cut off. With several
-- servers on one database, the object of another server's write under
-- way would look the same.
--
-- From this version a write adds a row to vcon.pending_write, with the
-- time it began in nanoseconds, in the transaction before it writes the
-- object, and deletes it in the transaction that adds the version's row.
-- Recovery keeps the object of a write whose pending row is younger than
-- the write lease, and deletes a pending row it takes for a leftover
-- before it removes the object; a write whose pending row is gone at its
-- last transaction adds no row. The version id is random and in the
-- bucket's object names, and the time is a time: both are in the clear.
--
-- The application role may read the rows, add them and delete them. It
-- has no reason to change one: a write begins once.
--
-- No row of another table is read or changed. The table starts empty:
-- migrate holds the database's lock, which a serving server holds too,
-- so no write is under way while it runs.
CREATE TABLE vcon.pending_write (
    tenant bytea NOT NULL,
    version bytea NOT NULL,
    started_at bigint NOT NULL,
    PRIMARY KEY (tenant, version)
);
ALTER TABLE vcon.pending_write ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.pending_write FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_rows ON vcon.pending_write
    USING (tenant = vcon.tenant()) WITH CHECK (tenant = vcon.tenant());
GRANT SELECT, INSERT, DELETE ON vcon.pending_write TO vcon_app;
