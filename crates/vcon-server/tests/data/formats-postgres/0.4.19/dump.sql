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
-- Name: lease; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.lease (
    name text NOT NULL,
    holder text NOT NULL,
    taken_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL
);

ALTER TABLE ONLY vcon.lease FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.lease OWNER TO vcon_owner;

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
-- Name: pending_file; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.pending_file (
    tenant bytea NOT NULL,
    file_id bytea NOT NULL,
    started_at bigint NOT NULL
);

ALTER TABLE ONLY vcon.pending_file FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.pending_file OWNER TO vcon_owner;

--
-- Name: pending_tenant; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.pending_tenant (
    id bytea NOT NULL,
    started_at bigint NOT NULL
);

ALTER TABLE ONLY vcon.pending_tenant FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.pending_tenant OWNER TO vcon_owner;

--
-- Name: pending_write; Type: TABLE; Schema: vcon; Owner: vcon_owner
--

CREATE TABLE vcon.pending_write (
    tenant bytea NOT NULL,
    version bytea NOT NULL,
    started_at bigint NOT NULL
);

ALTER TABLE ONLY vcon.pending_write FORCE ROW LEVEL SECURITY;


ALTER TABLE vcon.pending_write OWNER TO vcon_owner;

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
    wrapped_root bytea,
    pending_wrapped_root bytea,
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
    role_cap_sealed bytea,
    wrap_ref text,
    wrapped_kek bytea,
    pending_wrap_ref text,
    pending_wrapped_kek bytea,
    CONSTRAINT kek_whole CHECK (((wrap_ref IS NULL) = (wrapped_kek IS NULL))),
    CONSTRAINT pending_kek_whole CHECK (((pending_wrap_ref IS NULL) = (pending_wrapped_kek IS NULL)))
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
    tenant bytea NOT NULL,
    removing boolean DEFAULT false NOT NULL
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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-27T11:29:36.305466Z', 'cli', 'tenant_create', '\x594ec3a2f8dd439fb0f8ebf03a3ba3959e36682c3006614ccac3de0296f4817d', '\x011e3c440e0f74b372fbb3d9761d5a45acf70489d2a54a49a28d13298fce9673c8bf9074955eb931b2c2ff2066aaed4395492a5373faee812f8f218f0b', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-27T11:29:38.132674Z', 'cli', 'key_create', '\x7201779458c71226701510213ce128cf0765b641a2f6b88e2434ed60bf785948', '\x01175063d4f635b5d14ce65d320871364904e388df108ad4ec28d4b437461fa2cca84dd40454f28f87ce570875', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-27T11:29:39.874966Z', 'cli', 'key_create', '\xa551a9e32d9c408a65248d0df526d44de21724cff61cd8eda482f48f0b6492fd', '\x0164e23c5db9c1d1c94a2ebbf2107a25c090e12bf048d0bc2b964cad6876967b94b42c1299536e4475e97bb919', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-27T11:29:41.686071Z', 'cli', 'tenant_create', '\x2c94c5288c9e6a9c98efbd992d7a76af60da9a153c29b46586ffbb47af7d0869', '\x01e6f2d02c5e586f1a4a74812b690536fd9c8380dbaa6193df2137626c0e3f89e187854b4adf582666447b5ea5e58d5d38c604b8bb715ed494f7ab8e0d', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-27T11:29:43.556538Z', 'cli', 'key_create', '\xcdc2763ab702849bac96c79c65052750f38459d03617e359ff464446f07f2e0f', '\x01c9e58845c4d053f01089fd5b0518621e1e098509aa27c442685a33a86a0286d1de8af3af280e64d6dcb814ac', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-27T11:29:45.368794Z', 'cli', 'key_create', '\x0c106f56a01d5a2bca8e0951bb04a1f40c407cefecf33b9a54f74cc7ec5398fa', '\x01096d6bf851fac567ebed7e786965b1db86eaa88f8fc7e7b6f568bf4ac45a49414868bdef771665856add0ecc', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-27T11:29:47.135077Z', 'cli', 'key_create', '\xbcc48c73e8105ac98fa0f7d102557f7333004578208fa7c92b2f8deb009d760a', '\x0153578919d342090c82f67d70e022303dbb5f723321d16523674d69628e8b9feb330e39b8946ae21f37033feb', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-27T11:29:50.053669Z', 'cli', 'user_add', '\x18aebe29bd6aac94951769fc51692391a5325786ae2ce7ed047bc3d916e9fd7b', '\x01c3a4090b0c923407cd10c154e8dbe21a1b122508667fc738c312b028b253fe2083887ca1f0a08a6289602c6d8787e00615f41230b76aa62feca75170', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-27T11:29:51.888781Z', 'cli', 'user_add', '\xfe9e75352e208ec8c4762906a239768e33acb8e5c4f64c39a0dd500f58b4ac1d', '\x015f3d1f1df32435def47956dc94fcd5c01ebf122d94e9c4cd62fba67b2e7694c16db65668f1c057933a383a01f90def04eab9f8ee49942d5b36eeddbe', NULL, 201, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-27T11:29:53.703345Z', 'cli', 'membership_grant', '\x268c24a73780d1397d36880b682f8ebe67d66773079c1cb525bee295cee77f76', '\x010db15f9065dc768eeed44ac3482259172deb18abcbb676c820e589625539a2092eda4ebe062abdc798da511fb0b0d0c255f64d03b013221268cfb50dd356ec46fd2f52e8e6b833214d1986183dd1e5dda110a4ea7b903212d5f60cd620d442ce67007513', NULL, 200, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-27T11:29:55.588016Z', 'cli', 'membership_grant', '\x4834d7ac5c7ab85a2265937b3e50a5403a0c2a3bd3c98d23cb841778ee347cf7', '\x01c885aec9e6577bb1c8ac764e6eec7ff9b015472ee09235674ddd6bf718acd28c2a745776e81881a2e56d77d677c8bbdc3ef6b49396b5951b9bd1577b738bac40f52576339dd3f16b7db0be0d6b4a0cc337cf5d772ccadf8da88e36572e4cb61d9a2426bec0', NULL, 200, 1, false, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-27T11:29:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 1, true, '\xbad75ba866c84081');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (26, '2026-11-27T11:30:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 2, true, '\xbad75ba866c84081');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\xbad75ba866c84081', 1, '\x01fdbfd6e77157d0a54b5a0e94e7aa9704c67f7beadb972165cd1e22a209676f36a5f1428dd9b4a49914948a8c2c50b16b82f454cdc2601fc6bfe873e2');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\xf4827b0e021fda03', '\xb58b38b1a2f0b84bc845c84ad29ba658', '\x74e5a4d670f6496f6bb54740eed61d956f609dd437c0715d6e4e4a0691365f17', '\x0111e0fa95022bb51a7ceb6bbbd51065c6377f4bcd96676d5a03700333119e1a08', '\x01b4488a9dfc560fcdb4deb070ee507279e0eb69c2007a7a0ace0b3a826a74bcc9964b8559619f52ec4e9a13', 1795778979874966000, NULL, 1803554979000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xaafe141a90c900fe', '\x93f5d0d372e354ccdfc220cee1b49b87', '\xd30efb4c7ddd60cf983ebe45039d90a3710d03effe3aefe53e2b9338374d267e', '\x01fc31bfa42cc585f7979e7dcb8b254589db31c865b8cdf1056d6a19e3a6873abf', '\x01674c15ebec1cd1c59ba86b1af99a5a893298b34d9ba83b5eca1b989ddd028d76fd508791cfa37419aef872', 1795778985368794000, NULL, 1803554985000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xc07ee090f6fae354', NULL, '\x1dd771fa737efc4649cd2968eca7787b2915299e7acbd9edb521b62abb81ab00', '\x011ce05ea2a0a7ba1583913c0069e55fa28221aab115a7b07d2d5c70d26cd09f9285', '\x01a7c5e32ab8b21d7d4678cbcd65e10ec6be01d4df18492ffe71d46162d1ee5e', 1795778987135077000, NULL, 1803554987000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x4989e39d1b4acf4e', '\xb58b38b1a2f0b84bc845c84ad29ba658', '\xc2c78843c81546b7b2f58d8f439164d39048c54ad23df86cbe636b1e3ab2f2ec', '\x01bbf6571c2cae9c054eaedec2fbfdddd33f68fd3901f584c05c3943d2e42c7d935d9b4eb7979aafaaf92bb5dcd05dd4cdb46089b84bc9cee0e8', '\x01dd3fde5583d4e297d2ffd4b2e3fee17bbdd88acee00d8378da10ecfc43ce2d51dfd6e2eed31500ddb6304a3db7', 1795778978132674000, NULL, 1803554978000000000, 1795779000000000000);
INSERT INTO vcon.api_key VALUES ('\x407020b958b98a3d', '\x93f5d0d372e354ccdfc220cee1b49b87', '\x3cfa20369d4c9a67ca0cdd7c0688633bf5b8b649f7fa345a9548587b876ce8d0', '\x01b8e30865c82689ba3c22f7b14b9a16046f486bd64092645d2c7a5e9d8b598f83e05803d882cfa80c341a3dd8bc6868c4b0ca52e504101d4526', '\x01c15e5b4e3e632b6d66a17772e2461e4870d7a955cf9ee207d2f08c48b26ed6da6602d62b31d9a10974906e7979', 1795778983556538000, NULL, 1803554983000000000, 1795779000000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 2, '2026-11-27T11:29:38.138493Z', 'cli', 'key_create', '\xce10067b4c5943fdf0770e230ca71d9f9093c60e0d44b645bcad709c51d84c4b', '\x017f19a18f6d07fac371eadfdd9d4950be12968e8234c830aa38e1709b24e783c0d7fb6474f7566cffbb0c2781', NULL, 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 4, '2026-11-27T11:29:39.879916Z', 'cli', 'key_create', '\x08a99b92cf2b2d4230038a4b7bd3fc3db21346310373a63f456071eb7c22e383', '\x012f177d0e765db714ac541f29c917a7f1815a552b7b8baaa18dcb3d96158b8460ea89d022c20a0d312004bbe1', NULL, 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 6, '2026-11-27T11:29:43.562857Z', 'cli', 'key_create', '\x80197d20e4f24eb6092450a47e54bc3c1b9d54ac2abec02a9ee6482a0916432b', '\x011fb788989246bd076afbbea96a6eced5ba742a8e0587e62b5097bda2a41a4a9d76cb5fc14cd2d48580cfd8de', NULL, 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 8, '2026-11-27T11:29:45.375061Z', 'cli', 'key_create', '\x18620eac5f9522204847fe744a1a4634894d05382894f7c14807488abff3101f', '\x01e352f859d9fabbea395fb21b9aacc0493a4a875bbdc21ee99476d05ab455a83d1022499a1996ba6f1c4357b1', NULL, 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 10, '2026-11-27T11:29:53.710412Z', 'cli', 'membership_grant', '\x976ee3d211bbda6c5e1a7030c0e935b52299a5f3cee8ccbb3a5e0f4419973658', '\x0154470d3c3d44e6b590a9cd9dac9a9c70f819532b90e52fb942c7d6163869d830163dc43b195ee1acbd92e0757b0e63908de9bb23b85b6f73d8ad90dae809c97f9f62eb80d33881db5ab1b807a6a20a1189caeffdf81b79c8740dff8c2e39edb2ed174163', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 12, '2026-11-27T11:29:55.594325Z', 'cli', 'membership_grant', '\x38c9fdf7d6e8944b7479715b064f210671cbff1f2caad4e61dfca7921056928e', '\x016433ad1c57d638ba13aa2a30f7731a54a8aad87fbe773093d64fc2a4614bb04829d3a9299d656c444c7b2821028c3b02e20f92d6b5e243a41eed92899e83275f4b899f207a6a7699536bc120d3288bb0ff238f314556642bac2d878583a08e3f9d2d75d83b', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 14, '2026-11-27T11:29:57.954816Z', '4989e39d1b4acf4e', 'create', '\xd69335915e1f41ab54377cb9648589d89dab572c619b29116f73df7e9e9b5e19', '\x01e198e40ca7576458d1d6643aae1cd8e7060b029d8df5a759ebf61779244fb75e04886a36b950f3ea68b683cfb4b68f4bdb24c7679148adfe70fa7d928fe18b28', 'afc88b9fc3ccc88bdfed1d7965ed1078', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 16, '2026-11-27T11:29:57.993191Z', '4989e39d1b4acf4e', 'create', '\x4255fc120a28a6cf7c8a1783b06bb1eccbc2a16ea9d940e132bd343009d6b207', '\x01494da4be549449787b366fdb74428316188fd7ff1c875831ff35f0596d352e45c99b599038531e8d6a186e6c41baa3f380e9cba93cc42186264d2d670b72fb1f', '7badae7d037a408a06ee8ac03b286e8f', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 18, '2026-11-27T11:29:58.019354Z', '4989e39d1b4acf4e', 'create', '\x83a3ebe9146d7a798b3b62c7415d9632b8c2eff7950904efc4227d3622fee0bd', '\x01c86ad9e46ae3ff3a111d6d8706babffe72d46d6d524717f747f1eea3c81db8e5c39dff757f8da5f46da274481f02dbc96488a3d2328a57901d5f6061e98f19c3', '3bef0a3e9433961e0da8a3cda6ae7e57', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 20, '2026-11-27T11:29:58.044427Z', '4989e39d1b4acf4e', 'create', '\xda7f1ebf0ce7f23c1857bdb9a13eec5cbf64a9a9fbdfd111b3decbc2e548c3f4', '\x0178f38b9c51daeae2d34090b929e0add3150c9e86a4265a9e6355fdf919aaa2df15b23161769cc6f942d702e8b4c39b77bef4667f71c4d84a01283fb958bca338', '1fbcc3c80e4b09db703ecdd121010551', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 22, '2026-11-27T11:29:58.081428Z', '4989e39d1b4acf4e', 'create', '\xea010f4db1fab36fcb65d4e0d2eb3c1e895d6071d582bb550443b0d04a4e47c0', '\x01b64366f027daba88aa499db183820e09695322a8453c6b345f61aca5b9829cf88dec83c418986d09fcab68aba29eb4afa703b541da8172522a67383f12480db8', '2f07a13789b2e7c3dd22ef8d052e2a34', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 24, '2026-11-27T11:29:58.124972Z', '4989e39d1b4acf4e', 'create', '\xbce39087f18910bea3faa272cacbfa964ffae45d3bdb7c0a0bc26f437a6683e9', '\x01aa593817adcaab2fd6e032b8872c5550464dd7a96e914af964a09e0537511cac7e70eca16375f084ab5d30eff814ed67bb488850ef96523b39771f1ae184f9eb', 'be52d022b41d84d43e2db223937a8fea', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 26, '2026-11-27T11:29:58.162042Z', '4989e39d1b4acf4e', 'create', '\xa08467e5be1a169f0d33c776b3772c3911de5aac84b9ffb6b2da092e7abf1a2d', '\x014ab7579945a2be74fa57cf8bfc2ecfa9e1a595ef4bc69885dd1eb78cf27c84bddc115ca4cd0c881c51326e05f0b62a374aec91cd833936c215a423ce8879241e', 'fa9c498657ffa3680b4544df1b99fe4b', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 28, '2026-11-27T11:29:58.200215Z', '4989e39d1b4acf4e', 'create', '\xee7fda1b85e4a993271c51cb0de9b5c1818208ab5bc010e53e6e873790763059', '\x0129d901506eed3ce939134a572cba1d3c1e1c18e9c8a73e5fd0260883497419b7734ad10c7d253bf2b9b2b976ccb330305203dcdc3e060e5c38c52659f39711aa', '08230e03fc049ed6f9aef30e186d63ce', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 30, '2026-11-27T11:29:58.223726Z', '4989e39d1b4acf4e', 'create', '\x402a77886f5de5e1c57d3025564ca0afd25f7b10e062a138374ea7e1e4db7fb0', '\x01f75ac109c4bb06cdac3df60e91d4cd6c37224b8f0cc94ab51aacb83c8d82de1309ad6ee2f9f45fec12e7e11d87ccbd106fcde6bc32ce629752f86718df47f2aa', 'c439c3bdab4f1fc2bd53c6e1fe4e2778', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 32, '2026-11-27T11:29:58.243903Z', '4989e39d1b4acf4e', 'create', '\xeec14cf17b8b91cfe713e5c57d9b984e4283450dc78d1010140de43f18c91a1a', '\x01e4cecf35ae49ce76597850ed73be72a55fb7e7e5f12d9a41fce95b825c3fde39904b69633960283e7f03b4e4e1926087186e54b0bb076bceb18b3308a133fc74', '711a6ee8d63b28c568f6a8fe07b2da32', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 34, '2026-11-27T11:29:58.264956Z', '4989e39d1b4acf4e', 'replace', '\xd69335915e1f41ab54377cb9648589d89dab572c619b29116f73df7e9e9b5e19', '\x01ede341f34ab39e26223583017ed73388afb24de9d017894e5e3fb3e43244869e8ec2f2cae901ec580efb22425b9126a4c323da28d129b12a4352620e045f0b5d', '37d3a6f7241c9b2901a4fcc8a95ba778', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 36, '2026-11-27T11:29:58.286666Z', '4989e39d1b4acf4e', 'replace', '\x4255fc120a28a6cf7c8a1783b06bb1eccbc2a16ea9d940e132bd343009d6b207', '\x01bce76eef1d6f18b0497869ee128cdd3666751fe2d3aa4e52f8580a2666c91a3c7839404ac35306097ef26a4b82d668dbfe2abdd9eedf1c9d49dce29ba9d3a1c1', '2652e0493b3e6ad7b404599132deba71', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 38, '2026-11-27T11:29:58.292782Z', '4989e39d1b4acf4e', 'delete', '\x83a3ebe9146d7a798b3b62c7415d9632b8c2eff7950904efc4227d3622fee0bd', '\x018e27e9439de95dd03c99077128222284fb63a1aff7cbe901fb71573eabaae857eed0e42be14af6dd3d2477fede7ea2191530345828baae5a1aa9a308483fbf67', NULL, 204, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 40, '2026-11-27T11:29:58.298952Z', '4989e39d1b4acf4e', 'delete', '\xda7f1ebf0ce7f23c1857bdb9a13eec5cbf64a9a9fbdfd111b3decbc2e548c3f4', '\x01a731fb178534a96724a0a91d2105ddca6c6b7573c06c7f1a83a5c70d0c911f76187d335915d8ef48eb73c5fd1c88e3e61929871afd94a014b867f18206f62ee3', NULL, 204, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 42, '2026-11-27T11:29:58.304495Z', '4989e39d1b4acf4e', 'hold', '\xea010f4db1fab36fcb65d4e0d2eb3c1e895d6071d582bb550443b0d04a4e47c0', '\x0122c7dad66d9e2a5b1bcf762f46c57e66581b95dccd70fcb4c665fc830654b775084fca02f8d2f5ce597b7b0b749d14f5eda6c74f5949ac36b43634f677e7e03b', NULL, 204, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 44, '2026-11-27T11:29:58.308324Z', '4989e39d1b4acf4e', 'delete', '\xea010f4db1fab36fcb65d4e0d2eb3c1e895d6071d582bb550443b0d04a4e47c0', '\x01e31f4072e473aebc0785d55f9bd8d79169e6a58cd20ee102c5a3956ea31456c6a45e55eedc8f092d174b34a657c869a0d5f1c77a36e170211fa176a523cd6dfa', NULL, 204, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 46, '2026-11-27T11:29:58.313485Z', '4989e39d1b4acf4e', 'hold', '\xbce39087f18910bea3faa272cacbfa964ffae45d3bdb7c0a0bc26f437a6683e9', '\x0102de6bf37cd94bdc298bb239b9f59f5a35adbd2168ce1b7c7b74023f56064c8cae37e1f3bd8cf65b780972774e16f67508e66d39e984077c4c5beaee25dbde36', NULL, 204, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 48, '2026-11-27T11:29:58.355723Z', '407020b958b98a3d', 'create', '\xfbb458612d77f89bdcf53db795b6185850e78817eb04fe25ba3aa3f0756367f7', '\x01eb6dbe6b22ee554db3908e8c40b767107b469ea84a56db46ecb06d73b39e84d2916cc61bba2f1c769bc7920d37c957158785bdb89a96ce901ad7f439e811bec1', '3e3127b9358c761d0a1655e6987913e6', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 50, '2026-11-27T11:29:58.399737Z', '407020b958b98a3d', 'create', '\xb034fd1214435b79aff9a2ab1589a82861f400ba498ffd3bd8a967742f99db32', '\x01b1401fa21339c470a8f25c33fb3b011270b17cc5c04dda3fe9b4dc984e69feb2ed66d19a2068dc52468b782866b58fbb42e259b0ce40a6a72968a9b4a405dda9', '14110a22a55e21c5a8c5539342647c20', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 52, '2026-11-27T11:29:58.443739Z', '407020b958b98a3d', 'create', '\xa1ddbf7b94a4802c9de6f53c00ca9b8862e5ee92a71e7ebb76302bcdd34d1a02', '\x0172aa2afe2ca7d371abcff723dc63d1860a1f9e8b8db63d81dde77655a2b0e01a17d7265b4f41bec15837399b1175620bf2a3213864116c11cc52e84fd6905aaa', '20692fe6e50e91b26c289bc3cfa7330d', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 54, '2026-11-27T11:29:58.492529Z', '407020b958b98a3d', 'create', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x017f88663c43405d4981fcfa4622e7d6d3a08dffdce521cbc4a20b0568d45432d6d83e5363836a27020adacc36fbf33e11e42dae5a68a4cc3874d2ce59dc210b92', 'ba7795caf4a53e1f67b98152c5b5f054', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 56, '2026-11-27T11:29:58.536752Z', '407020b958b98a3d', 'create', '\xd6172d06225e3f89f5938d984a2d30c7257dd56aaad546efd2d6faf09e629606', '\x0172a49ea2081d31b9b09601bd688a2c30e48430c8a66c486d45f896acdb54a1e6f0af7956316bd73ca5751c56ca13a851d8c6b1b1e1093ffc5aec118ed10a9c13', '412284edb7b0601f8e5eb8fc18a77505', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 58, '2026-11-27T11:29:58.580329Z', '407020b958b98a3d', 'create', '\xe0fe94e711967c4c015f1d7b5f124e4836a364930c9632bc1b24954899aa8722', '\x011373327967cfd615ba988478115624addd093c2144a4e6de3250c67d1b7808dcf3332e0d8f1b2512cba3bd047cd1826d2afc71f0444bc8eaa2d3c6a8ca75a44e', '6402755df100697c062d1d35e44dcada', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 60, '2026-11-27T11:29:58.62424Z', '407020b958b98a3d', 'create', '\xceb755e2a736e1aa8ca10c2d2a2007fe498f69d07fa129a0f06778c8b3aa9021', '\x01b21194684bbbebc343f92d2bfe832412b66312bf9926738bc9731c76e61074853154493e2c1f27f8725c48be158149c6f607328c04bad8c0098253ef1f12ca83', '0af2ac0bfd1b794e1ba4a4707f009284', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 62, '2026-11-27T11:29:58.668081Z', '407020b958b98a3d', 'create', '\x85083b2e6d98e99cedd8f222e2949435387d8ec6d8abbe3900bac42aa3f1fe3c', '\x01c5a0cc36ce5dec52b0d24e2681531e1aeadeb7ff456b231c3fa65a234ee55bfe1315eabb6040ad33886ab4898bf4de0fdaffc1597f0e0a28b49882c522f43428', '12928a085361d94ed0db80de98ed561f', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 64, '2026-11-27T11:29:58.714088Z', '407020b958b98a3d', 'create', '\xd30f99e80792c4658c77bd247234d1e079174634c16286222ff82105f46a9974', '\x0166ca6db12f2dea5971c1858387e504ac7d6323039851234cf7b741bf22b33eaec2d33c104b2b553ba5e2ead43a07040bf498938ddeb2652dd967f69062f1627e', '9a776efcafd6b4f05cbd24ed5c174795', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 66, '2026-11-27T11:29:58.757912Z', '407020b958b98a3d', 'create', '\x9a0db4dc9b573fcef214500e31686c86e9b78a9cb04843d6ddf40475797c548d', '\x01aa70689ef5773c9b1813af87a0a1d6c39fe5cece051f38eb121c581cc0acc11ea3021be6294b23bbc35d9607a1ba5bfc9761fc40a5c96c57a959830cd242bff9', '47a8ba77a14a9533737cb6aca87897fb', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 68, '2026-11-27T11:29:58.78555Z', '407020b958b98a3d', 'replace', '\xfbb458612d77f89bdcf53db795b6185850e78817eb04fe25ba3aa3f0756367f7', '\x01f573c214b0894487fcab76af17803ba3e6e63ab85b38a4597bab9e5ec813a1b23fa318bd0898c34ee9bf7bea1b7d76114c40b1be3015fd489487ab6c043923f6', '3cd70bf28017b7db2c366f39d6a8af03', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 70, '2026-11-27T11:29:58.79163Z', '407020b958b98a3d', 'delete', '\xb034fd1214435b79aff9a2ab1589a82861f400ba498ffd3bd8a967742f99db32', '\x01770ed1326ae9b8c058154fd9c339024cbfb2401cd3d4f5580eac6a2b24ae37c58f5266d14454629b67fa4452a18a577bf63be8dd42fb6120d3ba14137a57661f', NULL, 204, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 72, '2026-11-27T11:29:58.79349Z', '407020b958b98a3d', 'purge', '\xb034fd1214435b79aff9a2ab1589a82861f400ba498ffd3bd8a967742f99db32', '\x018658b37b5f88abe1293840b8f18b02458a422b425b302981fd3d6d726658b1aded3160ababf691aaaeaddd539eaa3a8420104ee5ac998c5900da06efa3a4bf02', '14110a22a55e21c5a8c5539342647c20', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 74, '2026-11-27T11:29:58.924346Z', '407020b958b98a3d', 'hold', '\xa1ddbf7b94a4802c9de6f53c00ca9b8862e5ee92a71e7ebb76302bcdd34d1a02', '\x01b27b18e6d1479b385c8eaf5487c0ed00f6d9d4ab23cb348c096b5821657225e9752ee409df0bdfdaad3d2c13f37263d1cb01896f842e6408444e7567106a2a10', NULL, 204, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 76, '2026-11-27T11:29:58.929324Z', '407020b958b98a3d', 'delete', '\xa1ddbf7b94a4802c9de6f53c00ca9b8862e5ee92a71e7ebb76302bcdd34d1a02', '\x01d40b5a6af4cf922f38a1bbe89f5c1a4cc8ad1cdc29ad312b2fd924837a8c1bfd8d613d1a733671862f71f81fd391d8c2a607188e6723d1e3047a29c3bbe527d5', NULL, 204, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 78, '2026-11-27T11:29:58.934826Z', '407020b958b98a3d', 'hold', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x0187f3f91cbc32c8972be3210d287f40dd9b56fbf3a39c19b32875873798e781b1adc5419c89cf532ba00e1b09f8ebedd082b8f4aaf64212ba41de3f65585eb664', NULL, 204, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 80, '2026-11-27T11:29:58.9758Z', '407020b958b98a3d', 'replace', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x017954da8d2957fabbe413574a5b28445dc229dd7360200b565fb1587236fbc3bb97846b421ade287715889e52b3d8141de26c9e0ed6052a04cff02f31a98e517b', '1f0900c67a7055b570b78487f66637a2', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 82, '2026-11-27T11:30:01.082633Z', 'sweep', 'purge', '\xfbb458612d77f89bdcf53db795b6185850e78817eb04fe25ba3aa3f0756367f7', '\x012382ed259678fba376e6512e2f6269fb2629acb4d2753aa10e98f479a181cb09d3c0a808164a2fb1d43e2f0ce96fca1570dfdb568794b27b860087d9e3c21070', '3e3127b9358c761d0a1655e6987913e6', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 84, '2026-11-27T11:30:01.441304Z', '4989e39d1b4acf4e', 'legal_versions', '\xd69335915e1f41ab54377cb9648589d89dab572c619b29116f73df7e9e9b5e19', '\x01f25fa26daf1ad33fc4811a34159f49aeaddf6c0ab424bca300dc9ac6ec75abc4c7a310e7ab2440ec412aeff0e562e2fd193916f39cb45f617908ecbed1688a62', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 86, '2026-11-27T11:30:01.445651Z', '4989e39d1b4acf4e', 'legal_read', '\xd69335915e1f41ab54377cb9648589d89dab572c619b29116f73df7e9e9b5e19', '\x01975b8e312839b027f9b3aa628f77d5bf8d2ea38e41dccacbcf536b543a21929da4a42d96b1d2d3bbc3144a37e04750b40d337a6270f2bc7d4dde7fa07395c6ef', 'afc88b9fc3ccc88bdfed1d7965ed1078', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 88, '2026-11-27T11:30:01.454792Z', '4989e39d1b4acf4e', 'legal_read', '\xd69335915e1f41ab54377cb9648589d89dab572c619b29116f73df7e9e9b5e19', '\x0191938726d34b2d9e97cc3f82f28c186ea0a9660eba1b6e41d9148f9724f0f0a61a6c9a25f9999b7fa9f3f4a0991edafdd18587aea0dda731113c0e0c34057682', '37d3a6f7241c9b2901a4fcc8a95ba778', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 90, '2026-11-27T11:30:01.462332Z', '4989e39d1b4acf4e', 'legal_versions', '\x4255fc120a28a6cf7c8a1783b06bb1eccbc2a16ea9d940e132bd343009d6b207', '\x01011596e29b776101066bc1a756ed75c826d3b245972d5f3a213fadfa54be8b0398a97500d49b1deaf5d5c7af99a940b9dc4eda0197ebc2e032bc2988b067073c', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 92, '2026-11-27T11:30:01.472798Z', '4989e39d1b4acf4e', 'legal_read', '\x4255fc120a28a6cf7c8a1783b06bb1eccbc2a16ea9d940e132bd343009d6b207', '\x012b139377fd936699b0fc5a81659cd59c68acec36a3c1c8c478bb28273e37e6e59695fa84c635657cff3ce8911539c97cda3926c65a637d141179e200cc9b1442', '7badae7d037a408a06ee8ac03b286e8f', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 94, '2026-11-27T11:30:01.489405Z', '4989e39d1b4acf4e', 'legal_read', '\x4255fc120a28a6cf7c8a1783b06bb1eccbc2a16ea9d940e132bd343009d6b207', '\x018f6b36721ded650f2e98d9b449d9aaa525f758490745ae2be7ccb615819d4b0c515ca39d1219fba05b6f0ba5acadfa544941b29d8d1591336ac42904ba2da5d5', '2652e0493b3e6ad7b404599132deba71', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 96, '2026-11-27T11:30:01.498309Z', '4989e39d1b4acf4e', 'legal_versions', '\x83a3ebe9146d7a798b3b62c7415d9632b8c2eff7950904efc4227d3622fee0bd', '\x0133afab9984eaa607b9c97c8db5e3c585a4399b8740997c1a0435a68c8b566abc7561a6cd172f2d17ea2567b6b4e512aeb97ad2427c232715b9edac8cce5c5cb9', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 98, '2026-11-27T11:30:01.502702Z', '4989e39d1b4acf4e', 'legal_read', '\x83a3ebe9146d7a798b3b62c7415d9632b8c2eff7950904efc4227d3622fee0bd', '\x01dea7f87b08cd2987cbfe3451788019915b1fac72264fa7e0defe0f4d65c529abc8a02511aac34831667f95bd2f0c199c41e129b2ed27873dde586023eba49a62', '3bef0a3e9433961e0da8a3cda6ae7e57', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 100, '2026-11-27T11:30:01.510362Z', '4989e39d1b4acf4e', 'legal_versions', '\xda7f1ebf0ce7f23c1857bdb9a13eec5cbf64a9a9fbdfd111b3decbc2e548c3f4', '\x015bfa7db9f402df4619ed9103608aab95bae5aadb842438adde0a5ed8611364ee486e42e8da5ae04f58920f77c9e4877e03a98b48b70e6f548691fa85f338ce9a', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 102, '2026-11-27T11:30:01.515863Z', '4989e39d1b4acf4e', 'legal_read', '\xda7f1ebf0ce7f23c1857bdb9a13eec5cbf64a9a9fbdfd111b3decbc2e548c3f4', '\x013a538189119a0bf7bc6c4969975d3e00283a8b3939229015750b2071072d95ac03d7facdc52b0d164c4b7ee280973b576363bf51433f4ea752913d3969871c5d', '1fbcc3c80e4b09db703ecdd121010551', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 104, '2026-11-27T11:30:01.521949Z', '4989e39d1b4acf4e', 'legal_versions', '\xea010f4db1fab36fcb65d4e0d2eb3c1e895d6071d582bb550443b0d04a4e47c0', '\x0166f04066924567edb869c7587836ba3784fb03c0ca76ad5a9bfd3068dae547036838300e7889ad6bd1bcb63d5c3f8f25e8799842a7e50ecf11ecb74b13075ff0', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 106, '2026-11-27T11:30:01.527045Z', '4989e39d1b4acf4e', 'legal_read', '\xea010f4db1fab36fcb65d4e0d2eb3c1e895d6071d582bb550443b0d04a4e47c0', '\x014362c3b31f7e984a4ab522532203192f4736f6005688f7157ba7893dbe2155f5a5aac4072a408d4ce05e62bab3abcc00417ee771c7c406648e582f851b4e018a', '2f07a13789b2e7c3dd22ef8d052e2a34', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 108, '2026-11-27T11:30:01.533019Z', '4989e39d1b4acf4e', 'legal_versions', '\xbce39087f18910bea3faa272cacbfa964ffae45d3bdb7c0a0bc26f437a6683e9', '\x016689832b0a4582976d105af74e5a35cbe28ed031dd35921e035f33ddadd6fc6c8aa7a2bfbd7f15adb1764f76185c2975be2dbe73d34e714536875d8c4b6429ee', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 110, '2026-11-27T11:30:01.536904Z', '4989e39d1b4acf4e', 'legal_read', '\xbce39087f18910bea3faa272cacbfa964ffae45d3bdb7c0a0bc26f437a6683e9', '\x0198a6484c7eba3f664639fac9d01bc58152136d01d4b6ed995e298fbe722087b1ca111b0daa89a8312af3bd06471c133fb56dd9aeeecba856de1be2a078349577', 'be52d022b41d84d43e2db223937a8fea', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 112, '2026-11-27T11:30:01.543477Z', '4989e39d1b4acf4e', 'legal_versions', '\xa08467e5be1a169f0d33c776b3772c3911de5aac84b9ffb6b2da092e7abf1a2d', '\x017b601c3390d2f6ddd7d22990fd0c2055bf297222927de43b3b3aaae4587838feb4e3f680c9c9af5a94a1cca871e250751b6b489927d280f4903b5c190251cac2', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 114, '2026-11-27T11:30:01.550258Z', '4989e39d1b4acf4e', 'legal_read', '\xa08467e5be1a169f0d33c776b3772c3911de5aac84b9ffb6b2da092e7abf1a2d', '\x017f668457b1b34e3a0cfffb1753fcbf8e5b284b8cb799d824b6bf6bcdce52c64de517731c670f0db6566e01e5c408162fc220221eef0c2b5e6b17035b6e26e20a', 'fa9c498657ffa3680b4544df1b99fe4b', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 116, '2026-11-27T11:30:01.555544Z', '4989e39d1b4acf4e', 'legal_versions', '\xee7fda1b85e4a993271c51cb0de9b5c1818208ab5bc010e53e6e873790763059', '\x01c113e587754983712d1b8bc06d773e7539621a3c6d2fcf6966e4b21b6fda5c4f3a1870156c90aac5e184660993ba73d63ed3d0969df3460be4fadfa9842a5578', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 118, '2026-11-27T11:30:01.55917Z', '4989e39d1b4acf4e', 'legal_read', '\xee7fda1b85e4a993271c51cb0de9b5c1818208ab5bc010e53e6e873790763059', '\x01b5161cd5a151df07fbe87f1c3e5aa9369c5dc59dc50e59ba6dfde153ba952693bf49a3d53f8ac042f4c635bcd93d9d4e58247b3f4de4a3f161619aa01ef0b755', '08230e03fc049ed6f9aef30e186d63ce', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 120, '2026-11-27T11:30:01.564527Z', '4989e39d1b4acf4e', 'legal_versions', '\x402a77886f5de5e1c57d3025564ca0afd25f7b10e062a138374ea7e1e4db7fb0', '\x010a1abc99ef253c849655ac385577b85525636aa87da2e09cce14e0b89e3735d19acc1e7ad9a32f5026273beaf7e84ee3c40171ef82961bd1e2ff05f4aaa6014e', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 122, '2026-11-27T11:30:01.569756Z', '4989e39d1b4acf4e', 'legal_read', '\x402a77886f5de5e1c57d3025564ca0afd25f7b10e062a138374ea7e1e4db7fb0', '\x018563d0122999b1e0142dfdf3b506216365f5d324b65cb6ffca7cb4a3ba436e1b0b9f1bb2e6f886decc4de454fe988d470bffb65e27031e28a7d0ac313a6c7ef4', 'c439c3bdab4f1fc2bd53c6e1fe4e2778', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 124, '2026-11-27T11:30:01.57501Z', '4989e39d1b4acf4e', 'legal_versions', '\xeec14cf17b8b91cfe713e5c57d9b984e4283450dc78d1010140de43f18c91a1a', '\x0121f41a212ae641aa63af42ba277b8ff2c0993fcaf7ce61a4629e981aae2bbd480589f14765455525e15195bda495037b6068a81d12c12e4a5623875c022d9da1', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 126, '2026-11-27T11:30:01.578306Z', '4989e39d1b4acf4e', 'legal_read', '\xeec14cf17b8b91cfe713e5c57d9b984e4283450dc78d1010140de43f18c91a1a', '\x0171452b3b39867d9581c4268c59d6d443eb94fe697a0bf274dd7b1789bddaa02ae582a0288696679d2a91e051f3b0aa8a8cf50b22f64ed33027fe9c37030f94d4', '711a6ee8d63b28c568f6a8fe07b2da32', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 128, '2026-11-27T11:30:01.58325Z', '407020b958b98a3d', 'legal_versions', '\xfbb458612d77f89bdcf53db795b6185850e78817eb04fe25ba3aa3f0756367f7', '\x0161a8bc57c0b987feb1c41a363e5c81267e3efcde7f8a3fdc69bc11f317b7b085a0f9051bde6ceca5c7683ba33408b3b14e0f967fb7403660cb0554edccf660c8', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 130, '2026-11-27T11:30:01.586532Z', '407020b958b98a3d', 'legal_read', '\xfbb458612d77f89bdcf53db795b6185850e78817eb04fe25ba3aa3f0756367f7', '\x01ef59d445d1208607a784e470167c62455e0e85e508f19b007318610cb0ed3d7e70af54e55271baaea949edde2cff2eb707c2879b0d8f40236fac01dfea5de2dd', '3cd70bf28017b7db2c366f39d6a8af03', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 132, '2026-11-27T11:30:01.590973Z', '407020b958b98a3d', 'legal_versions', '\xb034fd1214435b79aff9a2ab1589a82861f400ba498ffd3bd8a967742f99db32', '\x0104d7cb4ef5f00accc30b137a2389d09b041bf961e8f1565be4bfcb181a7885dd031a32331ecffd45716308d915e8853917235527815575308ccfc98118f0f19f', NULL, 410, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 134, '2026-11-27T11:30:01.593821Z', '407020b958b98a3d', 'legal_versions', '\xa1ddbf7b94a4802c9de6f53c00ca9b8862e5ee92a71e7ebb76302bcdd34d1a02', '\x01e3106b38c66171fc101847dcc778e383ae09121356d78b38490be6ffcdcee004d5086f5d3146144a90ddddafeb0f6a216b2050e3fbaab6064beade47ec0d5b34', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 136, '2026-11-27T11:30:01.597019Z', '407020b958b98a3d', 'legal_read', '\xa1ddbf7b94a4802c9de6f53c00ca9b8862e5ee92a71e7ebb76302bcdd34d1a02', '\x01c7742496cc597b44134bea690374ae2b5455c9a1737f3ad2b9e522fbf05dd4bfb0675a0cef7af219b17514587111c4d37081acfedbe4bc70fa90ac41348bde8f', '20692fe6e50e91b26c289bc3cfa7330d', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 138, '2026-11-27T11:30:01.601419Z', '407020b958b98a3d', 'legal_versions', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x01412812750f0276440a76c4e56c6821cc40abe12125c8355936cfb2a78a1d3e338123a1c52ee94e0288d7fba94d24aba41c65b3bce62ea07b6bae4784d6549308', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 140, '2026-11-27T11:30:01.60459Z', '407020b958b98a3d', 'legal_read', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x01418c5ff874d8344ff711dda95c20d0a382dcd957928ac5feb416b81e5dca79c2bbc4d954167944a665e006c3c2f363832fa9133d0456bdb7ad5660c79dff3239', 'ba7795caf4a53e1f67b98152c5b5f054', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 142, '2026-11-27T11:30:01.608951Z', '407020b958b98a3d', 'legal_read', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x01fa692214a241c2cdb601439872c67a348088a1249aecdb1a0fa73fdf64c6f2a6e621dc02eec21caa2bccabb055f0b1b0ff9e51e1448784bc59d56a9fbdebc123', '1f0900c67a7055b570b78487f66637a2', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 144, '2026-11-27T11:30:01.61358Z', '407020b958b98a3d', 'legal_versions', '\xd6172d06225e3f89f5938d984a2d30c7257dd56aaad546efd2d6faf09e629606', '\x01ed2381c4104cfefd55bed4037dde23a909da09a23560d2d673841e535945d4630f5a7db31a5cfec12da2be4fdf14ba76edcac902206f80e6d24e253bc7c34041', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 146, '2026-11-27T11:30:01.616757Z', '407020b958b98a3d', 'legal_read', '\xd6172d06225e3f89f5938d984a2d30c7257dd56aaad546efd2d6faf09e629606', '\x01221d35c18814799b1ac9f822d36e9f7f3575748fec9ab2f651e6b697215ce9bbe6f37ccf5e375abdb83e40b4763dcb6f08f2bdc8c5072b14949e56a42abec890', '412284edb7b0601f8e5eb8fc18a77505', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 148, '2026-11-27T11:30:01.62118Z', '407020b958b98a3d', 'legal_versions', '\xe0fe94e711967c4c015f1d7b5f124e4836a364930c9632bc1b24954899aa8722', '\x0145ec4f1cf90d83cd4b4e5dbb5cda70b3482d4df9214b3c4fce60e3dd5da6c24d9764f8ab74c2d9c6f71f6fc7f9d2562f9e9f2474eae6fec5817fc23a30258b7a', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 150, '2026-11-27T11:30:01.624789Z', '407020b958b98a3d', 'legal_read', '\xe0fe94e711967c4c015f1d7b5f124e4836a364930c9632bc1b24954899aa8722', '\x01f4a5c39dc53a4c76811c09b2cf4e7ab8ed11379f4d4d7b1a25fe06670859236f6cccde88ad28b2392ab055f32d08c827ef56838f15ab5b7494a452bcc7f3b675', '6402755df100697c062d1d35e44dcada', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 152, '2026-11-27T11:30:01.6293Z', '407020b958b98a3d', 'legal_versions', '\xceb755e2a736e1aa8ca10c2d2a2007fe498f69d07fa129a0f06778c8b3aa9021', '\x015269d452f217aca47fc8d56160b993c8565f970398e68a0d9bea0d7aa2b30f68a53216b32e0b6c9d6b7626ed955c22f25258f62f638405e02acf3f465cf43f9a', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 154, '2026-11-27T11:30:01.632469Z', '407020b958b98a3d', 'legal_read', '\xceb755e2a736e1aa8ca10c2d2a2007fe498f69d07fa129a0f06778c8b3aa9021', '\x01d63eeb69f7ec89323e9d6928740f3f65f67afd6d712ee75e9a2b4038d1c25acb2cae368c87fd0b432bcf7ba7b6f1e417972d8f194deb3785070c445bfb1475b6', '0af2ac0bfd1b794e1ba4a4707f009284', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 156, '2026-11-27T11:30:01.636943Z', '407020b958b98a3d', 'legal_versions', '\x85083b2e6d98e99cedd8f222e2949435387d8ec6d8abbe3900bac42aa3f1fe3c', '\x014a1a10add2e2e540eff8d1db117431b19d7d7c172133a790a149d47f2c4b3b122f7250f113991f43f214df7c579817e73564684df68d07e98476c54b87b2f4a7', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 158, '2026-11-27T11:30:01.640159Z', '407020b958b98a3d', 'legal_read', '\x85083b2e6d98e99cedd8f222e2949435387d8ec6d8abbe3900bac42aa3f1fe3c', '\x014744038d4ceb40b5b515e21b0b107e722c3e95ccc5f12ba6a7e4c74633a5fbd3845b9a767319f72ff5ebc6a4fc8fc15f2cfaaaba53bde81f5559f6557fbfae17', '12928a085361d94ed0db80de98ed561f', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 160, '2026-11-27T11:30:01.64442Z', '407020b958b98a3d', 'legal_versions', '\xd30f99e80792c4658c77bd247234d1e079174634c16286222ff82105f46a9974', '\x0115fca32024aca0e1fa95b333c43fb6c5b35291062a7826fc5a72631b6b3be859943dadf54423066aa6a03eafb69dbc03b43aaa1bafab531d7fa98cde82c43a51', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 162, '2026-11-27T11:30:01.647668Z', '407020b958b98a3d', 'legal_read', '\xd30f99e80792c4658c77bd247234d1e079174634c16286222ff82105f46a9974', '\x015c1250d184a85501f1afd9f07fd3c82aeea4f139d7d9afee8d8f48ea757d1f3b39597169c7acda298e8008529008b3f8f3a9d739da412f5e0a88b51d2a76d5a4', '9a776efcafd6b4f05cbd24ed5c174795', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 164, '2026-11-27T11:30:01.651884Z', '407020b958b98a3d', 'legal_versions', '\x9a0db4dc9b573fcef214500e31686c86e9b78a9cb04843d6ddf40475797c548d', '\x01ab717f72e5be78c5235af96974f1c6079a7ac998904f1ec5627851bb1f4fbf9d3e725c4e6c713c7272a510bce72cca6b2d83dde5ee17d76ad63d0c39ddd0cde2', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 166, '2026-11-27T11:30:01.655237Z', '407020b958b98a3d', 'legal_read', '\x9a0db4dc9b573fcef214500e31686c86e9b78a9cb04843d6ddf40475797c548d', '\x018fe966e1448f093936ec66935fb83a594f2651c1bf8febf07cf44034d298d1b97ab01580b494fa36fe164599b887ea4b7c03b9a22801e39e101646c9b5334904', '47a8ba77a14a9533737cb6aca87897fb', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 168, '2026-11-27T11:30:06.602147Z', '4989e39d1b4acf4e', 'create', '\x6e92f654e85f5c6eec9890304822a981f4542932a18901ae5fc6bb7e612c1b24', '\x016f9033fafd5f69bb244c35b08c06610a454631497447a914ca9e011f531b5ed325e7af640a1d48ac79f6faf663f2ecf2b506e1a8883cf8fc1f711fd37a9f2785', 'db8e249a890f311c8d8b8ef6ec08f18b', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 170, '2026-11-27T11:30:06.648396Z', '4989e39d1b4acf4e', 'create', '\xa371c0d7b8e1c4d57d884d0d13dc210f08110e9584d956156496532f76218b98', '\x01cb5fa0f2aa708b402646670f53a464a5ed36ad1e7765e89ea3849286b1cd2c3048e86e50a644e2fb33305907541fc87bcb4a669e7acbc7776e4c2acedbc6cd9b', '22c8a8c8054a74ae4c3980682f4e9f6f', 201, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 172, '2026-11-27T11:30:06.676039Z', '407020b958b98a3d', 'create', '\x5fce8736b69c223f52fc6d34f803e08888069e24ceaa4db4852d071f1aab80f6', '\x019ff8949d3a107385891210046c5fb5a39df73cfe3a04b6df3ad352eb973af95b6b08c8a0127b96b7ae9770e79d27da7404ca5ad30ba2898b8fc28ea0ad9c7ea5', '98aeeebc2c1cd9886df765a2493818d9', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 174, '2026-11-27T11:30:06.696673Z', '407020b958b98a3d', 'create', '\xc40da1e2bd0867ca040f0d90af8603151bb6b1785ccc5259a9e0b87c3abe099f', '\x016349b1dcf3380490d3c73ab68184dfefb959772ca69fbafe584adb570a92ba37f26ab7fc2ee82cd49243cdb27dd35230f55e81b745bc0aeb96911aa391a001ce', 'b3cb099b5a7d6eed60582ff21f3a8a63', 201, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 176, '2026-11-27T11:30:06.701988Z', '4989e39d1b4acf4e', 'legal_versions', '\xd69335915e1f41ab54377cb9648589d89dab572c619b29116f73df7e9e9b5e19', '\x01cc4be3912e24ca43d598353462de14ebee736f073ac787fc52a41d6030037783cda144504755d06551e4775dbdf229a914b00e16a0e51d232781bd37aee12553', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 178, '2026-11-27T11:30:06.705816Z', '4989e39d1b4acf4e', 'legal_read', '\xd69335915e1f41ab54377cb9648589d89dab572c619b29116f73df7e9e9b5e19', '\x017e035d060bf444683d0ca5190273d0c921f42f90fa3e71cf29f2d0a6d5c79c1e88e4e668a9f2494da7dac729998483f98c7c98c787a9abbc2dbfa51d10ce6736', 'afc88b9fc3ccc88bdfed1d7965ed1078', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 180, '2026-11-27T11:30:06.712431Z', '4989e39d1b4acf4e', 'legal_read', '\xd69335915e1f41ab54377cb9648589d89dab572c619b29116f73df7e9e9b5e19', '\x01e5eb2b3f20b564d473c69f34af71db7753cecba905b6b3bef867411352532ef41aa5a3719ce2a42c88b8b0c1c99535e083b5cb913e02c2d425dc711cacdb91ec', '37d3a6f7241c9b2901a4fcc8a95ba778', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 182, '2026-11-27T11:30:06.717523Z', '4989e39d1b4acf4e', 'legal_versions', '\x4255fc120a28a6cf7c8a1783b06bb1eccbc2a16ea9d940e132bd343009d6b207', '\x017fd28a0ca23549f0569f1227f53f1673867b2278282894f3a8f048a9cbbff45ee1f9e3b053c7cc337b628a7135570e9e0d9ee9252ea0e215c34150b1f6b18f31', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 184, '2026-11-27T11:30:06.721263Z', '4989e39d1b4acf4e', 'legal_read', '\x4255fc120a28a6cf7c8a1783b06bb1eccbc2a16ea9d940e132bd343009d6b207', '\x0107e1adaa3d3edb8dca01beef37182901ce4b93604f99a2bf7c1464f0708020848d81a9515bba36dff5a23a1f1934f4529bfc31f709bb4de57c239719a9ea1758', '7badae7d037a408a06ee8ac03b286e8f', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 186, '2026-11-27T11:30:06.726319Z', '4989e39d1b4acf4e', 'legal_read', '\x4255fc120a28a6cf7c8a1783b06bb1eccbc2a16ea9d940e132bd343009d6b207', '\x0140a19c35056997da9d08987d26e1100ff0eecf174dba5e2b24690a48acc0a23c434d724d0301a6ea55ca353eecdd6c6806ffdb009f18261c4781152c70f2da32', '2652e0493b3e6ad7b404599132deba71', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 188, '2026-11-27T11:30:06.731268Z', '4989e39d1b4acf4e', 'legal_versions', '\x83a3ebe9146d7a798b3b62c7415d9632b8c2eff7950904efc4227d3622fee0bd', '\x012c854d77a756d5e1048549ba86a1be112a013f755261dd309a5cea36c3de4d098017e0225eb3d358c61b79c529f6a0957f8cda8ca5402e78ca678b2b2a6239d3', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 190, '2026-11-27T11:30:06.734875Z', '4989e39d1b4acf4e', 'legal_read', '\x83a3ebe9146d7a798b3b62c7415d9632b8c2eff7950904efc4227d3622fee0bd', '\x0139e9b4fa2db9ca692c7dea118ad77830f77d2197bc127e5e9a6a6363c221a6584c1df9efea2bc7436828512a889749400a5101ba7ff1899bdf5e639a808fcc45', '3bef0a3e9433961e0da8a3cda6ae7e57', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 192, '2026-11-27T11:30:06.740039Z', '4989e39d1b4acf4e', 'legal_versions', '\xda7f1ebf0ce7f23c1857bdb9a13eec5cbf64a9a9fbdfd111b3decbc2e548c3f4', '\x0173ab324c2d6c28b91ec6e3a8715fec22944100cf8e02f07bd09b924469a81917c435a88505b8934b84fecd864d2f9c7440fd9d8d4e3f8c8f210d2f88bc74bb57', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 194, '2026-11-27T11:30:06.743565Z', '4989e39d1b4acf4e', 'legal_read', '\xda7f1ebf0ce7f23c1857bdb9a13eec5cbf64a9a9fbdfd111b3decbc2e548c3f4', '\x018c1b2d9087930fc4072a7b567d309c71286fdd8b7f1c5ec031533418539230913b8b8c0ff02699871d1366e7582c3a7814af0dae69eb7c308c75429f11773fc7', '1fbcc3c80e4b09db703ecdd121010551', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 196, '2026-11-27T11:30:06.748958Z', '4989e39d1b4acf4e', 'legal_versions', '\xea010f4db1fab36fcb65d4e0d2eb3c1e895d6071d582bb550443b0d04a4e47c0', '\x014e7406a350b739f7e7f255b411f7af4d5ac918ab281a00d6fe06150b83588e5a53757e69217946a931d2dc9b487e5a08ce70c086a59d3eaf079cbd58bdb183a9', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 198, '2026-11-27T11:30:06.752537Z', '4989e39d1b4acf4e', 'legal_read', '\xea010f4db1fab36fcb65d4e0d2eb3c1e895d6071d582bb550443b0d04a4e47c0', '\x01ecbd0cdd6af4bba9302f81d15de0a5a1e8aec52a9fb2f85a4ae1677f386c36bb820111f1ca061eebd14f522a0dec7f493db46fb37ef215cb54c558966daa5b5a', '2f07a13789b2e7c3dd22ef8d052e2a34', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 200, '2026-11-27T11:30:06.757783Z', '4989e39d1b4acf4e', 'legal_versions', '\xbce39087f18910bea3faa272cacbfa964ffae45d3bdb7c0a0bc26f437a6683e9', '\x0143d79e984e2b99f03558102d5a8bbc3996413046177b8bc4a966c1685282104e405ea6f7e5a28182d1a0869e3240d5da808b1890ef62fe0979ecd5e59be76aed', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 202, '2026-11-27T11:30:06.761353Z', '4989e39d1b4acf4e', 'legal_read', '\xbce39087f18910bea3faa272cacbfa964ffae45d3bdb7c0a0bc26f437a6683e9', '\x01c99093074462d854d67a8e1b4430cb7f0af2a17ad01560d581fe029d21506b4a30183804096d11a752e2019c4b4f2f3cc9fc5e86bb93e2c4a9a78eec82408a4d', 'be52d022b41d84d43e2db223937a8fea', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 204, '2026-11-27T11:30:06.766783Z', '4989e39d1b4acf4e', 'legal_versions', '\xa08467e5be1a169f0d33c776b3772c3911de5aac84b9ffb6b2da092e7abf1a2d', '\x01c29cbedb5d953e89c37d83ff26b54b1ef62184d6bb0b2b2f23a3c57b240f453754eb9faf9b952050ed443a1382ea4236036d7c6001d632282d7e88a9d4e20e5a', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 206, '2026-11-27T11:30:06.771629Z', '4989e39d1b4acf4e', 'legal_read', '\xa08467e5be1a169f0d33c776b3772c3911de5aac84b9ffb6b2da092e7abf1a2d', '\x010406e82c18aebb5c4acf73d90f1c349f29970ea5844bd7a14293786ebc4bea3f0004b138960f17fcf028fb0139ee72025fb817e6a4207ed475671fa7937172a2', 'fa9c498657ffa3680b4544df1b99fe4b', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 208, '2026-11-27T11:30:06.776788Z', '4989e39d1b4acf4e', 'legal_versions', '\xee7fda1b85e4a993271c51cb0de9b5c1818208ab5bc010e53e6e873790763059', '\x010e6d53610e43b170ffd1832e546b3f5a0da95fe22cca98e77c5278bea044863b07631a74547bce992c9c3f906b079c02f9fac33908a51d791da49c904008bfdc', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 210, '2026-11-27T11:30:06.780236Z', '4989e39d1b4acf4e', 'legal_read', '\xee7fda1b85e4a993271c51cb0de9b5c1818208ab5bc010e53e6e873790763059', '\x010cb8b63adca7338814fbe6ccc54d9f0b29ac383d88445d48a2bad00049391ec3f1a590251b0488280aad4af6d698ca1b14cdbca81b53cb95ecf1557a79ac8e66', '08230e03fc049ed6f9aef30e186d63ce', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 212, '2026-11-27T11:30:06.78511Z', '4989e39d1b4acf4e', 'legal_versions', '\x402a77886f5de5e1c57d3025564ca0afd25f7b10e062a138374ea7e1e4db7fb0', '\x01b946b7d794ec0ab938fc8b83fd4675b62a558f610ccb8e6d5e191340cbad270d7bc3181a7bd1575ad8ee8d8e867e8f58aec6f83f8d2d384346864f056e89f890', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 214, '2026-11-27T11:30:06.791425Z', '4989e39d1b4acf4e', 'legal_read', '\x402a77886f5de5e1c57d3025564ca0afd25f7b10e062a138374ea7e1e4db7fb0', '\x01953f5668ba824ae8074a602c60b4f5a977cf16465a003f0f9a637fc6afdf97387f427a9b085b8036275599bd65129d3b64f380a811d8796d93282fee261c3ff0', 'c439c3bdab4f1fc2bd53c6e1fe4e2778', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 216, '2026-11-27T11:30:06.798973Z', '4989e39d1b4acf4e', 'legal_versions', '\xeec14cf17b8b91cfe713e5c57d9b984e4283450dc78d1010140de43f18c91a1a', '\x0161bece38fd6b241874bb0d96fdf0f3a9f304d56b0f3e0bddd682e34eefa06626ec7d115225c118952e767a8db0288a4b73dc445b516909e11b2b2517bd73550f', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 218, '2026-11-27T11:30:06.802941Z', '4989e39d1b4acf4e', 'legal_read', '\xeec14cf17b8b91cfe713e5c57d9b984e4283450dc78d1010140de43f18c91a1a', '\x01f7f9b1b7c1e7d735bb08f90ff3cd17bfdb919808cf42b0e3a100cb4bfeac938027b5886907e36df822b69312df32d47bf6782d4f05d60147c524bd6b2a889496', '711a6ee8d63b28c568f6a8fe07b2da32', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 220, '2026-11-27T11:30:06.809249Z', '4989e39d1b4acf4e', 'legal_versions', '\x6e92f654e85f5c6eec9890304822a981f4542932a18901ae5fc6bb7e612c1b24', '\x01b577b6b5183dd594d0faa1d81c764d7c68515bdadb73996566b930930b97bac88dc32d8f23abe25059ff2fbb089f6369418a5b012b377a002f339d795e1551b1', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 222, '2026-11-27T11:30:06.813909Z', '4989e39d1b4acf4e', 'legal_read', '\x6e92f654e85f5c6eec9890304822a981f4542932a18901ae5fc6bb7e612c1b24', '\x01f758ed6fa054a103dd829436e633cbae8d4adc2ceb8105bffb90449489766442eefbdaa497f442d9e02b72fa62d900594cfce74de18b169efcfc082c30353fdb', 'db8e249a890f311c8d8b8ef6ec08f18b', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 224, '2026-11-27T11:30:06.81917Z', '4989e39d1b4acf4e', 'legal_versions', '\xa371c0d7b8e1c4d57d884d0d13dc210f08110e9584d956156496532f76218b98', '\x01e0cfdb7e7666f54595ed6714fa969011c504ca89192e0d0c07eb67826c3df36e802a5cd38e7569c3e0362a942983374ae6bd41ac52edd2506f1f78fa9afae6ff', NULL, 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 226, '2026-11-27T11:30:06.822778Z', '4989e39d1b4acf4e', 'legal_read', '\xa371c0d7b8e1c4d57d884d0d13dc210f08110e9584d956156496532f76218b98', '\x019afb886b5e6bc228ebd0d3e9bdc0e58dc25f20630e74ac7546af960c828ecc78b4c854098dbb7bdfd3a1d703048d28448dd35d501270ccd7ddcf6b6dc388de1a', '22c8a8c8054a74ae4c3980682f4e9f6f', 200, 1, false, '\x2777fe956ad10480');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 228, '2026-11-27T11:30:06.828402Z', '407020b958b98a3d', 'legal_versions', '\xfbb458612d77f89bdcf53db795b6185850e78817eb04fe25ba3aa3f0756367f7', '\x01947cc4dbd51a5ea27ce77a62c9ddeedb9478c400a1e76e13bc783bcf32a193eca12530a1e322a932720cdae2e53f743ce47787b4a0a90ceec5ce9fba8783ac32', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 230, '2026-11-27T11:30:06.831908Z', '407020b958b98a3d', 'legal_read', '\xfbb458612d77f89bdcf53db795b6185850e78817eb04fe25ba3aa3f0756367f7', '\x017f59b4c6a168c6492a093558c0f2329fffa6dee85cba3dbcef4c507d69e692db83f07906e0d19c78b57d5ab4c572a6301cc686ddd1d34e4931c9e5977165280e', '3cd70bf28017b7db2c366f39d6a8af03', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 232, '2026-11-27T11:30:06.83841Z', '407020b958b98a3d', 'legal_versions', '\xb034fd1214435b79aff9a2ab1589a82861f400ba498ffd3bd8a967742f99db32', '\x01ab7e64d9f2cf218688be310844d09d7a5ef6b75c9589c4aa6355ecdeb0e7f7f22764ee6346d6b5e07598e33a5f00f50d055cb4c56d609e3735ddc31eac961252', NULL, 410, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 234, '2026-11-27T11:30:06.841801Z', '407020b958b98a3d', 'legal_versions', '\xa1ddbf7b94a4802c9de6f53c00ca9b8862e5ee92a71e7ebb76302bcdd34d1a02', '\x0132562f07fd6a20d2cce3579cd702f14e3cac9faaa56fb8b2a0014da9eb10eeae281246ba3eb0f9ab5faf64b80625525f234011eebe97f6406430c1a5b9e4f954', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 236, '2026-11-27T11:30:06.845326Z', '407020b958b98a3d', 'legal_read', '\xa1ddbf7b94a4802c9de6f53c00ca9b8862e5ee92a71e7ebb76302bcdd34d1a02', '\x01b7f66595dee77854c9485e7273b92056d5c331cc21a6775333064817638a8bcc9d54754cc7c1c601164cd262ba327d05ba1a38c9cd9e8201f0b0daf32efc4c7c', '20692fe6e50e91b26c289bc3cfa7330d', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 238, '2026-11-27T11:30:06.852382Z', '407020b958b98a3d', 'legal_versions', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x015b8a97f4dbdf5dfc6baba6a5f2484f2ddd25ffe7b285f00c4b5df2f79ba2c8bbf7e7718f99d5fe7eff285308b4e1001cffffc88b7419990369ce64858de0d6b1', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 240, '2026-11-27T11:30:06.856111Z', '407020b958b98a3d', 'legal_read', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x017253205d706a43465ed438c17aca9c2de7169ee999e73bbaf9dee554878f067e378332e8e8dc159f0f55db593179c919eb767cef4e09bc3f8ba0ed5f4412a07e', 'ba7795caf4a53e1f67b98152c5b5f054', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 242, '2026-11-27T11:30:06.86112Z', '407020b958b98a3d', 'legal_read', '\xe278cbc6cdcd89bc2762caadfc759d2c3ca2bacfc22e97b538aaf8808ecdad08', '\x01b8580fd5bb043b9b2c82807b20357ce73195ea9ef9a52c71bed5e969f7b9fe51785052790950514f8c40d061f582e487fba9fd7bd3226ffc6874f81d438ef163', '1f0900c67a7055b570b78487f66637a2', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 244, '2026-11-27T11:30:06.866005Z', '407020b958b98a3d', 'legal_versions', '\xd6172d06225e3f89f5938d984a2d30c7257dd56aaad546efd2d6faf09e629606', '\x01a97ba108a741c21470af1f0f8e8b015c2ca138e504b368cb3c4398054a3597d0ea4fb9030cdc5d4f1a78d6e7c23785f2cdb8351e3941baf5cf8b60b2ee3a0c43', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 246, '2026-11-27T11:30:06.869511Z', '407020b958b98a3d', 'legal_read', '\xd6172d06225e3f89f5938d984a2d30c7257dd56aaad546efd2d6faf09e629606', '\x011744ff885b6c20ea67a568c3be9c3c5a247dd22bd9bd326291b48c85db088fdfc8f040fa32d8d0981b98f99c6d3474e54e4862b1d6ed4e4b782aa271eaeb31fa', '412284edb7b0601f8e5eb8fc18a77505', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 248, '2026-11-27T11:30:06.875829Z', '407020b958b98a3d', 'legal_versions', '\xe0fe94e711967c4c015f1d7b5f124e4836a364930c9632bc1b24954899aa8722', '\x011defcaa842e3f944172aa7ae786dccad28cc45b1fa1704164d1ba440bab0267097d6ca16422765e9b655afa99c2cc00b498bf62bbbb6f846068b515d1d79bc23', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 250, '2026-11-27T11:30:06.879258Z', '407020b958b98a3d', 'legal_read', '\xe0fe94e711967c4c015f1d7b5f124e4836a364930c9632bc1b24954899aa8722', '\x018c8a181085f5fffc68f2513bab4a57ed40dcc371526508f5f1e67a69cfb20d48122ae4a1d58b51c5b70abed180fd3adfdd77cb67473d721c4f24987f226f20d2', '6402755df100697c062d1d35e44dcada', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 252, '2026-11-27T11:30:06.884148Z', '407020b958b98a3d', 'legal_versions', '\xceb755e2a736e1aa8ca10c2d2a2007fe498f69d07fa129a0f06778c8b3aa9021', '\x01c985ab6daed7dfa6dcdcff5cb13b360865531c62d6fbff1bddb39949ecfb84d871afdf3e55396979de66bbebe86ba9a36816d7bfc2a6d41adcad050442acc69a', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 254, '2026-11-27T11:30:06.887703Z', '407020b958b98a3d', 'legal_read', '\xceb755e2a736e1aa8ca10c2d2a2007fe498f69d07fa129a0f06778c8b3aa9021', '\x012ec18d04a413f4fff23e9d63c3025335ecfd5a9bb69fe9230df54ccb57f04999ba9d98aace977369a5920fff5981bdf7409421608e0dc22b43bb9a4e0fc50b22', '0af2ac0bfd1b794e1ba4a4707f009284', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 256, '2026-11-27T11:30:06.892859Z', '407020b958b98a3d', 'legal_versions', '\x85083b2e6d98e99cedd8f222e2949435387d8ec6d8abbe3900bac42aa3f1fe3c', '\x01182e09f4eaf2e87c0f537d84040c224d1cd95db5e2ce45494ca05f1de2d4cf106c2647eb32edef8cd43f9715716b138d4d512d44d269825b33fc2a6b6b7620db', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 258, '2026-11-27T11:30:06.896928Z', '407020b958b98a3d', 'legal_read', '\x85083b2e6d98e99cedd8f222e2949435387d8ec6d8abbe3900bac42aa3f1fe3c', '\x01261c4dd6bd2d33338a291e1e1b99a0a5d79a9a2667efcb926943679464f21fea1f4d2030610b6a5f4ec133b3e8d7571a0ad928e36c8305b5bf64d2c40c404f06', '12928a085361d94ed0db80de98ed561f', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 260, '2026-11-27T11:30:06.90369Z', '407020b958b98a3d', 'legal_versions', '\xd30f99e80792c4658c77bd247234d1e079174634c16286222ff82105f46a9974', '\x01530d3b0b17c9f47618f1d81420c333212127924faa6514ca461c48affe98d63c2a1be48a63b17055a7b06d1554724125cb14fef8f9caede2e26051737ee621da', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 262, '2026-11-27T11:30:06.907416Z', '407020b958b98a3d', 'legal_read', '\xd30f99e80792c4658c77bd247234d1e079174634c16286222ff82105f46a9974', '\x0103fda5844c2c0bd9ccbc629b168fde6c13ca0f49b604771dbcdedfb8fbf6437b93d976ee0474aaf8ac407fa6f9fc2e26ea8ce1c8f842133429d437ed28349903', '9a776efcafd6b4f05cbd24ed5c174795', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 264, '2026-11-27T11:30:06.912824Z', '407020b958b98a3d', 'legal_versions', '\x9a0db4dc9b573fcef214500e31686c86e9b78a9cb04843d6ddf40475797c548d', '\x0133c1882271a8f4cba7da83ab16b8c23d3c4b28ecf6fed1de509cc72cd564b23a2193f3419f3786999cdee617dad2823179ca941b4617655d3b788e9c1fc1379a', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 266, '2026-11-27T11:30:06.916405Z', '407020b958b98a3d', 'legal_read', '\x9a0db4dc9b573fcef214500e31686c86e9b78a9cb04843d6ddf40475797c548d', '\x01f938ac8165e3c70de5065b6a5e54c14d4078b7e32094f084eddedcab54423f24df41ada357d838e858cdf524dd73f006b6e5a119ca1f37b7a02f2b106f178643', '47a8ba77a14a9533737cb6aca87897fb', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 268, '2026-11-27T11:30:06.921222Z', '407020b958b98a3d', 'legal_versions', '\x5fce8736b69c223f52fc6d34f803e08888069e24ceaa4db4852d071f1aab80f6', '\x0157034b8603f364f220ed86fa92f130c37ae0e74e152663499012e9a8cc4c97f4d34033cb4a090cbced9cfb1960eeed1f6e5b49e80d2616541ddae47cd0186965', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 270, '2026-11-27T11:30:06.924669Z', '407020b958b98a3d', 'legal_read', '\x5fce8736b69c223f52fc6d34f803e08888069e24ceaa4db4852d071f1aab80f6', '\x0126d02ecb90e6fe24eb6039219f3b08890ae4e7a0b12ee53046c70c9dc9e3074dd2c2b7b536911f356283b1460786ecead6b6a305896450176e1f74d472994361', '98aeeebc2c1cd9886df765a2493818d9', 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 272, '2026-11-27T11:30:06.929565Z', '407020b958b98a3d', 'legal_versions', '\xc40da1e2bd0867ca040f0d90af8603151bb6b1785ccc5259a9e0b87c3abe099f', '\x01b88b3abdda85bd4ab7cf34140fa87bb807fa78022d5709b9a46a090b3c5165653b446e3aaf7e6cf54750a8587761f8779114dce538aac69d175ea37edbd2bc54', NULL, 200, 1, false, '\x886185ffb78459b5');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 274, '2026-11-27T11:30:06.933188Z', '407020b958b98a3d', 'legal_read', '\xc40da1e2bd0867ca040f0d90af8603151bb6b1785ccc5259a9e0b87c3abe099f', '\x01334578167021c7b475292c1e076b7f09d52cbb71c31ddbf36f2ddfa8770f6b8c4556be7dd8b1c589f3f1536a2073af6896bd9a1f2d93056f3924cd0b52a885f0', 'b3cb099b5a7d6eed60582ff21f3a8a63', 200, 1, false, '\x886185ffb78459b5');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x2777fe956ad10480', 1, '\x01978922f1438c622c04b87ce293a32dc07892768de43b999833b65b13b812e74180cc0995a4bcc1bb81efbf68548523d96fe3292ced05772dc4ad000c');
INSERT INTO vcon.audit_key VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x886185ffb78459b5', 1, '\x0172eb1e145d32c0bc0deb7098ddc8b8348edd9e914e3ca441ff572a2570217f9566819c1c0bbafb67ff55ee4ee70bf3dd9f61461fb5025ec8c25e9528');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x7359d9574d6a2669', 14);
INSERT INTO vcon.key_use VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x4cf5038d2231277a', 14);


