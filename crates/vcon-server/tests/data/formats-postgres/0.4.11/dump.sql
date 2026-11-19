--
-- PostgreSQL database dump
--


-- Dumped from database version 16.15
-- Dumped by pg_dump version 16.15

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: vcon; Type: SCHEMA; Schema: -; Owner: vcon_owner
--

CREATE SCHEMA vcon;


ALTER SCHEMA vcon OWNER TO vcon_owner;

--
-- Name: audit_append_only(); Type: FUNCTION; Schema: vcon; Owner: vcon_owner
--

CREATE FUNCTION vcon.audit_append_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'pg_temp'
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


ALTER FUNCTION vcon.audit_append_only() OWNER TO vcon_owner;

--
-- Name: in_registry(); Type: FUNCTION; Schema: vcon; Owner: vcon_owner
--

CREATE FUNCTION vcon.in_registry() RETURNS boolean
    LANGUAGE plpgsql STABLE
    SET search_path TO 'pg_catalog', 'pg_temp'
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


ALTER FUNCTION vcon.in_registry() OWNER TO vcon_owner;

--
-- Name: installed_version(); Type: FUNCTION; Schema: vcon; Owner: vcon_owner
--

CREATE FUNCTION vcon.installed_version() RETURNS integer
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'pg_temp'
    AS $$ SELECT max(version) FROM vcon.schema_version $$;


ALTER FUNCTION vcon.installed_version() OWNER TO vcon_owner;

--
-- Name: number_row(); Type: FUNCTION; Schema: vcon; Owner: vcon_owner
--

CREATE FUNCTION vcon.number_row() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'pg_temp'
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


ALTER FUNCTION vcon.number_row() OWNER TO vcon_owner;

--
-- Name: tenant(); Type: FUNCTION; Schema: vcon; Owner: vcon_owner
--

CREATE FUNCTION vcon.tenant() RETURNS bytea
    LANGUAGE plpgsql STABLE
    SET search_path TO 'pg_catalog', 'pg_temp'
    AS $_$
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
$_$;


ALTER FUNCTION vcon.tenant() OWNER TO vcon_owner;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: admin_audit; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.admin_audit (
    seq bigint NOT NULL,
    at text NOT NULL,
    actor text NOT NULL,
    action text NOT NULL,
    target_mac bytea,
    target_sealed bytea,
    version text,
    status bigint,
    count bigint NOT NULL,
    counted boolean DEFAULT false NOT NULL,
    key_id bytea
);

ALTER TABLE ONLY vcon.admin_audit FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.admin_audit OWNER TO vcon_owner;

--
-- Name: admin_audit_key; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.admin_audit_key (
    key_id bytea NOT NULL,
    ordinal bigint NOT NULL,
    wrapped bytea NOT NULL,
    CONSTRAINT admin_audit_key_ordinal_check CHECK ((ordinal > 0))
);

ALTER TABLE ONLY vcon.admin_audit_key FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.admin_audit_key OWNER TO vcon_owner;

--
-- Name: admin_audit_seq_seq; Type: SEQUENCE; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.admin_audit ALTER COLUMN seq ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME vcon.admin_audit_seq_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);


--
-- Name: api_key; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.api_key (
    id bytea NOT NULL,
    tenant bytea,
    token_hash bytea NOT NULL,
    scopes_sealed bytea NOT NULL,
    label_sealed bytea NOT NULL,
    created_at bigint NOT NULL,
    revoked_at bigint,
    expires_at bigint,
    last_used_at bigint
);

ALTER TABLE ONLY vcon.api_key FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.api_key OWNER TO vcon_owner;

--
-- Name: audit; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.audit (
    tenant bytea NOT NULL,
    seq bigint NOT NULL,
    at text NOT NULL,
    actor text NOT NULL,
    action text NOT NULL,
    target_mac bytea,
    target_sealed bytea,
    version text,
    status bigint,
    count bigint NOT NULL,
    counted boolean DEFAULT false NOT NULL,
    key_id bytea
);

ALTER TABLE ONLY vcon.audit FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.audit OWNER TO vcon_owner;

--
-- Name: audit_key; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.audit_key (
    tenant bytea NOT NULL,
    key_id bytea NOT NULL,
    ordinal bigint NOT NULL,
    wrapped bytea NOT NULL,
    CONSTRAINT audit_key_ordinal_check CHECK ((ordinal > 0))
);

ALTER TABLE ONLY vcon.audit_key FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.audit_key OWNER TO vcon_owner;

--
-- Name: audit_seq_seq; Type: SEQUENCE; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.audit ALTER COLUMN seq ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME vcon.audit_seq_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);


--
-- Name: key_use; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.key_use (
    tenant bytea NOT NULL,
    tenant_key_id bytea NOT NULL,
    wraps bigint NOT NULL
);

ALTER TABLE ONLY vcon.key_use FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.key_use OWNER TO vcon_owner;

--
-- Name: manifest; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.manifest (
    tenant bytea NOT NULL,
    seq bigint NOT NULL,
    published_at bigint
);

ALTER TABLE ONLY vcon.manifest FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.manifest OWNER TO vcon_owner;

--
-- Name: manifest_seq_seq; Type: SEQUENCE; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.manifest ALTER COLUMN seq ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME vcon.manifest_seq_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);


--
-- Name: master_key_use; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.master_key_use (
    master_key_id bytea NOT NULL,
    wraps bigint NOT NULL,
    since bigint NOT NULL
);

ALTER TABLE ONLY vcon.master_key_use FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.master_key_use OWNER TO vcon_owner;

--
-- Name: membership; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.membership (
    user_id bytea NOT NULL,
    tenant bytea,
    roles_sealed bytea NOT NULL,
    granted_at bigint NOT NULL
);

ALTER TABLE ONLY vcon.membership FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.membership OWNER TO vcon_owner;

--
-- Name: parquet_file; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.parquet_file (
    tenant bytea NOT NULL,
    file_id bytea NOT NULL,
    date text NOT NULL,
    rows bigint NOT NULL,
    live_rows bigint NOT NULL,
    bytes bigint NOT NULL,
    created_at bigint NOT NULL,
    unnamed_at bigint
);

ALTER TABLE ONLY vcon.parquet_file FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.parquet_file OWNER TO vcon_owner;

--
-- Name: purge_pending; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.purge_pending (
    tenant bytea NOT NULL,
    file_id bytea NOT NULL
);

ALTER TABLE ONLY vcon.purge_pending FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.purge_pending OWNER TO vcon_owner;

--
-- Name: registry_key; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.registry_key (
    one boolean DEFAULT true NOT NULL,
    catalog_key_id bytea NOT NULL,
    CONSTRAINT registry_key_one_check CHECK (one)
);

ALTER TABLE ONLY vcon.registry_key FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.registry_key OWNER TO vcon_owner;

--
-- Name: schema_version; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.schema_version (
    version integer NOT NULL,
    applied_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT schema_version_version_check CHECK ((version > 0))
);


ALTER TABLE vcon.schema_version OWNER TO vcon_owner;

--
-- Name: session; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.session (
    id_hash bytea NOT NULL,
    user_id bytea NOT NULL,
    tenant bytea,
    scopes_sealed bytea NOT NULL,
    created_at bigint NOT NULL,
    last_seen_at bigint NOT NULL,
    idle_expires_at bigint NOT NULL,
    expires_at bigint NOT NULL,
    idle_seconds bigint NOT NULL,
    amr_sealed bytea NOT NULL,
    csrf_hash bytea NOT NULL,
    CONSTRAINT session_csrf_hash_check CHECK ((octet_length(csrf_hash) = 32)),
    CONSTRAINT session_id_hash_check CHECK ((octet_length(id_hash) = 32))
);

ALTER TABLE ONLY vcon.session FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.session OWNER TO vcon_owner;

--
-- Name: storage_claim; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.storage_claim (
    one boolean DEFAULT true NOT NULL,
    claim_id text NOT NULL,
    CONSTRAINT storage_claim_claim_id_check CHECK ((claim_id ~ '^[0-9a-f]{32}$'::text)),
    CONSTRAINT storage_claim_one_check CHECK (one)
);

ALTER TABLE ONLY vcon.storage_claim FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.storage_claim OWNER TO vcon_owner;

--
-- Name: tenant; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.tenant (
    id bytea NOT NULL,
    name_mac bytea NOT NULL,
    name_sealed bytea NOT NULL,
    retention_sealed bytea,
    metadata_sealed bytea,
    created_at bigint NOT NULL,
    tenant_key_id bytea NOT NULL,
    wrapped_tenant_key bytea NOT NULL,
    pending_wrapped_tenant_key bytea,
    key_since bigint NOT NULL,
    role_cap_sealed bytea
);

ALTER TABLE ONLY vcon.tenant FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.tenant OWNER TO vcon_owner;

--
-- Name: tenant_catalog; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.tenant_catalog (
    tenant bytea NOT NULL,
    catalog_key_id bytea NOT NULL
);

ALTER TABLE ONLY vcon.tenant_catalog FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.tenant_catalog OWNER TO vcon_owner;

--
-- Name: tenant_lock; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.tenant_lock (
    tenant bytea NOT NULL
);

ALTER TABLE ONLY vcon.tenant_lock FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.tenant_lock OWNER TO vcon_owner;

--
-- Name: tenant_setting; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.tenant_setting (
    tenant bytea NOT NULL,
    name text NOT NULL,
    value_sealed bytea NOT NULL
);

ALTER TABLE ONLY vcon.tenant_setting FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.tenant_setting OWNER TO vcon_owner;

--
-- Name: tombstone; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.tombstone (
    tenant bytea NOT NULL,
    uuid_mac bytea NOT NULL,
    uuid_sealed bytea NOT NULL,
    purged_at bigint NOT NULL
);

ALTER TABLE ONLY vcon.tombstone FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.tombstone OWNER TO vcon_owner;

--
-- Name: user_account; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.user_account (
    id bytea NOT NULL,
    label_sealed bytea NOT NULL,
    identity_mac bytea,
    identity_sealed bytea,
    created_at bigint NOT NULL,
    deactivated_at bigint,
    CONSTRAINT user_account_check CHECK (((identity_mac IS NULL) = (identity_sealed IS NULL)))
);

ALTER TABLE ONLY vcon.user_account FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.user_account OWNER TO vcon_owner;

--
-- Name: vcon; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.vcon (
    tenant bytea NOT NULL,
    uuid_mac bytea NOT NULL,
    uuid_sealed bytea NOT NULL,
    current bytea,
    deleted_at bigint,
    hold bigint DEFAULT 0 NOT NULL,
    stored_at bigint DEFAULT 0 NOT NULL
);

ALTER TABLE ONLY vcon.vcon FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.vcon OWNER TO vcon_owner;

--
-- Name: version; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

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
    seq bigint NOT NULL
);

ALTER TABLE ONLY vcon.version FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.version OWNER TO vcon_owner;

--
-- Name: version_seq_seq; Type: SEQUENCE; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.version ALTER COLUMN seq ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME vcon.version_seq_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);


