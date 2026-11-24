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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-24T20:00:15.307548856Z', 'cli', 'tenant_create', '\x71eeb1603b963fe03a5a6abf29f4e25316e8ec7ed6dba263355e22c68b294a5c', '\x0182bfae15fe73ddc02462a701bf2c7d03f29b24cc3e2042d42e667faf59ffb5568ed39457a750a9871be3f390543b47232afe0dffdcd41a9213da9ec7', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-24T20:00:17.146325346Z', 'cli', 'key_create', '\x20491d2511fc4ca5ffcfada4d8c97e8e7177964a417cd974656f062141168988', '\x01477777bf9e1245cabb8176ed4dd9b29813401235c5fc57d541de2e708ae4b4b9df743390b2a9c41f39af244e', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-24T20:00:18.94013907Z', 'cli', 'key_create', '\xd06973841bf0ab3aa2c2005e97cf8b6ed21ff94a4e69a219dea79e58e74585c1', '\x01ddff6081b80230f3a1f85104f44416d8d9bc049ba33fda4cdf3c4761fa025718ece035873bbf843e8f994397', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-24T20:00:20.83108102Z', 'cli', 'tenant_create', '\xf52ae81e7f4d2d431f456e0f4c4fff0a159a223d6607e0cdb1b2b0d94d0702dd', '\x01a1fa23e28df857bac3bfd27407d819f0e44657f6e2894eb1423fcf82a53eeaf007881521f2cfb564a5d5e61c8b45c8101f0519a010df512d77f4d858', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-24T20:00:22.626944062Z', 'cli', 'key_create', '\x58247c29e97dc950f4d92c7ec603ed4ee0c71c6a033afa157a8511f20a227fff', '\x01458058e6f0d9fae7734996981f11cbfbad8f52987820bc027e3e49b02b8d5a09ab07752cfc2f12dfc7ee58de', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-24T20:00:24.376091749Z', 'cli', 'key_create', '\x9c5e447eacabc9ec9371960cc41dc45d8a880983b497a83c57f1dba27bee4504', '\x0185614de1e2da3ff29b94d14d10fd8237f68983156da1b798a1aec72f2cfac45d19e24d516e8c8c0104b8a677', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-24T20:00:26.185547341Z', 'cli', 'key_create', '\x8f97af343e78aaf8758b87f3d9cdf21418afa5542ae6aabfe8bba356100b1874', '\x01d644c73883471410cde955e9f862847ac3f6eaa55683f5c081c9e9b2870e521b3db55738d5de60f1597b765f', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-24T20:00:29.174983549Z', 'cli', 'user_add', '\xb9929a859d6d33da34b105ff5e60156711eae66dfe64c579a5edd6aa8d64a8ad', '\x0188ebdfd7580043899018f0eb0fbbabd6c9e2d384c1d86fe8866076f434b980908124debd8af2838bd88d5e9b9b04c9b5148c424a472eb59a226b8ad0', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-24T20:00:31.561389353Z', 'cli', 'user_add', '\x837ef687773f67f549ea21ae6354085ab3c8e8d5fdfa5415e8ad8312c39833b5', '\x01d12a880e88b1b6cdd9cd5a93d8404f47c2e675366c1a70cd4241bca7ce281591a6ffec13dd377bdde226bd692c3f9951da5c96e94613ac7917bc7585', NULL, 201, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-24T20:00:33.564039982Z', 'cli', 'membership_grant', '\x261c16f3badaf1cc0c5fb2744716805213ffe10dae249d82798292c48374df73', '\x01612efdab97099a43e577afea2ce71d18000dcf4cc7d8cb11bda27f2a7dbed917f8236d362ae56ff7849e7917912d5e40d905de20cf97d77f7f521c728663f0ad9cd1b80f514b073fdfebafde29ba8f357df80e6b5ce837c111c0181c16b5ac7e6ac18825', NULL, 200, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-24T20:00:35.475830834Z', 'cli', 'membership_grant', '\x9803877117bf2c31dce3107e92230d5591c492d2cf3f5dcb8898b5a536b5556d', '\x01c45357ab25e3d5feb937e72ece51351e9e2f0349984bf5cfd681774d380a347d8d4eaccf8b75988aa3dbe252fa3390f473b91dbaa791a5ec7ae4bacb6d04a4133a386e7639bb8c348862d0c384c8898be7e86614b820ed5d8c21f275b03cb24235c53fea6d', NULL, 200, 1, false, '\x384eba6cb8875517');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-24T20:00:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\x384eba6cb8875517');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x384eba6cb8875517', 1, '\x01e0fd0875c7fc9d926148a61b140f3c53e0b37e1757127b8dc7dafa0ed043589317952fbb7e5202ddc1bbc4c4d95fce6833bb91dbf9da73aa0638d2b0');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x23c5b7fbab34d979', '\x0ccba7d5b806a9ee6cce897b8988302a', '\x67af03fa413d30bcf6cca9923ded75e8464d66b493ab6c881aab1da3a8e12d60', '\x01b3119103558aa62757aee457d143440f464ad2a0373088151ebde14069aed953', '\x0169d7aa04a8c5c4e176af263033827a44973147973f70dfc99602e7ec6dae028661d9a96fb9f8d183472b1b', 1795550418939749219, NULL, 1803326418000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x6f595c31bcbc83b6', '\x447b2eace76a8db85ca208b6cf0c5c13', '\xe9e57ab8417f3dda7eb18d642c7ffa61943f2c1b01dcbf4ab776499bed408cf0', '\x01a2c522bb73995a2df6bb0c4815469a3a621afc462f7deeb599b9b63603f7dc96', '\x01a9a22108ad25e099efaf27bc89189aeb6349ef492c4cd0f339bac8e6afd91737e4e97e8240b36b790e035b', 1795550424375645435, NULL, 1803326424000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x7f9cc3a0556ca576', NULL, '\xafd93d7c162d90616487f0e107201affa3473e9b00adcbedcaec09726ccccaae', '\x01459fc33d8bbcd7ca4f0d1f21cfe29150d8397a0184e5fc2bc8076f316ba61dab79', '\x0150d3ca73680e7b41db094cd6309197e09d4c8dec09d1a809d8bae03b5a3098', 1795550426185005462, NULL, 1803326426000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xf855fa086da6962e', '\x0ccba7d5b806a9ee6cce897b8988302a', '\xbaca0fd5549408f29b0e61081b8f578f5d34c67a0301cec9fffcf16f3551c954', '\x01bec7eac83603f2fe1800cfbc2d3d2fc7afab4b328927dabf1b166566ba4b760513e90e8085fa67c29336cf7f2b5458e9f28e5009d761947f6d', '\x014a47cf6c6e08425c75d37aadbfa1572baa17b2590b278659a3d240b1780e4add0ef0b07cc202d68618c3e27947', 1795550417145720078, NULL, 1803326417000000000, 1795550400000000000);
INSERT INTO vcon.api_key VALUES ('\x7ec775fb52033d27', '\x447b2eace76a8db85ca208b6cf0c5c13', '\x757bd2d7cf36560188c6f8fcffd32885a253bad8f5c49185e913d30134527d3c', '\x01258ada513d951ad3c3f66e6e45236795064610f07a78c3f11063035ffb7c06d2a5e5ecf13f851639bceb49798e7b58413c4e8eed7b2b479e72', '\x01de380eb8f34c74abb1b7c63e4ec24e34fc3a486cb19280ad14a0c39962ad9592268f927ba1ea3c6d4520f87cc8', 1795550422626572016, NULL, 1803326422000000000, 1795550400000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 2, '2026-11-24T20:00:17.149665245Z', 'cli', 'key_create', '\x7e08d6ffd2636a3099cdd26a618ea087adfa33048af446f432d0118945aa37c4', '\x0198d385d5dba4229c34665a7ee9bf77931ecfc95277ea74d34aa61d0d94e90c3e51d25d61e383af1d8aaccdbf', NULL, 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 4, '2026-11-24T20:00:18.943245516Z', 'cli', 'key_create', '\xf52ff6d746c335882d22f0d8708b24e36eebe46f51009f7b8382246184320809', '\x0182987b167cbe606d85fffafd232185e660973d9fda3f0b5c46d75e1f51749063f4f5ad8936a0f04fd269797d', NULL, 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 6, '2026-11-24T20:00:22.629811343Z', 'cli', 'key_create', '\x8bca6b796206bd84fffe7701a54377f5756aeea4a22d373247d1c1b6ab077587', '\x01514d6463ab0d9291aa84983d87801e00fb5ac1b03e2fcbe574728b00bea2415f193d7d9d8fbe7636a683b506', NULL, 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 8, '2026-11-24T20:00:24.379618315Z', 'cli', 'key_create', '\x703d7896ce048844989da68d778dc26ac5187cc05e432917bdb277583ffc936d', '\x0124d251ad10b7c629f735fbd22eb8ccbae90371b55ce01d9238385c00f3dca854cfb01052105f071354e1cec9', NULL, 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 10, '2026-11-24T20:00:33.567110078Z', 'cli', 'membership_grant', '\x732d9a1c81d2232af308081f44d754fe262a70eabbfef85f856c421bbd12c301', '\x0169192472733889f513e5270bf06513903da4fd9d8cb24f4c27030f560dfaa7b5454193763349957dbf3a344c9fb07f8d1ef09c13b744ee6f2edc4351a69923c321743e69f44a25930f0f01710a59a0e2d644dcc26f1cf21498dca2da400d83a80eac9b80', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 12, '2026-11-24T20:00:35.479262632Z', 'cli', 'membership_grant', '\x7768fa973001654499493064d0447be56467ff365193398c9ae1840c700e5517', '\x01b1bdde795babeb3495e80978ee8f79297bac7e14d397b030bcfb4b0177857f4ec8110afaee631427e13d7032fc56d882cd852ed463617c63bd843409e0f8dfed84261202382b62453cfb82a5b3e45f158d1cb249e242b5f63e421e6b1990ddbdc555b1c56a', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 14, '2026-11-24T20:00:37.862497588Z', 'f855fa086da6962e', 'create', '\xb02af1eb2deae367f1ec47ffa02722e1877f9813316e9a804521c06a826e5a11', '\x010eb3ad9e61570afcd20af6d078e2b7c448cfe90b8d022413fd50688f48c547238fc12bfe5adabc0674be6ef9b28a6a7ea3b6d33a8e03b67ec8affcdf9c449005', '64af7b33f83ae3c4734fdfc0b9bbf389', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 16, '2026-11-24T20:00:37.908224335Z', 'f855fa086da6962e', 'create', '\x7b64b370b22afa8dfca9b57ba3fbee930b135aed632562b1ea36c010a9d658f6', '\x011943821e09ebf01e4efa27102261283b4d6eb9f6c8a570e1188ea4a61a71edb748a4759000fbe291e7796d3f63707c450f3158e62a058c381104d5d3b9d91dec', 'b082c48b7d8614e28c0e7e8d02cc6ad3', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 18, '2026-11-24T20:00:37.951750752Z', 'f855fa086da6962e', 'create', '\x1043ae9e8c2596792f11197b429129fdcf25029797701779e758a33abd0380c3', '\x01baa2a09962fd17558f059860d78e8c029ef8626e27fba86d54f02e1fc0c068864b1212ea6ba10b1315cca60c30a18aa43838f35cea566f15239a327021de1694', '1c7c2cca79fdfafd0b4dec61dc3bf337', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 20, '2026-11-24T20:00:38.00632112Z', 'f855fa086da6962e', 'create', '\x4f0ce6fc13ce195efabaa51636a4cb32b08815e7ce36751228f98b1854b65232', '\x01c986470b02b25a541fbef6a1287c4f208334162e8988c9db344bc66d72d0988df094d42938ed8a991ff55c182228dd237c0ffdc9f8a2a6f2cd89ed7d6522932a', '3dd96fc88a9f8ef66dbef837a3c3cd15', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 22, '2026-11-24T20:00:38.055734645Z', 'f855fa086da6962e', 'create', '\x29b5cae40c6f00a1174991a837fe1c5809b5e39be9ba02828292cbad0df34ec4', '\x01b14cc14e700eb4d6ed175e24c8009ae2789bd2516e0064fd727bf9d869b7319394fc1e4c3bf4dc13acc41c3935b486f59ae6202d3e2c8c7992bffbe59745410d', 'ae808ac83a2dcd2710a82a53afa8001c', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 24, '2026-11-24T20:00:38.108456894Z', 'f855fa086da6962e', 'create', '\x1eb719b9815a3f3151eb00eca660c4338ab4397516c51e63f74a164a1f55b862', '\x014f06dd687bd42203be4d567279b9706c9b4983ab49afcdec971292f3d20dba6ddabd76c66d5aee8c66da1313f7e851b4037412a2da3fbcd6d96993d701b83789', '7dc908a7c733fa3625996afe0ed56982', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 26, '2026-11-24T20:00:38.17197661Z', 'f855fa086da6962e', 'create', '\xdb72985900c51f43734feaa605385d6bf6708f975e89ea771a633454f6f6ed43', '\x017dd2c3248e9839bdb0d8ff74e93d02a9621ff00b32b3ed34f28d16fc29f791a9c5b1cd7f5a1efcf9bc9934b93cab370249bfe42bd03a2586071f1d526a0dbe4a', 'e42131b6b82713b087dd1bb3d98fa065', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 28, '2026-11-24T20:00:38.223615102Z', 'f855fa086da6962e', 'create', '\xce16ff00483e8388e019be3cae3b5b2d702fd2d1b92c943dc86647b5882dc761', '\x01cf676d3725030dfedb5f4029103d13585f651f38cac7ffe33a734063fea301a091b03a1c5324715fb4cbb1045445d53f18fe4f7785dd5b8687be72c1e1b73290', '583f4a9a96eb50ef61b2691b995b3786', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 30, '2026-11-24T20:00:38.275685964Z', 'f855fa086da6962e', 'create', '\x5c360d883e82c9b26400cf00ef97a4b2ce0f53c004238c40907ff57e5912dacd', '\x01adb1e87dd9c09ab72685a3d468d554c11615a70b23611c49561d57bb78fa0235c3cf9c8dfc2c3ce57115553246edf93a9af46a7e3e63cff1d9b3f072f6f6b775', 'e4e21918b2d808a1914fb337276a2ed9', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 32, '2026-11-24T20:00:38.313074198Z', 'f855fa086da6962e', 'create', '\x31a158c963db909b0e7a555d361cd3d03fc63dbc1342e570ccdb4d87eff1823a', '\x014bfb8df4624cc8a31bf5ed7d677497ef8c16d46d5d675d407c3531ca05c8a477b9929beb9792eeea871e839b88785220fd2d97c4670b5fec65d4eed6c84602fb', '8c05b3582f993d8f7e29a7ecb9f33ade', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 34, '2026-11-24T20:00:38.350101823Z', 'f855fa086da6962e', 'replace', '\xb02af1eb2deae367f1ec47ffa02722e1877f9813316e9a804521c06a826e5a11', '\x01b959a6c8e4dc1ace1677ea80e0e9fd64bbb8fe2347b76a654cd62f455d5e6487d7843e92dda6c32b9b5396cc42465123c21545aa2ecf910b076f8aca5f9c7ebb', 'd5619da83c3a2949c4e288134f74503e', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 36, '2026-11-24T20:00:38.396889436Z', 'f855fa086da6962e', 'replace', '\x7b64b370b22afa8dfca9b57ba3fbee930b135aed632562b1ea36c010a9d658f6', '\x014dd9a32b844e27d03584e82a076ac1266f251124b87756582baf0b806b996a855559f77fa90ee5b2ec4118a02ed09a09533d14d355ae76b150e4e5e7e7d690c6', '6653e7d4d8cf3ca57ca05a3c461d480a', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 38, '2026-11-24T20:00:38.400895873Z', 'f855fa086da6962e', 'delete', '\x1043ae9e8c2596792f11197b429129fdcf25029797701779e758a33abd0380c3', '\x0110fca617a920275506387aa29963e66d18e04e118c32f89bfebe1225155ccc192883eb88a7631a64ff69d60057b66c3464be8c5e19c9917efb5fa7c1676968b1', NULL, 204, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 40, '2026-11-24T20:00:38.408987337Z', 'f855fa086da6962e', 'delete', '\x4f0ce6fc13ce195efabaa51636a4cb32b08815e7ce36751228f98b1854b65232', '\x01222cb893bbb1d661a82214feba1601eeb804f69b3687d59ae7b39d8d24465141b8c447d8fcbf0fcefebb94d1cd5da1db58abd398c212a54044f5b59d3e805891', NULL, 204, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 42, '2026-11-24T20:00:38.41516512Z', 'f855fa086da6962e', 'hold', '\x29b5cae40c6f00a1174991a837fe1c5809b5e39be9ba02828292cbad0df34ec4', '\x016730ea8808b668f1efc141ed98c2e61699f40c942f7f1fdc32bbd6b8ae97e0c45e00a9448c5b12ff7eca2c0f6408ac34bb51de22071fb2e561a9c7ce252c8f0e', NULL, 204, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 44, '2026-11-24T20:00:38.419156658Z', 'f855fa086da6962e', 'delete', '\x29b5cae40c6f00a1174991a837fe1c5809b5e39be9ba02828292cbad0df34ec4', '\x0171d3d19a65bb99e4a2fe49f30438f1f3b6bdec8e5261d7c57c4f048feb1f0c0809cc5c81353e80a0a6583e59f727c57ad01ff4a570095f2c1ad4f8c8b293ff8b', NULL, 204, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 46, '2026-11-24T20:00:38.424589323Z', 'f855fa086da6962e', 'hold', '\x1eb719b9815a3f3151eb00eca660c4338ab4397516c51e63f74a164a1f55b862', '\x0153817462ee9ca607b8f33386f0cfbd52c4bf89b68b1b34931df9801b6ec5d445c756f45da14bca4b825717c0968fa6c673222af9ca284cf8483e961527bcb5e1', NULL, 204, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 48, '2026-11-24T20:00:38.472158927Z', '7ec775fb52033d27', 'create', '\xdfb0cc740972e6b89cecd1938b750fa20611c6180792860fd03e9c09bb9ed94c', '\x01c2ae810705a6823a63c5b29b0a62b432995e558dda40a5c2e6304ab58e877c29e0aa6e1d9a3026e6b55b7e6a03c97380e85f538f0b73d5bd87ea4e0f9dd4304c', '06fcf8853bb2736deac5ea2b81e89186', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 50, '2026-11-24T20:00:38.519337662Z', '7ec775fb52033d27', 'create', '\x652de9a8cc9cb613772fc0d875f252724ed1a4c44c855bdd10a86ff6b1337525', '\x011900e7eee656661cd4cbf719e46bd0fb7a80ee5f4e7eca5ff6103fe2bea3e728cec653af739a761220ff10b36e10fc457ad2b1a6abcb6b258f832a153fc27fbd', 'b992cc1127208345c1199f2a8a34f4f8', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 52, '2026-11-24T20:00:38.550286311Z', '7ec775fb52033d27', 'create', '\x0d66c4314e38f040916a6f335d1fa3485c35a898acfb3d1b5fd76cc0261de15d', '\x01670977a127a58025c712abe8127c50abb313163000b0869794f81a547f5c69a97c35e8f60a7425ebd0195d0137760e1d41015a5e90dc72e6fa06adc043f4a2e0', '2dc7186ecd178b4705e33cfd33937f59', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 54, '2026-11-24T20:00:38.597822804Z', '7ec775fb52033d27', 'create', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x016b91f32f9347e7e430d87eee4ca0f8f25e24cfb235196f5e98558c28ca264f5f986403e000b42338eb802ec4cb38a12f0ed7957cbc645d0c462f498e92a9e162', 'c1bb35e1973fca8cd4bbb5dfd8c5e8fc', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 56, '2026-11-24T20:00:38.644869696Z', '7ec775fb52033d27', 'create', '\x41b468bafc0022dd83f082385b85df01ddd7913b74d374980bf5ec5c4758643a', '\x014c9f2d5ea345b41f329d9a79a8770858118e4c85266537845479a425581a9242f170c1ed66ec5885691e84329926c506859ac8cd4e9a8157d20e9694f18b64de', '41c0dc030c50c7dd08dd49826f04822d', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 58, '2026-11-24T20:00:38.692269391Z', '7ec775fb52033d27', 'create', '\xa22523320d9b59aa6be2c7959eabf5c8062930b125d70262cd6e00088b570394', '\x01ad354b4f45f7665169644d0ca757855aa394241bf3753df0c33491006a9649a33b709721b4c61249c48fef0aa5702c06e0e9b817e3cc3d8cc752fb99cb26e980', 'de1c779bcf984040d01fde40628c2da1', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 60, '2026-11-24T20:00:38.713822493Z', '7ec775fb52033d27', 'create', '\x1c861a3b69c28424e574f021a311824ced3815fa40b941e0d47e14b6114cb2b9', '\x01c82f329ecdecd5dc72619f78992f064adf5ad0e8e4582977a92b254337973a698368a646a01844ae01f251a1892aa9127677e64c3082ead8d68b14deaf0ff961', '30087fb40c0b95be82c8ad0930e8ea42', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 62, '2026-11-24T20:00:38.73220014Z', '7ec775fb52033d27', 'create', '\xb2fdc45777f7027cd6bb6350890109f102c2c29563b733c4afd8bf2e996e7855', '\x015ef35d2dc683c8a2f34a43e804b202ba262dfbeebc8e595b262238a0c222d360772b3aa1aae53b1a1fce919842b3395374bf5a48103c6722fbf35f1c6e815f00', 'cb1b1614839435819e7fcf03536e7c47', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 64, '2026-11-24T20:00:38.765413439Z', '7ec775fb52033d27', 'create', '\x20335bc6984e9895ea0d9d507994983cf0c193cc2e2053ecdad287f4e3237bb8', '\x01e6f7972d3f53b3c1b09309d9319a2d93f4f62e10125d94695fd4ccd5254ef8a105bd1d19fb4b52e595f89cdd7f960420efc065c39dfb9fa31d5aa925d3c11c26', '7c7ba535414e8c0cfea23b0999ab3dd1', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 66, '2026-11-24T20:00:38.813010042Z', '7ec775fb52033d27', 'create', '\xd82da7b1f94119318f8e9374d6c4df3f8e439a8bbcbc9f970d6303b003fd2fe2', '\x018bc5ba5613654fe3e180cc37b2c6540ee99ec24251c8e49bc43c4ed3bfbd65909f256c718e37692c7a1d39076fb340b529b219783bcaf35f0ebb827af4b3901f', '281cc5c21dcb717c91b68ffe6b49c8cc', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 68, '2026-11-24T20:00:38.86236102Z', '7ec775fb52033d27', 'replace', '\xdfb0cc740972e6b89cecd1938b750fa20611c6180792860fd03e9c09bb9ed94c', '\x012588dba28355629b72c2bca56667d7f48653f0c283f41fe1a442536d42f5c865547695a3cff1939d84434018e5b993efdf0eb081fcbc43b594cb65b5f6d154fe', '67ab3b32300d8573514562e5305b4f4e', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 70, '2026-11-24T20:00:38.866300095Z', '7ec775fb52033d27', 'delete', '\x652de9a8cc9cb613772fc0d875f252724ed1a4c44c855bdd10a86ff6b1337525', '\x01dda1784ceb73e40c6bdf2931719dba504ea53e33c7883f83fad52a7e61e610a26324b95ade957ab60a8281528f8067a64eed930ec2a097e8bd216fa38c46cd48', NULL, 204, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 72, '2026-11-24T20:00:38.868661932Z', '7ec775fb52033d27', 'purge', '\x652de9a8cc9cb613772fc0d875f252724ed1a4c44c855bdd10a86ff6b1337525', '\x014fc05b68b7d212a6e88c533dbf5916c69f3ea3881fa45ad17533e4535307f42016d0792ab95309e7a3acbdfb66cc72c4a1e259300788a84d77016b3cff5053d2', 'b992cc1127208345c1199f2a8a34f4f8', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 74, '2026-11-24T20:00:39.015470999Z', '7ec775fb52033d27', 'hold', '\x0d66c4314e38f040916a6f335d1fa3485c35a898acfb3d1b5fd76cc0261de15d', '\x011997ebc9a58a77a1850a32c3391709213596a2b21b90a4a6645fbf3893ff1fd76b86ce6b9729731075abf1bc6b52b5b0e4d0b3ab8ffa2cc070fc125739eb5647', NULL, 204, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 76, '2026-11-24T20:00:39.019759573Z', '7ec775fb52033d27', 'delete', '\x0d66c4314e38f040916a6f335d1fa3485c35a898acfb3d1b5fd76cc0261de15d', '\x01b4a12ee51f9814b58e52f366e5828d51cb330f912a1182cbce301f218ff0f35cd8c2c0bb79f8edc96b78d7e591b2202eb77f0ff865140e86e065af64f9c219e7', NULL, 204, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 78, '2026-11-24T20:00:39.025239431Z', '7ec775fb52033d27', 'hold', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x01fca1f275158d947aa6822e932c4590daa5f61cfaa5274ff1e203d25d595461f37aae4e2fcface4cb1fb68ed30d2550be5666587b0781ced17974f3a3eedbad85', NULL, 204, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 80, '2026-11-24T20:00:39.074158411Z', '7ec775fb52033d27', 'replace', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x011c356e7cebb8b147c8c14c2bd48eedbd135ef1d72cfe31568f97d8707a30393aaba3ca4b40104b95821b3c63d9efce8dc1348ec04fa3acb30b0857737455e63f', '019ceed85254aa1abdb52fe1129a8307', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 82, '2026-11-24T20:00:41.447298625Z', 'sweep', 'purge', '\xdfb0cc740972e6b89cecd1938b750fa20611c6180792860fd03e9c09bb9ed94c', '\x01189988cb40ff84ef4378caade4da394882414e415813af6ca2c32982f5d77c07098b187a8c04d1d2ff22c659e50884698af177f84e1463e13b692681bdfe7349', '06fcf8853bb2736deac5ea2b81e89186', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 84, '2026-11-24T20:00:41.826759986Z', 'f855fa086da6962e', 'legal_versions', '\xb02af1eb2deae367f1ec47ffa02722e1877f9813316e9a804521c06a826e5a11', '\x0107a7b23e6ede13ed1b8b8428ae5f77791673f3f8601d40407778db78a2aaeba0fa3fa131692b12609695ebe3c0f1fb961aa3c0a9f398c6540ff3448543e99a7d', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 86, '2026-11-24T20:00:41.833213121Z', 'f855fa086da6962e', 'legal_read', '\xb02af1eb2deae367f1ec47ffa02722e1877f9813316e9a804521c06a826e5a11', '\x0139dd25c9cff814699342e94fa571adfff75627adc9e684c1768a99438e4398e58581ceca8fe3cbf15090feb31daa473cf675d436cdc45143eea464d85ebfa555', '64af7b33f83ae3c4734fdfc0b9bbf389', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 88, '2026-11-24T20:00:41.838100277Z', 'f855fa086da6962e', 'legal_read', '\xb02af1eb2deae367f1ec47ffa02722e1877f9813316e9a804521c06a826e5a11', '\x01b97689464fd2e2a96858b302a4b8fb797f736e918258cf65b4940ec01b2b014e6e03cbc83c96437eab06bcb7f5d8cd4642b18ee872fc3689db6d98a342989ea4', 'd5619da83c3a2949c4e288134f74503e', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 90, '2026-11-24T20:00:41.841520974Z', 'f855fa086da6962e', 'legal_versions', '\x7b64b370b22afa8dfca9b57ba3fbee930b135aed632562b1ea36c010a9d658f6', '\x01f57df2a5df29c3bf641efbb4812e2e5d16368995c72975cb0130c116fc590208d0e71633d311153f93061a0a3f6e62892411aca0fc9035f6a5fd2d11b28f1ee2', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 92, '2026-11-24T20:00:41.846061751Z', 'f855fa086da6962e', 'legal_read', '\x7b64b370b22afa8dfca9b57ba3fbee930b135aed632562b1ea36c010a9d658f6', '\x011a35180c770e90d88b7c437b2dd97ce2f2ba879434d3358fc4b104fdec99075a548793f2d9eca0e2cf1b1e3c9603a0c4142d42ac54cb0d9026b1653e5238ba85', 'b082c48b7d8614e28c0e7e8d02cc6ad3', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 94, '2026-11-24T20:00:41.850657807Z', 'f855fa086da6962e', 'legal_read', '\x7b64b370b22afa8dfca9b57ba3fbee930b135aed632562b1ea36c010a9d658f6', '\x016092cf44c4e2706a580039ac67219e1fac7d6537ee43af102db531f0b9c779e90d642fd51ea7bad7ed7e933b1f98dde9ea8125b430f7adc4f091568f8678c712', '6653e7d4d8cf3ca57ca05a3c461d480a', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 96, '2026-11-24T20:00:41.853898059Z', 'f855fa086da6962e', 'legal_versions', '\x1043ae9e8c2596792f11197b429129fdcf25029797701779e758a33abd0380c3', '\x01946e8d82fe83efc33f0c443969987688fcc881274bfd93405465928eeaf774f8cf0d6d90aa91681a190cbc3eb41b6feea7e57b7ec694bd32cc655fcdf250b886', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 98, '2026-11-24T20:00:41.858189309Z', 'f855fa086da6962e', 'legal_read', '\x1043ae9e8c2596792f11197b429129fdcf25029797701779e758a33abd0380c3', '\x01ac18fe354ab1641efb28e9f4876d2ac27d350f91b477ea20363a2d2b334b1a161020618af5ad46ceb05471fa769c5bb515b5beeaeb552fad992e409e712440d8', '1c7c2cca79fdfafd0b4dec61dc3bf337', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 100, '2026-11-24T20:00:41.861309349Z', 'f855fa086da6962e', 'legal_versions', '\x4f0ce6fc13ce195efabaa51636a4cb32b08815e7ce36751228f98b1854b65232', '\x01a30107f4b4bc1946dee8b17e946e59a7c75e3df8643aef6c6637c7f38462263a0e7ae6c1bf630f9865c9336ff3708115725b4fd9a86d964c89f4723d0d9e4bae', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 102, '2026-11-24T20:00:41.86537847Z', 'f855fa086da6962e', 'legal_read', '\x4f0ce6fc13ce195efabaa51636a4cb32b08815e7ce36751228f98b1854b65232', '\x012dfb426dfee6cd7073817c5b82f1bc03ed23896bf0e5688511c6bc03c2256bbc372d9a9259be8a2b6e31cfe6602583181779c15f5aa446198c38df9f587cbea4', '3dd96fc88a9f8ef66dbef837a3c3cd15', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 104, '2026-11-24T20:00:41.868410926Z', 'f855fa086da6962e', 'legal_versions', '\x29b5cae40c6f00a1174991a837fe1c5809b5e39be9ba02828292cbad0df34ec4', '\x0163530ebad7fbdcedcea785c5835acc550e4df1f3330cc3eb78de1710bc84d682965a978efb6e8d13e4ad7ce46685f705eec22ce2c25e7a6f1bf5f085a1509f07', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 106, '2026-11-24T20:00:41.872543418Z', 'f855fa086da6962e', 'legal_read', '\x29b5cae40c6f00a1174991a837fe1c5809b5e39be9ba02828292cbad0df34ec4', '\x01582eaf3bfdde0b81d43fff181af648401e66e69295e7e634a66e4874e1f0532a60d0ae226ce2182916e536cc95097af7bbd897b39f4062107b49b1208c1bf65e', 'ae808ac83a2dcd2710a82a53afa8001c', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 108, '2026-11-24T20:00:41.875683781Z', 'f855fa086da6962e', 'legal_versions', '\x1eb719b9815a3f3151eb00eca660c4338ab4397516c51e63f74a164a1f55b862', '\x019abc9bf0f1f463fd2ab9c8913212bbdae8893c0bea49677d58fc0e987f903d5261922b98de223cf179cbdf76b4de3317f6c8b6ff9b98c8cab93f7dc20bf3da01', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 110, '2026-11-24T20:00:41.879809985Z', 'f855fa086da6962e', 'legal_read', '\x1eb719b9815a3f3151eb00eca660c4338ab4397516c51e63f74a164a1f55b862', '\x013d712a393a75b319d29a5e6d8f454d58d3f2178f973790d554e0cb461de90dbf3647e964bc3ccd35194b6cacc1bb3ea463f4feff0f29997eef3d089affc82a6c', '7dc908a7c733fa3625996afe0ed56982', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 112, '2026-11-24T20:00:41.882870673Z', 'f855fa086da6962e', 'legal_versions', '\xdb72985900c51f43734feaa605385d6bf6708f975e89ea771a633454f6f6ed43', '\x01631df20b6d60266ab9b933e829628287f906ffcb072e89ef95eebaf0fbcbd727c9e2940d4a5849b33590e2b7517682d72b161238dc63cf2be6482a4fee46cad1', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 114, '2026-11-24T20:00:41.886924452Z', 'f855fa086da6962e', 'legal_read', '\xdb72985900c51f43734feaa605385d6bf6708f975e89ea771a633454f6f6ed43', '\x01b1fdb646ac58ddd6a77d321f46bd4d9e4498120bca7780115f252d6ec1f3334e56e0057ed3c653f6c0241d1f9f8be4a7491c30facc343b1850973c6b1486e403', 'e42131b6b82713b087dd1bb3d98fa065', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 116, '2026-11-24T20:00:41.890064778Z', 'f855fa086da6962e', 'legal_versions', '\xce16ff00483e8388e019be3cae3b5b2d702fd2d1b92c943dc86647b5882dc761', '\x018a78d8ddb739dde9f8510081d030058ffd15e1399b242542a54828fda42f6d076b2901b56732b24d75028aa10e770cc3b49baf5d7fdb0290d88156ec48b32cd8', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 118, '2026-11-24T20:00:41.894033779Z', 'f855fa086da6962e', 'legal_read', '\xce16ff00483e8388e019be3cae3b5b2d702fd2d1b92c943dc86647b5882dc761', '\x019938b565dcd81bf2eb36743a70a4c4290ceadc26fa44ba1b617e4bb6daf9ec65a226c5b58bc5b6ea4079a2fae3770d3539674d1b45e83ddd86d3eebe79fa6e01', '583f4a9a96eb50ef61b2691b995b3786', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 120, '2026-11-24T20:00:41.89716781Z', 'f855fa086da6962e', 'legal_versions', '\x5c360d883e82c9b26400cf00ef97a4b2ce0f53c004238c40907ff57e5912dacd', '\x013ac5821582155d875c4a5861c303f0a00eae651e7276076b09e2341158173524a5c055b7eabe1c0eed8c836456042ce323553e6525623d41b987cc86b039d519', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 122, '2026-11-24T20:00:41.90120568Z', 'f855fa086da6962e', 'legal_read', '\x5c360d883e82c9b26400cf00ef97a4b2ce0f53c004238c40907ff57e5912dacd', '\x01d0a45ea8f862d17dd56cfd5e1fca927c3ca070acd8dd640fa9d1739f2fb24c1a38b070703ff6d140c2466fcc415ea47ba1bf486fd3bdc09d5d4ac34c2e5b409c', 'e4e21918b2d808a1914fb337276a2ed9', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 124, '2026-11-24T20:00:41.904263637Z', 'f855fa086da6962e', 'legal_versions', '\x31a158c963db909b0e7a555d361cd3d03fc63dbc1342e570ccdb4d87eff1823a', '\x017617f394aabf139692f5e3cc327fe06c213d7e41564eafff50e869ee4b76a0d9d3a58c0a933cd836bc1f2c19eb7fdda67e99ad6cec0d81f219a32a2c61d0e256', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 126, '2026-11-24T20:00:41.908247305Z', 'f855fa086da6962e', 'legal_read', '\x31a158c963db909b0e7a555d361cd3d03fc63dbc1342e570ccdb4d87eff1823a', '\x01556700b85343d6faf6bd5b5939b98c03abef6f6b460c1da11c92648fec92583f3778be5eade459cb55bc3f67874ad1c6fb1d321debcb05ba5e71d5453ec7d596', '8c05b3582f993d8f7e29a7ecb9f33ade', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 128, '2026-11-24T20:00:41.911427039Z', '7ec775fb52033d27', 'legal_versions', '\xdfb0cc740972e6b89cecd1938b750fa20611c6180792860fd03e9c09bb9ed94c', '\x01489b597a7dba56c6c834003cf9f8a5d8e5c2db44468538fb3d59ee0c922dbdfa1332586c1f08c2e2f33446120590aa6728d49dcf05410c9876e67e8d61c60594', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 130, '2026-11-24T20:00:41.915665891Z', '7ec775fb52033d27', 'legal_read', '\xdfb0cc740972e6b89cecd1938b750fa20611c6180792860fd03e9c09bb9ed94c', '\x01b4e1a0e04d0dd8dcdab7dee99eac8f6b07aa2929a9f2959fe0f0cb07330aae7b3b43f0293cc013fa9c8d6b7d6b6139fb4c0cf3d85e1ce484dde9bf21c45caa71', '67ab3b32300d8573514562e5305b4f4e', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 132, '2026-11-24T20:00:41.918311932Z', '7ec775fb52033d27', 'legal_versions', '\x652de9a8cc9cb613772fc0d875f252724ed1a4c44c855bdd10a86ff6b1337525', '\x01a91483ff2ef6dddd815ea58b8375511265e8b8407abb1c99a2a779b54803ffeaa1c283bbd53b5a4a5a9844d414e3bf730e1da86688013f99940f0ba7ea7951eb', NULL, 410, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 134, '2026-11-24T20:00:41.921298379Z', '7ec775fb52033d27', 'legal_versions', '\x0d66c4314e38f040916a6f335d1fa3485c35a898acfb3d1b5fd76cc0261de15d', '\x0117b66b76a03f842f22e345b1a3f0084b864a399848659c01d7812e88cd3d2b51ecd8458dd4ea991c5a9e1a59617419006c0c9edd0fa35078f2b0495b2dfe217f', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 136, '2026-11-24T20:00:41.925324807Z', '7ec775fb52033d27', 'legal_read', '\x0d66c4314e38f040916a6f335d1fa3485c35a898acfb3d1b5fd76cc0261de15d', '\x011ca9ec27c4c25e573eb4f8f655ae58fe7d79a0f1cee088ff9345f407fd01d42602ca244b923673318741b04209da3331016340c496251026571c7e3d8794f726', '2dc7186ecd178b4705e33cfd33937f59', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 138, '2026-11-24T20:00:41.92835167Z', '7ec775fb52033d27', 'legal_versions', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x01d7b15d17184cec36666d557867efd2e11b467349ebed4893d739e8d81bd486c1b74073d6a6cef2112d674cf03d58f83f76d9667c1c8668eca166468e710d434f', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 140, '2026-11-24T20:00:41.932472986Z', '7ec775fb52033d27', 'legal_read', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x01c53a68eb9aaf1af0a9d9a846ad9d8df96c8ecf58b453334becf8e9a1e5f2874325623333cd33d8849637c0ebe42935875a3370b806f0a88dcb56820e7189ecfc', 'c1bb35e1973fca8cd4bbb5dfd8c5e8fc', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 142, '2026-11-24T20:00:41.936689144Z', '7ec775fb52033d27', 'legal_read', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x0120401bf75745d3df8ca303591515cd0af36538b7d10ab21649b6412357a0844799154eb1f3b3cdca9583a8921ded5e940778eeb1db01b8ed1f3e70502a2a484d', '019ceed85254aa1abdb52fe1129a8307', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 144, '2026-11-24T20:00:41.939884961Z', '7ec775fb52033d27', 'legal_versions', '\x41b468bafc0022dd83f082385b85df01ddd7913b74d374980bf5ec5c4758643a', '\x01fab13d94c5a2cb4a52c1d18e4c9fbd450f52adbef5df7ada9dff4053e57f14a48e45c998da6293b1105b85086846432a8d0228a98c932337254f306d7f0a895d', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 146, '2026-11-24T20:00:41.943863824Z', '7ec775fb52033d27', 'legal_read', '\x41b468bafc0022dd83f082385b85df01ddd7913b74d374980bf5ec5c4758643a', '\x01a7231a840a60171733422409e6e47db5454fa9dd04d7dd36b570e2457d5a6e19c2b55c03455c76dc29284f38a7a83f8a07bb80968b1375cba1988d04599e75da', '41c0dc030c50c7dd08dd49826f04822d', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 148, '2026-11-24T20:00:41.947138474Z', '7ec775fb52033d27', 'legal_versions', '\xa22523320d9b59aa6be2c7959eabf5c8062930b125d70262cd6e00088b570394', '\x011e2ffad0e9de09c77c9b237212457558f1b6389c91268e4548293f7f608d368a4dce4d45d3ea36d8a1b969fafc1c66699b5f05ee4c70bb185d12268cbcefa387', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 150, '2026-11-24T20:00:41.951097447Z', '7ec775fb52033d27', 'legal_read', '\xa22523320d9b59aa6be2c7959eabf5c8062930b125d70262cd6e00088b570394', '\x01f8920f89859ed8d013224618ddaa5d57bef504b1e76d7b78783f0677914b7474a0ba1d6ce77883103b16d0998924891982c3eae48c2c17055fd6a65f0eb21a76', 'de1c779bcf984040d01fde40628c2da1', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 152, '2026-11-24T20:00:41.954412219Z', '7ec775fb52033d27', 'legal_versions', '\x1c861a3b69c28424e574f021a311824ced3815fa40b941e0d47e14b6114cb2b9', '\x01947e5151451620d58acd57d34fcb1f0c473c7b1adad6b3eac3cae3b0d50fd9086a736878d8a1262ebb0422d59e3dac5588c119e05b2c9f3f51bba2d8dd98c20c', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 154, '2026-11-24T20:00:41.958683894Z', '7ec775fb52033d27', 'legal_read', '\x1c861a3b69c28424e574f021a311824ced3815fa40b941e0d47e14b6114cb2b9', '\x016ca25de141d4952241018ac8f6c08488e7d7cbb57066a90a0e88902369865d0dfac03bf653931538cee5187f8482de9de8a962b3bfcc4dd2de658f631f65005e', '30087fb40c0b95be82c8ad0930e8ea42', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 156, '2026-11-24T20:00:41.961898851Z', '7ec775fb52033d27', 'legal_versions', '\xb2fdc45777f7027cd6bb6350890109f102c2c29563b733c4afd8bf2e996e7855', '\x01f28c9f62a9ee97280d758e271a9996390ce96b3b20de00aa017ddab3dd968f7a94839eab517e33fb09b443c753b3f82e2df71bed95a88b65db7d57e46bef492b', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 158, '2026-11-24T20:00:41.965862334Z', '7ec775fb52033d27', 'legal_read', '\xb2fdc45777f7027cd6bb6350890109f102c2c29563b733c4afd8bf2e996e7855', '\x0185c4dd2905cc17617728bd3784344b6e46ec495177e09a58690c73b69f92f01718454376d5a77f8d98e5023ec21a04d1fb7bc632677a5379fed1940ae0496c64', 'cb1b1614839435819e7fcf03536e7c47', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 160, '2026-11-24T20:00:41.968848697Z', '7ec775fb52033d27', 'legal_versions', '\x20335bc6984e9895ea0d9d507994983cf0c193cc2e2053ecdad287f4e3237bb8', '\x014b82df636ea1b110f9a252578010643a3c24bf58e2e2b253439c1b5964cf6815f2368e9cbe4c6b7e779a6b00e7136aa86f0fbc3ef6e0a4dddbaafbe7f5453f83', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 162, '2026-11-24T20:00:41.972826319Z', '7ec775fb52033d27', 'legal_read', '\x20335bc6984e9895ea0d9d507994983cf0c193cc2e2053ecdad287f4e3237bb8', '\x011b81d819d388b019114cc2971a2c474107ed5728cf8381ae8bf2b4d14440030220f70f458c48bb52f642c1edbdc3f05b6d6ef451c80dd35224e12dd91555670d', '7c7ba535414e8c0cfea23b0999ab3dd1', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 164, '2026-11-24T20:00:41.97581822Z', '7ec775fb52033d27', 'legal_versions', '\xd82da7b1f94119318f8e9374d6c4df3f8e439a8bbcbc9f970d6303b003fd2fe2', '\x019d3bc6a587deaddce8b0b5099450b1f4d043cd2d44bcce7c60f6d1a339fd208e7881a03a979ffd958c4489e7c5ce1a0aecc9aad5bf1154068606c77a738cda6f', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 166, '2026-11-24T20:00:41.979870128Z', '7ec775fb52033d27', 'legal_read', '\xd82da7b1f94119318f8e9374d6c4df3f8e439a8bbcbc9f970d6303b003fd2fe2', '\x019c24ab80b87491a563e61902fa4a598112e878422dafbe50917bc7749b0375bd507282259004523192cd21de12b715bcdf970097025e21d90d714b0c606c7797', '281cc5c21dcb717c91b68ffe6b49c8cc', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 168, '2026-11-24T20:00:46.786892385Z', 'f855fa086da6962e', 'create', '\xecd545e554345a315aaecb1d82cb40baeb4c6d67ae994a08976cfd4ab52ab061', '\x01f90999f57584d10e69ec4b3d02ea5006ffb2d40d2c857a5c15b69c5af34a56e46d12e61586c43c6854fc42f7254d99fda108bcba55f92737b18b40854f95ee78', '8efe4e0c4d336e7ffb1b4adb5de646fb', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 170, '2026-11-24T20:00:46.832276616Z', 'f855fa086da6962e', 'create', '\x1871c66d22389bb1fe0281d69db8bcb2c29122c24a82cbaf584e3ba0b240a085', '\x01c793baa62aa7c980f4c956f6a2a4496ef5eb2caba85560c03a0cebdd4b6786aa76b43ca5e8a37876f82fa9893c034cf7aa2604ccea2376ba08bf1dee728d19c9', 'dc1ddd0155816ed9c335074480b77eb4', 201, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 172, '2026-11-24T20:00:46.878819269Z', '7ec775fb52033d27', 'create', '\x6879c64a083678f2efb3135b3be90353966e71888a24ea3542c098182709e882', '\x0134ae8bf6d20a7c5a0c295624e221f5dc1c6eee55d6a5072573a1eccdf3ab66526e71c59e102e13ade739e4a954cf1bc8fb86ba642b2ea192218b929a8c38ad60', '3b064bd47cb93530023d12724fc3124e', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 174, '2026-11-24T20:00:46.925190396Z', '7ec775fb52033d27', 'create', '\x9daf635ce8bb2136c3659eeeffd36cfa94b26ec843ef40060d71a9fd4a14330e', '\x019c2d0217aad00ada82b30acb90683cff9d411f56cff2f7ec409bb1f605445183ef9d0fe2f2286de21b082f79d7b75fb315d217429ee7be074d200886f86f3269', '92637ffc3dd17458826ce689b6121094', 201, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 176, '2026-11-24T20:00:46.928906063Z', 'f855fa086da6962e', 'legal_versions', '\xb02af1eb2deae367f1ec47ffa02722e1877f9813316e9a804521c06a826e5a11', '\x01703a38f16a89c95bd25c5f72f3b964679fc80b507638ba24f3ed2b54ea0ac83ca445818aeba6b3c93e9fc5e81c574820312aafef01000779e18d634bc1b26d9f', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 178, '2026-11-24T20:00:46.935028949Z', 'f855fa086da6962e', 'legal_read', '\xb02af1eb2deae367f1ec47ffa02722e1877f9813316e9a804521c06a826e5a11', '\x0158333371253ae56d082704b4c640416b9232ad76da8b37333a3a4bc0bb44eff4e67f5173cc280677796fd26d8e7b43e2512b8a63f2c68c1414d668c0109ca77f', '64af7b33f83ae3c4734fdfc0b9bbf389', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 180, '2026-11-24T20:00:46.939720976Z', 'f855fa086da6962e', 'legal_read', '\xb02af1eb2deae367f1ec47ffa02722e1877f9813316e9a804521c06a826e5a11', '\x01c042ba6db0fadaf191149e5626becbea38cf50ff7acd7a1c584489214f16c91dc5bf69008060ef0478253e24b23d6d8972cf981ba35ce95e98c1bd7aa09e8328', 'd5619da83c3a2949c4e288134f74503e', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 182, '2026-11-24T20:00:46.943227052Z', 'f855fa086da6962e', 'legal_versions', '\x7b64b370b22afa8dfca9b57ba3fbee930b135aed632562b1ea36c010a9d658f6', '\x01dd50498c739954e43e05050335b1ac77260708e66ab11f5b29ecd4d82066f5658d50a2d8e61d8fba62a4806bddf36a4f4a348097f313f7744d1db033fd6f3832', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 184, '2026-11-24T20:00:46.948144005Z', 'f855fa086da6962e', 'legal_read', '\x7b64b370b22afa8dfca9b57ba3fbee930b135aed632562b1ea36c010a9d658f6', '\x01b127fbec9fc4a74f1577776c9f8d8caadc98ba5d2a813dbfc8cc5a557025f7261c4898457ad10b62ad5143ddaf29373e3b060af353f11b73330229fac460327e', 'b082c48b7d8614e28c0e7e8d02cc6ad3', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 186, '2026-11-24T20:00:46.952545931Z', 'f855fa086da6962e', 'legal_read', '\x7b64b370b22afa8dfca9b57ba3fbee930b135aed632562b1ea36c010a9d658f6', '\x015a2ca655511bbb845ef90b33a4f44b1ab8a72611abd602ed77ea1cb9d3817fe8a8c5f6f3d2153b46fbe798cf5e982e3bfc3c9ef0d816d48ad4ef54df05e40ab5', '6653e7d4d8cf3ca57ca05a3c461d480a', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 188, '2026-11-24T20:00:46.95595172Z', 'f855fa086da6962e', 'legal_versions', '\x1043ae9e8c2596792f11197b429129fdcf25029797701779e758a33abd0380c3', '\x01959797d89db0ca513cf8b1f7ec2cb9fbcbaa723921489d71ed0421b815f63bc5716c57b8cfac1e1a10935549d1d3488fae572473ca7341d623d1015a81e54d36', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 190, '2026-11-24T20:00:46.960320526Z', 'f855fa086da6962e', 'legal_read', '\x1043ae9e8c2596792f11197b429129fdcf25029797701779e758a33abd0380c3', '\x01e0acd967aa1ee507180a64a8629a8b9095bfb747b49d0260f1c5d316f455f39bcdf03415472b0c1c7113d02e4310624e2731d917e21cf8c1a38486c6dc06246e', '1c7c2cca79fdfafd0b4dec61dc3bf337', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 192, '2026-11-24T20:00:46.963652167Z', 'f855fa086da6962e', 'legal_versions', '\x4f0ce6fc13ce195efabaa51636a4cb32b08815e7ce36751228f98b1854b65232', '\x018926544de592c879217b88c1fc3f438629bfb03b2ebf76db2cc21ac1022ae020257c020111208365496af662b3e152114e30ebb37be45147fd7e96e5b0ffb61e', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 194, '2026-11-24T20:00:46.968189333Z', 'f855fa086da6962e', 'legal_read', '\x4f0ce6fc13ce195efabaa51636a4cb32b08815e7ce36751228f98b1854b65232', '\x01f38fdd9556da17253c07557ab32568f66647ae0a6f2480582f5c9ed4564744a935f25587fd2550583b244435b9844ad2afccdf56c8e885c73a91deab134783d4', '3dd96fc88a9f8ef66dbef837a3c3cd15', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 196, '2026-11-24T20:00:46.971909687Z', 'f855fa086da6962e', 'legal_versions', '\x29b5cae40c6f00a1174991a837fe1c5809b5e39be9ba02828292cbad0df34ec4', '\x0147bf84a763ecb0e9312606d04adeffd95331949fce07b7f3b32801d97e216d82cc3151fbe9bc0f5b3b09a4f371b6310c786f9598e65b8d69bfde9a587fd188f4', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 198, '2026-11-24T20:00:46.97617102Z', 'f855fa086da6962e', 'legal_read', '\x29b5cae40c6f00a1174991a837fe1c5809b5e39be9ba02828292cbad0df34ec4', '\x014fdaf7f5393f556a62c78f8c2879393af6ce1a1afb1a003bbbebcc8bd6c50518440f79ad379e6a5eefa0e73eaaafc8b0e166c26e50c797c61b102cb03567f454', 'ae808ac83a2dcd2710a82a53afa8001c', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 200, '2026-11-24T20:00:46.979447634Z', 'f855fa086da6962e', 'legal_versions', '\x1eb719b9815a3f3151eb00eca660c4338ab4397516c51e63f74a164a1f55b862', '\x01f8a105895a2d99b315270c312138050580f609c8b3c911a4c7b22c1248dbc9e52b2ea0e438c175e876638431b2be31f6d7ac3759db7cbf8eaa794c3dfd94f88c', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 202, '2026-11-24T20:00:46.98376718Z', 'f855fa086da6962e', 'legal_read', '\x1eb719b9815a3f3151eb00eca660c4338ab4397516c51e63f74a164a1f55b862', '\x01e2d801451431c30c8ebe6cae2e0ad8f5c3427cf4868aeaaace6d756388e6a36ec9e5a2a9f325795e74dc2e5444176a3b2965d6ab00ea49965ef0fa753638f928', '7dc908a7c733fa3625996afe0ed56982', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 204, '2026-11-24T20:00:46.987195571Z', 'f855fa086da6962e', 'legal_versions', '\xdb72985900c51f43734feaa605385d6bf6708f975e89ea771a633454f6f6ed43', '\x0119f13c43e72a004112476e0bd16dd563dab335bc9d217435ce964c63914f02134316f47b7b9edf233290d74b4330a73e902240dfa1ceb10308fb42e3d2a6d648', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 206, '2026-11-24T20:00:46.991588544Z', 'f855fa086da6962e', 'legal_read', '\xdb72985900c51f43734feaa605385d6bf6708f975e89ea771a633454f6f6ed43', '\x017d5e1fdfb0b82b414f102a4e987ea50ead990e370741d58e1d1f433e20f5904a39817ea9f19bfb5a3d604b9b163987086c27c9a4c9e0cffa361cd8bd6bcbf3ee', 'e42131b6b82713b087dd1bb3d98fa065', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 208, '2026-11-24T20:00:46.99482587Z', 'f855fa086da6962e', 'legal_versions', '\xce16ff00483e8388e019be3cae3b5b2d702fd2d1b92c943dc86647b5882dc761', '\x011bcecedb8cc582f9aa1735b5e2ca70dfcdd652abe3299964b5a00a5a91b4343a4610a9f46f4dc8472b46651c749ee86bbef2b3ca467650e5976430d4bac9787b', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 210, '2026-11-24T20:00:46.999298573Z', 'f855fa086da6962e', 'legal_read', '\xce16ff00483e8388e019be3cae3b5b2d702fd2d1b92c943dc86647b5882dc761', '\x01d71ff8835e1abedee7e99724e11b9ec944a057fd3353129ed15f9a86fd790a0c133518e4d5c86d6a1c3afd0373bc42a674322d84c87aaef4b91057fbee7e63a6', '583f4a9a96eb50ef61b2691b995b3786', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 212, '2026-11-24T20:00:47.002850279Z', 'f855fa086da6962e', 'legal_versions', '\x5c360d883e82c9b26400cf00ef97a4b2ce0f53c004238c40907ff57e5912dacd', '\x016c32e204c104940ed882aff778b3c664d67e5d5c14e289aba9c8f9409b59d55eff881ecbfe364e7295eb2ad420ec8b4c46f6abb51e0769486b026c2b98e56d07', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 214, '2026-11-24T20:00:47.007195714Z', 'f855fa086da6962e', 'legal_read', '\x5c360d883e82c9b26400cf00ef97a4b2ce0f53c004238c40907ff57e5912dacd', '\x0196aab5b49804c06cb0b1a9e09f2cc483dcec8b81c227bbc1f44dd7f4bca2f9472f1a2959a5c10958bf0b90a91ee203a89b0376c737b609fa9c18ee5ef2c6d19e', 'e4e21918b2d808a1914fb337276a2ed9', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 216, '2026-11-24T20:00:47.010484892Z', 'f855fa086da6962e', 'legal_versions', '\x31a158c963db909b0e7a555d361cd3d03fc63dbc1342e570ccdb4d87eff1823a', '\x0131d75f5f97ad90e2fd71432fcae036b409dbcc803569c76a8844be672195ddd61cdc16c0be44bb6a31210fc277528050968af5ff3bafd6fdacb11d0ea7433305', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 218, '2026-11-24T20:00:47.015328243Z', 'f855fa086da6962e', 'legal_read', '\x31a158c963db909b0e7a555d361cd3d03fc63dbc1342e570ccdb4d87eff1823a', '\x0176d1b947b48981fe4f3101c55e6c5d9e1fc8bbd32bccc20f02491d28223e8289dd4013926009cf9b9cd4fc6964901ba3ccccee60933073899ec363c26b0e8628', '8c05b3582f993d8f7e29a7ecb9f33ade', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 220, '2026-11-24T20:00:47.018777588Z', 'f855fa086da6962e', 'legal_versions', '\xecd545e554345a315aaecb1d82cb40baeb4c6d67ae994a08976cfd4ab52ab061', '\x012f69b8841db85a52ff2c3ba77409e2e8532e6fa1e361a07a2acce79df40f4a6dd47d1adfc119b4141e18893e2cfc12429c41f6274c9f6b399c2d51f22648d48c', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 222, '2026-11-24T20:00:47.024465974Z', 'f855fa086da6962e', 'legal_read', '\xecd545e554345a315aaecb1d82cb40baeb4c6d67ae994a08976cfd4ab52ab061', '\x01603e4039fb28f385c2abe4a2f06dc858d0092f017bcd6efbf748fbcd2dd42461cf1230173d9c66d1ec98c897ebf60a732ae4019a34ca84dd1a648533cb90ffa5', '8efe4e0c4d336e7ffb1b4adb5de646fb', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 224, '2026-11-24T20:00:47.027832921Z', 'f855fa086da6962e', 'legal_versions', '\x1871c66d22389bb1fe0281d69db8bcb2c29122c24a82cbaf584e3ba0b240a085', '\x012226a0da663b388fcf577b06b5be00b005baf5f49d4d662630d49de52e2259425f2a0561a5107f3c49b9014b07eabc959f69aba33a3596415f593e78c07a8f4d', NULL, 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 226, '2026-11-24T20:00:47.032320764Z', 'f855fa086da6962e', 'legal_read', '\x1871c66d22389bb1fe0281d69db8bcb2c29122c24a82cbaf584e3ba0b240a085', '\x01809a571b167886443176bb62c7f00a2961bfe1941e8b5ffeba500d605a44bbebb33f695eb3090532fa50e50c9bbda19b95c82b4a3ad3eaa23a521f4fd807d766', 'dc1ddd0155816ed9c335074480b77eb4', 200, 1, false, '\xd3bc6e4a439dfdfa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 228, '2026-11-24T20:00:47.035701349Z', '7ec775fb52033d27', 'legal_versions', '\xdfb0cc740972e6b89cecd1938b750fa20611c6180792860fd03e9c09bb9ed94c', '\x01bc057e1f28cf0ad2452e5116371d264a2ea0fe764eed506e627650e17517768b2b6b93683d77bd5d3c6e45a028b41853282afa8cb39a0f09fe89235c641a9b52', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 230, '2026-11-24T20:00:47.041075607Z', '7ec775fb52033d27', 'legal_read', '\xdfb0cc740972e6b89cecd1938b750fa20611c6180792860fd03e9c09bb9ed94c', '\x01043e718405e77d78d19e1aa5b4874f229bcd30a30fb060a8c6ab2d6f8d400a12110919b0629c015729ca55d77b4a83666fbc03102ee606a24e8b01f8415454be', '67ab3b32300d8573514562e5305b4f4e', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 232, '2026-11-24T20:00:47.044165479Z', '7ec775fb52033d27', 'legal_versions', '\x652de9a8cc9cb613772fc0d875f252724ed1a4c44c855bdd10a86ff6b1337525', '\x010b6066fafc631080bc24a8bbd1336521fbc82d84c8e1a9e8d6311718516444080ad07e9543cae7add26567c0c844480abd9a4b6912349e1cd663ebda2e6f3814', NULL, 410, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 234, '2026-11-24T20:00:47.047564305Z', '7ec775fb52033d27', 'legal_versions', '\x0d66c4314e38f040916a6f335d1fa3485c35a898acfb3d1b5fd76cc0261de15d', '\x0125269cfbd64e19b1126ec3522bb49205905c01e52799d15100f341238fa5e97b35a4807332934e3b510945d23f25bd514bee194a098ee98f73223501652dce21', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 236, '2026-11-24T20:00:47.052045602Z', '7ec775fb52033d27', 'legal_read', '\x0d66c4314e38f040916a6f335d1fa3485c35a898acfb3d1b5fd76cc0261de15d', '\x01b3c7843c9ea01e0d8cc63d103b91732b6dd953978fc1238ca4df6bc5899caff1606233ad41bc4a4e8e070f5b7a4e79981078516532f3dd49f2e4837a1e40d209', '2dc7186ecd178b4705e33cfd33937f59', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 238, '2026-11-24T20:00:47.055383493Z', '7ec775fb52033d27', 'legal_versions', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x0152cac206ba6637c1f00a346e54b6b639c06c4832a9d1d8ee0b0204a31f5885efd5d63a4d99bae2da8e18a446f45a850bf3bc3a4e1af56fc331e738f21b016311', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 240, '2026-11-24T20:00:47.059959409Z', '7ec775fb52033d27', 'legal_read', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x016cd8a84dfda1614af98bd524b2bdde2e82d2c1a4af5a050fd6ccb1a71d43c12f64d7e8119c5bc3fd0418c328d5ce2e86176c4b53910665b184b98c60280f2688', 'c1bb35e1973fca8cd4bbb5dfd8c5e8fc', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 242, '2026-11-24T20:00:47.064281215Z', '7ec775fb52033d27', 'legal_read', '\x63fbafb82d1c97ef8511e205301797132ab23170820951da13ada1c626896256', '\x01b5c60cd44aa61f30b8af1745d547c9501cb2173366de55c05a3dd827031f83f2d61c1f13cdce3b6d09215c29bccb0f046c514074d24a8d62b8520259a2e6e025', '019ceed85254aa1abdb52fe1129a8307', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 244, '2026-11-24T20:00:47.067585745Z', '7ec775fb52033d27', 'legal_versions', '\x41b468bafc0022dd83f082385b85df01ddd7913b74d374980bf5ec5c4758643a', '\x0107d16fdec20af91f7405bcf45e653727e1c8a9b0433b93bb2fbc59b5f7ce6c2f71c475dd5601f5e5938bee502cd53f10991fbae9ca466640c88c3d4dd1c3177b', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 246, '2026-11-24T20:00:47.071935235Z', '7ec775fb52033d27', 'legal_read', '\x41b468bafc0022dd83f082385b85df01ddd7913b74d374980bf5ec5c4758643a', '\x01a7cd6ce36ba401be837b062f7a96abc4a5fba43ac1f5de7edae67e1f903583b86509440aa4325c6983511ec09f10a62da6be08c454b66176e591aa2ac37cd24c', '41c0dc030c50c7dd08dd49826f04822d', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 248, '2026-11-24T20:00:47.075464728Z', '7ec775fb52033d27', 'legal_versions', '\xa22523320d9b59aa6be2c7959eabf5c8062930b125d70262cd6e00088b570394', '\x01ea292d3ccdad02fa2ae20bec2e718c81ae11a92d0d51d5e68ff5bdbbe1c1eec10cdc2887a2a1e6289558722d85b416b8341e1605387746b07d57a61e6f73bb40', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 250, '2026-11-24T20:00:47.079741673Z', '7ec775fb52033d27', 'legal_read', '\xa22523320d9b59aa6be2c7959eabf5c8062930b125d70262cd6e00088b570394', '\x016bf607408e9ad31083505ee046cc6caf5cfcc6612f32e7e7ca7f2a21923b10fa7c538b7024e9fa74ef1f9624a83d9a44132be9887fff2b86ab497a690f314564', 'de1c779bcf984040d01fde40628c2da1', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 252, '2026-11-24T20:00:47.083029323Z', '7ec775fb52033d27', 'legal_versions', '\x1c861a3b69c28424e574f021a311824ced3815fa40b941e0d47e14b6114cb2b9', '\x019c1855b4c08e281edd7a84424c2d02366a556e6b38d7c3ef2cba2c39605d5fa62fa3cefb17f9660873ac662a2f5fafea8c78d87993c21716c9ce4eccb2d2b711', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 254, '2026-11-24T20:00:47.087289157Z', '7ec775fb52033d27', 'legal_read', '\x1c861a3b69c28424e574f021a311824ced3815fa40b941e0d47e14b6114cb2b9', '\x014a0d1c8a3d8a24928ef3556c1203e3cdad6bd568a3c4787660c1968fcfb84a5f6bbfa12794b5b7a72bd11795960e494cd5e07a0b4f47febd4cf3f44da707b378', '30087fb40c0b95be82c8ad0930e8ea42', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 256, '2026-11-24T20:00:47.090496225Z', '7ec775fb52033d27', 'legal_versions', '\xb2fdc45777f7027cd6bb6350890109f102c2c29563b733c4afd8bf2e996e7855', '\x01c06f2efea529e8d1da342f6e73eb581bbf442ad32cba852e78af1b6e34c7c20aea6d85e24b9a5b005ca2dc25274bf7ab5299743dd2f20256bdca5e8235a409f6', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 258, '2026-11-24T20:00:47.094532929Z', '7ec775fb52033d27', 'legal_read', '\xb2fdc45777f7027cd6bb6350890109f102c2c29563b733c4afd8bf2e996e7855', '\x013064a8e6688586b7fe907a437768329fca521e8fe2981ab057649649efb53f5360d30d8c7f87be543e3b983048867a276e6f0fee34fd21eeb82cc39ec9239ba3', 'cb1b1614839435819e7fcf03536e7c47', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 260, '2026-11-24T20:00:47.097808931Z', '7ec775fb52033d27', 'legal_versions', '\x20335bc6984e9895ea0d9d507994983cf0c193cc2e2053ecdad287f4e3237bb8', '\x0138dbde7a2ba89048519d5c985496525a5d98f534a10f286a31962aa5133798ddce0de843b60141fb0c92f07b4c86a4832fef12af8785f68cd4c19a9075e8f060', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 262, '2026-11-24T20:00:47.101987506Z', '7ec775fb52033d27', 'legal_read', '\x20335bc6984e9895ea0d9d507994983cf0c193cc2e2053ecdad287f4e3237bb8', '\x01088714bd3ae64be55ceab134b160666813f9d7fcdd2f8ef169af4ad47011250e11225524013cfae2a263d6c1c57b8cd5ce203848339b6350be68ab1569e87255', '7c7ba535414e8c0cfea23b0999ab3dd1', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 264, '2026-11-24T20:00:47.105160185Z', '7ec775fb52033d27', 'legal_versions', '\xd82da7b1f94119318f8e9374d6c4df3f8e439a8bbcbc9f970d6303b003fd2fe2', '\x01397f1a32fdce6967bf260e9b9790a6d41f3dc95ea9e1672957740ad742b348f4ace3db29491a4513d3f3dbbd27a2ff051b8bb1f86d0c5b092b1ac2a3ed718af0', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 266, '2026-11-24T20:00:47.109231121Z', '7ec775fb52033d27', 'legal_read', '\xd82da7b1f94119318f8e9374d6c4df3f8e439a8bbcbc9f970d6303b003fd2fe2', '\x016c5705395e2c714d38e2c95f1909cdc7fcd1ba097e0c44cb2b82e0b0f1917bd674b43684f89548a21de3d75f49e3f877b7eedf237c3b709b3d5a3886ce8c1fb9', '281cc5c21dcb717c91b68ffe6b49c8cc', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 268, '2026-11-24T20:00:47.112304614Z', '7ec775fb52033d27', 'legal_versions', '\x6879c64a083678f2efb3135b3be90353966e71888a24ea3542c098182709e882', '\x01f273922e8298456b55f6550d4fed5bad3935e57a703edff2dcba4539f1b0f50f1d93eed67006672bae27267132aa06d278c0167cd32a9b53c304615c4c1a122e', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 270, '2026-11-24T20:00:47.116526948Z', '7ec775fb52033d27', 'legal_read', '\x6879c64a083678f2efb3135b3be90353966e71888a24ea3542c098182709e882', '\x01fe8826137ea88f0e93f1ee74f7e34651bc0b6ac0cab363c04cca41869754b48577287398d3bc72e9ad31a3a1d65ffa1090f8265059d17c30b95c5360d9324fad', '3b064bd47cb93530023d12724fc3124e', 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 272, '2026-11-24T20:00:47.119552904Z', '7ec775fb52033d27', 'legal_versions', '\x9daf635ce8bb2136c3659eeeffd36cfa94b26ec843ef40060d71a9fd4a14330e', '\x0108dbfe4304d5d48f03c78cb68d40bc4c76ccf3d5f0c9d50bf5212a9b62a069b34ad12a85c6a8fa409a536148d65a3d04fcd053dc39feff54f2f39db42b823423', NULL, 200, 1, false, '\x3c417b0825f4c40e');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 274, '2026-11-24T20:00:47.123689748Z', '7ec775fb52033d27', 'legal_read', '\x9daf635ce8bb2136c3659eeeffd36cfa94b26ec843ef40060d71a9fd4a14330e', '\x01190f1465ee71130507bf655055fd2d56dcca1a3942c288e6aa505e11b1b17a8e400009b3d7c244311d2258e4cf6301fc21bdf1913e6f9aaa7149d546c117c02e', '92637ffc3dd17458826ce689b6121094', 200, 1, false, '\x3c417b0825f4c40e');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xd3bc6e4a439dfdfa', 1, '\x018990c82b1cac1dc19a5448d4acc7f973a2d604cf3d8986b86c31fa1605bfec82b8c66bed668a99e340edb58fa2383f814c55f80539219c3d4f1dc916');
INSERT INTO vcon.audit_key VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x3c417b0825f4c40e', 1, '\x01999106032ae2e338cea6a1b6934ea95acab6fe383f08c9eebee278d5f9786f4599755a7daa825b065dec5ce8eaab3fed6173442575278ac2c24a9086');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x59328370962670e2', 14);
INSERT INTO vcon.key_use VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x00d58438596f6406', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', 2, 1795550445393907883);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', 4, 1795550446691330461);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x83ec96c80fa25e81', 2, 1795550415295728327);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x45464dde3a0c370fd6ff30b368ac6609', '\x0ccba7d5b806a9ee6cce897b8988302a', '\x0150d66f4ad27cf0ff5beb59d550e90b7b368ecb678364d1d2d116f5693f154cbf5673', 1795550433563100623);
INSERT INTO vcon.membership VALUES ('\xd2df296e3888fe4308d1260cad4235e7', '\x447b2eace76a8db85ca208b6cf0c5c13', '\x0183b64f5fffa2c82101b7fefe11863a13099277080624a08b2a03fdea81b180a31e1761', 1795550435475041872);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xa6fc78a5cd3a21ecb0194f4415bca7db', '2026-11-24', 12, 12, 7401, 1795550444389448511, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x8367f3fa4d36079483e85d21141b4f60', '2026-11-24', 10, 10, 6237, 1795550445482798663, NULL);


