-- Schema version 15: the registry records its tenant creates under way.
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