--
-- Data for Name: admin_audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-19T08:11:12.110207169Z', 'cli', 'tenant_create', '\x3160e83c689f2a78c65cae3b10e872836cb8d7debd38b7075bd4e2ef0fa94d5c', '\x01967df0e1251adb77f33a32d74c47786fdd223bbe9fcf1faa733bf453b1ab3bce546cd8ce23540fe8b0bfdd8d55c62d084fa841f5800198ea2ea2393e', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-19T08:11:13.71717037Z', 'cli', 'key_create', '\x4f2f6835f083aa717695fea6002004e25764cfd82c9aa6cc4919e9f983d86e31', '\x01877ce1b88f27db369ad77ca5d4af455db858cc082f238caa7cf170fa57144b26c0fac9d73971c6cd64e76b4b', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-19T08:11:15.416617791Z', 'cli', 'key_create', '\x469707d70ee923f6404cba4072e242d724c6e4ab83234dd8266da8665862e4dc', '\x01988858c8c5216ec58e102df1a17bb86fcf33d5dcfb64fc33107da0e563c0cc6247a60710a9e6e9bcad9473c7', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-19T08:11:17.016833596Z', 'cli', 'tenant_create', '\x4ab775ee38e790bf4463893ba28905ea7ea9ad618bd67ee234363c4d226fa95d', '\x01d70eb8b5220349b5e6ff704ea0d92391c406ff8834ebc3e34727590418222116e1134680a92884e251ece4adec51b50afa38fbdebc85faf6585bbd38', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-19T08:11:18.721335499Z', 'cli', 'key_create', '\x1ebdda4f800e4dd475c3d72662a80ac9d4a28d456a1f19c2b3f0809748a89a32', '\x018914afce82508292c6e183da4b00d28a553f7bb53e9766c81d95cab9777da1a71a8442581dc19ffca0373249', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-19T08:11:20.366697325Z', 'cli', 'key_create', '\xb48789fc0a876a5be733cce8fff2acabe33079f35e52c147fecc322e2afc2efd', '\x01e115b4b6755ee2e9f916ab789951481cf9e34e6ce79c0094629d9bd7c9e19f77ea1bb91321435136d207bc3a', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-19T08:11:21.920776133Z', 'cli', 'key_create', '\x8254821e17ac2d5f41b5e2dedca494cc89c3eba31f818791be60ce003f052369', '\x0179822ade6ff2d75f630beb449e685459e7bb044cdfa7052d206c9e2436deec4eae6eab48476bcc924b8ee05b', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-19T08:11:24.32673366Z', 'cli', 'user_add', '\x2ea09fba5a0fc1b5d4317176bc41e70e5eff3d892cbd1b911515862f2ea5df0c', '\x0148b4f243d8614f244a7b7c284b51e28090a68ba247b97646433fce3d5c6bf650d22078dc6df90b1b83647b675cd13b2af312c16a0487acf115c847c3', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-19T08:11:25.932948538Z', 'cli', 'user_add', '\xc6618b20f435077601114c4559dc215b418aa9cb57afb74e83a35db5996a5e0f', '\x0191bd6cd0607d3b55bd6a3246891bb726380a2530b675182df56dfbb177134e0151f0dc2a4f8a42ba4e572b692645dcb3e64de7ccb92eeb2d97f45c4f', NULL, 201, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-19T08:11:27.506718062Z', 'cli', 'membership_grant', '\x6efc0785e0e71aadb41e835937313db0684ae539845b1488e2c1d70cabaeed5a', '\x016a8e44a2fedfb7f9b9a0ecfc63baacc9741f9c623b166374886a151e1101f58ed419532dc9fb1ead31b45b2d49df051e76bc60763e0b667c3346a47535d3cbc35765bc04f46c6f2c47f60ed4ad5c67e28fa59c3f827c87c9328613780367ad78524db025', NULL, 200, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-19T08:11:29.095155351Z', 'cli', 'membership_grant', '\xa903812781a17c7a58c565cac9e8898add984de5fc6037374a78262fb467e053', '\x01589846dc77928c76fe460d3d2afb91152ff775c7a5c49ef7441014a95984ac4338367fd1d1711a4c3b9e6e609f900a951a0a8716038394481e5aefcb2fab1488d997fd192de96f1e4267dfdf2bf38316a694a7c288ef4f9c2a44818fe126415de3593adc0b', NULL, 200, 1, false, '\x3315676f1e092f0f');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-19T08:11:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\x3315676f1e092f0f');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x3315676f1e092f0f', 1, '\x01ce61ab064248dd02e25cccbb6242dc4267a8e30b231fb8234a5ae1d2e338c174bd0cb3afb49f22bee8043333f6648e1711cf91a83a8aaa2cd34e5200');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x3b1f9f19901c2fcc', '\x2310938706cc9455ba496088cfe81c99', '\x2c2b1295b4293678f69aebdd45f3fecde106d14f27d14340d6e1b358b87e0282', '\x011bc33812cba7ec656e8d5a55a0269e8395431e17ae4131ca694a4ebf85ad904c', '\x01e73b53c17395aeedfbf15334355a68f89b234afe255fcdf5c6db279667b2e81a9c6a079e4dac5f6487e45e', 1795075875416222440, NULL, 1802851875000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x6f4408924251b3a8', '\x27395d36d38a1cde2fd7540275f4be0a', '\xd644557fe88e1fc830f8df6e40125e6116068b1e237fe0b02553a4ace9204ea9', '\x013315934c901f36dc962e9c6db42548acdc2420c6248db0a17d6c44ffd4fa3b79', '\x01b61ede1b21533ba9b0c6b051105476a1ccb833cb8fa5223c72e2375683b431397fbb9453ca1abe6ea6bb43', 1795075880366287742, NULL, 1802851880000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xe2cb832abc130c85', NULL, '\x071a65b49f40438a2c3644c43fda067c5340f3043d7377a2dac81f69860840d2', '\x011bf844b1c02860d13ebd7a86a0fb811893f202bc1aab9a16680946dccf261ff2ad', '\x01a07b83e43ea0e7f477dff8f8cb69b9a687e3b4768ae94022f19a116d4f148f', 1795075881920220347, NULL, 1802851881000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x01442f0717a43211', '\x2310938706cc9455ba496088cfe81c99', '\x8abdf335588b58d13e3e2c723306b11649a57f1b25a683e7dc400bb1830e1215', '\x0157eb0d3d1edbba6f559ddae7e2185ecb36c326b16e20e0d9de05f28990becc186c5993e54861081353735293d40f9b6a3e6382d59e8d7ae6a6', '\x01b44405241ccbcc394ca4f01d24daecfa9c9fd43d9399a64193fc81d85680d5d1116ecf62302dbdc98212fec849', 1795075873716611075, NULL, 1802851873000000000, 1795075860000000000);
INSERT INTO vcon.api_key VALUES ('\x98aacc6d6d57f755', '\x27395d36d38a1cde2fd7540275f4be0a', '\x72e3f46bfb12cbe0e90a50efc66117abc1470be8b9cefb58903eb2d0d6733283', '\x013c268f2cc28c288353480d08f468de9c16537fe73a0c0e0043ad504be4b16b35d7164c8c4a8d14615c38002869193f1c0430c71e85591a3b42', '\x0137c74c44303e9931de70406277f09bbe5a5bff88505f0a3e518dffcbf2da6eb74736b095263eb28227db03b444', 1795075878720919259, NULL, 1802851878000000000, 1795075860000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 2, '2026-11-19T08:11:13.720481792Z', 'cli', 'key_create', '\x6f0a7937cc0fc0b8019a07b3ce411e026974efd341d4c77c6fbdca4a4bc5507f', '\x01ce432fce932d6fa5a335ae409132d975ea9d8ffc11300605cfc34baeacbe6fc291afbf0d6d5b7f4dfa44ce20', NULL, 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 4, '2026-11-19T08:11:15.419750563Z', 'cli', 'key_create', '\xe42fc386777d0f05b481c9eeca0a6caac75f64158b18d38ff69a60d9d51455cc', '\x012e54df74045c730ccfe9eafc56e64f7487d73008564477a1e954a435243667d9ab9ac1459c363a864b8e287f', NULL, 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 6, '2026-11-19T08:11:18.724464559Z', 'cli', 'key_create', '\xa88ce1e58d400c44f90d133d1daf8c69ede37c93bb7cced864c6cbe61485f719', '\x0109c31d3f981d2d0e91aca7930ec76cdd097fe14231da5231f37ff3ddc46523840a463607e4de0645e512f6e7', NULL, 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 8, '2026-11-19T08:11:20.369708783Z', 'cli', 'key_create', '\x6619591ba8d03546f33cb08d259fad7dda3c712c16044fec60d3838a956c8c29', '\x01e0629a9f8f74976ebbc3fe63a27a4dace0fba0f0e9a00afd45c5f0cfb6158b70f0861012268a2d87c4e81a0b', NULL, 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 10, '2026-11-19T08:11:27.510066075Z', 'cli', 'membership_grant', '\xa2bef7570bee2fcf3448ecdd9d43ddd5e690b8a143e4e618c493a868ab0a392e', '\x018360428792c89751a40f96f23f5fe7dd0e3452a44ccd30bf8d46c7b362dfe8172cd31c52a59612bf96f0735493e66a5a907d9c0d8b1cf6d2d1e57fd11ee2166aa3f6bdc91d09ec60544e16a7c00e29b3d1fc3edfcafffa33f78790b63e2ae1955f1e7397', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 12, '2026-11-19T08:11:29.098250966Z', 'cli', 'membership_grant', '\xfc5cc64661edcb4ab728cf226778044d5bec4132174caea11315f749ddefafd2', '\x015d7a394dba60fc3f3e1b48848f16c254ff5b1f3261388bd9c2d8df57a22843299b274b5342a024b1bd6db1c6d80eaead4a5c389b703c3b38464dcca369b573f9f1ae632a3cd056757774b42ec818a4d2e10d5718ca1eea83fd9f44eae535c939875107e05c', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 14, '2026-11-19T08:11:31.214976052Z', '01442f0717a43211', 'create', '\x851bebc824bbfdf938bd94c09647a8be20709f11a9ec2fa34108e2a8888ef868', '\x013d8f8a91385e5e687aa5097ad4932106826aa4322a23e86d65bc0955c09a5475a601db7026427d0cb2a2fc0d328c7f4766f575a5e5237f3f37a4fcdf4451eef6', 'adf26912b71c0fbb734f4108f72a6fdb', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 16, '2026-11-19T08:11:31.224598825Z', '01442f0717a43211', 'create', '\x67146df5589dd73c40bae0c11e1ec2c1b5479401274e85235dcae79b3eb17b55', '\x013ab38e16fb373b614f6c3fd9a5c568823eb7de0ccefc68261ac823979c3e38cff3fab7a711c37bc9f22fcd30147c657322085a854dcddfcb56a1a93f779c940b', '800566393a6eb84d733fdb20e7c4f109', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 18, '2026-11-19T08:11:31.233253063Z', '01442f0717a43211', 'create', '\x0f266eaffbfe61b689bde53795973e5016d14da472b6bc5c9c211b23a23b75b6', '\x01150bb538987dd0fb0d578edadf7f1a70cf72d463e635e817d6477534ee83fe648b9d167c105f605c60480d96cbb8df947a1f6db557d4c0fa9a64a5403d1bc3c2', '204c1c781833ec943f9571e4f7fbbdc9', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 20, '2026-11-19T08:11:31.242836003Z', '01442f0717a43211', 'create', '\x680ced00a3238db300a7796bb34ddac9b287fbd1ef65948d7074352fb2fe5425', '\x016a7df16fe99a40568971125bdf0ea6041847e5aa6b3a8c2ddfd63b209bf39cb54a71bb2e89e237d2dc9a4af20eacb71280066dacbde88f10ac3d24b7debf0aaa', 'fac1050016f218f2cd19cc993323c51e', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 22, '2026-11-19T08:11:31.25178186Z', '01442f0717a43211', 'create', '\x805818342f0df8e8a312fe08d7cddb189bf371a2909378ee0f27b6111fb35558', '\x013adcd4204b00a8b2b0a91973698c3167d3c0ac785276c8e8edf3adc65abd24aa1ba6c09fbc3119d3fd984007efd8aca5dd1ba356c2b971a3e70ce29fd136719f', '792928dff89c9bc05ae9a4d54e8c5789', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 24, '2026-11-19T08:11:31.261179392Z', '01442f0717a43211', 'create', '\xd51de3406ecbf57b7d9c4c94affd436caba88548fb3635538d69de7b9d76afd5', '\x01e09043c21d75bcc3f85db6d5523567e6d8e0071ed641394b045a8828ab8a4c3d75f41bed27b4cc8c5fafd922f948abb30efe60e030ee4a9ccb22a03a5e096c3c', '40ad6a6283a11efcafb8011cf4464a66', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 26, '2026-11-19T08:11:31.270495776Z', '01442f0717a43211', 'create', '\x7c1a6dfd551d7fe7f131cec0aaa3d9d02d6de530376a467ca812ae2e967dd034', '\x014e540602e568b532445408d37b9c268876909c8e04d0786ffcbf5a2a1f329a9d18ff21578b6da91e07f042280830f74afe9f44ee44d7f9fa088f2b8a2c8d4d36', '2f8d0af0a6bdc83d5adcc26721d7a6fb', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 28, '2026-11-19T08:11:31.278840986Z', '01442f0717a43211', 'create', '\xe01e9b220ef9ee0ba9e0061d5427a56151fb94864ad102c8e563bb0f147e0731', '\x01ba33e6f0b5d7a379d0d56771b62688f93ef0373cd85289c1794313d606e7bf9c33c03a2589b084ff020604f672bb3148671f27067a7927224ef6e7087b3d99cf', 'c425bc5573e8313221ed5667f61d858d', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 30, '2026-11-19T08:11:31.287308946Z', '01442f0717a43211', 'create', '\xe54616fc4167fe82955ec6a6414a2770c1a74222046399c7afa6c5f6615e7fcf', '\x01c66c482a1ac6584c657d5ddb58d45c4e08d5976a09336149e5018f9313e14589c6261592f19baf24bba1d5f3331de025761914f8d05511800d23965c980b8160', '20ac82c95461cbeb032d52bed1537955', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 32, '2026-11-19T08:11:31.295678073Z', '01442f0717a43211', 'create', '\x5544c18f437713a982b4fd0878038908af91877053775889fb60828eb4268a9d', '\x01657d83044a7b2cee6d3ac77f80e8219bb6faad55bb639f5523a2343684d652235566cabe2d0b4d40d3bada6cd5a26101a604d208bc8912e5b2a5144724f71ebf', 'f4eba540e69b860c9d6b6ea0eaf54af7', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 34, '2026-11-19T08:11:31.305577809Z', '01442f0717a43211', 'replace', '\x851bebc824bbfdf938bd94c09647a8be20709f11a9ec2fa34108e2a8888ef868', '\x01dc744f598f5c2382b9ce770a09368d815b8ab4d7b1d89707b157f7b9b704038ca8df2538dba40208b7b6fab7cb3cc235609836cd09185684fdc19341aba4e283', '1ac67498b0920ad3bd709df46f7ead7c', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 36, '2026-11-19T08:11:31.315001823Z', '01442f0717a43211', 'replace', '\x67146df5589dd73c40bae0c11e1ec2c1b5479401274e85235dcae79b3eb17b55', '\x01ff4741ba18c9dc1c50dac4713a2238ebcc913b58ae7afdba81bc5329fdc4e26081b1227c8db939cfcd42800e68171d28e62a2423e5f9db6b9b97f6313f7cf955', '989e03e5be7a68d1dd27940ec1ee050f', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 38, '2026-11-19T08:11:31.318881224Z', '01442f0717a43211', 'delete', '\x0f266eaffbfe61b689bde53795973e5016d14da472b6bc5c9c211b23a23b75b6', '\x01116c2d93e8726c6efde25c0b3f88af5619aea6d4d347b0e7e90e50712d9994dc619ebd15589367f39b504c53ed4ca402ac28e20ebcb8fd3e0d25c58c8c2378c4', NULL, 204, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 40, '2026-11-19T08:11:31.325005567Z', '01442f0717a43211', 'delete', '\x680ced00a3238db300a7796bb34ddac9b287fbd1ef65948d7074352fb2fe5425', '\x01d7dac31e61492e63fdba94530183d7d9ee9d36f9b6d51c870ac8af3a63f188410f4c6878e871c98e7ad5616a9b60e5d7be3213155e8f3d51bdcfaf916657db46', NULL, 204, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 42, '2026-11-19T08:11:31.329881996Z', '01442f0717a43211', 'hold', '\x805818342f0df8e8a312fe08d7cddb189bf371a2909378ee0f27b6111fb35558', '\x01f33328e062e7efaf6af969e1befd1998eca01bf54228ffb4679991f770e242a1dfea4c358b517a828ff3fce5ffc9094f6b88d5ffb5f6c8569ac68ff2835af36a', NULL, 204, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 44, '2026-11-19T08:11:31.33341863Z', '01442f0717a43211', 'delete', '\x805818342f0df8e8a312fe08d7cddb189bf371a2909378ee0f27b6111fb35558', '\x0106ab5d746aab96fcc44b00dcaa98c59a04ab7191280ddd0093c6088503986cfe446765f9f440fe7101f300fe7cfffe099fd66af448cb798f845bb605f1a02993', NULL, 204, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 46, '2026-11-19T08:11:31.338392697Z', '01442f0717a43211', 'hold', '\xd51de3406ecbf57b7d9c4c94affd436caba88548fb3635538d69de7b9d76afd5', '\x0162ed88cfaf85c416a19b2ace4ad8313fff146e785abb1a342693d5fc40f1358af95822f6471ff65c70ce3fddd6bd51f9adad2d6e2970c8c1d60b604b9440fb89', NULL, 204, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 48, '2026-11-19T08:11:31.347280397Z', '98aacc6d6d57f755', 'create', '\xa0fa82e39a01c5062731b5bce3320a7fb7673ab93758cdc02664db48c1536c21', '\x01cfddd4faa7d461a1ac50a70cbf4d7331483ce1af7617407ed71223552a1ea577323b00f9683253f01747767c65ea9dd11854f45415a71324dd0da877dfc06cd7', 'b7af636b47bb06d2b97a389d51f18f34', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 50, '2026-11-19T08:11:31.355417682Z', '98aacc6d6d57f755', 'create', '\xc23e181e57057f0aef92e1683472136913d839a4ce37f474610dd2867f2f98d1', '\x01800b5bb2e0f7d9298f070abf40900707db96f4368b7df24a54b2bda4b36b253abfcef509eea664f6bec6f0a44de23d50534df8341d956950889ac3765b75ee35', '1c22191506798348f9ce429cc515b595', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 52, '2026-11-19T08:11:31.363526577Z', '98aacc6d6d57f755', 'create', '\xc8638b56830dea3287c6ee156d3c74dccc72f359da31fb4777da7714e1ddd1a4', '\x018d5a1797f3a35e776d3c0f54902662756c3632fa5be2cc47f71c103fec77d37a95a26f7845477f14f8cd088539f1138031446144ed99ce0d4dff74344846d22b', 'ee75e810c1550dfb929efb3a5525e9ee', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 54, '2026-11-19T08:11:31.372556083Z', '98aacc6d6d57f755', 'create', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x0114ebc1abcfae5578b7065e85f8f5bee9c5c58265f8059eb822ad6a899785f2ffe9944db4923920ddc9d41b7bd5e076b5af6fa702dbd3da1793e8a99d5cb4600b', '35391eba1fdada77a8bb3d8ad3fab647', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 56, '2026-11-19T08:11:31.380585525Z', '98aacc6d6d57f755', 'create', '\x61be65b534e926fb09644085a0a44056db7e1b8d415c8e8cd15a119871b5195a', '\x015db11726a7e6e07bb6878b3e1f38e9ac21ad0e6e8f74267d6ab1727886b97cf58a5eb21ce0395c9f98ec8540aa547e02141bdee137c4dc61e5ffc634ca9787cc', '78311d6bf08f02e3e8337be8ca5bdf05', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 58, '2026-11-19T08:11:31.389791937Z', '98aacc6d6d57f755', 'create', '\xa49159743e5a964119f608118864a8e5b1184a77f5aae58e2296501307bfda35', '\x017c33e108d3b1ff82999514446d3d253ac8601fecc4542e9d530bc473358665f484a976249d5755c061c4f46fc3a1b4280e95ee2ce3993a17f75d6508867606e1', '00607846eb2bfbcc5059fc93baaff08e', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 60, '2026-11-19T08:11:31.397622426Z', '98aacc6d6d57f755', 'create', '\xc7b39d24b1133141d3839d9e3598e00a0ab93b1f50cf22059007110feab13745', '\x01655710bbbcf0a9f4ceba00cf45acfec14ac7bb04d1017dbc63744f487217e46fb83a8ec2874826fbf9af9c7d0a28bd8785b7356147dab654ea060edbf96e82ff', '130a0b3777a8c4ca0fb664113ffb4e4f', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 62, '2026-11-19T08:11:31.405903766Z', '98aacc6d6d57f755', 'create', '\x3e4f521137ef361ec41a1a7bfc24563234183baaea3e4d925b6e2222d9fcb0cf', '\x0149c5c8e38ca0ab7511cad14b5916c222ce7252b165f2caf48a4dc1fbf89c6902fac56a2696a3d6c903b13724459724bd65977f00cbb415345b78dae8eb5408f2', '4d091ae1cec4be0b744e630c7eced2f5', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 64, '2026-11-19T08:11:31.413910764Z', '98aacc6d6d57f755', 'create', '\x12648009fb2c27635ba058800a4c3a2e7787eb7baef1001ef96545d1900baa39', '\x01144582704a0d95fc907814e89e3db3605549e603dfe5cf8598bbb34e3d1f7659f917e22a1aca6635dff8916de4d7f986c9eddd20db9b3f2d9a50d601a28f10c7', 'd8c45ecbf1f9fae70caf9ae59990a54d', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 66, '2026-11-19T08:11:31.421676447Z', '98aacc6d6d57f755', 'create', '\xfee188b349172b71302791dcdf92e08fc3d4ee57053c9d2407c6f80069a0e5b5', '\x019d5fb8339a24de8fa6099c67b65cb515c828e3806b8c4e4080d6c54c481160903157a6440f00da130131b29dab8af8f7c5e4d15b2e6b473332cbd21b811c1ce6', '92d80ff347bf79417ca0fd5c8f1f3fd6', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 68, '2026-11-19T08:11:31.430736611Z', '98aacc6d6d57f755', 'replace', '\xa0fa82e39a01c5062731b5bce3320a7fb7673ab93758cdc02664db48c1536c21', '\x01b65aad2f9c01458047a24e949ece95e8aca5d6d9fec25670b8a3ab46507b2c456cd443a2f49d4366c4a5ec9079bdae7b5e87e795b0d2508cb224e3c31e5fa19c', '8d3b193df3b77a0a63f5e0f6a14097b7', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 70, '2026-11-19T08:11:31.434572392Z', '98aacc6d6d57f755', 'delete', '\xc23e181e57057f0aef92e1683472136913d839a4ce37f474610dd2867f2f98d1', '\x018ea823b684fbe9615fa2f43e1f90b5678217a722287ed4260c8ad18f6eac672c2d56501f3c4e17ed90e1c09e286b189b9b49af0ae490efeb6b84bcebc1bd6c97', NULL, 204, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 72, '2026-11-19T08:11:31.436834851Z', '98aacc6d6d57f755', 'purge', '\xc23e181e57057f0aef92e1683472136913d839a4ce37f474610dd2867f2f98d1', '\x01152185946e6aef48fec434bee40e6e46640688e1a409fc5f924407139323a64d9d3ec37ac5db2f6fe2d2c332266bdf7845e2c4052be2cbf63a076c50021754f4', '1c22191506798348f9ce429cc515b595', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 74, '2026-11-19T08:11:31.454636882Z', '98aacc6d6d57f755', 'hold', '\xc8638b56830dea3287c6ee156d3c74dccc72f359da31fb4777da7714e1ddd1a4', '\x01c495abd85db67ef7de0007253e631d0df5f68064564d0dd52b2d74ab96b8d790639f86f61cd92350134f20c1cc7e6fcd50b39f832765cf2b7f0b2bde94c85385', NULL, 204, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 76, '2026-11-19T08:11:31.458540237Z', '98aacc6d6d57f755', 'delete', '\xc8638b56830dea3287c6ee156d3c74dccc72f359da31fb4777da7714e1ddd1a4', '\x01eb2b296f396266673fe40b52129fa104057c6ace7af31806c20b8e7555c6513bc6fff9e9722b65034fe4e97742701a0bd732655093787425b127f2fff207a1fc', NULL, 204, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 78, '2026-11-19T08:11:31.465068986Z', '98aacc6d6d57f755', 'hold', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x01b024dbd3253e5b4525ea76b1b578ad7db846b2ce12fe30ebf3d0b47f3014b3e5c0ef260331d119341b9629afc6db39d77ab74f1858866704fc26a280ad4ea461', NULL, 204, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 80, '2026-11-19T08:11:31.474515491Z', '98aacc6d6d57f755', 'replace', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x01f2fcc6cccd28fae7ce1e05400aab2fc6eb3cfe1dcb75f29af558414ef2dbc81f43e7b348a9d0052a7a2dedcf646b32d912b9391dd0551bff6b96ad5951edbbcd', 'd5fc1e59ac1be35cf8ce9de160381c7a', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 82, '2026-11-19T08:11:33.740626813Z', 'sweep', 'purge', '\xa0fa82e39a01c5062731b5bce3320a7fb7673ab93758cdc02664db48c1536c21', '\x01346037de21b1f9171f790a268f227cd3ecd78dde06e9ca38cf01c3a1f0af5bdfab218ca6b0c6a0fd00130d7ee385f796cc6b775a1b9962dcbe50e634a96cbc97', 'b7af636b47bb06d2b97a389d51f18f34', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 84, '2026-11-19T08:11:34.01769868Z', '01442f0717a43211', 'legal_versions', '\x851bebc824bbfdf938bd94c09647a8be20709f11a9ec2fa34108e2a8888ef868', '\x01ae117cf78c0829d53e2a14a0116ca6e1a329f94e3ac583269ac6e8e41cb5ea5d7d6a42a46e55e21bd94519fec38eda25f50d9dca879ecb28ccb8110c553aa0c5', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 86, '2026-11-19T08:11:34.034746183Z', '01442f0717a43211', 'legal_read', '\x851bebc824bbfdf938bd94c09647a8be20709f11a9ec2fa34108e2a8888ef868', '\x0180664ee4f859cb71196848ec70d64d05c6482e89a23f999c1b6349717986df0054abb6dc1d2d511ed9367c83fc0857abcab237a063dcf54f130fe2e3c63c2ade', 'adf26912b71c0fbb734f4108f72a6fdb', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 88, '2026-11-19T08:11:34.048311329Z', '01442f0717a43211', 'legal_read', '\x851bebc824bbfdf938bd94c09647a8be20709f11a9ec2fa34108e2a8888ef868', '\x01bb9bbeba2bf7e026cba45570489b09ffac34740bbb87509620a50a21719e49f69b947d823923b2a98c6bc3d96a7d857959d298a20e8d5e039500c89d1cbd7832', '1ac67498b0920ad3bd709df46f7ead7c', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 90, '2026-11-19T08:11:34.066792792Z', '01442f0717a43211', 'legal_versions', '\x67146df5589dd73c40bae0c11e1ec2c1b5479401274e85235dcae79b3eb17b55', '\x01ffd1fa3bac58320bedb82e5049f5d7f30d83d92f7405feb8058c9dde54f29272c62a4dd734a1a1ce5df1ed21b1b14a9a3bcedc30311ccbec7a68b65c8a58180f', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 92, '2026-11-19T08:11:34.077521637Z', '01442f0717a43211', 'legal_read', '\x67146df5589dd73c40bae0c11e1ec2c1b5479401274e85235dcae79b3eb17b55', '\x017bcf92f2a8cdb9cbb8f981bdb4de34931aa04576c897f1f34672a6d867c2cbd4736f01d50f1bd0e8046185ecc4f86eb1a0bc2f503a7ef4e6fb34d4e2a395f9fb', '800566393a6eb84d733fdb20e7c4f109', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 94, '2026-11-19T08:11:34.090939062Z', '01442f0717a43211', 'legal_read', '\x67146df5589dd73c40bae0c11e1ec2c1b5479401274e85235dcae79b3eb17b55', '\x01874ff41c33ae2c28128c0cca25b6a519dac300a5eb5e49998f291580c03d1c432781633d32e84d9569d99b0dc095d268369a6345b701b59117068c3f0d3c4a83', '989e03e5be7a68d1dd27940ec1ee050f', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 96, '2026-11-19T08:11:34.098564384Z', '01442f0717a43211', 'legal_versions', '\x0f266eaffbfe61b689bde53795973e5016d14da472b6bc5c9c211b23a23b75b6', '\x0174660c1a2126cdedbfd2eb09cfb2c4161519b39cb500415837415b02e1711e68fa95b92676cc028b03ee33390d432719226e90332fe126230f23f23d66a23fb6', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 98, '2026-11-19T08:11:34.109343897Z', '01442f0717a43211', 'legal_read', '\x0f266eaffbfe61b689bde53795973e5016d14da472b6bc5c9c211b23a23b75b6', '\x014b9478f797512d42441e3da74e3cca9432edf9a4425dbd8932be4dee7d5127ef3f48031a2f760a5edd45528b856dee108e24622302af09a9fe64a240746e1030', '204c1c781833ec943f9571e4f7fbbdc9', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 100, '2026-11-19T08:11:34.116984024Z', '01442f0717a43211', 'legal_versions', '\x680ced00a3238db300a7796bb34ddac9b287fbd1ef65948d7074352fb2fe5425', '\x011753a02a6bec8bd841f3e9bef6fc31d70703669b04ff492085c9963ad4f4b3cd96e7d8fad3f3e8a3b67370c0857b72e0b723fffebd01f603ce4b634a1ac42ce3', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 102, '2026-11-19T08:11:34.126558194Z', '01442f0717a43211', 'legal_read', '\x680ced00a3238db300a7796bb34ddac9b287fbd1ef65948d7074352fb2fe5425', '\x015510c2e2a62b442e240b9252c14ebac810011870d03359a3d73ead6d66ce77dd371b489044bff20b81c8aa8fae0ea138ec5e4cf8f1187697b95e64d9db1e5109', 'fac1050016f218f2cd19cc993323c51e', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 104, '2026-11-19T08:11:34.138189215Z', '01442f0717a43211', 'legal_versions', '\x805818342f0df8e8a312fe08d7cddb189bf371a2909378ee0f27b6111fb35558', '\x01e3c5e71f72bf37281c7c06fee00dcc702a534d5dd601a039a48eb5b343c609e37fbe44f8cccdb6bb764bd616767659385cd59a7d696e26e4e82d330ca48f271b', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 106, '2026-11-19T08:11:34.15073364Z', '01442f0717a43211', 'legal_read', '\x805818342f0df8e8a312fe08d7cddb189bf371a2909378ee0f27b6111fb35558', '\x012c1e26903762a64a828b9be42d64f1b3910f9de277b27bbcb417f29df7055e4bb736e2437327db1e68e295c1ea27a064700fd63ba2b9015a8b4b6820df925750', '792928dff89c9bc05ae9a4d54e8c5789', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 108, '2026-11-19T08:11:34.156237216Z', '01442f0717a43211', 'legal_versions', '\xd51de3406ecbf57b7d9c4c94affd436caba88548fb3635538d69de7b9d76afd5', '\x0120ee4d05fb15f6a7d45bbb8f5d8c43313201c72546ce11f5a234e98eba5e87a406eb8c9fe5922fe065c258be074bc76d281dd638a71f224bbc7699382964ff63', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 110, '2026-11-19T08:11:34.169869456Z', '01442f0717a43211', 'legal_read', '\xd51de3406ecbf57b7d9c4c94affd436caba88548fb3635538d69de7b9d76afd5', '\x013f58dfc5582d8ea2ba5f599024373ab0d72694e8715652c1182d622e99b94707c0938139a42ff490b975e020d9e60cd240ca1e803f49952517b7be036a0725c3', '40ad6a6283a11efcafb8011cf4464a66', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 112, '2026-11-19T08:11:34.18324775Z', '01442f0717a43211', 'legal_versions', '\x7c1a6dfd551d7fe7f131cec0aaa3d9d02d6de530376a467ca812ae2e967dd034', '\x01f6c931617d3eb0f3dd6b5c3bd3f04161525ccc84e4deeb447e528b9b9ded37e8266398978cc734926bd8e84ab5dfc6e69fb8fcf14d6fe355bb309de03a4735ce', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 114, '2026-11-19T08:11:34.205091505Z', '01442f0717a43211', 'legal_read', '\x7c1a6dfd551d7fe7f131cec0aaa3d9d02d6de530376a467ca812ae2e967dd034', '\x013d9b82661fb72383380a4ffaf6f945fdf6497db48e8c2d282dda35a397fbefa72d87ffd7523613e3a0442cf4718d8e72acc252ee5739736f7152a34c171541bb', '2f8d0af0a6bdc83d5adcc26721d7a6fb', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 116, '2026-11-19T08:11:34.214381445Z', '01442f0717a43211', 'legal_versions', '\xe01e9b220ef9ee0ba9e0061d5427a56151fb94864ad102c8e563bb0f147e0731', '\x01b1800c1232a59a682c695d64dd4ae73a7f096874605ed2b256998502fc52d980173c403b4443ad37f5d7133fcecff3de0349d3454dcf964e6b31411d47468358', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 118, '2026-11-19T08:11:34.225483466Z', '01442f0717a43211', 'legal_read', '\xe01e9b220ef9ee0ba9e0061d5427a56151fb94864ad102c8e563bb0f147e0731', '\x01b9802b1ee4b174b1dbcf1d238f684c664173ebc1f83aa0d72b81122e4c91b6d3bb9ea45f4c2774fb06bd8a61af7be1b19cc17b32a03d0848bd6235c5d970d2c1', 'c425bc5573e8313221ed5667f61d858d', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 120, '2026-11-19T08:11:34.231409633Z', '01442f0717a43211', 'legal_versions', '\xe54616fc4167fe82955ec6a6414a2770c1a74222046399c7afa6c5f6615e7fcf', '\x01bd7643a6857bc8cdc08b3fa4020479a51aeda425f1b37c882ba252809f547b858a52605b9463a5ff3bcc8ace2320c762da1ed6423509ac663b41f293c61cbb48', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 122, '2026-11-19T08:11:34.24221558Z', '01442f0717a43211', 'legal_read', '\xe54616fc4167fe82955ec6a6414a2770c1a74222046399c7afa6c5f6615e7fcf', '\x01d039492c7befa9d55e6dfec32ad55354cc7826df4a0a9ac14a59581e2afce8d5858f86924f509de1ddc85ec796c1188b34d54045121fd6955ed39e5eb8a0be30', '20ac82c95461cbeb032d52bed1537955', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 124, '2026-11-19T08:11:34.248099404Z', '01442f0717a43211', 'legal_versions', '\x5544c18f437713a982b4fd0878038908af91877053775889fb60828eb4268a9d', '\x015cab19f42002b98016e74044e926d678a34edf8ef3d486bd8bf256c32ace521951f2bd3a607d52390dd998c45cbd569b4bb0492d1dfba6d56aa4ca1d3e5a00b5', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 126, '2026-11-19T08:11:34.257310058Z', '01442f0717a43211', 'legal_read', '\x5544c18f437713a982b4fd0878038908af91877053775889fb60828eb4268a9d', '\x01e916908df07face3ace2501b0da2b20742deef4af2059c027f75e8c212e4d134d8322cd972be31c8b55e490bb3882d13874ea833a32234e95fd399ace44669ea', 'f4eba540e69b860c9d6b6ea0eaf54af7', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 128, '2026-11-19T08:11:34.263634891Z', '98aacc6d6d57f755', 'legal_versions', '\xa0fa82e39a01c5062731b5bce3320a7fb7673ab93758cdc02664db48c1536c21', '\x01f9c5fc7ec5a3e324a61f6a2e4cecbc36225d689b09d91dd3edcfaa8f80e0ff7a3fa20d6ea7f90f25d2599a6cdd69353dfcba6f04e1752eb917859cce581a93ad', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 130, '2026-11-19T08:11:34.285869701Z', '98aacc6d6d57f755', 'legal_read', '\xa0fa82e39a01c5062731b5bce3320a7fb7673ab93758cdc02664db48c1536c21', '\x0190f909218c7bec7a540d8124aeebdf5153eaaa66d2bc9ebcb4c6f556c7aa6df905b1547460685347ccbfb1c9f8c9f54b202cae92c48e0e4249adfb7ca1b3d500', '8d3b193df3b77a0a63f5e0f6a14097b7', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 132, '2026-11-19T08:11:34.29560577Z', '98aacc6d6d57f755', 'legal_versions', '\xc23e181e57057f0aef92e1683472136913d839a4ce37f474610dd2867f2f98d1', '\x015fa9b6ffd37e97afa8681a7ef7ffae2cacfd54899c341d76a0b89002425918b22f5725a3cb61c9a25f8eb4d8bbaf6332aa534bd1bcfa29a9042c1c952a3be525', NULL, 410, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 134, '2026-11-19T08:11:34.307191809Z', '98aacc6d6d57f755', 'legal_versions', '\xc8638b56830dea3287c6ee156d3c74dccc72f359da31fb4777da7714e1ddd1a4', '\x0162d4f3c3d1a1e179a0d019d80c4f0ef5a62776d112138fe33ce6876fed4a2172d981473f8a0a483008652fb390d7e44ac7d34d86b16b0dacc6c32dc9e6a7f1d9', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 136, '2026-11-19T08:11:34.318496719Z', '98aacc6d6d57f755', 'legal_read', '\xc8638b56830dea3287c6ee156d3c74dccc72f359da31fb4777da7714e1ddd1a4', '\x01578d54e66dc60368e82154347ffa5cd6cf25beb5f1fbe6eba04f2115d046896a93ebf130dc94f768f03e38144e92bd4ff3384bc3ad160820a09ccd3551485981', 'ee75e810c1550dfb929efb3a5525e9ee', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 138, '2026-11-19T08:11:34.326641606Z', '98aacc6d6d57f755', 'legal_versions', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x014d0d41f736c411b01d9f97bce1f5e691f4c8614ef2afa76eade71db15a5bb4007c4969b98fab5493b391352c0ce04fb997b1dbcccd8d4ee3a4fca4df7c0ba395', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 140, '2026-11-19T08:11:34.339437152Z', '98aacc6d6d57f755', 'legal_read', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x01cac3e7ea3c7ae5035972e408739e4c8e9714032730c8e8c17fd7f9ce60e1ab64b3aba6188a3fba69740dfc1df52d1ebbf6eaad7f0e5b61c77e613878b7fd417a', '35391eba1fdada77a8bb3d8ad3fab647', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 142, '2026-11-19T08:11:34.353650715Z', '98aacc6d6d57f755', 'legal_read', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x013ae8255f83fba53aa97b6316691406d302292e50c0f9158f3b4adf95ffbaf4caafd57801c83a76e68b4df4163e36ba9e4800775f45fd78fe49c688c19d55658d', 'd5fc1e59ac1be35cf8ce9de160381c7a', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 144, '2026-11-19T08:11:34.365123792Z', '98aacc6d6d57f755', 'legal_versions', '\x61be65b534e926fb09644085a0a44056db7e1b8d415c8e8cd15a119871b5195a', '\x014e7cecc4ba8829018ef1bacc17db4cd6ede123f120f5ffb671d1bd98aa9c122fcc8ea61abf2d9a00063c72dfea3512c56f32fec85f3acdc140c6b0d3f3172a71', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 146, '2026-11-19T08:11:34.376774616Z', '98aacc6d6d57f755', 'legal_read', '\x61be65b534e926fb09644085a0a44056db7e1b8d415c8e8cd15a119871b5195a', '\x01e4c4e59750a5241f32875c5320e0e3f64726530ff08aac1169f2aca32233851789f127fc06f683ce5559f6e9bc91ceee0c8c04b6c1dceeb0e105a9a9ead8ae5c', '78311d6bf08f02e3e8337be8ca5bdf05', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 148, '2026-11-19T08:11:34.38472555Z', '98aacc6d6d57f755', 'legal_versions', '\xa49159743e5a964119f608118864a8e5b1184a77f5aae58e2296501307bfda35', '\x0143be77eeb02620b544be39ac02b59be67f12d3a02803a4dc10933eb65ad12a451e4dabc321c573b20025e51e8e982ec261efd1014ca72ca80d0e7d630e441df0', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 150, '2026-11-19T08:11:34.394324222Z', '98aacc6d6d57f755', 'legal_read', '\xa49159743e5a964119f608118864a8e5b1184a77f5aae58e2296501307bfda35', '\x011a5b9695af63c2857a46567b8e594f3d24662faf7d92e8493dcc4d26724123686e8bf075578a6f1989ca90747728795b951839852361d25618cb2920283c056d', '00607846eb2bfbcc5059fc93baaff08e', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 152, '2026-11-19T08:11:34.408986811Z', '98aacc6d6d57f755', 'legal_versions', '\xc7b39d24b1133141d3839d9e3598e00a0ab93b1f50cf22059007110feab13745', '\x015ea9c63b1d85aba4824bba5212f79928c8d4ba892855e925d6663c67a5a30a8fd855fa58770a49d1d74f0e1dd4d919213d4963704e62fe138ac6ede0bcfc9dc9', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 154, '2026-11-19T08:11:34.424068048Z', '98aacc6d6d57f755', 'legal_read', '\xc7b39d24b1133141d3839d9e3598e00a0ab93b1f50cf22059007110feab13745', '\x017fe1fd5fdba5c2e182768588d32f67edb8d86c163581cc28035400cbc2593d361cc4236839eead37b2e34b3d11f2d463bd1f2b088b51c4a9f16042d4b90762ca', '130a0b3777a8c4ca0fb664113ffb4e4f', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 156, '2026-11-19T08:11:34.433654211Z', '98aacc6d6d57f755', 'legal_versions', '\x3e4f521137ef361ec41a1a7bfc24563234183baaea3e4d925b6e2222d9fcb0cf', '\x01695984e1df2a64d742e0cbc98cd6cbc808451e8fadc26dc8977394c51a8bede4620a0eb67bc8208c9ab2ffa6f5354949a2231e6d36d90e297962b844f912b3c0', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 158, '2026-11-19T08:11:34.45818411Z', '98aacc6d6d57f755', 'legal_read', '\x3e4f521137ef361ec41a1a7bfc24563234183baaea3e4d925b6e2222d9fcb0cf', '\x0121fc6f8c13253f238c788d1b5d3bb60b74d9b58b0fca9a8fefe22c043d1302ccdf318737eabb7082fb66193a09de4a9a3b02794fe63c758c599942a2e861f0ce', '4d091ae1cec4be0b744e630c7eced2f5', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 160, '2026-11-19T08:11:34.468552428Z', '98aacc6d6d57f755', 'legal_versions', '\x12648009fb2c27635ba058800a4c3a2e7787eb7baef1001ef96545d1900baa39', '\x01264e6365f2ef9e0dee385b7105598059998623aae3bf4edec750ef08aab881f940a49c7fe987cf4ae9cebf3981f5b561f90abce6123284d4a3eb9278176ee289', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 162, '2026-11-19T08:11:34.481887685Z', '98aacc6d6d57f755', 'legal_read', '\x12648009fb2c27635ba058800a4c3a2e7787eb7baef1001ef96545d1900baa39', '\x01c836db433b42972736443890d411913b730ccd73770ca6b37558f3a135389d942be1d5b4778a5a0e091535a040ae7bc6ee5e4805b5c9ef1ead799337d4ac014d', 'd8c45ecbf1f9fae70caf9ae59990a54d', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 164, '2026-11-19T08:11:34.491120654Z', '98aacc6d6d57f755', 'legal_versions', '\xfee188b349172b71302791dcdf92e08fc3d4ee57053c9d2407c6f80069a0e5b5', '\x01adefee85d994bcbe828cc2a2ca5464118bb5eeaf903d988caa644238d75ba09b4fcf58cf71c1ce53fac76b0f0d9b1335754f0c5cb84b65a0188790f587566ff9', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 166, '2026-11-19T08:11:34.505332003Z', '98aacc6d6d57f755', 'legal_read', '\xfee188b349172b71302791dcdf92e08fc3d4ee57053c9d2407c6f80069a0e5b5', '\x013f308447a88c81b7aa34601b9a026b8cd45866fb21c064cb27b3efbc6cfe15af3bd280f39389cf3f3efac89e71326a1382bc72b7338aa04d6c5776f3a98bf033', '92d80ff347bf79417ca0fd5c8f1f3fd6', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 168, '2026-11-19T08:11:37.10319499Z', '01442f0717a43211', 'create', '\xc2b0b6c1be3b44e2c16d6bdd1e89dcd46f7a25aef38c8e99b9ca25a9bf8c139c', '\x010b6388fd674b66d03ff4312aad17ba95096b9d41fc439eebd2518c0799d8c2c604c9c7b6999f778d5d1dc5eae9ec0e5920bebd2ebd63abadaa6a59b56b9e9b20', 'c2d9eeb470303955029d9f406d433165', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 170, '2026-11-19T08:11:37.112688179Z', '01442f0717a43211', 'create', '\x2ddb700269d16bd3486bf773bda32815a00131f911b6c253e195771ca99705d1', '\x013a4baecbe67ec38c9ff0de439ca78ef842947858d81efc56911ad1526f1ee2c381817b8a1b6a69079edb17b29161dbd3267a915cf0237a8ea3ff88fc37ca654a', '32ed69c747896249375217ccf9347205', 201, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 172, '2026-11-19T08:11:37.121304278Z', '98aacc6d6d57f755', 'create', '\x13b94c4b4463818a0056826d29e122238aa2ceefa32942dd5591a50cc57ab857', '\x010c9376533d2b2a55009662ee1c13b1b0cf44c7ea42add3ed5b9238b24aeb0baaa21a40bd20d5952780581bb3e9fac9619e16f7d3498acca311f95745d49b7672', '74f17484552d36bb5d0fe580e60eddcc', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 174, '2026-11-19T08:11:37.130763681Z', '98aacc6d6d57f755', 'create', '\xa314f47b655d25d5e793d6586f22d331450ce8f1e4640bab9868d8075fa4ec55', '\x016ea3b604eca3d1ccc2212e8fe1b77e7f9263b4ca8008daf74ab9250349c6707886b4de0cacc046572fee614481e7562568ec3f147d87c05c2b661d5519d6349e', '8ef13d3b3e773de7ca424579529f0eac', 201, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 176, '2026-11-19T08:11:37.13463211Z', '01442f0717a43211', 'legal_versions', '\x851bebc824bbfdf938bd94c09647a8be20709f11a9ec2fa34108e2a8888ef868', '\x0181385c788dca7c3fa62ccb6fff023172d2f04c88b48b00005d58be9ac3c553cab0a2e2ca55fdf51af82e51d98187658175ab7e4f62d87f7dddc3c57be82ae6ad', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 178, '2026-11-19T08:11:37.140777212Z', '01442f0717a43211', 'legal_read', '\x851bebc824bbfdf938bd94c09647a8be20709f11a9ec2fa34108e2a8888ef868', '\x01832760dc3fabcb37c91bc6f4975443daef1bfabcbaaf1a3f3d0708c0111d9854460a515bf8648543cd2fdb9e9b498b258f71ca9e90985abe5eea1b64e4070dd1', 'adf26912b71c0fbb734f4108f72a6fdb', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 180, '2026-11-19T08:11:37.145531798Z', '01442f0717a43211', 'legal_read', '\x851bebc824bbfdf938bd94c09647a8be20709f11a9ec2fa34108e2a8888ef868', '\x01bb60d6f8a9c330a86d5b7e7470980437f8bde2755d981cd8bdbed3aa084ae15d3d4126612451ffcbfe80870d5ffd20127b66294a41a26a3dacda5e9bb6e637c7', '1ac67498b0920ad3bd709df46f7ead7c', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 182, '2026-11-19T08:11:37.149021367Z', '01442f0717a43211', 'legal_versions', '\x67146df5589dd73c40bae0c11e1ec2c1b5479401274e85235dcae79b3eb17b55', '\x014940764afa059f3f8cba1e47dad671d4ebdde9ddd049497199bd10cdc5498e872179b103e0875199b717b468aae4702758992ec49a6670deb4deaf62f3871c70', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 184, '2026-11-19T08:11:37.153922583Z', '01442f0717a43211', 'legal_read', '\x67146df5589dd73c40bae0c11e1ec2c1b5479401274e85235dcae79b3eb17b55', '\x01c515bde308c6516bd51bd4637a743bdf61f25f4805ce4106adafee787109c4221edc6afa683159244736113880ce015b5adbccb8bd3ed16001db22fb21f2367d', '800566393a6eb84d733fdb20e7c4f109', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 186, '2026-11-19T08:11:37.158620519Z', '01442f0717a43211', 'legal_read', '\x67146df5589dd73c40bae0c11e1ec2c1b5479401274e85235dcae79b3eb17b55', '\x0197cae911eef8b776d72f8aa5604da50528d234a36702f6f97dd0b02323094c4e0a00f7d68475bcee39638351a8cda3f8e95f172e5069801a51a2c825e39caf6e', '989e03e5be7a68d1dd27940ec1ee050f', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 188, '2026-11-19T08:11:37.162163162Z', '01442f0717a43211', 'legal_versions', '\x0f266eaffbfe61b689bde53795973e5016d14da472b6bc5c9c211b23a23b75b6', '\x01f35ee3989216e44686cca48835a28c36412b5c1a2c81a2f36f2d2b705ac811e65a280976ed392729d568f7883f1a474c516211930014290c069cb64e3ad86d54', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 190, '2026-11-19T08:11:37.166700295Z', '01442f0717a43211', 'legal_read', '\x0f266eaffbfe61b689bde53795973e5016d14da472b6bc5c9c211b23a23b75b6', '\x012e29d8dfe010f40b7a90b9be56911d92a2d6d077499d780df6432d7d756e55e0a17ea4263509304bdc99907dae412762fc1ec89f6d8124fc84bd713746451aa2', '204c1c781833ec943f9571e4f7fbbdc9', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 192, '2026-11-19T08:11:37.170182438Z', '01442f0717a43211', 'legal_versions', '\x680ced00a3238db300a7796bb34ddac9b287fbd1ef65948d7074352fb2fe5425', '\x01d098c47416cc02995d7b79a7251015679732fa18671dba4b25526f9009dde0fa365bf72d17a448092ce1250e89cc4e092cc7ffb0a74988d991eeb63883333abe', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 194, '2026-11-19T08:11:37.174676283Z', '01442f0717a43211', 'legal_read', '\x680ced00a3238db300a7796bb34ddac9b287fbd1ef65948d7074352fb2fe5425', '\x01bdef3696e4a18c4af4efdf33b3bdc1d8bc9e61f647865361fd7cf6af9a945e8cec51b6b5dfd475f27f112fa91915ce1bb68b9a10158709cf8f40a35bdfbc1b32', 'fac1050016f218f2cd19cc993323c51e', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 196, '2026-11-19T08:11:37.178132241Z', '01442f0717a43211', 'legal_versions', '\x805818342f0df8e8a312fe08d7cddb189bf371a2909378ee0f27b6111fb35558', '\x01e6fdee08634e8cfc5c72b1644ab708a9353568c8fb107ba7c5b2abdd60d208ac7b34d2088b08f4a59f56db31f1a45c4eddd53987a1b0af5f95ae8873d5783a6a', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 198, '2026-11-19T08:11:37.183953066Z', '01442f0717a43211', 'legal_read', '\x805818342f0df8e8a312fe08d7cddb189bf371a2909378ee0f27b6111fb35558', '\x01a3d5ba60b0b3415f8d24c64ee9738c4a66803a18422ec0607c4c79d75799ec011d6ba94a0eca5edaea4985631a6ee3413027819e54b47f5a888331333ae917b3', '792928dff89c9bc05ae9a4d54e8c5789', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 200, '2026-11-19T08:11:37.188070504Z', '01442f0717a43211', 'legal_versions', '\xd51de3406ecbf57b7d9c4c94affd436caba88548fb3635538d69de7b9d76afd5', '\x013c97185c58cacb485410c3b962f2bf289b4efc24242edfc4142be22e0e0858ce6c3ac8c900429066d95b4b817cf446a966a23b8d28c7520f13b1b969dbb225f4', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 202, '2026-11-19T08:11:37.193400098Z', '01442f0717a43211', 'legal_read', '\xd51de3406ecbf57b7d9c4c94affd436caba88548fb3635538d69de7b9d76afd5', '\x013634208bf4cad95cc381986464e730f47f3e886142921d0f6e24168b4a72efc1c596b8b7d911a8f73c8fede25c4be492febaa148542811a72c34a4c83723638d', '40ad6a6283a11efcafb8011cf4464a66', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 204, '2026-11-19T08:11:37.198011471Z', '01442f0717a43211', 'legal_versions', '\x7c1a6dfd551d7fe7f131cec0aaa3d9d02d6de530376a467ca812ae2e967dd034', '\x012ba0d603a2f899ae7b610d554ec22250577b6d5e9e12aade041850d799d7d6c90ab0329c5b798962f8b08b1fe7aaafafaf98290ed90cfc7d07614c11651844bb', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 206, '2026-11-19T08:11:37.202881908Z', '01442f0717a43211', 'legal_read', '\x7c1a6dfd551d7fe7f131cec0aaa3d9d02d6de530376a467ca812ae2e967dd034', '\x0118ca547e4228fd74ce16fcb264348efb46da0f931597b3cf6262dc64b683fd038c9e0efa0eb9c4502e9e767b276ac22418098a0d91c63e5f34297001405eae02', '2f8d0af0a6bdc83d5adcc26721d7a6fb', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 208, '2026-11-19T08:11:37.207140717Z', '01442f0717a43211', 'legal_versions', '\xe01e9b220ef9ee0ba9e0061d5427a56151fb94864ad102c8e563bb0f147e0731', '\x0147bdb732f358d86648f21c14cc3726eaf2f32b4617b5db61f653426e3bc05a6bddd1df6989e443d0869a75d7f4b39f75e53260ea0d76b71919b4add3daf5f3ad', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 210, '2026-11-19T08:11:37.212296608Z', '01442f0717a43211', 'legal_read', '\xe01e9b220ef9ee0ba9e0061d5427a56151fb94864ad102c8e563bb0f147e0731', '\x0166d3d74972591ed4c65c957c6c6529a736fb0646dc60ad9a3712dc08b4c221f6b9f33dfd756bb95d9ef4379ec165cd293f3742bd746103ac0bc70d14afcce747', 'c425bc5573e8313221ed5667f61d858d', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 212, '2026-11-19T08:11:37.216276268Z', '01442f0717a43211', 'legal_versions', '\xe54616fc4167fe82955ec6a6414a2770c1a74222046399c7afa6c5f6615e7fcf', '\x016a23885ca8e58027573dd7c0c041789dc9a5c2b91004d31b427a2ae29b49d165d47f5e34128e31310da02b5ae22f018bd98bfbaae540bd2022a55139888f8209', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 214, '2026-11-19T08:11:37.220936178Z', '01442f0717a43211', 'legal_read', '\xe54616fc4167fe82955ec6a6414a2770c1a74222046399c7afa6c5f6615e7fcf', '\x01f5ae950d36d5e02d92d1bb5f978a128aba8eee1cd2ea5d22f1569d85ba9ea85a57384ac45ab1c8c4849bcffcd77ce946c359a037b0e672cd69eec1375ce2f57b', '20ac82c95461cbeb032d52bed1537955', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 216, '2026-11-19T08:11:37.224707691Z', '01442f0717a43211', 'legal_versions', '\x5544c18f437713a982b4fd0878038908af91877053775889fb60828eb4268a9d', '\x01f710a53b204b95ada01bad3a4272143df191f0465a3a835f2ce062dc43579f8baeb9d6a1c2b670c47b7f7c20856b8fdeba555ab0aaae8b1422ac025014664ebc', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 218, '2026-11-19T08:11:37.229289796Z', '01442f0717a43211', 'legal_read', '\x5544c18f437713a982b4fd0878038908af91877053775889fb60828eb4268a9d', '\x01b7083ba65813dca254ab55d73f8334c2bfc41c47776d3df26bf26bc91993a85777ab0bde5f06a3bd15d6e94f67df06e1afff0eefd56361a0ae89e3942957cb75', 'f4eba540e69b860c9d6b6ea0eaf54af7', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 220, '2026-11-19T08:11:37.232935365Z', '01442f0717a43211', 'legal_versions', '\xc2b0b6c1be3b44e2c16d6bdd1e89dcd46f7a25aef38c8e99b9ca25a9bf8c139c', '\x01ab56247d6732749fa253f92b37b24eb01e6559d12a34df7947f35bb2cd93be12458c3342ac11e01bd58c4c53e57805c1b84dc9095c91924c797e70c05c6c9b0b', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 222, '2026-11-19T08:11:37.238983133Z', '01442f0717a43211', 'legal_read', '\xc2b0b6c1be3b44e2c16d6bdd1e89dcd46f7a25aef38c8e99b9ca25a9bf8c139c', '\x01c2d828f8c81c083287dea83b67f04a36fe5d25cd569c2bfa5265ca420809653000b7ae29296f8af8020bcdb56d13a979c06ac6eaf919711a1c63d689e3928251', 'c2d9eeb470303955029d9f406d433165', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 224, '2026-11-19T08:11:37.242766248Z', '01442f0717a43211', 'legal_versions', '\x2ddb700269d16bd3486bf773bda32815a00131f911b6c253e195771ca99705d1', '\x01a40de6caaba95d5ecb48c2f81af5770621c988326b819dab1cd4ce5b58ea91f1071b2f627a9888d5acffdaa4c23a693e364ce4cc339c322a103556ba6e3e59b4', NULL, 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 226, '2026-11-19T08:11:37.247612546Z', '01442f0717a43211', 'legal_read', '\x2ddb700269d16bd3486bf773bda32815a00131f911b6c253e195771ca99705d1', '\x01b8ae5d459a600904a0dc465c5808721a512182d2b492f77501e6f73992d84b2ab8a58a058ff8e734af3fe9814f12ba4fdda747c77efa6daf8d0eebb2a63a0d0c', '32ed69c747896249375217ccf9347205', 200, 1, false, '\x1d231f3593d20481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 228, '2026-11-19T08:11:37.251319541Z', '98aacc6d6d57f755', 'legal_versions', '\xa0fa82e39a01c5062731b5bce3320a7fb7673ab93758cdc02664db48c1536c21', '\x01855b2d80e82f320ea93d7020dd3c8d3cfd1c47df8e10c2ce6e423b5392ab4281e50e3e3ba7bc693f01a6a17d5e8696d1944bae5471a8e5bcf8a8e5ebf1bd48b7', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 230, '2026-11-19T08:11:37.256828718Z', '98aacc6d6d57f755', 'legal_read', '\xa0fa82e39a01c5062731b5bce3320a7fb7673ab93758cdc02664db48c1536c21', '\x015de126bf4295326c05477b6692597d6a19673315711903c669bc4351c16eedfa67d7af9d8367b7607cae84ec7d3c4d20a1c9c01c2a32e89c5be54ef6529d11b5', '8d3b193df3b77a0a63f5e0f6a14097b7', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 232, '2026-11-19T08:11:37.260018926Z', '98aacc6d6d57f755', 'legal_versions', '\xc23e181e57057f0aef92e1683472136913d839a4ce37f474610dd2867f2f98d1', '\x01775fb773cb4f7fbfabcafb6706181081e853582cfb631822e743fb253d96bc9d271d2ca6dd493cb83e13c0a3761a6dbf2c4e046cc3e1d582ea1081ef72b77d6b', NULL, 410, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 234, '2026-11-19T08:11:37.263610791Z', '98aacc6d6d57f755', 'legal_versions', '\xc8638b56830dea3287c6ee156d3c74dccc72f359da31fb4777da7714e1ddd1a4', '\x01c412dff3a19bbcaf923a385f10d2a8027bdb3a9e7370fd7272c6c51498707ba79963b126390c1aa8d83551aaa9ddaf02c7719d047d04c5f1fad362417d683416', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 236, '2026-11-19T08:11:37.26847946Z', '98aacc6d6d57f755', 'legal_read', '\xc8638b56830dea3287c6ee156d3c74dccc72f359da31fb4777da7714e1ddd1a4', '\x01970eba372f7a663d5ab710b57fa942942be76f35b9f53d65665bf85737784465f2918827f1f9c5df941566ff355265a2adc57d5dad7e08e9599a9a60cb41aac6', 'ee75e810c1550dfb929efb3a5525e9ee', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 238, '2026-11-19T08:11:37.272206214Z', '98aacc6d6d57f755', 'legal_versions', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x01680e3365d97a1078097cc75f339630e3b4f5d74e1cc33619005e9ce240c81aa7b18a514763fc6ed6a090bc6d66087f5e884d67d195cb7cee56c96eb8a2fc9ec9', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 240, '2026-11-19T08:11:37.276886291Z', '98aacc6d6d57f755', 'legal_read', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x01ff266a01a7a453e5a4cd5f1f81daad534b867ec601001501c41d172c20e9d0b1f710828c984dc9d9e1acb586142c163b6c9783e42bfd7c98d01dbd866dbbbc15', '35391eba1fdada77a8bb3d8ad3fab647', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 242, '2026-11-19T08:11:37.281333691Z', '98aacc6d6d57f755', 'legal_read', '\x7b93b448a1203917468a77159115a2fb2ae99c454d60242cd3eb69cdafe183fe', '\x01a67f0daf19da7daed3f92d661d5374655ccfc3b954f8d0737a3b0c6a80c3202bd2e8336751f1c0e0c4504dac6537992d7c34268dbd5d9c46f5800e3f1d78d33e', 'd5fc1e59ac1be35cf8ce9de160381c7a', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 244, '2026-11-19T08:11:37.28491364Z', '98aacc6d6d57f755', 'legal_versions', '\x61be65b534e926fb09644085a0a44056db7e1b8d415c8e8cd15a119871b5195a', '\x01b498b28dbbffe4a2c3454b97014b7de21374cb7d1e2187d70ad892af4f9bf405915863ecf927534cff28fe7542fe31dee07713c92117a6cd22c32537c1af8b13', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 246, '2026-11-19T08:11:37.289558771Z', '98aacc6d6d57f755', 'legal_read', '\x61be65b534e926fb09644085a0a44056db7e1b8d415c8e8cd15a119871b5195a', '\x019e6c461d12c91cf3e03e5e00298507ae5c96d4679ba8f2db81dc6321ba06025a3a42b978b5b975d320b55801086c3c494943596c66ebcac11bbc6ff72d0619f9', '78311d6bf08f02e3e8337be8ca5bdf05', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 248, '2026-11-19T08:11:37.293110729Z', '98aacc6d6d57f755', 'legal_versions', '\xa49159743e5a964119f608118864a8e5b1184a77f5aae58e2296501307bfda35', '\x019160cd3ad377515b86466d9f6c718d40e590f1e85fb41184ff2e5198529be5ee592d9a61dfefd736e7ac4138276c12cd6e5d7782a1eea5fc550c365b91879cb6', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 250, '2026-11-19T08:11:37.297476103Z', '98aacc6d6d57f755', 'legal_read', '\xa49159743e5a964119f608118864a8e5b1184a77f5aae58e2296501307bfda35', '\x01ecf03cdc3bad16a0191a6c15de9f7236f1342deb64eaa9653517fb2cc0947ccc628521770fea01120d9cea215917feac865c309111cb51bbf70c774c998dd213', '00607846eb2bfbcc5059fc93baaff08e', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 252, '2026-11-19T08:11:37.300996959Z', '98aacc6d6d57f755', 'legal_versions', '\xc7b39d24b1133141d3839d9e3598e00a0ab93b1f50cf22059007110feab13745', '\x011f0a1c4904ee7b2138c9c15af30fb799b7546af94c8b5071ca9a5731478a6578baa07535db8e37da2a91877bc09c4387e538ada763fa7f110a2fddfc8345e7fc', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 254, '2026-11-19T08:11:37.305565665Z', '98aacc6d6d57f755', 'legal_read', '\xc7b39d24b1133141d3839d9e3598e00a0ab93b1f50cf22059007110feab13745', '\x01e68e3de532e3b7868b3bf1777c4681d25540142fa131e5f9c746a38b30dc4559964834bc07e93f75199fd2f42ac75f55ef25c5d0d329ab98520aa1d6a5a31c3e', '130a0b3777a8c4ca0fb664113ffb4e4f', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 256, '2026-11-19T08:11:37.309104067Z', '98aacc6d6d57f755', 'legal_versions', '\x3e4f521137ef361ec41a1a7bfc24563234183baaea3e4d925b6e2222d9fcb0cf', '\x017ce729d0d7c940a5b673ad6b6ab670ecdb6a1811988898999f60c8f681d487030ae004f647bd4947984ed499a43e95f71de4880530b3c3e05dc8f3b5e37cff5a', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 258, '2026-11-19T08:11:37.31346295Z', '98aacc6d6d57f755', 'legal_read', '\x3e4f521137ef361ec41a1a7bfc24563234183baaea3e4d925b6e2222d9fcb0cf', '\x01a40518d68a5fa12dcb47f249591290d8f3c4531e564b9cc0eb2db31420aff6a6cb55edc227ca6a3bf8a7721753ab46d5dbf68f421fe195ebe892e2c5ad0b7c35', '4d091ae1cec4be0b744e630c7eced2f5', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 260, '2026-11-19T08:11:37.317150685Z', '98aacc6d6d57f755', 'legal_versions', '\x12648009fb2c27635ba058800a4c3a2e7787eb7baef1001ef96545d1900baa39', '\x019429069a334ea7fc45e5eedfe9f701d34486bcd685ecef071d476b84bd2d4b02aa9c45e426c2b1907028f8fab8c35c881fae8de555aa4ccab567bc466768297a', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 262, '2026-11-19T08:11:37.321535401Z', '98aacc6d6d57f755', 'legal_read', '\x12648009fb2c27635ba058800a4c3a2e7787eb7baef1001ef96545d1900baa39', '\x011902757fdaf0c80d64fa453471c305e4252e3135d07a2773e16b8b624b83a06dddac8c574bd9c8b99f983759edf3a30e5ebc8b22aa5eb86c2937c705e11c2054', 'd8c45ecbf1f9fae70caf9ae59990a54d', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 264, '2026-11-19T08:11:37.325061053Z', '98aacc6d6d57f755', 'legal_versions', '\xfee188b349172b71302791dcdf92e08fc3d4ee57053c9d2407c6f80069a0e5b5', '\x01cbc4886c7daa9f11256d33e0ef75d839583fddd2d9e960d80b8257b7d39d9f061110657022bf54dba3e8a82459bbb49d743c82b6ff5fa8c7581e7193b5a196d6', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 266, '2026-11-19T08:11:37.329436398Z', '98aacc6d6d57f755', 'legal_read', '\xfee188b349172b71302791dcdf92e08fc3d4ee57053c9d2407c6f80069a0e5b5', '\x01bab58785c27a7747ae3aeea8195db676ecafc85018aeb145481f2a07b7f659ba65db19da37aa9744b19826192d934c79204c443cb4a62d69479fb72254198924', '92d80ff347bf79417ca0fd5c8f1f3fd6', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 268, '2026-11-19T08:11:37.332980041Z', '98aacc6d6d57f755', 'legal_versions', '\x13b94c4b4463818a0056826d29e122238aa2ceefa32942dd5591a50cc57ab857', '\x01a85f963b2c41559ce970e7b69df44bb5214f5909eefee76aefe9f90ae53f707d21c95c2e39d999fb5134e3bcdf0ab76e34a6a81cdc9f81ef1d50baa9120b14b9', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 270, '2026-11-19T08:11:37.337716321Z', '98aacc6d6d57f755', 'legal_read', '\x13b94c4b4463818a0056826d29e122238aa2ceefa32942dd5591a50cc57ab857', '\x0118e3559d48b9ebe9b1e59ad012229964d1fd98082e57b4641f338a73d1a9936483039ca958adbe55285392115196cda4ebf7f0fd9b201e40a4e93c6306c2683d', '74f17484552d36bb5d0fe580e60eddcc', 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 272, '2026-11-19T08:11:37.341269427Z', '98aacc6d6d57f755', 'legal_versions', '\xa314f47b655d25d5e793d6586f22d331450ce8f1e4640bab9868d8075fa4ec55', '\x019be8a5fe0d34d14c374a53b581d0f4bc29f85664fde6da87087c8275659607b3149a8613fcb36dd05ec47447e7ef79de8cfce343ca592aabd6c9873f210d0093', NULL, 200, 1, false, '\xe32b37506480e3d5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 274, '2026-11-19T08:11:37.345922911Z', '98aacc6d6d57f755', 'legal_read', '\xa314f47b655d25d5e793d6586f22d331450ce8f1e4640bab9868d8075fa4ec55', '\x013575cd6ceb5d056b09f65d2eb419502f0fed4f5eead9bb594d9a39d59b8ae49f2f13c0e2d98d4dbb8b3039ea73f3e5d04cce0d9a71bdbb754e29665ee342a164', '8ef13d3b3e773de7ca424579529f0eac', 200, 1, false, '\xe32b37506480e3d5');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x1d231f3593d20481', 1, '\x01c6490cfa7a741279f995b0fbd97006c791038062e99f80a5f13beedefd2814bb89d3c9db8354512f0e0288a1bfff3a5a5afacf3e602f1a8f7458a81e');
INSERT INTO vcon.audit_key VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xe32b37506480e3d5', 1, '\x0159f6bdc2706df76a3512447b457539de38ed067f212ec0135f5849333d49e40fa1125896300de5d20b54f35e7bb461c4935f1fc85196a7e9c2aace2e');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x9105465dd519958d', 14);
INSERT INTO vcon.key_use VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xe3a3b4b78db60505', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', 2, 1795075896901310365);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', 4, 1795075897027305989);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\xd85515df541de7c1', 2, 1795075872100924165);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x0906b34cb18bf57b7f0e7e6fc5ad773e', '\x2310938706cc9455ba496088cfe81c99', '\x01f3db7e172347b959c52ac3ff02c2e5b2d1fb40a148a79e7a6d57b51a1c29dc0ba1a5', 1795075887505697424);
INSERT INTO vcon.membership VALUES ('\x646d46ddf8607ba4c42ff21145a1d303', '\x27395d36d38a1cde2fd7540275f4be0a', '\x010c857e7e766689905253e3ee149efbb916bf310786ae03566e985704f88786d083ebc3', 1795075889094374740);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x314cbf943f66860f839ee6cb26d760b3', '2026-11-19', 12, 12, 7404, 1795075896762509777, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', '2026-11-19', 10, 10, 6240, 1795075896918643200, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\xea7ba947ddf6d432');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-19 08:11:10.38771+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-19 08:11:10.422225+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-19 08:11:10.428764+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-19 08:11:10.429317+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-19 08:11:10.431412+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-19 08:11:10.434491+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-19 08:11:10.435053+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-19 08:11:10.439983+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-19 08:11:10.443841+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-19 08:11:10.444334+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-19 08:11:10.445393+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-19 08:11:10.448978+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-19 08:11:10.454255+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, 'fa49b3ea01db02724752bc4683990c82');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x7aaf506a893d48935211c6489a8b3bbbb3f309ab87a49cce905ad19ab0a30847', '\x017b99c924b5e96a1558892208d76ccc840115ce3e78003e0301635454fde3fccbf69d3d89', NULL, NULL, 1795075872109906956, '\x9105465dd519958d', '\x01b138652865e4f17e13b82b79698acd762fa7f2880d2ac9e01ce9f5015c7a3adfbced8f707dbc9b3ad408eb644389d6c58e0a1d604f7abf2c206b2d11', NULL, 1795075872109906956, NULL);
INSERT INTO vcon.tenant VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xfb810249ca651a602697577d6847b9f1c360e4aa5884c96dd5032ff860c4be78', '\x01c97c9fad3b3ebbacd51e04ac892358e15682c94394d73aea5fbdcb695a2b5f7feb99b0b3', '\x01fc1323bc93c01bdd8255e4c3a1726bbd8ae2d2736f8f43e6a8962cc65e6a', NULL, 1795075877016652940, '\xe3a3b4b78db60505', '\x01b50cff737c35d01b6652a7c7be945221a6a715784dd61680314886aafbc909a3c6e6c92de94b8939b6ae044e4105cae3c2586f9c65a066439977d0cd', NULL, 1795075877016652940, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xfd8c56c57f0890aa');
INSERT INTO vcon.tenant_catalog VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xdffb140242baa685');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\x2310938706cc9455ba496088cfe81c99');
INSERT INTO vcon.tenant_lock VALUES ('\x27395d36d38a1cde2fd7540275f4be0a');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x8b9b0f60f8a2f60f75256a033c6994ef679ed5ea13eb2589fd1ea04d84418f48', '\x01773466dbb94b4385f3cc80c52f80a70076e4cd657ca7e542781f751507de620b88dbeaf1c81fb2e29d3d106b', 1795075891435665732);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x0906b34cb18bf57b7f0e7e6fc5ad773e', '\x0159a524f429962a8eb693236ddfaf8647b86a742838438d082c662b772c5bf96fd86f4a063f73f08e47a4', NULL, NULL, 1795075884326076050, NULL);
INSERT INTO vcon.user_account VALUES ('\x646d46ddf8607ba4c42ff21145a1d303', '\x01a0d4404f7b1e0f3d26a4eead8c98d3a338b2e6180300ff4b7e7b8b5d39b5781513795126fd6fd16955855b', '\xc3c0661e19cad9ace32d7511a3f5125127654692dca6115716820ce02870a8a4', '\x013cbf428b539724ff21c8518a0782a05cca1bc89af3083ad09ae7576adf6e8e71a341cea6934c29bcfe9856dfdb4c0905bff62148d05ae10a6bb7711096ce85', 1795075885932644206, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x536add1560cd231f3dbe90112d8942cb61a8582ac38a7f3fb1d3ed83b6c5e4af', '\x01c7727266e5d6b2f150f36022b87174ba0e34d76843499c07f4bb40b988c67aeeb53fbbb71debaf88d93289cd', '\x2f8d0af0a6bdc83d5adcc26721d7a6fb', NULL, 0, 1795075891270155407);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x584d5e0fb495737c7e7ec5383662115da70ce847388163fbb3f25c2988a85638', '\x01dd00b253f8d9c77a88d6c63ac8d710499a10917e17ab9cf76bf34ddf77df01659a5163e432d4fd2c91a5a394', '\xc425bc5573e8313221ed5667f61d858d', NULL, 0, 1795075891278504469);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x1e2c225df0db6db13002e9bb3af626eb336e4199bd263066c7423735e05fd7d2', '\x0179444b9c1fc93f886df16ed4c32077b0c23e17456daa9321616a68cb64a621de3e6579fa5102e52e4c517962', '\x20ac82c95461cbeb032d52bed1537955', NULL, 0, 1795075891286975724);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xd537d99dd0f88a0f2e5cc0dba7b618e1ba312193e5f983d3f1cd84836037aa58', '\x0170c4d2fd0140f2af153e7966734219b97c89beaa0672cea4c3cc22b577adb361395f75d17a4038e5c18e1b92', '\xf4eba540e69b860c9d6b6ea0eaf54af7', NULL, 0, 1795075891295341666);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x1a8730ecc8ff3c9eb3234b65e5042bcb48ee07abd543a3dfa4861adba89fb8da', '\x0177e5c863ba566ca22d597317b24df5acf56b3ac7e25775c48a964246c8fc6a1d02a250f018c279ac8e4d536e', '\x1ac67498b0920ad3bd709df46f7ead7c', NULL, 0, 1795075891213871638);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xcacb8576a7c0100d001421efa552a622cac857f05543cea2fadb0b0dcd15fae0', '\x01150b192dc04d96479e7d4efc2b9abb78fe96cec35fa3ffa017178d54e5a14d2ceeadbf33ae8d462bcdef32b2', '\x989e03e5be7a68d1dd27940ec1ee050f', NULL, 0, 1795075891224254789);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x37226a8f8cd1a59c8466e72e4f64968adf06b6dc82a25906c03ce192c40e9b2e', '\x01c8de7c3ab544089df63a641b0b60f64e2b544972ddf9baeccfb485bee21e9f729506deb9434d7bb563f3d956', NULL, 1795075891318545661, 0, 1795075891232916656);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xe2119349068f00069a74d75e14e6065b8bdb91b9a687e3371a777728218f2892', '\x01f82734ba9f117b3b36fb9b049fcf541e02ab742fb2cf8643faf80027ff9c5de260aa7381cc03f8abfb448cad', NULL, 1795075891324682337, 0, 1795075891242500836);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xd3f4620458107c9cfa798d78caa6d02b774bbc8e801590284aa7dbc5df595eb3', '\x01b71edd933b50009a11120da23e59882e8a0ad82b3e711d4d886194787f17a1418f330575e49e4dfcd5f09f07', NULL, 1795075891333091084, 1, 1795075891251431230);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xe0dbf1ebaf1f676e8590256cb237eb0460aa4d5887128900089d2e73181d3bfd', '\x019eeb0c43dad836d612a995720a959013f63e2f2783eadaa68f3e9a4c31bd4b9546dd0988c1a5fde831e170cd', '\x40ad6a6283a11efcafb8011cf4464a66', NULL, 1, 1795075891260835216);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x11da59cb0272cc9f04b83f16fb114f36ceded96caaec4b0917d5343e58596b20', '\x01aa53ce0fe3592edff9629a56476d76290571c973f69f8af9b2514c1712806c8979892a973d55a24a4de7fe31', '\x78311d6bf08f02e3e8337be8ca5bdf05', NULL, 0, 1795075891380259164);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x0dad5905c305301ed9f5ef16539c61b6c856640cfae13a76b64e42c84f096145', '\x017b6747f41bd92ce1039ca40a5e7e1aa418831f8f10ad35d8a74046adc51979f89fc4e2bccbe2f9b111a3c644', '\x00607846eb2bfbcc5059fc93baaff08e', NULL, 0, 1795075891389438012);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xd088fdb7d69a64cb5841f85525afae411e5693855491bfeca4d35f8473de3101', '\x01805889cb2b23df6f408b9c85d3d883c2e2c7b20d63e727ba3d604990ec8c045a113ab4b0f57e9ab455869382', '\x130a0b3777a8c4ca0fb664113ffb4e4f', NULL, 0, 1795075891397292232);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xe9b44e05e0a2b793d4ba3b91f400c93c1bd8610e5cf41cac7bb53c9d626a4e7d', '\x01428a4a89892161e1dca1b39902543e212e617d170cbf99741c430a56929bb0e96969bb556dbb32f8ade21f8f', '\x4d091ae1cec4be0b744e630c7eced2f5', NULL, 0, 1795075891405549257);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x4208d89e2aabb99224b8692ba5d88ce447c52ed22cf7b136599d4c495b89b625', '\x016b7628ba394aca2470b365cce3848025b9a2b490636fa5c73edf8329bd838e6a7e4ca4f064a648aeed352466', '\xd8c45ecbf1f9fae70caf9ae59990a54d', NULL, 0, 1795075891413554311);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xbbacbabe6a10afb16543d02aad297a1c59f130710a426a66228fad08f990f3e8', '\x01caf2fd68f34c313728198bf71eeb85f1c60097996c6486e3c4f6b89daef1871b4a4aa540e984e00aff179478', '\x92d80ff347bf79417ca0fd5c8f1f3fd6', NULL, 0, 1795075891421358753);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x1b4ac26d3564a7f46f8560222acbbda83d082da8162bd86145961dd014850517', '\x017f431bd8b8d6fedfcf76d3a5dc1467679c47a1da361e114d34ecb5f8d4877f3b57bf068b40f7b18852bb5de8', '\x8d3b193df3b77a0a63f5e0f6a14097b7', NULL, 0, 1795075891346936721);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x0ec7b34e984f8539e2c338d0b43dc8b7f50b6414aede5ac6a49f405eadcbf7b2', '\x011fd36059cd43bbf0a83d9408c154fb45c50955fd6bd9cb1a545dabefc674ce2c88e5efc4ced2eb4bb47e24cb', NULL, 1795075891458196080, 1, 1795075891363176198);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xf7721bde95d5467bdf5a958948e4fa5a448f0b0bc0725ab6ca99b0f02c11591a', '\x0145c7389256d6c4bdd7e4b1382995cf518aa952a8174414a0641cae93d24a53c10eb3d6c925fdbc5700043f0b', '\xd5fc1e59ac1be35cf8ce9de160381c7a', NULL, 1, 1795075891372228463);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x33c0725c862fea5528e40d4253335a6648ec721b321ec6ffc0c7e6be00dda7a8', '\x01a13f3541ed7c60b27ae21121e951ea5b681f5b573279b1dcc20c2f1998217ff8e7630d6a4100decfba43df9e', '\xc2d9eeb470303955029d9f406d433165', NULL, 0, 1795075897102600583);
INSERT INTO vcon.vcon VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x4fbfc125cfd49bc7a07128ac3f407a642e6ee583142e1fc8f1302f5691a4c888', '\x01488461987d1b34a310491c5781cd6710fa14c2e1dcec6a1870e49932eea95d6db6a8a7a4d070921d12b936c6', '\x32ed69c747896249375217ccf9347205', NULL, 0, 1795075897112306430);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x0f8c4f37954cadaebc52d45d583e762cac2a00f7e99c74aed11aa7bd433a2426', '\x011fe8c0abec07ff2a0b7ec13dc652942d9dccc12f7544ee48aeed6b4f8f10049f25ebe12645a9d798c85ad0b9', '\x74f17484552d36bb5d0fe580e60eddcc', NULL, 0, 1795075897120923389);
INSERT INTO vcon.vcon VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xfcbd1f4407a4bf72e440f7e0344d3e3f4c6a365396f92e4b3f49d77c995c771a', '\x0107b3fe7f6129d554f5d713a63fdfc1a9a8c80b0ae0a7ab342b348737cd5a0170306a65f227acc6d7ef86db3d', '\x8ef13d3b3e773de7ca424579529f0eac', NULL, 0, 1795075897130396228);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xee75e810c1550dfb929efb3a5525e9ee', '\x0ec7b34e984f8539e2c338d0b43dc8b7f50b6414aede5ac6a49f405eadcbf7b2', '\x01f343be4c61a5984d510fc93a4d703d7e34bc5f51ad85cdf46b4381034877e91776e08181', '\x017d5e1c94d8abb40e0d293092c12b403b81c48f5edc98b7107966834200', '\x01c895f30309eebfb2a6325b66eeebd5676f7a2d4726d51d816a799a09bf10d3060602c0eb', 1795075891363176198, 1795075891458196080, '\xe3a3b4b78db60505', '\x01e275e1bd0133c5e6f461f9b0f20b9b6abc01648623328051082f655313b7abe73527292631c88b820461e1b683d46b7582674d1d66633ada37439b13', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x35391eba1fdada77a8bb3d8ad3fab647', '\xf7721bde95d5467bdf5a958948e4fa5a448f0b0bc0725ab6ca99b0f02c11591a', '\x01c3ec3bb3abcd419e93067c531b4f91648f7b23110d02e33c648df2d12002caac7622435c', '\x012375dcbeb71de24cc3c409c3708ad8f4713d586f483a18e61b654fe10a', '\x01d1816de5fe2a2026d4340a53ade4dfda1a76c18bddcdc32049411029d6c6056b368aa004', 1795075891372228463, 1795075891473924807, '\xe3a3b4b78db60505', '\x016bb4fb2d0a8b9dbb0817bb73700be864d4fa29433121150ecad591aac3ed92b55ab4c78f33c7eef358d9ab76b4aaf44f92dc491582e5fe38cec69558', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x78311d6bf08f02e3e8337be8ca5bdf05', '\x11da59cb0272cc9f04b83f16fb114f36ceded96caaec4b0917d5343e58596b20', '\x014c2e06aec9a87f11b36a9d02aaf900ba78f30c0b5edc734a08e54b0f229eda3cedb26361', '\x01131bb4144b8236400414d9fed0f887c5275bd2501dfea248a837f1640d', '\x01f587820b85cdc8149f3c97002512047cc1f769a4f061332f5ae4ea488e0182b4de4086a1', 1795075891380259164, NULL, '\xe3a3b4b78db60505', '\x0128b8c54a12a524bf6153dd975e71e3954a30dc81bfe718747b94497e1d14ec3aecbcfed0e638fd4680aceb355de4f7a3901499189d6f216611039356', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x00607846eb2bfbcc5059fc93baaff08e', '\x0dad5905c305301ed9f5ef16539c61b6c856640cfae13a76b64e42c84f096145', '\x01c6adfc553f3bb31df0bd9668499938385e1c9436e11b89bcd541e962007c42fd9c89f4cb', '\x01484c1a5ac3fbfefe41fa253ae1dbee15085616714677b2e61e670298a3', '\x0175f4ab3d298703e3fdd6f6beb5d8b9d233ddd8104f069cdd76db606290bded9a22e41cf0', 1795075891389438012, NULL, '\xe3a3b4b78db60505', '\x019271089fa870b6d88cbbd268b4f37dbc34606012c51eef2a101cd168505acfb3856ba3353b92be65c097486cc9659d37334deb87a315a7de4c3b9f82', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x130a0b3777a8c4ca0fb664113ffb4e4f', '\xd088fdb7d69a64cb5841f85525afae411e5693855491bfeca4d35f8473de3101', '\x010351f50bd01f71eda6564aba668d0df9b5ea8c27705a7ae40007480e62bec2b7d845cf28', '\x01d0dba10d886799639ca19db1c2e2e214d116967ae6e4c3dbf1b3fbffb4', '\x015e2fc5a097c4f93686b6f31633ddbad32cec5411926cd04c7d54a07fa39980e810070b99', 1795075891397292232, NULL, '\xe3a3b4b78db60505', '\x014fd7ca5604becb1788a6c2c85b1619ab40f7275c4947300034ce71f589a2cfa9bd91256a0db6ebc20f875ecd88f3ef5d303fe2a3cea49d88dc74dbc1', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x800566393a6eb84d733fdb20e7c4f109', '\xcacb8576a7c0100d001421efa552a622cac857f05543cea2fadb0b0dcd15fae0', '\x0145d7f7370f4ff8e5bd6cc9431d04939e1df7639c6a13161626d03d0703b5be9080dfa02c', '\x0192ebaa4d8f5cd4bb2db333ce4e1784040b88657d94f56908b4248078ca', '\x01e504fd8ec793f07cf9cb9c4d76bbc176bbabc2a5552c2e59b7f2135fe7dd15b843906636', 1795075891224254789, 1795075891314423194, '\x9105465dd519958d', '\x013423d9729568b90c5d94a8112ffc3f0b30f0a3582e06450552816abd09f353338b8c020a70ae54079d1acb992f1d42dd79c93dcf2927fd1d5603a90f', '\x314cbf943f66860f839ee6cb26d760b3', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xadf26912b71c0fbb734f4108f72a6fdb', '\x1a8730ecc8ff3c9eb3234b65e5042bcb48ee07abd543a3dfa4861adba89fb8da', '\x013f0866e05020b626491640593f9802d30ea5d6ecb496c9f7bd7eae2b0436a1c4644f4ee4', '\x014337e4cf28bee86d39d24279d3c7b2e8e102cb305b5995d1c442801e56', '\x01f2e80eac8fb7364d22d1093ecfe74c2f6534a893dc22eea89f6e1e29f95b280eceea367b', 1795075891213871638, 1795075891304963189, '\x9105465dd519958d', '\x01eaa330066aaeb701c0734287f1bc87e507dda88f8c6cb126cd7230c4a6e4db5e1b00f71761be35455a497665e1ed38f6e1de8164464869cfcfc36508', '\x314cbf943f66860f839ee6cb26d760b3', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x204c1c781833ec943f9571e4f7fbbdc9', '\x37226a8f8cd1a59c8466e72e4f64968adf06b6dc82a25906c03ce192c40e9b2e', '\x01871bb61b9fb899b36fbd8f8260596503fcfbff32495d9835f1e16301bb3532f9014f87dc', '\x013fe496fcad6f8c03abac45210aa7db8409df6dae52c7b472e01d7efb5c', '\x01d35adb7a20b7d646cf06899119573a5958015585421e055da766321cf2e49c94e9183afe', 1795075891232916656, 1795075891318545661, '\x9105465dd519958d', '\x01eb779298bccda5f6f23e5e129901b1bab2e8980329b241322eb080e05977d82fb272fcbb38bc621944a6b528d6377e833272cea7ce28ffdf62f12c4f', '\x314cbf943f66860f839ee6cb26d760b3', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xfac1050016f218f2cd19cc993323c51e', '\xe2119349068f00069a74d75e14e6065b8bdb91b9a687e3371a777728218f2892', '\x0108c5821cf71b24a55847bf0c207d9b40ebd1b367d033c9d64748f3ddaded1c7fad5d38c9', '\x01ba2c63d76abfba5261a2de84a332591e588fc517597523657bd718641b', '\x01ab9bcdf4fccb4cdca26035af8f99480809bf648a9476de6f3fa9baabfdb95c6b442452ef', 1795075891242500836, 1795075891324682337, '\x9105465dd519958d', '\x011f06e8de95e9dd765b580e0272f6a1055ce54e8d9c05fa57148c23546383a27f60a4af6a3d36ab2c6b996fa48b3e9ec3e679c73a278ffd94da5e4f69', '\x314cbf943f66860f839ee6cb26d760b3', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x4d091ae1cec4be0b744e630c7eced2f5', '\xe9b44e05e0a2b793d4ba3b91f400c93c1bd8610e5cf41cac7bb53c9d626a4e7d', '\x01b86a9c398e9ce3d79ca5cb3dffb6f8a3dcd1b1adcd4e236f70b1030adc0aac1663c840a9', '\x0144d392931250721a8aff81053bb1232c4e4f6ed3db2279d0327fe2c8b2', '\x018d3de858e6c5bfcb2c447feb88d214e6d614973d1de65742337f3dd5f09cfb72b91f54dd', 1795075891405549257, NULL, '\xe3a3b4b78db60505', '\x01695ce51cda50ac99d5749213c9e62d54a7e1195677031c1a5f9c0b747871e0eec1eb854d6366a1767abc97994c6cb7adef1470fb0af683ffb7384b9e', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xd8c45ecbf1f9fae70caf9ae59990a54d', '\x4208d89e2aabb99224b8692ba5d88ce447c52ed22cf7b136599d4c495b89b625', '\x0184977c02130d3be1ce23367baa4983d2e30fb550179d498069a0b01a6869741ea3c4462d', '\x018b297cf118306fbf0db86f550eae70f5c24633431f76b5dfa54bc6a92c', '\x010122854055f710a73a255eca923b8c27f44331164575525046fb0cde6e51a937602ce554', 1795075891413554311, NULL, '\xe3a3b4b78db60505', '\x01e8ac39356ede98e5dace975cae9bd66ca13f0b03e6fb2fcda41396901885decee5c403fea0683ec97f3964d01c2ecb0f7aaa6b5ef35479e4560a3a1c', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x92d80ff347bf79417ca0fd5c8f1f3fd6', '\xbbacbabe6a10afb16543d02aad297a1c59f130710a426a66228fad08f990f3e8', '\x010399f422c87f09e1860690d82dd29e7e1113a62cfeab7789f5b7f5a68084fa984018affc', '\x01541092d997817d1611a3aaaa19b366f064b7430144ead79b9c3d1d5bc2', '\x01d94a6d1d3172c2138f75d860ca399cc0708f29c9f39a29dd697055e8dbfa2ef0921b33a3', 1795075891421358753, NULL, '\xe3a3b4b78db60505', '\x016a358149c37040afb333f51b36a2f7b11dcd9d45056b8720e7896d9897231004bbc2aede35f108e49e635a1e315d27bc042e06d65b98839801b036a5', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x8d3b193df3b77a0a63f5e0f6a14097b7', '\x1b4ac26d3564a7f46f8560222acbbda83d082da8162bd86145961dd014850517', '\x01e3103d1e4b4c8832679c19883a4cc1cf5c413da139b66f980a6e5d6699a096be21599e57', '\x0182acb91901fc819c8cb93a22db305208bf112fc0e38e5e92ae49591d2b', '\x012f6ffcd76878c8fa615d4e2e358c995bb1fa99712cdc8df80bd850e64139e0aae3e84885', 1795075891430159407, NULL, '\xe3a3b4b78db60505', '\x01df766a6b4bcb0dc81336a8e5b20b7501af8f95d481de646f0ae99b6518376136b9f29663d9a6eba1ba0b974864865f2b6792e7ed3b3937af562dd83b', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\xd5fc1e59ac1be35cf8ce9de160381c7a', '\xf7721bde95d5467bdf5a958948e4fa5a448f0b0bc0725ab6ca99b0f02c11591a', '\x010fe5b33b9c727f233166f33d3caa2bcd51f5b3a36292d88d763b7f759feb097b30c0f751', '\x0134f93cac355ba2b100c39af302632eb748fda8842269035febb109f629', '\x0159c3549178944c5482469c496dd66518a15834520c87ca0b63516529fe96b51644807e2b', 1795075891473924807, NULL, '\xe3a3b4b78db60505', '\x011e297970dd2edbf8e0a56d5e98b7d076879957bc5a8432df037f1ff82a41195bbf24a32f5d3716cebd62eb723be21a704f51cded92a4ca072d68930b', '\xbbef02dcc92ad4bf3ebe810d0078b2b9', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x792928dff89c9bc05ae9a4d54e8c5789', '\xd3f4620458107c9cfa798d78caa6d02b774bbc8e801590284aa7dbc5df595eb3', '\x01a39eb62805133d0870fd8f1de2c207b0b19a37e8f746134b76ac89b1dfe2d3774b52e89a', '\x01d24cc43e4fef7b22ba5d8b71643e115ec849fc5c79cc1a37237c349fa3', '\x0148f6f539c935761a31dd0afdda950712b32f61dc615e1a84b04d13b84bd1cc8ff0eac95d', 1795075891251431230, 1795075891333091084, '\x9105465dd519958d', '\x01e9612b142c1c28d294501220b93ce06e2ccb075304fc6a2a2c588a6c74c8b1c9b7e6f6bb1f7e0a90a67af270f99340f6de9cefc0a04ebf537601fc2f', '\x314cbf943f66860f839ee6cb26d760b3', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x40ad6a6283a11efcafb8011cf4464a66', '\xe0dbf1ebaf1f676e8590256cb237eb0460aa4d5887128900089d2e73181d3bfd', '\x01ea59e2a4692e6850bae320d0c41178a326355485af1b5d55888394dd04d2bba8f44edf19', '\x014b2dcd27d0cd96f678a2b69c732c7e80381104099e62e6a204e4f87baf', '\x01391949ce29e908bd6a72a9b644aeeed7d865c3a5d653f7f57a8de9cc33f81bb868488149', 1795075891260835216, NULL, '\x9105465dd519958d', '\x012201514b12f879478b7ef4d2ac4df789467b59ef0906afbc552630953942c896c87362bfd2d086bdcd3cdfc2e4a72f2e0eab7fc05ce0b49197ecfde0', '\x314cbf943f66860f839ee6cb26d760b3', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x2f8d0af0a6bdc83d5adcc26721d7a6fb', '\x536add1560cd231f3dbe90112d8942cb61a8582ac38a7f3fb1d3ed83b6c5e4af', '\x013994fd5b326383555f63833c0daeaaea335f60dc6c62d97e0ef6c9f7b3200b5808f7f3c0', '\x01057bb0479afa300a69e286a206d37207be80e956c29d249fb3dd99a46f', '\x01277396e080ed48679b6f006428cc891c108efde9e2d06a0991b43821fd40b7483b115e25', 1795075891270155407, NULL, '\x9105465dd519958d', '\x0118e76311f906074d095ec085d43c636ddd9baeef70fe33b975dccbb46615e7f934439e0ab34cdab3164225e18bc96c3003529434542ab35afd2bfb12', '\x314cbf943f66860f839ee6cb26d760b3', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xc425bc5573e8313221ed5667f61d858d', '\x584d5e0fb495737c7e7ec5383662115da70ce847388163fbb3f25c2988a85638', '\x0178753e47af3bf03cd76273f670755d4f3e4e8b8b67b21c7975d930bafeac025dfc206043', '\x01908affad668ef4ad007cd1ce9f3cf1de8f311c9d92fcca79a1709c1040', '\x01a56919c3e29838f5c48469df7f7ffddf9c8ccbe0989bc3009b8545fab16310f8f79b5e5f', 1795075891278504469, NULL, '\x9105465dd519958d', '\x0198fb79408daf70c14ec1638b1d0c8beb27585d3b37a901cfe2886b96ea26b4b6c1fcbe96198c4c2a6b55349c058c10c72eb3c517d95ef01f7b9115d9', '\x314cbf943f66860f839ee6cb26d760b3', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x20ac82c95461cbeb032d52bed1537955', '\x1e2c225df0db6db13002e9bb3af626eb336e4199bd263066c7423735e05fd7d2', '\x0153f946b46f88d575de21a0f80f018be1f994d0c8d0ba7391b2dae002da07a2fe6c2f3c1e', '\x0114f1e825b2f2ec9c8ec3fb2843489d6eb87cb58263e60e1e888cd611cf', '\x01573bccef398af25bab693c7dc695faf841a762c75ef08817b5bfccd28c7109c3d395203b', 1795075891286975724, NULL, '\x9105465dd519958d', '\x017e344e4ea7a2de92f69ff9af279fa106fec2351d74ee3a1ee066ba6f2bdfb2d9cf44eb14c83a5caea2caf14b8d3c38c42fca2aa838325f3f90e5bc44', '\x314cbf943f66860f839ee6cb26d760b3', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xf4eba540e69b860c9d6b6ea0eaf54af7', '\xd537d99dd0f88a0f2e5cc0dba7b618e1ba312193e5f983d3f1cd84836037aa58', '\x01fa789e0cdd2a93f060ab2f5a0eec6db3834fb0293bfb29dee4b2e9be846728841eec0c94', '\x01710da608b52a5c9163d772336de8af8624ae5f486c53ea96f9a2e3b425', '\x01f0566d59fbaca60760940e4368c62e551f9df0f31b1443760e20975b343cb2e341966b34', 1795075891295341666, NULL, '\x9105465dd519958d', '\x01b4754f8760fca854d6dc5818d186da60f64510c154eb2fac605734e94d7d9658827681b4af9bceec85fa33acb508ed6a97d11990a4386cc5c2068ecf', '\x314cbf943f66860f839ee6cb26d760b3', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x1ac67498b0920ad3bd709df46f7ead7c', '\x1a8730ecc8ff3c9eb3234b65e5042bcb48ee07abd543a3dfa4861adba89fb8da', '\x014f68c9ffe2607f7aa83614f6fea9127d34b364aa505f77fedd5bc70ecf61211a494c64ad', '\x011b7abe8092553debfcd415d588d2e8c2067069eaf06fc20a709b0037ca', '\x01d43da99fdae20cf5b1497c169a1d4b64c5b104323d4ec93b5de9b330892cc971afd4658e', 1795075891304963189, NULL, '\x9105465dd519958d', '\x01b00fc56deccc5b3a56c106e64ae573e23c74a057a787582457e404292962bae671b5a334232d5b9c8f83499b0cf74fb6ee8c564d10e5ee9f1b07b7d3', '\x314cbf943f66860f839ee6cb26d760b3', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x989e03e5be7a68d1dd27940ec1ee050f', '\xcacb8576a7c0100d001421efa552a622cac857f05543cea2fadb0b0dcd15fae0', '\x0142875ffb6bb89f90cfb4d3f876d99d15b6e19a79e2f0793c52412704756857974c11d6ad', '\x018f4d3d1cfe50a5205e34e1fdc5d527230293fdfb35400087868438af4d', '\x0168709e5ec687b2310d23d6c47ca9f704a99641519098f2d89bc7568067ad4765f1248955', 1795075891314423194, NULL, '\x9105465dd519958d', '\x0148db534713d2380b83e5dacf276e530549c89bb5e859bbb222aff7ea7693f40c6e3abd39a703d63edfe5dc02728642e2ef174bf22541c411c1af0dd1', '\x314cbf943f66860f839ee6cb26d760b3', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\xc2d9eeb470303955029d9f406d433165', '\x33c0725c862fea5528e40d4253335a6648ec721b321ec6ffc0c7e6be00dda7a8', '\x01b3c1f21469dfd06d2cf8ce4f663bf3504a063ffc2034b0945d7d26bf28adfce384201e61', '\x01aaf930860a0d078a6cfb06dacf91f84f9cb5c0948b95d7f341b05fd383', '\x01c03cca9c09ffe2d245e78f8892558b697adb6a57c730591d94788a764a3a76d321afee95', 1795075897102600583, NULL, '\x9105465dd519958d', '\x01f64fedadadbfc51cf57e7050fe51c83272cca13be8d2fa4ff5ad665f326965b4deac4828e9e1fffc0427732dc34e55b359b8d8cf95d5219588f24495', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2310938706cc9455ba496088cfe81c99', '\x32ed69c747896249375217ccf9347205', '\x4fbfc125cfd49bc7a07128ac3f407a642e6ee583142e1fc8f1302f5691a4c888', '\x01ed2d55abc97aa47f681ec3efe1e02f1c7ca8cf2468a40dbc9a9d1e806f6ad9ead83ecd8a', '\x01fa9e8671dad78448037c5d91a66319eaee94356d22748132c906ff1e4f', '\x011f87275ee85d75ec2ab84f25f815dcf4bbecae7db1de89d71eb4f4622c26d7b8d70eb329', 1795075897112306430, NULL, '\x9105465dd519958d', '\x01d2ace28ddf2d8a3df4d934e1d4ea84e92da4fcaf160f522f99c349757005b5faca53c3d940067a22cac89cbbb549e3827621097a1b339c3b2eea0857', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x74f17484552d36bb5d0fe580e60eddcc', '\x0f8c4f37954cadaebc52d45d583e762cac2a00f7e99c74aed11aa7bd433a2426', '\x015661bfc7a17b084bb02e618f7c66be181e736b84b429523992afb7941a29c36a37f3fe3a', '\x01c125578b2dd975ce08a56a66a03717b346c6f37c26fdad57c4077a4bb9', '\x01d130e497d0fea346bae16246b8ba27a867ca99ed441728d80481698a1f80ed98c111e5dd', 1795075897120923389, NULL, '\xe3a3b4b78db60505', '\x0136457db9e88e6528fed7db45ef14d7c65ddc303fc9e67593ee867881cf26a5d8d4852ed3070accb3b00f439be1afca70a6a00690577bb04f11e66c30', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x27395d36d38a1cde2fd7540275f4be0a', '\x8ef13d3b3e773de7ca424579529f0eac', '\xfcbd1f4407a4bf72e440f7e0344d3e3f4c6a365396f92e4b3f49d77c995c771a', '\x012b98545c159bf3c6b4482bbec779b5f0ff2385d11b61de76b8c941a3754c1eca409a2f6e', '\x01d74ba37133059f4b4431f048e766a559cb9dc43fa0a13ade221b8cb909', '\x01895294d611109551651a49b7ea1f1fafc029b44a66d387005e66db8a9e4f5d33149e63a2', 1795075897130396228, NULL, '\xe3a3b4b78db60505', '\x011a78ea88c45af823b5e8dccb4e8e45a9f30a1fc7459df6013a03a43b5f129cb95212a0389939574c00220346da528bb61933acb892a61d434582e0e5', NULL, NULL, NULL, 56);


