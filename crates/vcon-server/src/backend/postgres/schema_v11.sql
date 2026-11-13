-- Schema version 11: each tenant's audit log has a key of its own.
--
-- Until this version the target of an audit record was sealed under the
-- keys of the tenant's catalog key. A rotation of the tenant key seals
-- every sealed column again under the new catalog key, and since version
-- 8 no role can change an audit record: the two rules could not both
-- hold.
--
-- From this version a tenant's log has an audit key: random, made with
-- the tenant, and stored in vcon.audit_key sealed under the keys of the
-- tenant's catalog key. The target of a record and its MAC are made with
-- the keys of the audit key, and the record names that key in the new
-- column vcon.audit.key_id. A rotation of the tenant key seals the audit
-- key again, which is one UPDATE of vcon.audit_key.wrapped, and changes
-- no audit record. The rule of version 9 stays as it is: it compares
-- every column of a record but its count, and so the new column too.
--
-- A record names its key so that a log can be given a new audit key
-- later: new records are then sealed under the new key, and the records
-- before them stay readable under the key they name, which is kept. For
-- that the application role may read a key, make one and seal one again,
-- and may not delete one; a record cannot name a key its tenant does not
-- have, and a key that a record names cannot be deleted by any role.
-- ordinal orders a tenant's keys, from 1: the key with the largest seals
-- the new records. The server writes it with the tenant's lock row held;
-- it is no column a sequence fills, so it needs no trigger of version 8.
--
-- No deployed database has an audit record: a server of the preview stops
-- after it connects. So no record is converted, and the new column is
-- added with no value: no row is rewritten and no trigger runs. A record
-- that names no key and has a target cannot be read.
--
-- The constraint that a record names a key of its tenant is added NOT
-- VALID, which checks every record written from now on and does not read
-- the records that are there. Each of those has no value in the column
-- this same transaction adds, and the constraint asks nothing of such a
-- record. The read that a plain ADD CONSTRAINT makes runs as the owner
-- under the forced policies, with no tenant set, and is refused.
--
-- The registry's log, vcon.admin_audit, is not changed by this version.
CREATE TABLE vcon.audit_key (
    tenant bytea NOT NULL,
    key_id bytea NOT NULL,
    ordinal bigint NOT NULL CHECK (ordinal > 0),
    wrapped bytea NOT NULL,
    PRIMARY KEY (tenant, key_id),
    UNIQUE (tenant, ordinal)
);
ALTER TABLE vcon.audit_key ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.audit_key FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_rows ON vcon.audit_key
    USING (tenant = vcon.tenant()) WITH CHECK (tenant = vcon.tenant());
GRANT SELECT, INSERT ON vcon.audit_key TO vcon_app;
GRANT UPDATE (wrapped) ON vcon.audit_key TO vcon_app;
ALTER TABLE vcon.audit ADD COLUMN key_id bytea;
ALTER TABLE vcon.audit ADD CONSTRAINT audit_sealed_by
    FOREIGN KEY (tenant, key_id) REFERENCES vcon.audit_key (tenant, key_id) NOT VALID;
