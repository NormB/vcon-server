-- Version 7 of the catalog schema, applied by `vcon-server migrate` as
-- vcon_owner in one transaction, to a database at version 6.
--
-- It gives every version the order it was stored in, and lets the
-- application role vacuum the tables a purge deletes from. The local
-- catalogs need neither: SQLite numbers its rows as they are stored, and
-- checkpoints its own log.
--
-- seq is the order of storing within the database. Two versions of one
-- vCon can hold the same stored_at, a time in nanoseconds from the
-- server's clock, and the version id is random; the newest version of a
-- vCon and the order of its versions are read by stored_at and then seq.
-- The number is the server's, not the application's: GENERATED ALWAYS
-- refuses a value an INSERT gives. The rows there are take numbers in
-- the order the server reads them.
ALTER TABLE vcon.version ADD COLUMN seq bigint GENERATED ALWAYS AS IDENTITY;

-- After a purge the server runs VACUUM on these tables, so the bytes of
-- deleted rows leave the table files sooner. From PostgreSQL 17 a role
-- that does not own a table may vacuum it with the MAINTAIN privilege;
-- before 17 only the owner may, the application role's VACUUM is skipped
-- with a warning, and autovacuum does the work later.
DO $$
BEGIN
    IF current_setting('server_version_num')::integer >= 170000 THEN
        EXECUTE 'GRANT MAINTAIN ON vcon.version, vcon.vcon, vcon.tombstone TO vcon_app';
    END IF;
END
$$;