--
-- Name: admin_audit_seq_seq; Type: SEQUENCE SET; Schema: vcon; Owner: vcon_owner
--

SELECT pg_catalog.setval('vcon.admin_audit_seq_seq', 24, true);


--
-- Name: audit_seq_seq; Type: SEQUENCE SET; Schema: vcon; Owner: vcon_owner
--

SELECT pg_catalog.setval('vcon.audit_seq_seq', 274, true);


--
-- Name: manifest_seq_seq; Type: SEQUENCE SET; Schema: vcon; Owner: vcon_owner
--

SELECT pg_catalog.setval('vcon.manifest_seq_seq', 4, true);


--
-- Name: version_seq_seq; Type: SEQUENCE SET; Schema: vcon; Owner: vcon_owner
--

SELECT pg_catalog.setval('vcon.version_seq_seq', 56, true);


--
-- Name: admin_audit_key admin_audit_key_ordinal_key; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.admin_audit_key
    ADD CONSTRAINT admin_audit_key_ordinal_key UNIQUE (ordinal);


--
-- Name: admin_audit_key admin_audit_key_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.admin_audit_key
    ADD CONSTRAINT admin_audit_key_pkey PRIMARY KEY (key_id);


--
-- Name: admin_audit admin_audit_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.admin_audit
    ADD CONSTRAINT admin_audit_pkey PRIMARY KEY (seq);


