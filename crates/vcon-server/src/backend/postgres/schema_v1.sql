-- Version 1 of the catalog schema, applied by `vcon-server migrate` as
-- vcon_owner in one transaction.
--
-- Sensitive values are never in this schema in the clear: `*_sealed` is
-- AES-256-GCM under a per-row key, and `*_mac` is HMAC-SHA256 under an
-- index key (crate::envelope). Times are nanoseconds since the epoch, as
-- in the local catalog, except audit.at, written as the local log writes
-- it. Every key begins with the tenant, since row-level security does not
-- apply to unique checks.

CREATE SCHEMA vcon;

CREATE TABLE vcon.schema_version (
    version integer PRIMARY KEY CHECK (version > 0),
    applied_at timestamptz NOT NULL DEFAULT now()
);

-- The version installed, for the check every new connection makes before
-- it has a tenant: the table itself is not granted to vcon_app.
CREATE FUNCTION vcon.installed_version() RETURNS integer
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = pg_catalog, pg_temp
    AS $$ SELECT max(version) FROM vcon.schema_version $$;

-- The tenant of this transaction, from `SET LOCAL vcon.tenant`. It raises
-- when none is set, which is also how a later transaction on a pooled
-- connection reads the setting an earlier one left empty, and for the
-- registry's marker, so a query cannot pass by finding the value absent.
CREATE FUNCTION vcon.tenant() RETURNS bytea
    LANGUAGE plpgsql STABLE
    SET search_path = pg_catalog, pg_temp
    AS $$
DECLARE
    value text := current_setting('vcon.tenant', true);
BEGIN
    IF value IS NULL OR value = '' THEN
        RAISE EXCEPTION 'no tenant set for this transaction'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF value = 'registry' THEN
        RAISE EXCEPTION 'this transaction is the registry''s, not a tenant''s'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF value !~ '^[0-9a-f]{32}$' THEN
        RAISE EXCEPTION 'vcon.tenant is not a tenant id'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN decode(value, 'hex');
END
$$;

-- True in a registry transaction, which sets vcon.tenant to `registry`;
-- it raises otherwise, so the registry's rows are refused to a transaction
-- with no tenant and to a tenant's.
CREATE FUNCTION vcon.in_registry() RETURNS boolean
    LANGUAGE plpgsql STABLE
    SET search_path = pg_catalog, pg_temp
    AS $$
DECLARE
    value text := current_setting('vcon.tenant', true);
BEGIN
    IF value IS NULL OR value = '' THEN
        RAISE EXCEPTION 'no tenant set for this transaction'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF value <> 'registry' THEN
        RAISE EXCEPTION 'this transaction is a tenant''s, not the registry''s'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN true;
END
$$;

-- The tenant's rows of the catalog.

CREATE TABLE vcon.vcon (
    tenant bytea NOT NULL,
    uuid_mac bytea NOT NULL,
    uuid_sealed bytea NOT NULL,
    current bytea,
    deleted_at bigint,
    hold bigint NOT NULL DEFAULT 0,
    stored_at bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant, uuid_mac)
);
CREATE INDEX vcon_listed ON vcon.vcon (tenant, stored_at, uuid_mac);

CREATE TABLE vcon.version (
    tenant bytea NOT NULL,
    version bytea NOT NULL,
    uuid_mac bytea NOT NULL,
    form_sealed bytea NOT NULL,
    gzip_sealed bytea NOT NULL,
    size_sealed bytea NOT NULL,
    stored_at bigint NOT NULL,
    hidden_at bigint,
    tenant_key_id bytea NOT NULL,
    wrapped_key bytea NOT NULL,
    place_file bytea,
    place_group bigint,
    place_row bigint,
    PRIMARY KEY (tenant, version)
);
CREATE INDEX version_uuid ON vcon.version (tenant, uuid_mac);
CREATE INDEX version_place ON vcon.version (tenant, place_file);

CREATE TABLE vcon.tombstone (
    tenant bytea NOT NULL,
    uuid_mac bytea NOT NULL,
    uuid_sealed bytea NOT NULL,
    purged_at bigint NOT NULL,
    PRIMARY KEY (tenant, uuid_mac)
);

CREATE TABLE vcon.key_use (
    tenant bytea NOT NULL,
    tenant_key_id bytea NOT NULL,
    wraps bigint NOT NULL,
    PRIMARY KEY (tenant, tenant_key_id)
);

CREATE TABLE vcon.parquet_file (
    tenant bytea NOT NULL,
    file_id bytea NOT NULL,
    date text NOT NULL,
    rows bigint NOT NULL,
    live_rows bigint NOT NULL,
    bytes bigint NOT NULL,
    created_at bigint NOT NULL,
    unnamed_at bigint,
    PRIMARY KEY (tenant, file_id)
);

CREATE TABLE vcon.purge_pending (
    tenant bytea NOT NULL,
    file_id bytea NOT NULL,
    PRIMARY KEY (tenant, file_id)
);

CREATE TABLE vcon.manifest (
    tenant bytea NOT NULL,
    seq bigint GENERATED ALWAYS AS IDENTITY,
    published_at bigint,
    PRIMARY KEY (tenant, seq)
);

