-- Version 2 of the catalog schema, applied by `vcon-server migrate` as
-- vcon_owner in one transaction, to a database at version 1.
--
-- It adds the registry's users and their memberships, and lets both
-- audit logs hold no status. The same change is schema version 5 of the
-- local catalogs (crate::catalog).

-- A user: a person who signs in. The label, and the issuer and subject
-- an identity provider knows the user by, are sealed; identity_mac is the
-- HMAC of the issuer and subject together, for the lookup at sign-in and
-- for the rule that one active user has an identity. A local account has
-- neither. A deactivated user keeps its row, for the logs that name its
-- id, and no longer holds its identity.
CREATE TABLE vcon.user_account (
    id bytea PRIMARY KEY,
    label_sealed bytea NOT NULL,
    identity_mac bytea,
    identity_sealed bytea,
    created_at bigint NOT NULL,
    deactivated_at bigint,
    CHECK ((identity_mac IS NULL) = (identity_sealed IS NULL))
);
CREATE UNIQUE INDEX user_account_identity ON vcon.user_account (identity_mac)
    WHERE identity_mac IS NOT NULL AND deactivated_at IS NULL;

-- A membership: a user's roles in one tenant, or as an operator (no
-- tenant). A user has at most one of each, which the two indexes hold,
-- since a unique index holds nothing over a NULL tenant. The rows are the
-- registry's, not a tenant's: the tenant column names where the roles
-- count, and a tenant's transaction cannot read it.
CREATE TABLE vcon.membership (
    user_id bytea NOT NULL,
    tenant bytea,
    roles_sealed bytea NOT NULL,
    granted_at bigint NOT NULL
);
CREATE UNIQUE INDEX membership_in_tenant ON vcon.membership (user_id, tenant)
    WHERE tenant IS NOT NULL;
CREATE UNIQUE INDEX membership_as_operator ON vcon.membership (user_id)
    WHERE tenant IS NULL;
CREATE INDEX membership_of_tenant ON vcon.membership (tenant);

-- A record that answers no request, such as a listener that filled, has
-- no status: NULL, where a local catalog before its version 5 held 0. No
-- release wrote a row to either table at version 1, so there is no 0 to
-- change here.
ALTER TABLE vcon.audit ALTER COLUMN status DROP NOT NULL;
ALTER TABLE vcon.admin_audit ALTER COLUMN status DROP NOT NULL;

-- Row-level security, as version 1 gives the registry's other tables.
DO $$
DECLARE
    name text;
    key_column text;
BEGIN
    FOR name, key_column IN VALUES ('user_account', 'id'), ('membership', 'user_id')
    LOOP
        EXECUTE format('ALTER TABLE vcon.%I ENABLE ROW LEVEL SECURITY', name);
        EXECUTE format('ALTER TABLE vcon.%I FORCE ROW LEVEL SECURITY', name);
        EXECUTE format('CREATE POLICY registry_rows ON vcon.%I '
                       'USING (vcon.in_registry() = (%I IS NOT NULL)) '
                       'WITH CHECK (vcon.in_registry())', name, key_column);
    END LOOP;
END
$$;

GRANT SELECT, INSERT, UPDATE, DELETE ON vcon.user_account, vcon.membership TO vcon_app;