--
-- Name: api_key api_key_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.api_key
    ADD CONSTRAINT api_key_pkey PRIMARY KEY (id);


--
-- Name: audit_key audit_key_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.audit_key
    ADD CONSTRAINT audit_key_pkey PRIMARY KEY (tenant, key_id);


--
-- Name: audit_key audit_key_tenant_ordinal_key; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.audit_key
    ADD CONSTRAINT audit_key_tenant_ordinal_key UNIQUE (tenant, ordinal);


--
-- Name: audit audit_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.audit
    ADD CONSTRAINT audit_pkey PRIMARY KEY (tenant, seq);


--
-- Name: key_use key_use_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.key_use
    ADD CONSTRAINT key_use_pkey PRIMARY KEY (tenant, tenant_key_id);


--
-- Name: manifest manifest_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.manifest
    ADD CONSTRAINT manifest_pkey PRIMARY KEY (tenant, seq);


--
-- Name: master_key_use master_key_use_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.master_key_use
    ADD CONSTRAINT master_key_use_pkey PRIMARY KEY (master_key_id);


--
-- Name: parquet_file parquet_file_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.parquet_file
    ADD CONSTRAINT parquet_file_pkey PRIMARY KEY (tenant, file_id);


--
-- Name: purge_pending purge_pending_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.purge_pending
    ADD CONSTRAINT purge_pending_pkey PRIMARY KEY (tenant, file_id);


