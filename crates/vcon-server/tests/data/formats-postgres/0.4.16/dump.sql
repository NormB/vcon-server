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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-25T01:15:30.017136476Z', 'cli', 'tenant_create', '\xf883d520447d4b2f27c10c29aee92dd8670d5f3cbbde145213fa829f0aa87253', '\x01a92b300b4d909e8b7e0fa9bde745870578333d6801117a6017fec5c6c0cb7344420c43191fdf58022ac603bd342ce04ad903f760c89dcb46a7d74447', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-25T01:15:31.939346776Z', 'cli', 'key_create', '\x838f33cb6f4fd06ec27851585b296c58d6d615ccfd211bcf3bb22dcf89fa8e36', '\x019732329f4d76fc028ad8738deb73e0049a15d647ae3f58d14e4c11e95c669144e170f6412be427f330d983fa', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-25T01:15:33.783539443Z', 'cli', 'key_create', '\x5178d49220d03658bea0a6ab91c2add8cf44c94afb0aee3d4c26309d072d232a', '\x0189dccb7c6059dc5f51ae7b586c9be7e0b4a4d2f66812192e406805b1203663e9474770d4d19af14c01cbed29', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-25T01:15:35.743441862Z', 'cli', 'tenant_create', '\xd4f765d942eb6c13e3b151fa4fe11df0287740f604681723e2c09abcd8014d3e', '\x01a75c38845f24b119bd1870bc3088008eb758012aa06c17ab2561b139300ef8f1f46051b69bb92f601e5623e1fee5938665a134aeefe682994cdd5e00', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-25T01:15:37.724821294Z', 'cli', 'key_create', '\x90ab26c85b837ebbbdd63fed25c2c8012fd619cde441fc7c62723441509e125b', '\x01700d3e48dc2dfd2d6c85a3f8322a76ea66f552c2876ab38311fc6dca8b82969447ee191e03413c4e8dffb14c', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-25T01:15:39.681509438Z', 'cli', 'key_create', '\x6a31cb6167d2c877ec495a792187fa4f223eb9b5d2e8ee337ee73c32b4ad34bd', '\x016e83970d31e6dbe53934e72baacb1e61b697865721444205e42d28331a2fae464b98e46d5f1fa0d0a31fec91', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-25T01:15:41.59937131Z', 'cli', 'key_create', '\x76ef781883a7bbd08c46e4c5e9eca84282c9349b4cb3a3da504f55439ecaa3ad', '\x011487b05daa485d24d2951584644952bc40a8b9a6cfa80ec0a1a2bd7d588e72d65f628bfc6d7bceb65562f104', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-25T01:15:45.213382445Z', 'cli', 'user_add', '\x6aff566bbae5afdd1fadc579fc62ad7a523c2dbb5dc8ba73b592316cc93b68e9', '\x018f708a45e50eac9ab3d915e55ef52347d6e8beae696629abfbf52e59d324f0625aea0dedfc152b3327226359f553c70f11f4c700fff6861efb764d74', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-25T01:15:47.230573299Z', 'cli', 'user_add', '\x034835b6645fe7c27c70365adbab0849cb666c8ee7d6a5dcbc38ade086c54da9', '\x01c9104d25c2ef795860eae2afb4651e992ff83144e55a691440a5aa60dc633f2a813b1ab93367782532262f57d947c589873d261702ba34c864fbd14e', NULL, 201, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-25T01:15:49.29593751Z', 'cli', 'membership_grant', '\x7ce539629a0ed9b4484809c40032634576ea6b5fa8c9664c49a9515e4bf2b9c1', '\x01f4056a9249373c83619c42584cd4b532c98a4e53caf999050aad2219537201c00b232b9ec4afb3840ca4dc11057222fd61e65ec3572d000b589964ab55114959378cb7333169a2be879212f7676ca8fc2fcbf295b6613672eff3d6f342cb8b634868fa74', NULL, 200, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-25T01:15:51.357997393Z', 'cli', 'membership_grant', '\x1696ae50bde38e4d5bb427f906e3667c9cd010b2a77c7370511ac960076a1b65', '\x014e1ea67298409cd065ddabcaba260b1804e8abb0b37bea80ea9957c355a49c937d8459e47d2aa517d3abad44c3ad3906a82592ffe98a3e8074db1df5386d7572128ae22d2145f09f4d5ab374588b6a1cd41f116943fa34bb649f9ce79901ea39b08612f04d', NULL, 200, 1, false, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-25T01:15:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 2, true, '\x2c6d8c1f85413d8d');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (26, '2026-11-25T01:16:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 1, true, '\x2c6d8c1f85413d8d');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x2c6d8c1f85413d8d', 1, '\x01bf521772fd02ca2ce7fdea9f391d5f964f4de0d14b3e9d12e85b968e976104bf70630b313a13aaa90a8add6d23d70b5e9bd274f299c6374c4d55d222');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x2462475223ca97c1', '\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x12980634a85453f7520877d2342f9f5d0d4e2f94a4e741333fbf31f915feb2f7', '\x0128f1f3cfc988ba871bf253f5f6008bb453d02a0ea9a799bfaff9bcd146be66d4', '\x014abb8f9abaa42ae4299fee4cf858fc45d79936a14ca3512d8ddba4696905263be000f61cd4626dd907431d', 1795569333783128119, NULL, 1803345333000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x1a0ca98137645b09', '\xcd07f74570f0b6fb2dadb7f28cae0780', '\xb8e2226f2a6071c7fdc054bf799023ebfab8fdf7ceacfac31d2e299514c079f8', '\x01eb305e7436c52c6bc535651f2281e3de97a2640738c8d649921ddb3bee4aee45', '\x019bbee7be739c34716674c70eced6e2f024436f867b59701e86e2640683af803e1cce3d45af0afc1cbbf740', 1795569339681095123, NULL, 1803345339000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x00bb80b1958b23cc', NULL, '\xaca159926fe29bbfd252682bb1bd674c905f89ff9224def26857ce9dc70ce172', '\x01113ce46e2de8406f7c9f5a1337a20c090df2a4f8ea6f9134d277ebfcdd0ab75154', '\x013fa04f1ae35733fdc1fd40d8b9c04f2c5c65dd793b2953d5edd4840a0a0b5d', 1795569341598483921, NULL, 1803345341000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xf382bf2530d2f13a', '\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x31a5f63b82912abcc3310556fe729b729a3bec5d4da0c0ddab447e26969f1abc', '\x01b61acfc2188992e9d2ac03f406e6b221957cc7d416cf3287507f16f1be634cb6db89188dce87350b6dae5374b981d13a3780c827ed6cc25336', '\x01044aefae807b8f23b8d7f97f0e7353480314f9e82bf6b9590d22f7e2ea8d9e100b903b6803328bb6eb48f74864', 1795569331938774517, NULL, 1803345331000000000, 1795569360000000000);
INSERT INTO vcon.api_key VALUES ('\x6c87a66d73803f47', '\xcd07f74570f0b6fb2dadb7f28cae0780', '\x8e410563fff68ee0f02db3bb8a67568f50e744bcdf5e983cf626ceb7b804e4f2', '\x01ca411fe1521ee1a8a43145831d9d7c8a4886374384ef73a76b7de76cb105801f769592ebc63bf566d5f92d8b117cf7a7ac64c55af561cdfd9e', '\x01884cd3a8a1a97ad2baa8482f1508cf717faeaa5d3c736d3de54fe835b3a09ba70ed5178ddf21b67f80846f7653', 1795569337724412100, NULL, 1803345337000000000, 1795569360000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 2, '2026-11-25T01:15:31.942836433Z', 'cli', 'key_create', '\x9b116ea19f0b3249c228e52e1fdb58162e2c9629f47d0e8a33719c07adfd8a9f', '\x0115814c4ed81656580084bce936be30b850cec5234181042c772cb2ca01e0b57ba3c58ce03877698f9281ef7f', NULL, 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 4, '2026-11-25T01:15:33.786922664Z', 'cli', 'key_create', '\x092781af6bdf266c0094f97d394e121e19d0fe1bcad3c03670e1602e57005ace', '\x01dd121660aa44420ab19f529d6b0d9dd899c91f6f523462e45c26032bf7e4de6ae2e3574f95cc91903dcd3462', NULL, 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 6, '2026-11-25T01:15:37.72819633Z', 'cli', 'key_create', '\xb46529d7bd824a45c9ca856a45593e1080aad24c8a233bdac1e6520890c7239e', '\x01fa76ac2a820e5973f1b3d5eccaff1060bebe684d66e1dfff8585c63d9d8f89f6ec57f35e52588fdc236481e0', NULL, 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 8, '2026-11-25T01:15:39.684789567Z', 'cli', 'key_create', '\x0568ecda3c01e37afc0ff67b932eb3cd08aa21cccf6d1afb3ac545435a2cc2b0', '\x01a442b55a970b14376a11f7b06bd62a4eeb4cabab7ac875b37737c353937a99f69cdf8cd58debb33954095c32', NULL, 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 10, '2026-11-25T01:15:49.299583889Z', 'cli', 'membership_grant', '\x5ed4a7da83d3e93abb9e4f7e5db6531d8cf4e80e7c5abae2c594effad36edf4a', '\x012fff080ecaf230a621a9fc6489da8fc7a33b5807d7564c3ae4f490446b7543e85072cbab8daf190c2545c26601869fb3ac65ef12e20dfc59fed311026feb6cb35cf028c7f018e019144e9a8ac9d4e3cc52c5b67c651b722b9666f71d77195aa6e3bc8527', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 12, '2026-11-25T01:15:51.361853725Z', 'cli', 'membership_grant', '\x14ebe1134608fc73477a429f026170e69ddf270c2d481e907e531ee3eb375bcd', '\x010ffeb3699ac5795ead4a2a4a72fb6e076a62a7af4440aab231c9569fe4ce770717090980003bb516ed8a07a11685e97bd5b38b4c22e2abdc6fb01a0e319f3fcf2cd8536403072c1393b552b3da6ddfe5cb4b6ee88f3d2b4498d4030ff4931c02d101b32fed', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 14, '2026-11-25T01:15:53.913236574Z', 'f382bf2530d2f13a', 'create', '\xb014e2fbb44798d7a601d729cfcdd96b8b34758dfebeb3c0ccf7620b7ae0f0df', '\x011080ad040da1ba0ed1d845e4392d3365f1574f5d6fcb1b18dbb57e40d5f39f358aeb21a4c39c3cb5ead1930e0f6040a9b4e7f27fa1c2d2895bcbf2d8f0980b76', 'bba95e756b306de519228a93f8dd059f', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 16, '2026-11-25T01:15:53.980427094Z', 'f382bf2530d2f13a', 'create', '\x1c2dd9ded98d7cba3cf06799bf09362e401fca8e8879e75de449dba4f495df87', '\x01b09685d89b9ac2722729dc534508380abbea31e73e7c9faf0545dd32a88331d69d64bb8ef5db58f678fd43682d105d5f0dc0322d52b42add8f7dc619a7398d60', '2cd8c79f2191d994c775ead9ed87b9d6', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 18, '2026-11-25T01:15:54.006185328Z', 'f382bf2530d2f13a', 'create', '\xf4705bd2f6eb2f35dd4c1e84e7a4a381945bbf163d4e7ae842b3c1d8f2f28f93', '\x01860dea925d377b9461247f6b76f12e44742dba1bdc80a16db80bb60be1fb6fa886b3fe55939f9cc9205446d65096d0c4feaa6c907ecb12b7013bb961f0ba4d89', 'ccdafa572aa15e852a9843bcdd4373c9', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 20, '2026-11-25T01:15:54.030820545Z', 'f382bf2530d2f13a', 'create', '\x6a59acad6c453282f86f4a165de5a42f0837643eadc76f836850c2af4b513151', '\x015fbcd8eddac23d16db93a804c9ef81525e6e3dc0b6357e1d84c5c75b01f3f45231da15bf267582ac92b92a74a161e094e387fcd87baf9be5c1128e347ab02bca', 'e09cb7464b63c54d4f4dc218b6944e19', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 22, '2026-11-25T01:15:54.052840097Z', 'f382bf2530d2f13a', 'create', '\xedf108f0e446a99f362879a165e89f428820a3d1a290c48e04a6a4478ea59424', '\x0133e18cea4fe12b163b4352c0f88eee982056712d022cf4b4f61e5a826a2f42b43264713f76c3ca97713126e27e7604ad55b073b0d3399ebb338d3c5dc5803abd', 'b24391dea4117a184a6cc9a85b89c4bc', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 24, '2026-11-25T01:15:54.085589312Z', 'f382bf2530d2f13a', 'create', '\x70c75f04ea6115533a50be94e4beb0331780999c4f8c3f294ad323a5410e6b72', '\x010dec0e0f6357407af4819c5c8731e05da5ad5a169ef66c47bb3edf1573c6cb423c2e8897550640495416f903b3c983a114b051e0d8752bcff0b91448d8e8ccc7', 'e5ec09790023fdf18cd114c5c15d55e4', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 26, '2026-11-25T01:15:54.135213811Z', 'f382bf2530d2f13a', 'create', '\x2745aa8119afdf39246030a6aee38e63d9390a2974eb055cc77acb07df955336', '\x01aab232dd391408a104e82f05e99ca06e16543266e8ba59e8d168f99d629b9e68f7542208c21f13e8f4b3cd2b538c834e0213da1263df2dd95e203bb507b77a1b', 'b9818a16dbab1aab7eed824bfe60bcaf', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 28, '2026-11-25T01:15:54.19531865Z', 'f382bf2530d2f13a', 'create', '\x57723b14a7685dae99982d2720ec88b3a3c017218a094e8903cfa79fd8e809d2', '\x0149d50526ba41506d24140635282876c59ddfbf9a84bf5d0e218cd1f35292aacfa5e469f0a6f7d33dbd5097f9be885d77dc6d7b7051cde1375e0d5b35d84d027a', '8a7449ac1587238f7e501a9cc41ae642', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 30, '2026-11-25T01:15:54.254088157Z', 'f382bf2530d2f13a', 'create', '\x0027904a731a01602a88cd2fa76be1583c1def248a2a0b631d4cc835d3f239d1', '\x01e414b9eca37be9fa6ed90f3a6dfe7563dc66e07b2d29857e3cda5e1bc5d6adf8c6af6caec6e5086a604d56d2efbe2ba6b4d65536939acef36494e2d953296bb0', '9ad2b8e68a360e09f004eed2196d77ff', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 32, '2026-11-25T01:15:54.311340328Z', 'f382bf2530d2f13a', 'create', '\x01ad847132e9e8de7db7627e6c310aed6302545ff5cc8fb21b168d743e0e0164', '\x0199964ea07a597b1613f9756f54e1ffe51424f2ae839f85d727a7b1650618e89671852a9a870b6b969a443a5cca958361a98a85b883e9434634896b1931e4e50b', 'e981f67ae55ad1a1a8f81a05f001bdfa', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 34, '2026-11-25T01:15:54.33952684Z', 'f382bf2530d2f13a', 'replace', '\xb014e2fbb44798d7a601d729cfcdd96b8b34758dfebeb3c0ccf7620b7ae0f0df', '\x01cb4fbd6507778e3fa9d97568b4e22844c28bb5e0c13db50d8357597a49c40ae200cda259af9972f32bb54860d216a531f291325d546e1fffa73d99ae49ba1e2f', 'a53a037d231ab9e23edd702a948c940b', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 36, '2026-11-25T01:15:54.373834997Z', 'f382bf2530d2f13a', 'replace', '\x1c2dd9ded98d7cba3cf06799bf09362e401fca8e8879e75de449dba4f495df87', '\x01e91f5702f4be5791d2ad3ffee2b6daa39f1d10fd9b83a3531d80f8946455bae7eeedd9aef22fdc68421adf523024cb92daba9a5268cb52ec37ffcdd9ef3e42fb', '4c19cb6723d6585b7e16083583d3c02f', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 38, '2026-11-25T01:15:54.379813672Z', 'f382bf2530d2f13a', 'delete', '\xf4705bd2f6eb2f35dd4c1e84e7a4a381945bbf163d4e7ae842b3c1d8f2f28f93', '\x019b73bf394095bc0a1097814db5f527d6457fa55fd361994fb704e948610f9d217e5d8a1ad14556a445e9f0cfd503fe590acc412634640607041fd097efa5817f', NULL, 204, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 40, '2026-11-25T01:15:54.38961304Z', 'f382bf2530d2f13a', 'delete', '\x6a59acad6c453282f86f4a165de5a42f0837643eadc76f836850c2af4b513151', '\x013ac60da968c5c547afef50262a8ba4c685cc1b25665786dbd63dd4303488743b6fcee3306f2db3a2c0464946e91f97f61e6f8a3eed6a29b8c6fb3f984b71a34a', NULL, 204, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 42, '2026-11-25T01:15:54.40096075Z', 'f382bf2530d2f13a', 'hold', '\xedf108f0e446a99f362879a165e89f428820a3d1a290c48e04a6a4478ea59424', '\x013754b7cdd548e27f76d0a48a524fad3fda71e613d588afcdc2a166307ac9018d9d6cf6df01c00b43ff46d2d045d61fe8cf3b2ee2440b533713d5205aea19ade7', NULL, 204, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 44, '2026-11-25T01:15:54.406166684Z', 'f382bf2530d2f13a', 'delete', '\xedf108f0e446a99f362879a165e89f428820a3d1a290c48e04a6a4478ea59424', '\x0122e7f802268bc957d346893ae2a1d5549647eb1362b06845989f240752434725d9220609586c1ad76551179fdef0745e11d64a9714559fa3ce2d091691ac100b', NULL, 204, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 46, '2026-11-25T01:15:54.413300849Z', 'f382bf2530d2f13a', 'hold', '\x70c75f04ea6115533a50be94e4beb0331780999c4f8c3f294ad323a5410e6b72', '\x01808b43f119efeda09cd0a1980074088a1d75a51096f67c17a5ba6b4c9fad18527f66d3f855ad76e412c726e146bb2827dfd3b34bee87cad03f957fa321f504ca', NULL, 204, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 48, '2026-11-25T01:15:54.438157343Z', '6c87a66d73803f47', 'create', '\x1e340a24e121f04890fb336f83484f79b0c79c4ce422f686115557128b5ad891', '\x01243edd6097aaf18c9ca9e8264ea39faa3e101865c6f590b31eabf29e6598156b8948c52668377f12c475a2d50ee376570f7c0d759c3acf54cb6dfa0355623b3e', '026ff32c100ef3ed329efd0f2fea1dc5', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 50, '2026-11-25T01:15:54.465744771Z', '6c87a66d73803f47', 'create', '\x46de4cf5fc5d5758e671c1f94e01cbbf3b7ddca05ca0ff46a7261b256111b8f4', '\x0187c4b3ac9679d51d2a30d9d70f991a49573074404de394c3ac039bfdf05863ed4cfd5b96c06c2f66b7d171050f24fd78c9293d556e21bc5a346047c48dfb46ff', '87e777f7778203850c1071d2eaa3d839', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 52, '2026-11-25T01:15:54.521656146Z', '6c87a66d73803f47', 'create', '\xee73a8dee41feb80f55dbb78d798a64bbfc854500192902f5d03591ef8756923', '\x0104cf054046acccfda25993fc606ac63a9bfaec4d2900d7bed0fbfdb8850f5ea13987f566329ba3d7a8000b3ce15c6b5fb4d3e77b8a00f4cce3d1214ed5dd4d04', '8ad67cd52678b00f97f450c843cf69cd', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 54, '2026-11-25T01:15:54.571394772Z', '6c87a66d73803f47', 'create', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x012cd4368ea49b6a08365a5bcdb1758eab7c1cae2c5340c6a12ed80a610e6046335a99992ce843f122fd5b4e966b1e202597e952c2f54c626805a4a8919faf10dd', 'b6f94cc0086fa6f99fc4be78caf594dc', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 56, '2026-11-25T01:15:54.616113623Z', '6c87a66d73803f47', 'create', '\x8b2e6aaabda7e70f7a52d7b048ba8b47b814a3179fcf41e16b1b2865169b9dee', '\x014e6d18e1cc15d220deea072b0f847909edd9f6d64eb5a0dd2e689ec2060e280111d88aa2c7a7f51a87da412d64f32d9de5a7dcc44a39987567a38612311c96e6', '784b591d42d9de0ca50bc46e88d2316d', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 58, '2026-11-25T01:15:54.650400948Z', '6c87a66d73803f47', 'create', '\x7562d57e9bbca73ea85c40708d9a3aae5e2a8edf024da42d7111ae7b7574f750', '\x01f04d65002210ee5e55bf0322e32506d40c4668462d6a612cb43dc2f68c15e850d3d408319123bfb1433bd9e9553dabbe30526894bc913aed9644a50b0e6f0005', '55c5c72607820b101c4d7ef5a50b6433', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 60, '2026-11-25T01:15:54.70045699Z', '6c87a66d73803f47', 'create', '\x9a1b01d5d7d8eb94cb5b3a77432cac654043e681d53df28fdf43220a69086b26', '\x010eab42186017448b92e00cb17f701b793ccc2f5a9552d6deacfc5062d3964dfe975f10f130ca887813ea32e65166aefa18c3ba9b639ca6ce0bac200edd20f096', '78de5dd45f522aaec1dba3da9d1bc799', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 62, '2026-11-25T01:15:54.753593329Z', '6c87a66d73803f47', 'create', '\x311d6b021088213052d480c5104601e7d4ceb76326263fbad7d2d1e9561093f4', '\x01af0074e05ac4f920f243013c809fc85a823ba2248e5e868ace3a1821fd603ff46177032decb3afe2d437a00b00a3f15f0d4ee869bdf24658bb60a18450331476', 'e01baebf8dba4c4f2c7a568ce463ff41', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 64, '2026-11-25T01:15:54.778116638Z', '6c87a66d73803f47', 'create', '\x82af17960feaed69128dfaa49961e224e1a18fe39bc2d6400803ff00f261f0b0', '\x0171bfd929c28305584975cf9d56bbe0cfa9311a478ee8f5db4d23ce4522f504765bfc1fd17acfb12f23e699c9497c7daebb62d71c17f35e47392d476e9679b300', '86e8adca8fd64b6176f675c671a5daf0', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 66, '2026-11-25T01:15:54.808775928Z', '6c87a66d73803f47', 'create', '\x4c26728fd69e371c2e314fbd7a354af22c86a79fb90e422cbd98a78a5e6838f8', '\x01b8d20afa897ce8857f0d4f9c5cf81460a136e4410dbeb3a7704ebab029b5f240b4252fabb4c7040c7fdc2363764c761d6b8ee5c9d9ba2a43f06dc31862fd2a24', '91ba50d1aed0a393b25f344c3aeb5972', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 68, '2026-11-25T01:15:54.840780347Z', '6c87a66d73803f47', 'replace', '\x1e340a24e121f04890fb336f83484f79b0c79c4ce422f686115557128b5ad891', '\x010cbaa90e991f54cd6ee5b51c461b432baec3d3540ce4ef146ddb1ba9d3bc76fd4d6f8f0e0de7fde7b99a734b2462151b824443df2f2360a489e0ab4c18b75764', 'aebebc3d08b49be947d3c467b72c13a9', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 70, '2026-11-25T01:15:54.85178716Z', '6c87a66d73803f47', 'delete', '\x46de4cf5fc5d5758e671c1f94e01cbbf3b7ddca05ca0ff46a7261b256111b8f4', '\x019cdd429ada2d3b02c13f6c5237c3be368f45c1ee9be9889521356fd03fc1ffffae3b400fe1c4c1b2da30c6b44084b71b169ff56933fb19e860755dca1132b9f0', NULL, 204, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 72, '2026-11-25T01:15:54.854517752Z', '6c87a66d73803f47', 'purge', '\x46de4cf5fc5d5758e671c1f94e01cbbf3b7ddca05ca0ff46a7261b256111b8f4', '\x01392d18daafc17d695870e8e65c8ed977742dc54501f9db9f1a10f6b77617eec037a8f203bb5a852e6f39c0f372e3642bd5a3b30a8eaadeb9f9e78ba6a5431b8c', '87e777f7778203850c1071d2eaa3d839', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 74, '2026-11-25T01:15:55.039775639Z', '6c87a66d73803f47', 'hold', '\xee73a8dee41feb80f55dbb78d798a64bbfc854500192902f5d03591ef8756923', '\x01e70223a621928e0fcc465584320318d638d67a3f3a61b917a0780fb8511820be7f3d0ea05c4d5a9d5aed7fda069fc143af57a78c525c1cfd8827b44f76948e96', NULL, 204, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 76, '2026-11-25T01:15:55.049374442Z', '6c87a66d73803f47', 'delete', '\xee73a8dee41feb80f55dbb78d798a64bbfc854500192902f5d03591ef8756923', '\x01459ebaff49ed11817c1258773189f99bb122b6d0df3a310aef1fdc86c6edcf5b05f82c78a4338e88a4fa37a595e7671741ee60707666be494d903f77bc069421', NULL, 204, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 78, '2026-11-25T01:15:55.057194264Z', '6c87a66d73803f47', 'hold', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x0135c193fd471f4c8d30bc2e6e0b8090fd984719d5a214f2b508d7180b61177e7379db685645238dd15dad7352e9281d8fde27bf4a563eddb1ab02b95a5862969e', NULL, 204, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 80, '2026-11-25T01:15:55.081070166Z', '6c87a66d73803f47', 'replace', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x016b2bb13f2645c30c0f01fc7e5ad945e838ba1dc915d81c885d12135a842b1db94bc9077f015cff89ecbecb9e81d90685cddc6283c3b3f3e65136ce82cee354a3', '7559ea7238822c972d12c4de453a16bc', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 82, '2026-11-25T01:15:57.425328069Z', 'sweep', 'purge', '\x1e340a24e121f04890fb336f83484f79b0c79c4ce422f686115557128b5ad891', '\x01187b42234526b8ba60dfd54f77b4f45e03d3e47a1c9f0d7f7f14d4dad9b17851e02b703ad9faf19bd93423c457a9d89aae45c081114d0fc37cff44e7bc24e0dd', '026ff32c100ef3ed329efd0f2fea1dc5', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 84, '2026-11-25T01:15:57.776309723Z', 'f382bf2530d2f13a', 'legal_versions', '\xb014e2fbb44798d7a601d729cfcdd96b8b34758dfebeb3c0ccf7620b7ae0f0df', '\x0152fd377eb8d94fa65f062e8fb437b14e5a8fb2cf01667b166c6308bd7b5a976c109b05f4f62d67ee3d52b6602cf282679930ca396fac06b2f4795170d04feae6', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 86, '2026-11-25T01:15:57.792318729Z', 'f382bf2530d2f13a', 'legal_read', '\xb014e2fbb44798d7a601d729cfcdd96b8b34758dfebeb3c0ccf7620b7ae0f0df', '\x01f9f4df2fe0e535b2cb23599c54b8aab4acb7e7a1e3e5d4549cc9baba2cc9810327a6811462164fa3459a7ddae3b4da8e80f58a3fcab5af02933cdaa9f2ac9856', 'bba95e756b306de519228a93f8dd059f', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 88, '2026-11-25T01:15:57.802621337Z', 'f382bf2530d2f13a', 'legal_read', '\xb014e2fbb44798d7a601d729cfcdd96b8b34758dfebeb3c0ccf7620b7ae0f0df', '\x0177d67246928244d40679a928b7433b72df0abebf2eef3659e2177ef990b1b52c12aae29277c5e8f2a2e18ceab1053581fc20a47a990b563efd55f0e699dbec5c', 'a53a037d231ab9e23edd702a948c940b', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 90, '2026-11-25T01:15:57.811716613Z', 'f382bf2530d2f13a', 'legal_versions', '\x1c2dd9ded98d7cba3cf06799bf09362e401fca8e8879e75de449dba4f495df87', '\x019d3d3d9cad0c7fad376b4b02d2536301fad389c93832a7a0751b2318735b5fbfb82251d75da45cdef7842fb203d38b64349a6af9074f316813193e2032451e4e', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 92, '2026-11-25T01:15:57.827562044Z', 'f382bf2530d2f13a', 'legal_read', '\x1c2dd9ded98d7cba3cf06799bf09362e401fca8e8879e75de449dba4f495df87', '\x01b67514bba417f96e7cdfb7462dfe1a35634476bd2c653fc3a525cdd5fbadae49a024d42d68e35da3add963ce6fb44e8f08b49df077098ab3cbb60565980e11bf', '2cd8c79f2191d994c775ead9ed87b9d6', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 94, '2026-11-25T01:15:57.839095541Z', 'f382bf2530d2f13a', 'legal_read', '\x1c2dd9ded98d7cba3cf06799bf09362e401fca8e8879e75de449dba4f495df87', '\x01599567fd01f5668a28f03d3a35437857e34a61d99fcf99a138cce8633d47ea42ca23c9cdbbcf7ae7c0872ebe278ca32eb77d6968b98eb213728fa3eab24c051e', '4c19cb6723d6585b7e16083583d3c02f', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 96, '2026-11-25T01:15:57.846187234Z', 'f382bf2530d2f13a', 'legal_versions', '\xf4705bd2f6eb2f35dd4c1e84e7a4a381945bbf163d4e7ae842b3c1d8f2f28f93', '\x01a2b3e8b653bb25ae5b905ecdb23a3e6ec99c1e2a765d858490bce38b28d8d96eda72d2b9ea1db3f899260c807bfb3e6c069e3eb2d4950a17d5010b7989413e0e', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 98, '2026-11-25T01:15:57.858209018Z', 'f382bf2530d2f13a', 'legal_read', '\xf4705bd2f6eb2f35dd4c1e84e7a4a381945bbf163d4e7ae842b3c1d8f2f28f93', '\x01969eacfcae54cbdc796a73d35c86e6f9b7c4998660f282254563706885bea8b835d6271189e69fc8ccbe06bea7dac2c49c8839d994409b7dd824ac6478c0ef55', 'ccdafa572aa15e852a9843bcdd4373c9', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 100, '2026-11-25T01:15:57.867857914Z', 'f382bf2530d2f13a', 'legal_versions', '\x6a59acad6c453282f86f4a165de5a42f0837643eadc76f836850c2af4b513151', '\x01ff2a04e903bb11e1823da572f34acac13cf84c3fc471caa7c0c12c8b72c477594eb824e9d78be02c23345c8fac528350f50e62832c4c36eabc11aea595a6079d', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 102, '2026-11-25T01:15:57.879238725Z', 'f382bf2530d2f13a', 'legal_read', '\x6a59acad6c453282f86f4a165de5a42f0837643eadc76f836850c2af4b513151', '\x018180c6dac9e350f1975eceb438b85bc464a0966b9a991aca646d4edbb087754922d155121e6db13aea1a52242cd7a27808ca0d6b66532b3cf96f155d87e53895', 'e09cb7464b63c54d4f4dc218b6944e19', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 104, '2026-11-25T01:15:57.885919002Z', 'f382bf2530d2f13a', 'legal_versions', '\xedf108f0e446a99f362879a165e89f428820a3d1a290c48e04a6a4478ea59424', '\x019366b5a97e0543853d0d448184673c4769ea5d33ce59a490b273db48471655497cbb10e0e7a7889746bedf525c25e7f87da1b49517003594bb0f81cc06e8f9fc', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 106, '2026-11-25T01:15:57.898897832Z', 'f382bf2530d2f13a', 'legal_read', '\xedf108f0e446a99f362879a165e89f428820a3d1a290c48e04a6a4478ea59424', '\x01f286fc073e11bc55a71c87a8052d62b3fd2eba220ca28bd11853ff7c903fb25bc2f23699282319a575102717e3dc6bbde9a54b61e2ad950d4fd1e59b386f4c4e', 'b24391dea4117a184a6cc9a85b89c4bc', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 108, '2026-11-25T01:15:57.906176543Z', 'f382bf2530d2f13a', 'legal_versions', '\x70c75f04ea6115533a50be94e4beb0331780999c4f8c3f294ad323a5410e6b72', '\x01728fef17f2d64a1b7e1f4fd3896e47cd826d588c78dde36a9ffc521231c02770175f2865030916a3df9478d5d12449d28543d349f4cdea6d274792c4325df8c3', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 110, '2026-11-25T01:15:57.918981531Z', 'f382bf2530d2f13a', 'legal_read', '\x70c75f04ea6115533a50be94e4beb0331780999c4f8c3f294ad323a5410e6b72', '\x0170cbccd1395308684ce8bb5e178d0d82ba12c500f5b6eec98909584491f44bc452ae0edfd86a2243f792f8833dbd6d65a22c05c7f2be9289a4954aaef25d5544', 'e5ec09790023fdf18cd114c5c15d55e4', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 112, '2026-11-25T01:15:57.929887787Z', 'f382bf2530d2f13a', 'legal_versions', '\x2745aa8119afdf39246030a6aee38e63d9390a2974eb055cc77acb07df955336', '\x01b331dcb5afcf782bf017cad899d3e9be36de82d8a42191d0f0795203001df0f2d63fdda7326de28cfee994baf8952abbfb7680dd3cd9383c760e6eb6e1eed0a1', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 114, '2026-11-25T01:15:57.941911182Z', 'f382bf2530d2f13a', 'legal_read', '\x2745aa8119afdf39246030a6aee38e63d9390a2974eb055cc77acb07df955336', '\x0155ce5bee38aacac1ebfb582ca31316f6acf37b860b2cc346e440b5defc43fa2ba49fbff15a41b88e15ae7282d20a3000e083285ff719c59455c2c7d03082a8d2', 'b9818a16dbab1aab7eed824bfe60bcaf', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 116, '2026-11-25T01:15:57.950475513Z', 'f382bf2530d2f13a', 'legal_versions', '\x57723b14a7685dae99982d2720ec88b3a3c017218a094e8903cfa79fd8e809d2', '\x017f6bea110cb433c5bcf9ede2b23e9b6ff1730a2cc916a4c30536ac4cb1b89ec1836d0f614027b94e637445036850e98ba81ac52e4a1ca0819af44a36ce41995f', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 118, '2026-11-25T01:15:57.960041205Z', 'f382bf2530d2f13a', 'legal_read', '\x57723b14a7685dae99982d2720ec88b3a3c017218a094e8903cfa79fd8e809d2', '\x0114caed3ab2c8246612f58cb8cf1dc15c4e0e9548a23cf9c04b423d08d6c5587942a8e80b3c889850a4286ee90566ebec9df74fa20257c4d040a57920713d520a', '8a7449ac1587238f7e501a9cc41ae642', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 120, '2026-11-25T01:15:57.96916399Z', 'f382bf2530d2f13a', 'legal_versions', '\x0027904a731a01602a88cd2fa76be1583c1def248a2a0b631d4cc835d3f239d1', '\x01f4810269f668b771506494eecde135989e0806843c36ef68c38bfc9fe971b0ad2c3b0c877f8d73f323a3f8b41b449e58e09e21acd67bc99d7d90a81b082f0501', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 122, '2026-11-25T01:15:57.978433979Z', 'f382bf2530d2f13a', 'legal_read', '\x0027904a731a01602a88cd2fa76be1583c1def248a2a0b631d4cc835d3f239d1', '\x01b872af60d8e1b66e8261479e3d873501a0aea87f4033c623b93e35d55a59ba4d1736d120161a3096445b6578fa027fac67190785149261801c239244584e6056', '9ad2b8e68a360e09f004eed2196d77ff', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 124, '2026-11-25T01:15:57.987089208Z', 'f382bf2530d2f13a', 'legal_versions', '\x01ad847132e9e8de7db7627e6c310aed6302545ff5cc8fb21b168d743e0e0164', '\x019ee708a8590e04dc79922d62d045dfc20093bc998f5fec76539df5051f72dac32505b8aa0972ca0d3d21a3567819ae02e8b76eb9eb72038f1ed7943933d9f491', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 126, '2026-11-25T01:15:58.001867196Z', 'f382bf2530d2f13a', 'legal_read', '\x01ad847132e9e8de7db7627e6c310aed6302545ff5cc8fb21b168d743e0e0164', '\x01022cf0fe8ac81d05d4b9b7047e72543444054fbf27eb1289ea7c5ba51d8724caa7803336015c6b7c0b50091d395539565d881db5b69e2266d7f182d7a807554d', 'e981f67ae55ad1a1a8f81a05f001bdfa', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 128, '2026-11-25T01:15:58.014299146Z', '6c87a66d73803f47', 'legal_versions', '\x1e340a24e121f04890fb336f83484f79b0c79c4ce422f686115557128b5ad891', '\x01e386ab4718050c2e83e30768f8d351bed39427a877c6851415680d65e814665aacdefb92a3bf110cb6d7256693aa1fb14b65d61001e4a45c18264c36df8cf4b6', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 130, '2026-11-25T01:15:58.025063485Z', '6c87a66d73803f47', 'legal_read', '\x1e340a24e121f04890fb336f83484f79b0c79c4ce422f686115557128b5ad891', '\x01228275d2ffb4ff62e7ed1662fc5f87633364e421c1fb67540c4e64666d573eee3ca924da58b2cd844d2af8a9fe1e6b187912c1407a97def38ed1c6a3e2a3ffea', 'aebebc3d08b49be947d3c467b72c13a9', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 132, '2026-11-25T01:15:58.035177789Z', '6c87a66d73803f47', 'legal_versions', '\x46de4cf5fc5d5758e671c1f94e01cbbf3b7ddca05ca0ff46a7261b256111b8f4', '\x0114a61fb56d66ad465ad64cf3e86402e371d6757d412ec28166b742e889ebf81d03324f5b31adf29d49c54807af01506265d0828e9fa3f9b359e257b6671b4ed0', NULL, 410, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 134, '2026-11-25T01:15:58.050462127Z', '6c87a66d73803f47', 'legal_versions', '\xee73a8dee41feb80f55dbb78d798a64bbfc854500192902f5d03591ef8756923', '\x0193a0d8470220d2cfc917a5a50c2b6145298cf4e3823102aacb70fe64a8c63976c6ed814c140eb9d9a454dacff15549594ac44d31d749df75e94e03895eeba971', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 136, '2026-11-25T01:15:58.064963235Z', '6c87a66d73803f47', 'legal_read', '\xee73a8dee41feb80f55dbb78d798a64bbfc854500192902f5d03591ef8756923', '\x01bed76a637d773a1ad06f958fe92163ad5badae6be7432c1f4eb474f8757e3ee93aa234d082d10c2593c9b3cc8ab91b7e174c523ac99e65006b719f9ef5915422', '8ad67cd52678b00f97f450c843cf69cd', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 138, '2026-11-25T01:15:58.074388335Z', '6c87a66d73803f47', 'legal_versions', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x01a7912b339f3db2c159e0df32aaf6e115ccdbf81472321a59b89e8d01bcae6ef236fead3d56df04c9b47fbe2504dcc2c9c89e1e94c207f47915ce8eb43696ac1f', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 140, '2026-11-25T01:15:58.083627369Z', '6c87a66d73803f47', 'legal_read', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x01a9531fdaecec9b223e8c51fd52ed7d4140f7709292481cec2e7fc78ebc9c1ea492c0795bd5777741eacf0ce13164aaa282b7cc093083de237a438f63f1aa62f8', 'b6f94cc0086fa6f99fc4be78caf594dc', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 142, '2026-11-25T01:15:58.092692672Z', '6c87a66d73803f47', 'legal_read', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x0190f3b55c0d382d33b5230cc361d0cd1f579a41f36e297eca575622ef5b09d8e02530ec67ba770425a1a4fa18e6815ccef3efe46fa3fc1bfae65a09fb2962d3e0', '7559ea7238822c972d12c4de453a16bc', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 144, '2026-11-25T01:15:58.102071327Z', '6c87a66d73803f47', 'legal_versions', '\x8b2e6aaabda7e70f7a52d7b048ba8b47b814a3179fcf41e16b1b2865169b9dee', '\x013049fd00ba5f90f37aed4c1c228c3a793b32ba76a641d37a7fa0009ed8e0deebf9ec2487cfee78605195b1b2740c712570900d9f26a5deb2899dfeb84fb1856e', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 146, '2026-11-25T01:15:58.11786586Z', '6c87a66d73803f47', 'legal_read', '\x8b2e6aaabda7e70f7a52d7b048ba8b47b814a3179fcf41e16b1b2865169b9dee', '\x013f6c02e2d83b4ae0269dd6977a3ea7d47a686330db5c1311e07ecd1ab58204157faf6582047967252aca4c2585e9117304a11677d5cee1b8174bac77488f8223', '784b591d42d9de0ca50bc46e88d2316d', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 148, '2026-11-25T01:15:58.128053664Z', '6c87a66d73803f47', 'legal_versions', '\x7562d57e9bbca73ea85c40708d9a3aae5e2a8edf024da42d7111ae7b7574f750', '\x01301ceaa6329e801e3e807d1a51c71f13e49718919b83dfbc659159018d3c815b8fdba6748301e03d6184a4c8b131d1a437ed072d5c04fb8eba47dd13e89bec17', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 150, '2026-11-25T01:15:58.13850805Z', '6c87a66d73803f47', 'legal_read', '\x7562d57e9bbca73ea85c40708d9a3aae5e2a8edf024da42d7111ae7b7574f750', '\x0177a539b9ebc1ed1ff6c325e2716e7df1db5fd282e594117fcac55eca2f6fcbd7c8e632b6d50a6c61586f439f5d8c8fac016b99542086e6739f654f4772bddd66', '55c5c72607820b101c4d7ef5a50b6433', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 152, '2026-11-25T01:15:58.149609362Z', '6c87a66d73803f47', 'legal_versions', '\x9a1b01d5d7d8eb94cb5b3a77432cac654043e681d53df28fdf43220a69086b26', '\x01f9e6aaeb979be5048aa89555aa5e710efa7573e5c75dbb6ca56f2c493143a3b599e3aac1dcb9ff1de2476f3544a8a8c56ba6ae66d8f38a415a8649032c8124ed', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 154, '2026-11-25T01:15:58.166539442Z', '6c87a66d73803f47', 'legal_read', '\x9a1b01d5d7d8eb94cb5b3a77432cac654043e681d53df28fdf43220a69086b26', '\x018e1d475cb8f8d543aeae05ee87b77baaa3c6cb8c44330baaa84f5a48f86af05cf4a4962c6043f640cc2666d39aa31abc9932d36d557487735e2ce2547f9107d1', '78de5dd45f522aaec1dba3da9d1bc799', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 156, '2026-11-25T01:15:58.177732931Z', '6c87a66d73803f47', 'legal_versions', '\x311d6b021088213052d480c5104601e7d4ceb76326263fbad7d2d1e9561093f4', '\x0107bba1cdad95a7c0ab7a890a915ca536e57f59f13538e3a5cb079badee3190c6adbc84924ac85c26ed41129ddfde95c42f2c5041edb5f9c19fa36335cdf5166e', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 158, '2026-11-25T01:15:58.187897586Z', '6c87a66d73803f47', 'legal_read', '\x311d6b021088213052d480c5104601e7d4ceb76326263fbad7d2d1e9561093f4', '\x01d9bfb05055c2c2604ec33f6223070f0e24d85e1a71bd3c9c63ca0b5ec85fafd9168403b7dc36fadbb9d6b604c4e6101eacb8f129ad81c1e87e33aad76cfed687', 'e01baebf8dba4c4f2c7a568ce463ff41', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 160, '2026-11-25T01:15:58.196897612Z', '6c87a66d73803f47', 'legal_versions', '\x82af17960feaed69128dfaa49961e224e1a18fe39bc2d6400803ff00f261f0b0', '\x01eefebdcb8fce77868f7eed22b718e0dbc0b0a9cd0cee5b302cb67946905733f336836d7eb5472efe806c18bf9639b49a4ef09c4774037a9fe8cf38e6f9ce7410', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 162, '2026-11-25T01:15:58.209807822Z', '6c87a66d73803f47', 'legal_read', '\x82af17960feaed69128dfaa49961e224e1a18fe39bc2d6400803ff00f261f0b0', '\x01f57e5d1213cc7a1fac7f41d528b644d0dbfc6639db3e85bc88b16ac737532294f86240d54e070ac506331e841e4c1c46edf05f029fbf1e7bb3b4cbeeb61449a7', '86e8adca8fd64b6176f675c671a5daf0', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 164, '2026-11-25T01:15:58.216728256Z', '6c87a66d73803f47', 'legal_versions', '\x4c26728fd69e371c2e314fbd7a354af22c86a79fb90e422cbd98a78a5e6838f8', '\x01581137a2848c7cc0fb3ec5302feee79cab725a75ee24faeb340a281656dc465fe8789678ef696132a2842db191b64f78810c56f4a6735bd722b3e56052155dd5', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 166, '2026-11-25T01:15:58.227101532Z', '6c87a66d73803f47', 'legal_read', '\x4c26728fd69e371c2e314fbd7a354af22c86a79fb90e422cbd98a78a5e6838f8', '\x013ceaeb9d0864e0318f2ae0b170bd51710cd6d0e9f6fa20777edea50c653752bdcdd6e9951fa11cb53786951845384f2af3ed68a8b77932f9b9fcf4ec34991d8f', '91ba50d1aed0a393b25f344c3aeb5972', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 168, '2026-11-25T01:16:04.448783977Z', 'f382bf2530d2f13a', 'create', '\x8619e5e2bdfce6e5227c91291e7e451a028694b2f2d7ef749a21ca0ac0a0685b', '\x013039afdc758d8409f3cceb478f3e3ba17e05456b224376f5c9b4437c785b32f1be1af8a06fbf48b31372b67a63c11a017e395d9498e2df944b7230fe6042e598', '356d97090bd902789c510a5056ae9bd0', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 170, '2026-11-25T01:16:04.496526662Z', 'f382bf2530d2f13a', 'create', '\x3e7a2804322bffd5fa49092c564b49d957e03f93fa0e7f4ddd24b701061fc79a', '\x014dae3bee2921257080b2ee2febd328bdb08dfee231f42e0c8e43516d9b201a6c3655c0ad9db98fd97a79d921ff98548c5b9bbe1d81845d3bbcd214bb36b3c758', 'd9335f526c764a8afd9fed1ee8f10a05', 201, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 172, '2026-11-25T01:16:04.547362632Z', '6c87a66d73803f47', 'create', '\xae22d7f8e2d8d4d1ae3e8c5f0d04e74b27f68aa762345c3bc67f61fbf2f1325d', '\x0135dd20db7f67f3b9dd725ec828f37f1e350c386e9aa04e1723b12fb52f7c3ef3f6de676a8549fccb6dc17cf64ec6be118c9602c07860d08afae65a40a502e560', 'da8fb156bc7324b83935b3eeb5cca42e', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 174, '2026-11-25T01:16:04.6001974Z', '6c87a66d73803f47', 'create', '\x62a4d99aa3d72856e37b7ff1fc9cceda990a5889ab1b42ee5a7f2e00f0cc6091', '\x01c54085a056e01b4768fad24943af84a9b3b962424545dd18220cc4e52a4fdc9980e1c2cee8bafba1893fc19dd23389aa1a8fdc1256f1b422acdf623ae818b49e', '1a1254c959c8435c58abedff4f2bb80d', 201, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 176, '2026-11-25T01:16:04.609988028Z', 'f382bf2530d2f13a', 'legal_versions', '\xb014e2fbb44798d7a601d729cfcdd96b8b34758dfebeb3c0ccf7620b7ae0f0df', '\x014719124d9a1934365f4cad70ad5dd9a26ff76cd64e6d08987918382568c62e6dff7b7fcdd24ceb7d79ba8221a68462b86ba205654ad15b5aa84a6a3a2e8d2f76', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 178, '2026-11-25T01:16:04.618957128Z', 'f382bf2530d2f13a', 'legal_read', '\xb014e2fbb44798d7a601d729cfcdd96b8b34758dfebeb3c0ccf7620b7ae0f0df', '\x012a54de7f8ef640485871b9feec512d9aca3738c7d2ab1c99624111e27a16722d28252a3ae304d722c2cb353210c0a2359b61224c1419fe1aa04331ffa36f00bb', 'bba95e756b306de519228a93f8dd059f', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 180, '2026-11-25T01:16:04.630663264Z', 'f382bf2530d2f13a', 'legal_read', '\xb014e2fbb44798d7a601d729cfcdd96b8b34758dfebeb3c0ccf7620b7ae0f0df', '\x01b397dda93b038309da33b0535521249b2042a4c0c3073bf6decfc2eea9c652a88663be3c1917af4dcad6d5a5cffa24b04b021f740ad271ccd367fc64627efc34', 'a53a037d231ab9e23edd702a948c940b', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 182, '2026-11-25T01:16:04.63986654Z', 'f382bf2530d2f13a', 'legal_versions', '\x1c2dd9ded98d7cba3cf06799bf09362e401fca8e8879e75de449dba4f495df87', '\x01dc5f499167e623921ddc254a9bef0d9faf9c2f14be90da845864a0f20436d65d0cbaa3911c589261da00bae02810f1c7b814d984736d61a744c43ffab228c4c6', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 184, '2026-11-25T01:16:04.646480325Z', 'f382bf2530d2f13a', 'legal_read', '\x1c2dd9ded98d7cba3cf06799bf09362e401fca8e8879e75de449dba4f495df87', '\x01364677d12146722651f5a8cc3dfd080a4d399bae057a64870fc90b5e11f8a9fe30da542a3a1e5c110c5312961d3331a470ffb5dc6a2ca7f0dfdbfe910d2fbad1', '2cd8c79f2191d994c775ead9ed87b9d6', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 186, '2026-11-25T01:16:04.654521889Z', 'f382bf2530d2f13a', 'legal_read', '\x1c2dd9ded98d7cba3cf06799bf09362e401fca8e8879e75de449dba4f495df87', '\x0117caedbcb6be471e80f1fbe4f52f97f6e6d2177a73e59eb9fd720826030cb2062473d0528180f83dec0392751635fb909a4ea7834ccddce4af569fd1ee0da2f9', '4c19cb6723d6585b7e16083583d3c02f', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 188, '2026-11-25T01:16:04.667215969Z', 'f382bf2530d2f13a', 'legal_versions', '\xf4705bd2f6eb2f35dd4c1e84e7a4a381945bbf163d4e7ae842b3c1d8f2f28f93', '\x01e5579fb9156d4434735b18ac28a10c52daee9a945a8a65789f5c2b04be54acc7da12cbc9d8b52f67d4e138aa50eee20b36f69e6e795461f7dd0912ae2add96ee', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 190, '2026-11-25T01:16:04.680651242Z', 'f382bf2530d2f13a', 'legal_read', '\xf4705bd2f6eb2f35dd4c1e84e7a4a381945bbf163d4e7ae842b3c1d8f2f28f93', '\x011aa5d2340bb23257327e96d822a4bc8e84313deb10617e4409d738047dd6de5f59a338f8dbf3f5d3b755de620791fc34aa8036f58cf1ed6a524a9b7516ca9042', 'ccdafa572aa15e852a9843bcdd4373c9', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 192, '2026-11-25T01:16:04.691577805Z', 'f382bf2530d2f13a', 'legal_versions', '\x6a59acad6c453282f86f4a165de5a42f0837643eadc76f836850c2af4b513151', '\x01c0a6bbd5fbffd5fe51817fd353ad5b8a6658aabb46742407d4ed2075e669905c2bdfdf7c601f0ef552bb8eef15710db9964a564223bab86e5e2588a8c109aa20', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 194, '2026-11-25T01:16:04.699421451Z', 'f382bf2530d2f13a', 'legal_read', '\x6a59acad6c453282f86f4a165de5a42f0837643eadc76f836850c2af4b513151', '\x014404767d7a001f2548e1d436588134f1513d2d532a90c792d2d3736f21a7ced4b21843d70f01a8db5eb5ffbee1f6c060a839cf7723065a6a710a3b0e930af27d', 'e09cb7464b63c54d4f4dc218b6944e19', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 196, '2026-11-25T01:16:04.707237504Z', 'f382bf2530d2f13a', 'legal_versions', '\xedf108f0e446a99f362879a165e89f428820a3d1a290c48e04a6a4478ea59424', '\x01e13a19609b43463fa72d3a3529a0b391024f9e6aa1df250883b148820a49eb75ca44ffa41bd8d45f6b99d78beed3b6dbba486ece056de9b95b372e8094fe7f23', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 198, '2026-11-25T01:16:04.714995947Z', 'f382bf2530d2f13a', 'legal_read', '\xedf108f0e446a99f362879a165e89f428820a3d1a290c48e04a6a4478ea59424', '\x0139e4ca6de9174105c4d8a4bf901c4eddab18eac1f6513e838d0fcc5de06f0e466519b55e094fec01dacd9c5a97942f273d29bc8d15118004c437ca959a93a882', 'b24391dea4117a184a6cc9a85b89c4bc', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 200, '2026-11-25T01:16:04.722872972Z', 'f382bf2530d2f13a', 'legal_versions', '\x70c75f04ea6115533a50be94e4beb0331780999c4f8c3f294ad323a5410e6b72', '\x01d530913c9c2ea0fe60de9c34bcd88bf567a5e5d9ebb7178eb2729b501684497d7b49869f14179d1a4cca0d28c389215d2c946f8bb30141f1076df944f35dba11', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 202, '2026-11-25T01:16:04.733209091Z', 'f382bf2530d2f13a', 'legal_read', '\x70c75f04ea6115533a50be94e4beb0331780999c4f8c3f294ad323a5410e6b72', '\x013e94d18f7685f6f441265fe935b5f0f3b9cbcfcb4b49dfb541d25cd424901fd654f7ddc38489a8a97df1651c1f0ee8ab7f8d93d87ed5f3d947fc8d9d72d73170', 'e5ec09790023fdf18cd114c5c15d55e4', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 204, '2026-11-25T01:16:04.738950358Z', 'f382bf2530d2f13a', 'legal_versions', '\x2745aa8119afdf39246030a6aee38e63d9390a2974eb055cc77acb07df955336', '\x0164db9aa12423cd64bd184cd100bb541fc75b03e32b5e871a0b07575b9e7bf2ac66b266ad3b4c359bfda81c7405d907a349e37f9aa2695cbdd869fcde5b50c619', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 206, '2026-11-25T01:16:04.746272152Z', 'f382bf2530d2f13a', 'legal_read', '\x2745aa8119afdf39246030a6aee38e63d9390a2974eb055cc77acb07df955336', '\x01dfb9ec8795f09410641f14e2f7f0eae02f4f1319a1274cbb4ad618ab41af94c56c32154adb4f66fa2cea8200be1aa0cc5353c21953d85111057355f7b4b9ac61', 'b9818a16dbab1aab7eed824bfe60bcaf', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 208, '2026-11-25T01:16:04.761065482Z', 'f382bf2530d2f13a', 'legal_versions', '\x57723b14a7685dae99982d2720ec88b3a3c017218a094e8903cfa79fd8e809d2', '\x01c6f064b6dcebf01ece26aa1e76a1fb6d446e3d7c2e66b2d9d41ad3a4554c8ef6f0169c74d226c44ae98fa21ad4221b8a97e7e80b92003b2d1876a78a603bde6f', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 210, '2026-11-25T01:16:04.767758591Z', 'f382bf2530d2f13a', 'legal_read', '\x57723b14a7685dae99982d2720ec88b3a3c017218a094e8903cfa79fd8e809d2', '\x019b32262226f4f40aa5658bad24489a05d71e8435468ffd2a729f3b30aa5c41e11cc2d4f88b9d041f6aab91dfac3bf145ba7cad870ca7b3bdffee11263d3a02ec', '8a7449ac1587238f7e501a9cc41ae642', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 212, '2026-11-25T01:16:04.774379683Z', 'f382bf2530d2f13a', 'legal_versions', '\x0027904a731a01602a88cd2fa76be1583c1def248a2a0b631d4cc835d3f239d1', '\x01ced8d517af8d255a0768a23b5624523b59d37951c985decd38da526ce925c5cf711c50946a382b2b7dadd9740f4ba5da98d06c0c64ec2a163741631ce566427d', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 214, '2026-11-25T01:16:04.782096912Z', 'f382bf2530d2f13a', 'legal_read', '\x0027904a731a01602a88cd2fa76be1583c1def248a2a0b631d4cc835d3f239d1', '\x01929108fb904b6eb00ed92d06d00ba68b7ac8476233ce04802d6e24b4b258e0339668452d8b8069681dbd16640e4fcfe399b95b0ea5c93b6f92014b35739270f1', '9ad2b8e68a360e09f004eed2196d77ff', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 216, '2026-11-25T01:16:04.791288132Z', 'f382bf2530d2f13a', 'legal_versions', '\x01ad847132e9e8de7db7627e6c310aed6302545ff5cc8fb21b168d743e0e0164', '\x015d1df55da368b903fe464deea5b6b9f093afe829c31ba62ed7f627b3b321025bbe1aa0093580584d0679cf383856421dbccb5ab927b4933a29aed8d8eadcac0e', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 218, '2026-11-25T01:16:04.803954472Z', 'f382bf2530d2f13a', 'legal_read', '\x01ad847132e9e8de7db7627e6c310aed6302545ff5cc8fb21b168d743e0e0164', '\x010bfd2a9f5467980731b1d3b2925d77027cbd1623c79819d6751246b28941e0d96a59eff0da5071c3488bcbaf9cf3ba088cf1096c3dc1dcd5b05392495aa594dd', 'e981f67ae55ad1a1a8f81a05f001bdfa', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 220, '2026-11-25T01:16:04.812638589Z', 'f382bf2530d2f13a', 'legal_versions', '\x8619e5e2bdfce6e5227c91291e7e451a028694b2f2d7ef749a21ca0ac0a0685b', '\x0144db2c918715d49f891db2ecaa2d5436e0ddbfbc6fb8329bee7dfdf12a490402a838f94da111020487fc342240582dc7529805a4de70181863ec057bddeb3357', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 222, '2026-11-25T01:16:04.821320679Z', 'f382bf2530d2f13a', 'legal_read', '\x8619e5e2bdfce6e5227c91291e7e451a028694b2f2d7ef749a21ca0ac0a0685b', '\x01ef53cbb1d5f952dabb5fcd09b12f87c2396b29f57521a4099d09ea716af002fcdb4de91be12f776d909be403e25046b4a5038c5b0b46ec273f0a2bd9a517f284', '356d97090bd902789c510a5056ae9bd0', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 224, '2026-11-25T01:16:04.830412677Z', 'f382bf2530d2f13a', 'legal_versions', '\x3e7a2804322bffd5fa49092c564b49d957e03f93fa0e7f4ddd24b701061fc79a', '\x014678912b77f02c01d52ffd69f6cd2e875a97503d104c187cc4eff03d29e59f4c52d91c92be012f16d2cbe13395ee5c479694d9ca6a3ada5684621b67edbb3a94', NULL, 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 226, '2026-11-25T01:16:04.838441935Z', 'f382bf2530d2f13a', 'legal_read', '\x3e7a2804322bffd5fa49092c564b49d957e03f93fa0e7f4ddd24b701061fc79a', '\x01a307b942f28114c1a319bb724624abc192a944cc0dad8ff0165f8d18902635ed92733c2657533ff71b9df2c3e9a4cf4880ce29fe270b87f4904d6a28a85ccd6b', 'd9335f526c764a8afd9fed1ee8f10a05', 200, 1, false, '\xc314bdb8971a58bf');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 228, '2026-11-25T01:16:04.844470081Z', '6c87a66d73803f47', 'legal_versions', '\x1e340a24e121f04890fb336f83484f79b0c79c4ce422f686115557128b5ad891', '\x01966c46d8df43b75208a1b02010c564df24e5678e41926458a626db0868d73f493c8d716f23a6aa75288c16a03f0e2030447c4424a96bcccbb617a1ccfc0eba1d', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 230, '2026-11-25T01:16:04.857023199Z', '6c87a66d73803f47', 'legal_read', '\x1e340a24e121f04890fb336f83484f79b0c79c4ce422f686115557128b5ad891', '\x01bf55b27fe779d9c919e64d11cf6cba06a442704c334d76d52b2763577b650cbb553e618e7336457958682e0c267c9815dd874490dc7c7fd60cc3e9683f4875e1', 'aebebc3d08b49be947d3c467b72c13a9', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 232, '2026-11-25T01:16:04.861463299Z', '6c87a66d73803f47', 'legal_versions', '\x46de4cf5fc5d5758e671c1f94e01cbbf3b7ddca05ca0ff46a7261b256111b8f4', '\x0151d1fe6b0b1d628313ee0e353d2fa883225c294b313851a07a083133a4f5e199fe6c95ed3e8c37e4f7d095d8cb6ffd30152b72507547b268076868f31e828db7', NULL, 410, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 234, '2026-11-25T01:16:04.87022901Z', '6c87a66d73803f47', 'legal_versions', '\xee73a8dee41feb80f55dbb78d798a64bbfc854500192902f5d03591ef8756923', '\x01315d5a74f12981748bfb27cff7413d22eaa6510960e51710c446e2debcf5d0fe0c02d4d29ad88acca3795d3f0edcddd63f25f38b54877487677a47b8ec2ee8fd', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 236, '2026-11-25T01:16:04.882138442Z', '6c87a66d73803f47', 'legal_read', '\xee73a8dee41feb80f55dbb78d798a64bbfc854500192902f5d03591ef8756923', '\x015e0fc0ab871a532ed1ef16554b79405d9d77fe9a37acbf5177428fd9e21700a4e7334c492a3c1227804cb816a345eb4c9f02276784d166c62ab4f92a31a929ca', '8ad67cd52678b00f97f450c843cf69cd', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 238, '2026-11-25T01:16:04.889333663Z', '6c87a66d73803f47', 'legal_versions', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x01fc5fb78ec61e6b550e40cf3784ad0f757e7ff2c0d896d454ee04702cf72abea4b823edbfe99c1dfbc85c9e8e66e4f9db23bf5c1cc74bdbcc1b3eb82aa2aad43d', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 240, '2026-11-25T01:16:04.897017717Z', '6c87a66d73803f47', 'legal_read', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x01f25bee13a8b1af15d113bb764b5c64dab532678eae6a39c8bd5662335946cda144b0d2ce0121be1c711bfcafc76d73766d2f2bb1e974170174aab4aade915efa', 'b6f94cc0086fa6f99fc4be78caf594dc', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 242, '2026-11-25T01:16:04.906257372Z', '6c87a66d73803f47', 'legal_read', '\x3602703fc1eede592720771a95cdb103d83de99a931b74083addd1a7aa593975', '\x0162bca64d609112a59d24b1eca1496cabb47c07ee26fb6d5ddc32cfbc82511e346c0f6f06b0cdcc4dff3755638b855c7e982e919fa5d07332b88ef42902caece0', '7559ea7238822c972d12c4de453a16bc', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 244, '2026-11-25T01:16:04.915439619Z', '6c87a66d73803f47', 'legal_versions', '\x8b2e6aaabda7e70f7a52d7b048ba8b47b814a3179fcf41e16b1b2865169b9dee', '\x017ce93db72f351b338d2b22b99c7b4fbabe33a1068b80dfbe5a4a37f9f14f06910a6b8e64e7eb31ce78091bfa93a27b9af352eb1deb9846b8e526f8e2214f9900', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 246, '2026-11-25T01:16:04.922768609Z', '6c87a66d73803f47', 'legal_read', '\x8b2e6aaabda7e70f7a52d7b048ba8b47b814a3179fcf41e16b1b2865169b9dee', '\x019fec65782d54b8a01eb6489364f19616ec96e04925b756efb43043990c4f5c30d5228cc355d846fefec54784af216883b382981db08339e4a1efe101d4b7af45', '784b591d42d9de0ca50bc46e88d2316d', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 248, '2026-11-25T01:16:04.929037061Z', '6c87a66d73803f47', 'legal_versions', '\x7562d57e9bbca73ea85c40708d9a3aae5e2a8edf024da42d7111ae7b7574f750', '\x01130ddc39f3929ea3a329954f387b2893978ddbec8e5eb1dd669f05be90748ddcc28fc65560ce8c5c2e8d8b05edf6b6425b61fb597a9c3c54d803ae3cbee46d75', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 250, '2026-11-25T01:16:04.938193373Z', '6c87a66d73803f47', 'legal_read', '\x7562d57e9bbca73ea85c40708d9a3aae5e2a8edf024da42d7111ae7b7574f750', '\x0105edd14ac910545dd2b79909e4b2dd045da6a267b523006dfa630378c0d31d1cf4d57c709d11b4ea44b496ee3e61e11587e7c097892703ff3d2210ae2b8619e8', '55c5c72607820b101c4d7ef5a50b6433', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 252, '2026-11-25T01:16:04.943519538Z', '6c87a66d73803f47', 'legal_versions', '\x9a1b01d5d7d8eb94cb5b3a77432cac654043e681d53df28fdf43220a69086b26', '\x01a9f64f4f51caea3bbf6c0f79b2c1975b9355bbfb571e4f131e048d98c62f81eed25df9fec4be9487625e15eb6b389f121c001f80f8c2f428e3a0b8707d46eaaa', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 254, '2026-11-25T01:16:04.95104236Z', '6c87a66d73803f47', 'legal_read', '\x9a1b01d5d7d8eb94cb5b3a77432cac654043e681d53df28fdf43220a69086b26', '\x01b08ea7b7c1d63aa4c8fd2f97271da4c4cb1cc5118543345d55635122e77ce0d776561ee373dde30a9a161f15e610ec76fd8055666fd5aad3b3d8b7449ae80337', '78de5dd45f522aaec1dba3da9d1bc799', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 256, '2026-11-25T01:16:04.95716273Z', '6c87a66d73803f47', 'legal_versions', '\x311d6b021088213052d480c5104601e7d4ceb76326263fbad7d2d1e9561093f4', '\x0102b796be96049bf541fe201e993552bec954546e816992628ed8cd30dd55c5cc63608b3a606fa5ef22bef7dfed9f6c8ab4be3b0e191ad3ea28046fb0f89f01a5', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 258, '2026-11-25T01:16:04.967973504Z', '6c87a66d73803f47', 'legal_read', '\x311d6b021088213052d480c5104601e7d4ceb76326263fbad7d2d1e9561093f4', '\x01fd0a667151a8304845abf987e15a4e351fdd720d043190e5aeeed12bbc55cdc5883280184ba6b8016f183fd178bb6765c746fb3131bb439d268aacf453ed40b6', 'e01baebf8dba4c4f2c7a568ce463ff41', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 260, '2026-11-25T01:16:04.973467012Z', '6c87a66d73803f47', 'legal_versions', '\x82af17960feaed69128dfaa49961e224e1a18fe39bc2d6400803ff00f261f0b0', '\x017440d8a353c9a9276786492192e3ea7fbddb137178822df650d76262f8e9b790f3c49e058079e02c721797663d616d85c108e0e75cdcf3088f5d866edda0345b', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 262, '2026-11-25T01:16:04.989005564Z', '6c87a66d73803f47', 'legal_read', '\x82af17960feaed69128dfaa49961e224e1a18fe39bc2d6400803ff00f261f0b0', '\x015dbbddc427eed22a44caa3cbd1bc2b1dac93f29db0ede12a2a9f3455d5c8cfa018bafff2ff49b7e27d66d06c663cf3802e5a88d9a7abea5d5b5f367891617bdd', '86e8adca8fd64b6176f675c671a5daf0', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 264, '2026-11-25T01:16:04.997115571Z', '6c87a66d73803f47', 'legal_versions', '\x4c26728fd69e371c2e314fbd7a354af22c86a79fb90e422cbd98a78a5e6838f8', '\x01d436f22b52544182977562318ed3f3453b95c0fbae27ff4e286010577859d4bf9f570ea06ab69e567c63fe86ff3b51cbdb959ca1dbe613ca0c5495aba41979dc', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 266, '2026-11-25T01:16:05.010626383Z', '6c87a66d73803f47', 'legal_read', '\x4c26728fd69e371c2e314fbd7a354af22c86a79fb90e422cbd98a78a5e6838f8', '\x01d27ea46c2ae2c10c07fcca69020112b7a3c9bcfa8aa112c348a6a1465229284fb198162c70b920fbd84d2c8a5c5fab1f75e3a06a9a160087a3cb6f6f0110f771', '91ba50d1aed0a393b25f344c3aeb5972', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 268, '2026-11-25T01:16:05.018726576Z', '6c87a66d73803f47', 'legal_versions', '\xae22d7f8e2d8d4d1ae3e8c5f0d04e74b27f68aa762345c3bc67f61fbf2f1325d', '\x01d49a82f22184dd48c718892e7d89f177e5dca205172a65cf4a4de813275bdd88c4686125a161acfc9bc3064561dd07b98e3f3a01863c0c6b501543b2c0a3b7a5', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 270, '2026-11-25T01:16:05.03040411Z', '6c87a66d73803f47', 'legal_read', '\xae22d7f8e2d8d4d1ae3e8c5f0d04e74b27f68aa762345c3bc67f61fbf2f1325d', '\x01c9829fffa685011cd769101ee844a1e5f8b77555366d5ce5461f3ec08485d7acc460bace0acad116246fbb0e785e69eea40b269980fa5dc119a707f7f98758d1', 'da8fb156bc7324b83935b3eeb5cca42e', 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 272, '2026-11-25T01:16:05.044936663Z', '6c87a66d73803f47', 'legal_versions', '\x62a4d99aa3d72856e37b7ff1fc9cceda990a5889ab1b42ee5a7f2e00f0cc6091', '\x01909cc9395b065c99c914ce94b8c74f9bc97b2f374e4e640632e2583b5cd41f9d3bdf377d9fb36f796608b0353170b1629735d9366a38b925b9fd3f9fff8516a5', NULL, 200, 1, false, '\x5db59f9c5c148dfd');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 274, '2026-11-25T01:16:05.05609304Z', '6c87a66d73803f47', 'legal_read', '\x62a4d99aa3d72856e37b7ff1fc9cceda990a5889ab1b42ee5a7f2e00f0cc6091', '\x01cbe51833392ddf6a7239d00a1b473019a1185fd03cdf3d3ef2aa0e16b60ef526f7e75e8d837da1d99d9f00f6ff5e813f25ad86e667d8027906ae8c23fd4dd96e', '1a1254c959c8435c58abedff4f2bb80d', 200, 1, false, '\x5db59f9c5c148dfd');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xc314bdb8971a58bf', 1, '\x018a0b4b0546fca80039db1cb100a9130477e67783714f491bf306b1234d945efbb8a7f0ce258b75e4a214e332f556bdf5263014d653e2c85920029d38');
INSERT INTO vcon.audit_key VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x5db59f9c5c148dfd', 1, '\x010ddf80a86324e4a15d09468ced8c7a9ec19fb41a9b3cfaafd85a5f7cca010fdd0cae2227abc551245d9a041f56508dd5722a769e0675c39e502a8054');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xe2b47a9e099fdb7f', 14);
INSERT INTO vcon.key_use VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xd95d2fc90cea4b79', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', 2, 1795569362696471787);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', 4, 1795569364236863769);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x4f48f20d40d77114', 2, 1795569330006272636);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x861d381cd75e0f8ea62056cfc57ad9bf', '\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x01a9eb979d65693ec2df3031a76d19835b0ad6f7e6e9d4ed87968ecdc6e8e30b99ca06', 1795569349294937575);
INSERT INTO vcon.membership VALUES ('\x15192fd32eefe6802b338fe0165a7691', '\xcd07f74570f0b6fb2dadb7f28cae0780', '\x01be39b16cb9ee755ee61521aa7ae06632a8194fbbcd9eb21a7c555ef03b4927e07a5ab9', 1795569351357032162);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x6f724e8a2a332a429d144336ee818330', '2026-11-25', 12, 12, 7401, 1795569361040986174, NULL);
INSERT INTO vcon.parquet_file VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xf8510a540fdc9e75b8d877ac55f120a1', '2026-11-25', 10, 10, 6237, 1795569362811736564, NULL);


