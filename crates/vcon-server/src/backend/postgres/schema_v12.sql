-- Schema version 12: the database names the key the registry is sealed
-- under, and the registry's audit log has a key of its own.
--
-- Until this version the database held no id of the catalog key that
-- seals the registry's columns, as vcon.tenant_catalog holds one for each
-- tenant. A registry opened under another master key was refused only at
-- the first sealed value it read, and a rotation of the master key had no
-- row that says which key the registry is under.
--
-- From this version the one row of vcon.registry_key holds that id. The
-- server writes it when it first opens the registry, compares it at every
-- later open, and a rotation of the master key moves it in the same
-- transaction as the columns. The table takes one row: its primary key is
-- a column that holds one value.
--
-- Until this version the target of a record of the registry's log,
-- vcon.admin_audit, was sealed under the keys of the registry's catalog
-- key. A rotation of the master key seals every sealed column again, and
-- since version 8 no role can change an audit record: the two rules could
-- not both hold, as for a tenant's log before version 11.
--
-- From this version the registry's log has an audit key: random, made
-- when the registry is first opened, and stored in vcon.admin_audit_key
-- sealed under the registry's keys. The target of a record and its MAC
-- are made with the keys of the audit key, and the record names that key
-- in the new column vcon.admin_audit.key_id. A rotation of the master key
-- seals the audit key again, which is one UPDATE of
-- vcon.admin_audit_key.wrapped, and changes no audit record. The rule of
-- version 9 stays as it is: it compares every column of a record but its
-- count, and so the new column too.
--
-- As for a tenant's log, the application role may read a key, make one
-- and seal one again, and may not delete one; a record cannot name a key
-- the log does not have, and a key that a record names cannot be deleted
-- by any role. ordinal orders the keys, from 1: the key with the largest
-- seals the new records.
--
-- No deployed database has a row of the registry: a server of the preview
-- stops after it connects. So no record is converted, and the new column
-- is added with no value: no row is rewritten and no trigger runs. A
-- record that names no key and has a target cannot be read.
--
-- The constraint that a record names a key of the log is added NOT VALID,
-- which checks every record written from now on and does not read the
-- records that are there: the read that a plain ADD CONSTRAINT makes runs
-- as the owner under the forced policies, with no marker set, and is
-- refused.
CREATE TABLE vcon.registry_key (
    one boolean PRIMARY KEY DEFAULT true CHECK (one),
    catalog_key_id bytea NOT NULL
);
CREATE TABLE vcon.admin_audit_key (
    key_id bytea PRIMARY KEY,
    ordinal bigint NOT NULL UNIQUE CHECK (ordinal > 0),
    wrapped bytea NOT NULL
);
ALTER TABLE vcon.registry_key ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.registry_key FORCE ROW LEVEL SECURITY;
CREATE POLICY registry_rows ON vcon.registry_key
    USING (vcon.in_registry() = (catalog_key_id IS NOT NULL)) WITH CHECK (vcon.in_registry());
ALTER TABLE vcon.admin_audit_key ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.admin_audit_key FORCE ROW LEVEL SECURITY;
CREATE POLICY registry_rows ON vcon.admin_audit_key
    USING (vcon.in_registry() = (key_id IS NOT NULL)) WITH CHECK (vcon.in_registry());
GRANT SELECT, INSERT ON vcon.registry_key, vcon.admin_audit_key TO vcon_app;
GRANT UPDATE (catalog_key_id) ON vcon.registry_key TO vcon_app;
GRANT UPDATE (wrapped) ON vcon.admin_audit_key TO vcon_app;
ALTER TABLE vcon.admin_audit ADD COLUMN key_id bytea;
ALTER TABLE vcon.admin_audit ADD CONSTRAINT admin_audit_sealed_by
    FOREIGN KEY (key_id) REFERENCES vcon.admin_audit_key (key_id) NOT VALID;