--
-- Name: registry_key registry_key_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.registry_key
    ADD CONSTRAINT registry_key_pkey PRIMARY KEY (one);


--
-- Name: schema_version schema_version_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.schema_version
    ADD CONSTRAINT schema_version_pkey PRIMARY KEY (version);


--
-- Name: session session_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.session
    ADD CONSTRAINT session_pkey PRIMARY KEY (id_hash);


--
-- Name: storage_claim storage_claim_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.storage_claim
    ADD CONSTRAINT storage_claim_pkey PRIMARY KEY (one);


--
-- Name: tenant_catalog tenant_catalog_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.tenant_catalog
    ADD CONSTRAINT tenant_catalog_pkey PRIMARY KEY (tenant);


--
-- Name: tenant_lock tenant_lock_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.tenant_lock
    ADD CONSTRAINT tenant_lock_pkey PRIMARY KEY (tenant);


--
-- Name: tenant tenant_name_mac_key; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.tenant
    ADD CONSTRAINT tenant_name_mac_key UNIQUE (name_mac);


--
-- Name: tenant tenant_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.tenant
    ADD CONSTRAINT tenant_pkey PRIMARY KEY (id);


--
-- Name: tenant_setting tenant_setting_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.tenant_setting
    ADD CONSTRAINT tenant_setting_pkey PRIMARY KEY (tenant, name);