--
-- Data for Name: pending_write; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x3110ac24458ee041');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-25 01:15:27.857506+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-25 01:15:27.895392+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-25 01:15:27.901801+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-25 01:15:27.902677+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-25 01:15:27.904758+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-25 01:15:27.907851+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-25 01:15:27.908421+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-25 01:15:27.914381+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-25 01:15:27.916787+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-25 01:15:27.917286+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-25 01:15:27.918607+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-25 01:15:27.922317+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-25 01:15:27.92772+00');
INSERT INTO vcon.schema_version VALUES (14, '2026-11-25 01:15:27.930979+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '2fb79b272ac0c685c731fa5ed01eba7d');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x49ddc3091a909b5e52006d9cdb6938e6bf20bb94b8458109c39146af037c5619', '\x013129fca6a769e96482dd41f8a36d0781008caa0d25083d2de6dfe47d02d6e161f724ed4a', NULL, NULL, 1795569330016818893, '\xe2b47a9e099fdb7f', '\x01545083e1c4d4fecccc3417a0eafc37ac3c8b688d002f368a6c17752a9f11697547ea1a8540aa1590b9b639bcf3b0892f0bc77b39625e86d068124438', NULL, 1795569330016818893, NULL);
INSERT INTO vcon.tenant VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x3690edd3576d7e89a5efedb5d36feabc7b0ff5c0778e09a9a24bbac4da457115', '\x01529fd6737084ec1bcdf82ee34e59c89264c35659c990ff87e4e798de224b24f9155341f1', '\x01b05913ae3a2becd0c30d27ede4fa25d43643a0517709e5e7633f0e244bba', NULL, 1795569335743255788, '\xd95d2fc90cea4b79', '\x01661b49889f6d1c7c814a95595408e2c55ed39ff244dfc73a7303233cf9a9e06f461e692f70492d03a2450c28634fc79db564a4819acd5613726b9538', NULL, 1795569335743255788, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x5da9456711acf75a');
INSERT INTO vcon.tenant_catalog VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xe263de882ed28a23');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', false);
INSERT INTO vcon.tenant_lock VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', false);


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x508465eaa9635015a67e94a19b1580a77417bf0f2068e6344df30128a6826cad', '\x015a7607ebe65977f722d7b41f5333508d7eee61d9d8d7223e6f0a0650d9224b50acfe418ddb650a30b89cc8da', 1795569354853127355);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x861d381cd75e0f8ea62056cfc57ad9bf', '\x0173d6aef1713c30399046aa276b3f11956f3ca8bd7c121e0bc5cc3e7b02e1212e918b92e41113c44bfac1', NULL, NULL, 1795569345212668065, NULL);
INSERT INTO vcon.user_account VALUES ('\x15192fd32eefe6802b338fe0165a7691', '\x01336263e1e2f123a5ddb04f60f4d5e8bdb4eec2055428e5367048c0edf67e940f1dade7d7c6fabfef87fe4c', '\x280ff40f17837af326a037ed010c855876105fec2e8b63372decc98846a33c1a', '\x0121e7cf0215c00e1357ea5c6586a0b0b52a902ba7599a88ee7e68df3b34d3768a922b819791cba8ef4ae127772a71bc8320c6208e8d2103ac4e7782bb36c4ad', 1795569347230247374, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x717b5fba4ff4d85c06bf313880907ee209985ba6e8c643217084160e74052f1d', '\x01cfd795b16f81c83b96855617b6e66e04a85e52ec7aa07c96a41e1263819cf92e16e796762a37f686e7a4c7c7', '\xb9818a16dbab1aab7eed824bfe60bcaf', NULL, 0, 1795569354134413099);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x1d8935a20076b3f69ddccfad34d596a1b61cfc29804db2eed03b48dcf9b96aaf', '\x01e63ed78ce048ebc4f904fcf9ce5c0b8f897578832cd0c01194a1eab95312afe9a09ceb9a663982422a3c6c63', '\x8a7449ac1587238f7e501a9cc41ae642', NULL, 0, 1795569354194688141);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xf4152a45fc9f89a72cab999a4730b9144d771575b20775155f8a5aa91ea23150', '\x0141a5f6a33de0944692d9f074161dd8a2057a35789c8446c832e3de51d54b821d9be524ab83adbcf079478fb5', '\x9ad2b8e68a360e09f004eed2196d77ff', NULL, 0, 1795569354253105666);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x03720e93a26887fb2285d492ff8bec49bba7170a69954efaa420e964e6e29cf2', '\x01ec8594ee2506c3da636ba8f82ac979c6384842d886fe845247ba8ab1fe21bf6c9a750866c14bb2a1dd7ebe02', '\xe981f67ae55ad1a1a8f81a05f001bdfa', NULL, 0, 1795569354310487477);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x9825f0447252aca2e75d0f06bfc4e4cb557476206c1a27ee1071a9b44ed0eb74', '\x01a2af19a4e1c5b84745bdda44f827bc752ca61ce19c941f7f3c59ff6849f608faf0fbc9bdb0a67a5c95943277', '\xa53a037d231ab9e23edd702a948c940b', NULL, 0, 1795569353911517037);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x3d5b2e657d3466ddd2ec94bd56ad8fbe611280473c19a92f2f9df3e2ea3de0a5', '\x01550ef6f9d773db89c8d3c1ec38e897dedc216f0dd587f76341acbd1703c77521e3ed749ce47a2bf7b5d19f87', '\x4c19cb6723d6585b7e16083583d3c02f', NULL, 0, 1795569353979722733);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x0f78ebf4e91ab706b5047432374a3769ae3c18912667891f1274119bc72c5c5d', '\x01c46ce91596023eb00ee91d448af7a1ac79d02e6d4c98a240d31a0d7d76668f00a040fc544cc6cd3f6b78d18b', NULL, 1795569354379201339, 0, 1795569354005487291);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x4e63ffe0bd120e3be42dd1e0d3bcb63d7cb5c0d3a16b58a5259ac17cf158da62', '\x011f57b1d6e7317b6dfdc34c131b4bb7565e249735d62ed412472f3b095c208ef38e176034828afa6042a70ae9', NULL, 1795569354388539522, 0, 1795569354030238424);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x1ec344a39e7941ffe3e6129f7a0e503f2b67ed71c79302b5825e31ad6baec4e5', '\x010bdd3e96ad7dd76661f4f35ca3422f4638b4d41f0a0960b645e5858871a56ae8ab03b8b05b44d2f0eecf71ad', NULL, 1795569354405642832, 1, 1795569354051948893);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x392a891d04cd189e3905801488c1ce3660d2ca63697837d9615e8d108acae952', '\x0134e391b4ae209f7726107d6637e01f87f926b85582452a3b748b97a8b2dbe4731a34cce6013f8642c07e8a32', '\xe5ec09790023fdf18cd114c5c15d55e4', NULL, 1, 1795569354085030701);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xcffc3b6157b134be65b45439bde493fa13f52f6299446d975cf930fa607c569d', '\x01b2806a5dd8b5c2cbe41e03021adc014ed4ffa45dddd58c761a40ba48c1144d1af09bcb14abe0c10bbcb71ce3', '\x784b591d42d9de0ca50bc46e88d2316d', NULL, 0, 1795569354615367780);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x98eb120263416f641905ea31481cc82464b5b91021628bcb72770c388f56ccee', '\x010e646846fba71d7998b4640bfb0c71d98f3272d069681d0384ae57c8af518ae7712a0935177e6cae55e08085', '\x55c5c72607820b101c4d7ef5a50b6433', NULL, 0, 1795569354649624568);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x5f845d22cbd102c30bc40c3c9861accbb5fa03090f3a0e979d166f8bafdf5e51', '\x01424ffad06e98bded027c5f386d2e02022c25456022549ef4f2604669e7218174f68edd977cbcf8000a7c3cc6', '\x78de5dd45f522aaec1dba3da9d1bc799', NULL, 0, 1795569354699694028);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x11981913eba1044203706520a9473b4d700ebeeb5987160cc485387a47911e4e', '\x01c15f8bfda5aeb4334fe5f8bfaad006ddadeb191b6b4359c7e34989bb7c710c9c9cea42aac04cfd65576c10c8', '\xe01baebf8dba4c4f2c7a568ce463ff41', NULL, 0, 1795569354750529247);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x77f36d8ccc7c27e578ee36a0367690ff95d0959db3d62b79eb3123a12e5ac65a', '\x01a2dabd89bd1f3aa6df1a8ad6a2b3fc55ab0cde31556c7c136a51a2ece067e3efeca08adb2041611c7bfbd707', '\x86e8adca8fd64b6176f675c671a5daf0', NULL, 0, 1795569354777407056);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x80656fea11026aebe03fd5b7cec80a72228d7f4e13dee89988bc3e33190ed674', '\x01fc3db0fd7943958d9a721bea4f1c1ba7865f20e0926369d2a33f3fdca8c56da0346e7455525f41e69f1601da', '\x91ba50d1aed0a393b25f344c3aeb5972', NULL, 0, 1795569354808236160);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x0708a08dd2e7ab76031d6d281cbe4f1b3309f063e30552fb112949bd73e427f4', '\x01e32125d8c62487281babd8330389154b08b26c30e7565a6a751ce93df8833846cf33c99ac097cdcc5612b92c', '\xaebebc3d08b49be947d3c467b72c13a9', NULL, 0, 1795569354437249214);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xba670f595921d3abb8d704b301c41b4d82948833612409ddd30b1fa890e0bfb2', '\x0129a93f7f8411bfaf6e55b756540355c970c778e15abc3b8b5142d1dba06f20f009f208fd4e59ce024740dd2d', NULL, 1795569355048910016, 1, 1795569354520072017);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x182d3d3c50ccfc567dce0363ede97730ade0f657f06e446b303bd1ad28887e8f', '\x0120aaa5be08bbe2ea07f47ad61a9e4a3a9909566ef71e5155a945ccc07ff0a46526cc750a9bb8945c6da60397', '\x7559ea7238822c972d12c4de453a16bc', NULL, 1, 1795569354570634365);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xa38feb724801f57bfad2e682aba3f163a72ef6bf87251f02842326d736d58ed0', '\x01803afa3e3822b1b61f3eca482f169f1dd43081c8c3c5450f2d93339dd051141674d06a5585602d979b7f2a00', '\x356d97090bd902789c510a5056ae9bd0', NULL, 0, 1795569364448065144);
INSERT INTO vcon.vcon VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x90ebc043b2bfdc32091d24bdb655fd5e50e5f63e681ff44332944ec31b4d9b7f', '\x01227caefa1a636305532b69ab70e80d956d3d635e24793719f8e6b3101a21469fb18a52bf8cd89b046e251622', '\xd9335f526c764a8afd9fed1ee8f10a05', NULL, 0, 1795569364495822505);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x3888432a9f994a1290ec93db6e499d624cf129a74185a932d77d7c23d4b50a13', '\x0158b6e4ad1e54c57b4f3de2597f26af8dfa50d9ab2d98f163b08b7c4549746d521d332693ae55a5878803eb15', '\xda8fb156bc7324b83935b3eeb5cca42e', NULL, 0, 1795569364546771503);
INSERT INTO vcon.vcon VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x22083ce9a71961b56cb033aeb9486ffa1d3c69585d0eddb07a6168d9941e0288', '\x0153345d4af77ed1dd5e9db4ac4725449179427914d39384230bc9da76fca3df9622c3246cca6dd1c4a75a320a', '\x1a1254c959c8435c58abedff4f2bb80d', NULL, 0, 1795569364599466566);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x8ad67cd52678b00f97f450c843cf69cd', '\xba670f595921d3abb8d704b301c41b4d82948833612409ddd30b1fa890e0bfb2', '\x01cb56e4af60e7eb39ce467249b05f07c8a70ab09b38eb3ebb75bd74459047d477e6ffd512', '\x01dcb7f5b063104a290f5d167838d6e3ee182c32c482f2d885a205830d52', '\x01b6cdacb4409b0b07cece3cd1d0bc708349f461bcb219351c534c7bb4fdb44a709209c095', 1795569354520072017, 1795569355048910016, '\xd95d2fc90cea4b79', '\x01355e14a8f417cd547ac8c765d456602957550df4fb8f4be4e63c128b72d1b5a05547e373cdefe682020cff35f1620ab5503a68b6adaaa1a09e3e2d57', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xb6f94cc0086fa6f99fc4be78caf594dc', '\x182d3d3c50ccfc567dce0363ede97730ade0f657f06e446b303bd1ad28887e8f', '\x01d6141f7e40affd902a6d228a4cc73834b2ffcdba477a1ca4864bef4d6ad7bde09f648a1b', '\x016ab0650bfb8adc7880a77ed27ff2abccaa2d31d3140341aac3c553e50e', '\x01fe344d844554fc8f28f7565c363ae5ace35c9d038c2e1fb6ac55972a97f46c7021852680', 1795569354570634365, 1795569355080282342, '\xd95d2fc90cea4b79', '\x0173683670669d08fef0e0569f5e60d6c0197512e823c5aa83aea250d9cf25a93ae0c765463171e47991410fb66d093cef7be3c95701f0521d8e0f94f6', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x784b591d42d9de0ca50bc46e88d2316d', '\xcffc3b6157b134be65b45439bde493fa13f52f6299446d975cf930fa607c569d', '\x01703fa0da82d5042d62e48a575fec869c0c4aaa8637229a59a650f32f42a5ac7109d7184b', '\x01bf21e26c225d94ec70b256e2c35c071b7b64858cb912da18d15b188067', '\x012ca13dc3bb9cf3273e308374e4092b3f0a8dc2cdcd35d1fd5ae3ee5203a5be290462fb9f', 1795569354615367780, NULL, '\xd95d2fc90cea4b79', '\x019a0ee22dcb2d0b48fb131ed75e8e7315eee2882d17b6a6fa6d2a098beb4337d1b89b0361e89c94bc4e3d50e4b75d1cb2682cff4b1feaeac3f0a3c8cc', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x55c5c72607820b101c4d7ef5a50b6433', '\x98eb120263416f641905ea31481cc82464b5b91021628bcb72770c388f56ccee', '\x01559e254131f4371252729b4f81c4c3fef748af183918629c53bdb68e445de984930ca546', '\x011f9664b179123f332d03c10ab131604048e6ecfe3af6fbfb3908e61451', '\x01b0c7126c08caab6d767cc9bd05c397d71078d8f15604016bcacc88fe8bbba0ccf57337a6', 1795569354649624568, NULL, '\xd95d2fc90cea4b79', '\x0160387bb235ac288e56d2f38569e5a4396a5ad67238b6020792023cf5deb3e068d5ef0316549427ccbc10bd8617f4e880153dd3483465187211ecd641', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x78de5dd45f522aaec1dba3da9d1bc799', '\x5f845d22cbd102c30bc40c3c9861accbb5fa03090f3a0e979d166f8bafdf5e51', '\x011b649bb6f7106b94f4359b2c2825ba0f9e8d2def5a7e9dd66ac013e8f86b79ad6b2b26cd', '\x01fb6e13051cd3901d8a95a8c90c60d21dc1a44cb16b750549cbed7a0068', '\x0175fd80a8b755f714f5fe07440e5aa231511c4175125f1231de54c292df49af279f3e77fe', 1795569354699694028, NULL, '\xd95d2fc90cea4b79', '\x019b52ee9ad58d64974fc07260db0a5bb96fb3e3b93cd6ca681d5d9d489a200d168fdc448aec0ae3fab033dbc4fb8f86ade3a9a8c83fd1eed127c5a54d', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x2cd8c79f2191d994c775ead9ed87b9d6', '\x3d5b2e657d3466ddd2ec94bd56ad8fbe611280473c19a92f2f9df3e2ea3de0a5', '\x011c1ba385b4c23fa1c34e5580cebaa4b7c38f06f1ba1ad12fa67349a40b11fffb02ce8cf5', '\x01c457fc5bfc5909009e751c6342fb903831e1ce375ad341340d5a538a77', '\x018dd97e8c367fdce9f9feb9ed0636c143652e68f261ac5f72797be2da3be7af2149090334', 1795569353979722733, 1795569354371965693, '\xe2b47a9e099fdb7f', '\x01d1325a20be11f4cb6701e4de9b4c0a7d31f810dfa2e70d5100c3a29191bc0c96473bfabb461509f5f4ec2a1e2629b15a8327cdd01cfccf6ec0ea2d9d', '\x6f724e8a2a332a429d144336ee818330', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xbba95e756b306de519228a93f8dd059f', '\x9825f0447252aca2e75d0f06bfc4e4cb557476206c1a27ee1071a9b44ed0eb74', '\x013efdda02732830c2e05f0a87202af046d8cd55eeb9de5c898d7e9463e999df3db348c7a6', '\x01dba6420011af4357fd16c16b3d9447d0f3d288750f07978d6c932d0448', '\x01c145878e5047b5b14607b740cb5055234c69273e1954d35f61f6d9df36220eb0681b5ea3', 1795569353911517037, 1795569354338642183, '\xe2b47a9e099fdb7f', '\x01dca63fbaf5be026a94cb14eaa3bef3cf1a9e2207ba02d341d10eea78504adea7c3f4ce993705603bde49ef46a3b33fdbb06ab3023b93545ee4d0b405', '\x6f724e8a2a332a429d144336ee818330', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xccdafa572aa15e852a9843bcdd4373c9', '\x0f78ebf4e91ab706b5047432374a3769ae3c18912667891f1274119bc72c5c5d', '\x01ed15a2b40c5076ef4ff20efef0ab7ea9fb6d255980ffc4fd2c24c53d400d154eee2bbf33', '\x015b17002781d1220aa6c134e0f0da842dab99585165a4b38d71343f7c15', '\x01e177f10fabc282ab286b2b59ac5284c0c1e1545c19f024affcd7123b16b5970d4a945031', 1795569354005487291, 1795569354379201339, '\xe2b47a9e099fdb7f', '\x019d96c8beb2bbdeb0c85a68b1fe1d0873f4a4558de1d7bc19a3ee23f76207c6052eeade94e8064d29b7f8440c3e52af256d9fcc0a115c19a88458a546', '\x6f724e8a2a332a429d144336ee818330', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xe09cb7464b63c54d4f4dc218b6944e19', '\x4e63ffe0bd120e3be42dd1e0d3bcb63d7cb5c0d3a16b58a5259ac17cf158da62', '\x01f712e372fcab53425bdd6673e908d1639a8d1b45ef9b035f3d4e687f2803d1c5c3afa868', '\x0191e549b4cfb4da808b8a2fe29c7f48c74a04c8cf3c3453d63977938f26', '\x01f63485c9452d7a674b4616192fe17f364e06d3266da11c66c78441b4a249e916976fbb94', 1795569354030238424, 1795569354388539522, '\xe2b47a9e099fdb7f', '\x0129bd9ee24c1795710eef4dd711e666a3579b70cb07f888ac4e088f3e91c57bbf83d33e7086d8db5655a9317bb9cc7085938db36fbd895d4d21d2ee7b', '\x6f724e8a2a332a429d144336ee818330', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xe01baebf8dba4c4f2c7a568ce463ff41', '\x11981913eba1044203706520a9473b4d700ebeeb5987160cc485387a47911e4e', '\x0123bd1b95f196d9d15c1e9353a2e72d32a2a69517aaeae831b336f5a1ba7421365aa4b3c5', '\x01b15443eff733d529b9d1907b7ad7d95f2876e5d2a3ee853cefd6578e7e', '\x0182c8694dd56fdfdeba2cbc3bb3ef05a0828c09fefe0faac80d2df599bfbae8919967f3af', 1795569354750529247, NULL, '\xd95d2fc90cea4b79', '\x010914975ef2076c87bd662f488cae63f1c13799410a415cb215aadd5a45582eabb83e1640710783a3e6ddf24a508de2e83797fbf367f4bcee8120eef1', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x86e8adca8fd64b6176f675c671a5daf0', '\x77f36d8ccc7c27e578ee36a0367690ff95d0959db3d62b79eb3123a12e5ac65a', '\x0113e8a94d21a4958fb110f911757ddfc9b8dc09b55665cbb4bfb21935ace005032faa62a8', '\x012ca451f04c1d20e24d3d03df56483c4b6cbfd6ca609adf7a57f81d1833', '\x017b36919a666535a7f836735c064121bc9e3798e34081d07034812105b6e366e28b718394', 1795569354777407056, NULL, '\xd95d2fc90cea4b79', '\x01d3334afb24d27c2d60b1486cb32ccc86d0e978d81a9559516059f2e4d03d076fd1deba8b0df5b5b5460ef9c4de332a80b3818c8b92e94925dfc59589', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x91ba50d1aed0a393b25f344c3aeb5972', '\x80656fea11026aebe03fd5b7cec80a72228d7f4e13dee89988bc3e33190ed674', '\x011529fd51e10ffc61b372cfee2d507bac0ec2d4f1820577877e805fbb849c22193d21381d', '\x012e7d6804d06a973e72d814665fffc480db892da1520250eff260079ef1', '\x01cc2cc8c227aa94f735e83372d8ee9ce262e523d54423c09ba8e340dae07a2a1128bef622', 1795569354808236160, NULL, '\xd95d2fc90cea4b79', '\x018b5fcbec90e85bae5f5e9e5e54c3f08df9422da711eaafc13c58e30eb48f737a715f8103088013f07b3ac28febcb60b134556c08ef817a6c793d29ad', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xaebebc3d08b49be947d3c467b72c13a9', '\x0708a08dd2e7ab76031d6d281cbe4f1b3309f063e30552fb112949bd73e427f4', '\x014e526d4bd12c949da9268986b2face0d1f924396535ade9486ec49b46be42dcf563f1474', '\x0185e9464d6ab2540d2c97d70d321736ea517f7019fd4aaf8597c7a2395e', '\x017dda8926a6c5a7d004dac7631c8c3bcd7014843330be310eed7f1cfeed7f76abce791d9f', 1795569354839889626, NULL, '\xd95d2fc90cea4b79', '\x01445e1014cc4d3dc9f0cd68c7469f51e5c5a72c401c1b75d0f75560ff83ba247e0eb0473887fdb3ba17fb8e9fd0c925128ec5efa45e0b9516daa89547', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x7559ea7238822c972d12c4de453a16bc', '\x182d3d3c50ccfc567dce0363ede97730ade0f657f06e446b303bd1ad28887e8f', '\x010439fd44d2a6a185e1ecc7bc0004d2c34000d0ab90f8bd9051c0adea8a0ef0210535a74e', '\x01503d665c0b7bf291fbe6c82339c940e4855a30505756e8a8151ebc7f8c', '\x01d6c8deae817ccd3ae0ac1566c2bc4682f4403ca7dd5cd2363b78694d9d506b0bef7dcba7', 1795569355080282342, NULL, '\xd95d2fc90cea4b79', '\x013d8eb4aa3f4af60f1902778c4abdedcb41b4facf339356b5c12fe3e7015cf97221eeac1616627c606d5cb491a3e925a1f4047642a3097dac81668db5', '\xf8510a540fdc9e75b8d877ac55f120a1', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xb24391dea4117a184a6cc9a85b89c4bc', '\x1ec344a39e7941ffe3e6129f7a0e503f2b67ed71c79302b5825e31ad6baec4e5', '\x01374b250ee6eaabad664494b0580aa7012a1cfb348e4c920ec2439e6a704e9cd464b931cc', '\x0167de5ca93aa77f47644ef895e69e1d9fa6fcbc8b27d6d228c4aecf5179', '\x01964d365975aea2feda121205ce10652d0a1e01337541af360b8ef1cd11746db9466dcc7c', 1795569354051948893, 1795569354405642832, '\xe2b47a9e099fdb7f', '\x01f7f9212a3201978bee9eee9bec4d7f433dbde259d5252d9680fbcbfd848d4b361633fff4d7a4aa69ed164211e69429cb6e97c0fdc88a41dc416adcab', '\x6f724e8a2a332a429d144336ee818330', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xe5ec09790023fdf18cd114c5c15d55e4', '\x392a891d04cd189e3905801488c1ce3660d2ca63697837d9615e8d108acae952', '\x01345b9624159067c1132083969ebf4c2f1c11067ce9895e9e213cc7b4004b82748cb4dda4', '\x01df52d4d521d4838b02775cb2468f531b28537544776024e0723ef53516', '\x01df46c152113f99d986da9ccba3a0d20bb1c4474ad0363cebe4557c50754d6de9dec000ab', 1795569354085030701, NULL, '\xe2b47a9e099fdb7f', '\x0149f2ccc34f1cdc8f0d0c2d075e5cb1d8140b1f0daeb73e4ca85f4824c0858b791c498ca9c7725464a24f1089910b3b16370a46c532dbb40a2b134238', '\x6f724e8a2a332a429d144336ee818330', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xb9818a16dbab1aab7eed824bfe60bcaf', '\x717b5fba4ff4d85c06bf313880907ee209985ba6e8c643217084160e74052f1d', '\x01ffe8d986f5c46ad41b87baa1fda493f5179ade9c4f75bbb28b059df6695bc7f87a4f0740', '\x0179caabba79fc590d67853ca3b472c5ddff7aafa82fdd8c2af30bd74f94', '\x01e26bb776519a963c6c82d87ffbf3c59d1ad41e7f9b1e015a7ba8dabdac3a6ded4d25b9f8', 1795569354134413099, NULL, '\xe2b47a9e099fdb7f', '\x01ba14df1d9fdc01d725dd24a6d8eefdb80feb010fd23ad8b19af9e703af1b0e19fec79a19efd390317262fb087eea7f412613c52283137236a9363280', '\x6f724e8a2a332a429d144336ee818330', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x8a7449ac1587238f7e501a9cc41ae642', '\x1d8935a20076b3f69ddccfad34d596a1b61cfc29804db2eed03b48dcf9b96aaf', '\x01fb54cae0c699cd1ec6b65f3e8fa97663e1e7de6282eb6f572e6fc0e9e3b7a30c45a06634', '\x01a8701d8feb9736eeefd0378654e3bcf2a756004365d1f3609293413fe6', '\x01bcde16358f8ff16fe60e1b2712c81c635528b8359d4f6442a3f6b642cd565dd00209076e', 1795569354194688141, NULL, '\xe2b47a9e099fdb7f', '\x01b50bd9e7ce55ff46d6fb7beaa085e3e70554f6c4cca5eb088fb9e59fb26690bcdc6e8c3d1286c33a151df085291100bac497588827bb60abaac88ed1', '\x6f724e8a2a332a429d144336ee818330', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x9ad2b8e68a360e09f004eed2196d77ff', '\xf4152a45fc9f89a72cab999a4730b9144d771575b20775155f8a5aa91ea23150', '\x010f51b4bb90304578480fbc53636d523000a7152281d41e4cc1c270ae83486274d1c2c21a', '\x019354d80fe777d292a348ec33a5bcea2861814c65c0bbbd5234dc54763f', '\x01447e3af919b94468949f94565237abed0956e4f0039925da60c7d30835b8ba2a24484bb0', 1795569354253105666, NULL, '\xe2b47a9e099fdb7f', '\x016bea97fa610712a4073dbe4d6c0b3af73d840830935f265f89a85a6b87d6a9c561887de70a53ce84041defe5774c3d217ef0fa95c6a35d7be65b9fd2', '\x6f724e8a2a332a429d144336ee818330', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xe981f67ae55ad1a1a8f81a05f001bdfa', '\x03720e93a26887fb2285d492ff8bec49bba7170a69954efaa420e964e6e29cf2', '\x0104ff96dcaae81e607cabd995f8312f0a3c8a0fedd548781e1ab4019742591ee24b72b893', '\x016f5e26332d579ff3361c75872567420a2ada93454718f756c3ef95fd40', '\x01582d8399ded8f99fd9e0db1f3a074b5fea3f1c81253d87326321cfa674bbc12990e9f928', 1795569354310487477, NULL, '\xe2b47a9e099fdb7f', '\x0102ac445fde4a9b91b1facb64368f6cce5ffaab8fbd08d53b8f3dcb73a3b35cda058a5957681aca3f01ff3ae30ea74c70315eec6219fee776538770c0', '\x6f724e8a2a332a429d144336ee818330', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xa53a037d231ab9e23edd702a948c940b', '\x9825f0447252aca2e75d0f06bfc4e4cb557476206c1a27ee1071a9b44ed0eb74', '\x01ff0d59d5e0a70a7f6e695035ca0e1c5fc6b733af3435d32159b198d034c9e276267b22f4', '\x0188df457b6d7b18186c577ffdedf97ebc25543173fb2c7bc4d0e4444896', '\x01851233704ecb30e08eccc4f1700ba42d5ee609f6e5fa9ace46106491b40024f91f0ea3cc', 1795569354338642183, NULL, '\xe2b47a9e099fdb7f', '\x012733a10dfcae70f04730ff9eccfdd7c8b116bcc70b80b9ee9203deb4a46b542e9a5fb038417cca5f431ade90b7b34ae82a46bfa4f8013c36a6c9deff', '\x6f724e8a2a332a429d144336ee818330', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x4c19cb6723d6585b7e16083583d3c02f', '\x3d5b2e657d3466ddd2ec94bd56ad8fbe611280473c19a92f2f9df3e2ea3de0a5', '\x01623199687ba36789dc9cd87526d498084620d014e6f73c4703211a57997f12a94512aa03', '\x01e0fd8f7756c181de99bb2f7d6434979f43bd7753e414934b3867122309', '\x01bb49f1009e08ee26392604b5e5e7cb1527d71c454a4dc9638baf6b1645038ae585e60669', 1795569354371965693, NULL, '\xe2b47a9e099fdb7f', '\x01b814d73aff234feee2b09614a7e97f0fae83480c73c13936329cbca572d7e27d8da107bdea8a1efaff856b9b1cef805c52bdeae2a18c834562780247', '\x6f724e8a2a332a429d144336ee818330', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\x356d97090bd902789c510a5056ae9bd0', '\xa38feb724801f57bfad2e682aba3f163a72ef6bf87251f02842326d736d58ed0', '\x01012d704c31282efb19a94d0e0eb4dbde8a84721ca0abbdc123f24ff52e1ebc585eb7489d', '\x01e9627a8faa364f5f52fc10336f08c1307e69012dfc3ed7c8f751ceeefa', '\x012eab79e0f7a8b63cf886545c1969cfbad4956000d562da46c461b63a7079abd54fa14194', 1795569364448065144, NULL, '\xe2b47a9e099fdb7f', '\x01b1c5612b5c8c7a10ad1aee616f6ed58bbe8b1ff7d7c8cbb4e6191b833ac07b8658a8d88787efb16e87484397686185b8c7134e4590ef34e9fd588030', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcc56f5fd5b342df6aab4d9db4471cfb5', '\xd9335f526c764a8afd9fed1ee8f10a05', '\x90ebc043b2bfdc32091d24bdb655fd5e50e5f63e681ff44332944ec31b4d9b7f', '\x01cb0a9a25f18ba2497cf655230dad254f7556deafbfe594b04d950e0839ca6a4c5018b506', '\x01ead49289118726176e955bd598d1e7ff5e198465c4d01d779f62b1a0d4', '\x01fd91d9675fb50fb45f8c58a0d57837859fb3e89b5dd15f835fbcca810cc1422e07734d97', 1795569364495822505, NULL, '\xe2b47a9e099fdb7f', '\x01273430c6067bfd55d781d6b6c05d499afb61049125e56c834a5a925d88c3ae7b6f1c07ba75440ccc3abb6578598795d7710a573d31947810c7b890e4', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\xda8fb156bc7324b83935b3eeb5cca42e', '\x3888432a9f994a1290ec93db6e499d624cf129a74185a932d77d7c23d4b50a13', '\x01d92eb1d40c80409beaa7553c8735a61de58710fdcffde184215675c506a494dc99c5d7b6', '\x01178821c71a16856c8eb7e7320db63af92d5fffe35d55fe7a175557b664', '\x017405ffc12f0a9ebcf90eda8f4bd289f77ca533ac8705f68a1132889bb601a88380501f73', 1795569364546771503, NULL, '\xd95d2fc90cea4b79', '\x01fe239e52644cab4ff9d44c240a760b9aede78f928fa3d8ecf62d7fd66d30e4385c15c718e015c86623db1dc67a2b2d392411ad71bffa85df90d218e4', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xcd07f74570f0b6fb2dadb7f28cae0780', '\x1a1254c959c8435c58abedff4f2bb80d', '\x22083ce9a71961b56cb033aeb9486ffa1d3c69585d0eddb07a6168d9941e0288', '\x01eaa3ad0dede7b1f43b3cefaa700c67915603850b59167fee7dda0be34623e70538c1cae1', '\x01e3cff50bbb0aa07241c217554c74f895c0f04667f56d5a468e69105ba1', '\x01800ab28544177d0eb52e2ae9d8ae0a82545e481f3e8275079d16ae9fab34096c183dab09', 1795569364599466566, NULL, '\xd95d2fc90cea4b79', '\x01998d5943794b382ed007d0bde305f9c8871c02103241a0e4b27c9b9ded3b5be3a70ae841a84736b0177b8ae40dca650199fef89a9b5b6317a52fdaa8', NULL, NULL, NULL, 56);


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


