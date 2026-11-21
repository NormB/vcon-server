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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-21T00:45:05.319902498Z', 'cli', 'tenant_create', '\x0495cb4a7dbda1f5e636e7b180ef60be98eeac1f33a8550daa67cf988799daa9', '\x012bc7c7f9a843b1aae789638b2cd0f309a2f8e025291c1f7061d2592df495d162a52f43533ee8739595b55692fe16e6ec649fcf1348b1118f68421916', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-21T00:45:06.744150559Z', 'cli', 'key_create', '\x71efc2a200c44a41c5d832f004fdecfb4290b47764860729b426650515dd1ea0', '\x0141a12382b0d5e8748d32f0462acbe3a0e602350eb0fa08311558399a614535ab3f60beef5115ffaee7def248', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-21T00:45:08.160772988Z', 'cli', 'key_create', '\x00dca07f288b6bdaef7c4cf2a250083954189a87ef2dbf478c3f03c997dd8f90', '\x01868687c7682b2b28fbcdb37edd8ffbd0147f926e7a57d1252a6a15221d4ebd2953bc9d2526f95c5e1466694a', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-21T00:45:09.537742359Z', 'cli', 'tenant_create', '\x47029e23b0a4a3fa00547ea8e20657fe60f0ff3f63e9efba530fc6f29b891a7e', '\x013128ac94543fb1fd9623cc763a6922428d0814ee3c1d84c75264cfb7f131c66ca2d0b689cbf54424971ec87aa22003557734d620c73e829e9cbd595b', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-21T00:45:10.952792461Z', 'cli', 'key_create', '\x3842de5ec1104d120ddc71216017526708a3ff1057d08235016993c2e6567f28', '\x014784f44046c4c4bb01c1e96c36cf7fd4301d88fe5497157613481862aef4fd7e2118c32a8255c474ae8f1a15', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-21T00:45:12.292398168Z', 'cli', 'key_create', '\xba314de68f142251c462010245701ae8881f0216bc6624728aad721de8b5c5f3', '\x018c4612dfc25b840884ba85d245bddc9f1b995be2e1ec75ca1229c3dc14affd2b64bfa04f9cb1ed56a452c8b2', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-21T00:45:13.691539305Z', 'cli', 'key_create', '\x90b47cd5df66debf5b96577e35941412c59357fab33b76cb9588276bff730480', '\x016030e32c65e662a4797a34bbcde34aeab6c11e25c279528cfb496b762f9fd11dd9dc8a993813267e6b62804a', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-21T00:45:16.028325033Z', 'cli', 'user_add', '\x466945093503957c35e2c933526dbcfad2e8cbf5e02b36a53cfd7ae25973101c', '\x016b79ff4d552327ef2726a29a52b3310432619722be20e3fa5ed9afa45bb783dbdd83c61b5781ab1fe896616ca8e53a7a549006bdab10719077a89231', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-21T00:45:17.405898734Z', 'cli', 'user_add', '\xa76223911c1d621afbc940cefb8ee1218f178f29dbd09e7d6b940dc2f3512fa2', '\x0138c58ac38ca6e177453b6dbad8274f5b721249ff655c5b730f104f3f2fc018229d97fd2d71a694d9431bf3d70604c5d8d1e68cfd9272b5fb4b793848', NULL, 201, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-21T00:45:18.838444509Z', 'cli', 'membership_grant', '\xdee4372114b2fc24ee93a777b9cfb0e4912de916b22c0500691c22910f24ff74', '\x010b995ec66cdd4c05f90ecb1c95226649049b142cbb87b3f08c1b5f27af2b44a214905021213f2488bf4c96eb6efd650ae035fcbddaa364dc059e3878872b5f22ef6928fac61dec16f7a40053dd50060cb554214c7cf09365bb09b687e2e197fde00a9366', NULL, 200, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-21T00:45:20.345168588Z', 'cli', 'membership_grant', '\x59bffcf3f21829e1a70eb7127421c8d3a145f2d4fdaa733352382257ce0616fe', '\x0142305cdae6ae2f10d4bf5cca96affeae5ae06968242cd11d57cafbe2647b48862857667bbbb8c95afcea6e965c84ce6e5155058e53a772e4e0df5b2cb8ed7727cd6eb0fdde789a84b31f0a141703a6b12b10dc86e8dd61d987b1839676fc80afd4ec2921e8', NULL, 200, 1, false, '\x2e26a5cd65af6ba8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-21T00:45:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\x2e26a5cd65af6ba8');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x2e26a5cd65af6ba8', 1, '\x01210877f819c8e1a6be74b9cab7ac31d2c0526218ea5e9dfd784062f3e5c6a77abf434935a6eef8aa5b5b7a8fab12bd2bda15c8c97b070514ce830977');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x43db754a6ebfedca', '\x9574f6e0ea676332f3241d3fb4f187ac', '\x424cf50ae814dd381d357b4d3e2716da3aee4ee0758aa102fb24e525d06baa9d', '\x0116d565924e38526282d7e0750a6e1193cdbf41a4535613b127212ac51585ad74', '\x01df5e9824de3574a7b4bfdd4bd50807a6016f0cb60e45475e1fe313ceba1c6b1f483a55e4b018a4df54ff90', 1795221908160385904, NULL, 1802997908000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xdab5368fd6d43327', '\x21f248d3e6a94313c7231584f203c63c', '\xc5ce00cb86b9e68c7794cab9796a775ef1952c3c7230ea83fcbd420cc672c346', '\x010f1437941f1efed5dd58fc522ec460844f1c1d22f0cd7821ad02ac6579f33bf9', '\x018f50d7e59a3fb2a90a69b5a66c7adc127517e502abe304c123aa6fbd8f1fdf740598898559963ca19e4d43', 1795221912292002250, NULL, 1802997912000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xe0c0bbaa47d5b815', NULL, '\x32c29627b9fd34239a765cfdb90a89b8ce10513dae5538e48ddb02b9eb17d266', '\x010dc4124add8d5e5993cd6d9a60322b209bbdb285169cf949faf196dff38016dc99', '\x01af843e0065fe791c2967ace0e9368a879bfcdb4da7ccb0cdaf062cf6f41717', 1795221913690981544, NULL, 1802997913000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xf23e7086b1626017', '\x9574f6e0ea676332f3241d3fb4f187ac', '\x1b0b890c149cc5eb453b8bbaa2863931343d615b0826b9a3c3a25ae0cf02a5cf', '\x018d0e82b1649d5a23f89f298d7f3d5bb2bb7f30e71c3de7a5263b31cb79a89217536f4645d47e8287d538236f07b47b736a226cffe10a28f905', '\x01c1a9f409c091204385d68046a332624e12261ead3930502436689ab7455565b90c15918b3e046ab4416126617b', 1795221906743618272, NULL, 1802997906000000000, 1795221900000000000);
INSERT INTO vcon.api_key VALUES ('\x07ebb42578791961', '\x21f248d3e6a94313c7231584f203c63c', '\x7b82876e2572bc2c8063bd9baca1137896924616996958e73eb1cac52ea67f9b', '\x011dc8150f437455571158cb404eac90670d519cf430c21aac7ce5f4c28dfdd15b8c231f6fb6eec0ccaaffa392dc05780d7d81d5d65b2e88246a', '\x01de24c7d2f15725402f93dda3c3d4d8de04171337aabd61538b79e8f38ce4fc6f91655d19753e9b0ad38e9f6904', 1795221910952375951, NULL, 1802997910000000000, 1795221900000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 2, '2026-11-21T00:45:06.747353053Z', 'cli', 'key_create', '\x1cbaa146e7a87c15c7c37252261a7fe62389eddf96bd1e901d07983ca89f2436', '\x0137681ee9ff3b45a4b6d93acad1545f748be50344b84ac09365505043a34ec3ef945c7020cb99e24d4f8a0121', NULL, 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 4, '2026-11-21T00:45:08.163823436Z', 'cli', 'key_create', '\xb29f6d47dff46e660ec452173ab3eb59372e376b334ab22353dc1b93071d0e7c', '\x01aa39612eca215f07940372b5d3b47e9c45588af7a1f7d47660d5c6989233782c4d411cdee2cedd66106d4f5a', NULL, 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 6, '2026-11-21T00:45:10.956118169Z', 'cli', 'key_create', '\x5600e0a04d9fc5346f01e87521eaffe54c53cdc1e8d25e7c33bba4a2868dec66', '\x011e0a3849b348ba3e6a748f1a67dbf32f38ba63b90a8808a360b79682cdc7d3edb3147a3484fb6e65a51cf8bc', NULL, 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 8, '2026-11-21T00:45:12.295495829Z', 'cli', 'key_create', '\x1c6b369e42f0c0832fc7cf9270900b9660e2c53dd2bd6b843c7c635942856af3', '\x0160e014de9375a976623a9ea33fa957c9a90932f5b8d3d6d543ab48a4b613cdc2a1be364a37adf5d695afadbb', NULL, 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 10, '2026-11-21T00:45:18.841665661Z', 'cli', 'membership_grant', '\x51e84fd5131b4508e57d9503c69de738088e547a77c94cb8ca2193970efdd8e5', '\x0104769bc5d3c2bace26caaf16349651e1f1d0623151c71d3e1204d1bd6d0394bcff83297b58b898330ef7f966605f37a1e8b298722682dc95e9230b8740f372098120062209352b84041e863b1e4cf1a09b408bd04aeca1b6266dea6275f20e6bee025814', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 12, '2026-11-21T00:45:20.348284424Z', 'cli', 'membership_grant', '\x4fc5a7ee12e1e9726dc16327608ae2da09dd3166905215096dd477be0398e16f', '\x0162f172e330affb9ec26c84e77636df8db33f3b91d447d36748c7b0a69cb255d7e445c5aa2a9438d6da3b5a81e36eda835a380e755ce40a44c451425599016662cfcd2548b8215ac577d22855132a2c41c8ac4fee3d9a042e944c7ad432e2b0a8a57cd100c8', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 14, '2026-11-21T00:45:22.361588861Z', 'f23e7086b1626017', 'create', '\xbb3cd5e3d263694e9270a409686a970f67b83b546ceb01158a302a3d3f073717', '\x019663fade70be11da97cc939950f1a1c4378845d3c79e8332eb602189fc58329d7b2bfb9eea0191137dce42b905e643b83a4c196229e6383cad18a660b0985187', '2e5d0824f6339075963a571478702920', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 16, '2026-11-21T00:45:22.370758398Z', 'f23e7086b1626017', 'create', '\x0cc11de32ea83e2e1167e90b570ffdf1ea224a0ffc494dbb18f5dbdc95bab4a5', '\x0154c56aca2e32d93944668298f5fc37089aaecaf726bc283478a91b6a5ecee25fd45e6775594785d052b69b615d48644e1f299df8f724723c3c0e2bdea80d8f6e', '1e14096f90cf6094d34b3536593cf62d', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 18, '2026-11-21T00:45:22.378892177Z', 'f23e7086b1626017', 'create', '\x9d7007cbd329f5cd7d997594ea9bfcf1ba15e9a2d31101619958a63ff92d13d6', '\x01e0ada698c85e6c19b4c17f20db88cf29bc4668774264581cdbdcd52fac8f813037644996de081d48017227fc139021eb6308d4bce98df6eb95da62292bf8c9b3', '8b6e38d8252dd0fcd0f7c95cf4fda14c', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 20, '2026-11-21T00:45:22.38649338Z', 'f23e7086b1626017', 'create', '\xea481c99486624e2c8ac9d35c3647ee5951159000f038b5c8801d3609c2c674b', '\x013d22eef82826227eeebe223a51caabb8347c35286383ee72e98e2d64303eb9ae02b06cd5d1dc725c6138861e267d4085fc9bf9e47fcab6fa11bfe1c4ab22c77f', 'd9d531b73d1053120f51f51439a038a7', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 22, '2026-11-21T00:45:22.394663131Z', 'f23e7086b1626017', 'create', '\x15470c2890d6b5e979b7bac0037f492cafe09b3085e712601ed9f5e0f8782511', '\x011c4635ebf5aa53417902cec881a3533504c1cc8dcededd12bf7a4cd400c9ebb2d1eed9d441591c1ca13c939b18ef151f4ea7a3918a01f6b66b399c33126fcda5', '80ae40c59f2e30f05368491c237136ba', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 24, '2026-11-21T00:45:22.403071891Z', 'f23e7086b1626017', 'create', '\x02427bde72fb5f3e2ae17a69451556ece968ef406b346fe36463e108ffb0ccfb', '\x0139c803bcf5a14a042443273de5f8905cc11528c242bc9edb8662175e163597f870dc84a2f9e946e3a3bb0357472514b9f6a37dff51a7f900cf3b9f3880c84e4f', '2e8fd343889c18d4d55d644469905c62', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 26, '2026-11-21T00:45:22.410536751Z', 'f23e7086b1626017', 'create', '\xabf2e94e94004c8b23edf71317e1db38ce54936a5c743a0fc8d4196bb015b04e', '\x0128468d51ac994ccf6ce6bce635418ab3adddbfdc0e407553380b47459ad1ee12133cffa38a4e8eeb054d78f488d4ecacc30c5f754c3e370b06f62f6432411adc', '067b77ef44015f40c7d3a089367271d2', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 28, '2026-11-21T00:45:22.418019806Z', 'f23e7086b1626017', 'create', '\x4a4ad21bf89c72c5db115ee9a8fee16652a20066cf8c8054a857b6a532947342', '\x01518f38550d4b072d52b1dae8b28c3ec76e41daa66bd66d4ac9661447ea71d5e138581d8577961f8a9cca7eb014b8c5f2ff13bff8d85eb5c22a1b0499ce25b6e1', '2403f0a6878e922f4a3d1f4b6016fb0a', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 30, '2026-11-21T00:45:22.425285166Z', 'f23e7086b1626017', 'create', '\x0ca61ed5b8db5dd2ed3acf2afb17263aeea0c312ff82257395b59a0ba21f9820', '\x01f019dbb5f461a782c37c2e760af4dae3ce8be195c3dba539198677e668f28aa9647db1a425428974b48f647f3d92a580f562c1545555344107aee85f059a001f', '6d518f44a8d45b3ca9838984ebd3d6ee', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 32, '2026-11-21T00:45:22.435809799Z', 'f23e7086b1626017', 'create', '\x64cc7ae06016a4c68f78fa8aabe4cb3f856e5b5c8b58776d3080de9cc0a666bf', '\x015e54214e490a24f2bd7b874bd016881ed8fd04a8f4d2bf406eb959c52c06b73675e8598dfdd3fff7b4bc3565ccb077b88b6a5538d3c857f8ea3b308cd76c556d', '1e4a01953af6e970e9aa99eae10718f1', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 34, '2026-11-21T00:45:22.445199162Z', 'f23e7086b1626017', 'replace', '\xbb3cd5e3d263694e9270a409686a970f67b83b546ceb01158a302a3d3f073717', '\x01e2ea0817f85bcdf69ffbefeeab09747bab94ca99590fa4f683c7afdf2fad0e26b90d45a62f206f0ec3dd72710788b50c8ada5ca673f6bfd6fe2113b60ea3254a', '085c1277adb5520190cfd9be405a3f42', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 36, '2026-11-21T00:45:22.453640421Z', 'f23e7086b1626017', 'replace', '\x0cc11de32ea83e2e1167e90b570ffdf1ea224a0ffc494dbb18f5dbdc95bab4a5', '\x01a3b8c49a085b54a829ac1e77ebce0d52e42bf49bf87fb3cbd01da958afe76aaf6a50be262e22c6fc3f5aec4ec860ce7460cdc09c29a5f730366360f696afc299', '0cef96250da8cc262cac0a01da8b146f', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 38, '2026-11-21T00:45:22.456996869Z', 'f23e7086b1626017', 'delete', '\x9d7007cbd329f5cd7d997594ea9bfcf1ba15e9a2d31101619958a63ff92d13d6', '\x01747f62c3ee5d1d3599a740f2697f30d5f5de8b26717106da39d27216efd5420bf6485a9bf5c18180c438478dbb03b1513b3a83fe9f4f7bfc1f1ac735e2150f87', NULL, 204, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 40, '2026-11-21T00:45:22.46315809Z', 'f23e7086b1626017', 'delete', '\xea481c99486624e2c8ac9d35c3647ee5951159000f038b5c8801d3609c2c674b', '\x01bafab0c2b379266d848140a34ac13a06a4d0b876e9e76be1938545fd6d0d4ac3aed1f149aaed08a0b1b9e377147328a0714be0f1d35eeb18f8a87dab073764e8', NULL, 204, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 42, '2026-11-21T00:45:22.467722577Z', 'f23e7086b1626017', 'hold', '\x15470c2890d6b5e979b7bac0037f492cafe09b3085e712601ed9f5e0f8782511', '\x01860b56ff096d46dff42ae1917df6b00493f91dfe48c15ed2fb5641c2ecff93a7f093475eea3ad162e1f70dc70424402e253a2610077bcdf47810ae3cea414b28', NULL, 204, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 44, '2026-11-21T00:45:22.47090672Z', 'f23e7086b1626017', 'delete', '\x15470c2890d6b5e979b7bac0037f492cafe09b3085e712601ed9f5e0f8782511', '\x013fb223d2fd95c016a51773cd3bfb83cdb4e021d064b2a66df5cb8c223598f130922e391a731c26243461cfbe35f7bfcb4588b93772911ac19224d69d73023807', NULL, 204, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 46, '2026-11-21T00:45:22.475405863Z', 'f23e7086b1626017', 'hold', '\x02427bde72fb5f3e2ae17a69451556ece968ef406b346fe36463e108ffb0ccfb', '\x0166f2a60383917ee51be4529ad70bf9f5b779d2358ff920c3fe0f8d0dd7c306e7651faa50aad3dfd8122c5eddc7965aa08adebeac923341c134c94fcbcddae1ee', NULL, 204, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 48, '2026-11-21T00:45:22.483139345Z', '07ebb42578791961', 'create', '\x1ecfdf8c6ee4a90dda57c9d622375a37fed0cf5ed63e21f985b494afb37b45b3', '\x01fbb15b8afc908af32997e9465da17b186d906e719b28608129a92066f05e0d48d1f9fd3320ef36533965f9b7acfb1bc8eb5d39f9fb4483194edd605cee4c7216', 'a10f3019cd9ac2ef7cde183b37c11b64', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 50, '2026-11-21T00:45:22.490773623Z', '07ebb42578791961', 'create', '\xa399a49f895ecbd2494d92b66fc304c3850e6c47874b383d2827e15628d6e402', '\x011354458bd6c2516e6585f41c61d6cbaed83541614feb83a1f6f7deb701bd8afde751b5f009797e306144a9fb688a3e1f98793fc06f7a318cdcf0e8d10d3dc48f', '627dd9b97e888a7a14c98e9138f51535', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 52, '2026-11-21T00:45:22.498291354Z', '07ebb42578791961', 'create', '\xb016edf3130b0ca8bab552a6f0ce882fe7c06a7b64f2b82bb16dfa4ec225e6d5', '\x011ead1341070459a1278e8a7610dbf178a6116d4d4f247c15dc1c161da12e3bee91712efccfb00c5247c227693d4308eba5350c7643bbfd4171fd74274ff80276', '915fc57e334b663b86ca1448675402df', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 54, '2026-11-21T00:45:22.50568076Z', '07ebb42578791961', 'create', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x01ca09a6321f1bf764375ce3f52558e875621913785f7faa195010e5c8cceda1e3752f551ef285bdc90a607ae77cdc279b5a8ee1ec8f62c8da0d8cca7f5541207d', '6568abb1bb14efa5cf57089c2a49802b', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 56, '2026-11-21T00:45:22.512887769Z', '07ebb42578791961', 'create', '\x6901a806ca2d0a5d2425e923708f8453ab6fbe583975f30a26a00d226b76c26e', '\x01e649ef9d97dec060a4f29b73467ac7a010b8a6f387e9bb629d53924fbf2f96678abb6811bd1d2ba32d48ad95aa54298c5ab73ec6a0749fda38042684c8e3929b', 'dbc93a8b40866e7b9cba630399b893f4', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 58, '2026-11-21T00:45:22.520065046Z', '07ebb42578791961', 'create', '\x846e271fae6c32cfd174c582d8b24a62d13ddef1c81a3e53f1aade59396e8c64', '\x01a324024b31ba48bc988f56beb01223bdf5739f6b4bce0931e8a4aaadfbe760c3f5c02ebbf9b880bd15ffe425d9718254cf36e790cec763374d71b8a11a1d95e4', 'b38ebb6f16150e3667bc25601aa0abde', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 60, '2026-11-21T00:45:22.52737486Z', '07ebb42578791961', 'create', '\x5aff661cc5c8ae0027f699add5fc6430e7090c8229f970c26454c24817340ab6', '\x019af51040b38fd207cce730704bc45820794992257a401ce1f3df4f1091add799071b2c3b2338e95cda60b2e8a49812067ae9c2ee3d88358a12d4a2f29cb1ecc7', 'd85f693f3fe402d2908837071452421e', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 62, '2026-11-21T00:45:22.534617192Z', '07ebb42578791961', 'create', '\x53e31e362272de4b33d4dc0261fceeef9b36e5f232ecc0ff08b84e7a754b9b1d', '\x01b27a439bcdfd9a3f9604cf2a71242445791bf2c44a14a7a04cf6a039df500f4e7eda1b8063f7150528ee4309b91a87a79a2029a23d32f58dd0d1fdd7a17d3d66', 'a20f722ef1232d9721c02ee5767c4af1', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 64, '2026-11-21T00:45:22.541948821Z', '07ebb42578791961', 'create', '\xbeeceec232978b64b289465764c5fcd3b2251b4597e929df625e9d5b4f2a413e', '\x011bac4cef8e0908b16fb1f163d3206c0923c193b78dd7cb17b1a346feb07c268fe7eadfba0e3a90dab9df3cf45a49295a6e5735d0584ab8719eac635bb3dd8882', '6ccb3c3d72922be63d84aa0b7f7841d7', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 66, '2026-11-21T00:45:22.549177821Z', '07ebb42578791961', 'create', '\x12833fee5ae5b05dde28598fe82239021618de4f310a75844a5e4851ec2e1e33', '\x01b00f88a1a78f7d0b9b9873d27d73fc01b509c91df01f75b6204de883a7ad9daf54e6d305af8ccca8d5a0b4246c95cdf1f21133b7e356bd0f4863dc3a7b25f3d7', '0125bf2d6ce443635f8121bd7463e6c9', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 68, '2026-11-21T00:45:22.559039379Z', '07ebb42578791961', 'replace', '\x1ecfdf8c6ee4a90dda57c9d622375a37fed0cf5ed63e21f985b494afb37b45b3', '\x01078509bd7b20d96c9d3d0ba7064fbc1f8bd47177165275bd4d0998a0314748ce07048ff4fda93db6cd2dc87b583a4a876d99fb87921d79b22c31dc20a34ca172', 'd3335f05d8552d5b19b11ad03eb5c462', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 70, '2026-11-21T00:45:22.563186402Z', '07ebb42578791961', 'delete', '\xa399a49f895ecbd2494d92b66fc304c3850e6c47874b383d2827e15628d6e402', '\x017e71ff94db398d04294feede914d9d2e826760d2ac6c7ab23d96dbbddfc400bbb41d27194016a88f60c0a543e9c0208e672041919196adb7587b93c99c1dd7cf', NULL, 204, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 72, '2026-11-21T00:45:22.566198702Z', '07ebb42578791961', 'purge', '\xa399a49f895ecbd2494d92b66fc304c3850e6c47874b383d2827e15628d6e402', '\x0195db75d20a19bef8ca81a669a3f958effdf5c984d009bf07e38a047a608a23aa9be0181614d985a3cefef68b8010769ae0175b2595d6f82b2bfe77f5d47598fb', '627dd9b97e888a7a14c98e9138f51535', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 74, '2026-11-21T00:45:22.584312214Z', '07ebb42578791961', 'hold', '\xb016edf3130b0ca8bab552a6f0ce882fe7c06a7b64f2b82bb16dfa4ec225e6d5', '\x01796066d488bea7989df6c46d11ba2ec076fd4476eaa87da1884b43b9f7cf9f45850fbe6d329197c5ff5ea34a1e086e1c2bdbaa4f3465206362223926c89ecf70', NULL, 204, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 76, '2026-11-21T00:45:22.588235246Z', '07ebb42578791961', 'delete', '\xb016edf3130b0ca8bab552a6f0ce882fe7c06a7b64f2b82bb16dfa4ec225e6d5', '\x01f4702ba43933b09b5772d666bba7fe44dc2f7790c9b4f318711d1d9d912974a1497f6853b09969c7bcfbf500003e956a292047ff8c2233fa9054d94161d77252', NULL, 204, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 78, '2026-11-21T00:45:22.593411299Z', '07ebb42578791961', 'hold', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x01804f8c35d500a0fe59e5cd21f8c5568ec45ce6384bbd303c9ac16e000c6f5ea3c15b34a5702810dc6c45703738cfc6877fd074840d699c6085d53ef0dd7fb1be', NULL, 204, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 80, '2026-11-21T00:45:22.602947466Z', '07ebb42578791961', 'replace', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x017945a4c5cdc918c056aeae7a47c5c9f551df7c73393787a756ec9e05ef10c8ade6ce4fb1c51952551f586b695c7557e7d0ac9d520c79676de0674a93db134e4e', '83cab80e56bce7cdbbecd1fd67ed4896', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 82, '2026-11-21T00:45:24.473369206Z', 'sweep', 'purge', '\x1ecfdf8c6ee4a90dda57c9d622375a37fed0cf5ed63e21f985b494afb37b45b3', '\x010d74d7e80dd84e4c1047389cd72530c540da22ab6d8fb6b18122e95f12cb907dd2e8c7a657103b84c4d7899d371bf83505ce586048aecea08977eb3512588588', 'a10f3019cd9ac2ef7cde183b37c11b64', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 84, '2026-11-21T00:45:24.868741795Z', 'f23e7086b1626017', 'legal_versions', '\xbb3cd5e3d263694e9270a409686a970f67b83b546ceb01158a302a3d3f073717', '\x01e13914a71811c5a36da33a2b3f7cbd16272d8483cd02e974acad5573adb3db77018bf126bcbc09d515cb9bdcf303748e25991d8e48a014c0e247cbd2ad9c0940', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 86, '2026-11-21T00:45:24.875288413Z', 'f23e7086b1626017', 'legal_read', '\xbb3cd5e3d263694e9270a409686a970f67b83b546ceb01158a302a3d3f073717', '\x016ab3587c84719bbaf02480bea7812613f3d6e0bab46c4e088f63bba4040576e69be5a94265ca7fe9b5ce4b33509bf94f89cc50eeef9ef1e38929d829567331f0', '2e5d0824f6339075963a571478702920', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 88, '2026-11-21T00:45:24.880146873Z', 'f23e7086b1626017', 'legal_read', '\xbb3cd5e3d263694e9270a409686a970f67b83b546ceb01158a302a3d3f073717', '\x019028ab0287c33bfbf91c6dbab9896481366581e26d385e6a602674e2078837bfba00a5fc6a0ec6715b1189509af588570b4b41aaf8101b726813cc67114dd752', '085c1277adb5520190cfd9be405a3f42', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 90, '2026-11-21T00:45:24.883668942Z', 'f23e7086b1626017', 'legal_versions', '\x0cc11de32ea83e2e1167e90b570ffdf1ea224a0ffc494dbb18f5dbdc95bab4a5', '\x014834aa42932061b4ace1965e17f1a0c30cb2f4b6345db03789cfd4316427c9b372b1b5079d5b9289cf121cd67919f903614078d5690cb27b8f5d01906ccaec0f', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 92, '2026-11-21T00:45:24.88807316Z', 'f23e7086b1626017', 'legal_read', '\x0cc11de32ea83e2e1167e90b570ffdf1ea224a0ffc494dbb18f5dbdc95bab4a5', '\x0175709077fd41f064cab08530baa5458f830186d69df2edcf123284adb678d9534aaddb8b7c54370bc2c50b4069b508643cf7a8a078767be6a65865e410779550', '1e14096f90cf6094d34b3536593cf62d', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 94, '2026-11-21T00:45:24.892607453Z', 'f23e7086b1626017', 'legal_read', '\x0cc11de32ea83e2e1167e90b570ffdf1ea224a0ffc494dbb18f5dbdc95bab4a5', '\x01717f87a2b28d8b5bb7044cb95924e663da9e106291f5cf52842586364309010e3d42475fc46c16524a176cee25af8b89fa898d862e030cd8795f90a4714009ec', '0cef96250da8cc262cac0a01da8b146f', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 96, '2026-11-21T00:45:24.895978401Z', 'f23e7086b1626017', 'legal_versions', '\x9d7007cbd329f5cd7d997594ea9bfcf1ba15e9a2d31101619958a63ff92d13d6', '\x01abc05d985d91cfeef4f613ffba153a6187e99b5f3c6a4733e050655801c4c8b2ae84f9fbd1d5e22583f928b776c1d7b3623c09ae3c3dbb6aaf05194bf62a0b78', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 98, '2026-11-21T00:45:24.900515684Z', 'f23e7086b1626017', 'legal_read', '\x9d7007cbd329f5cd7d997594ea9bfcf1ba15e9a2d31101619958a63ff92d13d6', '\x01092ab18df2592ebbba57f3a7934e40dc230801171dee497818ed51b8f7fbe74bc732377fa7021d440ece384ee67291513bfd0276bbc9411e9b800c7bcd11557e', '8b6e38d8252dd0fcd0f7c95cf4fda14c', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 100, '2026-11-21T00:45:24.903831568Z', 'f23e7086b1626017', 'legal_versions', '\xea481c99486624e2c8ac9d35c3647ee5951159000f038b5c8801d3609c2c674b', '\x01c38564618c1fbdcaa6e8219a38e0f867f396f89f08d5086726616675666186b8cfbd298673de11fd09282edf6db924cdea6adc79b7d3501207a9488408e8e301', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 102, '2026-11-21T00:45:24.908040397Z', 'f23e7086b1626017', 'legal_read', '\xea481c99486624e2c8ac9d35c3647ee5951159000f038b5c8801d3609c2c674b', '\x01fb76e7148f5c12f007dfee29d798939a8b8c92298df3248cb565fcf1547472eb3d58ea50a2160041ef2a266154c37d8c79ac29bb48740914886abcc257152ee1', 'd9d531b73d1053120f51f51439a038a7', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 104, '2026-11-21T00:45:24.911896893Z', 'f23e7086b1626017', 'legal_versions', '\x15470c2890d6b5e979b7bac0037f492cafe09b3085e712601ed9f5e0f8782511', '\x01d6f76605d33bb8e1171c89fdfe21dcad6b58ff1a258d48d186f169cc6810bbffcc274c26d898d4e3396859d29d6e735c83299faaaf13450be99b4f447a6da2b5', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 106, '2026-11-21T00:45:24.916204657Z', 'f23e7086b1626017', 'legal_read', '\x15470c2890d6b5e979b7bac0037f492cafe09b3085e712601ed9f5e0f8782511', '\x0158cfb991133392c572bd72e4ceb3e4ff1b2a69cdef67487ec47c5d34c9671e9446e8cf4a85a2208afd3cd5b239d8373daa12cf6dea7148d51e51f6664312c706', '80ae40c59f2e30f05368491c237136ba', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 108, '2026-11-21T00:45:24.919580337Z', 'f23e7086b1626017', 'legal_versions', '\x02427bde72fb5f3e2ae17a69451556ece968ef406b346fe36463e108ffb0ccfb', '\x017f92149c475f568ee94a3ce3398874f7619be69c18877eaf8e5ca8dc3f032ffb0a58ebfea89ddc48c0cb73058b0027b8947bc8e9b93772afd475300f9f08ca93', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 110, '2026-11-21T00:45:24.923896963Z', 'f23e7086b1626017', 'legal_read', '\x02427bde72fb5f3e2ae17a69451556ece968ef406b346fe36463e108ffb0ccfb', '\x017b476e798322b0cdc229add054e8328184c65cde1878897499fc610fc7d872cc1deb4c2848e5494f1e7d4d03f2d86ad68a68104ce1fb67fb981b3e98a3809899', '2e8fd343889c18d4d55d644469905c62', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 112, '2026-11-21T00:45:24.9270803Z', 'f23e7086b1626017', 'legal_versions', '\xabf2e94e94004c8b23edf71317e1db38ce54936a5c743a0fc8d4196bb015b04e', '\x01d93128f79595a77e940fdec619dedd6d2c13eda591a359c61b1ed9ddac94e6b8c8846dba5d3624b8dbc43327648a45cbb71f2e2a00c1432bb08c2b67e4336ec1', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 114, '2026-11-21T00:45:24.93120462Z', 'f23e7086b1626017', 'legal_read', '\xabf2e94e94004c8b23edf71317e1db38ce54936a5c743a0fc8d4196bb015b04e', '\x017f4c1427d9d1a2ebb2c210ba7381b91c33fde118d1448b25196378db5e98b313d4b24dac56c9214c6b19921d783851e371fa45322d46e186624931c9f7547668', '067b77ef44015f40c7d3a089367271d2', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 116, '2026-11-21T00:45:24.934376781Z', 'f23e7086b1626017', 'legal_versions', '\x4a4ad21bf89c72c5db115ee9a8fee16652a20066cf8c8054a857b6a532947342', '\x01b8691f8aaee3a58deab24965474f9ac479c8e757d4d85efb7ad0158ed0cdb834f80611f6c09b0223a39eeeb0f36361aeda80aa916555bcd63c2a7bc2097e1f48', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 118, '2026-11-21T00:45:24.93833198Z', 'f23e7086b1626017', 'legal_read', '\x4a4ad21bf89c72c5db115ee9a8fee16652a20066cf8c8054a857b6a532947342', '\x019f5444ed64402f7cbc6659ea2a58e29e1626dcdbe0c76a2381e99c28cc3a48aa26061e1ff4b37553d3a98d3765d327df240d05e383b2ae99883b4a52165a2eb3', '2403f0a6878e922f4a3d1f4b6016fb0a', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 120, '2026-11-21T00:45:24.941527808Z', 'f23e7086b1626017', 'legal_versions', '\x0ca61ed5b8db5dd2ed3acf2afb17263aeea0c312ff82257395b59a0ba21f9820', '\x01cb6ece76ca00bc54cb4561a5b552136e1c6f0bfb9bf1d3c5098846aa96ea2bbfaa06237303e5755e4be1caaaf4603477f1f04b41f5ec3a958fdac444804d4e23', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 122, '2026-11-21T00:45:24.945579414Z', 'f23e7086b1626017', 'legal_read', '\x0ca61ed5b8db5dd2ed3acf2afb17263aeea0c312ff82257395b59a0ba21f9820', '\x01704b508918d1699238ac132fcc235c6e5d2547edbe4b3ca6b0aa0cb0a390de65cdd548b76fe1fc477ca393d50aa4009e11de81afc50427eba9a170e3c6a0a0cd', '6d518f44a8d45b3ca9838984ebd3d6ee', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 124, '2026-11-21T00:45:24.948704418Z', 'f23e7086b1626017', 'legal_versions', '\x64cc7ae06016a4c68f78fa8aabe4cb3f856e5b5c8b58776d3080de9cc0a666bf', '\x01e9d9f4fda1b75d0c7b79cdc690cccdff32f28c52030dfe9c34358ab304144e0cfb7ac9f2831dd0452d560dc22ebfefe6d809ebcfccf603e828aeb062ce5380ba', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 126, '2026-11-21T00:45:24.952802387Z', 'f23e7086b1626017', 'legal_read', '\x64cc7ae06016a4c68f78fa8aabe4cb3f856e5b5c8b58776d3080de9cc0a666bf', '\x017136b97bd2fb3f159096af78fc6c76e4d21899df1ed1dcb7cb9742d3f402e603bcafcdf7e6fa63fd1ee362ff38277a4ce700999ab1d07c8a8a315a4ed6d4fa41', '1e4a01953af6e970e9aa99eae10718f1', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 128, '2026-11-21T00:45:24.955892965Z', '07ebb42578791961', 'legal_versions', '\x1ecfdf8c6ee4a90dda57c9d622375a37fed0cf5ed63e21f985b494afb37b45b3', '\x01a5c3ef4365624a046f90999dc077c9af1c2963f9f7b905d0b0cf86a22a3dcdf8886d37bd6feea9fc3121b4669cbe29f82dadbc00b9ec79acb5890d41f4192a51', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 130, '2026-11-21T00:45:24.959868358Z', '07ebb42578791961', 'legal_read', '\x1ecfdf8c6ee4a90dda57c9d622375a37fed0cf5ed63e21f985b494afb37b45b3', '\x01b422f4f744cecdc66bef524613e0f033649ab42a617fa2aa3d793af5076ed6a6599fe22a9ff8daf8172407b3e1ec75874962e081cfe92908188fd42a9521e852', 'd3335f05d8552d5b19b11ad03eb5c462', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 132, '2026-11-21T00:45:24.962598362Z', '07ebb42578791961', 'legal_versions', '\xa399a49f895ecbd2494d92b66fc304c3850e6c47874b383d2827e15628d6e402', '\x01e2c9345fe6f1d5b441105de2a0b3d0e1a79a4d03e5434c959547a962bd518f93e01b50865d7aa61364647abf680a82a519a0190826ac3a8ceca03b9b0634ef6c', NULL, 410, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 134, '2026-11-21T00:45:24.965616856Z', '07ebb42578791961', 'legal_versions', '\xb016edf3130b0ca8bab552a6f0ce882fe7c06a7b64f2b82bb16dfa4ec225e6d5', '\x01b152cee530319381d837c180ea242996e362d43db24fb2f94d0509a8ea1bfe054224998973af98f01de6c469637d175466ab84f1795319bff57503d1c383ab9f', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 136, '2026-11-21T00:45:24.969635268Z', '07ebb42578791961', 'legal_read', '\xb016edf3130b0ca8bab552a6f0ce882fe7c06a7b64f2b82bb16dfa4ec225e6d5', '\x011911f705f9e5075cb7ad91037c1f2b8e50c61f42eddc98112fe1a85c966bfd9590c99ba8bf571b9537c29c6dd0f38a68a0db35f64288b6ace8b6e4382cf15c17', '915fc57e334b663b86ca1448675402df', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 138, '2026-11-21T00:45:24.973045568Z', '07ebb42578791961', 'legal_versions', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x015a5ef14d5d9b93f9fb70af0905760e2aaac657b518b4b578b448a1dad8676d6fd124db4002fd9e7de85a437b26977237de9c0354f59e25d04fd2eef642ea6207', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 140, '2026-11-21T00:45:24.97820851Z', '07ebb42578791961', 'legal_read', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x0149b12ab02bb09e81ffd2e6cc9ad102c4d0868bfee5b095f5f5920f593fd7d08aa8e27411c6258c6caf4704cca6bb5562394b5cd9f4b4c90221cc7448ca997679', '6568abb1bb14efa5cf57089c2a49802b', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 142, '2026-11-21T00:45:24.982481117Z', '07ebb42578791961', 'legal_read', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x010bbe6795c88c920ca98db41ba1b6c1a225ff3c9d072888867e2a8c72739e547068f65a16280bcae2af523bdac13a1eb7239089ef53ea98eebf8bf8d6e0d3cace', '83cab80e56bce7cdbbecd1fd67ed4896', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 144, '2026-11-21T00:45:24.985533871Z', '07ebb42578791961', 'legal_versions', '\x6901a806ca2d0a5d2425e923708f8453ab6fbe583975f30a26a00d226b76c26e', '\x012c8a772582462884eaf491a35033d498ab3822e9494ffb52818d52000bc0ba85bdaf8327306f9de6d4d465377a8a06a628e5eab35d8d16407c82cead8ccc89df', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 146, '2026-11-21T00:45:24.989604292Z', '07ebb42578791961', 'legal_read', '\x6901a806ca2d0a5d2425e923708f8453ab6fbe583975f30a26a00d226b76c26e', '\x0131390a0185fe3814e4716237119fac960d44ea005ada5120731d31a86d5e02e240c825ddbebcba276f50bf50ef44600c04008aab17ab78821df471b6ee17ac1d', 'dbc93a8b40866e7b9cba630399b893f4', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 148, '2026-11-21T00:45:24.992917213Z', '07ebb42578791961', 'legal_versions', '\x846e271fae6c32cfd174c582d8b24a62d13ddef1c81a3e53f1aade59396e8c64', '\x0134a00e63c26b93ef1ba6e8cd46c56d0a192331fe9e0e9eaba1f571bb4e694363104950b4513de733c9072c41971023a2178945fee957511ac7e5c627e6dcb441', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 150, '2026-11-21T00:45:24.996948635Z', '07ebb42578791961', 'legal_read', '\x846e271fae6c32cfd174c582d8b24a62d13ddef1c81a3e53f1aade59396e8c64', '\x01b1bbd3501f46ce0373d1e8d393446b2f98badb62594940de1642a172287139dbc67adf3ccb8121c3036c8612612d2c3cf6ceb3147cdbfe17f26229fa9eaf5d65', 'b38ebb6f16150e3667bc25601aa0abde', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 152, '2026-11-21T00:45:25.000021971Z', '07ebb42578791961', 'legal_versions', '\x5aff661cc5c8ae0027f699add5fc6430e7090c8229f970c26454c24817340ab6', '\x010111d08f52ad2d7d1c4b49bb1776ce1009d8b182ec13b1e0753dc94b31b1e29392904798809138f40dbcf100507a9e1568306440bdfd5fce164c9eceb77c159d', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 154, '2026-11-21T00:45:25.003936328Z', '07ebb42578791961', 'legal_read', '\x5aff661cc5c8ae0027f699add5fc6430e7090c8229f970c26454c24817340ab6', '\x01741eb16213c7a4e7e110c4e531c33301be61bc4a40361978a94e17cb090ad4f0b6c77ccfe3e775e2b351e53e21e64a37caea67dc108bc42d40b732990060b8db', 'd85f693f3fe402d2908837071452421e', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 156, '2026-11-21T00:45:25.006980887Z', '07ebb42578791961', 'legal_versions', '\x53e31e362272de4b33d4dc0261fceeef9b36e5f232ecc0ff08b84e7a754b9b1d', '\x016a535d22dd26b0c2a89927a597ce6f497def8733d68ff0c65acb2191d255fd9a2fd205f2f03fe4e92a4c6c38d1f16d8522183127c2c0465c3b86de56e6e9f346', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 158, '2026-11-21T00:45:25.01108178Z', '07ebb42578791961', 'legal_read', '\x53e31e362272de4b33d4dc0261fceeef9b36e5f232ecc0ff08b84e7a754b9b1d', '\x01e666d084b5e057402b94ec9351f0d31ba70e9b3944395091db5c9943432f1170aab00192c419cecfc707a2c907f5ca72f2524e643109639e001e1eb69ee5436c', 'a20f722ef1232d9721c02ee5767c4af1', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 160, '2026-11-21T00:45:25.01410171Z', '07ebb42578791961', 'legal_versions', '\xbeeceec232978b64b289465764c5fcd3b2251b4597e929df625e9d5b4f2a413e', '\x01eb2f0f379a41e92929c7e2bc26c5ff1c284b82bf8925904c0cdb66c0b2cdb510533a11de43ea31b58a5402a97430a78f339c3d4f4af65093231b262e3d3e79e8', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 162, '2026-11-21T00:45:25.017945964Z', '07ebb42578791961', 'legal_read', '\xbeeceec232978b64b289465764c5fcd3b2251b4597e929df625e9d5b4f2a413e', '\x01d760c5aa5793bcdfdacd27dacdb93ce216e863b4870d4f765c3b774a76f610b2e9663f92965fbf3d1cd8e8ce9c34622e825361166c40105afa304149e239092e', '6ccb3c3d72922be63d84aa0b7f7841d7', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 164, '2026-11-21T00:45:25.020986634Z', '07ebb42578791961', 'legal_versions', '\x12833fee5ae5b05dde28598fe82239021618de4f310a75844a5e4851ec2e1e33', '\x016fb7cf993ad999e4ca74ed182549506d4a798ee58aadc0ea60a3589b2971a858490a5948775b6d7c9198ab9b5e239c4b303eb42b9f74c8c3a5d72b0d533a85ec', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 166, '2026-11-21T00:45:25.024980815Z', '07ebb42578791961', 'legal_read', '\x12833fee5ae5b05dde28598fe82239021618de4f310a75844a5e4851ec2e1e33', '\x0137ab6739146cbf047a5c815a68e80cc10aed5c82d4b6974a8854e117da3e4753fb2e355de4ed86eec24f0581f997a3db1f9ec968e90335a499f775084458d093', '0125bf2d6ce443635f8121bd7463e6c9', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 168, '2026-11-21T00:45:27.320307752Z', 'f23e7086b1626017', 'create', '\x4c377140107ac01ce5b8a98308ba892e4e115feb51b88a86414d57ba5b3cbe1e', '\x0167adab75352d2f5463a980efe755617221a9dfc026886481c31e233b268feb3a887e3da52be2e3cd70212e2998d5d3f5708074aec0310f7af103631cb27ef7fb', 'ebb71506d79eece995cfea22a35034b2', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 170, '2026-11-21T00:45:27.32937805Z', 'f23e7086b1626017', 'create', '\xfe44a8754b32862de8f8349c0f2aa69cbcccc3c7b255d74beeafc3bd647c9602', '\x01df03df8057d66af1a4bdfd4047a0d4d01c4ddb12e34205ca90586faf090ffc3097bc817935746c8e9301c92d57250a1ead2872112b81253a4fb21c07b4a1be16', 'd105d070044818d3adcf798b443783ac', 201, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 172, '2026-11-21T00:45:27.337669004Z', '07ebb42578791961', 'create', '\x95924b8823377180b1f46fe3a115ff864eb4b399472b94a24d704a760578339d', '\x01891378ba472a4af9045124814074c492778ee23c6195b8faa8285e13305fa98a5a83d2e622ec6d4d3cbf7f12aa8d4e24281d4718019d436f2798076e0719d752', 'b3fbaadbdf50234353ce811fa847a53c', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 174, '2026-11-21T00:45:27.346182709Z', '07ebb42578791961', 'create', '\x689a5a91f78e9d8c9f4aa8b4824150e1215e4774fc4b8c963d3ea195b7e42c55', '\x01d01ded4843361298a83c1c840dd294b99fa792008660e577a0b0b8381dcf252ce0d59cf6892529f5c5b2c45a0f6faf0aad56e5b9440399352277c29d2bdf5b08', '2802e3dc5d6d930d2919141f80dafc8b', 201, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 176, '2026-11-21T00:45:27.349439702Z', 'f23e7086b1626017', 'legal_versions', '\xbb3cd5e3d263694e9270a409686a970f67b83b546ceb01158a302a3d3f073717', '\x01667371c6333ba02ab3dc2ceca1c6ba41e310c8c159fda478ffa540cc51dc86fa16069973b5469d022a2fb19bf652516ff4d14206d10cce6124171ab5d997dd15', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 178, '2026-11-21T00:45:27.354908876Z', 'f23e7086b1626017', 'legal_read', '\xbb3cd5e3d263694e9270a409686a970f67b83b546ceb01158a302a3d3f073717', '\x01303a5e14de9276514dcbd1d7d5f5623c2f4ebfaab831cab708d31a42560e51efc00d7492ee146fb9d045f613343b590f7249a25fe878675fe0ed54ea6dc198eb', '2e5d0824f6339075963a571478702920', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 180, '2026-11-21T00:45:27.35928601Z', 'f23e7086b1626017', 'legal_read', '\xbb3cd5e3d263694e9270a409686a970f67b83b546ceb01158a302a3d3f073717', '\x01b1099ed5b88bc2a3bd0fdf0b0957f70dff388da30d24314526a999658706ab7e9caeeefc827bfc55d8877cbed2fbb0c25a63d3b5091fb8c0deb091fe4932436b', '085c1277adb5520190cfd9be405a3f42', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 182, '2026-11-21T00:45:27.3622482Z', 'f23e7086b1626017', 'legal_versions', '\x0cc11de32ea83e2e1167e90b570ffdf1ea224a0ffc494dbb18f5dbdc95bab4a5', '\x01d0f0f2f486d3bc15dffda20ae9a7d129705d87bec94af563514322719a723d5a4bff8717baf54cda756515104bb9608e8ababe00a625b4d4201da401d4110650', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 184, '2026-11-21T00:45:27.36619862Z', 'f23e7086b1626017', 'legal_read', '\x0cc11de32ea83e2e1167e90b570ffdf1ea224a0ffc494dbb18f5dbdc95bab4a5', '\x01b9c6888d39d61a34d42d65450ec8499064b07f5c7f8410a123fd18490b95342c5a3a24ee59dfe8a344ff9471d802606c02699f9873147ee8399ded5cc95f0b2c', '1e14096f90cf6094d34b3536593cf62d', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 186, '2026-11-21T00:45:27.370087264Z', 'f23e7086b1626017', 'legal_read', '\x0cc11de32ea83e2e1167e90b570ffdf1ea224a0ffc494dbb18f5dbdc95bab4a5', '\x01d7faca1b2ae2d2d45529b8348c305694b82b725aa071565a3d657fe8f00acee073e96a5374a8135c9d8bffa27bae0a0650ebefab2600c2a407850f8c4faa7b8b', '0cef96250da8cc262cac0a01da8b146f', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 188, '2026-11-21T00:45:27.373049285Z', 'f23e7086b1626017', 'legal_versions', '\x9d7007cbd329f5cd7d997594ea9bfcf1ba15e9a2d31101619958a63ff92d13d6', '\x01c35ba81817b5454b490bf57fbdc99470f62871c76ffc6dcc5b75f4f1641a4d3784bcb383cf777aa940c6abd35a23e2532e679ecdd97b69212f09cd7257fbcc7b', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 190, '2026-11-21T00:45:27.377173911Z', 'f23e7086b1626017', 'legal_read', '\x9d7007cbd329f5cd7d997594ea9bfcf1ba15e9a2d31101619958a63ff92d13d6', '\x0165bfe5e6d1089ad8e39113ff117693c0bdf23952ccb8d229d458aedbedc52f8721b89146cc6d7e06d058bfc2f92eaee22107b391ced22e3baed61bfb5e1f8627', '8b6e38d8252dd0fcd0f7c95cf4fda14c', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 192, '2026-11-21T00:45:27.380226174Z', 'f23e7086b1626017', 'legal_versions', '\xea481c99486624e2c8ac9d35c3647ee5951159000f038b5c8801d3609c2c674b', '\x01f871f3e5effa102f76f97570cdc9b673f49c5cb714173e5625f486a9f8ce22d867c7c7fe6f321a8cc5fa14fbf41aeba5aa1dff094da63afb5ce7dbd4d12f139b', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 194, '2026-11-21T00:45:27.384400327Z', 'f23e7086b1626017', 'legal_read', '\xea481c99486624e2c8ac9d35c3647ee5951159000f038b5c8801d3609c2c674b', '\x01289e1b7159f2c37ca69b3f329294c72e16bf52fcb9d2bd7da51504bce18acdb1b85f53510ec01447f52928e3041dfb3b4e9b07714faa5ece3053c70c8fddeb50', 'd9d531b73d1053120f51f51439a038a7', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 196, '2026-11-21T00:45:27.387443025Z', 'f23e7086b1626017', 'legal_versions', '\x15470c2890d6b5e979b7bac0037f492cafe09b3085e712601ed9f5e0f8782511', '\x015172c2ae7f5e3f08221a7b09637ee0a945fb2fb956802be2594272b7e9b19656122cddf0979dced660a4b77d2289a52022c9662899944d51128ede5bd8bd76d5', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 198, '2026-11-21T00:45:27.3912654Z', 'f23e7086b1626017', 'legal_read', '\x15470c2890d6b5e979b7bac0037f492cafe09b3085e712601ed9f5e0f8782511', '\x01ceee1c57ee3cf6fc4b70c5e034cdeafa07f9323037933c612fec8a0b6bdbbe000c3433b2eafdf345c8c722bcb5983ac2aa9857edd80ed64e028af785ad18b1c6', '80ae40c59f2e30f05368491c237136ba', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 200, '2026-11-21T00:45:27.394277043Z', 'f23e7086b1626017', 'legal_versions', '\x02427bde72fb5f3e2ae17a69451556ece968ef406b346fe36463e108ffb0ccfb', '\x01190ea11203e201b6d46f6a480ec498427fff739e4275d03bc7bfad49e1614109e3e10e95fabcab58a5779655925ea015dd91cb42461072e44d82c00e597dbeff', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 202, '2026-11-21T00:45:27.398143574Z', 'f23e7086b1626017', 'legal_read', '\x02427bde72fb5f3e2ae17a69451556ece968ef406b346fe36463e108ffb0ccfb', '\x016efe7ada381d79aec9d73e0f2aadd634dc124a767adfa2c7d2b4baba4f953d930acbad8a62ebbf34ac37d69bc00ba94b7ce79288fd7d4602c7c34c1ff8f5b820', '2e8fd343889c18d4d55d644469905c62', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 204, '2026-11-21T00:45:27.400946356Z', 'f23e7086b1626017', 'legal_versions', '\xabf2e94e94004c8b23edf71317e1db38ce54936a5c743a0fc8d4196bb015b04e', '\x01d62f21521fb99384fbe4da34eb59cf8761107386cf429f4d1b1d3d22faef968666a233e8c04fa5b9c7db3ce80c94ad640b0e59263f19bed082962027d5a87ec1', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 206, '2026-11-21T00:45:27.404723675Z', 'f23e7086b1626017', 'legal_read', '\xabf2e94e94004c8b23edf71317e1db38ce54936a5c743a0fc8d4196bb015b04e', '\x01ea51837f8758432b56e4f7b699d206200cf2a085458a449953e4e82be078994ad2ccca2bfbac532d5bd2b743ab6bde6d3849ac1334ef65225a37d054fb2dc280', '067b77ef44015f40c7d3a089367271d2', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 208, '2026-11-21T00:45:27.407737577Z', 'f23e7086b1626017', 'legal_versions', '\x4a4ad21bf89c72c5db115ee9a8fee16652a20066cf8c8054a857b6a532947342', '\x01de81e6d9cd6a330f148c49668ef384a4b1073a9064d7471a36abd3d90d4e5c186dc24be80fc0b943494670dbc2000b3d49f843b14ce22be82f40662e3d1b0b0a', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 210, '2026-11-21T00:45:27.413108768Z', 'f23e7086b1626017', 'legal_read', '\x4a4ad21bf89c72c5db115ee9a8fee16652a20066cf8c8054a857b6a532947342', '\x013e4b5a3ddadfbafd5a5cb60cc13999014477c1442df385d8706fd6af56c60775333529d1c27e792ab2921459f58269aa3b6f349b7a808178a1cebedb74bbb7da', '2403f0a6878e922f4a3d1f4b6016fb0a', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 212, '2026-11-21T00:45:27.41635055Z', 'f23e7086b1626017', 'legal_versions', '\x0ca61ed5b8db5dd2ed3acf2afb17263aeea0c312ff82257395b59a0ba21f9820', '\x01f5b2917c32795e1da417e4967b690b82347308215deceda3132443d31af09b31de35d61640f6902a9ff6e4dad41790723d3d1ece68fd7488fbdb00259b6a27c5', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 214, '2026-11-21T00:45:27.420289489Z', 'f23e7086b1626017', 'legal_read', '\x0ca61ed5b8db5dd2ed3acf2afb17263aeea0c312ff82257395b59a0ba21f9820', '\x013872fb94b21fd6d3df8efaf5f4b88a0487b14c9f4c897ba9e5bbc891f88af9d040927f4e4e46d008456f303cd182294d0eaa6cae89f45ec1c83b6d65066ab886', '6d518f44a8d45b3ca9838984ebd3d6ee', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 216, '2026-11-21T00:45:27.423266891Z', 'f23e7086b1626017', 'legal_versions', '\x64cc7ae06016a4c68f78fa8aabe4cb3f856e5b5c8b58776d3080de9cc0a666bf', '\x01692d06a7523fa7a81a045c9cdf5dc42eba17b33a9fafbeb0722f98084a2b76116f01f8ecdac12d9151691b6793cbf32bbf9a260b7dc4276fd21fc97ba077642e', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 218, '2026-11-21T00:45:27.427321229Z', 'f23e7086b1626017', 'legal_read', '\x64cc7ae06016a4c68f78fa8aabe4cb3f856e5b5c8b58776d3080de9cc0a666bf', '\x011c2b46979ce987c2899648d03a025c96c3e78843edc5c767db848241518f7ddf1cec9f640ea440b5b5d72b839187bc24fea9f8d1d1269e8b296c04a85c3e040a', '1e4a01953af6e970e9aa99eae10718f1', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 220, '2026-11-21T00:45:27.430356482Z', 'f23e7086b1626017', 'legal_versions', '\x4c377140107ac01ce5b8a98308ba892e4e115feb51b88a86414d57ba5b3cbe1e', '\x01f64ba9a02f480fb4cca6ca98837d336a2e1ce88ae24374ef9e625b243c0f2cbf5979c8a28559a37ea52ebf71a270762215e7e3a31cf3be11d953be5dcfcbf563', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 222, '2026-11-21T00:45:27.435209729Z', 'f23e7086b1626017', 'legal_read', '\x4c377140107ac01ce5b8a98308ba892e4e115feb51b88a86414d57ba5b3cbe1e', '\x013377207f6204e59540df9eec0c2ce021e9b279f35286a708e5b8baa584a8fb70c5025b7440fa5c9ac3bd42db2aa0db9f8199d7d4ce6ca15782bbd48cc1db7dc4', 'ebb71506d79eece995cfea22a35034b2', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 224, '2026-11-21T00:45:27.438254149Z', 'f23e7086b1626017', 'legal_versions', '\xfe44a8754b32862de8f8349c0f2aa69cbcccc3c7b255d74beeafc3bd647c9602', '\x0159999f6012d55ca0302ab2a7995dcbf61e5d712180c555d2580500bdb00bba2ee8d9b982358d27a90228359c34740bc44fee0beaf141a16e02b9a892405fca93', NULL, 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 226, '2026-11-21T00:45:27.442176265Z', 'f23e7086b1626017', 'legal_read', '\xfe44a8754b32862de8f8349c0f2aa69cbcccc3c7b255d74beeafc3bd647c9602', '\x01a9df60b09135be44347617434b7dabc2d2f362c40e608387969e77b08bb451131678eb7012becd94af6ab5cb7213bbe9fb224cd70fc16624922b23997f23a701', 'd105d070044818d3adcf798b443783ac', 200, 1, false, '\xa21a9cbacd775dd9');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 228, '2026-11-21T00:45:27.445034167Z', '07ebb42578791961', 'legal_versions', '\x1ecfdf8c6ee4a90dda57c9d622375a37fed0cf5ed63e21f985b494afb37b45b3', '\x0100c3929fbef894a66e7b682cf8a4756b5eee3885f4a9061d3e4e333104975fa8f5e58d7edc1eeef9838623b98991e62bb1b94e07ddefbac6bcaf34943b8b0905', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 230, '2026-11-21T00:45:27.449625172Z', '07ebb42578791961', 'legal_read', '\x1ecfdf8c6ee4a90dda57c9d622375a37fed0cf5ed63e21f985b494afb37b45b3', '\x01b99003d4ab9c23f9a09fa27c4bf329ed41fe353c2bd03877adb7f1556d8c8e9ca07a8168d8e35ed73d98ee14496a81c2c3d36176f2a2bf8d2184df22ce860aae', 'd3335f05d8552d5b19b11ad03eb5c462', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 232, '2026-11-21T00:45:27.452232573Z', '07ebb42578791961', 'legal_versions', '\xa399a49f895ecbd2494d92b66fc304c3850e6c47874b383d2827e15628d6e402', '\x01c74a4e9665ec7e1c8d11bc87beb16a1531133e93d5663e8c0dbd4b245a9ba584df20b11046da8556bf5230bd241928a9337425b3f144900fcee6fe310fcfdca3', NULL, 410, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 234, '2026-11-21T00:45:27.455005179Z', '07ebb42578791961', 'legal_versions', '\xb016edf3130b0ca8bab552a6f0ce882fe7c06a7b64f2b82bb16dfa4ec225e6d5', '\x01b076083666bbe63b525d1cc010d7ecededa8b22009615ebe002e9440be4c2fe8203d56c51cddcd1a934f5e03947fb5fe63ce55519d25ad5af78fd10a814ca4d2', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 236, '2026-11-21T00:45:27.458715951Z', '07ebb42578791961', 'legal_read', '\xb016edf3130b0ca8bab552a6f0ce882fe7c06a7b64f2b82bb16dfa4ec225e6d5', '\x0180ed03a49df395c21741e8c7b8be1ba607f174d83408b1cdc794d1ef976efa6cf470764a89f73592c248e19b7701953d2994b8e267b1b293ff15bf77ffae1152', '915fc57e334b663b86ca1448675402df', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 238, '2026-11-21T00:45:27.461490769Z', '07ebb42578791961', 'legal_versions', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x014e98a8fcb01c7fc25fa7092a4719c1c38c8a8f56e7a8c405e71811d76ee220091353049dae33d8f2d5504cb9cabab7f07b3ca22e305cf1b8d444b2e41e23ff61', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 240, '2026-11-21T00:45:27.46506856Z', '07ebb42578791961', 'legal_read', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x01d4a1ba63d4b24408f189ffd90f8da9df1ab2c871abc7a22ecae1177d04be7e0d4d3e84f4837b0734066252b8618f5eb15b7e1bde649d7e892681a934840d65dc', '6568abb1bb14efa5cf57089c2a49802b', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 242, '2026-11-21T00:45:27.46873975Z', '07ebb42578791961', 'legal_read', '\xfadec3aa2c44a1a64eb631a71db07264fa901b90c27b682fd7dac65ad239e9ae', '\x01174ed3ce67d7384eff76f531e78be69fe6259fe3a418a8be972259660ec7092c1ee71ea400ad2ceabc5ebfbfd3b78cea4bf35ec509db34dd44e2fdfefb924910', '83cab80e56bce7cdbbecd1fd67ed4896', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 244, '2026-11-21T00:45:27.47148466Z', '07ebb42578791961', 'legal_versions', '\x6901a806ca2d0a5d2425e923708f8453ab6fbe583975f30a26a00d226b76c26e', '\x01199ac3a8755238ee89a83e36e11328fef86f3008c9adadec6c40e8e55c6c0b76706fc4282095d0df76272ec07864d8651d3f5151c357200575f2c4ea2129adaa', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 246, '2026-11-21T00:45:27.475087923Z', '07ebb42578791961', 'legal_read', '\x6901a806ca2d0a5d2425e923708f8453ab6fbe583975f30a26a00d226b76c26e', '\x0114d7ab2f1e079f4db70da058eb10dd2ba16eed2421612f5b01da8c1c951f2517c75ebbab61611d12225aa006b4af752b9c9437c3f6f4688217d36f803ee3c983', 'dbc93a8b40866e7b9cba630399b893f4', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 248, '2026-11-21T00:45:27.477828704Z', '07ebb42578791961', 'legal_versions', '\x846e271fae6c32cfd174c582d8b24a62d13ddef1c81a3e53f1aade59396e8c64', '\x01e3d447c25a578a456c87db3a0d50d77804e7f0b411fe4e2958ab62cdd3d8c790e245a1206c1ce02244ea836a67d911e0ec2e8e8dbb11652d60b991a16d2c3024', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 250, '2026-11-21T00:45:27.481429441Z', '07ebb42578791961', 'legal_read', '\x846e271fae6c32cfd174c582d8b24a62d13ddef1c81a3e53f1aade59396e8c64', '\x011c800957f8f170b98e8059301d4be65aec3b0147d0ca31c522baa7934abe891616f490f32c2ad64a30994259871dbd00dcaefb0eb259c2a878d4adfa3b88e1ca', 'b38ebb6f16150e3667bc25601aa0abde', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 252, '2026-11-21T00:45:27.484165657Z', '07ebb42578791961', 'legal_versions', '\x5aff661cc5c8ae0027f699add5fc6430e7090c8229f970c26454c24817340ab6', '\x012645e705fbe506e59d4567cff14b6f2b27e9e99a81acdb03d1f867904a615c972ac61e0953397a04818cd92083a4a95605c5f357a0a429f1616dc2342e27f65a', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 254, '2026-11-21T00:45:27.487832327Z', '07ebb42578791961', 'legal_read', '\x5aff661cc5c8ae0027f699add5fc6430e7090c8229f970c26454c24817340ab6', '\x01d6ff178cb8785b977f48ed1e6654a736018b14e70a877694aa74ef4c58e3539234f5ba5b8245e14fccc909669282d411c5c9204e741d446cc82a3ea40666e821', 'd85f693f3fe402d2908837071452421e', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 256, '2026-11-21T00:45:27.490678387Z', '07ebb42578791961', 'legal_versions', '\x53e31e362272de4b33d4dc0261fceeef9b36e5f232ecc0ff08b84e7a754b9b1d', '\x01688bae36c371de6791db39e7e9af8eca5861b84eaae7152e8160fe54963ae189abb5a1adf7d934129ffd4eb5e47010bd34c084cd33114aefddd27607c61189db', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 258, '2026-11-21T00:45:27.494988086Z', '07ebb42578791961', 'legal_read', '\x53e31e362272de4b33d4dc0261fceeef9b36e5f232ecc0ff08b84e7a754b9b1d', '\x01b90e34a94f08a54804d2d8985cb863ef6515449c6cb8173cbda033bf69d96baa701f9d2d645f92320eab95aae41570c4f777deafa469a883dd91083116ef32af', 'a20f722ef1232d9721c02ee5767c4af1', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 260, '2026-11-21T00:45:27.498021469Z', '07ebb42578791961', 'legal_versions', '\xbeeceec232978b64b289465764c5fcd3b2251b4597e929df625e9d5b4f2a413e', '\x01a2bb2f68f129a5609e7d0ab331786b7048eacf1ad90be6e8e6be4fe764c1b6c9e2e74942df7cb64577d7d8cea8a3dc6f33083773639f8b01bb2e0395a645ef79', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 262, '2026-11-21T00:45:27.501722002Z', '07ebb42578791961', 'legal_read', '\xbeeceec232978b64b289465764c5fcd3b2251b4597e929df625e9d5b4f2a413e', '\x01ced279fd530b534b0973e13feea412743d28b7c37e7fc879c3803f97fc0e2a3d1bffe43c8f3c56958c0a13784458339f6bb020df8bf57608973958f291c7e73c', '6ccb3c3d72922be63d84aa0b7f7841d7', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 264, '2026-11-21T00:45:27.504503533Z', '07ebb42578791961', 'legal_versions', '\x12833fee5ae5b05dde28598fe82239021618de4f310a75844a5e4851ec2e1e33', '\x017e89138343edff7aeab7847188fd52f350fbc3d279d05562950ea038f5dcd8211bed421303bff8b9d4540397c49cfc4701c046919bde6b3b401bf7426fa836b3', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 266, '2026-11-21T00:45:27.508436621Z', '07ebb42578791961', 'legal_read', '\x12833fee5ae5b05dde28598fe82239021618de4f310a75844a5e4851ec2e1e33', '\x0186774abb920c9e54ccbc5249743462a828e528981b481a3acb6c203965a342d4181077ceb45abd2bfbaea12ec43182e2ad28ff6ad24db5ce4f3951834e50ad3b', '0125bf2d6ce443635f8121bd7463e6c9', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 268, '2026-11-21T00:45:27.511206597Z', '07ebb42578791961', 'legal_versions', '\x95924b8823377180b1f46fe3a115ff864eb4b399472b94a24d704a760578339d', '\x0197333b967b565419665ce212a46c8c485643cf3b024fa91eb3b712e88264344f2807382b1ef94ca0b140ce95f3f05a729fd25123d7d6716efcb6e51312e81622', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 270, '2026-11-21T00:45:27.515021741Z', '07ebb42578791961', 'legal_read', '\x95924b8823377180b1f46fe3a115ff864eb4b399472b94a24d704a760578339d', '\x016229815be7232c02f2dffd31a0fea0ca32efa8fa999e9e3fe1ee34ed24d94898abcba048083eed33fe4ed5df2b00a5379e1c002ec1880864b9b94f640880355d', 'b3fbaadbdf50234353ce811fa847a53c', 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 272, '2026-11-21T00:45:27.518212467Z', '07ebb42578791961', 'legal_versions', '\x689a5a91f78e9d8c9f4aa8b4824150e1215e4774fc4b8c963d3ea195b7e42c55', '\x01cab968abf2c425c294b61261f07fbe616f361f673fd8abd6deef272ed1dc17e6c3cd4a9888dfbb0a76daf5b8bc7551b3ad2a47177aedc5051c99fcac770bb91c', NULL, 200, 1, false, '\xdd0e493fb9b68cff');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 274, '2026-11-21T00:45:27.522418054Z', '07ebb42578791961', 'legal_read', '\x689a5a91f78e9d8c9f4aa8b4824150e1215e4774fc4b8c963d3ea195b7e42c55', '\x018ef7e0c020ac7487ec533356d3b129063cf8b64a30fc7632fe27bfd108dc743975ed57d5f9f250d50ace81f4d29c84e52312c53d8c223e4d25b878026cdc5bdc', '2802e3dc5d6d930d2919141f80dafc8b', 200, 1, false, '\xdd0e493fb9b68cff');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xa21a9cbacd775dd9', 1, '\x0114c9ba955baecbd8d2acc284cf970b60a2ffaf72ea28133345fb2d00f7ce92ee33c486cd12d811ffa98ca0863c8d38f070c3cdab8cbfb7ad9199165c');
INSERT INTO vcon.audit_key VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xdd0e493fb9b68cff', 1, '\x01f5756bb82e5185905b00aecf67abd36f9859fa067866d243be43625f39a1b6996d8c82e5735a5c014115e0ad82a1b7008f6617ac748b1ce7883cdc2a');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xd7ecc29083d9b151', 14);
INSERT INTO vcon.key_use VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xcc9ae42a58ce0698', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', 2, 1795221927049218176);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', 4, 1795221927168219528);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x9144dde318204dbf', 2, 1795221905310181996);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x45295a6bc0e543bfff9bd8ab65ae7808', '\x9574f6e0ea676332f3241d3fb4f187ac', '\x012512f28aa952a6f191c5413cc624e523c7bdb20ff45605bd2d724312b12a2a24461b', 1795221918837406193);
INSERT INTO vcon.membership VALUES ('\xd1e722c252498aed2f90956e9f324519', '\x21f248d3e6a94313c7231584f203c63c', '\x01d6e14fdfb8f564d9b163038dea2c3df66b0ade12d1a4300f46d6592d72d91e9b1cc247', 1795221920344428818);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x8b540aa7a52801c48650c355eaa7c1b5', '2026-11-21', 12, 12, 7404, 1795221926922385312, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x4541d4db52f03439ad4629d6efd8d1e6', '2026-11-21', 10, 10, 6240, 1795221927066096149, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x6f5e0342ee340470');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-21 00:45:03.834128+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-21 00:45:03.870638+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-21 00:45:03.878031+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-21 00:45:03.878723+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-21 00:45:03.881281+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-21 00:45:03.885395+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-21 00:45:03.886245+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-21 00:45:03.891508+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-21 00:45:03.893947+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-21 00:45:03.89446+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-21 00:45:03.895508+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-21 00:45:03.899046+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-21 00:45:03.905128+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '8bba45734dd6397a77c847622bcbf86e');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x9fe9d8614d14a3f3a4b61b6360e7cdc6d55d0e0b0b9591b0c87252a57818de0e', '\x019771987ddb65b27ed8084ed8382ed11710453d523e37ade1703c127f23426b5d29271229', NULL, NULL, 1795221905319642294, '\xd7ecc29083d9b151', '\x01cd70edb05fb49c89ee8c8b28eab84f7d9f8bf94fb16353ee55820e51d8e2282a7e238a93cc864526386c15ea4f3c58bf15fc227cc17f4d6aa9d8ad1a', NULL, 1795221905319642294, NULL);
INSERT INTO vcon.tenant VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xb4153d3f0bf5c0a40f7efb61af2679e123babc59572e2cda5ec6057dc9e314ca', '\x01d7b5e9c69ac2b67145b83f002f45cb056331c558ad7efc20b9f38ec126ed2a6864ebf782', '\x01bc45caab4ebb7d1717a884b4122e80af9962310ef60c198250b2753719f3', NULL, 1795221909537570489, '\xcc9ae42a58ce0698', '\x0125fd97946aced7d639e22af9501c05b1461de302f05d8edce8f64aec5a4666499c86d9e870179e081892563c4eeac280b4ec6199bed75d054749b903', NULL, 1795221909537570489, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x6ee2727cd65f26eb');
INSERT INTO vcon.tenant_catalog VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x82c8331c5fb27ae4');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac');
INSERT INTO vcon.tenant_lock VALUES ('\x21f248d3e6a94313c7231584f203c63c');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x114c709d9e87e8d236429299ba75e0b8520b4619d1ffbe90f21e20138f253527', '\x01d1f9601b37b5bab98fd5d37e82b2e4bcd04cf55b1cd40086417244a1c924a784c2c5d2f18d6ee0c3bbf7ec2b', 1795221922564642348);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x45295a6bc0e543bfff9bd8ab65ae7808', '\x01f06c69a85e49ae315cd2afd35d346b6eed3214b06e9a5146e18244b0157227a095a71df68f2527cee254', NULL, NULL, 1795221916027649106, NULL);
INSERT INTO vcon.user_account VALUES ('\xd1e722c252498aed2f90956e9f324519', '\x0100d38b18b82ce35ee26f3eba5e45213dd4489747d5882c9c230c0e18aa97e8545022f80fc36a3e46f1699a', '\x3a50e93260546584aedc8db7a57e26d243c7032f6664cb86929ccc80d4c87adf', '\x01431d87a8cd42c97ce90af97b99a4be7d9910ac007859afbe50f243d1e5a806e3e81f6ff200469039a1f2ea44e786bbc98335da5f11551e970207f07fd9b431', 1795221917405583216, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x8667eaa98bc91bc6327a15c976b255dc25dbec1d013cfae3e4eecd8e2cbaa6d0', '\x016901a701ceaa9179bb19f0fece9cef81c3cef90672367322ab7c543ae6701a6fc1b6830f36060c38f6b2fd76', '\x067b77ef44015f40c7d3a089367271d2', NULL, 0, 1795221922410203491);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x2100248ba447ff0e7c9e969dad8cc717800fdea4bdd178aefb1e97eabd9a6717', '\x010b5e6a4c3eed847c7554970d999400a8992045a3bff3e3c3972d0a3d94ec34a1f2e5cf1c697115f5f488a14d', '\x2403f0a6878e922f4a3d1f4b6016fb0a', NULL, 0, 1795221922417662768);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xaf7ff0d445fdecdc2054b91e819c3fb23fa5f520fbfee15bc975a6faa9e318e6', '\x01ff0acd4e9469d5b7bae3c7fb307a09b8c408b5a6ea06832d28f9a297ea37d17d92b05a6634937f47aa9acbfd', '\x6d518f44a8d45b3ca9838984ebd3d6ee', NULL, 0, 1795221922424952426);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x451975f763ddf5bed1aa95021f65b97a5b09295f21c63e6bfeb6cd49e514a440', '\x01ad1114411fb221c5bdca72721138324c86d90f3a1021292cdbe5485fbf7962e72dac4a9bdbc39b0643b6810b', '\x1e4a01953af6e970e9aa99eae10718f1', NULL, 0, 1795221922435477900);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xf8280a39ad9ea00bdaaa0e868192a67d99a2f8153f288ff7de76a6dc736ee535', '\x0166ee855159666f9962587e685771f2e156d3e5371e01e4bec39aa39f98eba9c89f821a609cd3d19730b106f8', '\x085c1277adb5520190cfd9be405a3f42', NULL, 0, 1795221922360293637);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xf0d8debaf926081a616815c7d736e51a6b4548270af77d789c18d0f61d6e1b09', '\x017716afd366d8a777e73621a5659113d56f25d2b7d52ed8b682dcd4ce09c243ec94fc12b75fde92034a2b3d80', '\x0cef96250da8cc262cac0a01da8b146f', NULL, 0, 1795221922370389972);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x605da8dc4ff33e0b6d91075a710c0d5cef4c3b49771d120c4a82d9e2b1cb3823', '\x01a0549b76edc2a3daa13e5ac31024dff29796e0029d828589fc15bc464927b18d5b5c3fc95f0dabf476d06554', NULL, 1795221922456654119, 0, 1795221922378542778);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xd071c768b046bae2a4eeeab8adee3a1b5fe7cfd5e5ed3e565b0503e215b5d179', '\x016a8036d2d4014e5d1c1a92f04013a2617bb0f87dc7eff8a99dc62c871dffe61d35a6e9316bcb9126ae52e822', NULL, 1795221922462822006, 0, 1795221922386155018);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x0f1f7a88aba00de84940912bab521b07d86d0030070094fb6dbb1feb340041c5', '\x01506d6b67c861cc83d4d6cb69e5a573f55f09f0f1eaecdbd8c3d49928eee95520df6aa0485671c8c9641b19e7', NULL, 1795221922470576220, 1, 1795221922394309657);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x52d32db0de364228c74fa64fb50c9383be30e618c3c831f1b6db973fedc99e62', '\x01f4ab35aee7756e47525fff7cf947d066c6a1f3afaa302aff9789857d609f0e1c05fdfabe43a675d627ddcad5', '\x2e8fd343889c18d4d55d644469905c62', NULL, 1, 1795221922402736380);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xc84b4d79ee59aed8d6c500c23cffb61bedf1bb7da2e3d9c42e4ab492e127691c', '\x0115f83432f6497631083c1e9e7976a8acc10890835ffbbe0d2d788e6a1f26737dba80b87b102c4c827ed03720', '\xdbc93a8b40866e7b9cba630399b893f4', NULL, 0, 1795221922512564325);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x1ecb2376c33584344f18c2831f0c0e274f7be8b06d3f87dfd7137c5532feff91', '\x01de24a70b90b2ee3dff63998e0cef187b69aeb32378f9246e10cc31f96c59399a08aef0335166f358b6fa2e14', '\xb38ebb6f16150e3667bc25601aa0abde', NULL, 0, 1795221922519738295);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x23ddc3eb0f0971fa12b204491d156c13a908e1f7a969af8e0b07170ecf77a96c', '\x01656e3b013ca6805711e29077d806ef891400cf280adbc770f779cf3eab680b1d93f4efeff8d37bad538778dd', '\xd85f693f3fe402d2908837071452421e', NULL, 0, 1795221922527053507);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xbfb73717445dde03785cbe972bc1876c1841303dd967e15495e417e6738125f4', '\x0107b62a07714ac6c92260142188fba4fdc1bfa3567cf449e39722566de7aacfa9061a235bf4fdbd4adba4d317', '\xa20f722ef1232d9721c02ee5767c4af1', NULL, 0, 1795221922534295544);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x14b5e2aaefa3c60f3f31d353f01e687d238c3b22db3a282ca751bb64ec9e920f', '\x01faec56142b5d80d5cdf864f268536e236779d9af4774d5f6c7c7d77f0bd0aed8b3ff96c34c904b6e5f97c994', '\x6ccb3c3d72922be63d84aa0b7f7841d7', NULL, 0, 1795221922541553395);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xc87c3a46daebcde462d7c4790bb33c35f903f434a5fe235cabae1bdfd4fbaf93', '\x013ea33b31aad1619d8d108368e0f7f6062f4a7130e5f4418612ab5c43897c554e57bc19443070dcfe12a3e857', '\x0125bf2d6ce443635f8121bd7463e6c9', NULL, 0, 1795221922548857459);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xc76404ed69cbe1747f15ae500992d8c283b927853d9f35cd00e70709153b9b6e', '\x014102b6a463b67e09b828e785f1fa8aa214abf449a70c5163a80d8173fd0df9eebd3816bb6637f0c2666f29c1', '\xd3335f05d8552d5b19b11ad03eb5c462', NULL, 0, 1795221922482808622);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x78fe0903709cc0e2d10a4266ef1871bd7de80b0081f74ef65dca84bddeb0ea29', '\x015b08e0012754c9f0fdf0d4e946c3e62e7a5961c3c4364645175aa5253df62658ed4773cc2d6654eca5a9223a', NULL, 1795221922587803570, 1, 1795221922497769751);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x101effd5814c50803da3c4ec414d1b8e40756acbd39a1f91a7cb69ea6e55837e', '\x0157154c96fb7f007e8db6cf2aa7697466f388504d32e49117d2607bcd08f618e1dae462531cbd8ed00e1e1c02', '\x83cab80e56bce7cdbbecd1fd67ed4896', NULL, 1, 1795221922505361630);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x662ca4ebec5336f19cf0e402dc5aacd0a648710f24859cb171e5096fcaa8a90a', '\x010abaf84c57e96ddfa54a39b4539af51a2e49943a9383e52268b06be064cdec2ee247d83fcf8ded2423f45cd2', '\xebb71506d79eece995cfea22a35034b2', NULL, 0, 1795221927319744538);
INSERT INTO vcon.vcon VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x61a1af5514e2c3f2013f624024020a0f23915e3717c91915c43d5467351cc6e2', '\x0196d2f18d36ed95e34f29f9d5acf6cbecdaceb6cf2d5ec2d2955ff3cc93733072dca9f2c586b3126e86bd6ef7', '\xd105d070044818d3adcf798b443783ac', NULL, 0, 1795221927329012725);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x6d3e5fe9c7027edd6a06c62f9c02db5120274ccd7d3272f2a5d3ddfe9e74a940', '\x01a802afa46f17a82b93501f4d047a898e8e92f9711b123845132c680a3ba4b1dcedfddca110be56db3c1b0946', '\xb3fbaadbdf50234353ce811fa847a53c', NULL, 0, 1795221927337288411);
INSERT INTO vcon.vcon VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x980b7bb1d6ffba0c52a0edf19e67807dcc2eef36b64f0e6b5eb08ca19f834153', '\x014b45c0d725b0792399d2c032236c3faf80a158a54d27295f0d6038461b4b7b6d6c2939cee146f5b3cea0e83b', '\x2802e3dc5d6d930d2919141f80dafc8b', NULL, 0, 1795221927345802874);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x915fc57e334b663b86ca1448675402df', '\x78fe0903709cc0e2d10a4266ef1871bd7de80b0081f74ef65dca84bddeb0ea29', '\x018b536a253b140f635b4f78165d035afc12ed8c0daccb76669ea290053dc4172df77a3b0f', '\x01dd08a22dc78dde7fcf18f578798749d1a69f75756500642349a814e991', '\x013ad76bf6190edb41928c5c786d9bf57fa94b228316bba3630354d74f4400bd15d4f00142', 1795221922497769751, 1795221922587803570, '\xcc9ae42a58ce0698', '\x01a1b10ffabd675959e7aa2357e8090008ce0f300d67c33382c305996bc872dbe2fe9658f872c59133ccdc861c86e3356b87ec9024b8d941d52129dde8', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x6568abb1bb14efa5cf57089c2a49802b', '\x101effd5814c50803da3c4ec414d1b8e40756acbd39a1f91a7cb69ea6e55837e', '\x01edf0b57b4e87d6f70b177990272fae5f23845f8d82b3e64e3d1b61b1878356faa113d40b', '\x014f23b5b93488bd00321666d3fb754e42e337c3646ec068024fdc25c0f5', '\x01a70b4e1eb6f1f166bc91dcf5a1c7fe8e25d7b1066d10010f703f9d79b55e54e440225469', 1795221922505361630, 1795221922602272494, '\xcc9ae42a58ce0698', '\x016e4e65f09b95fee090ee36d5bbeb977cae906379923a3960b7432515c058076838b365819a4663bd0245730d8aec1e35ec2e6c9093c0802634c2bd58', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xdbc93a8b40866e7b9cba630399b893f4', '\xc84b4d79ee59aed8d6c500c23cffb61bedf1bb7da2e3d9c42e4ab492e127691c', '\x01c4496f195a012ff5219a9947fcc158e8eb1d8ea8aadde153bc1228cb89bf31cfb5da7edb', '\x019cae80b3e6d408e9695912bd3698c8c95ad393c1a7a0743df89da64368', '\x0106f307c6c0badc9de283a5d42e5efb130021b49eb29f42e6988cfbb0d87c3e827872e61e', 1795221922512564325, NULL, '\xcc9ae42a58ce0698', '\x019f93fc05474674e53a8f529eb7c422b5ca3092d873d4ccbdb39fcfcdf226eabd2da122f8fc47a7f27ad9f4d47d8d08ab36906e605f41d0002ff14165', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xb38ebb6f16150e3667bc25601aa0abde', '\x1ecb2376c33584344f18c2831f0c0e274f7be8b06d3f87dfd7137c5532feff91', '\x0167b5ba32097202de2ec0a5c5172064de0fc60da303d93aa840cec5acf770a2d2988ac3fe', '\x010517cd3d2a7779bd85e1a5e6f75d6806552653c646e88324863768fb78', '\x0100846887239f6149c161b9dc33bdaec4f131df9055b2aab98364e7b630321eb9a5de52a1', 1795221922519738295, NULL, '\xcc9ae42a58ce0698', '\x0126effe7f00d36d301b75ba79eb8ee1932b71f09aec77d6d874f8dbbcaf71c9b4f3ab04ad6724a14cd1160ba4a6f7f09dd6151c302aed250897c472e5', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xd85f693f3fe402d2908837071452421e', '\x23ddc3eb0f0971fa12b204491d156c13a908e1f7a969af8e0b07170ecf77a96c', '\x01c3ab8cb628a2743499388022a8608b3547d06b67292990a28fd09c7b3c4901a8350d6cf1', '\x01b48be46c46b6182fe7ebf04418e6ef4a5ca0e8d2bfdd7434c05bc4eeb8', '\x0140a4f2a5ba46dbdaca41db00ed793af859411ee8946a63c97fe0b1dcfdbe54e40df3e4d2', 1795221922527053507, NULL, '\xcc9ae42a58ce0698', '\x012402ad5b05744ba597e0e8eb0d5a29a8fac69b0fa8394760d48dff583cbb4542dfeb2bad19f62d883728dae9c0a7163c4908d84621e9164902770ab2', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x1e14096f90cf6094d34b3536593cf62d', '\xf0d8debaf926081a616815c7d736e51a6b4548270af77d789c18d0f61d6e1b09', '\x01f8f638d9800152c2c7dd784839d9a68f93cd24ee41191a3977059368e0b14d55b0f76e34', '\x01d74d55055d5dbf0c2afaba767f2f612179549c3471afac250cd084ad17', '\x01faf8ad6efd67ed265b438afa4165ed3f6d1e5ecc84a224895989d93409e46b38cc000c31', 1795221922370389972, 1795221922453094782, '\xd7ecc29083d9b151', '\x01a6c88db39adebfd01346e8833ed1c519eb6301e5f2bd7b662f7594afae03cb39b40a1d397d620050c4524e225b00f7852c4390ca8207867e6ab716bc', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x2e5d0824f6339075963a571478702920', '\xf8280a39ad9ea00bdaaa0e868192a67d99a2f8153f288ff7de76a6dc736ee535', '\x015ec03a367ec83d824ca06ea6847f0cd1696277a5ae3fa7df8f0227559fa8b347fb9f7fe9', '\x015ca1d702546f5d95360b82b7ec92aba7182f97f315716f0f9eb7e781ce', '\x0128311eeb7625e179ff81c399dc2a7f87e564454264e6a0769685b874b1317bc4291c01bd', 1795221922360293637, 1795221922444579911, '\xd7ecc29083d9b151', '\x0147d815ce3c6d54b530949a5cdd15c0aa5f00f371000d075faa3c2ff9d9a2f05ca7fddf2d39664d1b271355e3ff3157c3051b086a6699fca7a232132c', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x8b6e38d8252dd0fcd0f7c95cf4fda14c', '\x605da8dc4ff33e0b6d91075a710c0d5cef4c3b49771d120c4a82d9e2b1cb3823', '\x01e08900190742b56ff8866f9adbba9dfa63af19b60cf27dad577ca8db7b6bad15622f4d6f', '\x01586ebb6fa68317e083b0b0862e94c4bb5c0e4e1464c7e5111a5a649372', '\x01056cde1b554dd9e08a01fb64d7564a945fd4b2bdb0d02e96727d3a00e43403b26a8b091e', 1795221922378542778, 1795221922456654119, '\xd7ecc29083d9b151', '\x011371242e17578c77f17b60f4e08ab03ed19bec03ff80bd9ea45697d7805890fa6c85b65a6594a17d6a91ef91dac2912a5d9dca999ff08ade75e040d9', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xd9d531b73d1053120f51f51439a038a7', '\xd071c768b046bae2a4eeeab8adee3a1b5fe7cfd5e5ed3e565b0503e215b5d179', '\x01152db29c8cf609a4f1e1e9b9815f2d6e4c96156ce587c3ca43e8328d69a7e64cae0e1418', '\x0173270cb2ee8901b9b36c2eff437e07a774c0014f9e85df1d91cb631cf2', '\x01cc3ece008502ceba14c203d06bf367b242eea8524f2cd7439728bdb611596a05346004bc', 1795221922386155018, 1795221922462822006, '\xd7ecc29083d9b151', '\x01395fcf74bdf2d4c9a661aacfc53aa435081e2f380159946eadc9fb0e2e5944d02f6f51b1bcd859e978001a6a33164d554bc93b24b854b44896aa1461', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xa20f722ef1232d9721c02ee5767c4af1', '\xbfb73717445dde03785cbe972bc1876c1841303dd967e15495e417e6738125f4', '\x012bc24fe89c3491aee067664c8261bae0ce366cb6fd32062efc30b10dc0e0b62b578839d4', '\x01b4b4b729d0cdc11b59722f9f4f25cd64de90743e6096dc6856a8a3bc05', '\x01a1b9e099efa1ff5b7cd06a815a2359ba7c4699fb0dce6702e6b850384c80b14939d2617b', 1795221922534295544, NULL, '\xcc9ae42a58ce0698', '\x0147e94372eccb1847211d942a90699fa114d9db615c61db4ee601f9d90eecdf7d67d522f1539ecfa326a3728e5949e9a597b4f947c1363989e44fa9ea', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x6ccb3c3d72922be63d84aa0b7f7841d7', '\x14b5e2aaefa3c60f3f31d353f01e687d238c3b22db3a282ca751bb64ec9e920f', '\x0175ec4f22a6435969a50cbc14a176201e11c0f92ccf68b83886b26e3650a7cda2bb91c477', '\x01e4ce02f1d91a58d0ad012beb021311f434f3e1307683551f4f87352296', '\x0163a33e0d967d77eea12ac1931d71dfa5f1e43b8fe9174fc7b640fa5ee5716c422da93943', 1795221922541553395, NULL, '\xcc9ae42a58ce0698', '\x01087f02f40d9d60dcebb6af14c1c2a4e189f13fbf18d79f0d3a468b2650c402dff25670b7fc2d121b73d31fe485d8206eab644adae266197004ddd224', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x0125bf2d6ce443635f8121bd7463e6c9', '\xc87c3a46daebcde462d7c4790bb33c35f903f434a5fe235cabae1bdfd4fbaf93', '\x017e0c657a2a596df3b8b66ded98252680f7609139541cfc8f22cd4419a1c718670f6b1eef', '\x010a7cc8db3ffb21528d781bf1553383d453ec38a22646e53731af5020a4', '\x01dd846635380dc66346f32eb9193fde4aaffdc09dc362146179c1978f040dde76f761d3b7', 1795221922548857459, NULL, '\xcc9ae42a58ce0698', '\x016e8b487f2a079aff66dc3266e102649335b539637e00e1472f338df97c9a8fdf100da9a1e39163c0981d307374689fb67f37278311575437663b24ff', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xd3335f05d8552d5b19b11ad03eb5c462', '\xc76404ed69cbe1747f15ae500992d8c283b927853d9f35cd00e70709153b9b6e', '\x019a566bf18a5876ebc076df2f48fe84ac48db1155843d2ddc2485cf1a95b25792b6603080', '\x016f8453899d178448504b2a28c9e48d6bc8f26e9e34fd64567413b717e1', '\x01893497637f277f3f24c4693c90dd3d7752dab3e2e5950cd560568206974f57328a1a03fb', 1795221922558255813, NULL, '\xcc9ae42a58ce0698', '\x01eb5d8c1b4bbfc44f9fff58c0e64bc3c484e280d61c38dcf8fd52a1a9b52fece4d8c4d258b3fe5c160529bef74b172a85bd00e94077cff4c25290c5f6', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x83cab80e56bce7cdbbecd1fd67ed4896', '\x101effd5814c50803da3c4ec414d1b8e40756acbd39a1f91a7cb69ea6e55837e', '\x01ccf99b536b7899c3bc95ac28ead0ac1d3012db0cdc58825c19ea9ed32773ef6b8c3741a8', '\x011b8dc7e04d83c7a5ef340c7182948fcf76e27d8ec34c6781aefae6855b', '\x011d31ea7e4df17e3ab560e53c17b93abd3d3c5875ba0bbde01b8bf2e1a907b8c006a5eddb', 1795221922602272494, NULL, '\xcc9ae42a58ce0698', '\x0195a66a8c9657b1da21c16fc2e58a667cef92546ea9016a10f21d9aab5e255f3ee5cb4d3d72beb1d1a5a46c2d57725a24a490d2dede69d5b2d2007e1e', '\x4541d4db52f03439ad4629d6efd8d1e6', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x80ae40c59f2e30f05368491c237136ba', '\x0f1f7a88aba00de84940912bab521b07d86d0030070094fb6dbb1feb340041c5', '\x0120523afb58384b86eece48cf570eb69b05c9e6b1741a3353d012e75c16db732fb04139a2', '\x01e2ef2b6f578e91a5a8269c7705bf93a1ffd68498866d393245817e0b16', '\x010d3d2bb209b64d0a3c27fc6e98bf559fea58afe2d38a01ee01521581202f58017850f6ba', 1795221922394309657, 1795221922470576220, '\xd7ecc29083d9b151', '\x01cf6fe6e8b459f4e0cd5763d1932c98d8f3e68d5a9a78b91e0c6b809dd4642631fc5e5af53ae397cbf99fbf033f771c230354ca841fc78984cd94fe87', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x2e8fd343889c18d4d55d644469905c62', '\x52d32db0de364228c74fa64fb50c9383be30e618c3c831f1b6db973fedc99e62', '\x0127317c2a436413cfb7ba161adc4246a09b48c4abe85e93a6b5eedece2785df1eeb114750', '\x0198a99d0272fd4ca1f0d610fd69f6553893c7531550a8e0b76bb751f395', '\x014fb6809bb5f065ce671a5b9c16475815b23f94c9dd2cd239efc8f91f391deec78ebccae8', 1795221922402736380, NULL, '\xd7ecc29083d9b151', '\x01912289761ba134cfea5d2c51a3173538eb27a392fda3cb42748ebbcccee79b5fe58ebf50e21b6a16aba4bf6d672de36fd7d5df1d6113ab8d1f6e14d5', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x067b77ef44015f40c7d3a089367271d2', '\x8667eaa98bc91bc6327a15c976b255dc25dbec1d013cfae3e4eecd8e2cbaa6d0', '\x010ef89b6284e232b2d5da9be2532ec01970144d694727d017d4d613993589822349d198de', '\x01b789c5ee6918968417c5710643b2677ecc80a14fb96c7859b99cbe4a6e', '\x0157097e60d80e4aa80eb9fccc1a618e5d8f9f5b90d07f4030ed4af934d47171f3a43ac896', 1795221922410203491, NULL, '\xd7ecc29083d9b151', '\x01a293206af2e10e4df8068a0f9d4437169491fd5c2246a4f7d0279816586475725931ed7d61446ca266b9d75f5696af5665fc2fe6e7d10ec341756303', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x2403f0a6878e922f4a3d1f4b6016fb0a', '\x2100248ba447ff0e7c9e969dad8cc717800fdea4bdd178aefb1e97eabd9a6717', '\x01d8c3fa8943c36fcdeba968e01bf741932cdbd06df829a4a0f0836a45fceed7d5506ce1db', '\x016c45985a5687f136f82c4b02c00196b69045af4b45945c2011e6d9153e', '\x016956ba690c1006bcb3b9a6d152fec5663f5156bac85278ee309bf227d4c3cd80bb811300', 1795221922417662768, NULL, '\xd7ecc29083d9b151', '\x0198af4f31fff5e049636570230cc7a690f1009ea3349fb2648a4ae256681268f9d4e84bc685d7dbe66cd06fd328ed951807b2c89c4b4708585a53a7c7', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x6d518f44a8d45b3ca9838984ebd3d6ee', '\xaf7ff0d445fdecdc2054b91e819c3fb23fa5f520fbfee15bc975a6faa9e318e6', '\x015d98691015a0239ed704849d5e4fa527ca091cb3f273a5c277367d6afa27a963fb35227e', '\x0182b4dc03c51e086ebd2e1d578bc5889de4d688eb8785a13ad839b1caf8', '\x01a775d21033aad93784bbc6354a78d2846478ea20df5d25aef51749ec359eb996b0721bcf', 1795221922424952426, NULL, '\xd7ecc29083d9b151', '\x0123712266fb62c8a8d22c05ed76a8702f672bcd8c2994e3524f00032b48c9d5c681a4049de1f92862985d75d54b2cc927852bc14ba2798970c7eb7ba6', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x1e4a01953af6e970e9aa99eae10718f1', '\x451975f763ddf5bed1aa95021f65b97a5b09295f21c63e6bfeb6cd49e514a440', '\x018f2ebe7c46b231b99b1f6ba1f1862f660b72661a0a548ae20ce995ff92968ddd0a1e69b0', '\x01707f007f41214306851520c754b9edd71559435d90ad63020165b0a121', '\x016c9b308a0a158d0cbc376c3f19cec8e7cc74bb377af5c6f41ab2acb2ee246c1ddb7e4f9a', 1795221922435477900, NULL, '\xd7ecc29083d9b151', '\x01cd6fd5860f40323513fc2c1ed1152576336a1bf2af0367e2d67319604c143dcd80ea4358e2fd150aea014b1a1d1f777e44ca4f2356d83a5659140046', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x085c1277adb5520190cfd9be405a3f42', '\xf8280a39ad9ea00bdaaa0e868192a67d99a2f8153f288ff7de76a6dc736ee535', '\x0124c7825aeb9c94fed00326f9408599564b98ceab48b4bdee6158f30d94c312dae4ee82ca', '\x01489531e1cca4418906c8dd18a574248ddc2fa7ab8723d8b92fa888edb8', '\x01aedcc579a7d0ed091c92607a47863a7ccf69d9ac9d6d4080369c0dbf1d6c921fef14a0c3', 1795221922444579911, NULL, '\xd7ecc29083d9b151', '\x01a55e4750de5b9e7659842dd74f7b9046cba650660f19b136493ffe647c42ee0eec397aaea414fd0e2d38d539036058b970e1605b4711657190eba0c8', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\x0cef96250da8cc262cac0a01da8b146f', '\xf0d8debaf926081a616815c7d736e51a6b4548270af77d789c18d0f61d6e1b09', '\x011e9c08c31b839025a4341af0da4ead3698467fa245729d638e3e842ce01ea6465327b3a0', '\x01020458b3296695a23a7a3c2c64d29dfa647c013780c07139995f257549', '\x01a26b8c92b475889ede511c22829961ee444bc7da2441887d4a6520fd8265ddbf83520e34', 1795221922453094782, NULL, '\xd7ecc29083d9b151', '\x0143a2dd1cc033781bc975736f1cf7a6b723677203f50ffc3fc7ef025e23a8da8e36a7df75be833055c8ca94d92a270632ff02000295d873bb076069bb', '\x8b540aa7a52801c48650c355eaa7c1b5', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xebb71506d79eece995cfea22a35034b2', '\x662ca4ebec5336f19cf0e402dc5aacd0a648710f24859cb171e5096fcaa8a90a', '\x01922adb08dcbd99d527c26ccb76b5fb270229d5d0a23a7fbc74adc64686b587d1d1aaca05', '\x01105811ab756d6242b79f2bc708d64d3a1873750182f6b5368bcf84a433', '\x01144eb2490178e51b846e61a6072cba7b6bb525c306541e04e579bb614e6f7af88e619d94', 1795221927319744538, NULL, '\xd7ecc29083d9b151', '\x01e4b3c71f0466344bed2170f49cade3f208bec2b0f66e3d4355e288407583b4cbb782afef5b4b9f14b9fadfbf3cbd5d0600ccbd035025c3758633d628', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x9574f6e0ea676332f3241d3fb4f187ac', '\xd105d070044818d3adcf798b443783ac', '\x61a1af5514e2c3f2013f624024020a0f23915e3717c91915c43d5467351cc6e2', '\x014d0ce4e590e5e7a220a7544f6daee20063bff686c499aa2f9d6f2bffc77dd188ca4007fc', '\x01276e901d88ff400f798e7464d31bbc7b1eea23890e613cf45d5012faf4', '\x01ac12cb7373a0117d72b2349af8ab82b4e2bed6eb120ea3be96353c4e7a222f5de274263f', 1795221927329012725, NULL, '\xd7ecc29083d9b151', '\x01ffb83669ba505d2a919d971e599a1fdc095dbd9b6f8a70bc633430ad9c481828fb9bd22a32b71b1934de7462246c203cff651e5a185d7c2997e4b3fe', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\xb3fbaadbdf50234353ce811fa847a53c', '\x6d3e5fe9c7027edd6a06c62f9c02db5120274ccd7d3272f2a5d3ddfe9e74a940', '\x018046d19cff5848030b7719dddd780c52c45bd58c5d4e9e2df3f9aca7cae703d8fbe4c82e', '\x014e0949e29d0ec5ab50e608e88e7de2ac35546158d4cb2bc2e95dd0eced', '\x01043efa7aa386310eebe63c44bb419e4bae2101c9bed02b3385d2277c58a7bfe18370894a', 1795221927337288411, NULL, '\xcc9ae42a58ce0698', '\x01a078594582097b3b0e6fc74f91cff19c44c268a5e61ad532616fca490b2a37d3ef6558a8930fcb626370338a467cc828f6b5ede0a4816aabb025fe2c', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x21f248d3e6a94313c7231584f203c63c', '\x2802e3dc5d6d930d2919141f80dafc8b', '\x980b7bb1d6ffba0c52a0edf19e67807dcc2eef36b64f0e6b5eb08ca19f834153', '\x0112baf9cab73bc6f1ef6390ca1bf230187250648d80e4393a90482b75e167fd494cbb50c5', '\x01c9910e7ca4e20942732cad96e537dd5f08e0942c2af08a1b830f695eaa', '\x01c230a5f701a72d30b124d6955986ff972bc7f86e91d9b4a3f67a47bea0bd783967caa02d', 1795221927345802874, NULL, '\xcc9ae42a58ce0698', '\x01b058b7202ce17124854b31580802213b00e3c931d0b89a1a124294d27f016886b54df4abc55a4fcdab002797739e58be528dc0401cd0c9e8fedc4d39', NULL, NULL, NULL, 56);


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