--
-- Name: tombstone tombstone_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.tombstone
    ADD CONSTRAINT tombstone_pkey PRIMARY KEY (tenant, uuid_mac);


--
-- Name: user_account user_account_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.user_account
    ADD CONSTRAINT user_account_pkey PRIMARY KEY (id);


--
-- Name: vcon vcon_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.vcon
    ADD CONSTRAINT vcon_pkey PRIMARY KEY (tenant, uuid_mac);


--
-- Name: version version_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.version
    ADD CONSTRAINT version_pkey PRIMARY KEY (tenant, version);


--
-- Name: admin_audit_counted; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX admin_audit_counted ON vcon.admin_audit USING btree (actor, action, status, at);


--
-- Name: api_key_tenant; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX api_key_tenant ON vcon.api_key USING btree (tenant);


--
-- Name: audit_counted; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX audit_counted ON vcon.audit USING btree (tenant, actor, action, status, at);


--
-- Name: membership_as_operator; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE UNIQUE INDEX membership_as_operator ON vcon.membership USING btree (user_id) WHERE (tenant IS NULL);


--
-- Name: membership_in_tenant; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE UNIQUE INDEX membership_in_tenant ON vcon.membership USING btree (user_id, tenant) WHERE (tenant IS NOT NULL);


--
-- Name: membership_of_tenant; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX membership_of_tenant ON vcon.membership USING btree (tenant);