--
-- Data for Name: pending_write; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x92e21c7efe3ca627');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-24 20:00:13.315882+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-24 20:00:13.349963+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-24 20:00:13.355858+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-24 20:00:13.356392+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-24 20:00:13.358457+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-24 20:00:13.361496+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-24 20:00:13.36283+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-24 20:00:13.367385+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-24 20:00:13.369871+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-24 20:00:13.370391+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-24 20:00:13.371472+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-24 20:00:13.375154+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-24 20:00:13.380537+00');
INSERT INTO vcon.schema_version VALUES (14, '2026-11-24 20:00:13.383245+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '354f546774be375d29d5301901ef679a');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x61b6d7fbac785aa4ad58597ff021f03395b1fd211390445b86e4d9a81e03837f', '\x018b8b7c9293908070b1a8c4543767e22ae81182227f5def6eebff151b831219cbc8a7a692', NULL, NULL, 1795550415307293080, '\x59328370962670e2', '\x013ee73631a39b280295d93c293caa1bd2b971502593ed5b34829ef4174e8e80368fefc53913e2d35bed88b7ddd644105050183b188e72f31ba657ef20', NULL, 1795550415307293080, NULL);
INSERT INTO vcon.tenant VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xe6b2ecddcdcf91d74c9f7743260cab66e2f327036d10af55a8ed3dca3618160d', '\x0175c04451151ad68b0fa5c4bffdd8798d441d5718fb363ab0714ffc4abda410255365609a', '\x01ae3558a6e28d4a6028c380ce4099a0360af559288a328f1ff2f08d7ed491', NULL, 1795550420830893862, '\x00d58438596f6406', '\x01628f34e929aaeaecf567c4e62fa8817a6deed843d54475cc73fbdb305c1bac5c7263fa0cb5e40b6385681a4e57e3188567c8a22f82de726b5163c913', NULL, 1795550420830893862, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x0a874f6eb7426a1d');
INSERT INTO vcon.tenant_catalog VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x797485bcc5842116');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', false);
INSERT INTO vcon.tenant_lock VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', false);


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x6ab36a42073d50972b2ecc793d19dcb10144b219939ef526d092a9074c34d01f', '\x0134a4ca4df26e77508ca95486cc17ee75d060b11dd9a77313b3a9acd3eb5ee01ef693e53dd6168585b36350a4', 1795550438867452647);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x45464dde3a0c370fd6ff30b368ac6609', '\x01698455166578e437713d471f735bd0003371d417a88254892f7dbabd5ceea0f9c870ee7868bb47f15cae', NULL, NULL, 1795550429174354763, NULL);
INSERT INTO vcon.user_account VALUES ('\xd2df296e3888fe4308d1260cad4235e7', '\x01b6a015c696842efda76d02259f740244d16158a47745d5564eac74ec29e62c30358cdf6af101654e172504', '\xd330c320b265c53a0afec606670e9667cc4f6238c85f7717e14266cd79ca8632', '\x01a798a3b32ff10859025f0d926653407a5726fabd7d4d2a682ea967d2f42da39f0da5c1cdd268a496ad2a8ab0dccc182db804c4e6e858c6f2306b0ec3fc1158', 1795550431561103863, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xd82a12ff3c7b365fdb193815bf8b08ed16de007f730eeb3a5aaaec537f6d5f87', '\x01dd1b9e30ce0bd63181c84466cbc7bd5dbc47818cc6a554b0fec4dd2a65d16a0b97e99e7055c8654155c96c68', '\xe42131b6b82713b087dd1bb3d98fa065', NULL, 0, 1795550438171579583);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x5a351fc7a87edb1d49cca2fac077a64bb01bbc2bcf3be0d846fa83dd2fea88c2', '\x01637f507385f093726ca707486c1b70872484e3415ba05269f6a755f04d4be0788f1f044f595654eedc50c3a4', '\x583f4a9a96eb50ef61b2691b995b3786', NULL, 0, 1795550438223225844);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xd4e6d9b4ab79dbc3454648fb6ae1a5f5b033b624b9568aa752adc10fd1951940', '\x014f22146b867138d6a1a23225863c44c6281ca56c39bfeee2f8c11fc3e388eea8dafa62fa1002f92be383193c', '\xe4e21918b2d808a1914fb337276a2ed9', NULL, 0, 1795550438275293854);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x9f3c92314c2278fe893969a1e76d21d833d3794ce441ed1329214670085974cc', '\x013ce452edafb66bebf31430561a1024edc483a65046a27d751a2fefad4acdc68ddad039d233620ac2b97004e3', '\x8c05b3582f993d8f7e29a7ecb9f33ade', NULL, 0, 1795550438312684551);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x7abbb11f7b965e21c1be8e9e10c049bd0ef503bfaca8fe9f83dcbcdf40b6f831', '\x0167b075aef599133399e402f435e157bed745f0d15ddc1082ca4060e46c7b7d8b5e36b699d70a8f1736346001', '\xd5619da83c3a2949c4e288134f74503e', NULL, 0, 1795550437861390136);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xa04e4f775b4f15cf6bd69c8922c95581bf88e1827881129d7feeaff97cb22aea', '\x01051943088dd1e18f813fa9c69b35d2d24873d162cc0bdd8cf92e65065577fbc23063247801641fa9add06536', '\x6653e7d4d8cf3ca57ca05a3c461d480a', NULL, 0, 1795550437907874855);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x709fb0c359368be1fa381aaf4b16ef96d4034f2ece2fd8f7be3bd76210172aa8', '\x015c83f53a9716dd44f277130ca427a476c8ac853d6b2a73b28c7b5686c24484e8b82f0f41f75725f1e7540af9', NULL, 1795550438400571484, 0, 1795550437951243661);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x123ef0f31c58b7de2a36543a109304d020fa652b436fa88d944d98e61ae53c80', '\x019d9918d417dea8090e22901a91bee64c241bf6a6e44bf42fc233e89f8a147143afb3c0b962a6066c65265007', NULL, 1795550438408276505, 0, 1795550438005822362);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x981971407a8c5c6c3e6644544808b4ad24f42d415d2c4395058b45762c895e3c', '\x0150c4c966cac1e830c9fd85ef351f899c89f8caff1785caf524bea3fb03e3f8e68f95c8be8c75b9dcef88ecb8', NULL, 1795550438418760659, 1, 1795550438055331313);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x1f446f1fa588767560ddbde669409a51b166ccaa8c5eba390ec8230e86b48f7b', '\x01645c2f06012982bf7f02deea8af131fb70baf9646bb98f84d77898e437454f6bb21b80d5f7348e8f6ce5d36a', '\x7dc908a7c733fa3625996afe0ed56982', NULL, 1, 1795550438108079709);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x0b7203b8b160f93a38b8842aaa6eec66ce32a992b8bb61262055fc5e796261f2', '\x010fc1f06d8dba7f2b81f3a8cb5f12b905fe8315f362aa6380f53d9782324108d2f7325ae9cfa84c7647ac964a', '\x41c0dc030c50c7dd08dd49826f04822d', NULL, 0, 1795550438644500715);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xb732181b4af7318cd4787d441428b3cfc75ec18cc2b9d693a9725b9a5e5d86f6', '\x0130f730cdcb09996349ce6d1ed590448e3322ad5b5f8c086be22a0e1dd88b9e343fd97389aa0f558a0efaaef6', '\xde1c779bcf984040d01fde40628c2da1', NULL, 0, 1795550438691895022);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xdcbc7ca987348b21010f45cd33227ab2ed3da66b64213f119dcf994e9162002b', '\x01f5f17fcd9e630605422ad4ffbafb528aa25cd664b0f40d3353fe86a3811e0914b694783cd95aace5d6226974', '\x30087fb40c0b95be82c8ad0930e8ea42', NULL, 0, 1795550438713462540);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x6bfd868abe28a4789c7d896b7064299c159cff738459f9bdbc6c2a3e819b7289', '\x015c7e02785f73ff23b6e069c2c5955f43ea5194df6e90d80b73d357d418f55004ad96d7c9ff2736a31ea39ac7', '\xcb1b1614839435819e7fcf03536e7c47', NULL, 0, 1795550438731832047);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x1ea9b68dc7ef548077d6b40f838e3b79784ee3f01149fa0f55de54b0f5497390', '\x01d219c7d3c43a9a2a5a4ab39b69f08b43ef9b8760dcbd2c0c43148ea711f4c40b35049a07f41369684978b644', '\x7c7ba535414e8c0cfea23b0999ab3dd1', NULL, 0, 1795550438765065625);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x66f2ee23d3a4f6787e87a57d718c75e7fb8d0d906983654c03265afdc437e567', '\x01f4d8a9681adb39609ca1c910babd5f8cb34c539f05e8731b716cd24eba66743c0e0df90414c92d34869658bc', '\x281cc5c21dcb717c91b68ffe6b49c8cc', NULL, 0, 1795550438812643968);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xba3f9f7a44b2c79a5d29f241be3b2e004f1f7a433a91bbe158d9dce6ddddd4ff', '\x01acbe0ada06b00f6aab7d4466304cf1c6303f3b814b46ad487d0e7099ac66b2b3b4060033f51862c1d754eb29', '\x67ab3b32300d8573514562e5305b4f4e', NULL, 0, 1795550438471656586);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x67ae47e9889570f664cdb6dc94e718063f4fd384953705597cf716486b478f45', '\x017ea520f7fd6de7daa187966718af9c2b507c3c0bbdf7fb6334ee05bd09529ca79d986a8a9c82b8304ff4a820', NULL, 1795550439019322417, 1, 1795550438549781747);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xd24ae8bb8231dd5cdb88910c7c17e06e9a4ec8ddc658f68db40ccb41052e107c', '\x01167f5125d7958ac35c2d1ebd96bd23a640f3da53adfef6570be3e4e248055acef6920fe2d0446bce6ab8c98e', '\x019ceed85254aa1abdb52fe1129a8307', NULL, 1, 1795550438597445295);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xb9128cfbe13eeddaf87c73f728b8b27eea06b796863706d3217486121b8f9770', '\x010dfcab6fc11d054633402ae52827d852bd3b858f082ab8c920113f355c01226577c262369b6c88a5c0b824d9', '\x8efe4e0c4d336e7ffb1b4adb5de646fb', NULL, 0, 1795550446785989525);
INSERT INTO vcon.vcon VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x06e5dda97be00483a9f0a9d61832bcad89c03d41a6c298651bc88b147247fe94', '\x01e12bfb44b7a54cc9eedd862b7732236114f43b2e1c9521f718431d73f2f1ae3aedf27092911a639caf68b947', '\xdc1ddd0155816ed9c335074480b77eb4', NULL, 0, 1795550446831893357);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xbe76750b3f669d511edbabed61f556aa0a4c4876f268e964b167eeaa84a4aeb2', '\x01a1602383371c76f35c1ca09cd3a203e232212883f58fbcfd365c20b43f41ecdc2c6d50c96c7f8670375fc45c', '\x3b064bd47cb93530023d12724fc3124e', NULL, 0, 1795550446878414316);
INSERT INTO vcon.vcon VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x849c626ea0709806c0096ecc6d60c4cd4eb2b098bde39fc0e5c356a9f19c4f8d', '\x013ad89ca3903ad19502b5a248d778aca534d30755efa7b0cf14ad89e48a405b74035c5d3baeff171fa2635a2f', '\x92637ffc3dd17458826ce689b6121094', NULL, 0, 1795550446924828619);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x2dc7186ecd178b4705e33cfd33937f59', '\x67ae47e9889570f664cdb6dc94e718063f4fd384953705597cf716486b478f45', '\x012bdd3bb30e1ca09b1b38bb4860ceee1c16586b389b2f0f2e8f88d2ba3ecfaa4f911b4ee9', '\x015d81765235c06f0f9785da305f5f17d475fe5dbe74883fd872b4df7840', '\x01b947f9023b378feef576c5744bd8711fd84fdab93b8114c4f84c344523cc0875d8997fd8', 1795550438549781747, 1795550439019322417, '\x00d58438596f6406', '\x010e5e707659eccfb7f587975ab58f9160f44d4da2d4aa2fbe6609a509295d49c46b21047dc8e8bd8b512060ea17668274a328fc6b9ed6e05f20d378b3', '\x8367f3fa4d36079483e85d21141b4f60', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xc1bb35e1973fca8cd4bbb5dfd8c5e8fc', '\xd24ae8bb8231dd5cdb88910c7c17e06e9a4ec8ddc658f68db40ccb41052e107c', '\x010eaa31f6a97eb397dcb44b9d48ba70696720bfa5c90b5e67a9630622777081427890f53b', '\x01c090b1032022d6922657a07ceba9b06c791ce37fcf4b7c147ed7453fbd', '\x01578bb56dd06a13dd6899bfab8630cfdf74094e4f61473ee23b35f3d3d1a8dccba9a4056b', 1795550438597445295, 1795550439073518663, '\x00d58438596f6406', '\x0166e366d80000b5bb526dc2f48c38ea33665e766e7f3105517d9a36ba902106a6fead35cb9134e8d5d5461b05b5e9ee903c342e3fe35b627dca9e5858', '\x8367f3fa4d36079483e85d21141b4f60', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x41c0dc030c50c7dd08dd49826f04822d', '\x0b7203b8b160f93a38b8842aaa6eec66ce32a992b8bb61262055fc5e796261f2', '\x0135a94a4e9013726a44f83fdc9966c910ecbab94b2b20268b534f63a35f856e3e841cbf1b', '\x019a45365eb89d66ed2ed7ae2fedc2662afe8e09ecfa5c5e59151f28c891', '\x01440fcd5f6bdb8fe507125f328a99a7fa7752c382725a1032e5a26f3a37e7610ab554fac1', 1795550438644500715, NULL, '\x00d58438596f6406', '\x0143cc146c33cf8d4526c32b6309a7c40b25e77ff787f936fd098c76341f0beb4f98e2b468e88b214f9d82e31e59d106e712bc8d83386db1216be278da', '\x8367f3fa4d36079483e85d21141b4f60', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xde1c779bcf984040d01fde40628c2da1', '\xb732181b4af7318cd4787d441428b3cfc75ec18cc2b9d693a9725b9a5e5d86f6', '\x017b68ebdeaf22e0425c3d35f981b20f5db47e930b8b752ae420858b855bb1e415f9648d54', '\x019c1d2f6ebfc6aa015d56ebb8a7f6959371d9a74d8af37417a0d930c783', '\x01325176494b92452931c6fe876d7c54ac79ad306a74aa8abb2466690a45e9369605ae4161', 1795550438691895022, NULL, '\x00d58438596f6406', '\x01c22d768bfc33f2e8bfca2cc77fb4e9940cfac1ac3a2b43e1c7e20d5db75792b711818840fe1c6b799288e138bb5f0aa362aed0c6873fca36b0fe934f', '\x8367f3fa4d36079483e85d21141b4f60', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x30087fb40c0b95be82c8ad0930e8ea42', '\xdcbc7ca987348b21010f45cd33227ab2ed3da66b64213f119dcf994e9162002b', '\x014351a2b3ebdc758b90e546a8fe31f23557e1d4e72f5aaba922daee2aaacfd6216036d614', '\x01ffe348af52f4c37dd9d3f0292f91ef24532f56b870492ceac0c4c7ff57', '\x01761bb6c9d9e015161b3c273532fdcb39c2a4e51ece8df432454760d5511ccbf372cd8223', 1795550438713462540, NULL, '\x00d58438596f6406', '\x012241a31bcf6cceeecc97afa97ba788a91edf6d90a2904336f857bbf995e17798f88bdb26aa02f55ea61bc9e5d731ef1895237f31a8d366c7458fcd84', '\x8367f3fa4d36079483e85d21141b4f60', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xb082c48b7d8614e28c0e7e8d02cc6ad3', '\xa04e4f775b4f15cf6bd69c8922c95581bf88e1827881129d7feeaff97cb22aea', '\x01761e6c27f4fce9579493ece2178ca7dad7e702132825f30184d5eb082b3ade028bc24228', '\x019c6a045e85c5c1e862411ea92ef6f976313dcaf9254d00cae12ac02003', '\x0179382cd2d27a85fc0a9b50c01af6dc22f019d52fa51cd8740c92000bf0a533fbd1cb4f14', 1795550437907874855, 1795550438396379808, '\x59328370962670e2', '\x01e0f3883bfb7125a6e9ba1b951c5919cec72451d82e28e6e3b785e22e6909b494366f142c646d0b4edaeaa1ad5a2cf8494ef6c3849466cd2ef0642533', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x64af7b33f83ae3c4734fdfc0b9bbf389', '\x7abbb11f7b965e21c1be8e9e10c049bd0ef503bfaca8fe9f83dcbcdf40b6f831', '\x01591096b39da456374c4616f064417bda88e5770ab20ea580316a307f7d1adf4532035143', '\x0106237c3d4a266c2b284e5a4d9f1cb126f23e8a6c558f8a96b7dcc7625b', '\x01de70e88028ceebbbf27f8ec3b1aeeded00c409e4b3fff7d05a8145ff4531a4a00e6b0204', 1795550437861390136, 1795550438349427565, '\x59328370962670e2', '\x0160cfbbc818b9c6ef4d7263f281e901ed6e5a8bc32eaa1e77779a8504f1a4fc1a1860099369c3f83ed97dd8b3f10495ac9006e9b2898df4c01c1052f9', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x1c7c2cca79fdfafd0b4dec61dc3bf337', '\x709fb0c359368be1fa381aaf4b16ef96d4034f2ece2fd8f7be3bd76210172aa8', '\x01e5cdf6e9bb5f27d85f103af3a35fce9ee65e95fcb201be11b89a35556d7940892638ea3e', '\x01bbe7ad0bab14353a3b1cbde8014f200060249467048d61d654c132d851', '\x015f06388d79a0c2ec35b05566e84770d27bc6bc3d7de9774172db76c94cbf5c1103db47b2', 1795550437951243661, 1795550438400571484, '\x59328370962670e2', '\x01a81d7ec557041d9386cafcc24bd81496b116a7266dd629197e1f0c92a8e611f1552ce5bdb5f02784cf95b147711fde3eaf08794b8cb96504240a1f54', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x3dd96fc88a9f8ef66dbef837a3c3cd15', '\x123ef0f31c58b7de2a36543a109304d020fa652b436fa88d944d98e61ae53c80', '\x01756baecbf3d35e3b9e77a6cc4e11e6104aa7b2134c698426506f518f3dd8c6830521c0ef', '\x013c996a20a817f41999f68c8a10f849ff5c749caec4ba6a22c0eb0b2b9f', '\x01a20b691f28584cc39c1aad46094f4bd014964bbb38d058802246fe677f7a955f95ec0e37', 1795550438005822362, 1795550438408276505, '\x59328370962670e2', '\x0192f0c361e5812eaa1c0599b10499945dc581a7dca99d8fe802b7197ea7e16b235ae7445d944f07422f0cab781005735dc5b05861c13dc5e4d669a5bf', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\xcb1b1614839435819e7fcf03536e7c47', '\x6bfd868abe28a4789c7d896b7064299c159cff738459f9bdbc6c2a3e819b7289', '\x01a15290ef7bf7a3627327322a634af0f85ae75b4abe3a106cdd3ad52a6f5c5786dc699fdd', '\x0198342f983599a56be04bff9029c2875e7d6e646e94450a3109adfca10a', '\x018449c21ede3270ded567ae0f66383d35cdaefe16aec11916b65c88785145d345c7473251', 1795550438731832047, NULL, '\x00d58438596f6406', '\x013b1589a21716515fccdc1f6e5fe67677c105c90e76f19065e9b851d288ff50c77aedfe0e04444ceee126573d95b801b94e7146884553307b4a8fcf62', '\x8367f3fa4d36079483e85d21141b4f60', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x7c7ba535414e8c0cfea23b0999ab3dd1', '\x1ea9b68dc7ef548077d6b40f838e3b79784ee3f01149fa0f55de54b0f5497390', '\x01b671236f9d0c99b7849c255ba399cf1e28a618b495fac81484f5a5a232c361c6c60d3a34', '\x013eb2fab9f5c28e29f1d78789849b5cd7e9abe9b49750acc795b05f5b25', '\x01b5fc3b813700bb01c3e2d951da11ed8fbea4f08c805c05c27d520a0bfc3bc5f4f7ab5dbf', 1795550438765065625, NULL, '\x00d58438596f6406', '\x01a21515e1444eef1deee36baeb96138487c28538b750fe938607bdff1273bb8331a1650dacd5ae37bfbdfd596b72478c3de60ddb0c53256a5d9428e47', '\x8367f3fa4d36079483e85d21141b4f60', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x281cc5c21dcb717c91b68ffe6b49c8cc', '\x66f2ee23d3a4f6787e87a57d718c75e7fb8d0d906983654c03265afdc437e567', '\x0143b0692a011b757258bec7ba603eb3e5cef47dd2ad66ed42fa9e4227bc733b098d3b0c5d', '\x01106474e251ee261bd8a1e446a434a36da546266cc44a81f734a1e75488', '\x01654dba9b425d1ce0cb18dd7d28f5b0f8013b0b4016888adeebed7e7cba4a1e003eca7bdc', 1795550438812643968, NULL, '\x00d58438596f6406', '\x01504754b76bdef0732b7352d53e7b0a2772919f425d26f2347c25c051ad0a7a30408793e375902a257fac77d354a68a77b970b2bd3143e7659c14f4e7', '\x8367f3fa4d36079483e85d21141b4f60', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x67ab3b32300d8573514562e5305b4f4e', '\xba3f9f7a44b2c79a5d29f241be3b2e004f1f7a433a91bbe158d9dce6ddddd4ff', '\x0192b398f8e85aae55ef6ae748ec00025e58c6f2bb2f11c6a2efe071ec8c62b240d6791832', '\x0179a8525a77e0acd7df9e3358a21ddfeb3cd35ca0744a341e1fe3a8080e', '\x0127d03376d36694277f0574c3b6e65448c3ecfb656cc7a2dfb0b092138de2508ba98c9d6b', 1795550438861755169, NULL, '\x00d58438596f6406', '\x01e00f4352f8d4df737d6731781152e7a1776026cacfacfcee1ea9c77eb54d6d0edcc10b33880385c94102f69d12e8e439ad9049179b62a7b43b69797a', '\x8367f3fa4d36079483e85d21141b4f60', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x019ceed85254aa1abdb52fe1129a8307', '\xd24ae8bb8231dd5cdb88910c7c17e06e9a4ec8ddc658f68db40ccb41052e107c', '\x01e1ce64c7793976192464edc9c0cf6af616b59614d21eb35f4d97e9490565ca2d05ab9c04', '\x01aa670100e1eaac2999fcd9bcbb46dce56d39bf507920a70ca28395ff67', '\x017c2be0aa182c75be6d3022a4bd6ebb21674076718a64670aafc626651f63d0bb6a8eb243', 1795550439073518663, NULL, '\x00d58438596f6406', '\x01502ac9ba81f684346bdffcb6918d6bb085fdf69677e90875c06045fe555e3b89805af10a57baa83dbb2421b65552eef8e799353a1dfab90f3df71bd7', '\x8367f3fa4d36079483e85d21141b4f60', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xae808ac83a2dcd2710a82a53afa8001c', '\x981971407a8c5c6c3e6644544808b4ad24f42d415d2c4395058b45762c895e3c', '\x01918327c60e5ce1a56c8179b5e80f2fe2e031de35232856bed3c591899e2bb4e4f5fe7d81', '\x010e9280538070ef3b5ad74f64fdf9eb99b1eb2354a5019b2d2749ffc0ab', '\x01b156a2c394347b0cf69106d18ccfd2e757a9ffbd37ed13b87d732c6f5d9c2e01b5f634f5', 1795550438055331313, 1795550438418760659, '\x59328370962670e2', '\x0133011e3e1afa4b5e659a3fde6f58bf9f97c47458f333a8c308853c8119f4827682557a3aa395983e193f19a94dd7c7ad0de29eedc6f87b95b513d0ca', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x7dc908a7c733fa3625996afe0ed56982', '\x1f446f1fa588767560ddbde669409a51b166ccaa8c5eba390ec8230e86b48f7b', '\x0150687cae36425df68325bc800adc5811370ba298a8393dc88f91664efb64c850ea4bd2d6', '\x014033c9b4ade16649bf3becb093ed91b42101b6b39a10d106d1e9b6f64a', '\x01f55945a54143c1a719f92016bb1a93d77d5cf132ac05be05f9ff74764cd21e74be76c7eb', 1795550438108079709, NULL, '\x59328370962670e2', '\x01ba94fd689a822668de775a39e8d857ed9ccfbb1d17a2f5115bbddfc4ff1ae7e8eb7afa17a2f32b5b9f68586f23ccf837855675eab9b8b823bea9f139', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xe42131b6b82713b087dd1bb3d98fa065', '\xd82a12ff3c7b365fdb193815bf8b08ed16de007f730eeb3a5aaaec537f6d5f87', '\x017cdb4c95d258f7317dd499d2d0d810c1a77d560c304c10731c38ebb1f6486438920fec8c', '\x01fa36669ae83155d2f042da07a7b3aa3e565c07a085801d11f8d909779d', '\x0133836652802d8bbc429cf248bd06d308716f25eb74186c7870a0dd4922ee5ffcbfee1230', 1795550438171579583, NULL, '\x59328370962670e2', '\x01f94a1874e56463f10b4c8dffa5fdd0594ff331f6ec5e44d19be449f29e6d2ee1afd3e719bdcf6df02593a082f7330fe08a3a3c964cda27347c2377f2', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x583f4a9a96eb50ef61b2691b995b3786', '\x5a351fc7a87edb1d49cca2fac077a64bb01bbc2bcf3be0d846fa83dd2fea88c2', '\x013f36f077656b9201b310a63a97ff701878acf937d64c775fd5b051293fdd5f2dd90eff37', '\x01be4d1f8bfbdb553a04ab22ffa5edc249d30be9fc48dce61e7bf33b11e6', '\x015873885af9273daf4a2b374e95845a6354821d3e809a67be45f2d2ac7b69ff69685d057f', 1795550438223225844, NULL, '\x59328370962670e2', '\x01d0f50422485185fd5d5896b2bef22e63ba5a29491e9888dd5e1fac5f03be30d3645aa343a92c0a09e627c7a35eb353fe2002e76c435ac01c7438c7ff', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xe4e21918b2d808a1914fb337276a2ed9', '\xd4e6d9b4ab79dbc3454648fb6ae1a5f5b033b624b9568aa752adc10fd1951940', '\x01c7c700a746b9a820e6e62a29609d49632b9eb2bf7b01b62ba0ca98a4a66906d5307e199e', '\x01804f264710c4030c3fe9a9789b844f0a869a62fb658fe39007c75285cf', '\x01114ec5fb82aeac1aae53ed21dc4452f65afc0201f5bcf558f4f7cee63404d26d84c1a341', 1795550438275293854, NULL, '\x59328370962670e2', '\x01abad9e925a9ecb1e9f60cb6e49cb231c6afa2e60e7857b07251f542d1de4e28db74acc0d1d0b05dfc2e59a2245d7846d294657391cc83640fbf4c893', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x8c05b3582f993d8f7e29a7ecb9f33ade', '\x9f3c92314c2278fe893969a1e76d21d833d3794ce441ed1329214670085974cc', '\x015652a9219da037a79cd09f3e8ec59951cfdd7540a338dcd2e01a26cc904ff7b9069c6fba', '\x018445e0d1af17dda3daa3c84314e3c621f58a2cd0bec6019540144f0e64', '\x012f1616fe81155e66699e0413f7b6a1ac89fc89be479ad522908d321223acb6603b1b6dd9', 1795550438312684551, NULL, '\x59328370962670e2', '\x010dbac3762dd12ec69cae6c1685d4e51fc3833053f50bf1c17443339ee871461fb9899dc138ac11f9b49a08b95ecf5c6c9148d057656367345235b775', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xd5619da83c3a2949c4e288134f74503e', '\x7abbb11f7b965e21c1be8e9e10c049bd0ef503bfaca8fe9f83dcbcdf40b6f831', '\x01577782257e81406bc236b1f52a4c495c75037a244c817e840cab4e98cc68dfc7d38cd1a5', '\x01d38aa612aa2192a177ad55de2ce13dfafafb742cc2f7ae7af57cd105cf', '\x011682e00b545e5bc709ae137f85178db2a87628fb6c1f14542af16a1c9e3f48f051a4df95', 1795550438349427565, NULL, '\x59328370962670e2', '\x0115b51ab5ad715590d93ff6b8c84498a9e83baad313a0bdf0ba22d477758c9e0937140b65961d5169c3d3cd287d014d3eb314a510a50c4992cd4f2382', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x6653e7d4d8cf3ca57ca05a3c461d480a', '\xa04e4f775b4f15cf6bd69c8922c95581bf88e1827881129d7feeaff97cb22aea', '\x01ec5310ec9ec35727e802604d17a11165118adbf04c368db5bacfa93cea7bb98c9ad037ea', '\x01da04da0152ccf77e8a46162c5242ebdbcf72912f5aacf05cbcfa2b858e', '\x01bd1d61b4d501eebd99a21b3f79c73ba578fdfa19c44f5b99e58f9069bb918c79c38f271e', 1795550438396379808, NULL, '\x59328370962670e2', '\x01067de86920431e85bda62baef94586534e191dba5dd89554e110dd3d7cff66cd73ed559b47908c9ecc34fbb574010612f995d5a2c1f9b325023c3502', '\xa6fc78a5cd3a21ecb0194f4415bca7db', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\x8efe4e0c4d336e7ffb1b4adb5de646fb', '\xb9128cfbe13eeddaf87c73f728b8b27eea06b796863706d3217486121b8f9770', '\x014f2f183dd75b67a5f9cb37d8a6d51d13b3f56fea0b3eb3222b1fc251f84d12f0d05b9330', '\x012f8eebd29009b2c2c0840a2a1b4ae6f1d74f1a8b5c7e75cdc7da8ef6bd', '\x01082790a2d832d00f3b0d29260e70c52358e6951a52941edb9c36e20e00fb5beeb233fa93', 1795550446785989525, NULL, '\x59328370962670e2', '\x0103e5186dea2a9b914fbd7bca2470f651853cf41c31d6a0a9dca2faafd95d6d55e47a1196f9a2bd5c2f8cf615e8f5a792882fe7d79565b707c345c4a2', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x0ccba7d5b806a9ee6cce897b8988302a', '\xdc1ddd0155816ed9c335074480b77eb4', '\x06e5dda97be00483a9f0a9d61832bcad89c03d41a6c298651bc88b147247fe94', '\x017dbd5778ab94bc561a2af358cb4aaa0d87e83f23fb7e0e33dd8a2e08f86c75d124b9c1b2', '\x01b9e5f71f7903df90e16d99795bf996c1eb3cdc7cb08d3a9b672a81190c', '\x0144ff1e3284b182c9b79a840154326e8e726e8c2c747dba6351274d8a7ca03e8bdb15c964', 1795550446831893357, NULL, '\x59328370962670e2', '\x0113785617c636462a105ce872522c5c66099b785bb38fb7d1490ceea7e33ed918efb67266623435ebac54ade051f98d2ff10a193d53c7d2053f17cb82', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x3b064bd47cb93530023d12724fc3124e', '\xbe76750b3f669d511edbabed61f556aa0a4c4876f268e964b167eeaa84a4aeb2', '\x0113498a2f7485fcbea42f99c3d8d7c92b51b750f8ce4d89c059735524025e81270d42e5c9', '\x01e88e2db2385d0a250ff510a0068b88f0661e07c863eb33920b2baaa26d', '\x01b6e901c3def742a3cb6cbd6b6791ef5bcc852cffdfed9c37878774fddf4e563483c0eb8f', 1795550446878414316, NULL, '\x00d58438596f6406', '\x01ab2fb2a4eebc9a1fd4bbfcd8c2a03da273988f50b993d32a3f3e5230aed5d041a227b70cd2c31102009b9d77354b135b0c6e01c301953bbb5e5fc376', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x447b2eace76a8db85ca208b6cf0c5c13', '\x92637ffc3dd17458826ce689b6121094', '\x849c626ea0709806c0096ecc6d60c4cd4eb2b098bde39fc0e5c356a9f19c4f8d', '\x01387c75d52da8a395a39af84d1789b87163fd399e08cd65fbc2592ad3e23b993aaf27867a', '\x01610edebd392f39698c5ec6e588b86e356a94ab5073819d2f4aa51848ce', '\x0156d924b579cf1b67b0665e27cedf1cb29945a3c11910df6978bc1faa29781e51e0b28b35', 1795550446924828619, NULL, '\x00d58438596f6406', '\x01f5d6bf71180b60338ed60165b5247ed06c004b9af785a89bea42a781ffbd7ec8e5676bc6eea3dff4829e7e40f6ede5333a88d1a4f4653feeedff1a4d', NULL, NULL, NULL, 56);


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


