-- Schema version 13: the database holds the claim of the bucket prefix.
--
-- A server that keeps its objects in a bucket binds the prefix to its
-- catalogs with a claim, a random id of 32 hexadecimal digits kept both
-- under the prefix and beside the catalogs. Start-up recovery removes
-- every object under the prefix that the catalogs do not name, so a
-- second set of catalogs on the same prefix is refused at start-up: its
-- claim is not the bucket's. With the catalogs in a data directory the id
-- is in the file storage.claim there.
--
-- From this version the one row of vcon.storage_claim holds the id for
-- catalogs in PostgreSQL: every server on the database reads the same
-- claim, and a server on another database is refused the prefix. The
-- table takes one row: its primary key is a column that holds one value.
-- The check on the id is the rule the server reads an id by.
--
-- The application role may read the row and make it, in a transaction of
-- the registry. It may not change the row or delete it: a claim that
-- changed would no longer be the bucket's, and one that became another
-- bucket's would let recovery remove another server's objects.
--
-- No deployed database has a claim: a server of the preview stops after
-- it connects. So the table starts empty and no row of another table is
-- read or changed.
CREATE TABLE vcon.storage_claim (
    one boolean PRIMARY KEY DEFAULT true CHECK (one),
    claim_id text NOT NULL CHECK (claim_id ~ '^[0-9a-f]{32}$')
);
ALTER TABLE vcon.storage_claim ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.storage_claim FORCE ROW LEVEL SECURITY;
CREATE POLICY registry_rows ON vcon.storage_claim
    USING (vcon.in_registry() = (claim_id IS NOT NULL)) WITH CHECK (vcon.in_registry());
GRANT SELECT, INSERT ON vcon.storage_claim TO vcon_app;
