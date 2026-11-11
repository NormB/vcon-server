-- Version 4 of the catalog schema, applied by `vcon-server migrate` as
-- vcon_owner in one transaction, to a database at version 3.
--
-- It adds the settings a tenant's administrator sets: one row for each
-- name a tenant has set. A name with no row has its default. The same
-- change is schema version 7 of the local catalogs (crate::catalog).
--
-- The name is one of the fixed list in crate::settings and is kept as it
-- is written, so the key of a row is the tenant and the name; the value
-- is sealed as the tenant table's settings are. The rows are the
-- registry's, not a tenant's: the tenant column names whose setting a
-- row is, and a tenant's transaction cannot read or change it.
CREATE TABLE vcon.tenant_setting (
    tenant bytea NOT NULL,
    name text NOT NULL,
    value_sealed bytea NOT NULL,
    PRIMARY KEY (tenant, name)
);

-- Row-level security and the grant, as version 2 gives the registry's
-- users and memberships.
ALTER TABLE vcon.tenant_setting ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.tenant_setting FORCE ROW LEVEL SECURITY;
CREATE POLICY registry_rows ON vcon.tenant_setting
    USING (vcon.in_registry() = (tenant IS NOT NULL))
    WITH CHECK (vcon.in_registry());

GRANT SELECT, INSERT, UPDATE, DELETE ON vcon.tenant_setting TO vcon_app;
