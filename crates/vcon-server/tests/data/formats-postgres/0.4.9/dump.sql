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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-17T22:19:44.316854252Z', 'cli', 'tenant_create', '\x691eaacff232f008d85b19dfdc63f31d39840cbc163f65ce51eaf896d5545b9e', '\x016b6cd36fb4b2b4e810de8097fdbbfe2f2a80587aac5030b588fc6c6cc1d8235ac823a446a78bf47fa9d7301467ec9822a25e1f587170778ab640862c', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-17T22:19:45.876904523Z', 'cli', 'key_create', '\xabec4353867c2f9163f429dd564629baefd3b117f42f53e8c5bd55587648303f', '\x0114b4e9610e6d0d757db4753c009cefee68dacae8321c4f3de4a83182b205e4faf18bd92fef01b0fcf9bd51ad', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-17T22:19:47.488390965Z', 'cli', 'key_create', '\xf85a3c41c08d14601849c999cf5bc342fcf62c5679d4e61c9efd6a3f337a9d0f', '\x011386404fe17f52443c450093fde32aea81abfaee5a4f911bd543c260fcf6ce0755d8a81394b48ec633f742be', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-17T22:19:49.022472872Z', 'cli', 'tenant_create', '\x01eb68fe7d4a681569576e40863b27e8703976e693d8aead5802f270536db6f5', '\x01dad29a80cd15c5e4e9bab753c2c73ae54691df8dbfab3f2340249e58a9d95412ec8c3dbdb5cf0776fe5ae4574131be2526e35256ea1def8850eb5181', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-17T22:19:50.500777088Z', 'cli', 'key_create', '\xe2088c7c112d03c30654f995cff483f366edcba4a4afacf854f9561c8e088ee4', '\x01bcd7a2c103341ab508495195b96d72e757c607609a9ab75ceb874be6d898955f9ab15a4bc6bb8d42528175b1', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-17T22:19:52.028505438Z', 'cli', 'key_create', '\x1eaed3d9be8ed92465a21f6ddd3c0d0904ec01146644617970f70799f70d9884', '\x01855df8f0584405e24b0e3c4cf6855d651a5e9f145480a6da16312a8d75b55adb67bac70013b6eeca0ff79ae7', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-17T22:19:53.545685748Z', 'cli', 'key_create', '\x5535c0c7598897bf14e518ce03a168b9389d49db1410e4df515534f5491c4459', '\x018d6ef489d614cdeb8fab85a94bca4fd2207c87d4c8b72d02160b9523d9f44102d7e9999c57a88d3ce28ac17a', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-17T22:19:56.86070328Z', 'cli', 'user_add', '\xc082267783a0f2678ae3abf3f28a6bd4a8ca1f5f6cb2476e318b1d046522238d', '\x01ecbe44a82b06ec4de3ba813d88da3beb567f59d59d4ce078ee720b198a87d41947592977907a1d0514001810ede7f66d58e10d1a502c721c23c070c4', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-17T22:19:58.367036137Z', 'cli', 'user_add', '\xce48b30ab4afdeedefa40fcef3e15cc998c596669a8ab19e9eac8bc408185750', '\x0172574a06e5a3f893996e6130072854b795cbf7ac6ad0388543ec59b841a63886255ebb125442bac6fa6c72006823de329c4e72acdaf48720740cb191', NULL, 201, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-17T22:19:59.862259061Z', 'cli', 'membership_grant', '\x634dcc92d9707ed347a323826317b67a3b694722b3f71d7cbaabe1ce7c2b737a', '\x01a4e972caa2c4c8883fee302ff76eca47f01340c6f7832912af73d7a4b19ccf4717f64a80b69c9cf8ca85504d605bd7f70005c4bc34f07d385fcc404c3c23de4694fadd6f7b0e75163df5fb650f0c0ce9c1a29dbe019512a42762bf501a818c40c1ccde57', NULL, 200, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-17T22:20:01.390741863Z', 'cli', 'membership_grant', '\xf9fd028b40fff366744472d944622705ba2b82d8f23227574c0f7b51bd8be8b7', '\x01ceddef0cdcee01a6231bd1130b442c5c080f4d6c5c621f6369affc656cd3e278850b134430847b4c814f0de5474440bf28bac8752844b2c52e52f9fe139bc61865b3d81d108c5aef1eb5b70ab4ef7073f6c667cc6987d9d527d38b4d69f42557f09c174393', NULL, 200, 1, false, '\xa825270508e0f8ab');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-17T22:20:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\xa825270508e0f8ab');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\xa825270508e0f8ab', 1, '\x01b693552193c70cdc41f3d51e484ab950b5796379c882a1d2988e03e9f77c2a6e59645776e179ef146b4ed11b31a34358073d7a5c3357761d987f61d3');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x485faf2dee1b7ab1', '\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x1d768408ab229edfa7d02c75894560334701e5e5f28cd6e38b1ca16eb43b47d9', '\x0167dc4c81ffff34a6b180648752018e7b54d0147c410e188861590e99604989e5', '\x013b08eefe89c4245b1732b0e6da7e8031cdcdc46bc648c01b089d66ef358abadc6737122b175c4f0105ebd4', 1794953987487995233, NULL, 1802729987000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x06a4b0fbe665f645', '\x856adc4789b5e383dc8cc0dde06e73ff', '\x7daefea1613755a0b67a558925c952f2426425bdafc931c81c983241a8edbaab', '\x01d816d839dcd9c1eeadb014c5c1bd662f4d869df19d4452aff5c21f00d7fca96a', '\x015f4fadc09785fce1909b35e197368857bb39c7691c213183c4ebc0e2fb19230d85cdc802bc78baad197af7', 1794953992028112503, NULL, 1802729992000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x5e96f17b1e5aeef4', NULL, '\xa2bccd3f267795146d830519a50d50c5485cfb156b741cb04c716834418ad4e7', '\x01c5b857ac38766bd77dc05d9e3c4b27ce49dcc9b1e353a16a90b4b88df3715b9a06', '\x011e5c9fa79e2cb223a015f77000cdef325186749f96f2918148ef729e822175', 1794953993545097601, NULL, 1802729993000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x3340f530d5d64c3d', '\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x0d64299ccdff27da074e75185a77196b1c1f879b2333febad1f088a4f609146a', '\x0100bfc20b13714ad3b70298c51d8841addaadaf439eec9bd5ce8df660a06b7ea7a4a688249ea4c89a9dcfd65f3c0f7f2a0b1b504c552fa21b7a', '\x01b90d03a5160f1166a8ae97b4029fde56ffb1af450588fd241bc1fe9e0c2da252a3558abc9d3b7a2cc5aa155149', 1794953985876295634, NULL, 1802729985000000000, 1794954000000000000);
INSERT INTO vcon.api_key VALUES ('\xcdd908c38a78ae71', '\x856adc4789b5e383dc8cc0dde06e73ff', '\x440af8eef6163b80bd15c062282afc01202a7076d1e13a8a9654adc6ce1704be', '\x01236eda0734b3fa7acc90d1aa30e7dda138390841c526e6c7ed49c73837029682ee554850ee871efa7f759dd42df6567b6fb4dc952bc2e1eaa6', '\x01301732bde8cd2886dc64f9738b6f0286c192d8e6c8797a0c90b09a7f7a0b2a1285945578c2038b16cd83d7bf0b', 1794953990500387579, NULL, 1802729990000000000, 1794954000000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 2, '2026-11-17T22:19:45.880390668Z', 'cli', 'key_create', '\x223dabe0ccb95b3c4ae317cff0b25a97c20e96004712b5427fb162bc8991b07f', '\x01b40a7975b401d0116005bebd42e749161f5cb58289fabb38091dd8d23c61c1ae0ffa6da0eb5f472fd2fed30d', NULL, 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 4, '2026-11-17T22:19:47.491587898Z', 'cli', 'key_create', '\x3879c7eba6e19dbfd501f2e37b5362af0123bd57fd3c7df580db407c69a3fa1c', '\x01eacfe1dd1c473afa8e27ca599e8030c3b5c36955fbcee631d403c81184f8bb13c4377fcc26067f025e5a5274', NULL, 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 6, '2026-11-17T22:19:50.503896994Z', 'cli', 'key_create', '\xb05b8470080b739bec64f82bc53e8b79341f5c2a794c579d2f6c4ad12036243f', '\x012b787d2f9325a8fd825710b590c7e328acfcf2b182889cf1c6afad25c717dd4b5f4f916f96a8713fb6c53aec', NULL, 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 8, '2026-11-17T22:19:52.031630547Z', 'cli', 'key_create', '\xb5b94b8f69568d4011df4fef9c954b45f4b84640d872fa2a65e78f915b764baa', '\x01b22b7112ccc634e3fa84269f8ec0697e652352ffb950f7b282a3e921cd867c03817b41247c57cacc474552a4', NULL, 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 10, '2026-11-17T22:19:59.865364799Z', 'cli', 'membership_grant', '\x4bb1db7937a9deef5e29409d03d596cc3c43c49d494548edbc6d5d7c02134f3e', '\x014722f69983e9d4d99e6f315608a41ca0d19c6fb4603b2cbdf153f39cfa15db93d35272fa50f9a1eab08c0fdd3b77b85cc4a5180df0c43e0fa624deb87d00f5ca082005965fc9c62dbd6b56e529515a48acaab2ce77651c26d71add50e23e3d3e342dd39b', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 12, '2026-11-17T22:20:01.394021259Z', 'cli', 'membership_grant', '\xa5996343c6dcffabcb58f6fc5b3fdbd4086d26d0a35a3f5baaf87fc2741f96c6', '\x01496fab3143ebf7562a3438bf131d4278738b8830d26e2260e5a877dbd92742ae224965e524653ffd79999f7514dbc1058b0841ff61f1def11ee1a3a31f03a78436b29c93e8275424789a384c01a7ecc86f467eb2f92c0f16dba65153d754e1898753ecc00b', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 14, '2026-11-17T22:20:03.432636598Z', '3340f530d5d64c3d', 'create', '\xa10d366287a23707096363aa7350df389a617bf19cc63f14a430e37bad4fd789', '\x01e16aef60699326846c7bb12f633332bf425224fd4015e4a6f1a00f75dff2cdf5d04c4e0680870ed482ce5f52742ac6d9699fdb5298581a66c5be57282d278a50', '8406431664c8e4bd85b2a9ace2ed80c2', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 16, '2026-11-17T22:20:03.44434759Z', '3340f530d5d64c3d', 'create', '\x77bf477d3c67dc33b92cc4aa1a3d42219479dedf13e410b7fc4eceb3649c90b3', '\x01446f7f7d80b73afaf1d781811f4f9661827d0f785ba48b783feca750279c0f5bbb534b861a703be362cda0af6749c436ecce45f28e8e0f4611c7864ef47d9f9f', 'e75955cab8eb987c380060e6c513902a', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 18, '2026-11-17T22:20:03.457216646Z', '3340f530d5d64c3d', 'create', '\x28fe42019b02680faf373051e3f376c658a4c6585e304a9c7ac9adb96ab38815', '\x013baf1ffa37e95d94e3fa71e6255e0c1dbf9484bbcf6d9e8aa70cb62ae4bedb55dba9cd32b38a61ce49dc59707fd435677ec91e9ce12f4d1e8a79f41af0acc7d9', '990784fbafc63a63ad2af8b65c2746be', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 20, '2026-11-17T22:20:03.468859222Z', '3340f530d5d64c3d', 'create', '\x99c888e84394b3258ae73ad08bde58ab20fdd6ce04c8675c94ab626028b7409f', '\x01bda134d58a43321d52f7aea5facb60a2e02119499ee2c6902dc073dc07625481a452703b6b7c441965ca1dfa2f3f03de8bb1525a8b88c0c5d14e054f5e034c69', '4fc3a4db762576f3d5ea6544704682fd', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 22, '2026-11-17T22:20:03.481753038Z', '3340f530d5d64c3d', 'create', '\x8916a63313c38fcdff2f61c4ad0a3c4b26f29a3124b1c029cfef2bd9550921b3', '\x01db8cea2767f1c0963fb836b360f29ca6bbeac74efbff89de812f33367fefee2a689fe027c5b9fdf511de593c875b9821e301e510bf009b51c4cafd0a76b30f5f', 'bca3da17573568ab81c01e78064f17c1', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 24, '2026-11-17T22:20:03.491528467Z', '3340f530d5d64c3d', 'create', '\xdcdc625477b78044bb6faab60f482749112652ec38b91ea19b0fd12a1f004e3e', '\x010fe23734e7de9f6579f40fbfb573cdcc5962cdf463ba5a7e657a0d20dbc84381bf948eabf0899925304047433c4a1a131edaf2b8bda65859e432ea4bc0c602b0', '844cc5c80c49e92dc4a3faa705090bfd', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 26, '2026-11-17T22:20:03.501319664Z', '3340f530d5d64c3d', 'create', '\x8461c626c3d3edbe640002a600e5a58ff15ef2e23a09cbe777d9ec4f1fe7acb2', '\x0102998f7b1d70a72febbde28c2806a0b0bf5f2a53a94ab1cf9622b804d956baed2db94129348df6ab374d272caae83a7fafd1430486a931a0d8dcfbc868b18324', 'dfff8ad8a26ffbd54c2987f9c90d8cdd', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 28, '2026-11-17T22:20:03.511650704Z', '3340f530d5d64c3d', 'create', '\xcaf0771688f644e5e6d84518ec47c848796e858c027c3f3312d21d225511bcf8', '\x0125b30e85d4927098c0343284b34fe2aff8957661f036ffe368307b9d7d6f735dfdbd3347234f1184455ebf4583b4268e5cc0a886af95d018df4ec95cafa13b59', '2de52dfed21f9b52c96c280d5b7ab986', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 30, '2026-11-17T22:20:03.521020679Z', '3340f530d5d64c3d', 'create', '\x38b25d5d6b70b263a0e67e504b3590bb2b30064c3dc8468c2efd4e8fabc3d265', '\x01ba1e0547add55d2474fcd703e16a6374f84b1587027d8956d1e0ebd5a99d47b1f120353e457219722347e943fc7293163e499786f7d9b0156c76eaeaf06b74f8', '8f8f9a54492794f5f01b35778cdf6b81', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 32, '2026-11-17T22:20:03.530270201Z', '3340f530d5d64c3d', 'create', '\xfe0e4cb23713a175ae199000626c58067d90ac5f4a53543f5736a8c7aedab3ed', '\x01319993cb8188d4e472c212d8baf00f39158840cbff449f4a8b246b899d88902b3df7026bb24e84c4f37ceb5561a248eeff3af3793c07198bf5c1cd22dce63a54', '5caeb382d942a443efe1e5b8c8efed15', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 34, '2026-11-17T22:20:03.541413823Z', '3340f530d5d64c3d', 'replace', '\xa10d366287a23707096363aa7350df389a617bf19cc63f14a430e37bad4fd789', '\x014f76111ddad2716d66c2086caad15b3d4cc883bfe5791ea622e725f4526bb5b46cb9d269c3b269c0feef67a36d8c885ab233fdf8d3bc7c387fd3a5df6c983b2a', '6d750d195d6618a79852856d6986ff01', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 36, '2026-11-17T22:20:03.552799306Z', '3340f530d5d64c3d', 'replace', '\x77bf477d3c67dc33b92cc4aa1a3d42219479dedf13e410b7fc4eceb3649c90b3', '\x01f6c70dd27b3bc2a808f5c46ed9614e10699f49883b2251f5f9874105d609d349d56e91f5757dc5636c68f8c7ae617aa88c254568b3939a8fb382c968025e0278', '792d0a02b24f8aced766b943c48d8961', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 38, '2026-11-17T22:20:03.557385117Z', '3340f530d5d64c3d', 'delete', '\x28fe42019b02680faf373051e3f376c658a4c6585e304a9c7ac9adb96ab38815', '\x0141a0cbf467fcecab6f7e8214fdc7b0e4060e6644971f522b9daf04836f8d9819b227094abe8dc581eaede24487f0d7d0fb2a5165a0657dbcc912f321f2120448', NULL, 204, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 40, '2026-11-17T22:20:03.564432298Z', '3340f530d5d64c3d', 'delete', '\x99c888e84394b3258ae73ad08bde58ab20fdd6ce04c8675c94ab626028b7409f', '\x013d7f3b60035ac1675c9cc2cd8975d1e9efd786fd77862f37c47137e823d6a4d640a747e5b10605a43dfd8d69360970599f38d30b77144b985f193b61afa29d25', NULL, 204, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 42, '2026-11-17T22:20:03.569684442Z', '3340f530d5d64c3d', 'hold', '\x8916a63313c38fcdff2f61c4ad0a3c4b26f29a3124b1c029cfef2bd9550921b3', '\x010c4573e5a4383d50aa51b54c4d4e0f96d421c265c330c7b09c3d3b68c01375ddb140cdb0ef27a678e0e4d48938528869b165609f2fbf7c904813c0400bb7fd80', NULL, 204, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 44, '2026-11-17T22:20:03.573411976Z', '3340f530d5d64c3d', 'delete', '\x8916a63313c38fcdff2f61c4ad0a3c4b26f29a3124b1c029cfef2bd9550921b3', '\x01458138ffa5fe18caa48e38af8cec37e4e222ed04f2dcb89d851c0132893c85c905a03685727a8e2098d8615835832c1ae831db241f48ad6e38191203b15c6ba0', NULL, 204, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 46, '2026-11-17T22:20:03.578552955Z', '3340f530d5d64c3d', 'hold', '\xdcdc625477b78044bb6faab60f482749112652ec38b91ea19b0fd12a1f004e3e', '\x01f481fddb4f10a6d391bd5dd218118eb0bf48f74c837d2abb6293e36b38d74e12f1290c093a71472c088937d8f525a37d3cf9b76bd4f4314bdcb7e1f81bb761fd', NULL, 204, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 48, '2026-11-17T22:20:03.588245762Z', 'cdd908c38a78ae71', 'create', '\x1381f9e25c11a3f2760e1ba6748079f3b175e94e242a97dfdc9284536706b25a', '\x014fd6a4467d4d44be5177fff6a7df56095cf049122aafc444055a85e9958055b68f5dbda348d00067a0b5505e1c6618ad1a5cd8f14e761d5716a8ef8b7aeeafda', '0f282ce86c40f0489f04876560e49030', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 50, '2026-11-17T22:20:03.597228099Z', 'cdd908c38a78ae71', 'create', '\x45c229db5500a3ecd9b68a3d3504da4695a222d629ea9185fd503e03f3dc9a9b', '\x0186a5db0c4114d4a407684bfc1e74af6f06acb72e6b5e799208a45a01a40b481886173987c10ce60a71ea1ff112347f5baa263615b2ac7e57b6366dad5ece81b9', '6637344e3ea1ca3a878edc81198bc33b', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 52, '2026-11-17T22:20:03.606794417Z', 'cdd908c38a78ae71', 'create', '\x0f7da0b3c95fa87a95fd240ae0f91bd24a0e0a4c3ebd07143cc93a6fbbc7c4ab', '\x0140b86943411cfa37f80345e8d3e698161826f323a6eafa303a26a24a0f8961f6b7c52f01d620dadfc2c9b58e3572a8788880394c3cf54777462cfe56b8ab34a1', 'fc43ce460265daa4d42dd8b68ecd2bde', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 54, '2026-11-17T22:20:03.616748113Z', 'cdd908c38a78ae71', 'create', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x0147c798e14f0b43ac6299fead0fd67778d50dd99a6c03698ef2e310f9e02597d2369793ce103e0f66cbc27ea283b7ac2ffc126c0a62fa34e535a25ba6bbbb204f', '60e1b99da54ff00ef7d7358ec3b2e7f2', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 56, '2026-11-17T22:20:03.62648744Z', 'cdd908c38a78ae71', 'create', '\x6e13919933e5a54485f2e7d96c07cddb593bbd7e80e69a83bf9b63350d599947', '\x014689cba85c6f10f239365a8b2dcb2d5a7b8a3315c707a41a63535aca1ea72003d711106c660d9a0094b389550f563a9e5b685b53c6b5e38e032c33b7dca2cd08', '4dbb6f36e1dbc94927ebd8b70378e228', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 58, '2026-11-17T22:20:03.63644285Z', 'cdd908c38a78ae71', 'create', '\xab4e75a304e2af4b7a5e4bb9b4b965441804e6926fff7f62747424944ecf0114', '\x019293ab1706de23d5b513a5252900e1ee9d7aa4cab785e5066a16a2a5925e5c111c0d39d3237c9f4b87e9d1415593a4f2b1c7ae13126eb6a20b686e4359fdbb86', 'f65cad1eb1b8e7b320712fc144555413', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 60, '2026-11-17T22:20:03.645092751Z', 'cdd908c38a78ae71', 'create', '\x398e4d1c6f0053f668fc9c6dd52743a5a0e715a1d1f90a63f63ff503c6bda0bc', '\x019587de5af92f3135f9555f0c34f0fa8755f3a42fd9276cc49f126f239aefc73176bd5d5a15557e6f944b63dc835a10545011d6c459a61caa077bc7043c2e8a6d', '5c53122e0b815ba4a4aaaad201ca9adf', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 62, '2026-11-17T22:20:03.655109911Z', 'cdd908c38a78ae71', 'create', '\x6f4939c08555d117fcbb4b1624f9a4ae375628dc5a70bf2dd11d17293a65580d', '\x01887290affaa22807e3ce663c90ab80b8997a38bd5d49c01a94d5fa952f9229f1c7fe452d59707e5605f428301d0a7e1c710098e1ead826d240ead809016e3585', 'ad764c9e0eb5c5ade499ea928aa142f1', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 64, '2026-11-17T22:20:03.665948681Z', 'cdd908c38a78ae71', 'create', '\xaac3a12fa8108f44537c9ff22b175a4430fe80ab93e1d2ffe0504d39570e74b8', '\x0181a097c295e9233e67db9fa5c9ed4ad8610e724c1576c5e9a00af72ee68b1627d9563ecb60464bd31abd2fe2245971b171efd43eb219fdceafd84df8c8590fa3', '112e8fb448ac0f695c3caa60d67997ec', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 66, '2026-11-17T22:20:03.676037007Z', 'cdd908c38a78ae71', 'create', '\x9cf51672b17714b5ae1dbb2bc5e0aa925749284352ab85d86f81e860e41f3eb1', '\x014e2fe5518e7fe8d6c8f81c7dc333daa7d950e175b0c334d9963419aa48d5340e3d73c5bf7e6d3283d13410d86f6ce21972cb250bb3e6f96bc8d4214aecf870e6', '5e4f5b038dea4204c2a17b43ca84987b', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 68, '2026-11-17T22:20:03.686753972Z', 'cdd908c38a78ae71', 'replace', '\x1381f9e25c11a3f2760e1ba6748079f3b175e94e242a97dfdc9284536706b25a', '\x01c087fdddafe6c7daf2d2cc2748e6c4e383737585441fc278e958d9fdf3900f9c5e6f9ba9c76e98616976e267fcaaeccd112780528b2bf7c2a2984a6f1b3d8542', 'e2daf98563a0ed42bcfcf7920282c7a1', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 70, '2026-11-17T22:20:03.691392868Z', 'cdd908c38a78ae71', 'delete', '\x45c229db5500a3ecd9b68a3d3504da4695a222d629ea9185fd503e03f3dc9a9b', '\x0166363fb691a4d551383f28aa9900d61e0e5b9ca7e436d2ec7f35c2075b350104d450f409392ef7a69c85a6214950f11033f6ff48212bbd4c3222fcf874daaee1', NULL, 204, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 72, '2026-11-17T22:20:03.694336328Z', 'cdd908c38a78ae71', 'purge', '\x45c229db5500a3ecd9b68a3d3504da4695a222d629ea9185fd503e03f3dc9a9b', '\x01f3412b2233db25c8b94003ab8255ff7c2a9f3fce04087ec03680a7a553b7abcd259f95def8cf04f8141f1fe94acf51f8c25e99eb2afab358b019924ddca1213d', '6637344e3ea1ca3a878edc81198bc33b', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 74, '2026-11-17T22:20:03.715655822Z', 'cdd908c38a78ae71', 'hold', '\x0f7da0b3c95fa87a95fd240ae0f91bd24a0e0a4c3ebd07143cc93a6fbbc7c4ab', '\x0189150243e636093ff9847ec914e55df1d5e4ae5a759dbd6d4b1897017c1a433eabbc1f57824bd94b781db33aa0ad32ab7e35c95686cec58974025f073f0452f7', NULL, 204, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 76, '2026-11-17T22:20:03.720080162Z', 'cdd908c38a78ae71', 'delete', '\x0f7da0b3c95fa87a95fd240ae0f91bd24a0e0a4c3ebd07143cc93a6fbbc7c4ab', '\x0151b3266c541a088f2b614b144b0acb6ed9c13c34e7c5ffdcabf4a04861d69993bf206059eafb61a8092feb3d122461f7cc204171e1c4f22d796bc930c3cd1ee9', NULL, 204, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 78, '2026-11-17T22:20:03.726087787Z', 'cdd908c38a78ae71', 'hold', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x01e54eb8f3ee9afc1b1603c9ab39a83d36694569ac1cbb77ce72aa695c2adc284417012ac6335fefbcf94c6af49a0aa50cc5e75f5fafdd0d1a80af72ef84817bb3', NULL, 204, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 80, '2026-11-17T22:20:03.737814066Z', 'cdd908c38a78ae71', 'replace', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x011aa9341b8d8dc6ff67021b82862a4b28e064b37c267dcbedb2c16c883f6bbb1f4ddaac3023dbf98f2f4f786b2b3da947ed2b1a491ccd6ea934366e27abb19f38', '2ebe8b1628b7682129b1fa0afce9604e', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 82, '2026-11-17T22:20:05.734679735Z', 'sweep', 'purge', '\x1381f9e25c11a3f2760e1ba6748079f3b175e94e242a97dfdc9284536706b25a', '\x01b51b0782c966744126f3f6221d2c1034bd1ef0a2a6be0da013265cf795f89401d80cbf26ce48cb257fe88f68e0b508b6fb6ea3a8465f3b661a4e9ed7423c535c', '0f282ce86c40f0489f04876560e49030', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 84, '2026-11-17T22:20:06.132964492Z', '3340f530d5d64c3d', 'legal_versions', '\xa10d366287a23707096363aa7350df389a617bf19cc63f14a430e37bad4fd789', '\x0179487cd7e04fa23ba6e7136710acf46aed0284f23f66a09df783035687e03b0b44e4f5787bce1b8fe147795afd1c1ac2eb0fad3c2f9ab545bd0a3db74c5f5b84', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 86, '2026-11-17T22:20:06.138850905Z', '3340f530d5d64c3d', 'legal_read', '\xa10d366287a23707096363aa7350df389a617bf19cc63f14a430e37bad4fd789', '\x01dfb3d6187ef92dc5058daf829694c27cd868ef3e5690ec5d85a668694b002a1e6ea37ecf659361f1c72e85b6ffc04954f7051fad0aef08c3f500ca76d3329434', '8406431664c8e4bd85b2a9ace2ed80c2', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 88, '2026-11-17T22:20:06.143384522Z', '3340f530d5d64c3d', 'legal_read', '\xa10d366287a23707096363aa7350df389a617bf19cc63f14a430e37bad4fd789', '\x017be54b6c68e28d9e35e333deac4cc95891606a62d937ff049ad3c93e4f50d6c6976583b3175729d943a154b43e01b14a652f1c511603d28703998fc31c0d486b', '6d750d195d6618a79852856d6986ff01', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 90, '2026-11-17T22:20:06.146470678Z', '3340f530d5d64c3d', 'legal_versions', '\x77bf477d3c67dc33b92cc4aa1a3d42219479dedf13e410b7fc4eceb3649c90b3', '\x0194a5586d518043ba1b037550bfa0c479cd82e614ef5c05c55e3fdbdee770c1c55f8f4d3aac7c1171a293b8ef7cc04beee19c891d096301283fd90a6e3290a463', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 92, '2026-11-17T22:20:06.150586916Z', '3340f530d5d64c3d', 'legal_read', '\x77bf477d3c67dc33b92cc4aa1a3d42219479dedf13e410b7fc4eceb3649c90b3', '\x016a2672e94b2ca623dc98e8bba1dfa00be27d591d70e7402e7d3932d59fe09027193b6d8f53ac0c56c737dc324ced9dc52cf0ad39f8ef6c4a601f37b6c363f7ee', 'e75955cab8eb987c380060e6c513902a', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 94, '2026-11-17T22:20:06.154700709Z', '3340f530d5d64c3d', 'legal_read', '\x77bf477d3c67dc33b92cc4aa1a3d42219479dedf13e410b7fc4eceb3649c90b3', '\x01888df102871630dd5fe75ab3a7a25ac8f57ca2000ce69f22a2251e4dfd26606adfc6877e32cdc33bb1e2cbebb2925a420e58029a1e461f543c2afe6c816da1f1', '792d0a02b24f8aced766b943c48d8961', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 96, '2026-11-17T22:20:06.157678272Z', '3340f530d5d64c3d', 'legal_versions', '\x28fe42019b02680faf373051e3f376c658a4c6585e304a9c7ac9adb96ab38815', '\x010af8146df5ee858060edf3f874ee71819000f38465aeb790015a500e3ed8168720aa42aded67b8aaa4a2fb97ec46fc45acf0229c19cc2bfe09c185c54c5b9445', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 98, '2026-11-17T22:20:06.161558899Z', '3340f530d5d64c3d', 'legal_read', '\x28fe42019b02680faf373051e3f376c658a4c6585e304a9c7ac9adb96ab38815', '\x013da79f99f767c612da037bb60311f9349fe0207ad8666f733fe090d086eb76c1de9d98c21ad67d853848751914f6d0ed2b327defb6b0b1672161247a76ba0dbe', '990784fbafc63a63ad2af8b65c2746be', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 100, '2026-11-17T22:20:06.164446406Z', '3340f530d5d64c3d', 'legal_versions', '\x99c888e84394b3258ae73ad08bde58ab20fdd6ce04c8675c94ab626028b7409f', '\x01a01ffd8347e8e2469eabd7d10552bd0bb498bc53a7ed35b527cd580abe2320f3c77e976df17ef7f7db3ad2febee61b29785a3a9b594cdf3682fd7ce6e317a999', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 102, '2026-11-17T22:20:06.168291367Z', '3340f530d5d64c3d', 'legal_read', '\x99c888e84394b3258ae73ad08bde58ab20fdd6ce04c8675c94ab626028b7409f', '\x01c48fff6a631ef31ca2390c57ba312464c907124d642dbc9c67e78770e4585077df50d25b9cc698e844d8abd4cf2ff7fc7a7cb29fbbfb5407dcb31f3b43657e89', '4fc3a4db762576f3d5ea6544704682fd', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 104, '2026-11-17T22:20:06.171139893Z', '3340f530d5d64c3d', 'legal_versions', '\x8916a63313c38fcdff2f61c4ad0a3c4b26f29a3124b1c029cfef2bd9550921b3', '\x01704241703069b1626a99be935a8b2385de3321287b4e419c64c94a44f6948442d3349d00e8df0afd8a46e3bd3984d67109ea396eb4b0f04a51f83dd424d85d4a', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 106, '2026-11-17T22:20:06.175244001Z', '3340f530d5d64c3d', 'legal_read', '\x8916a63313c38fcdff2f61c4ad0a3c4b26f29a3124b1c029cfef2bd9550921b3', '\x0165ea8942473f1fb408966f0e4883ab9066891dfc9ab03358a24c76813d6611ecc797118840f8135131bb5c836ec5a15989dc6fcbe04258f15553eddb662263bd', 'bca3da17573568ab81c01e78064f17c1', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 108, '2026-11-17T22:20:06.178141805Z', '3340f530d5d64c3d', 'legal_versions', '\xdcdc625477b78044bb6faab60f482749112652ec38b91ea19b0fd12a1f004e3e', '\x015f18e24e7b0cd66910264066167374847cb32e48306c0135548983c3a22d6a54d305a1d9d2aba47a04e5767cf79788792610f99bd50a3ba169c55e8721c04047', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 110, '2026-11-17T22:20:06.182127849Z', '3340f530d5d64c3d', 'legal_read', '\xdcdc625477b78044bb6faab60f482749112652ec38b91ea19b0fd12a1f004e3e', '\x019d3dafffae32821a6177e06ac6eb8c9c34c100e09470c2ea6b52caf80dbd96bd54e84dd6eff79196f5afe3f7ee7b49d67505c5eadb372e5419c59d241454ca8e', '844cc5c80c49e92dc4a3faa705090bfd', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 112, '2026-11-17T22:20:06.185213505Z', '3340f530d5d64c3d', 'legal_versions', '\x8461c626c3d3edbe640002a600e5a58ff15ef2e23a09cbe777d9ec4f1fe7acb2', '\x01186652a8e6cb43026736c0258d50b13c9a0ee51ee021737c5b6cfc44336ac6efc703ba14418bfbc2b38eaaad8d97e54f31fbcdacb566bb5adc6db420f47a259e', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 114, '2026-11-17T22:20:06.1890819Z', '3340f530d5d64c3d', 'legal_read', '\x8461c626c3d3edbe640002a600e5a58ff15ef2e23a09cbe777d9ec4f1fe7acb2', '\x016d3bd5d0821669cc1b2d12d41441243dbe33b818faa0ae1c516ba057e4f99b8c880a658b0917b755d92e246c7ef01f8aafaf776e3c4e6a76e2a1545b2eab92aa', 'dfff8ad8a26ffbd54c2987f9c90d8cdd', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 116, '2026-11-17T22:20:06.192291472Z', '3340f530d5d64c3d', 'legal_versions', '\xcaf0771688f644e5e6d84518ec47c848796e858c027c3f3312d21d225511bcf8', '\x016370728046beb60b60c03f27d0b035629d9cce3c7b57c9bbc0f21ea2b04893c031fce2afbdfbf0b9991f172a466247a196d166d47bd8a9b65739e206b8389bf7', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 118, '2026-11-17T22:20:06.196771164Z', '3340f530d5d64c3d', 'legal_read', '\xcaf0771688f644e5e6d84518ec47c848796e858c027c3f3312d21d225511bcf8', '\x01314162359ca01a0eefbdc05ebf532a987038f93c9612690db782ee6354deebd523eed2631c373112cd2d05dba5a0eaf888d5267e543ee06546c93f1ce4078016', '2de52dfed21f9b52c96c280d5b7ab986', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 120, '2026-11-17T22:20:06.200019198Z', '3340f530d5d64c3d', 'legal_versions', '\x38b25d5d6b70b263a0e67e504b3590bb2b30064c3dc8468c2efd4e8fabc3d265', '\x01c55c79691d60150997a62949986a33d60202e30c3a9f2125f1e5a0bd174fe26552a48bbc8eb90d530cae4df253b64775b4d78898d9d835d5bef4191f52e02701', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 122, '2026-11-17T22:20:06.204136316Z', '3340f530d5d64c3d', 'legal_read', '\x38b25d5d6b70b263a0e67e504b3590bb2b30064c3dc8468c2efd4e8fabc3d265', '\x0114bf7b2fbdbe42e6c092f0169e11822e19d456ab6aabe8d509020034eee0a4d655b62c57b40fcf9d9b0a443af0f2b73b19c6e69c6e190681fdf267789a1607d1', '8f8f9a54492794f5f01b35778cdf6b81', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 124, '2026-11-17T22:20:06.207001425Z', '3340f530d5d64c3d', 'legal_versions', '\xfe0e4cb23713a175ae199000626c58067d90ac5f4a53543f5736a8c7aedab3ed', '\x014ad0c87bf6f2b041a4a0500da1780cc021f1bca84c77198778629369d6d6a810516a9900eb00b248264958b09223bdcc08d5f97b0f85584c1e6403011e1f6de7', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 126, '2026-11-17T22:20:06.210782784Z', '3340f530d5d64c3d', 'legal_read', '\xfe0e4cb23713a175ae199000626c58067d90ac5f4a53543f5736a8c7aedab3ed', '\x018d750ffa54c7e10ed654754f1804b2feae42d14cad8d23d8fcd23eb06f7ec6c950cccd54eaf0565a41f29d8e640e5e69b27aa6ab98a47d9691f585fc4abc8075', '5caeb382d942a443efe1e5b8c8efed15', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 128, '2026-11-17T22:20:06.21357793Z', 'cdd908c38a78ae71', 'legal_versions', '\x1381f9e25c11a3f2760e1ba6748079f3b175e94e242a97dfdc9284536706b25a', '\x014dbaaa907de2884573a1144953a26a2211b9d553fd9101a79b7828a3c1d74f468ecd35bd2c84741d981d2954d8f2103598346dca6233156be4263aaf0439e209', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 130, '2026-11-17T22:20:06.21733778Z', 'cdd908c38a78ae71', 'legal_read', '\x1381f9e25c11a3f2760e1ba6748079f3b175e94e242a97dfdc9284536706b25a', '\x01e39a71db3a6d43527d33b63728fdd78ca2deeb7f01818dcaedffd6bff3ec0b21b4e8250b6355949bbf0f36efe84f2d2ce1c3dcebe7c2af8b630f645ec636c29f', 'e2daf98563a0ed42bcfcf7920282c7a1', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 132, '2026-11-17T22:20:06.219864222Z', 'cdd908c38a78ae71', 'legal_versions', '\x45c229db5500a3ecd9b68a3d3504da4695a222d629ea9185fd503e03f3dc9a9b', '\x01ca2b2cc7dd9323b86d8e6ebf7fb932545d3e2506cfd9b872ba4ec38a0a0b29b79a3b8898fe09228dc69ce6fdc72c8cd67e3cb1857d43e018a7cab128a7a65158', NULL, 410, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 134, '2026-11-17T22:20:06.222776378Z', 'cdd908c38a78ae71', 'legal_versions', '\x0f7da0b3c95fa87a95fd240ae0f91bd24a0e0a4c3ebd07143cc93a6fbbc7c4ab', '\x01491f1abb96c6395ed213c8c8afc7cbda111175ef227ab03fd32ea67e94157dbb28d96338bd4ec226310ab1ae116b3f81e2884f5d460f73f381dc13a5c7199e6e', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 136, '2026-11-17T22:20:06.226473228Z', 'cdd908c38a78ae71', 'legal_read', '\x0f7da0b3c95fa87a95fd240ae0f91bd24a0e0a4c3ebd07143cc93a6fbbc7c4ab', '\x0127bcb2e286a02ab659857845d79566bd02cd0f7430d078d170383f6600dd1038cbc2bb6f578c16665a4e3a9ff6aca8dc91a3ccb57b27f8e3f637392f92932fdd', 'fc43ce460265daa4d42dd8b68ecd2bde', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 138, '2026-11-17T22:20:06.229284522Z', 'cdd908c38a78ae71', 'legal_versions', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x01c9f8950d5db978489ac49a2a1c7a75669e4513e2a9ede9d90151197b2948d99da2996f47e0fbb66f1bb2afb53aa4fd2742d1f14dc9348edd9b8fa076b5a8b2f6', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 140, '2026-11-17T22:20:06.232886427Z', 'cdd908c38a78ae71', 'legal_read', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x0109297bd32b7eca1c5bb0c647f47a0e8f898dc1ba9c3443645105c6df299fbace0c651c6dea7800c98042113d3c1f2c15557ab4983e4883122a48e7fe564e089a', '60e1b99da54ff00ef7d7358ec3b2e7f2', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 142, '2026-11-17T22:20:06.236538684Z', 'cdd908c38a78ae71', 'legal_read', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x01f15c8655b003c02b22a3ebeb36016b867b705f99fb682192d2910131440153ff0cc30c5960a45c71b7e538e82982c7ac31c955a6358f65114e33b9c1ab4a6878', '2ebe8b1628b7682129b1fa0afce9604e', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 144, '2026-11-17T22:20:06.23930383Z', 'cdd908c38a78ae71', 'legal_versions', '\x6e13919933e5a54485f2e7d96c07cddb593bbd7e80e69a83bf9b63350d599947', '\x018b3123e5d8d7d4fcbcd2b72984159c6c093c7a5767b3f2a8c96c43264f4cdbd2ecd53382823b018d5ab0b2c60ec170fd9c18d4029ed921a80d414c7cf5352414', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 146, '2026-11-17T22:20:06.24288718Z', 'cdd908c38a78ae71', 'legal_read', '\x6e13919933e5a54485f2e7d96c07cddb593bbd7e80e69a83bf9b63350d599947', '\x012ccdbb3799eaacdf99d5c95d71a43d3265f9109440f3d0c80c2b751dcbd581438d47132f5c9d69c610bddd40a188ec3c1f79fc4fde2ed117a3c41ece1b9b80e2', '4dbb6f36e1dbc94927ebd8b70378e228', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 148, '2026-11-17T22:20:06.245803252Z', 'cdd908c38a78ae71', 'legal_versions', '\xab4e75a304e2af4b7a5e4bb9b4b965441804e6926fff7f62747424944ecf0114', '\x01b81f69995f1b0b95c4b4b27fabd58be3b09f9e2b4a787ef3a93483a770838e1ca41f2e829f0cda673b8c8b3a332bac4c0daf5c14ccec28890339becb380fec6e', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 150, '2026-11-17T22:20:06.249382888Z', 'cdd908c38a78ae71', 'legal_read', '\xab4e75a304e2af4b7a5e4bb9b4b965441804e6926fff7f62747424944ecf0114', '\x01fe98136c78923b562639aa01322167c9aba793f74c747e96397985c993a64b8a88e2e174c6da7762b0d7956a4f4146c518fefa30a40d6dbb652932d3221ec711', 'f65cad1eb1b8e7b320712fc144555413', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 152, '2026-11-17T22:20:06.252067238Z', 'cdd908c38a78ae71', 'legal_versions', '\x398e4d1c6f0053f668fc9c6dd52743a5a0e715a1d1f90a63f63ff503c6bda0bc', '\x01779ad0ce254a91104ea8a4977c389b477b549cd8034fb41a374432fe4086ee07ef9287849e71171f3fea5712feb0ad1dc21d63c91c75b80a302f59f59e7d73a0', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 154, '2026-11-17T22:20:06.255639144Z', 'cdd908c38a78ae71', 'legal_read', '\x398e4d1c6f0053f668fc9c6dd52743a5a0e715a1d1f90a63f63ff503c6bda0bc', '\x010d2a9df72bf1830e7fc1d1601006fe65534b716b791d3319316a8ee935549adef3ecdfdc840838cc8a34cf1ef676a27e830c210517f07c8663f9237c48aa8cd8', '5c53122e0b815ba4a4aaaad201ca9adf', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 156, '2026-11-17T22:20:06.258364632Z', 'cdd908c38a78ae71', 'legal_versions', '\x6f4939c08555d117fcbb4b1624f9a4ae375628dc5a70bf2dd11d17293a65580d', '\x01c193c07ee0127e44d5006b95f08fa104a6435bd258a280f0f8611750015a72f8a22bbe972e9e54999b2bb4903257712e675ab01a7b92a28b55224001a9d446a0', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 158, '2026-11-17T22:20:06.262266417Z', 'cdd908c38a78ae71', 'legal_read', '\x6f4939c08555d117fcbb4b1624f9a4ae375628dc5a70bf2dd11d17293a65580d', '\x01f83892b7cdbf49f5d30ae4ab0a633fbda5f18743299f4d0e4fc9c97317a9cb0b7e466833424503f9138fd6a73fc774e093953566a66a9502fd5f2304da0b639c', 'ad764c9e0eb5c5ade499ea928aa142f1', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 160, '2026-11-17T22:20:06.265009545Z', 'cdd908c38a78ae71', 'legal_versions', '\xaac3a12fa8108f44537c9ff22b175a4430fe80ab93e1d2ffe0504d39570e74b8', '\x017f64b0537ff67d7e0a3ccd765a42f4e99d9d5713df859c3d611f41f8f5ef679efd4a92dac84d42c1d0f9c2d5f6765e7dea38fe6bd5416ef6a1c419e254f7c34f', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 162, '2026-11-17T22:20:06.268801255Z', 'cdd908c38a78ae71', 'legal_read', '\xaac3a12fa8108f44537c9ff22b175a4430fe80ab93e1d2ffe0504d39570e74b8', '\x016a39523c32824c2e8ad8edd43f5058fc20b5fd921747387b02c5c05dbf78ebc556360b12c6b76c6083a8b97303411e5d6975576fed7b3a151dda386f165a472a', '112e8fb448ac0f695c3caa60d67997ec', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 164, '2026-11-17T22:20:06.271599911Z', 'cdd908c38a78ae71', 'legal_versions', '\x9cf51672b17714b5ae1dbb2bc5e0aa925749284352ab85d86f81e860e41f3eb1', '\x01a4f65469b45b6256831120f4ad6c6157be3c8b3ea0e6aebb7809b088e8014e249731f82ca23e129d6bf7f1965bd5bf384d44c772dbaf6140efad80dd6fac8e34', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 166, '2026-11-17T22:20:06.275488204Z', 'cdd908c38a78ae71', 'legal_read', '\x9cf51672b17714b5ae1dbb2bc5e0aa925749284352ab85d86f81e860e41f3eb1', '\x0102eea72c434464392b270efb00398a15bacde47bd0d9b1ac3d4aa397dd1a01c67bcc3c9f3d5b8790845580c513108664db5d9fd94e5a6ef15dad4bcd91b20cab', '5e4f5b038dea4204c2a17b43ca84987b', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 168, '2026-11-17T22:20:08.47536368Z', '3340f530d5d64c3d', 'create', '\x21d931f1b3ee5e92aa6c64cdf38bbe81be22c206f00a2942ce4b8ce1c47d0334', '\x01ce3b2627c25961f4845339b0d7a549215584e093533c9143eb81b55cc7a4a34ffeac991eec109cac47cdc4094750f003d00b5e666265b133ead54297a6d36ec5', '7985cad5a50c2a1323dda4ba17eda385', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 170, '2026-11-17T22:20:08.484289637Z', '3340f530d5d64c3d', 'create', '\x58217a5829de8c418d034079a7a02a447e2ad35c65ea31b1faa94e39ed55f27f', '\x01d1d361f2fceaa7145deec0070a2ca1763c012e5ab52c2ade5a8eda27042b5768a2893e573c7f923150950f818b9d9d9f1d5258b9bec42bd95b0d16fc8be547f9', '36dcc5d80e44b33049b7a50861cd68fb', 201, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 172, '2026-11-17T22:20:08.492469391Z', 'cdd908c38a78ae71', 'create', '\x5e6051992f7a5bd1d442237ce78a9d798eb43c45fdacfe1a58d70d829fb4cff7', '\x01113c764e716a524c3faaf34aecef1f2d3731fdb79535a37b2ddedef8c802b3978e321a9da436d62ecac9dd49191280ea9a9ad1e85a0bd526f5381f4c0599e409', '465560473c25b104db3a6f9f21a02a51', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 174, '2026-11-17T22:20:08.500297664Z', 'cdd908c38a78ae71', 'create', '\xc1026e00036e146fa90cb28bbb2452b814623d737d62da968efc1a202e984d15', '\x01579bae67fe6ee1021c629a5af62169d02595aad244fa95b91e33200c1f740e1ea84fe82fd00abf5a9ab37b990bbec891dc99135d2ce9dc1066a27271bbfcb073', '2606bfee5bfcedeb49ffdc83310e73f1', 201, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 176, '2026-11-17T22:20:08.50351118Z', '3340f530d5d64c3d', 'legal_versions', '\xa10d366287a23707096363aa7350df389a617bf19cc63f14a430e37bad4fd789', '\x014f821f97c1a0f3ea4a38cad34f2839150664bd44b687cf560da1749a0c7761246c7c2dd342ca4870e4052a00e26b5cb94a33fa5c09fe4e9e98589b18c8d8aa7c', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 178, '2026-11-17T22:20:08.509082973Z', '3340f530d5d64c3d', 'legal_read', '\xa10d366287a23707096363aa7350df389a617bf19cc63f14a430e37bad4fd789', '\x01123b1e31fe69c169b1953d613874df68e7fbb1261e3cb58acc7ad30e83880073cc55c2191ff22314d38b3fe13e5f0fc6acc18030c217054cbcda60b596f19601', '8406431664c8e4bd85b2a9ace2ed80c2', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 180, '2026-11-17T22:20:08.513343775Z', '3340f530d5d64c3d', 'legal_read', '\xa10d366287a23707096363aa7350df389a617bf19cc63f14a430e37bad4fd789', '\x013bcc3bb36db74c2d64e205362a188fb11c654dd58afaba68e732298d8529083cef4955259ee54dce7a8aff6060828554f19312445fd9689c9c5186eb051fcad9', '6d750d195d6618a79852856d6986ff01', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 182, '2026-11-17T22:20:08.516316274Z', '3340f530d5d64c3d', 'legal_versions', '\x77bf477d3c67dc33b92cc4aa1a3d42219479dedf13e410b7fc4eceb3649c90b3', '\x017e37812d5b24bcec62eb7223e89619e92cbc0bf58d8e046344e4a63e2c23739d29a7ddc8da1afd6365697a123d3935092b4936e9eb90eab70b7d7f9b41a5d372', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 184, '2026-11-17T22:20:08.520256113Z', '3340f530d5d64c3d', 'legal_read', '\x77bf477d3c67dc33b92cc4aa1a3d42219479dedf13e410b7fc4eceb3649c90b3', '\x01110f659188cd98109e2bdb1c86108bbe349586634023c6066f2f06a10a897e63dfe11af6e858dad392250ef0c1db7fe9e67b0ebd675a53989509af93cc782a14', 'e75955cab8eb987c380060e6c513902a', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 186, '2026-11-17T22:20:08.524014121Z', '3340f530d5d64c3d', 'legal_read', '\x77bf477d3c67dc33b92cc4aa1a3d42219479dedf13e410b7fc4eceb3649c90b3', '\x01b820771e5fbbc1ceb737ce4d56ac1683ca9b92bd208703d6ce5b1ec0c5dee821b746b17108aa70334658c5efbd150caffcb6873eef4539c5679b92baf177650c', '792d0a02b24f8aced766b943c48d8961', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 188, '2026-11-17T22:20:08.526988369Z', '3340f530d5d64c3d', 'legal_versions', '\x28fe42019b02680faf373051e3f376c658a4c6585e304a9c7ac9adb96ab38815', '\x01817c594475f8e8e8bbb26b226fb5e16fd02a335650493cda59d0e1df228cc5d69ce56f3a18fd4c6a64c906bd609a37fcf480a775e93eff33fd4748e525ccf9bb', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 190, '2026-11-17T22:20:08.531001681Z', '3340f530d5d64c3d', 'legal_read', '\x28fe42019b02680faf373051e3f376c658a4c6585e304a9c7ac9adb96ab38815', '\x01c20378b0d2fb1698c983366e6d232f888420d41241b6f0d7f05805f721dad1490b5e889c12fff19605dc2b46b3c6092c398c8c0937ed292c9d407c9ff196c036', '990784fbafc63a63ad2af8b65c2746be', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 192, '2026-11-17T22:20:08.533815567Z', '3340f530d5d64c3d', 'legal_versions', '\x99c888e84394b3258ae73ad08bde58ab20fdd6ce04c8675c94ab626028b7409f', '\x014f6f1bed054079067aa9a7ed90deb4380dfbbb3cc2968d09ab8a40d43ac4d392ef4b747b9442b5853a25270df7f73ff10f908379b25d78fb46b770bf40742263', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 194, '2026-11-17T22:20:08.537512852Z', '3340f530d5d64c3d', 'legal_read', '\x99c888e84394b3258ae73ad08bde58ab20fdd6ce04c8675c94ab626028b7409f', '\x0146f584a3b2c31c4a91343c006b641b193eb97c97ecbf308bb132bf479900ab148a1817e3a94e9cb40393d58d0c122c8786ae7c8a891b2f1d409ff3cdd553c4fe', '4fc3a4db762576f3d5ea6544704682fd', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 196, '2026-11-17T22:20:08.540342174Z', '3340f530d5d64c3d', 'legal_versions', '\x8916a63313c38fcdff2f61c4ad0a3c4b26f29a3124b1c029cfef2bd9550921b3', '\x01462445fcc8d014eb91b97d34da131012352258bb09591c7fe0cb4dedae37409cec9d63960fd272d8203b19718fe659c6538f3e112f5d86b7ef564dcfedba97c6', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 198, '2026-11-17T22:20:08.544168588Z', '3340f530d5d64c3d', 'legal_read', '\x8916a63313c38fcdff2f61c4ad0a3c4b26f29a3124b1c029cfef2bd9550921b3', '\x01905832895844b00633009b7f6cd8b0bc3e647a09c4898b938f1cc8ef0dd9c646b3de7a8314bcee3f3845300e40b9d1cb46d0abee34d4dd6a63041525822f6e09', 'bca3da17573568ab81c01e78064f17c1', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 200, '2026-11-17T22:20:08.547020854Z', '3340f530d5d64c3d', 'legal_versions', '\xdcdc625477b78044bb6faab60f482749112652ec38b91ea19b0fd12a1f004e3e', '\x01a42e91cc18d61c044fa7aa0b044f1a9d60b5f40cab6f08ff13889c7416d5ea81c4bacb8de9c9e0b90e0771f8955d3d8df839dba26c233fdba2e20a56cc4014ed', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 202, '2026-11-17T22:20:08.550768288Z', '3340f530d5d64c3d', 'legal_read', '\xdcdc625477b78044bb6faab60f482749112652ec38b91ea19b0fd12a1f004e3e', '\x01e5b2a1a93d7dc42e7ca3f04a557845262ec883c3a4a95949701ccf11667bc930c8fbc6803896a5d27c055f966ece8985287757c66722c2b2925b43547198d5b9', '844cc5c80c49e92dc4a3faa705090bfd', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 204, '2026-11-17T22:20:08.553584471Z', '3340f530d5d64c3d', 'legal_versions', '\x8461c626c3d3edbe640002a600e5a58ff15ef2e23a09cbe777d9ec4f1fe7acb2', '\x0168d077f5213ef0467695eedd5619b2c54abb2c0540ef3a70e9f049a151a00a33ac34fa041e850cb4643c59ba744d7ee024e1c0360ec8ec123c6f038085678578', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 206, '2026-11-17T22:20:08.557433421Z', '3340f530d5d64c3d', 'legal_read', '\x8461c626c3d3edbe640002a600e5a58ff15ef2e23a09cbe777d9ec4f1fe7acb2', '\x01a149e4263857048051b53b444ef8ee3bf78ed168105d8f294c032a1c1c4ab82625788f43533b16ff1903a8f905a1b0cbeab603cb9dc69c2983528f8933846441', 'dfff8ad8a26ffbd54c2987f9c90d8cdd', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 208, '2026-11-17T22:20:08.560351401Z', '3340f530d5d64c3d', 'legal_versions', '\xcaf0771688f644e5e6d84518ec47c848796e858c027c3f3312d21d225511bcf8', '\x01fefe51959717e76cdacc4463c275658c5106f8c8ffcaf6c67652d2a784c60bc7bdd90bd8992fd9a83050a6b9d377493322de6ca67df4dd09c34ea27bce4f79c0', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 210, '2026-11-17T22:20:08.564094019Z', '3340f530d5d64c3d', 'legal_read', '\xcaf0771688f644e5e6d84518ec47c848796e858c027c3f3312d21d225511bcf8', '\x0112149d0b720b5b0c6e563b20c7843ebc33ee356db8b4cfc556963c18a87cdd94acca89d17cb41162bba905efce7e4a43a9b244cb9e8463ed3ff81c9d17ff581d', '2de52dfed21f9b52c96c280d5b7ab986', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 212, '2026-11-17T22:20:08.567023943Z', '3340f530d5d64c3d', 'legal_versions', '\x38b25d5d6b70b263a0e67e504b3590bb2b30064c3dc8468c2efd4e8fabc3d265', '\x01e53384ccefbd1fb205739e3b94d919359062f3e45d83ca3d09deb4e86a436fff3b5d00cb794a06738bd46aa2297529ccba8e6373de0d9ed654dbe621baad4612', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 214, '2026-11-17T22:20:08.570753542Z', '3340f530d5d64c3d', 'legal_read', '\x38b25d5d6b70b263a0e67e504b3590bb2b30064c3dc8468c2efd4e8fabc3d265', '\x01f8c286c6511fb7b8b5e9b57294ae1f7d34aa191905f8399e0cb03679c9893b49f2dd5bebb6a2b1bc199d1bfe9ac292b498446e36fc846be84222c939a540ab06', '8f8f9a54492794f5f01b35778cdf6b81', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 216, '2026-11-17T22:20:08.573591754Z', '3340f530d5d64c3d', 'legal_versions', '\xfe0e4cb23713a175ae199000626c58067d90ac5f4a53543f5736a8c7aedab3ed', '\x010611c611634355435a547ae041f1c1df4264c8e82e990e1e5ad0be8aad140c2c8d91c45e2505c2fdf25087c734a9fe75130631588a7140e902a952012187f17d', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 218, '2026-11-17T22:20:08.577350121Z', '3340f530d5d64c3d', 'legal_read', '\xfe0e4cb23713a175ae199000626c58067d90ac5f4a53543f5736a8c7aedab3ed', '\x01e4b90d4184123f8c4f6758f92aef53d4c54534a23d0957264695095aaf756489eb929b8d61aed81e3232cb7e7fa412e089f7308844ff950abdaca21f6a509425', '5caeb382d942a443efe1e5b8c8efed15', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 220, '2026-11-17T22:20:08.580356684Z', '3340f530d5d64c3d', 'legal_versions', '\x21d931f1b3ee5e92aa6c64cdf38bbe81be22c206f00a2942ce4b8ce1c47d0334', '\x0187593de6d4a33d3d8b1976d9865c7766ad008f8711a23f61a39ab936f72de08234cb22eceaa81bda95a0d8c0dd1fdba9bd5c4ced2511555d7f2cfad3bbc0a609', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 222, '2026-11-17T22:20:08.585071903Z', '3340f530d5d64c3d', 'legal_read', '\x21d931f1b3ee5e92aa6c64cdf38bbe81be22c206f00a2942ce4b8ce1c47d0334', '\x01cec60b01037c4c6430b5ee452c8893c71d2e923282134877bfa08f6ff42a37a136be12167e0584f1488ecf6d30916bc479f316b252801dc06c51a18012fef578', '7985cad5a50c2a1323dda4ba17eda385', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 224, '2026-11-17T22:20:08.588045447Z', '3340f530d5d64c3d', 'legal_versions', '\x58217a5829de8c418d034079a7a02a447e2ad35c65ea31b1faa94e39ed55f27f', '\x018ab8bf06a1ac6db1275a919c741c0fce8c0af955b433977f23a5a7084f07014ddea522efa68c057d5b0780e36dea2072781d094d7c1e0d8c6dc61232c84f18f7', NULL, 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 226, '2026-11-17T22:20:08.591899166Z', '3340f530d5d64c3d', 'legal_read', '\x58217a5829de8c418d034079a7a02a447e2ad35c65ea31b1faa94e39ed55f27f', '\x019ef76200e27e3bfb00ceba69e200850eff9deffc1b0cfad1d5e1b66821a38010fa3ef6d7550688fe000c07d0b6ee3dabc2ca583834f64bc06af5ab5bcaa37138', '36dcc5d80e44b33049b7a50861cd68fb', 200, 1, false, '\x02e8b34ffdce4c22');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 228, '2026-11-17T22:20:08.594901211Z', 'cdd908c38a78ae71', 'legal_versions', '\x1381f9e25c11a3f2760e1ba6748079f3b175e94e242a97dfdc9284536706b25a', '\x0189f5ba16e9b46802aa545a729defea53b084c52ecd3b65d42ed1c251d41905495a23f25d99acdb9b163246464dcceedbb1a524bbcddf950db2ceea6270fc34d0', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 230, '2026-11-17T22:20:08.60000743Z', 'cdd908c38a78ae71', 'legal_read', '\x1381f9e25c11a3f2760e1ba6748079f3b175e94e242a97dfdc9284536706b25a', '\x017db867cef373f392afba373311510f4e5a2d71c84fcee44d2ca409eda839759c2e3488eafa4091225e12ae07e4660035c88f64328af96a79c2ca0a678a4a28db', 'e2daf98563a0ed42bcfcf7920282c7a1', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 232, '2026-11-17T22:20:08.603065372Z', 'cdd908c38a78ae71', 'legal_versions', '\x45c229db5500a3ecd9b68a3d3504da4695a222d629ea9185fd503e03f3dc9a9b', '\x01ec6e46fd5bb42fccee54f554950010d7a2ccf3a778381fd822721fe31eccf79906141939c2ac89cbb1546ffb03fcfbc8f12a700cbaf00e2da79d0689e96de4d0', NULL, 410, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 234, '2026-11-17T22:20:08.606661906Z', 'cdd908c38a78ae71', 'legal_versions', '\x0f7da0b3c95fa87a95fd240ae0f91bd24a0e0a4c3ebd07143cc93a6fbbc7c4ab', '\x01fdd9b95ab2e8a3c830d842d5dc0f7d6fc291152ecee19f0beceadc28eed0c79f8a3affcb9fb288212ac3b5bbaf89e53ed50b004c8734c3a074a340cade0a050f', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 236, '2026-11-17T22:20:08.611044764Z', 'cdd908c38a78ae71', 'legal_read', '\x0f7da0b3c95fa87a95fd240ae0f91bd24a0e0a4c3ebd07143cc93a6fbbc7c4ab', '\x0113f64d8351cce9dfc3a88dcfffdea162b312e55a980f1cb251796248d5461ee50ad2fb21e6b03f44168d8430fa82b6f461f15937c7b3d77ce192d1fc4b01fb95', 'fc43ce460265daa4d42dd8b68ecd2bde', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 238, '2026-11-17T22:20:08.614093308Z', 'cdd908c38a78ae71', 'legal_versions', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x010d2387ebf07ddcfed90c7f7d676c3d80e82a99d0da0594e11e177b9772151c4a560244c7bf95e9f19bed6c4511f154c2d595ca431e317ff23b11b0e98db7d8bc', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 240, '2026-11-17T22:20:08.618026879Z', 'cdd908c38a78ae71', 'legal_read', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x0104f195a42d2476f662c7273e652ac43a22f435e5c6d1fb19c240ae4bb4a9d95bedeaaccbd38679e878594c7840e1502216e0b0cc66bee72c45b3a7809fd3cc4c', '60e1b99da54ff00ef7d7358ec3b2e7f2', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 242, '2026-11-17T22:20:08.62193472Z', 'cdd908c38a78ae71', 'legal_read', '\x147668965d5d3db7fead8ff472464ca2a0f75a67b81c974c2f81122e10e969c3', '\x01f047b674fae99f772d2348396c5d3591878b00c93c6ac22fca5fed89c1e9e4b22ff56050ef72dd96779a2b6f69627d1d8b79f3d99cecdcc132dc8c34e1eab3c3', '2ebe8b1628b7682129b1fa0afce9604e', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 244, '2026-11-17T22:20:08.624793505Z', 'cdd908c38a78ae71', 'legal_versions', '\x6e13919933e5a54485f2e7d96c07cddb593bbd7e80e69a83bf9b63350d599947', '\x0197423cbfcc18869ebdb3538b76ad56fdf22cb68353d5c00b151aef87f0bb3cce9217562f7ce41cd566d1deb799dd139be40d4a882b61a27abd996f3e2f62f1f0', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 246, '2026-11-17T22:20:08.628674595Z', 'cdd908c38a78ae71', 'legal_read', '\x6e13919933e5a54485f2e7d96c07cddb593bbd7e80e69a83bf9b63350d599947', '\x01785016f98aabdbd7749f77bf7fb4e62ef9e1dc30b3542c463c64f1bf369f7caac10528076376e2bdb75ce5fb246a32e7f78c33fc605839f9201a86d1fd82e2f5', '4dbb6f36e1dbc94927ebd8b70378e228', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 248, '2026-11-17T22:20:08.631660824Z', 'cdd908c38a78ae71', 'legal_versions', '\xab4e75a304e2af4b7a5e4bb9b4b965441804e6926fff7f62747424944ecf0114', '\x01a0f865b814bebed9b81b01bc8a4038d31f2d8eb10ab6e5dd7b4d0894b44c5bffbbf3b128f9fe8b6bd64016b5e44b02fdac77a056dd7d2c21b87affe2a9eb0d6f', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 250, '2026-11-17T22:20:08.63547134Z', 'cdd908c38a78ae71', 'legal_read', '\xab4e75a304e2af4b7a5e4bb9b4b965441804e6926fff7f62747424944ecf0114', '\x015c1bdfb18a14f14140709e0710b7cf1cb3c47fdf4b3a1210117959d9dc44192a000a6b5ffd84251e0e3920b883baea1a5043951875b7afdfb2dfe5ee227121e6', 'f65cad1eb1b8e7b320712fc144555413', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 252, '2026-11-17T22:20:08.638346635Z', 'cdd908c38a78ae71', 'legal_versions', '\x398e4d1c6f0053f668fc9c6dd52743a5a0e715a1d1f90a63f63ff503c6bda0bc', '\x0107de297f13a1a4ebec606f4d63502d8831400dded68ce640784cd4d22dca2a89fa004ea3898372c31dcea2f50db01c69a498fa97643b2d0b3b74e2985d9be5f1', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 254, '2026-11-17T22:20:08.642139511Z', 'cdd908c38a78ae71', 'legal_read', '\x398e4d1c6f0053f668fc9c6dd52743a5a0e715a1d1f90a63f63ff503c6bda0bc', '\x0121d5000f1e7129b3cdf6acefdcd550bf5e015f3757ca20c962eddc5f8594646b2bd55f92057c47df24a52ee1c84ed6874302f74ac3f863f7635eb615cee76ba4', '5c53122e0b815ba4a4aaaad201ca9adf', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 256, '2026-11-17T22:20:08.64524475Z', 'cdd908c38a78ae71', 'legal_versions', '\x6f4939c08555d117fcbb4b1624f9a4ae375628dc5a70bf2dd11d17293a65580d', '\x019a8878078679161e64f72488daecb547c7d027fdcc50fbf8c8f8b1a6992cdcb479caf49e598118f71f8e40cde0c273fdf2a7f3716c07f69a110ea4a5aed28faa', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 258, '2026-11-17T22:20:08.649063682Z', 'cdd908c38a78ae71', 'legal_read', '\x6f4939c08555d117fcbb4b1624f9a4ae375628dc5a70bf2dd11d17293a65580d', '\x0185c008718b61fea3aeed0436159dd05c90bbff6d2178f2c65291e69f7d4d2899dfbfee0c7f8897c652828407bca75e8488272af60cd87e4a4d4dcd2bf5a8b51a', 'ad764c9e0eb5c5ade499ea928aa142f1', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 260, '2026-11-17T22:20:08.652117402Z', 'cdd908c38a78ae71', 'legal_versions', '\xaac3a12fa8108f44537c9ff22b175a4430fe80ab93e1d2ffe0504d39570e74b8', '\x016628660151a77a90cc8a03dddec45dacecf0fbf0977d312a5adccb5864a10512f9f53eac7a1939db95105fc5160aa6c2dfa8f8ddad5016eb397d6c1a3445d9c3', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 262, '2026-11-17T22:20:08.655876835Z', 'cdd908c38a78ae71', 'legal_read', '\xaac3a12fa8108f44537c9ff22b175a4430fe80ab93e1d2ffe0504d39570e74b8', '\x01ecc74014dbcd3d973f1dae1562fe6650aed00efbdc2ebd5e745d2827d6639c74b55898b0784adfc4a37e721d59d8efb1afa55f24589a1a52392a7ff2eec25d92', '112e8fb448ac0f695c3caa60d67997ec', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 264, '2026-11-17T22:20:08.658741148Z', 'cdd908c38a78ae71', 'legal_versions', '\x9cf51672b17714b5ae1dbb2bc5e0aa925749284352ab85d86f81e860e41f3eb1', '\x01fe27f2659d1d675f6ac310bcadba2853dfc497a55f33f0afc17ca4e62c2c36378712a7baebca64ca6189ec3ca92abbfdf818e3a4994e1c25495c1bfa7d005778', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 266, '2026-11-17T22:20:08.66276159Z', 'cdd908c38a78ae71', 'legal_read', '\x9cf51672b17714b5ae1dbb2bc5e0aa925749284352ab85d86f81e860e41f3eb1', '\x0195193e0730647ca65b17f93f15b023b7b70790466f3389a03e7ca0c58f058472d90cdf215f7bac16b0608b59f2eebc4f17e89dd15e4feeaa837a0a0013d989bf', '5e4f5b038dea4204c2a17b43ca84987b', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 268, '2026-11-17T22:20:08.665592476Z', 'cdd908c38a78ae71', 'legal_versions', '\x5e6051992f7a5bd1d442237ce78a9d798eb43c45fdacfe1a58d70d829fb4cff7', '\x01d8efc3c20985fb22a1dc140aad9d4ce6fc36d5b3f957b502650252ab51d6bbef2995759a1729bc4991eadb1d4a51cc242b4fc1b8f299856f9d85579ea44b2685', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 270, '2026-11-17T22:20:08.669425297Z', 'cdd908c38a78ae71', 'legal_read', '\x5e6051992f7a5bd1d442237ce78a9d798eb43c45fdacfe1a58d70d829fb4cff7', '\x015fdd0f614e1792c3a64d36b815af95b213475f7a35b6de7d841e713f0263522e5798288d26ea0c9bfb239ec8c410995d230651247a82163157ec2c48e549ac18', '465560473c25b104db3a6f9f21a02a51', 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 272, '2026-11-17T22:20:08.67230575Z', 'cdd908c38a78ae71', 'legal_versions', '\xc1026e00036e146fa90cb28bbb2452b814623d737d62da968efc1a202e984d15', '\x01e2606497a2126bfbfbda2aaaeaf6139621a0ed35843b8e67dd2124dd4e473af00aadd9ed970402b3a86c0922bad0bbdb91c12b0bd755d90013d238c439d64fc8', NULL, 200, 1, false, '\x95cf23592c0d9b76');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 274, '2026-11-17T22:20:08.67615809Z', 'cdd908c38a78ae71', 'legal_read', '\xc1026e00036e146fa90cb28bbb2452b814623d737d62da968efc1a202e984d15', '\x0164af4490a9f1b7abf9d186e9a45c9e11e487130fad85706499aad14fec168381a1485b94f273a00f664dca2039353fcf93e0c9703ee53342f429b01f734bd9f0', '2606bfee5bfcedeb49ffdc83310e73f1', 200, 1, false, '\x95cf23592c0d9b76');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x02e8b34ffdce4c22', 1, '\x016491a963a4be292b51b060f4b242cc8b27edfbb1f47bf9ba1d8ef7ea98289c3d24ac1cbf0e7b242b78ce06b186b487892740f1bd0d2ce728f35192ac');
INSERT INTO vcon.audit_key VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x95cf23592c0d9b76', 1, '\x011cd400ef9686b9e5dcd24253bc1e16d2f0c6fa2af23359a036fec2f80ae37f86adf6185a79ee2556ff33737e2e2fccba0f7087bd2a64d9d11a893f08');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xa193accc4e020b22', 14);
INSERT INTO vcon.key_use VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xfe9ace24e76ca768', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', 2, 1794954008223801568);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', 4, 1794954008343140645);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x224d3a9f5feb7427', 2, 1794953984307334269);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x40ce7f83315d6d1ee126da65128aa4cf', '\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x0161f074f476d56619a55723035087549fc387097d08a4c14aa4e4d658c7c4f982dcee', 1794953999861273830);
INSERT INTO vcon.membership VALUES ('\x4b8c083c4a7ad9759f2df95019160e22', '\x856adc4789b5e383dc8cc0dde06e73ff', '\x01d78046504d08730a36e7f3aff26f23d79c88f5311ec7683057de52ab73797583795155', 1794954001389946447);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x23b9d7fcf7482133d623422f86d3ca8a', '2026-11-17', 12, 12, 7404, 1794954008095174194, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xef06dfd75568786396571e4d47ab11c0', '2026-11-17', 10, 10, 6240, 1794954008239118141, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\xce378d3efeb8f0c0');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-17 22:19:42.621942+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-17 22:19:42.655304+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-17 22:19:42.661728+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-17 22:19:42.662362+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-17 22:19:42.664338+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-17 22:19:42.667277+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-17 22:19:42.667818+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-17 22:19:42.672018+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-17 22:19:42.675349+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-17 22:19:42.67592+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-17 22:19:42.677091+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-17 22:19:42.680818+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-17 22:19:42.686342+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '1dc7748cad2d5a1248d500afe8ce761b');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x52baaed82b3eb3f50e4a21cbe8c887c856cd394f2dcd4b43bfe98f4e28c523c4', '\x01970a551fc869bcc04af615a56b5bceb1aa1cd1492bfc84b474d772e1b79ed07eea1cc201', NULL, NULL, 1794953984316539706, '\xa193accc4e020b22', '\x01bf9aa4580d94d63dd6ac01599c60eceacbe13262ef3b1c1ec9822bebf8e94e4918cbe3db8a1bfbe01c597d29c36e5159c61334359849148b52a07785', NULL, 1794953984316539706, NULL);
INSERT INTO vcon.tenant VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xee6f74bd6aa362a379b61a0941d5c63118cd464e39cb4a454041a37b1c78d639', '\x01b98c5aea9cd017b434c0e97aff7293429fabc0cf8feda1a24091182725f536065594deb4', '\x01d32023babb6e06ef8e2a6a4fa446fba24a6ebd011ec938dac1d342bc87c1', NULL, 1794953989022306501, '\xfe9ace24e76ca768', '\x01bcf6efb060d6a68b1ce0b4b7f8463516f5f2f2ec8ded709ae9e53185cff50d1ae6fc752d910c56dcc7cc1f152aa37a34360305684d38de6e5ae0c677', NULL, 1794953989022306501, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xa43bc6f127850dc3');
INSERT INTO vcon.tenant_catalog VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x3293408472b9a001');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa');
INSERT INTO vcon.tenant_lock VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xce3971923106118fc5ddc58b3b3b02edbdda997eb696848f6a699ba8d9c23c93', '\x01c685f4d1a80a019bca50493946afefe85fe3ba87da6f7bc3f9bb1d26feabcdd2aff98f23230bc9bbb664065f', 1794954003692782913);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x40ce7f83315d6d1ee126da65128aa4cf', '\x011a799618d45c8a8c56cd168fbcbe0e8168fcd3d17ff54206ec0a26b2e917710350b8c04fa42370ceef1a', NULL, NULL, 1794953996860086846, NULL);
INSERT INTO vcon.user_account VALUES ('\x4b8c083c4a7ad9759f2df95019160e22', '\x015bc2e26405c89bc83c1b8a5d0da89b6ab738474b32d3f9aba52ffe24fcbbe821faa689e78de7d5941493c3', '\x348f007fdce7a90c6c50c5c97a34d08b3c864cf1053767c0564fddffe171ba54', '\x01c2a30062daca286de37146c354cf0cefbdb4baeaaf6fb41bb17dae30cbfe9e68d7c950de703b1bafc18d223c275ce4956f50d9374f6691323b1e3938eb5458', 1794953998366734091, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xa483209f7185764470eb6e3086d2c98adde69947db507e39c63d4b70657e4f1a', '\x01e3aa93dfb1493ea0f421aeef03a589f59b58f65c052ba401a75bd7cec0d01cf9756663fcf8b151cc48174fac', '\xdfff8ad8a26ffbd54c2987f9c90d8cdd', NULL, 0, 1794954003500740304);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xd5dfcd6bce81b0cb737a8a5643e01d4c345700caa7a515f928c5bda8d1f02cb6', '\x0154e0ebb52be48c6a52a916f400efd13c59f2959b2c7fc15a7a8d496fc6e23854ba51448523ffe7dd8002c74f', '\x2de52dfed21f9b52c96c280d5b7ab986', NULL, 0, 1794954003511162279);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x5211f8088c99a61e3fee8b79d300f7e0be7a86eed4b13e144a7f8fb55fabe446', '\x0161a7d47eb336701cf8b4b5da33199a4c1e3d95352eb508ba9af5f3b8988b1cc7e1942771dcfa43f4d4fdefc2', '\x8f8f9a54492794f5f01b35778cdf6b81', NULL, 0, 1794954003520627920);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x7737373606ceff1c2d3f7f2b0cc9f292b7e4e6762bb2044e2c121be2f463f622', '\x01ac637429887c1b078a40c3af166926b401722540da16d3a8406e04bb131f8126b37c278c3e5706e8e95b6e76', '\x5caeb382d942a443efe1e5b8c8efed15', NULL, 0, 1794954003529888914);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x90c63ce493f2733dead9a8f3ba29b01a9390860c1a071d520810a049637282a8', '\x015ce59f6fe83e1cf700405210fb90a9ac5912d1776fb97f68f7e168f5e1a11e8f6b8d1356bc2f0a086fd4d4c9', '\x6d750d195d6618a79852856d6986ff01', NULL, 0, 1794954003431393905);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x042b4d175f7f88045d6c760f4fab5099f08f7f24bebce369c7e1b77596d5276a', '\x0185171a4553f163b047e55dfbe68a2f57b7c47e58c7753dea1e46314994da4a2868a6317155ebd2520f330004', '\x792d0a02b24f8aced766b943c48d8961', NULL, 0, 1794954003443880887);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x8b8a7852f1c168be30c458ab567158f2134989d31ff0ff13e330fecb1e0e3554', '\x0145ec8722319e5a6f5784ce5908e4d8601b26f5c6337de7a87468e5bf98b06d9445681deb53cfb9b8afd4c551', NULL, 1794954003557022618, 0, 1794954003456804776);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x874eb6fa34fe2a709abe2141fa6fea2cea66fcb0be3a8419cb86f3e06bbe1541', '\x010896b526eff243a3453c23bc840ff6d4a65fa53d620558d8b6837c9f4e4f6984d1b0072c3fe4050073378c0f', NULL, 1794954003564010743, 0, 1794954003468471593);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xb46fbbd79f555edec21e1caef098a8f476dacd34e067251cca488364f62ce8f8', '\x01e00cbe11f39be3cc5e490c6a15ce2d636db6016f0a780d87ceb1d2c1cfb64573b9985ea63fb56f59898a4ae6', NULL, 1794954003573057681, 1, 1794954003481271159);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x1d278256dd71e36dbe855e267cb976a1481c19dedd7e6ec854d866d2ff10c092', '\x01b89f533d630f0c49e469976fa2f8fbbebea97449d9b09b1e6df5ce81ec1435447da3a38fafaeed223aaebf84', '\x844cc5c80c49e92dc4a3faa705090bfd', NULL, 1, 1794954003491067819);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x3dc77f7d6bc24cf8f30c7efbb91adf8723b0f97438eb729ebbdbb06744e69499', '\x01e2408de37e5d4fb257866d3e6cb3f92bbb96703831ea5f68b638bc0be4799f7b6c54a8244b0e4f2a74321e85', '\x4dbb6f36e1dbc94927ebd8b70378e228', NULL, 0, 1794954003626010524);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x8de033f56cac3feb30e66ba4ca3b8e3313b5faaf114c38b0a2503b473b89b722', '\x01087f72b53374000f37438757aa2d6ecac4f3a664978cc5161db3580b5889411fe28d2e6dfb3d1694dd231c26', '\xf65cad1eb1b8e7b320712fc144555413', NULL, 0, 1794954003635977285);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x3df9cb539f8422255127d20c0f37ee61ed90d7b2375761041325f06b26fb72f5', '\x01447485c6a3a39ba7b490a66455d07361df3155bc962c109447f03747a3731462b32963cc488c9b293cf9ec44', '\x5c53122e0b815ba4a4aaaad201ca9adf', NULL, 0, 1794954003644636001);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xb289c7efce87945da03e46c2114b89047b1944d88b6ac7cb151a2ccf2ea18916', '\x014493193f5b52e6c69fc8a0f1fed0c22f70450eb7e4f89895639f0c16c0be94cef255808a9c7c380c83b7d942', '\xad764c9e0eb5c5ade499ea928aa142f1', NULL, 0, 1794954003654300495);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x5c3aa2a97b4b13fbec80533d7bb04b343766a2f5069b21555e94401e9cac8c59', '\x01a095369c4d3f76d5c90eae189e9d347fc0043e4c4fdd765cebd69da3f9c339b8019862d650cf4bca4389949c', '\x112e8fb448ac0f695c3caa60d67997ec', NULL, 0, 1794954003665379663);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x5495f529dcc575a6f95c85fe338084d7aa2844f7b6b08e8d31676c3239910577', '\x01193c0c0dfe4873bd66f90835083ec7846b11a462f195648e09a4f84ec46d99808bbd81b8215e41066eeb6163', '\x5e4f5b038dea4204c2a17b43ca84987b', NULL, 0, 1794954003675598952);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x1158cf4a54e8322bda529ab74c63fb3b83002cae7a25a2151c14e53b6cca75ca', '\x019c042096e44249c99583b58977f749e35236a3a4b1f4fc9a1fe3946800b50fff218847f920ad3fcf491dae60', '\xe2daf98563a0ed42bcfcf7920282c7a1', NULL, 0, 1794954003587862439);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xfdcd8f3af7fb48b1cbb85a404d268bb5327f9f9e25a2928a2f0249aa07732487', '\x0152e64b87c5fd0a770e06c702da6ef18c7d4a684a50fa71920473d1fac9b3dbcd609a83291981d1b03801d458', NULL, 1794954003719632255, 1, 1794954003606285250);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x3b48b3433a5f46e38b3a09cb091894c71494cfb9e4a93e0eccaf75f93a0e1238', '\x01dc58618012e35c5d35a6c798ce67ac0e1a1ae61678500baf744ea989485a84307789fd9be4b12292c7c06366', '\x2ebe8b1628b7682129b1fa0afce9604e', NULL, 1, 1794954003616266133);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xc11f802d7a457d117104efd95ad7122b9a21d2f82bf3fe3676c0329c49d9b727', '\x01034cd381cd10b8e625c06bb41288011f1bc04fba5b2056a146ba6c44d20bc8a7f5507863d4e394cb89a924b4', '\x7985cad5a50c2a1323dda4ba17eda385', NULL, 0, 1794954008474790375);
INSERT INTO vcon.vcon VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x22aecbda0d10459b12c6896f847cf08870f9ddd48e039d7786c8142fedabc3ec', '\x012381aee6b7c48d30d73a770a295bbcd25e45bb5d3ce8e5d14aa8e2435c61246a6f4bdd3a8c8197e523ea2540', '\x36dcc5d80e44b33049b7a50861cd68fb', NULL, 0, 1794954008483911887);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xb71ee5d2753b2751fdcc4b3ff337feeaf18e21193012c292b0d0ae7b03d8f5f2', '\x015937d678f451a88eb41d8c61dcd437f3905015f1f25bf2175825491e3ee3cbafb3d40efd9a6d8a3c8e65985a', '\x465560473c25b104db3a6f9f21a02a51', NULL, 0, 1794954008492103975);
INSERT INTO vcon.vcon VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x4f3bf9260773184d785f094872c7527e99b573cdc68452ceae362e6569c4bbd0', '\x018ec1c4a518bf1824347fcb64439cc96cf6c20e86a724ae66ee8062d2cf8453b596eb8047ddc69a8f96177a12', '\x2606bfee5bfcedeb49ffdc83310e73f1', NULL, 0, 1794954008499961025);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xfc43ce460265daa4d42dd8b68ecd2bde', '\xfdcd8f3af7fb48b1cbb85a404d268bb5327f9f9e25a2928a2f0249aa07732487', '\x013d39cb8332a2d9b6d4e6f7b050be4bd337fc2cb9d7fde24328b69a4c552229d82e298306', '\x010e949448ced2785311ddf37b69f84e8cc1ef691c0ac0d330a22dc35450', '\x01f1e1c7b08d4a26636364a6e00e2bd0a62a360292c63ad8c82bf6f19a96e7dc5611a81f48', 1794954003606285250, 1794954003719632255, '\xfe9ace24e76ca768', '\x0143b929f909cb6a6932573e3221ea6f1d6c8981cacb0cd8038d5d2eb91e4dc0fa8fd12ec0a135dd2b1f7a78574ffef80ecc7c1d0cf912219f1b5063f7', '\xef06dfd75568786396571e4d47ab11c0', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x60e1b99da54ff00ef7d7358ec3b2e7f2', '\x3b48b3433a5f46e38b3a09cb091894c71494cfb9e4a93e0eccaf75f93a0e1238', '\x01de77a1b24138c530ef288a06cb0a05ecbbca5917aacb47b9703aca0fd14befd4c6dc8ddf', '\x016d0831ad06e74f2b37e81f53248c749985c9548d1dbe1ab8a4ce411ce5', '\x01387b7f328ea7e31b072018d1533274fd7781b968a340d29980a7840745037ac5f8115d41', 1794954003616266133, 1794954003737087845, '\xfe9ace24e76ca768', '\x01cddc566760d4b62d8aa2d1c7438845e8b5184bf0ff319bcfee1dcdb92f942260457ceb5a64b8e73f18655567e4dadbc2b643b494c153b3b07f2c1c82', '\xef06dfd75568786396571e4d47ab11c0', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x4dbb6f36e1dbc94927ebd8b70378e228', '\x3dc77f7d6bc24cf8f30c7efbb91adf8723b0f97438eb729ebbdbb06744e69499', '\x01c884c094e001ed6c61b953cb11f7738d8cc983b12e21e923d163621af0db783ac2824629', '\x01176271ff8f1fce4dff4979fec162f01ae38225e24adaa9de2ddfb9a9b2', '\x01f3e92cccbe8dbc598e6bc435f3f96b1e246a41d090c034509e76d6358a4e76345a512b5b', 1794954003626010524, NULL, '\xfe9ace24e76ca768', '\x01ae8d3c5956da2c34f68df369afd375acba08530deff734bb0aa1c2cdc5670d52e13e39f9f010d14f979d09ab2a7f747f25c47878e16cc43355b3117d', '\xef06dfd75568786396571e4d47ab11c0', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xf65cad1eb1b8e7b320712fc144555413', '\x8de033f56cac3feb30e66ba4ca3b8e3313b5faaf114c38b0a2503b473b89b722', '\x017feed85478e831c72a4cdee750a8b1b194d5b02819cf11065a95d35b09dc639def65dbf2', '\x01cce16d29c5ee954e7da188b335be6e86b6527dfce4e7bf6f5a41361456', '\x016343e071a797a98d65f74a1d41a7e58ca5269604c7477e4ee07f39ca8a0569edf323da33', 1794954003635977285, NULL, '\xfe9ace24e76ca768', '\x011dde2c266ef82b620dd1a1af9b269101a19bab5c7a70cb8226e1e5f037c1bdae26ffcf1cb2ec76d894a004f655b7897307ec222ad6897b17dc34c3a3', '\xef06dfd75568786396571e4d47ab11c0', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x5c53122e0b815ba4a4aaaad201ca9adf', '\x3df9cb539f8422255127d20c0f37ee61ed90d7b2375761041325f06b26fb72f5', '\x0131653e49081a090e826bb984ea2f9a0e626cb8d3c5430da0833afae007371036a57341fb', '\x01525f8ffb3f86d9be5696b30f5f34ff34635903e174e525286fadb6716a', '\x01a86e890b21c6d248370a0fb007d9593180ed857e17ef2e5edaa0a4c11a516aa2673e99cf', 1794954003644636001, NULL, '\xfe9ace24e76ca768', '\x010cc06db5caede97cb5b89539df17038a85d217b41278f7bba1edcdda2329b97645e2534e220ecbfd15d6059191e27fd4f00bffddb2b18fe564bf7e15', '\xef06dfd75568786396571e4d47ab11c0', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xe75955cab8eb987c380060e6c513902a', '\x042b4d175f7f88045d6c760f4fab5099f08f7f24bebce369c7e1b77596d5276a', '\x01b046569c01a6b413062cc954d035caee28d75b43508c30489a299b2c415e8d2bf304079d', '\x0126acb9f86b34a5288e5b67cabf4dd68cbe1aa2dfed458ccafa6620f62c', '\x01b12273e9c467886432d03534e7a96a546788071c48023d381abf3237b8d8ff74a2edc6ab', 1794954003443880887, 1794954003552185028, '\xa193accc4e020b22', '\x01f9a6de43d380ddb0befc7ac8d03d5f85cbe4b16072ea2c53a8e374ce6c924bfa8a58642f9fe8817de3fbe836cc0920c9f1d01642967ca0a050a4a172', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x8406431664c8e4bd85b2a9ace2ed80c2', '\x90c63ce493f2733dead9a8f3ba29b01a9390860c1a071d520810a049637282a8', '\x0198aa3efbf6f600693aebb064a02609c7169568dd1a22598a18d09c58d95a9f883e4d477f', '\x01b15c3fa2681b27731ea6f929295ded888a6e73ee231129510b09fbdb91', '\x01ccf0c87b62c77d34cdecebde60149c024d69867b0259c599f789c652058b9f3a47951fc4', 1794954003431393905, 1794954003540736564, '\xa193accc4e020b22', '\x012d54c6e7a095383adc242dba0e72b345e01e28c11951f6762f13f58d260b56413135a496ca954e1f8ec7344c99af6cbf17b10a98584af0e9e2142568', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x990784fbafc63a63ad2af8b65c2746be', '\x8b8a7852f1c168be30c458ab567158f2134989d31ff0ff13e330fecb1e0e3554', '\x0135db67f7a4464eaeb4157ef5fb4b3a41deb8da29a9bec85146ad0d3809b1a03966b23a9f', '\x013a97f4297c8f95bc12680b0d5f50ee842da4aaf9ea1fa31a0df35f2175', '\x01106359050b3491f5e5caa0fa070ad28b153420fce2e8c087efec9ff176996f326dabaa9a', 1794954003456804776, 1794954003557022618, '\xa193accc4e020b22', '\x01fc08272d2877b2d25adaabc0fb3e53a0a8372121a6eee95568f130c85ad839ad69eda862be3ffae7a72a256c969e2683315793ef621f1e57b38043bb', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x4fc3a4db762576f3d5ea6544704682fd', '\x874eb6fa34fe2a709abe2141fa6fea2cea66fcb0be3a8419cb86f3e06bbe1541', '\x0146fde0f8f90513e2266d30515c59a1286045bfe959f6f60680f01c43d997b0815aa26594', '\x01e6ea46cb7334a58a55945d43ca284a108092fa7ef057c325965d2789f5', '\x01cf815226066691eea8b5f364bc1a727857777227b8a1fd7b422c81a6e3cdb6ba8f1d2989', 1794954003468471593, 1794954003564010743, '\xa193accc4e020b22', '\x01f6927d26ff206dfaa1c3d46205517cb7c786c41a7e7b8c79a4b01b8293f1fa11dc91eeb2be2327ea3ee6eba40d884f11cd8e77d14269203173ef537d', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xad764c9e0eb5c5ade499ea928aa142f1', '\xb289c7efce87945da03e46c2114b89047b1944d88b6ac7cb151a2ccf2ea18916', '\x0104faf4ff95faff5e934bbb1fd8f6e84fc3e047fc51a97b12d0309226ec82336cf60e07a8', '\x013aec58f406d275c5347f3a79b9432dbc6a298366b015380b5a439cf12f', '\x0118269b8eecd295e080c951eb000465e0b727ca1565d5bb7e52b0d9fff86f838e9cad631a', 1794954003654300495, NULL, '\xfe9ace24e76ca768', '\x01b07d83c9a8902b868c06403246a9cca3b8138518d4faa2dcc2fec2be35292e6ffc31ecc28df658425e21c1002bf4ed73c549743c7feb81fb850a6c3e', '\xef06dfd75568786396571e4d47ab11c0', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x112e8fb448ac0f695c3caa60d67997ec', '\x5c3aa2a97b4b13fbec80533d7bb04b343766a2f5069b21555e94401e9cac8c59', '\x017f4ad26e73d884546ffbd6bf5faca5bc549ab4d90cf6fc0013a770709d6221fd5d1d8b06', '\x0140d0bb500be202b01efc03dacf73dc9f760590f1ba439dc0bd4317a097', '\x0166602d5d7e12add2ec97f784ad592c612b398eb741180aa91967d6b9654a8b5214f90877', 1794954003665379663, NULL, '\xfe9ace24e76ca768', '\x010288be46fb959c0c550532a77124019ea2d2a89604900ca0ecf6446daff9a498a4eadb6b7ab408bcef2994cb25f881535c12f1605b2dc6a31b4a3e84', '\xef06dfd75568786396571e4d47ab11c0', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x5e4f5b038dea4204c2a17b43ca84987b', '\x5495f529dcc575a6f95c85fe338084d7aa2844f7b6b08e8d31676c3239910577', '\x012dd4c23a2262fe18897e015e5c6e3d6fd8286890dc7033e1f315e010a04d00c1635b6ee0', '\x013f0d0c4764e967e3e3768388ce748983efe9267cb8b79d0c3f7bcbde56', '\x011768bc74aa7e4e8022523961f96a9f2d3aa73766726b46a591d7797b430c6057a5e40176', 1794954003675598952, NULL, '\xfe9ace24e76ca768', '\x01c8cc0bc605141eadf21f78f9ff3f91ba10d5f530a500eabb8c9e14eb8a4610dd672f8cd9e83c2b3391d9072cb42ed783ac14e14f50dd980a95662571', '\xef06dfd75568786396571e4d47ab11c0', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\xe2daf98563a0ed42bcfcf7920282c7a1', '\x1158cf4a54e8322bda529ab74c63fb3b83002cae7a25a2151c14e53b6cca75ca', '\x0168b34cba1fad554e516841057d4d1a64ad91df33822b8bb7da602873fcaa495894d15775', '\x012c2328b2de1e4fb519b835b000fded977245da26d789aeea8177f385e4', '\x012d17717f6529c169ec05d1882f7156e6f454581e3628ba76d88f3da6549f83c7bc716715', 1794954003686000130, NULL, '\xfe9ace24e76ca768', '\x01f6d21511a41c6a71130b78abd09a30932b85d01874fd4b3a73dae47c6ae249292baa2d8298381070e1da1886b50c07e5e4ca96a29a509071250b3619', '\xef06dfd75568786396571e4d47ab11c0', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x2ebe8b1628b7682129b1fa0afce9604e', '\x3b48b3433a5f46e38b3a09cb091894c71494cfb9e4a93e0eccaf75f93a0e1238', '\x01468b304891e449fcac14a9242d7c7b1bd5813a4a7d732217d6dc9b62857222a4f2f85237', '\x01bd4e6e9189e8b467abe6bbb33f1a7238098736dac4374b00b4cf35e6ed', '\x01343d8be6aab7a181426be039258fb0a1b30e5df43cfc9fcc2fd627b935d3925720b1320e', 1794954003737087845, NULL, '\xfe9ace24e76ca768', '\x016e2a14dec4bb50568a6118e8ff8ab0f802d81719287d3642b6cbee801dbcfd9ba023e4246dbb3054b27e17f0d377bb9d4afafc027a9cc173accc85a5', '\xef06dfd75568786396571e4d47ab11c0', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xbca3da17573568ab81c01e78064f17c1', '\xb46fbbd79f555edec21e1caef098a8f476dacd34e067251cca488364f62ce8f8', '\x013696778c45509058d71f3d8d33f8fa2c4a5d20e17c1ccd42d0b80298948c154227b3cb04', '\x01c62c3e9922bec6d35206af3074cd65caa5dda612ea6c86b36dba25ac54', '\x017eca0103f145b5616d82a0ce1fb2c2e63b9b95a59dccf37150aeaaedc63d2a57997d0583', 1794954003481271159, 1794954003573057681, '\xa193accc4e020b22', '\x01146deb1e55effa5f48cc185036180f266cb1c6bdc1a391380d6de849246b371dd3fc3f0f68311edcd5db57ec81886cdc87e44bc89d5eead2cad9d27e', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x844cc5c80c49e92dc4a3faa705090bfd', '\x1d278256dd71e36dbe855e267cb976a1481c19dedd7e6ec854d866d2ff10c092', '\x01cc074f9e7b3be0dcf03d363ff661b4ce2138e1c47f281b926dd69bff014591154dd20d4b', '\x017ca203638c258254bb28629e8f4b15b07491e2c389dd7bfe059a15e21b', '\x0196d0c2b3bbe5bec9528c8b83a619ebe47c3a03aa5d48d47ab3fe5fa1fac794ab17280621', 1794954003491067819, NULL, '\xa193accc4e020b22', '\x017ae036026d951fc37d713b40a23879e5f5f8ebfad2befe7b83a0b516e84aab67196bc0e3ea7fe4b342371024ef8d0190b4121c0fc362ae76966425a7', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\xdfff8ad8a26ffbd54c2987f9c90d8cdd', '\xa483209f7185764470eb6e3086d2c98adde69947db507e39c63d4b70657e4f1a', '\x01152cb139bf26fd684e371eaaf6e312fc2fa40bbba1999cf57d5e7cb0152bf25c278833cc', '\x01704ce07602984df601aa1015fd90a711a6fb9f23d02bfd68fb331e3b75', '\x01b338bcf3d2cac97e076bbccd1255a47626fe6a80cc06d685579b160215225553ec061224', 1794954003500740304, NULL, '\xa193accc4e020b22', '\x01b83d324e4bb0a7864fb04817f4b36db41d5f0ec39eaa5907f5ce2b9aa664934c5e78221e7deffdfbce4ff15c06b601eb6b54d3dfab5489490ce11fea', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x2de52dfed21f9b52c96c280d5b7ab986', '\xd5dfcd6bce81b0cb737a8a5643e01d4c345700caa7a515f928c5bda8d1f02cb6', '\x01d0dba25ba2168c608747788fba5765ce214ca7ae89cd1a76be30341acf569628327536bd', '\x010a4f6de19ba2fb2942b34b106811562a0c9f116c6217a5eb6aca0ae501', '\x019d6bf6cd3315049fa4020cb13a3da0cd974d77565ae4fa4212885ece0d80af250b706271', 1794954003511162279, NULL, '\xa193accc4e020b22', '\x01f235a9d4f3575ae8b2b139b9dfa27fb527b92964d465fa0b53da090129298aae15c11e35d98a0d648cc39dce9f1318a263be7a6d0fb79cc524224b62', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x8f8f9a54492794f5f01b35778cdf6b81', '\x5211f8088c99a61e3fee8b79d300f7e0be7a86eed4b13e144a7f8fb55fabe446', '\x016a21e52066596f54e0a158b187bce2503b7c742e4a2e2700d4f80d722af93e4bea5562ee', '\x0111f1d05ce53d6c3806d63894adb8ad530a002d4ea651f09255b201c711', '\x016e317397e610fd839b78d885acbfa5782b7f342ccd2931b5dad51bb81c818a5c5683f147', 1794954003520627920, NULL, '\xa193accc4e020b22', '\x0184ad6b1f0d25d69f93a7b1aa68fdfb64f9ddef9d171c8f241225d59286095adc97418cbe6fede11fc0361d371478f64ce4659018e1a89bb311ae8667', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x5caeb382d942a443efe1e5b8c8efed15', '\x7737373606ceff1c2d3f7f2b0cc9f292b7e4e6762bb2044e2c121be2f463f622', '\x01b507fb162b24fa9613dc309621b7cfde136afb12d73b85ea98567c8dc0d94631322d9d15', '\x01e4506c9695e145b59910d97f41f076caf14b28bbfcee4fbe774ec9991e', '\x01d0970cd2932bc3a045f18882af53340a96748ea2edc6e5950f4618bae4e5ac03cc62ae01', 1794954003529888914, NULL, '\xa193accc4e020b22', '\x018a60a5bcb632ca4b56525fbecf3751a66635715687e701e1fb42b50e1de5e1d3214e022c83e3316c7114989d578ebee6b7b8142a99e301066c66bf4c', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x6d750d195d6618a79852856d6986ff01', '\x90c63ce493f2733dead9a8f3ba29b01a9390860c1a071d520810a049637282a8', '\x01cc641778d717842517abbca908312917b2d6dbf6d64690b72c52ccfd995acb662579fd60', '\x018169f13518ca4854b25cc2cb1e649ca3fdc3d6e097ab8e492da05f1e8a', '\x01eef7269f65635ed2dfc4490bdca1f637676fed1cc889ca6040946ba5d8dd927626c4a978', 1794954003540736564, NULL, '\xa193accc4e020b22', '\x017982735c80b25de0aef149114131da3016179edc423e858816d67407bc97984c5a3e97e568c4139ecbaf970b87f7903f0971a1eb1f933f834a54e58f', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x792d0a02b24f8aced766b943c48d8961', '\x042b4d175f7f88045d6c760f4fab5099f08f7f24bebce369c7e1b77596d5276a', '\x01d19e530faeeb3f9f5c2046f0daeb706e7ba2c7481698b590fd5cba3b1c6173d59f0b2de1', '\x01c007464c8875fbbd2b22d1a903b83629e608050162b4e523dd74665b7f', '\x01e037fbb38c59548eea7d9783997ddf93d3e6cfaa907cbe0ad47ffe8bc79de10beffb4c47', 1794954003552185028, NULL, '\xa193accc4e020b22', '\x01a10ba85502a7dd4dc5ab12b5ba55b48fd9514d8c290fc10e4d36271506d72fdd0be467aea973a6ce8b7e6b3e37bef6e1584e1a3d32907c78f9a09801', '\x23b9d7fcf7482133d623422f86d3ca8a', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x7985cad5a50c2a1323dda4ba17eda385', '\xc11f802d7a457d117104efd95ad7122b9a21d2f82bf3fe3676c0329c49d9b727', '\x01713e99c862327944188cacad0d80dc4cde38a52e8cc9316f233974079c491f896589c64c', '\x01881d95dfde073a724fd33be974ead9469d410880be9c5e4867b4fd625d', '\x018c17b11160e877f8f6dbd69a63d7879d6577d842295f2439b507aa60edb907574dc34070', 1794954008474790375, NULL, '\xa193accc4e020b22', '\x017e8282d6c8982b04b59ae149458e47ff3054e103478c3c005e2ce1085fb2621323bece9f2a12b72f3d7eab9f58894946998179fa8b4495ea5de8f308', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xd62e206ba8f0764b81f3ca024bb4d3aa', '\x36dcc5d80e44b33049b7a50861cd68fb', '\x22aecbda0d10459b12c6896f847cf08870f9ddd48e039d7786c8142fedabc3ec', '\x01a2bb385037bd1a8044c1d14b49268ddf80a87dc662f09c7b0010149de68cbab1e2ff532b', '\x01d1b48f0a974a843892dd19a4c44d9cfb98f56b59b211b726188ff4b57d', '\x0169a7df13af6bf1feaf4055b2dd3680654ff0511f906f3891d8d5060418ae395106a64196', 1794954008483911887, NULL, '\xa193accc4e020b22', '\x01ede8367304a257bd512424a19935a4fc49b6006060d8e671708cd92ba919b0ac92c3b9b4821ee7c26433d9aafaaefb996ef547d8f4e0811725d9e8ff', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x465560473c25b104db3a6f9f21a02a51', '\xb71ee5d2753b2751fdcc4b3ff337feeaf18e21193012c292b0d0ae7b03d8f5f2', '\x010eb2fc5b4b2edd98e0b6313d9cdd2beaf91a860a9e10ecef04f0ef9283a1d90fde5f8522', '\x01be048a42934b7ff4a252c4cea7c74aa47a87d872068f675b00d5d79ecd', '\x01971e81012f49beb0a92a596a1a5e86a718af67bab9b7318f43c966c4e40728dbc3cbe493', 1794954008492103975, NULL, '\xfe9ace24e76ca768', '\x01a10d70633e8e96536a7004a83e58348fe8242962f3a5ef03245016c07c7b856b073322b636acd783b45845230333e2fe29957ecd2ab81597921ecb19', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x856adc4789b5e383dc8cc0dde06e73ff', '\x2606bfee5bfcedeb49ffdc83310e73f1', '\x4f3bf9260773184d785f094872c7527e99b573cdc68452ceae362e6569c4bbd0', '\x014a7146e1fd199573ede9074fb5a55a38cd30b73ce017b74b5910900bef985215c6d5131f', '\x0140f3e1f964b2dfa20864721008cd64e1e0f480ef7f3b995b3fa011546f', '\x017720db3f128f683759db1a50fe2647bc809e81ed20f879bc6180b9b812d31530eb1bfb72', 1794954008499961025, NULL, '\xfe9ace24e76ca768', '\x01d8e43dcab4df450bacdb4859b9036001b6d3d878c07edc712b7a018d216b12ecfa540d0826ab4e9bcd8100029ea0b2a8222ffe21a34ff1a10cfdebbb', NULL, NULL, NULL, 56);


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