--
-- Data for Name: lease; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', 2, 1795779005001831000);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', 4, 1795779006237240000);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x92521874e9158efe', 2, 1795778976294367000);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x15f6a604d18b4534a6f777068e0a3283', '\xb58b38b1a2f0b84bc845c84ad29ba658', '\x0183d96b84cf2486b8517433288d91ff13e3313069c626980fc95ddb72826e12f4c266', 1795778993705072414);
INSERT INTO vcon.membership VALUES ('\xaf47274520135122031489ef1408884c', '\x93f5d0d372e354ccdfc220cee1b49b87', '\x01a6311c8de4d418d6683805921d695437a1da3c06e9c744d9dc5291c0611cc819b5df19', 1795778995589589804);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xb695c2702822cbc959913fb29b4601aa', '2026-11-27', 12, 12, 7401, 1795779004166794379, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xaefc92ab523c6aae2fbd240c34c0a8ca', '2026-11-27', 10, 10, 6237, 1795779005043856570, NULL);


--
-- Data for Name: pending_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: pending_tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: pending_write; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x791d6a145d0d3ae1', NULL, NULL);


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-27 11:29:34.351822+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-27 11:29:34.387411+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-27 11:29:34.393467+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-27 11:29:34.394173+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-27 11:29:34.396276+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-27 11:29:34.399468+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-27 11:29:34.400054+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-27 11:29:34.404526+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-27 11:29:34.408591+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-27 11:29:34.409144+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-27 11:29:34.410259+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-27 11:29:34.413811+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-27 11:29:34.419086+00');
INSERT INTO vcon.schema_version VALUES (14, '2026-11-27 11:29:34.42144+00');
INSERT INTO vcon.schema_version VALUES (15, '2026-11-27 11:29:34.423764+00');
INSERT INTO vcon.schema_version VALUES (16, '2026-11-27 11:29:34.428688+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, 'bceff509c89cc517bfff22233bab76bc');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x7bdf6ad8e3084eef4c7edbd2caefee3dbe52b9177c32c9c171dc7c9d5835af55', '\x017c6090bd214bdc794dc53965bcbf97a622e94550784c5ad3a9372667485594450c8da0aa', NULL, NULL, 1795778976305466000, '\x7359d9574d6a2669', '\x01eaf01c160c065095a0fce9341630c66a9331369470befebb5a8afceeb1f75ac1866967b647ae0b16a564cf9bc06197011ead40b5eb89540c3ecdd29d', NULL, 1795778976305466000, NULL, NULL, NULL, NULL, NULL);
INSERT INTO vcon.tenant VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xdd5c76a7c59c725456c988d91b80ad12ce1ec2009db8ce74cef45d7d1bc84d69', '\x014501fb9c13576bc2721a914ab1b4ca48855775bf83a9d574e2cddf75e391104f4ccbdae4', '\x01ba59f6f6f543ca0998b535e3bce57be5cb391cf7815da696e163983f988d', NULL, 1795778981686071000, '\x4cf5038d2231277a', '\x01192ddccddbd4a2fee79eaa69a196ba6d3f7e2c83fa405edf484c40a8b79a9b0c4659c9753904702686da0d5927a29ce6eb108a1c90699ff54ff1f91e', NULL, 1795778981686071000, NULL, NULL, NULL, NULL, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xa6c89e9df8637c20');
INSERT INTO vcon.tenant_catalog VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xf9436f1fc25f52a6');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', false);
INSERT INTO vcon.tenant_lock VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', false);


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x964e96ade6c07d9dec4459e666d70ef32ea4f56d7d5da35b8b4e6d05de283219', '\x0172d5d09b95267b9addf749b378a6bdb6d883bc9e24844c62f68bbd707435f0cad367f365c1fdbe6177468e7f', 1795778998793490000);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x15f6a604d18b4534a6f777068e0a3283', '\x01b751c23c255165952b4c111d8b6be56a4dc3b000ea2410bff3cafc8135de9819fb0c0be13afa8a23d9c5', NULL, NULL, 1795778990053508688, NULL);
INSERT INTO vcon.user_account VALUES ('\xaf47274520135122031489ef1408884c', '\x0125780ea4af8c2479665fb1581bc20a94e627ebb4607d6b12954f274d0e24021cab26dc4c3af0f5a8ab6cc2', '\xf4ffce6d5377346104498b0674c684d8f896e32dfd250edfab22fb657e7f20a1', '\x01dfe7aa40da28b5f2585e0e0f54bbb1c1a19f6ac6453fda9128d9279c76236bf7d4dcf8a1cd2edeba6283ba470df3fd85b0fd22c2bcc4a4938549b1a240d609', 1795778991889326842, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xb82586838cf9e23b2d0db087eee9735be48bc88234376cd990332773e0a4ad90', '\x015de196ee9908bd400cf82ec9bb91f33f56edba65f8c564c2c686b3c2299fd0c4cea903d9c34aedc3b9a88afa', '\xfa9c498657ffa3680b4544df1b99fe4b', NULL, 0, 1795778998162042000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x4342ce5fa8e4131830cf0ed96ddb2e79b0860c0b598cbf8faab86201983036c2', '\x01c0b35edc02303649d6e80d92e2c342ce964bf8b67b264d402783cc95b896e4d175c2199755bf5b3ccb77784d', '\x08230e03fc049ed6f9aef30e186d63ce', NULL, 0, 1795778998200215000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xd76aebedaa467cd9e4fbe29f68b08b1e1e47aeb799cb067200ea86a55f993d04', '\x017c22accdf2ae23c3abe80e0bf2ed6e2a0ee4f41a78a0c2af8cbca11ca13cf803c8c4bc79664d2a5d902624df', '\xc439c3bdab4f1fc2bd53c6e1fe4e2778', NULL, 0, 1795778998223726000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xc9ea4f44113efa4566b4940440dc1fcce654e2cc61ca05a8e27ed617a1e7ed85', '\x01c6137c5903669454358a870e1cf73e6a1d6985e7f494f7f344b98d6d2f1894f78844f2d1e16abe5a99536001', '\x711a6ee8d63b28c568f6a8fe07b2da32', NULL, 0, 1795778998243903000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x3b95685faeb3f15a05c8bba1a153c87ae1b8f71ba8f03b208276b8799c8bc4e0', '\x01061fa54b266f0f32b519de027e13799e188bd8c960e0c475726b1f6a9c0de33a82eb567dbdedc8239bcb5856', '\x37d3a6f7241c9b2901a4fcc8a95ba778', NULL, 0, 1795778997954816000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x756fa0d81fc984345d270bea551d64948e5232d0568efe21eafb08588370ca98', '\x01a2045faa7cd7c1dd0d66fbc20840985f96f46aa8f0de8fc6a5cc8b5bda27a457a5a70cb9b4b79647227095fc', '\x2652e0493b3e6ad7b404599132deba71', NULL, 0, 1795778997993191000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x2d7e29c8300a756024f497fc6776eb87d5046cc3e9066cb4ac6acdcd92df74b5', '\x018473840d7484050d1b70d7798d06e8c3bf44947fe79adb8079f717c140fe75e5353abb2dd3bb8738ea4a2387', NULL, 1795778998293117079, 0, 1795778998019354000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x2f354198a9e8be6c5ab529fb6a8e54fc95fe514f0abe9fa0dbf4aba9e7e09e17', '\x018fc87d69979fb0036fa6036cce38057e8be48e55b2b805fc8fc09e934ea99db59cebf22a46e6a582c8bfdfe2', NULL, 1795778998299276004, 0, 1795778998044427000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x230d1ce284dde928f74daa11f58dca09691a10c55ad8a97a2d5fc628562564c7', '\x017522454e5b460b9541b66840e1e32955f2187943510e2c6f69bedd98615213291aa17c9373a2ff870efcf738', NULL, 1795778998308639277, 1, 1795778998081428000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xdf7e8d391a4da2931b37e7d6c28c6534eaa9cd9554a02e2033f49c59699ee01a', '\x01a519a61db6fef8dff00d61f7ff284b4b083bf4ec7696dc8a9f566131198f7814c1c270aca2931b1339826f8d', '\xbe52d022b41d84d43e2db223937a8fea', NULL, 1, 1795778998124972000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x86e86e7f6c440b1bd4acd949ae39f2702894db329895d3eeae910f8ec9b4d064', '\x0126575c755c400661b780d9832cde9f955d13f33d8ff4558b18a1d6cb93e616c6dc01953b0833124efda4196b', '\x412284edb7b0601f8e5eb8fc18a77505', NULL, 0, 1795778998536752000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xe8751b6f5390b2154bd3ac595ff45f508c899fa8d8b70aef141a47aa1bc56d71', '\x01da6d240da350f0749562ceee40637a4a24eb70abc0f3f1bc9dca5699f73cb87f73dafbe519af176662c4fc99', '\x6402755df100697c062d1d35e44dcada', NULL, 0, 1795778998580329000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xc3c36476d17c239c436530434e2a06fc584bb30fb122ca8e14e6ce5570632a4b', '\x01818f9a97bfe4bfabd64c7483a2dba166156a21508b990310b3355c6c74622aabe948bd722461d526a5a11e03', '\x0af2ac0bfd1b794e1ba4a4707f009284', NULL, 0, 1795778998624240000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xc2a8d3d07c2535cc5765d6ee2b3fc42d69567024f53fe62a70d6838a3e157d2c', '\x0131d3567ed1e25e299a06ff0a1899b0aceb9c6c797b16bb19081961491ac10e07dcdaa148d06f72b2993b4745', '\x12928a085361d94ed0db80de98ed561f', NULL, 0, 1795778998668081000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x59cf61917e04dd24279ee1ab6230dca3bb7cfd6164f9f009899bfce38938e07c', '\x0105a24cdf5a4d64c84b094488c1b691ed2a525d57760ad64cb99a075d1d8444f0f75b532d10e4e421903a5763', '\x9a776efcafd6b4f05cbd24ed5c174795', NULL, 0, 1795778998714088000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xf1fa5d0eac650b978160ff667a5c21f67aa4c5034438030b9692012fc5374274', '\x011a989eff2fa0e0cd60ac499e87760465c550442e3ebdd80aaa8d9786ca149a9a3672ac66e109f09705b894d5', '\x47a8ba77a14a9533737cb6aca87897fb', NULL, 0, 1795778998757912000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x96f89880c66c4b3380d4e6dd3685c8327c7b297308c0e4ed3f59f14bb1b1eca9', '\x01f28626630ca51eca9f89bc0c1c1b71a8576c622563a06b6e99ac6badb9470f3cc0012fdd0f872ceddee43568', '\x3cd70bf28017b7db2c366f39d6a8af03', NULL, 0, 1795778998355723000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xcc66f70dc50678bc20493f9dc9602003ac20b2cf361c309f9f4c9dea1410ddae', '\x01d89dfd1f8ca89379e5224aef31b6089ea8d490a765b0a73b05fd7333085a0398b2173d4c0e66bc7cf113e93b', NULL, 1795778998929662019, 1, 1795778998443739000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xcdd9fa90843de7d3429f3ac900554a2046e454a360d94e8c5f8176e29bc8d145', '\x0155e624d325e6ad1d725a3ba54d06140bf76058f1c8acd3e56c56ae3ee95d92094596814ffd6a2f0d9f87a8b3', '\x1f0900c67a7055b570b78487f66637a2', NULL, 1, 1795778998492529000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x0d15127aea0f3f936f7cad4f5148113dcc3d95bad7bbbf6f65da422b175dbb81', '\x0110f59763e4e0a935189a5d85940a585aa0ce334f1456e1ddadc49189e14f926e34d9be00156a93594c2f3368', '\xdb8e249a890f311c8d8b8ef6ec08f18b', NULL, 0, 1795779006602147000);
INSERT INTO vcon.vcon VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x839c0617b9a3b00de9da075e232b341d33e62ad015b71c97be03bc4b4815bbee', '\x016283a7819c31f71daa4be164dc88c16e455201da7e71642434e60f97c5195a32b71ea61ed539327db8724138', '\x22c8a8c8054a74ae4c3980682f4e9f6f', NULL, 0, 1795779006648396000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x469f1443f99aab4f7b941c20424d094f9791a2b1e2619ada123e712ed183f3f9', '\x015ddb6ac98f2b6aaaf1a36f2968286c035e0e1447abc938419c2b7a14f246491a22833e80547ff1dccd7d7e63', '\x98aeeebc2c1cd9886df765a2493818d9', NULL, 0, 1795779006676039000);
INSERT INTO vcon.vcon VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x2f33cb71dd2d2184d468c771a0ac40130c2de8b92a8b3e0be33c5d01ec1f4ba1', '\x01d98bd0920b7100b0d33868c327f22e894cdf7bf056b9e620f2cac326239ae7aab89cf7b0b85279052013a16f', '\xb3cb099b5a7d6eed60582ff21f3a8a63', NULL, 0, 1795779006696673000);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x20692fe6e50e91b26c289bc3cfa7330d', '\xcc66f70dc50678bc20493f9dc9602003ac20b2cf361c309f9f4c9dea1410ddae', '\x019551f3f0560376a362ff689b4acd666b4455aaf09cfbf97b3a6f187555eb127a0717ef33', '\x01662297e0510f76d7b5571773d3e239fd7af2f70991b62d9d1969dc9992', '\x019170b593da6d9ae5c04fa5bd5453ec5358e5ff5ef397c956b8b96d273cc422a4880471f2', 1795778998443739000, 1795778998929662019, '\x4cf5038d2231277a', '\x012a169f0c757daa8fe07bcef6cb345584f4f558caf90c801695b4331f28a5d32b1105a45cfbb09a39536af1f12ca0ee0d6844501a3bf7baea92c8aca9', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xba7795caf4a53e1f67b98152c5b5f054', '\xcdd9fa90843de7d3429f3ac900554a2046e454a360d94e8c5f8176e29bc8d145', '\x01229d5781d00c89db00c37a28b4ec9f5f7c4809c01659dd86c04446e6685ed5243aea84ef', '\x013f90a35b1b42e05641b5cbff453ef3558971329c0f22dedb1f3eebe220', '\x01f5e3824c4caffdf407f490cd5ea81032d60cadf4b205740a7536ba0213218bb6388c3cd8', 1795778998492529000, 1795778998975800000, '\x4cf5038d2231277a', '\x01f2ad61e6943403d39b78b38a2aace8fb5e49354b607fa2bafb301ad34f2fa5f8d0ec92b5978870d2f232e446076c3fd9a5e24fe3f54f7ce1b9e82854', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x412284edb7b0601f8e5eb8fc18a77505', '\x86e86e7f6c440b1bd4acd949ae39f2702894db329895d3eeae910f8ec9b4d064', '\x0181b22cbf36b83e089f916cf8d0772ab90a15b02378909c81efb9981744b6d94b5d96453a', '\x0107d65f70716765308205ab114ffa952ac3fd749c045f065a120bdc9d67', '\x0191f95ea5d098dba1ab2ed7e31661b4e4631eadacffd3b2922f236561e89126863bd3a1e5', 1795778998536752000, NULL, '\x4cf5038d2231277a', '\x01bc26c91c3520859ea2b4bcc95c1208d07c3b83dafb7788d2fda9835dc0eaacda3c71a0415703489ee0d064a50a27b94d24614cbe1e7ac06a1d7a1fff', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x6402755df100697c062d1d35e44dcada', '\xe8751b6f5390b2154bd3ac595ff45f508c899fa8d8b70aef141a47aa1bc56d71', '\x01f41eb1c915405af21452d7f06db32049a1e7dc32e06415018a54d997fe65c7d55051a73b', '\x01445d87fa1ad5730f696a6d27dd6e3fc49045b8a7138122729b353c8f10', '\x011a6c0b50b02a14ed09f2010a29dc727694e86fa274815f0f2208eac555bfda266a6ad32d', 1795778998580329000, NULL, '\x4cf5038d2231277a', '\x0152697af73302c1998b0bf9e83eaa1458269278e4ec88ebe31b2a90fa5af5a324c0ca70c0ba1c17bd1c0403e0d8bdd4f5c52e8a1a3bd3d6695fc3bb4b', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x0af2ac0bfd1b794e1ba4a4707f009284', '\xc3c36476d17c239c436530434e2a06fc584bb30fb122ca8e14e6ce5570632a4b', '\x0105105df9fb3f4d78253af678038747f044826757e89b9546f391531c2666e7fefb9e55cf', '\x011020eba4585c8109d0c846282ff4b37ce7ce3e5da5b91f29a77dc5de2c', '\x01f767ba90f63e0567d96495778bdaeda191f5f74ed272b28cb86d6d36cb89e4c9ab7bd10a', 1795778998624240000, NULL, '\x4cf5038d2231277a', '\x01f54e30b90e5749fe889457b5807a081c1053467fa240ac28a3209b53f8497ae34757edf97ef9fa57ffa0e298449210e6b619c2fe98f91765de4d24fe', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x7badae7d037a408a06ee8ac03b286e8f', '\x756fa0d81fc984345d270bea551d64948e5232d0568efe21eafb08588370ca98', '\x018698ef806c4be774536f524fd207f52b367ff0393c0588410cb9e3bf78fe9f320993fd66', '\x01626dec4a766c8ce59d4f391cd10a3ed2de15dff880f000e934bda716d2', '\x0172303fbee6d67826376daf1af8b07aeb65ca1324c5eacb70c9eae2dfeaa5f5d895790749', 1795778997993191000, 1795778998286666000, '\x7359d9574d6a2669', '\x0169422588c7ce439f36c92d7f1aff5bb31619f342bef7c71b9f677f90102f8e12d4eb224c23a20f021ebfda3162d1108ca01a5010153ec2a66d996ecf', '\xb695c2702822cbc959913fb29b4601aa', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xafc88b9fc3ccc88bdfed1d7965ed1078', '\x3b95685faeb3f15a05c8bba1a153c87ae1b8f71ba8f03b208276b8799c8bc4e0', '\x01978d35d27b6abbf2583e7ff71bc3604315e4b4535d57257eacaa05cf459f1655036b3c11', '\x015673f1fc076cd55099eb3785af7a8d5e0e6e4202ceb7f0a8ffea9e39d6', '\x01b3e6cc916de35baa6de405ef8d3680b71e605a6a98489594cf2048b64b70429df15dfc8a', 1795778997954816000, 1795778998264956000, '\x7359d9574d6a2669', '\x0102d1552e51a5c3e156de9e993fa9abb883bad0e9782bf1ec3457065b478ee683cc66c81198b567c0886f53ab353cda70c773710aebd398c367385c13', '\xb695c2702822cbc959913fb29b4601aa', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x3bef0a3e9433961e0da8a3cda6ae7e57', '\x2d7e29c8300a756024f497fc6776eb87d5046cc3e9066cb4ac6acdcd92df74b5', '\x01291c9bdb692a1cb94fd7e3c41e3587a46a8d93ebe1e05f49eb01bb3b2a1d9ef398f4374e', '\x01158b9a0a90c93eb10e136fe226b2afb6e09afb99a5fa76cb8253b79182', '\x0107562c505b2914d1bfd747249ea56dc731db352d415e9cfbecd1643dbce4b0f5476a1fd0', 1795778998019354000, 1795778998293117079, '\x7359d9574d6a2669', '\x018b0f78e2f3afa9b719c8d7d93ec2e0bca9c268bdc5801d2f61a2c7433b4c4d1befd0c2f9477972621b0ba71cbfb51ad2ecbd18970f9ba45940c19075', '\xb695c2702822cbc959913fb29b4601aa', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x1fbcc3c80e4b09db703ecdd121010551', '\x2f354198a9e8be6c5ab529fb6a8e54fc95fe514f0abe9fa0dbf4aba9e7e09e17', '\x014bfc1f0b1303f141d5c8810ff6c5da796696aee5215a0cf9767fa82b138236b6e678484a', '\x0121e2a96d67fe69f92f70f7167950c4d00e8f648ce70434103412a62192', '\x01d58b551470600019f6e212b6dfa5a8c29cb73765af82a1ff74f792772123809024f366a5', 1795778998044427000, 1795778998299276004, '\x7359d9574d6a2669', '\x014fdd7ef7fd5863387c00af70aa3be0c8bd531cc6e0a5a4aa58bf3dc1d7a94aaf72a9289efb7da2f843e9ef8c5a98a620229112a761cd8ee0b0a6b637', '\xb695c2702822cbc959913fb29b4601aa', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x12928a085361d94ed0db80de98ed561f', '\xc2a8d3d07c2535cc5765d6ee2b3fc42d69567024f53fe62a70d6838a3e157d2c', '\x015c0bda7b28759d249f32e39b858a41c6f8412e601cdb53c4ac69f3c7b5ee06375cc99095', '\x0129e644ba554f279258a27480b0002305577dd6ca9ae94a3d8c13672ce4', '\x011dc02a24c7f579116cec3fee3e031fe443fbc9c64c36ab31e015c5963a0494525009cf3e', 1795778998668081000, NULL, '\x4cf5038d2231277a', '\x0182bea8ce3b4bf7320a98a73649448c0cd8c3b5f59a8ddd7a28c1c58e814a211164cab68fe10e7b59408b5270fb51fa5b0512020a185e05956734621d', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x9a776efcafd6b4f05cbd24ed5c174795', '\x59cf61917e04dd24279ee1ab6230dca3bb7cfd6164f9f009899bfce38938e07c', '\x0165ce0c3b151d74ad2c39929775b87d137a97fcafcf241a94aefce74e8f8639f9b105e912', '\x010f36a5ddfec4e48d746807e8a231d978a778bcfed4d3f91fed50cd2322', '\x01ae6d277bc98aa8272664b3c214333861e815032777f382ce9743744a574010d8dde39a08', 1795778998714088000, NULL, '\x4cf5038d2231277a', '\x0173eb661a6e3927633df64bac094eccdad3722098df6d171ae6d6f1eb405575a940c208b62baea9c9e9e607ac86e738f7e18be5984306cd80d0698ebf', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x47a8ba77a14a9533737cb6aca87897fb', '\xf1fa5d0eac650b978160ff667a5c21f67aa4c5034438030b9692012fc5374274', '\x01d1049503c43f81f5b5e8232e42a85f99ec25f06f763c161d2150d4d807ae554a986fe1ea', '\x01c4852f061295e4d6b08a33ecc8bc16ab7dc5fd8ff2a7c9c5760061ba68', '\x015efbb0fd7950f12ec7608427fe86ee2f2104c84a89f366ebc55ea255e78686667e5a0ec2', 1795778998757912000, NULL, '\x4cf5038d2231277a', '\x01cbe074c04fb8dbd493a88b7dab1562928d61ae3a52ad48a36724d32aef1e8ccfb04f3fd04b90823fd0f185a98a12ed7829a029b7a0b2f44902671fd6', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x3cd70bf28017b7db2c366f39d6a8af03', '\x96f89880c66c4b3380d4e6dd3685c8327c7b297308c0e4ed3f59f14bb1b1eca9', '\x0110575718cded6d13d64e15951f4c461ef13a1a592496913a68ac7fa55dbc1be695791b29', '\x01518aefcf5350162cdd332721f367f67909587cc2e47f656b992f6d9772', '\x01aa6af9fdddc2b497d812af5d15fa648eb7a4f08a656fe4f2316243c2a0a48137f1f65e93', 1795778998785550000, NULL, '\x4cf5038d2231277a', '\x01260c475130047648facdecc44828a53512e367c1a07019012e5d66f0b73df95fe3be6f74bf9fb128e7bd4661529b14db9ab1d7c7716d4b1154ccacfb', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x1f0900c67a7055b570b78487f66637a2', '\xcdd9fa90843de7d3429f3ac900554a2046e454a360d94e8c5f8176e29bc8d145', '\x01ad45cb2bc0735a61fdf4c2fdbc0763422ff719066a4eaf24791b45e956f1a3f3cd2d6bbb', '\x014e3329888472f4074d61c259014478f05a8abbc280019793fd77914c0d', '\x01fddfa1866710e90652d8d08b82e37502b1f4b1021b850529660e9b9c233f0ffc35e38bc3', 1795778998975800000, NULL, '\x4cf5038d2231277a', '\x01a7b85847247990966af0ebdfea3442b03b94738e040c17dae07548c8d4205a3f47fcd687aa53bc33827e66fdbdac31ca15c326ccdfac20d812ffa1f8', '\xaefc92ab523c6aae2fbd240c34c0a8ca', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x2f07a13789b2e7c3dd22ef8d052e2a34', '\x230d1ce284dde928f74daa11f58dca09691a10c55ad8a97a2d5fc628562564c7', '\x0119d2664332fb251aa626e71110ba5dbf4d0852079e282f9154d343cc96a8397198982e89', '\x01efb0e98d3eeda7cf0eea59df918e057c46f3d727241a1b1a44e1aa2045', '\x0174136b059302c84d8c93c21e66b9a8198001294475c89d0f6da871e3df36f36bba9e9d75', 1795778998081428000, 1795778998308639277, '\x7359d9574d6a2669', '\x0182a9cb4e369d48cafe8a33b06a317c413f9c51ddf7fc2bd1a0db33a43f59de079d2f3be52ddecfa3e5ae8e8e9dbf867fca8eab21cac9be61026038af', '\xb695c2702822cbc959913fb29b4601aa', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xbe52d022b41d84d43e2db223937a8fea', '\xdf7e8d391a4da2931b37e7d6c28c6534eaa9cd9554a02e2033f49c59699ee01a', '\x0182839bd28fbee19b793974c3b703f36e4fbc97973eb70c861a74aeab7a641725d79d73be', '\x01daafea082d84ea6862b3fec0cdfe7f5c36622d4b910c190bab229d02d3', '\x0159a0572a62329c7be04d8e88a9895f6b9900b605e25742572c75a3f2f4566ec2205bba96', 1795778998124972000, NULL, '\x7359d9574d6a2669', '\x01478312a65dacc03720ef79e2bca6eb264c2563fc2bdc042cfb687e39dfaf8204d3fe6c39b1f9207a442dcfab149f5a1079028c4de16042f9d0957f6e', '\xb695c2702822cbc959913fb29b4601aa', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xfa9c498657ffa3680b4544df1b99fe4b', '\xb82586838cf9e23b2d0db087eee9735be48bc88234376cd990332773e0a4ad90', '\x010523261831a8a4948c3f3d7ffee25e800c9613e31e25aedc3f77e2f31ced0e28e894af5a', '\x010bf313f86afe1c485089a9f926b751fc2faaf5fbebd1cf7235ba86063e', '\x01ec272119d2ae5c827dc6be37295323ba583e0f73aeb547d2f2fd15df3d4b4c8d5b94c026', 1795778998162042000, NULL, '\x7359d9574d6a2669', '\x0191f929910a3dc31210966a7e9ba25a6ff2dfad3326aec7751af04453be9774bb1ae6d608f7244f8e72738f176dd63c560b06e1baff41918859bb409e', '\xb695c2702822cbc959913fb29b4601aa', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x08230e03fc049ed6f9aef30e186d63ce', '\x4342ce5fa8e4131830cf0ed96ddb2e79b0860c0b598cbf8faab86201983036c2', '\x01c14f4e660279b3669e62b78c6a666897ebf093b7891c2f7f0b1edb31713599c83fe1c715', '\x015a0853f93cee4e5e4cb4601d5e750e9e850e54efd4fec3a0271e4a0073', '\x0100d578db6b1eb31c9c8b6fa567cc4e604aac3db6fed802296a5f1da7fe7e8c806b55021c', 1795778998200215000, NULL, '\x7359d9574d6a2669', '\x01168a5252c83553ef893248779e2d7a57e611adc74132cdb25e4fb04569b5c0cf2a2310f615c7281aa06b5e1c1ca6e5be857d2e05e2d9f5b05b4aeb38', '\xb695c2702822cbc959913fb29b4601aa', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xc439c3bdab4f1fc2bd53c6e1fe4e2778', '\xd76aebedaa467cd9e4fbe29f68b08b1e1e47aeb799cb067200ea86a55f993d04', '\x01541b5c5788d81a17548f747e46903d9396bfa649ccf945655eabbc437b54d4ffd0dfe17a', '\x016d562d63ff0b35c414bc0be9cc2d25b9c417eb4a9feeee1316a6fe0171', '\x012dd0035446b638e411dd1ec01a052b6dc9ef396220242837b9c0d2ca55a8d7de24a06214', 1795778998223726000, NULL, '\x7359d9574d6a2669', '\x0128bfc83dd3c58d86d4d98fe8f6d0e1775de3eeef1110427367d5a8174bf1c4d4fc5905dbc992807b128050daf569be10029c3932d9a48a99d51d79e4', '\xb695c2702822cbc959913fb29b4601aa', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x711a6ee8d63b28c568f6a8fe07b2da32', '\xc9ea4f44113efa4566b4940440dc1fcce654e2cc61ca05a8e27ed617a1e7ed85', '\x017e11e101f39e7d46eeb91f184042078cd43619e20487abba9a1077b37a3d445e6592bb18', '\x0123e075f15bcb6d9b6be7bbcb05eee8ea8d09dd47fffd9011fd5f3e3b46', '\x0177f1c5b69ab75f3b06be98f23c7fc1be461f7be5e8e7b1e5edd12ea5cbcfad10bfdb73b5', 1795778998243903000, NULL, '\x7359d9574d6a2669', '\x015a1ddb62680f4b229b18bd6a75828a5b606c8e51dbe1f42c3d8312463ba664e79129e45e2b597673c6003cfaf4f7672791260961a78660454589ceda', '\xb695c2702822cbc959913fb29b4601aa', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x37d3a6f7241c9b2901a4fcc8a95ba778', '\x3b95685faeb3f15a05c8bba1a153c87ae1b8f71ba8f03b208276b8799c8bc4e0', '\x016097a6760a2af3f10ca3bb99bbc5d6d048c82d8dc4bb8ae01981c7060e4b0cb42fcd67ea', '\x01804c1b028ed24ea66bd354f385869c927496ece75e7cc9c8301ffa257b', '\x0132dad46220733a8abf4c1ebe0a60e5efad02f627a57ef8ac95d57826de8549e9c4aa8a26', 1795778998264956000, NULL, '\x7359d9574d6a2669', '\x01ab982d9e1cabe692ace8127ef20f2cbe1ffb9e33c1e39b18f99a642847a5cea7a973d076a801d849354acbc16bd659c37229978a2d10a50feecc9bd4', '\xb695c2702822cbc959913fb29b4601aa', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x2652e0493b3e6ad7b404599132deba71', '\x756fa0d81fc984345d270bea551d64948e5232d0568efe21eafb08588370ca98', '\x013df95c6689fd4e02c1529d31f2393969669f06cb17b8f4eed632fda59c5ac1fc43efc4d7', '\x0142ccce507ab40546d6f2fd60e59b786a7dc7e5540ef9a15a17cf3d9269', '\x01ee2f3ddbb72aeec37c53fadb9254e9f768227512c46195b13e08e07e19fabdd7efa933cc', 1795778998286666000, NULL, '\x7359d9574d6a2669', '\x0164beb1c6a661b10cbc620ba8048ba0b3a1e1a6e4a48fbd2c1818c41cdbb74bd2fc31c99ed4fd7c0dbcf503c45662ad93a7120c57af8758ac9faa63e6', '\xb695c2702822cbc959913fb29b4601aa', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\xdb8e249a890f311c8d8b8ef6ec08f18b', '\x0d15127aea0f3f936f7cad4f5148113dcc3d95bad7bbbf6f65da422b175dbb81', '\x0153dbde1a77822ca7bef0cd5c98c4755a114ba615a5f583e255806916b4435af63cd02d4e', '\x01fc33c417ef4aac2e26ff0cc83a1ef1cb0ddc7c66aeed25c4832f2f195b', '\x01bede23bfed450cfbaeb99033ad323d2b9d6f652e14f34a8347a8f5ed06cfd3a42d2674fc', 1795779006602147000, NULL, '\x7359d9574d6a2669', '\x01ad760f39fbf1e8a5b32a914caca822f192d2c549592f5669ee24f96046e3aba3c82ce20ea7cc312861b9f31e2e93e78f8c19ff5aef27c8b102246d65', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb58b38b1a2f0b84bc845c84ad29ba658', '\x22c8a8c8054a74ae4c3980682f4e9f6f', '\x839c0617b9a3b00de9da075e232b341d33e62ad015b71c97be03bc4b4815bbee', '\x017c99292abb0372aa9ad059a08bbbc179e5b652d30bb48a18669e108e2f47021578475b0d', '\x01a70da670866059257fb06d7bb63737bd4137f88cf0fddb8f70fde17259', '\x016b6afd6aca1a147b55e13097ecfeb9eec973b839ffe7e8a387aac7063d0efc1b3557c947', 1795779006648396000, NULL, '\x7359d9574d6a2669', '\x01a52bdb712b950c435ca67000083544dab78d543c845ebd66ea3c58a2f6608fb950a6a1f01bfa20eaab3c956e5c2922e7811412bfc060b46d1356ed9b', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\x98aeeebc2c1cd9886df765a2493818d9', '\x469f1443f99aab4f7b941c20424d094f9791a2b1e2619ada123e712ed183f3f9', '\x013daeedf523500cffaaf49e0e99aab7680daf94168b2a3cb5f1184971987b68a77aa30bca', '\x01b034d9459854e556b3fad341879b1c818d04a8e220f8933f585cb56bc2', '\x014386bfc2f571fb9bc145d1c8ad7cacb372bbe84c1c07fd032685ca182885352bcf002046', 1795779006676039000, NULL, '\x4cf5038d2231277a', '\x019962784121ee778c697220c3e80cac67a3edbf45119a838c739e60b515084c1ec00710500218842bc5840bf6afedf13bd0c3bbd49ca1468acb73a0fb', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x93f5d0d372e354ccdfc220cee1b49b87', '\xb3cb099b5a7d6eed60582ff21f3a8a63', '\x2f33cb71dd2d2184d468c771a0ac40130c2de8b92a8b3e0be33c5d01ec1f4ba1', '\x01db7088169188a0344c80bbfabe4f156342acc7e1cec3d17dc2365484cacc412def3f6da3', '\x0187c17c0f9111618efba04c68fca7a6fe5b44d595df89d37102f44b423d', '\x019733411d37177ffa39f7bee80162d3e7a3d3a2621c99a4b1f28ba33bd4332b0f4adb0541', 1795779006696673000, NULL, '\x4cf5038d2231277a', '\x01f905fbcb4effc33ea9f61d2082d88d545d5e9f65763790b6e0286b686cf140811ad0110ea507fe8a29ae5149b9aeb3d2b4fb0cfa2d67b90b9ca20781', NULL, NULL, NULL, 56);


