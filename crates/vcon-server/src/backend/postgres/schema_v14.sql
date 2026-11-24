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

-- A tenant's removal deletes its objects first, then its rows in
-- transactions of their own, the lock row last. A read on another server
-- that took its first step before the objects went could find, in its
-- last step, the vCon's row still there and its object gone, and answer
-- that the version is damaged. From this version the removal's first
-- transaction sets `removing` on the tenant's lock row; it waits for
-- every transaction that holds the row. Every later write or read step
-- that takes the row is refused as a removed tenant's. A removal cut off
-- leaves the row marked: the tenant stays refused, and the removal is
-- made again at the next start, as before. The application role updates
-- the row already (version 1's grant, version 10's lock_updates policy).
ALTER TABLE vcon.tenant_lock ADD COLUMN removing boolean NOT NULL DEFAULT false;