--
-- Name: session_in_tenant; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX session_in_tenant ON vcon.session USING btree (tenant);


--
-- Name: session_of_user; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX session_of_user ON vcon.session USING btree (user_id);


--
-- Name: user_account_identity; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE UNIQUE INDEX user_account_identity ON vcon.user_account USING btree (identity_mac) WHERE ((identity_mac IS NOT NULL) AND (deactivated_at IS NULL));


--
-- Name: vcon_listed; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX vcon_listed ON vcon.vcon USING btree (tenant, stored_at, uuid_mac);


--
-- Name: version_place; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX version_place ON vcon.version USING btree (tenant, place_file);


--
-- Name: version_uuid; Type: INDEX; Schema: vcon; Owner: vcon_owner
--

CREATE INDEX version_uuid ON vcon.version USING btree (tenant, uuid_mac);


--
-- Name: admin_audit append_only; Type: TRIGGER; Schema: vcon; Owner: vcon_owner
--

CREATE TRIGGER append_only BEFORE DELETE OR UPDATE ON vcon.admin_audit FOR EACH ROW EXECUTE FUNCTION vcon.audit_append_only();


--
-- Name: audit append_only; Type: TRIGGER; Schema: vcon; Owner: vcon_owner
--

CREATE TRIGGER append_only BEFORE DELETE OR UPDATE ON vcon.audit FOR EACH ROW EXECUTE FUNCTION vcon.audit_append_only();