--
-- Name: admin_audit_seq_seq; Type: SEQUENCE SET; Schema: vcon; Owner: vcon_owner
--

SELECT pg_catalog.setval('vcon.admin_audit_seq_seq', 26, true);


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
-- Name: lease lease_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.lease
    ADD CONSTRAINT lease_pkey PRIMARY KEY (name);


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
-- Name: pending_file pending_file_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.pending_file
    ADD CONSTRAINT pending_file_pkey PRIMARY KEY (tenant, file_id);


--
-- Name: pending_tenant pending_tenant_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.pending_tenant
    ADD CONSTRAINT pending_tenant_pkey PRIMARY KEY (id);


--
-- Name: pending_write pending_write_pkey; Type: CONSTRAINT; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE ONLY vcon.pending_write
    ADD CONSTRAINT pending_write_pkey PRIMARY KEY (tenant, version);


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
-- Name: lease; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.lease ENABLE ROW LEVEL SECURITY;

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
-- Name: pending_file; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.pending_file ENABLE ROW LEVEL SECURITY;

--
-- Name: pending_tenant; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.pending_tenant ENABLE ROW LEVEL SECURITY;

--
-- Name: pending_write; Type: ROW SECURITY; Schema: vcon; Owner: vcon_owner
--

ALTER TABLE vcon.pending_write ENABLE ROW LEVEL SECURITY;

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
-- Name: lease registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.lease USING ((vcon.in_registry() = (name IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: master_key_use registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.master_key_use USING ((vcon.in_registry() = (master_key_id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: membership registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.membership USING ((vcon.in_registry() = (user_id IS NOT NULL))) WITH CHECK (vcon.in_registry());


--
-- Name: pending_tenant registry_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY registry_rows ON vcon.pending_tenant USING ((vcon.in_registry() = (id IS NOT NULL))) WITH CHECK (vcon.in_registry());


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
-- Name: pending_file tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.pending_file USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


--
-- Name: pending_write tenant_rows; Type: POLICY; Schema: vcon; Owner: vcon_owner
--

CREATE POLICY tenant_rows ON vcon.pending_write USING ((tenant = vcon.tenant())) WITH CHECK ((tenant = vcon.tenant()));


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
-- Name: TABLE lease; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE vcon.lease TO vcon_app;


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
-- Name: TABLE pending_file; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE ON TABLE vcon.pending_file TO vcon_app;


--
-- Name: TABLE pending_tenant; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE ON TABLE vcon.pending_tenant TO vcon_app;


--
-- Name: TABLE pending_write; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT SELECT,INSERT,DELETE ON TABLE vcon.pending_write TO vcon_app;


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
-- Name: COLUMN registry_key.wrapped_root; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT UPDATE(wrapped_root) ON TABLE vcon.registry_key TO vcon_app;


--
-- Name: COLUMN registry_key.pending_wrapped_root; Type: ACL; Schema: vcon; Owner: vcon_owner
--

GRANT UPDATE(pending_wrapped_root) ON TABLE vcon.registry_key TO vcon_app;


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


