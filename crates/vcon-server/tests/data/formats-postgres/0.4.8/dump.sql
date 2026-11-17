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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-17T04:56:12.034109947Z', 'cli', 'tenant_create', '\xa54dddba915ac651994ab31270898b0b7f6b2edf47d77a70bbdc8bfb4a898634', '\x01857c0acd5e22364a5149b534b7dafa5f452eb891eaca23c22d8bc720f480ec9b732d4d994382b77c553463697bbf7e5382016a4ca5773cee2f395915', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-17T04:56:14.754383922Z', 'cli', 'key_create', '\x4c79cc08ef6e2c56722b677e9fa8fbc29ad4f843a329945372ec10f51743ae94', '\x0139b7cfc3dd92172b920e92a133780011a2a00df18c6f79c307b7716ac6e8732bb4d960938d60aedb5caa8742', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-17T04:56:17.19035296Z', 'cli', 'key_create', '\x6fe6bd237bbd93c91a3eca3361a5736e2ed70e7975fb0452e363e0e770c662aa', '\x01163884d50250d6f20a9d290356d8f263ce4cfb03f12f44e71b3a1ea16568df5db77af8a7410fc58056c65ff4', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-17T04:56:20.195209388Z', 'cli', 'tenant_create', '\x565e339458f6f122ec6648030487753e00d83c964dd6200e4f4b50b6ab88434e', '\x0108098c5228c5a77957998e851b32d5bc587e2548dd45383b094fb22688a07e95e9adab7a9fd0b4a2769a0305c9e0eb5869f2179faed59943d475c83e', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-17T04:56:23.430339265Z', 'cli', 'key_create', '\x1ce66c3972525fd1356e7ec7bf1e9740ca5272141245fd4d854a708f7a5b9c48', '\x011d8b32c8d913d437ade98ecc12961eeac494b3c48beb05784747209c60bf87c7cb8f62107fadedb2d06217c8', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-17T04:56:26.878381975Z', 'cli', 'key_create', '\x26872a75794cee387d8c889f7d0a7e073d950c14022775034c247c52dbea56f6', '\x01f046a6d6e59df3d7e245f9f4231175024e817b55293fb131b09da9f12eeaca1a72cb1ef658a640754cfca3ac', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-17T04:56:29.926567167Z', 'cli', 'key_create', '\x6e14b13499284ea5ad0fde9da1b0ac966573fd860cc8aa969eb8c1907c2dfb35', '\x017e69d5aa19dd4361f338f9ad89dbb6989bc52b3ce5a7a7a9a951adc67bfffdbb7a46b791be689b93331e01ad', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-17T04:56:33.643967834Z', 'cli', 'user_add', '\x634b34bbefc96f6cb535c71c02cf52dc217680fd57bdf48678bf52e25a0cd656', '\x01615341cb20d2d23ad0bf98cb660baa2958289d64db73c746c4dc8d1696774f9c7249add0d394c5c9c3ca77c2e00682320c749357b63c753c21af74e6', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-17T04:56:35.870565329Z', 'cli', 'user_add', '\xb09e79a394faf4d4e8834ee8eb43c79d2f53e3c202c0fcb24ead5a72d1b326c8', '\x016bfb7aa49661f903f3df44c56af34a7f74b255d8d2ab0d109db56fdc1d0bba054dc6cf45ed5bddf8a416ce34c8aaa06409dc3d1f0c79c667d6ddc4c3', NULL, 201, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-17T04:56:37.943723197Z', 'cli', 'membership_grant', '\x1648921d6a4068f3bb859983b971a4d887ecb7448a8ba8a28685e773c52f86d9', '\x017525e8dd153ead26d306dd108017bacaf65c56461bc6427100efb2189cc7210016b89bba1e624ba4f7f30d2c008554ad11127e783ead04bc156c8cb6383aaec81bdf084792068a98515cd08d26da054bb81006a91afb142b1d1bc5d91235a433b14c33b0', NULL, 200, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-17T04:56:39.970386483Z', 'cli', 'membership_grant', '\xcc8c29d5fa6812dc6ee1c37bb0e5da2cd8711890af75391a3a4417f2986080d3', '\x01ffc1c799dfd25c8f37afd0cac4c262817421678348cbd85e44a57a5ac13a277349bebc3030750a48f235b023e54e2ac47fdf8a9de9f541a53abe71b8427539feee88574c7c9537172ac482e0e52035e25f362447d9a122373e8f86480980a6dedc7e21cec1', NULL, 200, 1, false, '\x8c4c10e0d9f597dd');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-17T04:56:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\x8c4c10e0d9f597dd');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x8c4c10e0d9f597dd', 1, '\x0197efc511b4dce80ce1766a01dd61d5983e903b6f9e1a3e92c5df7992bc1a2a83398132612afb81c6a72c929d2a29fc347933d0ebf50dce2ab73d8015');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x6f63e0e0d995da56', '\xb23d4c178942efc7520ff44b17f4c8e7', '\x020f752bd535cfc504491135baeea7184ff8278d115f1bb9470a62ebe2227985', '\x016cd43de91f7f075df6365887ad0b3962962634112c541b07df121a9c6596a8f2', '\x01eaf7a88a21b9438abdada9ac1af094b2fa056f1856dcd71d6d0aea751aa6bd630f601774f1beaac301a1c5', 1794891377189683571, NULL, 1802667377000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xd06577920e81e3fb', '\x283723e9f4593a5ab90b9fa0634a1415', '\x87a87c69d05cb0b592aa6ee207ff2bfd941868a2a0c6d9205bb8bc1bdb6e6aca', '\x015f06a29f599a7850acc1ebd61a8423c679e0f03436ca6be7bbee41b70f24f18b', '\x01f83089395e6b8d903cdb73c32d96b386d79a4f1cfc364b20eaa99f2ef6c679dcc6b1a8c187d1e5b6811e51', 1794891386877528234, NULL, 1802667386000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x7055150f151ba6cb', NULL, '\xa62743dbedf58321c38d843da0853ec057bb8d7e0133afc43f73288cd071bc64', '\x016c13d2aa54243458522aefa3d818f72f61626ac1d7ec9386620a2c8e55fb86954e', '\x013225bf8f0983306cec16fe033d9868dc38476525ed0855927411546a400ad0', 1794891389924829982, NULL, 1802667389000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x684b7422acffaf56', '\xb23d4c178942efc7520ff44b17f4c8e7', '\x3207da07f47b9d88adc68dc7d7e1bd38507df5f6707fd3dea891c8628fdbf062', '\x01376cda6f31f66b93d6d6a3580dfd4d2a1a890439d0aee95dfecdfb02e666c5b5080adbaea03885e344fe256ad4963babc999573383c05aa479', '\x01fe3eed1280392d9848af5fb3ad6f634fb0e3a01232091037a6fc58b04beaa8a0046d687e423663ec38a37a0e3a', 1794891374753475673, NULL, 1802667374000000000, 1794891360000000000);
INSERT INTO vcon.api_key VALUES ('\xae9016800219ca90', '\x283723e9f4593a5ab90b9fa0634a1415', '\x576dd4536e8fd8e351514d750f1c1da0ee0a86a6f1e60154c4ab1da292e05250', '\x01e60d789324dfb1b1c24d42082226d8f12a740f0f998d98019a20db8255e9a17e9408afa6e655969227eda6759818983b1447e430ad14d37cff', '\x0118790849b6bc050fb1cb673fe874fc8c3037c55ac473e7b757c24225726ba8cf5e761f20723cfa0e1f3dde0730', 1794891383429677349, NULL, 1802667383000000000, 1794891360000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 2, '2026-11-17T04:56:14.75837744Z', 'cli', 'key_create', '\x5d2b70d9a8d57fb9a3098904f8ff22a30129aad68ab3205661006d2e7142b69d', '\x0143de7d4cd8d1b18c020851e6074aae40ef51ac025d6d3d8385e22324afd621aee277e50e6d21dcb466a5c298', NULL, 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 4, '2026-11-17T04:56:17.197484809Z', 'cli', 'key_create', '\xb2582ca9db8fff42c550b858d683a090016b523b0366b959af15c757bd13d5da', '\x01f6e03b092b046c503d48f126e3fdbccf75b1e846f92e6cf5e0369e986c613c88fa71b41468d868949c175acc', NULL, 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 6, '2026-11-17T04:56:23.436773921Z', 'cli', 'key_create', '\x5203ccf2c9f58985cadbbade21d6368bd3306f3d8ce2bae82073c32f113be497', '\x019b265aa86e0aab9d65a855c3563aa0e860568c2230c8484756b053e3f6feab446899e2bc76ef4dd65837d684', NULL, 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 8, '2026-11-17T04:56:26.892036546Z', 'cli', 'key_create', '\x7f1374e9dcaf387c6d286dd6aa9f64ef74b1322f643099eb16ce7adc01fe8f69', '\x012775be3e99688d46ecb9d73c702bedbbb8b84d357cbec636642a450a3e737b8709e6602a2b153b5fb172d571', NULL, 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 10, '2026-11-17T04:56:37.948247974Z', 'cli', 'membership_grant', '\x1cacc213b16b574243efc35bbfae46e7ca9a01e74298fc421c54f62f618edc77', '\x014e9ea1602d5b4371959364bcc0b6582f91657b6502832e4d2007759e99473cbd78827d4cb08d5dd00ebaed9c26ccc58a4b9dc313dceba3c4bd042a65396bb0f074d087f3c02680c9d4a68d89c65aeba7da13feb39145da164d8719865fe3e35f2ba86145', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 12, '2026-11-17T04:56:39.974101899Z', 'cli', 'membership_grant', '\xef4a31db5d647f91c4ef6d40b7a042ef8c49e4c0e9b15625c98938742e322df5', '\x012b9d6162b405ea9872ede62dc7a5172bac101f5e768868ef729547e2bc5c80e0c200de4b97ae50aeb14e28264ecb6ea08faba45d26e990fab4c479c56c5ee2b70defb77ae852399f88143e5b4e0b92b913f334dfef8dfbae417e93ce9ca95344fa6e3dcd3a', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 14, '2026-11-17T04:56:42.289775609Z', '684b7422acffaf56', 'create', '\xe7723b2e642e52eabf08f30ead6c8601a4856c4da71a0434afddc3355ca684f7', '\x01963f4a6980a217a2ecc22de7cab74e07ebfe92e8aec9e7644821806ae10d6be4c30664a7b7d4c5adc0c1af94fce296b0cb43770b4c2b5865db4aa88280108417', 'fc44a51228f85f57e9fdb326dc1ba4b2', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 16, '2026-11-17T04:56:42.314015354Z', '684b7422acffaf56', 'create', '\x9ae45095b7745dacb6446255dab4eb183960e5d3cd5cebe0ef44f7a7b5bc85af', '\x0130b352c39a8e5903b0728500532da2076665e4169525582a35c2fe7235c16ee343a78b6b9c0d1ca350fb3d7d5f8b6c3d530ca7f4f9d52c78f79b8e9b1d9d6b43', '968c3999d1f5af0d5962cdd8a6f2e80f', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 18, '2026-11-17T04:56:42.332587756Z', '684b7422acffaf56', 'create', '\x7ff0ca73a5a3947b7638308d4f371ff43d2af964d77ecb7d7f9c90e8d0449397', '\x010749a7fe45e0e79b3f1e694a43b2e2df12ce864cf85e637bf5a825ecf849cac4bde7801652e5cdbb41cd00be831300ad3d3749f4b50e1c5094398ec0ec0b1cfc', '1db03c12259b4435d7f66ec0f3cb22b5', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 20, '2026-11-17T04:56:42.355912315Z', '684b7422acffaf56', 'create', '\x597273e99ab10b6aef5bfea5cd1ce04f61076535851bb9c6cdee27876b4d5b50', '\x018e2cb4884c918dc47ef19d5c1fb437d9e28e5299c8ce0f53f897c2cc44933902e3aa76e8b9ca5778e9cee75a131c00180615b6e4efdd4a3fdbf5fc193990d383', 'e93afe67f5420586054545958de66f44', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 22, '2026-11-17T04:56:42.375416745Z', '684b7422acffaf56', 'create', '\x6f60c1ee716b3ffae0b3a9245f9fa7d97e75342c2053afbd500c025e8f88db50', '\x01b4ec1dfed98eda33cd27c8cc7c83d1b61898cee0f4d07dbef4cba7515880b5018f87cf0ca6404fca8db2d2e875977f43d8c2d2be677e3cbe5b29f6a5de5014ad', '0b7b6661b68656d19936506eb549c404', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 24, '2026-11-17T04:56:42.4211344Z', '684b7422acffaf56', 'create', '\xb881c356013124d7bd9e9efae28c9588e933b4f8e2d10f0df0a579b0d38a8182', '\x01437c71caa70384357be8b03009f5969cbe401f877608b2fc3f18198700319cd021ea2c3d5439f2df74134a45d3cf8120db7c879f41659cea7aa98ed91cd99ab7', '141b1dc3bd78ea147adc904883e31b9f', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 26, '2026-11-17T04:56:42.436306128Z', '684b7422acffaf56', 'create', '\xb0af369dd1783bd3dd2742318008a71384a5eebd3a868c59ed437cfdf48888f3', '\x01567cb80d58ce684ff9d07ec9d7276d9e271ca2543164a17f0fc92fe518e7caa9af7e0be6f92871b25d14957c942997ca67bcc94ab0135b75152614af9a4ffcbf', 'aeda5ddc09ad8529461326fed7bf8e65', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 28, '2026-11-17T04:56:42.494792577Z', '684b7422acffaf56', 'create', '\x7b69ecf02f567ca6853649a711a889c618f7ba88712c5fe7ae8f4d10d6fb15a5', '\x010871426b41ceea9a18ec5418bd1225ca1dba6c9d4f1a32fea42d803ff31779ad9759d2b3638a4cdfdcb9439981f3ebc198a5d34f65a4d0a3c657ce1492c3e620', '5a2226c388b2dd894f58966dcdab0707', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 30, '2026-11-17T04:56:42.513471063Z', '684b7422acffaf56', 'create', '\x1cae3c109bf6651bf57a1b276ac3aafe145b743d5e439d9557815f6fb3a72b21', '\x014168ed541e5dc293e0abcb6de72c9c75679aca2958e98373e58de36a3138e7d92a1596360ae2e6e14e3400b997396384cc5ddd69524f24db22691fcc70bbfa55', 'fd7af32ef454cf4a88d887144fb5b014', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 32, '2026-11-17T04:56:42.537477029Z', '684b7422acffaf56', 'create', '\x032a66a7517bcff83e98073a0607e75da53ec5eaeedee7cd13765b97afc3cbe6', '\x01987d2716190e97cf48c7e3ad092ab7834d4bca9a3c7c96cb0ff31ca04f6d4df0eb688f53902de1088360601f8f86bfc9a11b37cd390ad6c59155f32a4cab6b41', 'cb9a74eccaeaf6a99cb53562c456d361', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 34, '2026-11-17T04:56:42.563604948Z', '684b7422acffaf56', 'replace', '\xe7723b2e642e52eabf08f30ead6c8601a4856c4da71a0434afddc3355ca684f7', '\x01b94dc4f92ce51b8584aae45d4e7491f11fd9f6ebbbab344a84293b9e459821c5953fd763d61d9a93e831efc674134af8793e16fa8092dccc162497a2f759e445', '3aa660bdf2c45f64e29343b429380b01', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 36, '2026-11-17T04:56:42.596060987Z', '684b7422acffaf56', 'replace', '\x9ae45095b7745dacb6446255dab4eb183960e5d3cd5cebe0ef44f7a7b5bc85af', '\x01f63b9a0f3eddd70bb89346fc6a80bbab3bbb26dc7c987e879e728dc4cb8b23c3144ddac7db8e5c6220a6e1a8017e4b9c455a13e39dde50e7c60f9f8497601ad9', '2697919c744a9b036b34f3f47c4a9857', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 38, '2026-11-17T04:56:42.621893277Z', '684b7422acffaf56', 'delete', '\x7ff0ca73a5a3947b7638308d4f371ff43d2af964d77ecb7d7f9c90e8d0449397', '\x011fa75e22ccf2ae4dd107bf917ba0f69fae5476f088e5c2bef888434c05c2977b69601aa042e0e200c82d4a2efdd9342cafcb3f677256019d9eeaecf7a6c388ec', NULL, 204, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 40, '2026-11-17T04:56:42.635776782Z', '684b7422acffaf56', 'delete', '\x597273e99ab10b6aef5bfea5cd1ce04f61076535851bb9c6cdee27876b4d5b50', '\x01cd7540ec4d0527552268a6d209ce96c3b5262ffbc603cd3f41b65695d92183020ed0b62023855efe9dfdaf7c40ad671d40f976144983e61a3a964c23bfceb21d', NULL, 204, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 42, '2026-11-17T04:56:42.644903484Z', '684b7422acffaf56', 'hold', '\x6f60c1ee716b3ffae0b3a9245f9fa7d97e75342c2053afbd500c025e8f88db50', '\x01e3493ff95ad02e63ae53c43a25ee3ed50d90fe392e1eaf694ce94491f3d0b5cebabd4f317f67f4dbd40cdb67fa66cb489bf8d08ecdd8a4d38f0a4f319a4b15af', NULL, 204, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 44, '2026-11-17T04:56:42.651064427Z', '684b7422acffaf56', 'delete', '\x6f60c1ee716b3ffae0b3a9245f9fa7d97e75342c2053afbd500c025e8f88db50', '\x014ad1e586bd5ea54a3cd1003add1e7c6bf0fb6dbe8f74557970352de08693734f96c1edfff91bad0d9b02fbd3423d8be6775d40875428a6ec40df3f2f35839d82', NULL, 204, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 46, '2026-11-17T04:56:42.69342587Z', '684b7422acffaf56', 'hold', '\xb881c356013124d7bd9e9efae28c9588e933b4f8e2d10f0df0a579b0d38a8182', '\x0111a7c9de04188c6803b96145ae597d510afd223641d7129c669f0f1ea770188160b380631e458419a7215c34a93d4e763d1cd8a1e6a1060d0afa45693f771d70', NULL, 204, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 48, '2026-11-17T04:56:42.742211876Z', 'ae9016800219ca90', 'create', '\x1de9b47eafd522a3e79bee94c607e9595b4c1d0a5225f58fec4ce1cefe48ade7', '\x014213f013e451d16d71737e52eb207d2fc05ad0175c66295fbf1da9141d5f7bfba3d33025fd7d8c7bc4385a6df48de6d1ac2aa543a043b9ac73ca5005890b27be', '32d0616fe88428a6f5889d2cd4d23465', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 50, '2026-11-17T04:56:42.795536568Z', 'ae9016800219ca90', 'create', '\xe9b7b3475e08915bc991db15ffb5249ba29665e6d4178975e810f5830c240a10', '\x016d6a93ff761269d3c81387c0863b50a1847d4e6b5889510c62a3823f00cd22494a21343e922de2a473b0549c833feb5f9711af6ed916e90b099f9034e10c769d', '8a86d1c776f6dfc2557181a2a48b7b6d', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 52, '2026-11-17T04:56:42.833723671Z', 'ae9016800219ca90', 'create', '\x2adc4230bfe749508d289c52f1874b52168b80cb9ce73ed15fddf0c03bcc2bad', '\x0151db37a523b3818626adaf99b9e08c952c1a42fed2736ee5a13fca72fd36fed49f93f12c882af97e827bd00f235300fe85691a1251606f25ae00487f8239c9f2', '25fc6a98105ed8e96df42f43cc490662', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 54, '2026-11-17T04:56:42.850549686Z', 'ae9016800219ca90', 'create', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x014bd55fc20add300f29cfc07626253a0631c99e042d0977b0301df893736f90fe21b79d325dcfff04d337ddae74d675ca342e8cef998c666f318955d8a33a0557', 'e303e974c474e0397c9e1ccdecd2d8dc', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 56, '2026-11-17T04:56:42.866248489Z', 'ae9016800219ca90', 'create', '\x58068f2633f20539e0a44b6f8d6f521e02c78a3e4375a0b6ebdda4defbf2984b', '\x0193cad9ce8e3c2fccf796eb33f9721f28c4f3c3430f3512178a417bb5081cfd27e9889e054701a2e6731caf24a0d42df2bcdba0c404cbc2b8fc6026ec084dc285', '4c91522031c9cbf757befecccb8c4ee2', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 58, '2026-11-17T04:56:42.883238634Z', 'ae9016800219ca90', 'create', '\x552c8de8b5f43d91c4b3c41e473943b0a4df28afd13bbb12a5d5bf3cf5913188', '\x0151589e2e0ac857e4337b4b7cc37ba4a0bfd19e4ea8a72d1651eafc69b904b0de8d21de727ade287eda9e891c490e5a9232def43010279ea0ad309f899420b16b', '1ffbc38a5175bc800d8666776af9b19a', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 60, '2026-11-17T04:56:42.97689267Z', 'ae9016800219ca90', 'create', '\x67b1ddd21bb4f1151659eed710e6887c70d78724b41f24f0dd7266f6ff283251', '\x019b666a4e2ab83ce80660d5bb755a6c9f9d8a5d25e8803be1e5ac285b998f25d38585e3a2d436fed2d6c08ebbe84d430826ac8182bff1567635dc0af59ca52921', '770b16fcf4dbacbecfbfa43ef12f6be2', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 62, '2026-11-17T04:56:43.037792572Z', 'ae9016800219ca90', 'create', '\xbd7a7530726b5f06ebaa88c7e55a0c5065398d0b4273c98c1419e28ae2a1a306', '\x013348b4d41a0d3f401ce4442ceb7af33da318defcc9a7a22bde1b67f6c93d05303292bc2fb0367cc92c99b3ecc38a804568619f27c541bc83e00741905632da46', 'b1c4835a7e58f8b8fa26c7361e052f46', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 64, '2026-11-17T04:56:43.063410038Z', 'ae9016800219ca90', 'create', '\x5ef75e0c4a1399332ec6f590399832651402377b9cb7731534421af64c1c102b', '\x01deaa1ca88117c4aeb12b964520cca254021a71dbc9a177ae1719080077f6eedd364ed242e8c7fc1dd570bffe9d845d618aef379b3454318e4b6ab1d69e4d7be0', '99300f9c7835036b92b37c4f5908ca8f', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 66, '2026-11-17T04:56:43.098507159Z', 'ae9016800219ca90', 'create', '\x6e205bc901fa69d2201a676dc15f81a14549e3b0376b76f3252101df9a88d6aa', '\x01a6585cebeec7f5e9e9071ee3afc8447517fa6f36d5bf0761570d71372315e85c7086c7df8a6e8fd0a2e4d0af8fcdeceab030a9b98661873460826160ca513e2e', '23e87f5b9d57db41f0af4de157349525', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 68, '2026-11-17T04:56:43.131609808Z', 'ae9016800219ca90', 'replace', '\x1de9b47eafd522a3e79bee94c607e9595b4c1d0a5225f58fec4ce1cefe48ade7', '\x0127efbcc3e75212e5711c89d71d89b7a4acecdfb19288c09f1c0e65ddfafa6e8def1c75a5c126cdede2f855f2fd935b013707dd3f31a9d2e0bac87dae27999f16', '90739613d619cb2038cb93a46c72620a', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 70, '2026-11-17T04:56:43.151040906Z', 'ae9016800219ca90', 'delete', '\xe9b7b3475e08915bc991db15ffb5249ba29665e6d4178975e810f5830c240a10', '\x0185576a7e08936ca12d1c213928b4ce248a068e83d260a13ad608f10620546c9a65c03695a175f4075e48e4aa79983627f4d34a444efd30f1e8b0a18f755ef5d7', NULL, 204, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 72, '2026-11-17T04:56:43.160578375Z', 'ae9016800219ca90', 'purge', '\xe9b7b3475e08915bc991db15ffb5249ba29665e6d4178975e810f5830c240a10', '\x01635d8ab33f14b4f581c98967f345b8789fd4e26e3e84802e8f4e80af2c52acb100f9cfb74b5397a751069984e7e67ec59bdeb597cd9c36757910a9b38f41fed2', '8a86d1c776f6dfc2557181a2a48b7b6d', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 74, '2026-11-17T04:56:43.224050499Z', 'ae9016800219ca90', 'hold', '\x2adc4230bfe749508d289c52f1874b52168b80cb9ce73ed15fddf0c03bcc2bad', '\x0153534d8f1776e0baba44c3eaf0cfc6f6538703656f393571c05922cc4ae263e582ae37633add5310dee2fff74356655774759d3464d3faa9e6a0f506fea214c1', NULL, 204, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 76, '2026-11-17T04:56:43.255036397Z', 'ae9016800219ca90', 'delete', '\x2adc4230bfe749508d289c52f1874b52168b80cb9ce73ed15fddf0c03bcc2bad', '\x01d780bb110b9332062cf194b4e06a0eb8fea9faf2341c0a2ed4a48ed17530a12ca3c160ceb8c819eda0c0b10d46ff405922b39891875279dafff71201a660c208', NULL, 204, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 78, '2026-11-17T04:56:43.270910052Z', 'ae9016800219ca90', 'hold', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x017db561d4cd4e3cbca984e9e57e375339304ab44900986fec677bf5f1ccee6bf22a9efbecd93e3a74e600c06e4d047e25da210061b726eee606579e891d3de3ec', NULL, 204, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 80, '2026-11-17T04:56:43.30017185Z', 'ae9016800219ca90', 'replace', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x01b72940ad5a7029454200352b5259d44a27806c796618a1454212d20e92495eab93062f1795d651deebb0f64251a5c5c8d5679bdc1b4cd14846f10f1998249c9b', 'a239cad07fd5f13741c69bb6ed584d6a', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 82, '2026-11-17T04:56:45.685804881Z', 'sweep', 'purge', '\x1de9b47eafd522a3e79bee94c607e9595b4c1d0a5225f58fec4ce1cefe48ade7', '\x01b87229e4c369a425319bbb578189ce0bbdfc5bd9a29714ba67818bb8a392a01cbd91adc86980e35414a896f4d9ddce4efbe888cffe97480b66b7c17b4a54f1ab', '32d0616fe88428a6f5889d2cd4d23465', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 84, '2026-11-17T04:56:46.032933804Z', '684b7422acffaf56', 'legal_versions', '\xe7723b2e642e52eabf08f30ead6c8601a4856c4da71a0434afddc3355ca684f7', '\x017bb78d238be0130ec879e6a3895e89b1111829a028d2e22e2539fa2c4ce6465f5c91a9ab21c616f7b0e8ebe83c10c1cac9590ee99a31510f3a416ab230fc909d', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 86, '2026-11-17T04:56:46.042596848Z', '684b7422acffaf56', 'legal_read', '\xe7723b2e642e52eabf08f30ead6c8601a4856c4da71a0434afddc3355ca684f7', '\x0156f93919b24182a0f97ccabc212d7218c855c2b705b4be001e0d8b77638886aae1557ffd818fdc11d1d569697014b75bc47dcecaffc504772b889aea34e46ace', 'fc44a51228f85f57e9fdb326dc1ba4b2', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 88, '2026-11-17T04:56:46.051459809Z', '684b7422acffaf56', 'legal_read', '\xe7723b2e642e52eabf08f30ead6c8601a4856c4da71a0434afddc3355ca684f7', '\x0102a05ad70dcc039e16843024ca4d7937b4d9235378f36e0334390da0df384abeb5551d68d77811ce11ba174f6add81b65580e284eeda8b01eb3541e5214f9484', '3aa660bdf2c45f64e29343b429380b01', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 90, '2026-11-17T04:56:46.058171909Z', '684b7422acffaf56', 'legal_versions', '\x9ae45095b7745dacb6446255dab4eb183960e5d3cd5cebe0ef44f7a7b5bc85af', '\x01892f03e951220ae207a703d4f2c2806f6fd3f3c945a8b8314dc78a8c150373c71ad7ab1470768afec8926f57aa11f06b1cbc73a94ee8bb1920069b647d064298', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 92, '2026-11-17T04:56:46.06535937Z', '684b7422acffaf56', 'legal_read', '\x9ae45095b7745dacb6446255dab4eb183960e5d3cd5cebe0ef44f7a7b5bc85af', '\x01373f6fc40f820271404856dd25d361cd5267eeeaf6ab946cbaba4e5423425eae9eabf8ad08b54cc0baed247bad244fe327f4c447c751c7be140a0638d5ce53a6', '968c3999d1f5af0d5962cdd8a6f2e80f', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 94, '2026-11-17T04:56:46.074398655Z', '684b7422acffaf56', 'legal_read', '\x9ae45095b7745dacb6446255dab4eb183960e5d3cd5cebe0ef44f7a7b5bc85af', '\x0170fe8b2662bce312316059437a6b5f856af990360be9217f20c67919cac6fc6f499d27fc26d9dd4677ce456061a8548a4ec782d70844a431617cb5455efe5d07', '2697919c744a9b036b34f3f47c4a9857', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 96, '2026-11-17T04:56:46.080527838Z', '684b7422acffaf56', 'legal_versions', '\x7ff0ca73a5a3947b7638308d4f371ff43d2af964d77ecb7d7f9c90e8d0449397', '\x01e6d9a33d32430f7405a902e9c43b6b90a957d96408dd4b178f9d3809855590d58c071726993d87dccf5f66fe8a480b91f88b9ab50c6b1c00fa3d617e27510ed7', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 98, '2026-11-17T04:56:46.088551031Z', '684b7422acffaf56', 'legal_read', '\x7ff0ca73a5a3947b7638308d4f371ff43d2af964d77ecb7d7f9c90e8d0449397', '\x01439f58b5921dd16e9c9cc1012860d881ba8407446f015a06a1e3dafa020da039d728c81b5e7e5be07726579381091512f2e435632eaf4a799ca98aad021b167f', '1db03c12259b4435d7f66ec0f3cb22b5', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 100, '2026-11-17T04:56:46.095359178Z', '684b7422acffaf56', 'legal_versions', '\x597273e99ab10b6aef5bfea5cd1ce04f61076535851bb9c6cdee27876b4d5b50', '\x0125cbb04a4d9cf6a94fb3dd325b7db329cf19a142e6702477871dacde283e6f09b007340b0ce9a429c4a4a8f579f8afdab66a503609a20ff41e0c0f265d515eec', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 102, '2026-11-17T04:56:46.103325333Z', '684b7422acffaf56', 'legal_read', '\x597273e99ab10b6aef5bfea5cd1ce04f61076535851bb9c6cdee27876b4d5b50', '\x0127b15f7e0a60481fb569375f0bb317aebb26058598a49ab0275ef21ee498a602bba3ff120fe93d0c50b0287c607dee84f5917c4eedde0d8061e66cb624898c62', 'e93afe67f5420586054545958de66f44', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 104, '2026-11-17T04:56:46.108654646Z', '684b7422acffaf56', 'legal_versions', '\x6f60c1ee716b3ffae0b3a9245f9fa7d97e75342c2053afbd500c025e8f88db50', '\x017ea1c78a65b832f570888a2bb1e569cd28ec3666cd5bcb43777d316fb4ca059196b196a2444e5c4e54656a32c6f8e2948e6ad8b4fa115ab6c706f4ec50c586be', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 106, '2026-11-17T04:56:46.115523589Z', '684b7422acffaf56', 'legal_read', '\x6f60c1ee716b3ffae0b3a9245f9fa7d97e75342c2053afbd500c025e8f88db50', '\x0191e166b0716c79170bcb70907fdc5f47a6c10031f788f3632844171092083cb63223535d921c0d06674a846ba840a53426d739b9edc16471907363ff4e5b0c24', '0b7b6661b68656d19936506eb549c404', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 108, '2026-11-17T04:56:46.120807346Z', '684b7422acffaf56', 'legal_versions', '\xb881c356013124d7bd9e9efae28c9588e933b4f8e2d10f0df0a579b0d38a8182', '\x011da9326e9841e7cd0cf7a98106b5c7df473026eed76ec4bb30dc5f409681a34a2e5fea0e560d4763631e3b05eedfaf0d97f95f18d6f00e737deea6b476001962', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 110, '2026-11-17T04:56:46.129007929Z', '684b7422acffaf56', 'legal_read', '\xb881c356013124d7bd9e9efae28c9588e933b4f8e2d10f0df0a579b0d38a8182', '\x01520c5b9ecd551ad7dc2078d662f05c408734863a9292fe7de23e065b51d44d8213fe1c65d8ff3bfd1a7449169eeab157b6486a99bf7ce7107bd4ab225463ded8', '141b1dc3bd78ea147adc904883e31b9f', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 112, '2026-11-17T04:56:46.13359726Z', '684b7422acffaf56', 'legal_versions', '\xb0af369dd1783bd3dd2742318008a71384a5eebd3a868c59ed437cfdf48888f3', '\x01bd5811edb7db7d31540c61df15bde7806041615e0c00c8b68f77df3a8c891a91f7dabc8805eff08c1936a88fb8b63ceefa07804e89d478942c6053e6bebd60e9', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 114, '2026-11-17T04:56:46.140022435Z', '684b7422acffaf56', 'legal_read', '\xb0af369dd1783bd3dd2742318008a71384a5eebd3a868c59ed437cfdf48888f3', '\x0143099b3ca42a2d66852215bd6c4ddb0a52c47b2ddcc437bc4328d389aa1ba245cedb6901aec0a5d6957623f9bbf3b0c1f1920b4f61dfcaa9eed9b7ddae186bd8', 'aeda5ddc09ad8529461326fed7bf8e65', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 116, '2026-11-17T04:56:46.14641222Z', '684b7422acffaf56', 'legal_versions', '\x7b69ecf02f567ca6853649a711a889c618f7ba88712c5fe7ae8f4d10d6fb15a5', '\x01d7f7ca2602d22005028bd053a1e6d62a3c3bf46ad1831a03193e51c136e8b26da5684d4da3c57e6aa9f580ae431738fe4536985205195982631c675dc9f36669', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 118, '2026-11-17T04:56:46.153113126Z', '684b7422acffaf56', 'legal_read', '\x7b69ecf02f567ca6853649a711a889c618f7ba88712c5fe7ae8f4d10d6fb15a5', '\x012fac888c7f4335cfca39cbab2be12e8dc3f5bf07fd8e19c0abc7ec5792d0909d921fd7ef7135f823472ff5be8ebc7a2aee83969431c2ab91fb2afff1a4c42699', '5a2226c388b2dd894f58966dcdab0707', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 120, '2026-11-17T04:56:46.159168254Z', '684b7422acffaf56', 'legal_versions', '\x1cae3c109bf6651bf57a1b276ac3aafe145b743d5e439d9557815f6fb3a72b21', '\x01911eec94e16b697c27c3e64a2e99e58af521d661a4ac917fea760abcf35b32f2ab2fdcb4402f663a2ddd7778967309dd3e71654e9eebc356283267d4efa95c84', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 122, '2026-11-17T04:56:46.166530864Z', '684b7422acffaf56', 'legal_read', '\x1cae3c109bf6651bf57a1b276ac3aafe145b743d5e439d9557815f6fb3a72b21', '\x01cc1c57be030423fb522caa3c35ef10af555a10b5c1f2e8e8ad76e7e32e1b11e3112a40636c4d7b516869efafb703d289b7cbb177cb7006ebcf071e0db2094830', 'fd7af32ef454cf4a88d887144fb5b014', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 124, '2026-11-17T04:56:46.171909576Z', '684b7422acffaf56', 'legal_versions', '\x032a66a7517bcff83e98073a0607e75da53ec5eaeedee7cd13765b97afc3cbe6', '\x01cf0a7058185c7d61050bd551347c0781b5f85120407e15b726d3f651287c04319bd9aa69817423ec30f6c538c83adab20475efc8655167efa78da16dcf5b00c4', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 126, '2026-11-17T04:56:46.179366481Z', '684b7422acffaf56', 'legal_read', '\x032a66a7517bcff83e98073a0607e75da53ec5eaeedee7cd13765b97afc3cbe6', '\x01131db450baf927e34e3f153f316921cf395537bcac5311a9b79c3c99252019959971e777a1895ffda555d4f43cd54e0d6a6b967bf5ad2071f4d9083a909345c3', 'cb9a74eccaeaf6a99cb53562c456d361', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 128, '2026-11-17T04:56:46.184827953Z', 'ae9016800219ca90', 'legal_versions', '\x1de9b47eafd522a3e79bee94c607e9595b4c1d0a5225f58fec4ce1cefe48ade7', '\x010f864aa21801cd2a58d716ac7657395f44ee94fda70c49b00b6f6f246475b4dcbc7c29c34ea28dbc71a3694e1806b69dfd9bc8d7e68b8b717c1d4fa39875d9e5', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 130, '2026-11-17T04:56:46.192160284Z', 'ae9016800219ca90', 'legal_read', '\x1de9b47eafd522a3e79bee94c607e9595b4c1d0a5225f58fec4ce1cefe48ade7', '\x013bd2bc3a5e63f5f63f0160fd0d5aef94df6be2822a4bf6efe162ec4756f0bd6dd3c9b7e7c7e878d6038d0978d2411c00dcd2a5f0bb182bddb3fe03967c3d2b99', '90739613d619cb2038cb93a46c72620a', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 132, '2026-11-17T04:56:46.197609969Z', 'ae9016800219ca90', 'legal_versions', '\xe9b7b3475e08915bc991db15ffb5249ba29665e6d4178975e810f5830c240a10', '\x01d13a4a4def27bba3915fa62e222bc532b2ef897ad618d65606a45e103695009ea62fe72b33833f851ac543819f01cb55ae6e484c3ecc5fc910b85f0b744ff91f', NULL, 410, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 134, '2026-11-17T04:56:46.203025967Z', 'ae9016800219ca90', 'legal_versions', '\x2adc4230bfe749508d289c52f1874b52168b80cb9ce73ed15fddf0c03bcc2bad', '\x0161fb3e7ce298de257f0fc8ef124fe092f636b15c63700a0c6678535c86d00cfbccb258872e806c9f448213e0b7f532dcf58ef31d4ea0fafe5a3565cb8d63ca1c', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 136, '2026-11-17T04:56:46.210859179Z', 'ae9016800219ca90', 'legal_read', '\x2adc4230bfe749508d289c52f1874b52168b80cb9ce73ed15fddf0c03bcc2bad', '\x01653065ff0190501998244f12f496cc8cc9556f9e657247cbc7ac80b14aa1bf9203f654ba95c90c9bf03da15f58e53da5c4bd0c74aba21425844697c21e1c0981', '25fc6a98105ed8e96df42f43cc490662', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 138, '2026-11-17T04:56:46.220318861Z', 'ae9016800219ca90', 'legal_versions', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x01f3354022c060189590f7f15791208f79945f31136b0c27d623c7d2833dc4ce3521e4c220d76a8fbfaaa2857468e65fd809780eacc3b52b2c8d9ffc434dc9adc1', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 140, '2026-11-17T04:56:46.230599656Z', 'ae9016800219ca90', 'legal_read', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x012ec1327d06277f959e53c1afe5ba2a83838e9466151482db86e142bc89bd3347945a11e404d65dee01435127200721211aec9e23a8c74e3fddc7a0f308cb113a', 'e303e974c474e0397c9e1ccdecd2d8dc', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 142, '2026-11-17T04:56:46.241766088Z', 'ae9016800219ca90', 'legal_read', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x014e36bbf8f3573cb84d9746492c81fd705e2f328033ebac2ddd9215a64d1623e1ff1a3cf2a16f7d09328b3f24d350945c3008bc97621dcac4afe0f16dd72383cf', 'a239cad07fd5f13741c69bb6ed584d6a', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 144, '2026-11-17T04:56:46.2497963Z', 'ae9016800219ca90', 'legal_versions', '\x58068f2633f20539e0a44b6f8d6f521e02c78a3e4375a0b6ebdda4defbf2984b', '\x01a2da1870352997ae2ed8e4b089e38bf84f38946ef0da3cf97cbc809d501356adac4026cd4c6f6194276b73e578ed3e1594f94fdb09acaf8093d98e49168edfc8', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 146, '2026-11-17T04:56:46.258642992Z', 'ae9016800219ca90', 'legal_read', '\x58068f2633f20539e0a44b6f8d6f521e02c78a3e4375a0b6ebdda4defbf2984b', '\x014b103b503c557a9d8d2b06f297d58ce6abec64c1ae807a5ab22d3e3b914d785949494b043a07abf2b85460c943c21d589e2ee5e9d02dce16de15f906fefeedfb', '4c91522031c9cbf757befecccb8c4ee2', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 148, '2026-11-17T04:56:46.266434889Z', 'ae9016800219ca90', 'legal_versions', '\x552c8de8b5f43d91c4b3c41e473943b0a4df28afd13bbb12a5d5bf3cf5913188', '\x01f8cae39318b60fbad051304962f36c07a37df8c8942a2a921c2fcc755c533636aeb41affbe4f9ec52d142258eb88ff7dd2ba4c4b1b6a39f32b0bc312898acbac', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 150, '2026-11-17T04:56:46.274374359Z', 'ae9016800219ca90', 'legal_read', '\x552c8de8b5f43d91c4b3c41e473943b0a4df28afd13bbb12a5d5bf3cf5913188', '\x01e0a8332c215d4e6bfa77569817f55e3cbb23b987e3cc05167c7cd5f8077417c89fcab042038fdee96f54cfb8f4b3c972d958e660c1a4c251828d672767b9af83', '1ffbc38a5175bc800d8666776af9b19a', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 152, '2026-11-17T04:56:46.280099738Z', 'ae9016800219ca90', 'legal_versions', '\x67b1ddd21bb4f1151659eed710e6887c70d78724b41f24f0dd7266f6ff283251', '\x011592053c63772d4a26fd179ce4cf6ed547f2922180cfbd86f24812d5c9e1bb48268155a668623020c1fdb37883d3192c6c40a2265dbf40b5a03ca2f6a17bd6f5', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 154, '2026-11-17T04:56:46.288868755Z', 'ae9016800219ca90', 'legal_read', '\x67b1ddd21bb4f1151659eed710e6887c70d78724b41f24f0dd7266f6ff283251', '\x019e2e9052879156878be22724efa644268977c67effe7a3fd0d3e83ae58570bbccfa6891a86fc6e66eba3361c75cf6cf7b7c5eb3f4d0a292a0cf86cc7221a1744', '770b16fcf4dbacbecfbfa43ef12f6be2', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 156, '2026-11-17T04:56:46.296776818Z', 'ae9016800219ca90', 'legal_versions', '\xbd7a7530726b5f06ebaa88c7e55a0c5065398d0b4273c98c1419e28ae2a1a306', '\x0159a9b03792ac75eee7a0409c4000335c5040501331934e544ea2c6d7fea900b4ddd89d10c4feca55dd7aea0a4523635402b61bce61c2118af9a4969e4fa16b8a', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 158, '2026-11-17T04:56:46.304067862Z', 'ae9016800219ca90', 'legal_read', '\xbd7a7530726b5f06ebaa88c7e55a0c5065398d0b4273c98c1419e28ae2a1a306', '\x01ab11d7bb9cf1a83f21f09286b096cd4c4967a410b6937a41cb84df5731ee209faeb0efc2e4cc94c83151f5aae70859f72ff92f8ccb0a09d429cb039f2b28e86a', 'b1c4835a7e58f8b8fa26c7361e052f46', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 160, '2026-11-17T04:56:46.310407806Z', 'ae9016800219ca90', 'legal_versions', '\x5ef75e0c4a1399332ec6f590399832651402377b9cb7731534421af64c1c102b', '\x013f012837bc6aa7371b1b03c6560e0615b9acb073f779e9b2314cdc69fc0d5a4b793bc64e2b85ed8201aef6417d63c5e3fb7a36b7e9ba83d633bff1f3d81f174c', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 162, '2026-11-17T04:56:46.316826711Z', 'ae9016800219ca90', 'legal_read', '\x5ef75e0c4a1399332ec6f590399832651402377b9cb7731534421af64c1c102b', '\x01c7faf9ee24331dffd1abb701d10f27529561d529d47b0a294f4370023a2154a2d71cda9ab55cc1b3a8d84bf03bda61be0416ba0b726b12e1b6c56db9092629bc', '99300f9c7835036b92b37c4f5908ca8f', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 164, '2026-11-17T04:56:46.321130025Z', 'ae9016800219ca90', 'legal_versions', '\x6e205bc901fa69d2201a676dc15f81a14549e3b0376b76f3252101df9a88d6aa', '\x01ae40b2c5f47a2bfdaa876abbd3cb37722b27972a273105be72a12e051b1ec47f9bf0ea1cd21dc596a3870b95bd86cd09b8064fb42b63d6c7a2c61ef302e4a0d9', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 166, '2026-11-17T04:56:46.327630598Z', 'ae9016800219ca90', 'legal_read', '\x6e205bc901fa69d2201a676dc15f81a14549e3b0376b76f3252101df9a88d6aa', '\x0157cce8ecb7f29669eeab53acd3e1cadd4d50ac6e113005725cc7f5eaef1cef54d0627b6b3d58f05a44fa6ab44ea735a4f9b22cba4d09f0acc1ec52486f78550d', '23e87f5b9d57db41f0af4de157349525', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 168, '2026-11-17T04:56:49.996950311Z', '684b7422acffaf56', 'create', '\x7b49e2da99a4e33b79887947825deb11a8cd68d8bd2cc07b5a2752f2d3dab2fe', '\x0166fab8743f44f42611a559cdcb2eea061d89d725642cd6da6bfabb4b0017f11906ea87beba8227c3724923678ec25bc6b2a4b82fb1a29ee83ce88740c116032f', '9d06eeff0bbd220b6e037da88138bcb1', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 170, '2026-11-17T04:56:50.013693446Z', '684b7422acffaf56', 'create', '\xee80db6b16cc5711d485b163f62c0d7db4dfe26cd0e82232eb3a158cfb7fe6f3', '\x0144e2d9d337c632267308022d85765fcb57d051d627881753eb9c409d264dd513e66cc78f9038776011b524ecb58b9d492c5fd1c363a176e0f5342675619f7955', 'dc1405519d9414d1d705bcf1c1dbd122', 201, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 172, '2026-11-17T04:56:50.026978498Z', 'ae9016800219ca90', 'create', '\x909688821211ae0add1b9699726b6aa854112038b84efac2bde416d1d635bbfc', '\x018012bed2112c85fc13126bed7a1132f19ac3d4c9dda880bba17965b51a1f2794856b0ed389143d21c4b9c9bbe71a09e80bec98af46ad991f5b3607904413fed7', 'a4a21fb3468515cf3f20d63c74ad37bb', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 174, '2026-11-17T04:56:50.040601078Z', 'ae9016800219ca90', 'create', '\x54d82288fd5ca02b8fcadab5a1fef4db3a192e5cb08f3426203744052549500c', '\x0171777013e93ec59e76e7cf6f1d611915de008e156e70ff1dbe45542eead9e0d93c81a040e6a34df18ec3d09825ff4ddf89ab5875bcf8dde3a728031fafbaf572', 'daf2f73011ce4d1c968e47ddfff89d45', 201, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 176, '2026-11-17T04:56:50.045860873Z', '684b7422acffaf56', 'legal_versions', '\xe7723b2e642e52eabf08f30ead6c8601a4856c4da71a0434afddc3355ca684f7', '\x01bda87c3ee4d424d1f99cab61d9b566dd509097aae6a3f6216d16aab3781226be00d9073e0497293779cf9f0c066bcf81af3a18537b3ad325bcd0ea505383b6a4', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 178, '2026-11-17T04:56:50.055307185Z', '684b7422acffaf56', 'legal_read', '\xe7723b2e642e52eabf08f30ead6c8601a4856c4da71a0434afddc3355ca684f7', '\x017a9305aaa79e3f935116669177e8603f4ad924a5b6cdfd77b8beb6bdcafc38547180513560668c1af73fa3171d3a8f3b350534b89749beaee0b6d2684758f0fc', 'fc44a51228f85f57e9fdb326dc1ba4b2', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 180, '2026-11-17T04:56:50.063308711Z', '684b7422acffaf56', 'legal_read', '\xe7723b2e642e52eabf08f30ead6c8601a4856c4da71a0434afddc3355ca684f7', '\x01391c07ce27fe087c1a954b9e191fd8c6592c541cd554889ee84e0497ba323531a9d5792ee34a35ecba1b06620d7b3b25e9068b8f1068fd3d9ad86c8e4da4312d', '3aa660bdf2c45f64e29343b429380b01', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 182, '2026-11-17T04:56:50.069032858Z', '684b7422acffaf56', 'legal_versions', '\x9ae45095b7745dacb6446255dab4eb183960e5d3cd5cebe0ef44f7a7b5bc85af', '\x01651a6c3879c5471bf34ad1654c499691dc3a97e936bb8094ab4eb1cf0dfdf808293ac96e974169992403fe7db3eedabf6f043ab5d3a59a6946b28a04c17115b1', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 184, '2026-11-17T04:56:50.07631855Z', '684b7422acffaf56', 'legal_read', '\x9ae45095b7745dacb6446255dab4eb183960e5d3cd5cebe0ef44f7a7b5bc85af', '\x0168dfb16140d5fbb101ee6da67f75124e14cd8f37bf57986689ee9e3f74c69a6c155cdbe33083f072a64092e40d07f12ae6346a4ce38f19492d6e3cf75b9ab4fc', '968c3999d1f5af0d5962cdd8a6f2e80f', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 186, '2026-11-17T04:56:50.083618511Z', '684b7422acffaf56', 'legal_read', '\x9ae45095b7745dacb6446255dab4eb183960e5d3cd5cebe0ef44f7a7b5bc85af', '\x01c5b292eefc0b1e5eb6bd16a05b49dcc3cf37692d4a23b2b361bc9c599e7097fd3a141d887b02394981e84de55c4a15ad7f4b777e03e4fb24fb72714640b15fbe', '2697919c744a9b036b34f3f47c4a9857', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 188, '2026-11-17T04:56:50.088454417Z', '684b7422acffaf56', 'legal_versions', '\x7ff0ca73a5a3947b7638308d4f371ff43d2af964d77ecb7d7f9c90e8d0449397', '\x0192f3225f544edfd4021f9eb8c34226059d4f83baa327da4c86fca77e616c455507713e81e4d96dfdc80a4fe9b73eaffe9b7c1074b3d613eed054e55ec8959342', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 190, '2026-11-17T04:56:50.09517048Z', '684b7422acffaf56', 'legal_read', '\x7ff0ca73a5a3947b7638308d4f371ff43d2af964d77ecb7d7f9c90e8d0449397', '\x015a201135e88d2e20d65ccfc14731d2c42acf49c288ab858f74799a3e2beb636ac7ef90f484541c1f3f1088c18a6a61b8fbcefbbd6181b5d1370cc1bba88eff28', '1db03c12259b4435d7f66ec0f3cb22b5', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 192, '2026-11-17T04:56:50.10103234Z', '684b7422acffaf56', 'legal_versions', '\x597273e99ab10b6aef5bfea5cd1ce04f61076535851bb9c6cdee27876b4d5b50', '\x01c8ce54727482ee69cc543c01385df009711b2774ece69f84fa97f1161d38f1cf9e926b40552677fdfc63ec5feda5bd30b19f7524807b9342ad38aa2eb62e607c', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 194, '2026-11-17T04:56:50.107489551Z', '684b7422acffaf56', 'legal_read', '\x597273e99ab10b6aef5bfea5cd1ce04f61076535851bb9c6cdee27876b4d5b50', '\x01d70a9214bea1c8914a36d7a9b29c63e9d2c43da158083d309356e9327d7729ab0045c2229d0fd1755576e6ef385dd28e2bd706b98692da53c17dbd9218fb4781', 'e93afe67f5420586054545958de66f44', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 196, '2026-11-17T04:56:50.113115031Z', '684b7422acffaf56', 'legal_versions', '\x6f60c1ee716b3ffae0b3a9245f9fa7d97e75342c2053afbd500c025e8f88db50', '\x013ac28d60e8aa973fb842cc9996ede176a5e1e986185083be70ffd5d463d03db031caf232ee1a61b6bbeb9725cf1c9f1279810bfd64e5c198164aefc183af59ff', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 198, '2026-11-17T04:56:50.12072014Z', '684b7422acffaf56', 'legal_read', '\x6f60c1ee716b3ffae0b3a9245f9fa7d97e75342c2053afbd500c025e8f88db50', '\x013692ab7b7d85aa546067328a5bf05eb9db79a95ea5c6d7304935b35c51b4bbc7ae4376df1bfad7fbbbf6b5a055831917b88388db4607a61ffeff0238361d4a8f', '0b7b6661b68656d19936506eb549c404', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 200, '2026-11-17T04:56:50.126067584Z', '684b7422acffaf56', 'legal_versions', '\xb881c356013124d7bd9e9efae28c9588e933b4f8e2d10f0df0a579b0d38a8182', '\x0118e5737beee6eda6662057e525bce7fb79ae0b4f44e189312a8a705a7cd1243dfdad775ede85ad46ccb277f00022783dc02078f657c4372f3f51e2701f7c9313', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 202, '2026-11-17T04:56:50.132574693Z', '684b7422acffaf56', 'legal_read', '\xb881c356013124d7bd9e9efae28c9588e933b4f8e2d10f0df0a579b0d38a8182', '\x015c8af7c90398a31f42d5c87ecee1a74882f19a921e2a3533334421bb6e04f940f9312d8d70b36d6a5b412a2930bb07a416cf7fabf15aaa80b82c864f8e028d8a', '141b1dc3bd78ea147adc904883e31b9f', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 204, '2026-11-17T04:56:50.137946618Z', '684b7422acffaf56', 'legal_versions', '\xb0af369dd1783bd3dd2742318008a71384a5eebd3a868c59ed437cfdf48888f3', '\x013286acdd8bd72b561b960ff5a87daa82a9f99ef0df1b66334941b099d5f5225409ac5dc2ea63296fe7139bace306d9fd25893c57f512776c6eee7021cfdc6155', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 206, '2026-11-17T04:56:50.143803245Z', '684b7422acffaf56', 'legal_read', '\xb0af369dd1783bd3dd2742318008a71384a5eebd3a868c59ed437cfdf48888f3', '\x01f554649cea1b7e65e6031114d5dcfdc6b20733c82744a163da3d58d1ad5daf9ea1d2118b9f9046ea0f30c3ebe52f88494c8edae7754cae74053cd8f2ee56cfe3', 'aeda5ddc09ad8529461326fed7bf8e65', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 208, '2026-11-17T04:56:50.149131087Z', '684b7422acffaf56', 'legal_versions', '\x7b69ecf02f567ca6853649a711a889c618f7ba88712c5fe7ae8f4d10d6fb15a5', '\x0190329d3b1ecaab21f3f1cf5e7962e3a6e0d6897257961823dbc88ed83e1f5a962e25f4bfb837303bcf23f9c9fee8dc4c5ba002f338dea943f12f95856a00f2e0', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 210, '2026-11-17T04:56:50.156221927Z', '684b7422acffaf56', 'legal_read', '\x7b69ecf02f567ca6853649a711a889c618f7ba88712c5fe7ae8f4d10d6fb15a5', '\x013e9684bd34708f21b9e005d7994c609fe4997a6ac35dbef8b6f5d0bdc06972e998b4651cbbf7e5715b84f3a46c8ccac7bd735e05114def23d066ba67cf9ec181', '5a2226c388b2dd894f58966dcdab0707', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 212, '2026-11-17T04:56:50.16219188Z', '684b7422acffaf56', 'legal_versions', '\x1cae3c109bf6651bf57a1b276ac3aafe145b743d5e439d9557815f6fb3a72b21', '\x0124e840466a998c5ddabfa979831fc1733b797592e7ca9c2cf17880b7084c2ada3775247db020f974bcd3de2120091bdf19309ef5c9eb991b0bf5e5ea817444c8', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 214, '2026-11-17T04:56:50.169116906Z', '684b7422acffaf56', 'legal_read', '\x1cae3c109bf6651bf57a1b276ac3aafe145b743d5e439d9557815f6fb3a72b21', '\x018ae64abbb4dd0a69b7b56f36c696614aab28d9834de4906a5e6e7ac233a47299e24eb6f3d7c03e7402610d9c3e0aa8fe1689fffcfbf484d301badd011331fee6', 'fd7af32ef454cf4a88d887144fb5b014', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 216, '2026-11-17T04:56:50.173831543Z', '684b7422acffaf56', 'legal_versions', '\x032a66a7517bcff83e98073a0607e75da53ec5eaeedee7cd13765b97afc3cbe6', '\x01cc6a0d790996d736dc803fefca5cfcfe36ff03379e7b43763326a700b2dcc1e57fa8a88a68be1b9e268cc7049f2eb1a92766c469ca82a3fc5c505f120bd7c261', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 218, '2026-11-17T04:56:50.180849347Z', '684b7422acffaf56', 'legal_read', '\x032a66a7517bcff83e98073a0607e75da53ec5eaeedee7cd13765b97afc3cbe6', '\x014fb4820f54c27da499d6a17036e1acf0c38a1d53639ad98ad119cf3214df93366cab2d88db9d89da0bc34423e6017a11ab5af7d5d3a0c2dd55202d1e726e0b63', 'cb9a74eccaeaf6a99cb53562c456d361', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 220, '2026-11-17T04:56:50.186413022Z', '684b7422acffaf56', 'legal_versions', '\x7b49e2da99a4e33b79887947825deb11a8cd68d8bd2cc07b5a2752f2d3dab2fe', '\x010b3d1ab4b4e90a6ee79dd1ebadca022fbf0d0dcd94882b3a0e0ebff05de93b46e3ef8fde72ed1a90867359bb3ad7d03d93af76ea7e3ef0658f315b822ce00430', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 222, '2026-11-17T04:56:50.193345057Z', '684b7422acffaf56', 'legal_read', '\x7b49e2da99a4e33b79887947825deb11a8cd68d8bd2cc07b5a2752f2d3dab2fe', '\x0194985eaeb7f969240f32a9c9c3688b2fec9f227e60eca22f0a7e96d5f3a42435653eabadf06a7ec2e3144098b69117663658c52c8d391f3c92e0de95aaf95c82', '9d06eeff0bbd220b6e037da88138bcb1', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 224, '2026-11-17T04:56:50.1982055Z', '684b7422acffaf56', 'legal_versions', '\xee80db6b16cc5711d485b163f62c0d7db4dfe26cd0e82232eb3a158cfb7fe6f3', '\x017bcfc98e7168812d55c1522f87fb89d123f4969450d9aef2770fa2cf8572f51e9b07f740325dad219eb9daa47aeb457b5ebd31076b7b472d6c5599f5b29d2acd', NULL, 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 226, '2026-11-17T04:56:50.205464701Z', '684b7422acffaf56', 'legal_read', '\xee80db6b16cc5711d485b163f62c0d7db4dfe26cd0e82232eb3a158cfb7fe6f3', '\x01b114c56e630fe0323739292571362775f17da8bfe6f1f75f4eec538487bb552b9e5e91d4b248783a329f374ba1d2cfb46fa64f8c304bc27b1feaf2c4bf607a67', 'dc1405519d9414d1d705bcf1c1dbd122', 200, 1, false, '\x5fabb96585857481');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 228, '2026-11-17T04:56:50.210187802Z', 'ae9016800219ca90', 'legal_versions', '\x1de9b47eafd522a3e79bee94c607e9595b4c1d0a5225f58fec4ce1cefe48ade7', '\x01b6c087ee23aae4b0a56b94edf17aa5251afd17db08ce6e0940b62bb7bd1b943b04e87f0d87fefdab2a0709a298c686f59824a2ee9a556583a52ebcc8f68414b1', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 230, '2026-11-17T04:56:50.218306208Z', 'ae9016800219ca90', 'legal_read', '\x1de9b47eafd522a3e79bee94c607e9595b4c1d0a5225f58fec4ce1cefe48ade7', '\x01299537c35746a3d3bf062dec9f0d4af516c609db54206ddd09bd0319a949902f41bcee256c5a4f2028fce20e7d1be54e17ad414925a3cbcd55aa649425703f3b', '90739613d619cb2038cb93a46c72620a', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 232, '2026-11-17T04:56:50.222845151Z', 'ae9016800219ca90', 'legal_versions', '\xe9b7b3475e08915bc991db15ffb5249ba29665e6d4178975e810f5830c240a10', '\x011bbd9e6185c9a17d9b524e0c1816d4c1139dcc42b5ff20c9d2bfb691b083584aaca15f0413a236d28c3465b089937bf6417dd37b14fcb4c88d7fb6e3bca78d51', NULL, 410, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 234, '2026-11-17T04:56:50.228389565Z', 'ae9016800219ca90', 'legal_versions', '\x2adc4230bfe749508d289c52f1874b52168b80cb9ce73ed15fddf0c03bcc2bad', '\x011d76a506351ff5a3f345f4845dd941c46b8588e18e5b04a644994cd8b66ecb2fe7f1303fa594f3b0a32b3aa3f09e612d0249167caabb46c897ed3c757006f105', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 236, '2026-11-17T04:56:50.235978009Z', 'ae9016800219ca90', 'legal_read', '\x2adc4230bfe749508d289c52f1874b52168b80cb9ce73ed15fddf0c03bcc2bad', '\x01be56ceb93e933feb5c585f52fda1c0920ef5b78a02ae4ae3f4b21d76c356308e0517ca09fc964f820ef430ec7963963e86f69c1a62e268b338baa136c90c132d', '25fc6a98105ed8e96df42f43cc490662', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 238, '2026-11-17T04:56:50.241728525Z', 'ae9016800219ca90', 'legal_versions', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x011ca7082ed582e8b1757173d090283162b96f0c07300c3d6be0e26ea5bcd04a9b4d0303915117aeafde234c74c87a4cb64ef73a8a773c15d01c7f01ddd87b21f1', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 240, '2026-11-17T04:56:50.249003495Z', 'ae9016800219ca90', 'legal_read', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x01f553541fa1e0dbfd5cc178be23d7beb4679195fba2adc41ab8dc9c57c9e023c351b0e04e81ec328bb9f9e59e030dae29ba17aae95aa927da4bd9c405c7d452bf', 'e303e974c474e0397c9e1ccdecd2d8dc', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 242, '2026-11-17T04:56:50.255087263Z', 'ae9016800219ca90', 'legal_read', '\xf3c939f29a627d69cd20ef8535902eb374d356557e770129cdd3b3a69dd6a5b6', '\x0152a39d18c221f06d66d451e9e0032ff96d9760322ff27e4a768e9f0c27c5cb50a1dfc9ed508843d9b7268a7948c95ad5b80b6d6d891b0d55a48d93f28f8081db', 'a239cad07fd5f13741c69bb6ed584d6a', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 244, '2026-11-17T04:56:50.260019734Z', 'ae9016800219ca90', 'legal_versions', '\x58068f2633f20539e0a44b6f8d6f521e02c78a3e4375a0b6ebdda4defbf2984b', '\x019f5b3ee778e662835a299a0d085303d1f5f708e240a9a928c1767f28bb12f0e6b4b64f367a0d9eb4317ddda404ef2e81492135da617e49a030595c2ba8421c6d', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 246, '2026-11-17T04:56:50.266191778Z', 'ae9016800219ca90', 'legal_read', '\x58068f2633f20539e0a44b6f8d6f521e02c78a3e4375a0b6ebdda4defbf2984b', '\x01f641497b8330397cb32ef7dd766afca31f6e7eb6581088e763dd7df02c2f9aaecda99c854ad87f21d638c149e482407a99003b6d0f06332c38684c2e4cb99650', '4c91522031c9cbf757befecccb8c4ee2', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 248, '2026-11-17T04:56:50.271697926Z', 'ae9016800219ca90', 'legal_versions', '\x552c8de8b5f43d91c4b3c41e473943b0a4df28afd13bbb12a5d5bf3cf5913188', '\x0139ee2dab5dedfbe1a5a0df9ad9c987a72096bd46d05b36ac17f17f943277de480ef1a43315b63e0a5c9a4ca389ffb2ebb774ba1ee1c8ad22913e1544234ee9f3', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 250, '2026-11-17T04:56:50.27836785Z', 'ae9016800219ca90', 'legal_read', '\x552c8de8b5f43d91c4b3c41e473943b0a4df28afd13bbb12a5d5bf3cf5913188', '\x01df39f6d3c34501ed96ab90b2c4a299757c8a2e0413165e8fa6910f5b6416ff0a9b080144c68632245b4595cb967a08f1b7b93773e6bead2d45162cd1c338294a', '1ffbc38a5175bc800d8666776af9b19a', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 252, '2026-11-17T04:56:50.284773366Z', 'ae9016800219ca90', 'legal_versions', '\x67b1ddd21bb4f1151659eed710e6887c70d78724b41f24f0dd7266f6ff283251', '\x012c16fa18ea5f1ddef7e5127e2033ab62525b9eef81d9491f94f431d6a3a04e475994a0c8223f2c4241480775b9ce583ecb816b627576e4d05513f8236bedcdf6', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 254, '2026-11-17T04:56:50.292554968Z', 'ae9016800219ca90', 'legal_read', '\x67b1ddd21bb4f1151659eed710e6887c70d78724b41f24f0dd7266f6ff283251', '\x01e150beb96ce743c188a6ec82034239c49745373cd6a55848d3ecbdcf15d7956e29dfe4e440e23f423379d45e5898450b3226a7fc6073e272e897245619a33cf5', '770b16fcf4dbacbecfbfa43ef12f6be2', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 256, '2026-11-17T04:56:50.298358345Z', 'ae9016800219ca90', 'legal_versions', '\xbd7a7530726b5f06ebaa88c7e55a0c5065398d0b4273c98c1419e28ae2a1a306', '\x01fa54ef744f929f85e3d32b8cfef5171da0c529b34e2df741e57ce6798b2eb6f919b6cff05485df87ccd3750d572004387e85fbfa417f845480cf97c1634f744d', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 258, '2026-11-17T04:56:50.305318882Z', 'ae9016800219ca90', 'legal_read', '\xbd7a7530726b5f06ebaa88c7e55a0c5065398d0b4273c98c1419e28ae2a1a306', '\x013206149e6de013399d0f3d638cbd2257d43c49cc9bb6e3ea9c8897e50a391ff0d4c8db3b38b5fa9b54da6c4516ff26c85dcf9ff87bc615dbaea63b10eefdbd74', 'b1c4835a7e58f8b8fa26c7361e052f46', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 260, '2026-11-17T04:56:50.311176195Z', 'ae9016800219ca90', 'legal_versions', '\x5ef75e0c4a1399332ec6f590399832651402377b9cb7731534421af64c1c102b', '\x01518868eca8e66223afd5a8d9a8562089935802d9347fe39e918777b690237e281ec2cc52a65fe83cdc40f07c7e149a2ee022bf7d0f75bbf322b65c8c7b0dfc6d', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 262, '2026-11-17T04:56:50.317754842Z', 'ae9016800219ca90', 'legal_read', '\x5ef75e0c4a1399332ec6f590399832651402377b9cb7731534421af64c1c102b', '\x01a297f437b591ef0d7ffbd8e4ba40fb184ad390a575811d32152ddb8802f9c92177f09989571b8762428d841af37c05f2d3ad7a72c6dc75ab710fd4707cc569de', '99300f9c7835036b92b37c4f5908ca8f', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 264, '2026-11-17T04:56:50.323235534Z', 'ae9016800219ca90', 'legal_versions', '\x6e205bc901fa69d2201a676dc15f81a14549e3b0376b76f3252101df9a88d6aa', '\x01fff0957904a1674ea3ec67b86ebc717ae681f7f76fd390073c655bf80152d76a130333cd2f349bc6d01ecfd378c92615951a202426a26fd39d7f1e83cdd8bc70', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 266, '2026-11-17T04:56:50.331377347Z', 'ae9016800219ca90', 'legal_read', '\x6e205bc901fa69d2201a676dc15f81a14549e3b0376b76f3252101df9a88d6aa', '\x01aea13b4ca24c0db196aa3b7d979fad5901a50bce661e3f25022313ab247ba02a417cdeab06696ff9d3ec0c0a8e847c8010c2a8a2967ac284296b6a061204ec01', '23e87f5b9d57db41f0af4de157349525', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 268, '2026-11-17T04:56:50.337583541Z', 'ae9016800219ca90', 'legal_versions', '\x909688821211ae0add1b9699726b6aa854112038b84efac2bde416d1d635bbfc', '\x019e6e015e9ccf878524a423cf7c547bb11008644ae1cb9fbaf985231a9a48eb662ea462d06c3756e881155ab267b71be74c446eefeffa057f27c49e8eb1736680', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 270, '2026-11-17T04:56:50.345151132Z', 'ae9016800219ca90', 'legal_read', '\x909688821211ae0add1b9699726b6aa854112038b84efac2bde416d1d635bbfc', '\x01aba2e6eec8b2c14043bb2362e76f0c4a4a28f09670edc791bf2a541b4f6ae9370499869acc8d661e9aae3597824e7bfcf4ed37efba21ce324744ec1a415d8499', 'a4a21fb3468515cf3f20d63c74ad37bb', 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 272, '2026-11-17T04:56:50.350838963Z', 'ae9016800219ca90', 'legal_versions', '\x54d82288fd5ca02b8fcadab5a1fef4db3a192e5cb08f3426203744052549500c', '\x01ab28214d4f3433df7841dce728d1c5f842900b881d64cc3f9eeeb42ab4c5d4e8ef50ae332132bf7302ef1a88e885b56edf650a34b48e978bdc5ab6d892421886', NULL, 200, 1, false, '\x987c37424c69faae');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 274, '2026-11-17T04:56:50.358589249Z', 'ae9016800219ca90', 'legal_read', '\x54d82288fd5ca02b8fcadab5a1fef4db3a192e5cb08f3426203744052549500c', '\x0103a337dfad6f9b188fa5d528122932903d57b1f457fad312bec0947ee17658ae518027731ff139fe87c08254d625ddf7229189cf4eb3e5150fdc79b33cfca1b9', 'daf2f73011ce4d1c968e47ddfff89d45', 200, 1, false, '\x987c37424c69faae');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x5fabb96585857481', 1, '\x019a2ea1c7f7d68eadf714bf701061e55749d3e10b829199bf301db28886a9ef8b8de22d991ab53c8b39b9f3f8fa548676811f594c8f42cdc46c6cedec');
INSERT INTO vcon.audit_key VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x987c37424c69faae', 1, '\x01757af0c361df31c2a11f7253297646fa1dc89c24fbc120ae3aed336e038c69e5d244209ca7618cea9ec7df7abc45ef2e51a95432e494ce2973c87575');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x56c77f6cb157e33b', 14);
INSERT INTO vcon.key_use VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xb74979e2f6024a63', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', 2, 1794891408855882391);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', 4, 1794891409084392387);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x16dda5d11b1d1596', 2, 1794891371994481973);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x526b4ce3c2357f058575ce05aedc9b13', '\xb23d4c178942efc7520ff44b17f4c8e7', '\x019537b3350b0e39847023a9c32df0f55591f439d925e427deb4386cc3f00e92ffe17f', 1794891397937002005);
INSERT INTO vcon.membership VALUES ('\x8a411abc9271f9c20af375ddc800f3cf', '\x283723e9f4593a5ab90b9fa0634a1415', '\x01555e12c46e8326fe4757cc4b86479c12be9a4ca4ccc4f8aa32d88195bcab1a7c00b8fb', 1794891399969407835);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xfae33c6dfad95438e9d916fd499ab813', '2026-11-17', 12, 12, 7404, 1794891408613200500, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xddb2f585c075b1bd3b01661cad7ecf1c', '2026-11-17', 10, 10, 6240, 1794891408885234634, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x39cad505cfbbb396');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-17 04:56:08.442619+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-17 04:56:08.679174+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-17 04:56:08.690334+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-17 04:56:08.691091+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-17 04:56:08.69392+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-17 04:56:08.698689+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-17 04:56:08.699416+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-17 04:56:08.71568+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-17 04:56:08.71877+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-17 04:56:08.719484+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-17 04:56:08.720797+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-17 04:56:08.734258+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-17 04:56:08.753047+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '71c18816ef449418853de604a5134ece');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xb9d67288cfbe4a77ba2047bc608d2facdf10d2705500e6838a16e9e461ea9d33', '\x010cee15b12240fe14a9928014a3e6c5fe0ff69401a75288587356de9d045ece1e875837eb', NULL, NULL, 1794891372033696623, '\x56c77f6cb157e33b', '\x017fb87abd807f0d6ecfabc51e30f9e8d9798dfcd29cf33ba5a4c7aa25edaf15a75c57fa57c18672a8f85304bc00e656215f1a0a30ec0e59c6b2e52ef5', NULL, 1794891372033696623, NULL);
INSERT INTO vcon.tenant VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x3cf71eb99c87d55786ac8429ad243625eac038c61ec9758edaa79492e2ae2da5', '\x01575f4edea638d47edd56410afc64954c1f6bc0fcd100cf5e124ae6cfb745b6d279772bb7', '\x019e1c9c0814b1e638874fd24bd5f5e1942abc72b25303bcde33b826a18dfd', NULL, 1794891380194895832, '\xb74979e2f6024a63', '\x01c6b821c93491916fb37ba785da9d6670f9c9a84cc9a89323e8b276245c70746249b36c1739f6ef5db29967e780d29b9274423a106f67bb146f8e9f6d', NULL, 1794891380194895832, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x03b838a005967137');
INSERT INTO vcon.tenant_catalog VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x67465c63708b8bce');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7');
INSERT INTO vcon.tenant_lock VALUES ('\x283723e9f4593a5ab90b9fa0634a1415');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x83296c956b284e5280d89646a1d32bd13ea45534cfbd35c6e318ba2fe4cce29f', '\x01e2e14facb846aafd895d6e3e88c869634ea7566889b1f2e9024132ed9fca018881d7f2229f824f7a12ca8c2f', 1794891403155316960);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x526b4ce3c2357f058575ce05aedc9b13', '\x013782d2cfdf528b4fa06c6d798f523d4cd3b925ed1dcf62b452402fafba706ea1a1aabe4fa0e6a22bbf6a', NULL, NULL, 1794891393643263234, NULL);
INSERT INTO vcon.user_account VALUES ('\x8a411abc9271f9c20af375ddc800f3cf', '\x01e2b513ed13174b33c6d1934061ba7ffad97d69ccfeb526b67ba82030db601b89053f7b74394ffdc7937248', '\x307301310e09dc8638f117191c5dc24177a62eaba2ff51437d0c572003a86440', '\x0107a5005a21fbcc733aac706dd0fc52976aaa3f70f94698be1c49835ae0d956f7e9c78e99a6d2abf79af477427aa1889dd99a420e8570b3072fc7afad58692e', 1794891395869962847, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x4a9ff1f01a32c4a881b238d53ee160a33310e3f0b0f6c9ec150ca6aa0494fc71', '\x0109614fba208a2185aa2ec1f46bc6183dd8b0d26238905ebdf3d6a4ada4b2802ef9dffca0650a62661015d8a1', '\xaeda5ddc09ad8529461326fed7bf8e65', NULL, 0, 1794891402435617721);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xf16cb1d2b9b6d777951ac598cbf31e11fb35e53a3e53004be872572dcf42537b', '\x0172ba239559c47581ea57929ba397371532841d34f38b72a5fa67df418f656ef91328622cfe4725bc4f7c6f77', '\x5a2226c388b2dd894f58966dcdab0707', NULL, 0, 1794891402490015440);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xc57a9a1bd285f8d6a7073b48d75b3aeeb1f7be419837aef71d7737de6c0062e6', '\x01fd74ab76495210aa648de924e1cde393d809e5d7328dcbc9d4ded3535796c732a7189e311ded06166a53cd2f', '\xfd7af32ef454cf4a88d887144fb5b014', NULL, 0, 1794891402512239396);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x809c15ce61eb014542e37450cedbe4af295f48fdb4d68662ba7a134505a55228', '\x0135eeb1293fb231870b066b89d07f03b7263cba3183173695efc5901b49c98b24dce2e86b8134692090afeeb7', '\xcb9a74eccaeaf6a99cb53562c456d361', NULL, 0, 1794891402536290094);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x8649ad25505b5ec931aaf7f897dcb386ea97549d5ddaefe2ae948a4426f0443e', '\x0111a5cd43e2f8fb5b24f28eb51d4d83f5364a8ec6fe718fea588c1ddedaa6aff2d09b1260de5cf41913d5c98a', '\x3aa660bdf2c45f64e29343b429380b01', NULL, 0, 1794891402287125129);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xfb450155225a0a36069e3039a6e7ed759a01b332d25114c01c43444f25be7243', '\x010a661b829e62eeaf7cfb20efa24dcefcf9104bcbebeb350d0f5281c4ccbed3a426f1406ec025a2172a2644ae', '\x2697919c744a9b036b34f3f47c4a9857', NULL, 0, 1794891402313223789);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x2f6ac1ba0da0ce143bc5e6fcc07376bc4d32874b6c69d2826f877e8df9ff5f23', '\x01ae31e5acba8d25295b802fdb4db3393756ef5d4781f589bd8f167f7aeba8fa970a370f3e6f2f31472b42b350', NULL, 1794891402619003972, 0, 1794891402331100015);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x79c76e6436a22c9956594255c4081515e84473beec471c87a0e6e40093210070', '\x01353e5317a13925ceacea8db0b9f932323aa67537f091510f52c93451d78d80be98a3b267c197e3ad2cae2c19', NULL, 1794891402635242921, 0, 1794891402355051797);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xc412d8a94792dd0dbce150720cd8754d756274ff52ca90d361c2809b9effa15d', '\x01e4e2f0bf0e29b58a7191961759a590bfd9ef3ad6bf132fe43dd2885b559c8db071ef370b53ce743ee48762e4', NULL, 1794891402650556233, 1, 1794891402374817551);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x0cff90eb62143776e43e2ae07c0b5c8a680e6cbcb555de0413c6e10004a3ee71', '\x0139bf24fb915f4c2efa713931c0f1b7ebd4227e2eead85ca1a565ea771a1fca6f5add1dfdc3561090fde22a28', '\x141b1dc3bd78ea147adc904883e31b9f', NULL, 1, 1794891402419202077);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xa9aac0388bb498de16c4a6835246daf37326aaca7c44bb0aed9ca0728c6df2fb', '\x0114344712f35a904b067e2e81a4a01a00276e5215753d67c87ad07c41dea06aad015e04d79f6fb7e3fca27566', '\x4c91522031c9cbf757befecccb8c4ee2', NULL, 0, 1794891402864286415);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x7b6b0b07e4005f01b0a1773b2b119bb0dd5e5415c558459bd7254deaea45b5a3', '\x0150b718da897cc7b9ea6e7b583fa739252d6f1038c86aca180bae4eedef882322048fd3a879ef38ef294b1761', '\x1ffbc38a5175bc800d8666776af9b19a', NULL, 0, 1794891402882536902);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x13945d79ba84b6d9086452a85c6e6f70d50a45d933d68dee8c8b6e5080bbab2b', '\x012e3dbf96ac74fd31219417475f3c18159b8aa5c3a7dec5e0b9a1c98f00c50faa9b4e9b07255b0cad7add9ce2', '\x770b16fcf4dbacbecfbfa43ef12f6be2', NULL, 0, 1794891402939368399);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x7767c7e60d942b67ff94b7a92c55027eb920e4f78c076e70fa7f7bf755c497d6', '\x01a61dcb57cafee5e103077b8e691227fe420a0b9e05db21d9077b0ed2d94e1bd8f3575de7372eb75cb2ad8235', '\xb1c4835a7e58f8b8fa26c7361e052f46', NULL, 0, 1794891403032180278);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x223a217c994875c1ef50b8927ccb0e76f847962e9d2157bbd2cce44196a2796d', '\x012ceb5d31c814d87ba34ef0172763d1d35960a30049e9c959423700454cc0c6db8997bdfd46cb93333077642c', '\x99300f9c7835036b92b37c4f5908ca8f', NULL, 0, 1794891403062071576);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x5f876c2aabe8e59fbbf0e10dc2aeba6f4db500c591d6a71fd09d928ffb193a10', '\x01d14a8c1f921bcd0f7233917aef27a250dd7ff87d4d04be98bb5537ee1d93656fb4109c51fd9d52b90091b1f9', '\x23e87f5b9d57db41f0af4de157349525', NULL, 0, 1794891403085363366);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xdbce3186b04650250476e5d70ff0c4d1c8148d44cba76effe2bfafebe8d08d9f', '\x014c9a34b614cce29cf832b00d9e6da70f76c2f78ef81c7a01371e42604b8cfbe54b57742b28ba2689b1e75767', '\x90739613d619cb2038cb93a46c72620a', NULL, 0, 1794891402736672980);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x3a24c240a5eb6a5f88408fa46b76b82906e027a410752b467f170ecec70f9dd5', '\x012b742c3b72a71a8dc4945c2d48d807e1ebf737a5131dbb9a5cdc9c6b2da5eadfbe82cab67e72455bfe2e1302', NULL, 1794891403238167772, 1, 1794891402832778801);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x97b2368d2b01562f8de303048b670d626391eee2f143dc40b6694930ce160b64', '\x01fb6d9fc413615df7d9bf54fdb93ca0c08821e5667cce538a68e074efcefd81dec1551bf2b8220443f41fd828', '\xa239cad07fd5f13741c69bb6ed584d6a', NULL, 1, 1794891402849764862);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xf603384b709fbb90470a281422a6b5d9a7571500c8eac393063c9fce8384c37a', '\x0102c7bcd2e0e814be639eb2d7f835e75d0b6ed90b1876113a0d86b745b2eb49bb04f5267999aaedee71bd8234', '\x9d06eeff0bbd220b6e037da88138bcb1', NULL, 0, 1794891409995937164);
INSERT INTO vcon.vcon VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x68421f928f17a1cec9ef56939713a5d23491e8b35e5b0111678b3487637ea2e8', '\x010ad2a535fc586433501a900a15d3d71d7852db3585514974ba08b4878752829530cdd9ef335d188d9db4056a', '\xdc1405519d9414d1d705bcf1c1dbd122', NULL, 0, 1794891410012953168);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xb688f97dcde3ae2078b0a83a877077587b0fd6fd13baf85a3f205e242448e1e1', '\x012e78bad3ee61db4cf6ae49adc306728c98cfb744c3d18df8a11434ccb5bdce05aeca285d12476d26b715fccd', '\xa4a21fb3468515cf3f20d63c74ad37bb', NULL, 0, 1794891410026085202);
INSERT INTO vcon.vcon VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xf199fd7501fa5fb31b13b46b27f9f3e20df8070f46d8406ba37d25533177b96d', '\x015e4a0791facd1de6e83a1a88c27b8d52b3804606cea8b225b459b27192f8fb691b2b43583bdfa13e61e551df', '\xdaf2f73011ce4d1c968e47ddfff89d45', NULL, 0, 1794891410039878930);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x25fc6a98105ed8e96df42f43cc490662', '\x3a24c240a5eb6a5f88408fa46b76b82906e027a410752b467f170ecec70f9dd5', '\x012022c85227b426293feafd8bbc1dc8500466c951a71204677f67b8e73b2bbdbca9ce2b42', '\x0187e02d1e40e7730a6b3e43ff7e3275da9e93f945e653066eafb3df4ea5', '\x01e9a4e47a3d8f71ac39a17ac4978c9ee86f963f0e7f859e3f3360c322536f4177381862fe', 1794891402832778801, 1794891403238167772, '\xb74979e2f6024a63', '\x0150640f9f2ea8900559d95b3ee99a1054319e31b67daa4eeea63f88c4f44c799f5b23af25f8c1b8b360f45856195dbc067a0c66d4707fc83958acb362', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xe303e974c474e0397c9e1ccdecd2d8dc', '\x97b2368d2b01562f8de303048b670d626391eee2f143dc40b6694930ce160b64', '\x01870d5246f222c7939dc4e8a9adb0688cdddbb077073536c562655ebb70d50018fbfb700c', '\x01675b67fbcb162e33a24ccb4debcbc088717e97b43d40bf8eecb37a44fa', '\x0159bb338510767b8bad4bd93ae386ff4bbbf2ec20c9f589fbd1cdb82772677ba3de60d5aa', 1794891402849764862, 1794891403299373535, '\xb74979e2f6024a63', '\x0192f6ecb9eb8e87d32edff222655e62b53206ff2486f91570311857a785fa0471c5839e9039fdfe48d6d2e4fd196e697928528d7f34136d181079dd87', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x4c91522031c9cbf757befecccb8c4ee2', '\xa9aac0388bb498de16c4a6835246daf37326aaca7c44bb0aed9ca0728c6df2fb', '\x01c306f95a7a31656d4d4e898824604f44a4f1052940f570a6190a9c6a1dea8825e1e1f151', '\x0159c659a5c2595acaf8e96553313fe56c392bd50c835ed91cca73c813f6', '\x01616c7e2426495c8984ef0e710dc8e1b970fe8b1e72c5c86b0ad686d8df29575564cf34d6', 1794891402864286415, NULL, '\xb74979e2f6024a63', '\x01204d3028ac18fa015cf6568c1cb2c4cf101eeb7af5f8ea07e4327a7c5c7c2f49f1f997877838d688790c6e76dcfe1704270952e132b33d349b0195a5', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x1ffbc38a5175bc800d8666776af9b19a', '\x7b6b0b07e4005f01b0a1773b2b119bb0dd5e5415c558459bd7254deaea45b5a3', '\x0104bc2f223eaa67d688212a6ab402b44994dd464a1008dcaf55a0e14914c657ef4167ffbe', '\x011616fba296104a94b7b9473a8b9e797c41e7d8553004b4fc9566c0c7f5', '\x0149eba63574c03b9f1c2cb9678d4c73d5cb3ca16fa1409a8775eac621d6a88e66d246b134', 1794891402882536902, NULL, '\xb74979e2f6024a63', '\x019bbd1ee68b1be7e814588b8a9b0e676ab8f71e464bdb09f2e64e97d2ba135a821e60fb2a47613bed3e0043d9febf9c53d353318aefa7c4454b7ac62d', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x770b16fcf4dbacbecfbfa43ef12f6be2', '\x13945d79ba84b6d9086452a85c6e6f70d50a45d933d68dee8c8b6e5080bbab2b', '\x0178cf86266c0911bbe85cb14f76ffa6fbffe2af71409beb1a839396f521ee49f7dd36b2e7', '\x01b17f15e7309bc9e5cf58c2e234026b5b73273c21cce1385d5133bd8340', '\x018f99d47875dd43336c8efd7ca3fcf5cece6e055b2aeccc360a5e9445421aaf60375726b5', 1794891402939368399, NULL, '\xb74979e2f6024a63', '\x01205cd36757092366a5bb8c112ef7600f94732bab166940f3e870c11804111fd69580582d16a7ebf46e24ece7911e00e1e3aa3c389fa702f4512878cd', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x968c3999d1f5af0d5962cdd8a6f2e80f', '\xfb450155225a0a36069e3039a6e7ed759a01b332d25114c01c43444f25be7243', '\x011ea24777b664397a03fccc422b30e3c370db25a16dfc1dcb9b4844132b43ac49ec2e1b17', '\x018d67b887c12e9deb658780f7e7bcddb9648d78859215f4d9de32418f62', '\x01621ca115c3c622043fca2fcfcb8f74b0c462247e2f1958631f4af09e7171904b53e3b787', 1794891402313223789, 1794891402595212117, '\x56c77f6cb157e33b', '\x01867a93bc6ec4a9bc3fcd8fbfd293d0876517c6de1f79c8198c9fc7b61ba4866caa187a0325c343718942819eea0e4d59dd1023876614e4f07cf5e7f2', '\xfae33c6dfad95438e9d916fd499ab813', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xfc44a51228f85f57e9fdb326dc1ba4b2', '\x8649ad25505b5ec931aaf7f897dcb386ea97549d5ddaefe2ae948a4426f0443e', '\x01dca313fd158f9e1cd56fdda781252cb31bbc2dd5df5f9bc033b8bd2718cdb50602f6fed5', '\x01961151459f713fd3be8eedace68a9bdc7df88c4e9f32c6129e3874ac9c', '\x017094e6006e69778ccb8e595ff879e78f745d52a3df927d63a475c68e393300888ba8099b', 1794891402287125129, 1794891402562181070, '\x56c77f6cb157e33b', '\x011435623e3eb4fd1c5b814ea459cf6f8102c5f53f36701e5fafdb22a0232357fbd18ac44fbebd49c32318cd08bd2cc9608aa8cd91f067b20ba12df89c', '\xfae33c6dfad95438e9d916fd499ab813', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x1db03c12259b4435d7f66ec0f3cb22b5', '\x2f6ac1ba0da0ce143bc5e6fcc07376bc4d32874b6c69d2826f877e8df9ff5f23', '\x01debc4955ba99aece7ead8c5df828560d1987c422393035c69610ae81d24b8c766155e22d', '\x0106cce71071a4bd63921c8c8e2a07c1de01b817547d1ff7a38aceacaaec', '\x01704ac526d353f42b3a4b45e47c1c79fe0b55b689b3486841b3f4ca92747b9b1350c5a12d', 1794891402331100015, 1794891402619003972, '\x56c77f6cb157e33b', '\x01c0046905d0893e1ac18f99be0d87f3b4f7ee6d82d6f70847adadd897cda1b8ab75f0ca390c2b7de55484557abad7c4cbc048d83b0058865d0fc8da50', '\xfae33c6dfad95438e9d916fd499ab813', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xe93afe67f5420586054545958de66f44', '\x79c76e6436a22c9956594255c4081515e84473beec471c87a0e6e40093210070', '\x01d8ae819c9ae0c9ea8f7d8fbf5c81c0f7ed7c76af1fc927501897879ef7ffeb907267d1c4', '\x01bab18edd662a9063a2722889cbaaaa3748a6d9543aa9e90b2bad6e5e74', '\x0104ed2777f5c3cd44311ad96d8eb998d5ef0e35290a52e79162bfc8fdb87a1c2086146280', 1794891402355051797, 1794891402635242921, '\x56c77f6cb157e33b', '\x013a8820f620084e5a5797f09cf1547a2c5e73fd8f72930c64870b282d7bcf5a43666ab043b7894a5adee1eb859b332635342e32e478b86af7a0c6384d', '\xfae33c6dfad95438e9d916fd499ab813', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xb1c4835a7e58f8b8fa26c7361e052f46', '\x7767c7e60d942b67ff94b7a92c55027eb920e4f78c076e70fa7f7bf755c497d6', '\x01eaaf9ffd33babf7bd0459cf3d5f8ae4cd3e95e59e45121fe78862dd8d40c912761a3b3cc', '\x01430430e2b9311c3aba6e6b20bec34020fd05a7cee14c135071cbc99bc8', '\x019b0e72234cd94031fc0576fb6eea0ae7d5a517eb0bb7dc067278cfe3135044127f2cc5d1', 1794891403032180278, NULL, '\xb74979e2f6024a63', '\x01cd76565531eea4db8a391b286cbe0c4567f56a7c66709d52e8401ae7f8558d63e2e021ac32e28f5eb431f90aaf0dddf701e2f654dfcbac92399cf8ad', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x99300f9c7835036b92b37c4f5908ca8f', '\x223a217c994875c1ef50b8927ccb0e76f847962e9d2157bbd2cce44196a2796d', '\x01d60564092b286fc459c288472cab3c36e02d174a917b7635d129ead2e08ab8bb4bea6175', '\x0157f4ac01aa96d51982896463a1e2953a3175a55f03ee23fe51897d9f9d', '\x015a1aa5b90a35921d2b3405305a407c6717666a5334ef4d63936c5c88931c567abec928f4', 1794891403062071576, NULL, '\xb74979e2f6024a63', '\x01b44043c0a914e2ca8af56b1d0d77d9bce7a0405da6f072fc3c1229ac6d0fc5a78d8b4a067c00096c772c486e2072a9e0f79641af3db6e427e2015806', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x23e87f5b9d57db41f0af4de157349525', '\x5f876c2aabe8e59fbbf0e10dc2aeba6f4db500c591d6a71fd09d928ffb193a10', '\x01bced6700768d4a81d161ab083d758b8fcab3da2d7995e1465c8507baab3ad8880e4aba4d', '\x0169c0baa5fae4a4e457e4e4ef7e64b1f4b486d800117ff592ff9bd2756c', '\x0136cf7bffb65d3e48caf6da4ea443d0a126bab06ad0bca3d0f6f91d9365c94e2225598779', 1794891403085363366, NULL, '\xb74979e2f6024a63', '\x01ca3faff248d28bca11c640ce84e1782e3a48cac63f6bf09a44beb97f7e9e4e14d067d3c7708e5d7499b90a44ffebd4fb45bd61588ef5483c5d98621c', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\x90739613d619cb2038cb93a46c72620a', '\xdbce3186b04650250476e5d70ff0c4d1c8148d44cba76effe2bfafebe8d08d9f', '\x01f2b1194c9093da041a6b76407e3bf9450800f4983377a133b50f7e573837856b6eb34b8a', '\x01d5c193477018abe1e4506af2dd99915eb3837bbc925e62d9a5534ab05a', '\x018a2687caf1f63797a6ae28bc28b4cfe64bf0376cdf204a9a6896702566ce96b9382dbe91', 1794891403125361810, NULL, '\xb74979e2f6024a63', '\x01799395555504d4d8dbfbacb03c5586a18b2340374e4fba3ed1541d53b20979759388c05f27f5e30ba817ac4bd72c5dcd1af64ef265f88080b6070248', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xa239cad07fd5f13741c69bb6ed584d6a', '\x97b2368d2b01562f8de303048b670d626391eee2f143dc40b6694930ce160b64', '\x01f1d085a254566fcd1b9f155234dc263f9833e3423b69be8680780a713d3871e622378241', '\x01a897d983fa9654687d8fd4e140dcf645d46f10ce066e6e1c986f6600f8', '\x01af7c42135d80bc384856f6f60d936f00d3dccca83f1d86e7de9fb8639c8b7d7eaa1f8699', 1794891403299373535, NULL, '\xb74979e2f6024a63', '\x013ebad2b68de27d6ee0eb2a57449925f1a1b391ed715bfb7568bd49c519183a3236ee2d0e83f4502521ebac71b54c13b6b6e277b3736eaf4af89680e7', '\xddb2f585c075b1bd3b01661cad7ecf1c', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x0b7b6661b68656d19936506eb549c404', '\xc412d8a94792dd0dbce150720cd8754d756274ff52ca90d361c2809b9effa15d', '\x012b421adbc2b81a992d719a7e6ceb4b7fbc7ec2352c1f91085704d182901681afd7344994', '\x0150acd8bad5436d9cf0d97660fd9ecbf527f9f76a549e56445a4d329514', '\x0159734ce1e3c031aad9101957565b6fc2f0ac27fcfe4c09fa6ff4ad81088e374b21aa525f', 1794891402374817551, 1794891402650556233, '\x56c77f6cb157e33b', '\x01e4caf49294111b108c2ab28567c494191ea47695eb4f6c97bb38b7542028a785d8f79e51c58d9781be262c38318db39f194f56c4399d030cdb7e4e41', '\xfae33c6dfad95438e9d916fd499ab813', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x141b1dc3bd78ea147adc904883e31b9f', '\x0cff90eb62143776e43e2ae07c0b5c8a680e6cbcb555de0413c6e10004a3ee71', '\x01aa1752e4e871aa0c8674c9c96ef67f7de3613d8c926fc236a66a97bc133a74aab408d930', '\x01f6f277052d5ebe48b4cac10f31d0dacfe5e75d8af21ee7880b4de99ddf', '\x015bb7d421ab20e442160b5da33da1012bdc2e6e2b13f32a9a4756f00cb52236766be05ea5', 1794891402419202077, NULL, '\x56c77f6cb157e33b', '\x01aa1c8d96f4cf9fdac2ea7f63cb7224f5a49fb489d7e42f83c6af26cb7d26a063d62e96001aec7670e2b2d15f01276676e8b030723c92f2c2000f1a5f', '\xfae33c6dfad95438e9d916fd499ab813', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xaeda5ddc09ad8529461326fed7bf8e65', '\x4a9ff1f01a32c4a881b238d53ee160a33310e3f0b0f6c9ec150ca6aa0494fc71', '\x0167ce6c08e8fe6fd1654aab025833275cba8165656dcf8033505e9232cc47c339edfb25f7', '\x015ed55e446e616ad3adeeda2efec5345536a39ccb2947d2df180dc0ce71', '\x01d44c012f5ec219704cc775d82809a65ed308e52c7c1064436aa151c65b1a5a645536981b', 1794891402435617721, NULL, '\x56c77f6cb157e33b', '\x01c067d3d6bfbe2e89d2c3c4e6c9b6247ac0769154f7411914144e19adb7cd4b269c5cbcdf39acd28ef40f5f460c8b2c50c66debdabbecbe7d90bae1b9', '\xfae33c6dfad95438e9d916fd499ab813', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x5a2226c388b2dd894f58966dcdab0707', '\xf16cb1d2b9b6d777951ac598cbf31e11fb35e53a3e53004be872572dcf42537b', '\x01e1d5e304a9318405a19e480ffda4ef3b3b7b1f3607cb3d93573e09c434ff07827c58a9aa', '\x0169d2a41718da199bf8a3d685e0f3635c4d4de5ad18a1dc2d8a61c2c1cb', '\x01bb247a0e3b7ded7398666d9078e16103d52dafbee934dca2f960944fb177d238deb5309b', 1794891402490015440, NULL, '\x56c77f6cb157e33b', '\x010844a14d49f4d936f5aa9154d952276ed8147e782f6d6d5f3141625fb73517767859351cd78cea823819a95fb4e3b1c1b2a1988f43fc9f6a1be7f9fd', '\xfae33c6dfad95438e9d916fd499ab813', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xfd7af32ef454cf4a88d887144fb5b014', '\xc57a9a1bd285f8d6a7073b48d75b3aeeb1f7be419837aef71d7737de6c0062e6', '\x019bb8710881c6e0a42d5c4b4b7abeaa611b1435d3c0d46b131e77b527f60a8601ef306b7e', '\x01f510cdf0d8f7c3dc9cb3119271f278d94ffc6ebbd91ab624eb6522d1df', '\x0110c6a07b045520ba21a4a663a0c9b527b1d9ceb7e15e8775405c216f692441b0067943ca', 1794891402512239396, NULL, '\x56c77f6cb157e33b', '\x016034af24f2adf5cb81b1134fb819ed751a4c4844abc963a5fa4743ae56dc05908eb27f3c33227b818a456e0d125d743dbc187f894bf8e04e69869fc2', '\xfae33c6dfad95438e9d916fd499ab813', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xcb9a74eccaeaf6a99cb53562c456d361', '\x809c15ce61eb014542e37450cedbe4af295f48fdb4d68662ba7a134505a55228', '\x01c6b69346abb4130da4182cea8c76ed5859b1db799c4f9db2b92a107da0a60c07c04473b6', '\x01e34a32bfbac71d5db2971a215e5e3ac6e31b8e68ad12c7e401514f5620', '\x01a4d3d16382eb6525d2157892214cd5ef4a57bbc2a24d835e0d49715c4b57602e3bd33088', 1794891402536290094, NULL, '\x56c77f6cb157e33b', '\x018d130a147b3584da1ca14b872f35d1cdbaa59e0d21731c6dc3172ba0d150501ed7eaf6787de352f4f43cecddceae3efa197c982a0863c77d99e42bff', '\xfae33c6dfad95438e9d916fd499ab813', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x3aa660bdf2c45f64e29343b429380b01', '\x8649ad25505b5ec931aaf7f897dcb386ea97549d5ddaefe2ae948a4426f0443e', '\x01c4a8726d0cf4cdcae767a8e07ad45c3611f9ad235606fbe20af4c9151209c9ee83289f3f', '\x01c797abae38d033a696b5dd095e7161cedc32dcb75be23982b50060ae03', '\x017cc98e254cb0d661c8d02ccf521d5beb26e2d5a100b71045f90cb546ca87bdbe8964d9c3', 1794891402562181070, NULL, '\x56c77f6cb157e33b', '\x012c19f7a570d92a73f5fc14193b53206d204a99cca56d79abb53baa1b8631efd9f7f71b2c8277a32531a50cc260ef8c93b0de418984a0b7c3857a0653', '\xfae33c6dfad95438e9d916fd499ab813', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x2697919c744a9b036b34f3f47c4a9857', '\xfb450155225a0a36069e3039a6e7ed759a01b332d25114c01c43444f25be7243', '\x0154290ecf5f5f69be38194073cacc0762ff49507be87029ba92ba262167425eaadd65510a', '\x01c514a63e599c8300239929d78aacb5746deb2d255ca0329a98082f7f7e', '\x011546e1cc9dd636fdb702f2ceeff9b0731e819dc1bcc5b5ebdff39d74f8fd139c69e908b9', 1794891402595212117, NULL, '\x56c77f6cb157e33b', '\x01fd2d405889b90230bcdc2550c4430d9ece51d7071f3526feffa386ace32748c2233db81246d220bc271ef480d04a99b35cfc3213dcc3b413d9587479', '\xfae33c6dfad95438e9d916fd499ab813', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\x9d06eeff0bbd220b6e037da88138bcb1', '\xf603384b709fbb90470a281422a6b5d9a7571500c8eac393063c9fce8384c37a', '\x012eabe850ba68354bf2a08894d1ce83bfd0bf8db7900d7e3bf774d1ecdb26fd4ac8321051', '\x01d93ed3e97f0d022b50ff85ca5c3047f42f48517a08826b52eef01862fd', '\x01367a191a9dd6263c220f9738f579da02b533b99b72a512759da65ef815c7330280756789', 1794891409995937164, NULL, '\x56c77f6cb157e33b', '\x01c29361867e3dbf0dcd93bc5e6939ad0f5162742ba678f971284d120667ef1949970e5bbaea65b27118093256183cc01c90dbf52691995caedf172d21', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb23d4c178942efc7520ff44b17f4c8e7', '\xdc1405519d9414d1d705bcf1c1dbd122', '\x68421f928f17a1cec9ef56939713a5d23491e8b35e5b0111678b3487637ea2e8', '\x01867f54ca82fac9c0e725a694d891cb486756231ae3ff82d59de5388019d5ad8dfe7ed3ff', '\x0157eeaa2fb8b6d105a2611d7a1ef11ccf43464e0d20b4fb43252486e765', '\x01d87e73202d7d264fa07112f70342ac93650a8fb22ddda759f6c22c56463e9f3f12f0b324', 1794891410012953168, NULL, '\x56c77f6cb157e33b', '\x0169a7d5208bda2f43954226a3f73a790e7326905275477292e3432a4685a04bd7403e8d6acac961cbb4cfbf869b4f9e2e970bdd02bd4a081b66f612b1', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xa4a21fb3468515cf3f20d63c74ad37bb', '\xb688f97dcde3ae2078b0a83a877077587b0fd6fd13baf85a3f205e242448e1e1', '\x01894974fce011ec54040374d510878bad6ad2ccc1beb2cdca213d17cb18f27710d4a7de1c', '\x0131d2d1008dbd9dc80b3ee1f3d63337ba5b8152780927923b6dde8d5f1a', '\x019e8ba75ae142dc7a0d80ec0aa705f4455a3ee1be9fa15afeb745e031c938839f343c8e85', 1794891410026085202, NULL, '\xb74979e2f6024a63', '\x01a3eaea635d7981e3d654b6a90d05923c26cba3436a89dcc3ab220d020f15d03b6cef62c6a56e7aa32301c34f60e2d3b9c4dde40798d8a72944f8c661', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x283723e9f4593a5ab90b9fa0634a1415', '\xdaf2f73011ce4d1c968e47ddfff89d45', '\xf199fd7501fa5fb31b13b46b27f9f3e20df8070f46d8406ba37d25533177b96d', '\x01bd9407d6e3a8387ca92bddb872351d4859006845fb34f890822c6171e650c349185cd6de', '\x019075af6e2fab927709431618ab62f7e0a6e2f43486b98eabff13248a12', '\x012232da53f7fbc4d5c0535956fba7de594a716ddee92f9d84803fee6f4c34adadcc683b9f', 1794891410039878930, NULL, '\xb74979e2f6024a63', '\x01428c004c05c8e07e93f698b17b37ebf240ef895a74e086b2d8f350d48a1c4ee85bcdbe3335f64dc6ccb9c906c4d9e882024156f6b39b551d3416524b', NULL, NULL, NULL, 56);


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


