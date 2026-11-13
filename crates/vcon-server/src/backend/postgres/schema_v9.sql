-- Schema version 9: the count of a counted audit record takes any larger
-- number.
--
-- Version 8 let an UPDATE of an audit log add exactly one to the count of
-- a counted record. A server adds up the refused requests of a minute in
-- memory and writes the sum once: for a record that was there, that was
-- one statement for each request. From this version one UPDATE writes the
-- sum: it passes when the record is a counted one, the new count is
-- larger than the old one, and no other column is different. A count
-- only grows; it is not set to what it is, and not to less.
--
-- Everything else is as version 8 made it. Every other UPDATE, every
-- DELETE and every TRUNCATE is refused, for the owner as for the
-- application role. The triggers of version 8 run this function by its
-- name, so none is made or dropped, and CREATE OR REPLACE keeps the
-- function's owner and its privileges (none for PUBLIC). No row is
-- changed.
CREATE OR REPLACE FUNCTION vcon.audit_append_only() RETURNS trigger
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
    IF OLD.counted AND NEW.count > OLD.count
        AND to_jsonb(NEW) - 'count' = to_jsonb(OLD) - 'count' THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'an audit record is not changed, but for a larger count of a counted record'
        USING ERRCODE = 'insufficient_privilege';
END
$$;
