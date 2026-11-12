-- Version 6 of the catalog schema, applied by `vcon-server migrate` as
-- vcon_owner in one transaction, to a database at version 5.
--
-- It adds an access key's expiry and its last-used time, both in
-- nanoseconds. NULL in expires_at is no expiry, and NULL in last_used_at
-- is a key no request has used. The same change is schema version 9 of
-- the local catalogs (crate::catalog).
--
-- The columns are the registry's, under the row-level security and the
-- grants version 1 gave the table, so a tenant's transaction cannot read
-- or change them. No row is changed: a column added with no default is
-- NULL in every row there is, so a key made before this version has no
-- expiry and keeps working.
ALTER TABLE vcon.api_key ADD COLUMN expires_at bigint;
ALTER TABLE vcon.api_key ADD COLUMN last_used_at bigint;