--
-- Name: admin_audit not_emptied; Type: TRIGGER; Schema: vcon; Owner: vcon_owner
--

CREATE TRIGGER not_emptied BEFORE TRUNCATE ON vcon.admin_audit FOR EACH STATEMENT EXECUTE FUNCTION vcon.audit_append_only();


--
-- Name: audit not_emptied; Type: TRIGGER; Schema: vcon; Owner: vcon_owner
--

CREATE TRIGGER not_emptied BEFORE TRUNCATE ON vcon.audit FOR EACH STATEMENT EXECUTE FUNCTION vcon.audit_append_only();


--
-- Name: admin_audit number_rows; Type: TRIGGER; Schema: vcon; Owner: vcon_owner
--

CREATE TRIGGER number_rows BEFORE INSERT OR UPDATE ON vcon.admin_audit FOR EACH ROW EXECUTE FUNCTION vcon.number_row();


--
-- Name: audit number_rows; Type: TRIGGER; Schema: vcon; Owner: vcon_owner
--

CREATE TRIGGER number_rows BEFORE INSERT OR UPDATE ON vcon.audit FOR EACH ROW EXECUTE FUNCTION vcon.number_row();


--
-- Name: manifest number_rows; Type: TRIGGER; Schema: vcon; Owner: vcon_owner
--

CREATE TRIGGER number_rows BEFORE INSERT OR UPDATE ON vcon.manifest FOR EACH ROW EXECUTE FUNCTION vcon.number_row();


--
-- Name: version number_rows; Type: TRIGGER; Schema: vcon; Owner: vcon_owner
--

CREATE TRIGGER number_rows BEFORE INSERT OR UPDATE ON vcon.version FOR EACH ROW EXECUTE FUNCTION vcon.number_row();


--
-- Name: admin_audit admin_audit_sealed_by; Type: FK CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.admin_audit
    ADD CONSTRAINT admin_audit_sealed_by FOREIGN KEY (key_id) REFERENCES vcon.admin_audit_key(key_id) NOT VALID;


--
-- Name: audit audit_sealed_by; Type: FK CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.audit
    ADD CONSTRAINT audit_sealed_by FOREIGN KEY (tenant, key_id) REFERENCES vcon.audit_key(tenant, key_id) NOT VALID;


--
-- Name: admin_audit; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.admin_audit ENABLE ROW LEVEL SECURITY;

--
-- Name: admin_audit_key; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.admin_audit_key ENABLE ROW LEVEL SECURITY;

--
-- Name: api_key; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.api_key ENABLE ROW LEVEL SECURITY;

--
-- Name: audit; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.audit ENABLE ROW LEVEL SECURITY;

--
-- Name: audit_key; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.audit_key ENABLE ROW LEVEL SECURITY;

--
-- Name: key_use; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.key_use ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant_lock lock_deletes; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY lock_deletes ON vcon.tenant_lock FOR DELETE USING ((tenant = vcon.tenant()));


--
-- Name: tenant_lock lock_inserts; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY lock_inserts ON vcon.tenant_lock FOR INSERT WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: tenant_lock lock_reads; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY lock_reads ON vcon.tenant_lock FOR SELECT USING (
CASE
    WHEN (current_setting('vcon.tenant'::text, true) = 'registry'::text) THEN true
    ELSE (tenant = vcon.tenant())
END);


--
-- Name: tenant_lock lock_updates; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY lock_updates ON vcon.tenant_lock FOR UPDATE USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: manifest; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.manifest ENABLE ROW LEVEL SECURITY;

--
-- Name: master_key_use; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.master_key_use ENABLE ROW LEVEL SECURITY;

--
-- Name: membership; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.membership ENABLE ROW LEVEL SECURITY;

--
-- Name: parquet_file; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.parquet_file ENABLE ROW LEVEL SECURITY;

--
-- Name: purge_pending; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.purge_pending ENABLE ROW LEVEL SECURITY;

--
-- Name: registry_key; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.registry_key ENABLE ROW LEVEL SECURITY;

--
-- Name: admin_audit registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.admin_audit USING ((vcon.in_registry() = (seq IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: admin_audit_key registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.admin_audit_key USING ((vcon.in_registry() = (key_id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: api_key registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.api_key USING ((vcon.in_registry() = (id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: master_key_use registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.master_key_use USING ((vcon.in_registry() = (master_key_id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: membership registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.membership USING ((vcon.in_registry() = (user_id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: registry_key registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.registry_key USING ((vcon.in_registry() = (catalog_key_id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: session registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.session USING ((vcon.in_registry() = (id_hash IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: storage_claim registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.storage_claim USING ((vcon.in_registry() = (claim_id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: tenant registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.tenant USING ((vcon.in_registry() = (id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: tenant_setting registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.tenant_setting USING ((vcon.in_registry() = (tenant IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: user_account registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.user_account USING ((vcon.in_registry() = (id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: session; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.session ENABLE ROW LEVEL SECURITY;

--
-- Name: storage_claim; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.storage_claim ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.tenant ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant_catalog; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.tenant_catalog ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant_lock; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.tenant_lock ENABLE ROW LEVEL SECURITY;

--
-- Name: audit tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.audit USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: audit_key tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.audit_key USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: key_use tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.key_use USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: manifest tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.manifest USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: parquet_file tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.parquet_file USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: purge_pending tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.purge_pending USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: tenant_catalog tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.tenant_catalog USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: tombstone tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.tombstone USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: vcon tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.vcon USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: version tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.version USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: tenant_setting; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.tenant_setting ENABLE ROW LEVEL SECURITY;

--
-- Name: tombstone; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.tombstone ENABLE ROW LEVEL SECURITY;

--
-- Name: user_account; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.user_account ENABLE ROW LEVEL SECURITY;

--
-- Name: vcon; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.vcon ENABLE ROW LEVEL SECURITY;

--
-- Name: version; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.version ENABLE ROW LEVEL SECURITY;

--
-- Name: SCHEMA vcon; Type: ACL; Schema: -; Owner: vcon_owner
--

GRANT USAGE ON SCHEMA vcon TO vcon_app;


--
-- Name: FUNCTION audit_append_only(); Type: ACL; Schema: vcon; Owner: vcon_owner
--

REVOKE ALL ON FUNCTION vcon.audit_append_only() FROM PUBLIC;


--
-- Name: FUNCTION in_registry(); Type: ACL; Schema: vcon; Owner: vcon_owner
--

REVOKE ALL ON FUNCTION vcon.in_registry() FROM PUBLIC;
GRANT ALL ON FUNCTION vcon.in_registry() TO vcon_app;


--
-- Name: FUNCTION installed_version(); Type: ACL; Schema: vcon; Owner: vcon_owner
--

REVOKE ALL ON FUNCTION vcon.installed_version() FROM PUBLIC;
GRANT ALL ON FUNCTION vcon.installed_version() TO vcon_app;


--
-- Name: FUNCTION number_row(); Type: ACL; Schema: vcon; Owner: vcon_owner
--

REVOKE ALL ON FUNCTION vcon.number_row() FROM PUBLIC;


--
-- Name: FUNCTION tenant(); Type: ACL; Schema: vcon; Owner: vcon_owner
--

REVOKE ALL ON FUNCTION vcon.tenant() FROM PUBLIC;
GRANT ALL ON FUNCTION vcon.tenant() TO vcon_app;


--
-- Name: TABLE admin_audit; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT ON TABLE vcon.admin_audit TO vcon_app;


--
-- Name: COLUMN admin_audit.count; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT UPDATE(count) ON TABLE vcon.admin_audit TO vcon_app;


--
-- Name: TABLE admin_audit_key; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT ON TABLE vcon.admin_audit_key TO vcon_app;


--
-- Name: COLUMN admin_audit_key.wrapped; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT UPDATE(wrapped) ON TABLE vcon.admin_audit_key TO vcon_app;


--
-- Name: TABLE api_key; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.api_key TO vcon_app;


--
-- Name: TABLE audit; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT ON TABLE vcon.audit TO vcon_app;


--
-- Name: COLUMN audit.count; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT UPDATE(count) ON TABLE vcon.audit TO vcon_app;


--
-- Name: TABLE audit_key; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT ON TABLE vcon.audit_key TO vcon_app;


--
-- Name: COLUMN audit_key.wrapped; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT UPDATE(wrapped) ON TABLE vcon.audit_key TO vcon_app;


--
-- Name: TABLE key_use; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.key_use TO vcon_app;


--
-- Name: TABLE manifest; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.manifest TO vcon_app;


--
-- Name: TABLE master_key_use; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.master_key_use TO vcon_app;


--
-- Name: TABLE membership; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.membership TO vcon_app;


--
-- Name: TABLE parquet_file; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.parquet_file TO vcon_app;


--
-- Name: TABLE purge_pending; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.purge_pending TO vcon_app;


--
-- Name: TABLE registry_key; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT ON TABLE vcon.registry_key TO vcon_app;


--
-- Name: COLUMN registry_key.catalog_key_id; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT UPDATE(catalog_key_id) ON TABLE vcon.registry_key TO vcon_app;


--
-- Name: TABLE session; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.session TO vcon_app;


--
-- Name: TABLE storage_claim; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT ON TABLE vcon.storage_claim TO vcon_app;


--
-- Name: TABLE tenant; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.tenant TO vcon_app;


--
-- Name: TABLE tenant_catalog; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.tenant_catalog TO vcon_app;


--
-- Name: TABLE tenant_lock; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.tenant_lock TO vcon_app;


--
-- Name: TABLE tenant_setting; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.tenant_setting TO vcon_app;


--
-- Name: TABLE tombstone; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.tombstone TO vcon_app;


--
-- Name: TABLE user_account; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.user_account TO vcon_app;


--
-- Name: TABLE vcon; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.vcon TO vcon_app;


--
-- Name: TABLE version; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.version TO vcon_app;


--
-- PostgreSQL database dump complete
--


