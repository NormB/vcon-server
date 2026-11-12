-- Version 5 of the catalog schema, applied by `vcon-server migrate` as
-- vcon_owner in one transaction, to a database at version 4.
--
-- It adds the sessions of the users who signed in: one row for each live
-- session. The same change is schema version 8 of the local catalogs
-- (crate::catalog).
--
-- A session is found by the SHA-256 of its id; neither the id nor the
-- CSRF token is kept, only the hash of each, and a hash is 32 bytes. The
-- scopes the session was given and the authentication methods its sign-in
-- showed are sealed, as a membership's roles are. The times are in
-- nanoseconds. The rows are the registry's, not a tenant's: the tenant
-- column names where the session works (no tenant for an operator's), and
-- a tenant's transaction cannot read or change a row.
CREATE TABLE vcon.session (
    id_hash bytea PRIMARY KEY CHECK (octet_length(id_hash) = 32),
    user_id bytea NOT NULL,
    tenant bytea,
    scopes_sealed bytea NOT NULL,
    created_at bigint NOT NULL,
    last_seen_at bigint NOT NULL,
    idle_expires_at bigint NOT NULL,
    expires_at bigint NOT NULL,
    idle_seconds bigint NOT NULL,
    amr_sealed bytea NOT NULL,
    csrf_hash bytea NOT NULL CHECK (octet_length(csrf_hash) = 32)
);

-- The two ways sessions are removed in number: a user's when its roles
-- change or it is deactivated, and a tenant's when its role cap changes
-- or it is removed.
CREATE INDEX session_of_user ON vcon.session (user_id);
CREATE INDEX session_in_tenant ON vcon.session (tenant);

-- Row-level security and the grant, as version 2 gives the registry's
-- users and memberships.
ALTER TABLE vcon.session ENABLE ROW LEVEL SECURITY;
ALTER TABLE vcon.session FORCE ROW LEVEL SECURITY;
CREATE POLICY registry_rows ON vcon.session
    USING (vcon.in_registry() = (id_hash IS NOT NULL))
    WITH CHECK (vcon.in_registry());

GRANT SELECT, INSERT, UPDATE, DELETE ON vcon.session TO vcon_app;
