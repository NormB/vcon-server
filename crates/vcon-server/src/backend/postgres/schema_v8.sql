-- Version 8 of the catalog schema, applied by `vcon-server migrate` as
-- vcon_owner in one transaction, to a database at version 7.
--
-- It makes the server the only source of a row's number, and the two
-- audit logs append-only for every role, the owner included. The local
-- catalogs need neither: a local catalog is one process's file, and that
-- process is the only writer.
--
-- No row is changed and no privilege is granted.

-- The number of a row. Each seq is an identity column, GENERATED ALWAYS,
-- which refuses a value an INSERT gives, unless the statement says
-- OVERRIDING SYSTEM VALUE: with that clause any role that may insert chose
-- the number (found on vcon.version, PostgreSQL 18.6 and 16.15), and
-- UPDATE ... SET seq = DEFAULT gave a row there is a new one. The newest
-- version of a vCon and the order of an audit log are read by seq, so
-- the trigger below takes the number of a new row from the column's
-- sequence whatever the statement says, and keeps the number of a row
-- that is updated. A row stored the plain way has taken one number for
-- its default already, so the numbers of a table rise by two: they give
-- an order, and no count. The function runs as its owner, since the
-- application role is given no use of a sequence.
CREATE FUNCTION vcon.number_row() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = pg_catalog, pg_temp
    AS $$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        NEW.seq := OLD.seq;
    ELSE
        NEW.seq := nextval(pg_get_serial_sequence(
            format('%I.%I', TG_TABLE_SCHEMA, TG_TABLE_NAME), 'seq')::regclass);
    END IF;
    RETURN NEW;
END
$$;

-- An audit log takes new records. A record there is changes in one way:
-- the count of a counted record goes up by one, which is how the server
-- counts the refused requests of a minute in one record. Every other
-- UPDATE, every DELETE and every TRUNCATE is refused, for the owner as
-- for the application role, which version 1 gave SELECT, INSERT and
-- UPDATE (count) only. The comparison is of the whole row but its count,
-- so a column a later version adds is held without a change here.
--
-- A row trigger runs for a row: a DELETE or an UPDATE that finds no
-- record is no error and changes nothing. The owner can drop a trigger;
-- that role is for migrations and is not the one a server logs in as.
CREATE FUNCTION vcon.audit_append_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
    AS $$
BEGIN
    IF TG_OP = 'TRUNCATE' THEN
        RAISE EXCEPTION 'an audit log is not emptied'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'an audit record is not deleted'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF OLD.counted AND NEW.count = OLD.count + 1
        AND to_jsonb(NEW) - 'count' = to_jsonb(OLD) - 'count' THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'an audit record is not changed, but for one more in the count of a counted record'
        USING ERRCODE = 'insufficient_privilege';
END
$$;

REVOKE ALL ON FUNCTION vcon.number_row(), vcon.audit_append_only() FROM PUBLIC;

-- The triggers of one table and one event run in the order of their
-- names: on a log, append_only sees an UPDATE's row before number_rows
-- puts its number back, so a statement that asks for a new number is
-- refused there.
DO $$
DECLARE
    name text;
BEGIN
    FOREACH name IN ARRAY ARRAY['version', 'manifest', 'audit', 'admin_audit']
    LOOP
        EXECUTE format('CREATE TRIGGER number_rows BEFORE INSERT OR UPDATE ON vcon.%I '
                       'FOR EACH ROW EXECUTE FUNCTION vcon.number_row()', name);
    END LOOP;
    FOREACH name IN ARRAY ARRAY['audit', 'admin_audit']
    LOOP
        EXECUTE format('CREATE TRIGGER append_only BEFORE UPDATE OR DELETE ON vcon.%I '
                       'FOR EACH ROW EXECUTE FUNCTION vcon.audit_append_only()', name);
        EXECUTE format('CREATE TRIGGER not_emptied BEFORE TRUNCATE ON vcon.%I '
                       'FOR EACH STATEMENT EXECUTE FUNCTION vcon.audit_append_only()', name);
    END LOOP;
END
$$;