CREATE TABLE vcon.audit (
    tenant bytea NOT NULL,
    seq bigint GENERATED ALWAYS AS IDENTITY,
    at text NOT NULL,
    actor text NOT NULL,
    action text NOT NULL,
    target_mac bytea,
    target_sealed bytea,
    version text,
    status bigint NOT NULL,
    count bigint NOT NULL,
    counted boolean NOT NULL DEFAULT false,
    PRIMARY KEY (tenant, seq)
);
CREATE INDEX audit_counted ON vcon.audit (tenant, actor, action, status, at);

-- The row each tenant's write transactions lock: no row is a removed
-- tenant.
CREATE TABLE vcon.tenant_lock (
    tenant bytea PRIMARY KEY
);

-- The id of the catalog key the tenant's columns are sealed under, which
-- reseal moves in the same transaction as the columns.
CREATE TABLE vcon.tenant_catalog (
    tenant bytea PRIMARY KEY,
    catalog_key_id bytea NOT NULL
);

-- The registry: rows of no one tenant.

CREATE TABLE vcon.tenant (
    id bytea PRIMARY KEY,
    name_mac bytea NOT NULL UNIQUE,
    name_sealed bytea NOT NULL,
    retention_sealed bytea,
    metadata_sealed bytea,
    created_at bigint NOT NULL,
    tenant_key_id bytea NOT NULL,
    wrapped_tenant_key bytea NOT NULL,
    pending_wrapped_tenant_key bytea,
    key_since bigint NOT NULL
);

CREATE TABLE vcon.master_key_use (
    master_key_id bytea PRIMARY KEY,
    wraps bigint NOT NULL,
    since bigint NOT NULL
);

CREATE TABLE vcon.api_key (
    id bytea PRIMARY KEY,
    tenant bytea,
    token_hash bytea NOT NULL,
    scopes_sealed bytea NOT NULL,
    label_sealed bytea NOT NULL,
    created_at bigint NOT NULL,
    revoked_at bigint
);
CREATE INDEX api_key_tenant ON vcon.api_key (tenant);

CREATE TABLE vcon.admin_audit (
    seq bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at text NOT NULL,
    actor text NOT NULL,
    action text NOT NULL,
    target_mac bytea,
    target_sealed bytea,
    version text,
    status bigint NOT NULL,
    count bigint NOT NULL,
    counted boolean NOT NULL DEFAULT false
);
CREATE INDEX admin_audit_counted ON vcon.admin_audit (actor, action, status, at);

-- Row-level security on every table but schema_version, which vcon_app
-- may not read at all. Each policy compares the transaction's tenant or
-- registry marker with a row, for the planner: it reads the function
-- while it estimates the comparison, so a table with no rows still refuses
-- a query with no tenant, instead of returning nothing because the policy
-- was never evaluated (tests: schema_tests).
--
-- That refusal comes from planning. A prepared statement whose plan the
-- server has cached is not planned again, and the policy is then
-- evaluated only for a row the plan examines: with no tenant set such a
-- statement is refused at the first row, and on a table with no rows, or
-- with an index condition that finds none, it ends with nothing and no
-- error. No plan returns or changes a row with no tenant set (tests:
-- schema_tests::cached). The code that runs a statement sets the tenant;
-- the policy is the second check.
DO $$
DECLARE
    name text;
    key_column text;
BEGIN
    FOREACH name IN ARRAY ARRAY['vcon', 'version', 'tombstone', 'key_use', 'parquet_file',
                                'purge_pending', 'manifest', 'audit', 'tenant_lock',
                                'tenant_catalog']
    LOOP
        EXECUTE format('ALTER TABLE vcon.%I ENABLE ROW LEVEL SECURITY', name);
        EXECUTE format('ALTER TABLE vcon.%I FORCE ROW LEVEL SECURITY', name);
        EXECUTE format('CREATE POLICY tenant_rows ON vcon.%I '
                       'USING (tenant = vcon.tenant()) WITH CHECK (tenant = vcon.tenant())', name);
    END LOOP;
    FOR name, key_column IN VALUES ('tenant', 'id'), ('master_key_use', 'master_key_id'),
                                   ('api_key', 'id'), ('admin_audit', 'seq')
    LOOP
        EXECUTE format('ALTER TABLE vcon.%I ENABLE ROW LEVEL SECURITY', name);
        EXECUTE format('ALTER TABLE vcon.%I FORCE ROW LEVEL SECURITY', name);
        EXECUTE format('CREATE POLICY registry_rows ON vcon.%I '
                       'USING (vcon.in_registry() = (%I IS NOT NULL)) '
                       'WITH CHECK (vcon.in_registry())', name, key_column);
    END LOOP;
END
$$;

-- The application role: the rows, and for the audit logs nothing but
-- SELECT, INSERT and an UPDATE of the count.
REVOKE ALL ON SCHEMA vcon FROM PUBLIC;
GRANT USAGE ON SCHEMA vcon TO vcon_app;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA vcon FROM PUBLIC;
GRANT EXECUTE ON FUNCTION vcon.installed_version(), vcon.tenant(), vcon.in_registry() TO vcon_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON
    vcon.vcon, vcon.version, vcon.tombstone, vcon.key_use, vcon.parquet_file,
    vcon.purge_pending, vcon.manifest, vcon.tenant_lock, vcon.tenant_catalog,
    vcon.tenant, vcon.master_key_use, vcon.api_key
    TO vcon_app;
GRANT SELECT, INSERT ON vcon.audit, vcon.admin_audit TO vcon_app;
GRANT UPDATE (count) ON vcon.audit, vcon.admin_audit TO vcon_app;
