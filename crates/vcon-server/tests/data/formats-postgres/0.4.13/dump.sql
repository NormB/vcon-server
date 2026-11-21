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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-21T15:17:53.344398759Z', 'cli', 'tenant_create', '\xd6c2663152389b308059ae7c3a3c55f890e26f5ba51565efbbf03c6f26f0e19d', '\x010bc8ae6af31f017c859b19b7c2bcae6ace85665b6b11d9fb0797d470bc6c56a657edc44764311b76a590a9495bc40f985f92c4852a07bd4ddfa52592', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-21T15:17:54.844259533Z', 'cli', 'key_create', '\x8f06f873016bae7da5c4014c88f1fb768abe0fe638bd95527c9b2e46cf65ded0', '\x01ec2a56dae7993c66a71e9ade8fe795bac678bb2db2d62951045960f55249817bf5fedccebfc8d673fc22644b', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-21T15:17:56.49675612Z', 'cli', 'key_create', '\x884822216b895a5d27b436115cc2fe6d6ac44caf6789d5c975aea811f1479117', '\x01c12077b5e38f97b6901461c4296f94f0c1838f6963a42e3d60bcc4e54fa80067a487465ec1a2a7706f79bfd5', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-21T15:17:58.137539148Z', 'cli', 'tenant_create', '\x17c8d27c7a232780c85f9e83662d8f639c86f185cc002dc8632a003f5d0ea4e6', '\x01e9901c5a869f4092be22ba3330573130f98deb3ef8ef1ac5f7ade898388d9e1554fed9234af748ff0c4093b2d1d724763c93792743580560dbc924ee', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-21T15:17:59.662557852Z', 'cli', 'key_create', '\xfa0ec3f001170dee809cc19c22ce503bb6ef0ebc09dbce650b4c387e2372c272', '\x0135ee5d0fd059ce1fc7ed30c0ea65a30d09fbb1e3627554237b7eab3a06be57620be62aba60029af296667c55', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-21T15:18:01.277569553Z', 'cli', 'key_create', '\x0c6e0fd636def01246c4889e395392c5dba6fe8c014ac310a2bd1c2f71ff688f', '\x01c320b709f021475d52a4f9d5c09147bbe65784f0b228294785d73f18281832bb0384374022e898cd5249fe77', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-21T15:18:02.823350331Z', 'cli', 'key_create', '\xb86e03733f76465e3d0c2ac3894046ef076623289650552b46150cd7f68248a5', '\x018b43511a52e6df6d224d5f9d8c525ef855b80b33245da9b97f93c2b2f82e270eefcb3a2ca5eb7649967a747c', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-21T15:18:05.377562866Z', 'cli', 'user_add', '\xa3e997247d70e02fdaa71e53c67d3cc98f86f7a25f1e3ef28e05a6ebd0aaf50f', '\x0158edc99060382581c26cf3a0f43fefba11daefe2d680b6f953aec10371b9fef174a378bb63933d052e6cb83a10706bcccdc815e70e1863c261fcfe72', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-21T15:18:06.889711111Z', 'cli', 'user_add', '\x8fe9b8c7a98fce6e1417107e52b9a48d9a504ff67bf284c3ab3eb3bc86086ff1', '\x016de09d9993b33dad64d25f28b52ea8944107a12fee1fd5a693bb26dbcfd039398220e01de1ae84ab35374d1221602369c900eccf5a3b57c95e2b0fdd', NULL, 201, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-21T15:18:08.399259939Z', 'cli', 'membership_grant', '\x926e6174c9f43d95fc4b64e4853fa57051478a1e515a0a51e7195c3ad3b65387', '\x016bb033730e9fda2b1f260deada58a376d6e8305bf221b0e5a33fcf8539619f3c4b403b0c8a12490b224f19f4ca7c0fea606ec762945ff9dc312bb6b325cdfb8a807e8852f4043caa7844884686afb7967af24b8689391ff3e895124b203056050da0e8cc', NULL, 200, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-21T15:18:09.959131559Z', 'cli', 'membership_grant', '\xf0516a1ba142bb82eabaf0e6d26e7d42d30cbea0c9f655be244d43c06d293b01', '\x01dd373b791d39517bccf7bbd0d540d9a0b634a620c88725e852a6c5ef5f688445a3ec4311fa59b252f233cf746c743f471a9ee8cef3befc8eb1ae6efb362a0b2b82ef1ef6d6dc63ea0d3c47e069253ccfa9ba401a929c08c1181bd05e9e88f6c7f5b4e408f4', NULL, 200, 1, false, '\x51ab9046c8072148');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-21T15:18:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\x51ab9046c8072148');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x51ab9046c8072148', 1, '\x01edbb7e2c892569a0a5f09d83efc5b1b479728d02514e373bbf0629ade59beab3efed8c763eb0945684e641a49181a6393cbec69d9f4a8000b61d0ce8');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x666e95e009106e00', '\xae9209d8c518001139b09e3c6adb06de', '\xd9d631d6520e7c63f43b8026e2f92b9dc032bdb28df00f5fee1f155bc132047a', '\x0169de909bcda4d6d8894c94eb115ae52e0045182d30f52f01e74d0358d63b870a', '\x01fd6b789a52a45c5635be461dd18e1c9ecec35743d89bbb31efd7513c001cbed65d7a94983060b68a2c61fa', 1795274276496363718, NULL, 1803050276000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x55224885b32147e0', '\x46fb33e90c78285745052227167e8d3e', '\xc1ef7d2d422dd38065f3741572363dd896b6773d03dae49ff3a04ca06b0384d5', '\x01051dc443defa9c68b94119614feffc5eff70e7ca1a82ccc121f9e120d7d17348', '\x01d7e04e258738cea2b4a595a9709eba7a4216c9c43a7c4dcc2d4f079d381c9ba23e58d0c10a468679ec860e', 1795274281277181429, NULL, 1803050281000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x587757f8d7c32411', NULL, '\x64928903b4abb1df45477a497b8dcf869014d6eac68e7b5e17fa2d30a2fccc2f', '\x01398c00d0b86ffbb8039c7614158fcb50709d22e8c7474ff33994fedadd1907f75b', '\x012d17dd9ebbcbb7c5e4d73fbd54f7d4d700ea17582bbb313dcb39c25ea8373f', 1795274282822825853, NULL, 1803050282000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xce9f8f624308dc04', '\xae9209d8c518001139b09e3c6adb06de', '\x14dbf6deedeb139baf411cc57f22ac7ef0507669c2a0f367a97627725baaac36', '\x015518c58ee589b3eaee49ca15dcfa2a6d24ee840cfafd46f7255b03f8dffb4255ae0e3540e12d0e04fa7c853960f70323f6b8115173e30b756b', '\x0135a3a39e4c158c22ebb4773eb7d8b9f7f22fea1c0c15cfae8d59b9bb08512562b3d14a1ceb27f7969abf6a7d71', 1795274274843705120, NULL, 1803050274000000000, 1795274280000000000);
INSERT INTO vcon.api_key VALUES ('\x7ec3b21389de698a', '\x46fb33e90c78285745052227167e8d3e', '\xee427c399821b92ed22877c924d0419f8bc8d5735a26cf4518e4334dd60b9c1b', '\x01f704983247e1d62e64b4a89d318a695757cbd1d697119ded70552ad1407823344e53fcfc696cbefe5222099d78c7b3afe386222aa62cf71f43', '\x01b60c8264a469d76774dd771e0a4431b1ad59715a8bc591a3dbef4103d2cd7239d5086e2ead095f9dbc2cd3e6dd', 1795274279662157072, NULL, 1803050279000000000, 1795274280000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 2, '2026-11-21T15:17:54.847385191Z', 'cli', 'key_create', '\xf4c874357347dd9fa6fe54a1841f732f5248f001b2b3bed79f638730ee86486f', '\x01144fc6becc21b3288a889d6f77051c11d32aa5483bceda4918ca428505e1f13e73456cf9d5b6ef201f699272', NULL, 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 4, '2026-11-21T15:17:56.49989801Z', 'cli', 'key_create', '\x25902f01b8d9141879ba17dce2feffb5d5fbe11a1426e422e08cc465e3340ce1', '\x01c3d092766d46fdf98b19f56a49d9e4376cfdd39b5f1eef3f96dfba12bca9e338b6931aba1b61772ddd187009', NULL, 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 6, '2026-11-21T15:17:59.66572029Z', 'cli', 'key_create', '\x7464817bf7ec0ea22901021deb6f0382625db54cb3b3d902b764bfe54025987b', '\x015715b58fc0aaea58705061b02cca440ab0c3872c57c89436152ac1a2271c690d9e45fd21ab956e0105cbd73c', NULL, 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 8, '2026-11-21T15:18:01.2806806Z', 'cli', 'key_create', '\x70b3ca1457c98200c4dacf678fb75b07ba2cd842780f63de214be35625d1fe23', '\x019f75d0291c414eb2a5faf5a943717e7b5208ba1b6f0f0d88c675f93f2346547eccbaf88f34efdd367d4254be', NULL, 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 10, '2026-11-21T15:18:08.402532774Z', 'cli', 'membership_grant', '\x1dd3b67cee355de529de0dbf08a2b955bbab1dc947bcad77a1078de55620f546', '\x014fad1819deb94875274f00c69acd68f127ac07019ad21f7030ef0f9f62c1c43c3d8accd0f7248fd03848f60e1702c5995f5172635c08b1833284605c6158257add231204cc4bf3f876ce156d14a27941d448323a6b36e27cfe468d6b432c563169be05c1', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 12, '2026-11-21T15:18:09.962392663Z', 'cli', 'membership_grant', '\x4e03b7fe9c9c386a0e4489c31b76458cf42fa2ef7a18d1de6b6b97990829f67c', '\x01d463d20f3070703c867227d1f1503657b2517a83cf87f510895e2070bcb9c770bafe46af5f214d87bfb651bbdfd35a61456f4bf644307794587931163bc6b02a2f72326fec0fedbd159521c100e5a6aee0b5b72c7940b527cab73c2650c63f15c772582b1b', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 14, '2026-11-21T15:18:11.959707471Z', 'ce9f8f624308dc04', 'create', '\xb4a7d55ec8e911552208e6613ea1444d8a2aa3fca1dd79ba3a1f5a2ebe452998', '\x0146a396d4e39f7fb237bdc3c304935cac7e6a431ab9b75f8bd3ff76f2341467d1fcf25e8e2ec29c2380e1d8bffd68f37f009e5435bc9615e007afcb0a9e2d5e8c', 'd77070b1f61f4468151bdd57c49c8179', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 16, '2026-11-21T15:18:11.968992767Z', 'ce9f8f624308dc04', 'create', '\x72c267245d5917c6ec04a8e216d7c7b573911f9470c02b9d5d4f21e89b0a435a', '\x01198ce043b3ee38c85b0e4760de63ddcdc5cc2935f4934c3797b170953eae432432da248d7b7a8de3efb5e16d9d4fd5404a929edbeb6802fbd6aa57a4200b799a', 'ac0323f72e2f01a6c48243679229ddc8', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 18, '2026-11-21T15:18:11.977468826Z', 'ce9f8f624308dc04', 'create', '\x58378a0a58298e5049dad1c3069406b82cedd3da6172a15e60651fd1a5d68629', '\x01a7f15571c859aad3b71ba048a85d53e02fb6fb51a4871fc507e154fa3e2ae1b7453de802df2e642d046f8addc06d6a6e921ed3a465634912b560a69dbc2a1925', '9c6111c70ce30631f2b1e0d35cd2127d', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 20, '2026-11-21T15:18:11.986101459Z', 'ce9f8f624308dc04', 'create', '\xaf38557482c88675dc6fe4873a1e1b8dff03acbaa3da6f0acf280f93061089b1', '\x014e9631d84b9fc3100b28185e098293f29dd5b6d927d2304d0644f031ca16554453e22f38f3f4b06f3cfcb603894f455a27f8918002dfba27529e327d6a72c2dc', 'c6e1dfe9f11fdc392c7ade63ba11f83d', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 22, '2026-11-21T15:18:11.99521107Z', 'ce9f8f624308dc04', 'create', '\xac283844310ea3a614db4dd15b65444be275f6c4515c12efe64f593680fa8aa5', '\x011bfbc35daf47eabced0030414303935bd7771261dfc2374450cc6b31e8cfbe952d5b827a82e046c327d220fa2f48e0fc8e0d041193f32367c4911b555f70cc6d', 'e837e126b62a0a86daf9a70db1fa46aa', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 24, '2026-11-21T15:18:12.00413328Z', 'ce9f8f624308dc04', 'create', '\x9d09ba0dfd231a84c21510293b044f194544fdac6b96a9bab82995a62f2a180f', '\x01bda731caf04aac97829fd29dc9e26828ba3230b5d8a0a06f3b5092c514c776926607836a299e33259df727f676bbb635d31a41016f4d81378d58b797aa0ccc18', '6f5414f3b68fec4d24534211ce1eaa5d', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 26, '2026-11-21T15:18:12.012099769Z', 'ce9f8f624308dc04', 'create', '\xbc013866ad2b9f8241810df4d3275397eb7bb75441c33581737ffd14c1faad75', '\x01540504ad55f333182291c521c5fe5855a938a06cc90241e72e133639fb5b879bfbdadbeeb222b968077ae6ed34419b3eda7425a16eeee76c2911a2f8d3215acc', '13a86df30049927d2fb0d66c7b2a2110', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 28, '2026-11-21T15:18:12.020566031Z', 'ce9f8f624308dc04', 'create', '\x4b46dd6700ea57702c525d6259516fc26d23999f83a95750494e8463fc84f58b', '\x019b82c89245ec5c3cf7b6d000bf4374007dbb584f24286eae00eab0e5c99ce3177848222429b7530e31acaf3264325e8c6322ec3c8c4dabc22ded42967f0cafc3', 'ab153bd041a333f65664b100f1eb387c', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 30, '2026-11-21T15:18:12.028589752Z', 'ce9f8f624308dc04', 'create', '\x69571665c08c655d7956c39f22df6a2d2d2436887f47f403a43a4a8d68a166e3', '\x0107cc60139c433f9e9c3b5124134de05088a2bd8c43304085a63496c0bfcc8163cf246c610a67851c7df74000f7c6ee0bffd163cc618d9fe0210e6177739e884c', '449f0c2a02ddcd495bb011f7a9728801', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 32, '2026-11-21T15:18:12.036542277Z', 'ce9f8f624308dc04', 'create', '\x96cf8f902bcebf5aeae5df28897d4ff57d29b9a3cf1eb42d6f6aff04b37ea2aa', '\x01c270f1773461386111f71067102af375eb57201d81adb7737bdd38496c10379b66e6b4879f4100ab7c3926e5a02b25536b3cbffe6714bc3e22cd47b588889afb', 'd749839ad7da34808e31bcb96f5e28d6', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 34, '2026-11-21T15:18:12.046210856Z', 'ce9f8f624308dc04', 'replace', '\xb4a7d55ec8e911552208e6613ea1444d8a2aa3fca1dd79ba3a1f5a2ebe452998', '\x01e4aaafaca5dfbb9c45373957f11b28b44f0a7895d6ca7ac2c77915942eaeba1c2a18c69dfb86f9f9cbf01220461e0f26208d3714f7e1a48e7c97f601d4e8370f', '11c22862c9af250a39ed5051cf4ab315', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 36, '2026-11-21T15:18:12.055155974Z', 'ce9f8f624308dc04', 'replace', '\x72c267245d5917c6ec04a8e216d7c7b573911f9470c02b9d5d4f21e89b0a435a', '\x0182ff2e2acbab3a34e797e5d9f8f650c6d6c5f21413e6f8e2b3be0b38f776950e1f2431ed2afa75071149599b5022ca4d4dbc7cd51641e9fd35a5733059c3015e', '19303f037846a48eb00caa0af9ff2d43', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 38, '2026-11-21T15:18:12.058873656Z', 'ce9f8f624308dc04', 'delete', '\x58378a0a58298e5049dad1c3069406b82cedd3da6172a15e60651fd1a5d68629', '\x0137556d2ac0651e9f8e28e1da0243c686af8cde6478f99bc8e803e43b410b85b04e57914eae32510d9b00389cbc1a19903ad687dbd7a79d973f4b34d86fb0b831', NULL, 204, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 40, '2026-11-21T15:18:12.065076703Z', 'ce9f8f624308dc04', 'delete', '\xaf38557482c88675dc6fe4873a1e1b8dff03acbaa3da6f0acf280f93061089b1', '\x01cb79dc75e063ff8fd02caf707f7b3c4dda2a35f2a0aee0311859f9b2697d9904eb8467fc1636ca015e2c0bba5e9b93dc4d106be18f76f9aaa6214d3be12f7806', NULL, 204, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 42, '2026-11-21T15:18:12.070005951Z', 'ce9f8f624308dc04', 'hold', '\xac283844310ea3a614db4dd15b65444be275f6c4515c12efe64f593680fa8aa5', '\x01999dd1aad174749a02e9a5bbe0af3093b57de63ea1b71dfb7369139dd261b8a9d755a221ddad4f1272b0339bc2bf7143e20164bced0d902a8b063f99c293d05c', NULL, 204, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 44, '2026-11-21T15:18:12.073743799Z', 'ce9f8f624308dc04', 'delete', '\xac283844310ea3a614db4dd15b65444be275f6c4515c12efe64f593680fa8aa5', '\x0163059bce55cb43d6eea1ff86fe06c7e7a4700412c3bb5b12c5972e23693a85341f7fd29004ee4a6408b2f0d8d69db5ce75aa8da19e012b111f9d79b17e973c93', NULL, 204, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 46, '2026-11-21T15:18:12.079185877Z', 'ce9f8f624308dc04', 'hold', '\x9d09ba0dfd231a84c21510293b044f194544fdac6b96a9bab82995a62f2a180f', '\x01a46f16047e642868eab7f27ad0825cfd024acda242677bb6a45a9c2a7b76add67c970d4fbfeb5a354d1d882a40b203da5321fc754a7013548f0b1e56558a4d48', NULL, 204, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 48, '2026-11-21T15:18:12.087934132Z', '7ec3b21389de698a', 'create', '\x907b3508aed394749617a6f745e91acd009629800bf268a1f8067b52bf5b8127', '\x01a3dcdf6e4418098c046c72d6aa98268cafd0b94bb15d00deff6f7278596d3542a0e9dec02476c3f2dbbebe260f812011261247aa1d5c75cd5c3b79296cc72c28', 'b370d2514af5590dd5ab3aa6563ee844', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 50, '2026-11-21T15:18:12.096290513Z', '7ec3b21389de698a', 'create', '\xf3b1506f1b5beb0c1c08f028c544e8213da1ce91ad4cdaf8d63080e0a239a2df', '\x0145abe9d629eec04a4db045cfad82dacfe64bfa4bb9477a9f06a51f127c5e6cabd5e93ade6df407e05c21a5e2875716ceba6a929b63a7cb5183351698e32685fe', '1498c47d71518d4f5b886f0f5a747013', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 52, '2026-11-21T15:18:12.10434616Z', '7ec3b21389de698a', 'create', '\xa145588c9c84e8d612e88122faab8a2366747e1787949058d24189121d6c9cd5', '\x010c8f83f56737b1cc41da732162888456521ade6e5a198e1bb3f1381d92ade9527ff92fe9788d13c279f45b2f0a1dc05f94438a21e0e6d43418520791aaf3f84d', 'c5a552d39ad97ad7603c0220027f9144', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 54, '2026-11-21T15:18:12.112318537Z', '7ec3b21389de698a', 'create', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x0134dd692cb5e418b72763fda00ff8e52167b5f45b685b8ad204a0566a18f1d21eafdc6d2ebc9d0c05648b162092e1d1c7344783f41d644e506740d2e3512e038d', '51695d266cb91e42c6d854a284face4d', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 56, '2026-11-21T15:18:12.120494148Z', '7ec3b21389de698a', 'create', '\x3055f5105a85c92f4a8725d85f157c86ebe8fc8aa3fa4a2cf9e983842acb91c5', '\x010912b4e93544d906166981f9f026cde3172d3b35d447b8acbfc3a9aed39923289ad84018f82924639bd46ca296d5b84e66f5bb8c8aaf814c1c6afe846efaa394', 'd80af39bc74cbcbec9bef220a073c102', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 58, '2026-11-21T15:18:12.128174015Z', '7ec3b21389de698a', 'create', '\x7d836c32f7aac9b6875f9e105e27681d25294c649420da5ed198e57aa816bd97', '\x01676179020bb81071e8391114ef92efadb18cacac2cea7cc67a3430502f54f305967ed66891033acd6fea956d9502b074a4ecc0b87843fbab528a539fd80783ce', '9385e49b17fadb56233afa180805b0f0', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 60, '2026-11-21T15:18:12.136061521Z', '7ec3b21389de698a', 'create', '\xcd6f0a1a71fe860cf07ecfe5c4f48ef863dd5c84eea06559dc7a3bc955ede595', '\x01fa7d69bc2821ace1220e52a8056479483551ad342e09b5233b3ea36b306582b8cd355d51b36c3cec2ab16d37c91c718577e2fcfe89f1470944767651ea18ff80', '2f6d572db1e50423b1ca6f1c470713ce', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 62, '2026-11-21T15:18:12.144011854Z', '7ec3b21389de698a', 'create', '\x7448d5dca43f0795e73a79d4d217075883bad7eaeac97ee9c1636bf2df41b607', '\x0146703d9fc377fe0f05d86b21620573369d9c582b60aadc4518ff538ea572d7e76e5a488c092c2003ae44187b68108730a94df15c19224cf9f709868d8b21c20d', 'bfdfedde205237afa5df9d01aa52ce1a', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 64, '2026-11-21T15:18:12.151783007Z', '7ec3b21389de698a', 'create', '\x8cc45230fd79e6278022bb4d0703e5968ed72a90ce096248fa29418202c7ae79', '\x01eb3e6067f7fbad1b64a412f35c02074948f05b12f508e1ba551635cc344567a8c1c7b0526c8bbd47c51a629d99f15c0e2c64cc7e496d4e828e78b341c11d5f53', 'c6feb9fa21397712cfe386dbb485514d', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 66, '2026-11-21T15:18:12.159595773Z', '7ec3b21389de698a', 'create', '\x82ec1b3591910511633651b64fb44219c26df2cfa80020521862afc48dfd19ef', '\x013236ca50861bbdddc31ec04b5f27efaba6a1a541b391a406056c464dd47a5ce9eca04890f85e99f8701de156cc5990b420018ce324199f1a9a49da17d12a612c', '9f72b12eec27fb0359b2e07d6ac654ae', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 68, '2026-11-21T15:18:12.168678948Z', '7ec3b21389de698a', 'replace', '\x907b3508aed394749617a6f745e91acd009629800bf268a1f8067b52bf5b8127', '\x01423adc82eec35dbb3a86486582c708c5a0a56a3392e4d2a188e359dc980fdac6c6f21fce857be60cd2a327bb68aa6b4871b7b586e5ef946af265d8c8d3ae30cc', '8f585f4731fb3fe7d1015d189c64d233', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 70, '2026-11-21T15:18:12.17237488Z', '7ec3b21389de698a', 'delete', '\xf3b1506f1b5beb0c1c08f028c544e8213da1ce91ad4cdaf8d63080e0a239a2df', '\x01f57368a3447de020daab3b4a58128ca32b62fb44e1ab4d3f0fead6b69ae5da4c2f336d3065cc64650b6eb1fb0d311762faaa7f48b037fd747036e3cb209cd1f7', NULL, 204, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 72, '2026-11-21T15:18:12.175048163Z', '7ec3b21389de698a', 'purge', '\xf3b1506f1b5beb0c1c08f028c544e8213da1ce91ad4cdaf8d63080e0a239a2df', '\x0161078d987ee2783a90e5f7472fdd71e859cd2b7ab58292ed0bdd9bfa2db834d8c55ae9c81e317ff79e0aec99c85003a5324e6ce42f9d9b9d93938ae688e4421f', '1498c47d71518d4f5b886f0f5a747013', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 74, '2026-11-21T15:18:12.192899854Z', '7ec3b21389de698a', 'hold', '\xa145588c9c84e8d612e88122faab8a2366747e1787949058d24189121d6c9cd5', '\x01a73ae3f4f0d3c53bc3a8f20604338950fb0dc6c2f36e7baebfa1df2a0bf5a910a1e8afeb57a41591a33162545991c05fad091c79c86eeaf10f7d0af57240f946', NULL, 204, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 76, '2026-11-21T15:18:12.196845992Z', '7ec3b21389de698a', 'delete', '\xa145588c9c84e8d612e88122faab8a2366747e1787949058d24189121d6c9cd5', '\x01d3d15cb197ab8e04e08bf44bc14db880530f95a79f9d47d2d621d690e72db59781e742b7aea030a3a2993d6ee1c578c655c56fea6b66ff4c30b7337eac3e10b1', NULL, 204, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 78, '2026-11-21T15:18:12.202601923Z', '7ec3b21389de698a', 'hold', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x0192fb0f5e4f591b5de2b7890c3450a914ea7250a5cc9c694230f71a085b4163066598dab4dd0ae2a3ae7ee1d5868d0645c145739c75ff8dd384849950a8b65909', NULL, 204, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 80, '2026-11-21T15:18:12.214254121Z', '7ec3b21389de698a', 'replace', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x01a32d943319f5fdae8fd09eb422f0c3d06c3196d229fb711ab6a0fe471ff28eba3597ac6f04988c49ddf6e1d5db1dc4788f3da5e2ef70f3ee481a87df8b700611', 'b8c07b0d66e9e95d9c321960dcd1dba3', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 82, '2026-11-21T15:18:14.145740491Z', 'sweep', 'purge', '\x907b3508aed394749617a6f745e91acd009629800bf268a1f8067b52bf5b8127', '\x018bb6a35d4fb891abe80b75bae4725a4bd562f5083652a2cfb21f75003f6d75724d3c4353bdc7af679cdd112fa8671f48e984bf7da5e626980d43610518ee27cd', 'b370d2514af5590dd5ab3aa6563ee844', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 84, '2026-11-21T15:18:14.527014233Z', 'ce9f8f624308dc04', 'legal_versions', '\xb4a7d55ec8e911552208e6613ea1444d8a2aa3fca1dd79ba3a1f5a2ebe452998', '\x01243702cd47575d427ecc332aaf5dc38a44c26f0587d7974a92203795612c34efe6ad78c6957a0f58737d2e10dd59cf8eb2477b6ac6cac3effee1658fc82104a7', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 86, '2026-11-21T15:18:14.53422628Z', 'ce9f8f624308dc04', 'legal_read', '\xb4a7d55ec8e911552208e6613ea1444d8a2aa3fca1dd79ba3a1f5a2ebe452998', '\x015c55403e93062bad5460bbb7a2dc36971cce4dd26cd2736e5a4a395c5b7053d92836c0eb899b4727ae25abde933ed85e709db9b16dbf517cd17b00f2134762c2', 'd77070b1f61f4468151bdd57c49c8179', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 88, '2026-11-21T15:18:14.540452123Z', 'ce9f8f624308dc04', 'legal_read', '\xb4a7d55ec8e911552208e6613ea1444d8a2aa3fca1dd79ba3a1f5a2ebe452998', '\x01370ba77be68eb1f387e1c744e81b0368e6d8c0a6f98c7e64ad5661fec18e727bcbb2351a26a9ed96802e6e6a8253807d7c2a4d3d8416a6d996d10c889effd939', '11c22862c9af250a39ed5051cf4ab315', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 90, '2026-11-21T15:18:14.544177324Z', 'ce9f8f624308dc04', 'legal_versions', '\x72c267245d5917c6ec04a8e216d7c7b573911f9470c02b9d5d4f21e89b0a435a', '\x01fe55478bd2158ac0f80497db854d60238687a3eb8d19c99eb3ae46a139909f56500a47a760cc29e57e1acd841afd759d2506bc298126a5803ad5e6f6309121be', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 92, '2026-11-21T15:18:14.549289518Z', 'ce9f8f624308dc04', 'legal_read', '\x72c267245d5917c6ec04a8e216d7c7b573911f9470c02b9d5d4f21e89b0a435a', '\x013f35c93d7ff46c81e24db013577ce2a418784a50c516695508207ea75c5f2294932aac9c62ceeb412e5325da2d1dc0a2cce9084d4450ca1b390dcd532a608de8', 'ac0323f72e2f01a6c48243679229ddc8', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 94, '2026-11-21T15:18:14.554473658Z', 'ce9f8f624308dc04', 'legal_read', '\x72c267245d5917c6ec04a8e216d7c7b573911f9470c02b9d5d4f21e89b0a435a', '\x01ad0162a714a5aefc53038b7f7f45a881fd1c53fefe1a3969c3d4cef24f137077aed442d5afc7350ccedf3b785d90e0741046a92e7b4bb5d0f463c1c3a823fd6e', '19303f037846a48eb00caa0af9ff2d43', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 96, '2026-11-21T15:18:14.557833938Z', 'ce9f8f624308dc04', 'legal_versions', '\x58378a0a58298e5049dad1c3069406b82cedd3da6172a15e60651fd1a5d68629', '\x01b2a47e57ac07973fdb180f36186e10db509d69034e7373d43a345db4e04d5f2a8146ca97500ff8eb06f1e927b94f7f195287959e2b20508b1c7e9c79d11d4048', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 98, '2026-11-21T15:18:14.562411628Z', 'ce9f8f624308dc04', 'legal_read', '\x58378a0a58298e5049dad1c3069406b82cedd3da6172a15e60651fd1a5d68629', '\x01e1849a3ce3c97937f67d7930afb6a5288ed4182e78fb300953a04498be98eaf1a92d58a38d3006b5b71894a07f4bd8f2966f8f3165abe9238bec408e8d56d93c', '9c6111c70ce30631f2b1e0d35cd2127d', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 100, '2026-11-21T15:18:14.565612101Z', 'ce9f8f624308dc04', 'legal_versions', '\xaf38557482c88675dc6fe4873a1e1b8dff03acbaa3da6f0acf280f93061089b1', '\x01f7d8590bfa171756edca2b82decee574a9493cb1a802f46bd36a087110a1d8b9c259aad65b13dafed30a0a66c605014ad8e7dee8fd53008479563266b2b56cf2', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 102, '2026-11-21T15:18:14.569914937Z', 'ce9f8f624308dc04', 'legal_read', '\xaf38557482c88675dc6fe4873a1e1b8dff03acbaa3da6f0acf280f93061089b1', '\x01e3783b1ac229e43dfe98a947347a91f49606f92d73f6af5fc5d6cbd59b48236d2adb48840472b120bb4cf87f3d204cf3fe863e1d4c13b64d364717942f7101b3', 'c6e1dfe9f11fdc392c7ade63ba11f83d', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 104, '2026-11-21T15:18:14.57294126Z', 'ce9f8f624308dc04', 'legal_versions', '\xac283844310ea3a614db4dd15b65444be275f6c4515c12efe64f593680fa8aa5', '\x01e9e502418f675b784c49bf98673d482a65539db388cec49247abaddacfb11e6a9e652c90102b069db83e0c76ce894c8a23d17d6fe325c0e83bca67c119717298', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 106, '2026-11-21T15:18:14.577100492Z', 'ce9f8f624308dc04', 'legal_read', '\xac283844310ea3a614db4dd15b65444be275f6c4515c12efe64f593680fa8aa5', '\x0191e90c386cd926f1b0679bea54b8a7c3e1b1cd3b74a5207453312fd397f16966aa89e65f06b5a8050d40d22fc71d6a13125d5cc519dbf10d4f1300920bbaec17', 'e837e126b62a0a86daf9a70db1fa46aa', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 108, '2026-11-21T15:18:14.580074426Z', 'ce9f8f624308dc04', 'legal_versions', '\x9d09ba0dfd231a84c21510293b044f194544fdac6b96a9bab82995a62f2a180f', '\x019442e2269e255f9576e3342e80710acdd2c2b85d20c4d6ffd656549f9c280017597b678531f82d5133a5c60a2bc692c300ea171e8443ee65bc3a73d557814ed5', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 110, '2026-11-21T15:18:14.584259001Z', 'ce9f8f624308dc04', 'legal_read', '\x9d09ba0dfd231a84c21510293b044f194544fdac6b96a9bab82995a62f2a180f', '\x011a4cee2e322e4b516e447d91c295412090d088c72153070d55c83748aac0cddf9fd8ccc53b88f1f97b6e4e34f67680e0666af07e42471780b84fb34194177839', '6f5414f3b68fec4d24534211ce1eaa5d', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 112, '2026-11-21T15:18:14.587229408Z', 'ce9f8f624308dc04', 'legal_versions', '\xbc013866ad2b9f8241810df4d3275397eb7bb75441c33581737ffd14c1faad75', '\x01e167abeaeb962bbbebacd2e94a24ef22b392cd4de3414f1a458b1b5160f90f70ba5345ad267e0ee2c95175261c74abb6dde778fa06b0ff9873b643fce9c1a7ef', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 114, '2026-11-21T15:18:14.591177824Z', 'ce9f8f624308dc04', 'legal_read', '\xbc013866ad2b9f8241810df4d3275397eb7bb75441c33581737ffd14c1faad75', '\x013c8c8ba8748fa53ea88ff02891c1d50eb16b8b321af2d982fff4fa28f3119bb0f29cdfbca60873d2d98213eaa964e416f602c35a62a3213813487e3a14f4d79f', '13a86df30049927d2fb0d66c7b2a2110', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 116, '2026-11-21T15:18:14.594151591Z', 'ce9f8f624308dc04', 'legal_versions', '\x4b46dd6700ea57702c525d6259516fc26d23999f83a95750494e8463fc84f58b', '\x0174319d5317d1f191ccee681275055ff799b508a09a8eb9c7c6d2e17299e5ab691033a37a3b8cfba85617346c1f28c6fb28e1ec6a90cae833d257f73c3cc389e0', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 118, '2026-11-21T15:18:14.598120932Z', 'ce9f8f624308dc04', 'legal_read', '\x4b46dd6700ea57702c525d6259516fc26d23999f83a95750494e8463fc84f58b', '\x01f695ed40173bb08702411310085fc6b0a630f1177eaf52d7bd1798470c920e766bcd6049f1ddecc0c6befb0566887b0f0ef3537457577836409fd48fed5f16b7', 'ab153bd041a333f65664b100f1eb387c', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 120, '2026-11-21T15:18:14.601457749Z', 'ce9f8f624308dc04', 'legal_versions', '\x69571665c08c655d7956c39f22df6a2d2d2436887f47f403a43a4a8d68a166e3', '\x01e4179092bd9bf34b65d7b7111e7e0b9eecad7a3e733fa408860d9fd858a00ec82a558e4d9c8b68a0340bbe2fe64421e7cf42516400e343a615109003b8149e53', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 122, '2026-11-21T15:18:14.605589639Z', 'ce9f8f624308dc04', 'legal_read', '\x69571665c08c655d7956c39f22df6a2d2d2436887f47f403a43a4a8d68a166e3', '\x012e0e32c61de4f742e81d199c745b8fa060f646dd60f40d71d17119ccfe307df214c75685345369a525fac62ae02ec3b47b6db9e80b45ffe42ec65b993533b7a0', '449f0c2a02ddcd495bb011f7a9728801', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 124, '2026-11-21T15:18:14.608647529Z', 'ce9f8f624308dc04', 'legal_versions', '\x96cf8f902bcebf5aeae5df28897d4ff57d29b9a3cf1eb42d6f6aff04b37ea2aa', '\x01b24d8fd42c4a5288e1cc0585008679393817f712c64a2a75951e6ded37114d35705d65b9f56b3451469f91afd0d22ff9184422c9292be8327fe6d420cee03f68', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 126, '2026-11-21T15:18:14.612574286Z', 'ce9f8f624308dc04', 'legal_read', '\x96cf8f902bcebf5aeae5df28897d4ff57d29b9a3cf1eb42d6f6aff04b37ea2aa', '\x01d3319914167a3e82742ab6cdc856d6b92a73a7e52774bfc8debe600d729d5d3fe31f7456eb19d9a02996148ba24196f6df1811b404e8f7487e9c9b5547032ed4', 'd749839ad7da34808e31bcb96f5e28d6', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 128, '2026-11-21T15:18:14.615483516Z', '7ec3b21389de698a', 'legal_versions', '\x907b3508aed394749617a6f745e91acd009629800bf268a1f8067b52bf5b8127', '\x01d0cce4f1aee60c43a15b79173dbc52856b7f32199afb70c39843076288468700bd6ac5d5343a2273e4481318ab3dc18618bf45b9d824dd62256d6f47674fefdc', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 130, '2026-11-21T15:18:14.619403245Z', '7ec3b21389de698a', 'legal_read', '\x907b3508aed394749617a6f745e91acd009629800bf268a1f8067b52bf5b8127', '\x01a005abc87566fae0ced5b7d29c8f5f02729bbcb6c27b0a2d4ead55fcbcd7b6a96081e4e956d37579b0659feb0d9d33788ddaec22d65f7e806c13b04f4acf298b', '8f585f4731fb3fe7d1015d189c64d233', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 132, '2026-11-21T15:18:14.621996103Z', '7ec3b21389de698a', 'legal_versions', '\xf3b1506f1b5beb0c1c08f028c544e8213da1ce91ad4cdaf8d63080e0a239a2df', '\x01462428ef966678eab6fecd5dd10a27087ddbca98824e36a004e4678fef00d66dc78fbc11892d1c56e6f7694997a95a43abfc92352c106f1308921f3f40b2d43e', NULL, 410, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 134, '2026-11-21T15:18:14.624892434Z', '7ec3b21389de698a', 'legal_versions', '\xa145588c9c84e8d612e88122faab8a2366747e1787949058d24189121d6c9cd5', '\x0154063b7d5398a0fce8441730be39bf296e5e2b6b86cc2b9e117eec3d0504838444c5d27c3b074e4dd5bf267c9362a27d5577a959a23c6f78e93c5d8af8b7fa7a', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 136, '2026-11-21T15:18:14.628823341Z', '7ec3b21389de698a', 'legal_read', '\xa145588c9c84e8d612e88122faab8a2366747e1787949058d24189121d6c9cd5', '\x0198a042d816db86a219e67bf85081220cfa4b10bbe3c7ebd9a908ec9e0b584524e6f0c82d4ec8badbfdbc1c9be9307c3f31d74d17e7c49ecd23437918b86cf418', 'c5a552d39ad97ad7603c0220027f9144', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 138, '2026-11-21T15:18:14.631698117Z', '7ec3b21389de698a', 'legal_versions', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x01315ef0e6e1b552f31c850e52468a9e976f276f0b529cd7c7f8e3fc99f7aa20d988b343820f621655d56612da861208b6a2d7c60395497809b2fbe6bca7d5d424', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 140, '2026-11-21T15:18:14.635497058Z', '7ec3b21389de698a', 'legal_read', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x01ddf73b4e3aeef71686a1f21dd7b543543ade86355becaf990c655b2d03de06eb970746f9baca23ea7303d8d7a3d1e4ab808ec732c1a351746e0e06e0a60ac898', '51695d266cb91e42c6d854a284face4d', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 142, '2026-11-21T15:18:14.639379205Z', '7ec3b21389de698a', 'legal_read', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x01f2f5bcb4fd207e12f1dffc2166e653faca407f0925989b59330bb3a0fd9b49fabff3e2a08afdc49fe14e08603ce1fd7cad2d53510d27da5a06e881043a2dfd49', 'b8c07b0d66e9e95d9c321960dcd1dba3', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 144, '2026-11-21T15:18:14.642301212Z', '7ec3b21389de698a', 'legal_versions', '\x3055f5105a85c92f4a8725d85f157c86ebe8fc8aa3fa4a2cf9e983842acb91c5', '\x013a9e3c0649e0ee1aa1c93e627b1e13d7178614edca9ea16c6b0bfd686cf9b92e8a86e403265c3f80ab9a96992c5605e61f772d7544606ebfa196f5c556002e22', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 146, '2026-11-21T15:18:14.646234378Z', '7ec3b21389de698a', 'legal_read', '\x3055f5105a85c92f4a8725d85f157c86ebe8fc8aa3fa4a2cf9e983842acb91c5', '\x019a6b792354a343a1e85f218d7d38c2a2799c7fd09da0fa7a30c0be9c0c7df4bf6421d42ae3a023d2b927583351c93538eb4e76d893fefaa7ef794bcdfc0c9fe6', 'd80af39bc74cbcbec9bef220a073c102', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 148, '2026-11-21T15:18:14.649257044Z', '7ec3b21389de698a', 'legal_versions', '\x7d836c32f7aac9b6875f9e105e27681d25294c649420da5ed198e57aa816bd97', '\x0119b3b74c39fd2cf52665323e119009850fb84ea5fa86eb9e2f090ff6bce2668091e79e6565009e0b724a138b028fe445482d7be8ce9c35bc06a249d3143ec4d3', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 150, '2026-11-21T15:18:14.653097968Z', '7ec3b21389de698a', 'legal_read', '\x7d836c32f7aac9b6875f9e105e27681d25294c649420da5ed198e57aa816bd97', '\x015c15896025e9dbb0ce7be76a5b6e755e96ed4fa7993bcf943f6a004f59bb77b7b095bf8c0c2af169af891d21cd96e4462fbe1315bd955411d7bc923c3bf12441', '9385e49b17fadb56233afa180805b0f0', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 152, '2026-11-21T15:18:14.656587351Z', '7ec3b21389de698a', 'legal_versions', '\xcd6f0a1a71fe860cf07ecfe5c4f48ef863dd5c84eea06559dc7a3bc955ede595', '\x016db2784acb126f65c7e0378006e6d4c26531915efdb03fb9b13c110b699904c5fe555d126f735aeafc665994925ea8d1b3004232b9265b328645cad2aa025c75', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 154, '2026-11-21T15:18:14.661353849Z', '7ec3b21389de698a', 'legal_read', '\xcd6f0a1a71fe860cf07ecfe5c4f48ef863dd5c84eea06559dc7a3bc955ede595', '\x01995fb576b8d64d04b7addf9702fee38315193a3248ab91a41338cb3bb6b90b05eb5a51ffd4e4d97ed60145fc11dbbec1da2ed2598d44dd8a63b3368dc5d83609', '2f6d572db1e50423b1ca6f1c470713ce', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 156, '2026-11-21T15:18:14.664657582Z', '7ec3b21389de698a', 'legal_versions', '\x7448d5dca43f0795e73a79d4d217075883bad7eaeac97ee9c1636bf2df41b607', '\x0186df328002f5e888164c88318fd7c64137d81269e4ca028c63cc0ba94cb7d04d90e514ca85ddf3e863812cf8c77e976603adeeeae936568062367a40d84317d2', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 158, '2026-11-21T15:18:14.668816194Z', '7ec3b21389de698a', 'legal_read', '\x7448d5dca43f0795e73a79d4d217075883bad7eaeac97ee9c1636bf2df41b607', '\x01f35628d0759137f0eb603900e732a0b3af2c84e423d1fce3088a43a409f8f31ccd0bf45fa75e7f563d99190dacb04df24240d0c7df89ee296d4e286208903280', 'bfdfedde205237afa5df9d01aa52ce1a', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 160, '2026-11-21T15:18:14.672002334Z', '7ec3b21389de698a', 'legal_versions', '\x8cc45230fd79e6278022bb4d0703e5968ed72a90ce096248fa29418202c7ae79', '\x01661aa38dd8900e1cf5363049c8be9fb2d6b6e758f5f57c61dc10e3020cb8a0bfab1777f0075f11a28e892afe23bda783f816b3360fe6a540848541b97613f4f0', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 162, '2026-11-21T15:18:14.676594403Z', '7ec3b21389de698a', 'legal_read', '\x8cc45230fd79e6278022bb4d0703e5968ed72a90ce096248fa29418202c7ae79', '\x01ba9e016a47beb16d400b1ae67750de580ba5f4504c4caf2326178e432d962ed21b8ed1e4407adfc11dc14e1b40d8b32ec27d1a38ddc9279f6be3af813db33941', 'c6feb9fa21397712cfe386dbb485514d', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 164, '2026-11-21T15:18:14.67979121Z', '7ec3b21389de698a', 'legal_versions', '\x82ec1b3591910511633651b64fb44219c26df2cfa80020521862afc48dfd19ef', '\x01f4893278a510ed9cf9378770fec717a6540174efb7a7ddd6ebf054caa804bc769a462b5ae986a83b9d7ff81a805a9e0c25bf93ee7c7f51951c32b5594bd0b9d1', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 166, '2026-11-21T15:18:14.684153824Z', '7ec3b21389de698a', 'legal_read', '\x82ec1b3591910511633651b64fb44219c26df2cfa80020521862afc48dfd19ef', '\x011ffdff7aa9d42c66fd1c9af9747af9c37264db84364b189e265aa0b742781afb014a275d757266eec631a00a659339fa8c1b12c58add5a61dc78da80c66df4e8', '9f72b12eec27fb0359b2e07d6ac654ae', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 168, '2026-11-21T15:18:16.853443022Z', 'ce9f8f624308dc04', 'create', '\x2ef439289f85a2b4377be4791d2477410312197aa902524f9c3c2240a665e9f2', '\x01640cfa11d1a5f57d42091143894c0d32b4d47ed34ad446b34b859d9f60e28499280627535143195fb5a018d3df8cebe818703a6ba08f2e12cbb7b2fedeca07c6', 'b177b7e4b740ae575e016adbb4e091d8', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 170, '2026-11-21T15:18:16.862474038Z', 'ce9f8f624308dc04', 'create', '\xed729fcf40a022a4779b1b282991702d8dd179f08915163e1ca1b848f7e6af85', '\x01650b3ea1ad7a174a230bf82569b4c3b5a4e8e7e471cb6445889e55ac49f6f0b10faf03e459fcd53101bc10a83065ba1a8d9929ca20fd8dacb914c41aa64d45cb', '8ce7054759d13b3554dba77725561896', 201, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 172, '2026-11-21T15:18:16.870628899Z', '7ec3b21389de698a', 'create', '\x2f2ee5d0941c476829b02a1eb479ae775b570d35d9d690f08d88a4cd8b23d934', '\x010f87bb22b27343780063c04a6b4c4cf108ac5abfc5494197001688be3638d5630777d817797bf936414d1d8a3854d234196e2073719d23315bf16a96a22a27f8', '3085a8236d4dc94fc3720199c905615f', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 174, '2026-11-21T15:18:16.878638092Z', '7ec3b21389de698a', 'create', '\xbfc819b7d7f1b91f22211cbae8a87d2bb1fc1bd9e3f6641250e97b1ba7177854', '\x012a7ec9a8d4cc54d30f14f2f840b3c5c46da698c347e1162a5775369110f1a45a4d65c81b3842d9eae09362e20e5edfdf8274a7e180dbe37894712cfe1f3a06a0', 'a51a9c26635c3af10bcfcb2227850803', 201, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 176, '2026-11-21T15:18:16.882134021Z', 'ce9f8f624308dc04', 'legal_versions', '\xb4a7d55ec8e911552208e6613ea1444d8a2aa3fca1dd79ba3a1f5a2ebe452998', '\x01b0c2f6e00143fa1d4f49984df6b074e07550d39ac0c0fec5316a0a38f548ea0e9aa2bd8388b6f0b0d90ee2fa3a7878aae6ed7a4593ef183c0ff4aac18f13d3ba', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 178, '2026-11-21T15:18:16.887704515Z', 'ce9f8f624308dc04', 'legal_read', '\xb4a7d55ec8e911552208e6613ea1444d8a2aa3fca1dd79ba3a1f5a2ebe452998', '\x0152a5a6b84b60f209aaddf47ef4d8860db8603188513c3b53752e2f14a0adaecbf6f1810a53aa5287c534a63484a5837f06e99d2dea86b61ecbe74cffe44c43d9', 'd77070b1f61f4468151bdd57c49c8179', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 180, '2026-11-21T15:18:16.891962165Z', 'ce9f8f624308dc04', 'legal_read', '\xb4a7d55ec8e911552208e6613ea1444d8a2aa3fca1dd79ba3a1f5a2ebe452998', '\x016a0d9cf49f858158d921661f9fa3d6e1912219738f79b6edd9b41c1398ee08169f535e4d9fb220fd47da5cdd6fbd13ab407ff553b243a0a7375eab9f834ca388', '11c22862c9af250a39ed5051cf4ab315', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 182, '2026-11-21T15:18:16.89515912Z', 'ce9f8f624308dc04', 'legal_versions', '\x72c267245d5917c6ec04a8e216d7c7b573911f9470c02b9d5d4f21e89b0a435a', '\x0171e7c5df717b83eeccf365f67a629708e176195435854a4f2de2224312bdcc696fa2709add533f32ec8a30a400807816736497b4c9e63a73b19f4af4993a8bc7', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 184, '2026-11-21T15:18:16.899280629Z', 'ce9f8f624308dc04', 'legal_read', '\x72c267245d5917c6ec04a8e216d7c7b573911f9470c02b9d5d4f21e89b0a435a', '\x0165a7d982c23e2c04b3b518e4f8135f93ae20470c52dd992f85cd9d01eabe91e3599ae95a0c2732595277bb5ce84a412f922e01b68773e44e6e52a966d104cb8a', 'ac0323f72e2f01a6c48243679229ddc8', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 186, '2026-11-21T15:18:16.903122274Z', 'ce9f8f624308dc04', 'legal_read', '\x72c267245d5917c6ec04a8e216d7c7b573911f9470c02b9d5d4f21e89b0a435a', '\x017844f654d5933e6cbd482cd9e9580be6206bf7800698b30b1b401922fbe585e2bff16578fbefeb15a9b77c1e6aeba16d9823b31557c0f7340e12ae5e491f61ac', '19303f037846a48eb00caa0af9ff2d43', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 188, '2026-11-21T15:18:16.906258913Z', 'ce9f8f624308dc04', 'legal_versions', '\x58378a0a58298e5049dad1c3069406b82cedd3da6172a15e60651fd1a5d68629', '\x01dd10dbb0ca4fb000460c1263fc24af26eb4b17e4c42e651ec52ed601670de32de8b12ba2617ed1e5ddd1e59431eb34b16c75d2d484ed733f396b67e060e9906a', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 190, '2026-11-21T15:18:16.910111096Z', 'ce9f8f624308dc04', 'legal_read', '\x58378a0a58298e5049dad1c3069406b82cedd3da6172a15e60651fd1a5d68629', '\x01c8f5b7d891085832d5f8f3b8ef41ccb3241deba8df45dbfff5414870074323d383a903876b7764836f13590175ac03adc00c380b8e7c6dad9a760daf889d43e2', '9c6111c70ce30631f2b1e0d35cd2127d', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 192, '2026-11-21T15:18:16.913010271Z', 'ce9f8f624308dc04', 'legal_versions', '\xaf38557482c88675dc6fe4873a1e1b8dff03acbaa3da6f0acf280f93061089b1', '\x010ab3dcdca3f8042eacaa72f9b984c80cb81a47f544d5ebdff80e9320f9f92549e95bf5f08351b7c285318a0cd0e671da7490c63ce25eb695876b204d8d1b9b11', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 194, '2026-11-21T15:18:16.916713591Z', 'ce9f8f624308dc04', 'legal_read', '\xaf38557482c88675dc6fe4873a1e1b8dff03acbaa3da6f0acf280f93061089b1', '\x0199c5e1c50dac0d21c001eb465112a540cb61a6d55f34e36ea5bd110d835221ab74c2eb29ccfbe119c6648dd87049c312c36143d2775ea470b20e6a54d67961d6', 'c6e1dfe9f11fdc392c7ade63ba11f83d', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 196, '2026-11-21T15:18:16.919513866Z', 'ce9f8f624308dc04', 'legal_versions', '\xac283844310ea3a614db4dd15b65444be275f6c4515c12efe64f593680fa8aa5', '\x01688ffcf78c2d9736208c498b55bfe404e8d06f97341d908c79841c258b87fd61e52518d9c15e57c12d1cc6e29d36eebf1422d3929b8aa8c1ccb2e3f83063c260', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 198, '2026-11-21T15:18:16.923108065Z', 'ce9f8f624308dc04', 'legal_read', '\xac283844310ea3a614db4dd15b65444be275f6c4515c12efe64f593680fa8aa5', '\x01567d9583f1c96a6070451eded02a69176d59b35f538dc7567f3488a928ccd8294603245de77e5a87b18b36874eaceec59f44e4310ebd37e87399601e7744d11a', 'e837e126b62a0a86daf9a70db1fa46aa', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 200, '2026-11-21T15:18:16.925962535Z', 'ce9f8f624308dc04', 'legal_versions', '\x9d09ba0dfd231a84c21510293b044f194544fdac6b96a9bab82995a62f2a180f', '\x01d9123ffae7cf9f420e3eeca99611ed23ed8b4d24759ab4dac4ef5797893718851c7252c73de00e0f6499c55e84f6dfe4751ae6bfcafe4e71fa1fb4dc7d5e280c', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 202, '2026-11-21T15:18:16.929603624Z', 'ce9f8f624308dc04', 'legal_read', '\x9d09ba0dfd231a84c21510293b044f194544fdac6b96a9bab82995a62f2a180f', '\x013a55441da6c3f9f7ee55dc9a2cc6324654b65ef0a550def69debfb9f600e59210c9343d4b588f0f1cb2d4a7824a961180d52bf5905cfa133ba6435a5f1c7038c', '6f5414f3b68fec4d24534211ce1eaa5d', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 204, '2026-11-21T15:18:16.932352602Z', 'ce9f8f624308dc04', 'legal_versions', '\xbc013866ad2b9f8241810df4d3275397eb7bb75441c33581737ffd14c1faad75', '\x018482213cf1b2828c7b75b85dc1ab4359889062b18abf51dbb2599ea808a255150dcc9074b6717645bd2ebfac674e3a95e5cba766990953aae088def57637a8f8', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 206, '2026-11-21T15:18:16.935974431Z', 'ce9f8f624308dc04', 'legal_read', '\xbc013866ad2b9f8241810df4d3275397eb7bb75441c33581737ffd14c1faad75', '\x0104ba9038b331c0b14d31747d5d47cc61e98cc79da76e96396cb5e410927593e65c4ffe818b37329c0ade5992ae90ed2cbb8c3b6fa304ed7550055efffbb0ef67', '13a86df30049927d2fb0d66c7b2a2110', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 208, '2026-11-21T15:18:16.938748993Z', 'ce9f8f624308dc04', 'legal_versions', '\x4b46dd6700ea57702c525d6259516fc26d23999f83a95750494e8463fc84f58b', '\x0123b228f218d4540b812a632c8ce68483cd0649dd66298c72c97fccb3424b6d38d999b6c4278e75df50feb4fa716dfedad6d1a656d1771b21d6915464a1533be6', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 210, '2026-11-21T15:18:16.942329312Z', 'ce9f8f624308dc04', 'legal_read', '\x4b46dd6700ea57702c525d6259516fc26d23999f83a95750494e8463fc84f58b', '\x01f33b1c1d714a8e9cce15a02871c9b6cec53f2efee983c1511b75f81c0a8aee645d378f1c9efe62217d351c240b4f9c991eeaa1ccae07ecff2f4ace92aeb59d4a', 'ab153bd041a333f65664b100f1eb387c', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 212, '2026-11-21T15:18:16.945109485Z', 'ce9f8f624308dc04', 'legal_versions', '\x69571665c08c655d7956c39f22df6a2d2d2436887f47f403a43a4a8d68a166e3', '\x01e5ce3eca2b327d4597ea42b3a6eb6169558d1221f533e8bc7aafc0a7b7a56aef2cf25c83eaeaa94f8d8d84fcbe60dbda12faa4cebfcc13dece95f5c814bd9808', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 214, '2026-11-21T15:18:16.948837343Z', 'ce9f8f624308dc04', 'legal_read', '\x69571665c08c655d7956c39f22df6a2d2d2436887f47f403a43a4a8d68a166e3', '\x01081282c5101e10d1a6ba259d2b5f6541bf435aae7ebd921d4cb8cd0a8c960842d5169bad3fd5167e75f0d9750ce33873df6cd57b7f7c088827a9a2eb66d9f2f4', '449f0c2a02ddcd495bb011f7a9728801', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 216, '2026-11-21T15:18:16.95181437Z', 'ce9f8f624308dc04', 'legal_versions', '\x96cf8f902bcebf5aeae5df28897d4ff57d29b9a3cf1eb42d6f6aff04b37ea2aa', '\x012dd4bc9216efa0fd775f9fe18e469d54c2b4574229224ecda168f388ad854344e7033ebf770664b0c4209b31a18b5b9299211abdb24959e064cd9b094a9653d4', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 218, '2026-11-21T15:18:16.955528107Z', 'ce9f8f624308dc04', 'legal_read', '\x96cf8f902bcebf5aeae5df28897d4ff57d29b9a3cf1eb42d6f6aff04b37ea2aa', '\x0192fe691b44d208a7707039eca4151c49f8dadfbf3ed45b4f6cc390b8fd547d3e18a7f03427eeb75eacfc7534256f28ebd6cf538a540be264bd687f642c80d521', 'd749839ad7da34808e31bcb96f5e28d6', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 220, '2026-11-21T15:18:16.958364909Z', 'ce9f8f624308dc04', 'legal_versions', '\x2ef439289f85a2b4377be4791d2477410312197aa902524f9c3c2240a665e9f2', '\x015d4bf0b214f2658d571b27f7a03c8d58a21aec7b4ee0ddd6c069ed0a61c2b89384efaf33bf766c2e1a460b19126b3a49f4fe0918c2999da7dfff56acb6a09b0a', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 222, '2026-11-21T15:18:16.962975386Z', 'ce9f8f624308dc04', 'legal_read', '\x2ef439289f85a2b4377be4791d2477410312197aa902524f9c3c2240a665e9f2', '\x01958df369fef250b88e8a13e2a18c79d38b1f989db4eeda28c96265374b85331448be1a51becc6f719093c7776f9e1d46815cc1aa0ec0d4e09d1d95361d43eb01', 'b177b7e4b740ae575e016adbb4e091d8', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 224, '2026-11-21T15:18:16.96578793Z', 'ce9f8f624308dc04', 'legal_versions', '\xed729fcf40a022a4779b1b282991702d8dd179f08915163e1ca1b848f7e6af85', '\x011d70c2732fd545162dc53e7cb1418fcafcf5c21dd24e8b2daa827a12f242c687b8a0eabaa969db763af3ab61b7a39dd2404b89d01bdc0ad58e58635df5c17186', NULL, 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 226, '2026-11-21T15:18:16.969737845Z', 'ce9f8f624308dc04', 'legal_read', '\xed729fcf40a022a4779b1b282991702d8dd179f08915163e1ca1b848f7e6af85', '\x017cd40c06d9eb19b14749742f5b74de8e825c70c77118f762458ca36d26d6842a0a9291717d4420717a45673f56dc3b696df08448e932a9771c0d833ab3b58ade', '8ce7054759d13b3554dba77725561896', 200, 1, false, '\x5f0296f9521e24b7');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 228, '2026-11-21T15:18:16.972550131Z', '7ec3b21389de698a', 'legal_versions', '\x907b3508aed394749617a6f745e91acd009629800bf268a1f8067b52bf5b8127', '\x01ca39e9f68e72a3185d2ace2afd23594b20d28d8f9f2584ed20b57625326a98a3206bdf997432d85e7224ec36de3892de76903a6aa53fda1923d86021f4031d3f', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 230, '2026-11-21T15:18:16.976935808Z', '7ec3b21389de698a', 'legal_read', '\x907b3508aed394749617a6f745e91acd009629800bf268a1f8067b52bf5b8127', '\x01b18bbd9736640bfbd57681f7530e782ebd05c6192fae28fc653d087038dbaf2216ce63db594f1eefc0f170f51a28cdc30b406f99c89a63b745ba1d5cf5e67557', '8f585f4731fb3fe7d1015d189c64d233', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 232, '2026-11-21T15:18:16.979442701Z', '7ec3b21389de698a', 'legal_versions', '\xf3b1506f1b5beb0c1c08f028c544e8213da1ce91ad4cdaf8d63080e0a239a2df', '\x011ca0f7a2cb02fc9fd6800791a63747f4a77495cbfec027f3887e11e651d87e4cbba327821faa35362e0ef0af48eb861b97fff39aefff143b4e03c7f9cbc5dfd7', NULL, 410, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 234, '2026-11-21T15:18:16.982397931Z', '7ec3b21389de698a', 'legal_versions', '\xa145588c9c84e8d612e88122faab8a2366747e1787949058d24189121d6c9cd5', '\x01b91391cc15ed2bbc78f5f623e6302c525066a8868d38fd101ffdb25bd507bf465a1d03fc93f3a9292d0c024f245bec77e93d07408e5a7509bf37f8fd5fbe4cf1', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 236, '2026-11-21T15:18:16.986258216Z', '7ec3b21389de698a', 'legal_read', '\xa145588c9c84e8d612e88122faab8a2366747e1787949058d24189121d6c9cd5', '\x01d61c4f62c7016e9fb246e2b43c41c404de94ffb7b0dd3179196620bbb7eaf5f34dc060c54f4d86d15a097e009e222a7dbed40e3d071222c2fd3639db1ab67d80', 'c5a552d39ad97ad7603c0220027f9144', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 238, '2026-11-21T15:18:16.98920904Z', '7ec3b21389de698a', 'legal_versions', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x01345665f749a7eac11daafc44fbed499e3d70b8853c221b57892e3e81eebaba9ecbb30f7c5fec4f326ee1c0feb8a29d375ecf02e11df011eb7de22e61526fec0e', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 240, '2026-11-21T15:18:16.993076333Z', '7ec3b21389de698a', 'legal_read', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x01e8e5f37a26aef2e0876e2702b136d6955141f9bc45472113e08024293db61b0988c71e5e00fc289b0393ace394792de91d5aca0e593e89c4f64166973b3f8cfe', '51695d266cb91e42c6d854a284face4d', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 242, '2026-11-21T15:18:16.997557337Z', '7ec3b21389de698a', 'legal_read', '\xfdd34a691d6b1370676828793291af6c8c24be77784efe44948cc6173eef0e5b', '\x017463ae0cc2d76735973d5ebce1ed342ffca099b689f8f54c0ca949c957d49b68345c4799e6d5fb3daa37e1a7d049ba64a7036e5b4ee37d6469a2712e3f00d79a', 'b8c07b0d66e9e95d9c321960dcd1dba3', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 244, '2026-11-21T15:18:17.000659642Z', '7ec3b21389de698a', 'legal_versions', '\x3055f5105a85c92f4a8725d85f157c86ebe8fc8aa3fa4a2cf9e983842acb91c5', '\x01620080d31208df2ab11a0f1fbaaabb40317da446da500bb07ea5740c08f0f94271764b7c26bb46583201f383201a6ad316f7930692b4c2cfed420ba11d5411a0', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 246, '2026-11-21T15:18:17.004832448Z', '7ec3b21389de698a', 'legal_read', '\x3055f5105a85c92f4a8725d85f157c86ebe8fc8aa3fa4a2cf9e983842acb91c5', '\x011fed6fc6322cafabc32c120db0b69bb8b0e87cfa0c697e235435be68321004912ecac7e52832bdc2103d3c5e8a56df942b4dec768d9a56fcd8f9541a08677efb', 'd80af39bc74cbcbec9bef220a073c102', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 248, '2026-11-21T15:18:17.0077224Z', '7ec3b21389de698a', 'legal_versions', '\x7d836c32f7aac9b6875f9e105e27681d25294c649420da5ed198e57aa816bd97', '\x012853274ed5981610a32df327b12ddabd5608bfcab09f042b49fa685b910d532aa4b2449104925bc7267300b8103e95bbafde110651a4156df5e364b8e79243fa', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 250, '2026-11-21T15:18:17.011530739Z', '7ec3b21389de698a', 'legal_read', '\x7d836c32f7aac9b6875f9e105e27681d25294c649420da5ed198e57aa816bd97', '\x019c8c92b3c53f7ed176d185043d8b5f695fee0bcb7a0b5fd07ba3644c4d90e350440f3e82b94b9dac696345b314abfcece9e312a6d40680e94b647d7a965c431b', '9385e49b17fadb56233afa180805b0f0', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 252, '2026-11-21T15:18:17.014424284Z', '7ec3b21389de698a', 'legal_versions', '\xcd6f0a1a71fe860cf07ecfe5c4f48ef863dd5c84eea06559dc7a3bc955ede595', '\x0112d9a8cd66501f003f6a92b2b0fc302d05f45d133d0112d607048cd7beeefcd24d76bd49a2723a7687ad2d892861faac917fab008ab0fff85355812ac1f34a92', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 254, '2026-11-21T15:18:17.018242893Z', '7ec3b21389de698a', 'legal_read', '\xcd6f0a1a71fe860cf07ecfe5c4f48ef863dd5c84eea06559dc7a3bc955ede595', '\x01cb50e734b61ef6c63300396c4ae190d3a70f3710ea0c36b1051f2f8d7d556a4c7ccddb6f4a9923c60fe20dbeefa6e7e0b7efa1869ecd0a85a8027627dcce46f1', '2f6d572db1e50423b1ca6f1c470713ce', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 256, '2026-11-21T15:18:17.021057751Z', '7ec3b21389de698a', 'legal_versions', '\x7448d5dca43f0795e73a79d4d217075883bad7eaeac97ee9c1636bf2df41b607', '\x01a282b6b1b7592ef99ea025433c8c9c708f97b5837350e32dc2beef8c91e032cb172afcfa9a97a2d947f0f4191ca8500145b1ae189578a687e8d82dbdb73fdfbb', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 258, '2026-11-21T15:18:17.024671191Z', '7ec3b21389de698a', 'legal_read', '\x7448d5dca43f0795e73a79d4d217075883bad7eaeac97ee9c1636bf2df41b607', '\x019c7bf93654a046eaed73663f3f70286d8f2f0bac17f10ddd8c0ab2ca82a5627735b0b7994a2b12962f35bb8d1368aab38f1c8fda45028cf60ab57aeaba113904', 'bfdfedde205237afa5df9d01aa52ce1a', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 260, '2026-11-21T15:18:17.027601023Z', '7ec3b21389de698a', 'legal_versions', '\x8cc45230fd79e6278022bb4d0703e5968ed72a90ce096248fa29418202c7ae79', '\x0172082e9e3b08872bfdcc2326ab21bc5bf6577c4f62272809483d5416eae8573d7f9895bcaaf733c47d9cbb7577c9e9009a8d7a132f660a13602cfe02f509e3aa', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 262, '2026-11-21T15:18:17.031282334Z', '7ec3b21389de698a', 'legal_read', '\x8cc45230fd79e6278022bb4d0703e5968ed72a90ce096248fa29418202c7ae79', '\x01a3af374013daab39551887ac49acb9d0fe669bd521a59a208829f2825393bef52c7393531a22649ee054bd31ca8f2052dab7a4884220b62245b4854ac735dd8b', 'c6feb9fa21397712cfe386dbb485514d', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 264, '2026-11-21T15:18:17.034200702Z', '7ec3b21389de698a', 'legal_versions', '\x82ec1b3591910511633651b64fb44219c26df2cfa80020521862afc48dfd19ef', '\x01af4520973bf4d49987e7c78b501fd5755cbfb10aee913208aec91fe1874c2b655bafcb5c44d1eb908fb0922d5a75090a69910eec446ab2a90b989535549aad5a', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 266, '2026-11-21T15:18:17.037782956Z', '7ec3b21389de698a', 'legal_read', '\x82ec1b3591910511633651b64fb44219c26df2cfa80020521862afc48dfd19ef', '\x01d20e53df30576fc295f18cb69e3268affc6fb06356b3e1423ec32dbf33606b4fb403c823c39043d95ffece7f8484a0157a0f871036c66f1cdb50c812abcc95ad', '9f72b12eec27fb0359b2e07d6ac654ae', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 268, '2026-11-21T15:18:17.04057137Z', '7ec3b21389de698a', 'legal_versions', '\x2f2ee5d0941c476829b02a1eb479ae775b570d35d9d690f08d88a4cd8b23d934', '\x019559319513813a11b89d5024643ba83023463a2a8bc8f08d28c96feb042f05c10a7d51b1168eb49fdf8598779c7d93a9d52a66fcca0053d30263d65bdfffde74', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 270, '2026-11-21T15:18:17.044238968Z', '7ec3b21389de698a', 'legal_read', '\x2f2ee5d0941c476829b02a1eb479ae775b570d35d9d690f08d88a4cd8b23d934', '\x0147cc6ff1afd9ebf8c79e7da6805c73571cf54a38a376ed543332a64fcb8d367c95f51016bb769d4ef07ff31d0680e1f5b2af9ce8473d864add9d300137655a7a', '3085a8236d4dc94fc3720199c905615f', 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 272, '2026-11-21T15:18:17.046974187Z', '7ec3b21389de698a', 'legal_versions', '\xbfc819b7d7f1b91f22211cbae8a87d2bb1fc1bd9e3f6641250e97b1ba7177854', '\x016933d263ee4ff27be83cfcf91b1db9a40a4c44aec168efbce87961b7d91b69a6faa4583b95a3de4c57f7537270c71e9cb19ce775c00a70035c257c924b4accc6', NULL, 200, 1, false, '\x6e0d68c47f6655b8');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 274, '2026-11-21T15:18:17.050684396Z', '7ec3b21389de698a', 'legal_read', '\xbfc819b7d7f1b91f22211cbae8a87d2bb1fc1bd9e3f6641250e97b1ba7177854', '\x018bae133c81848ca49d1c5445ac5b98fe2d4e2834b0acacd01df5e5420ec87baaf71f667065eab874f273383792c5b0844174ba424305210684bb4cf90ddc7e39', 'a51a9c26635c3af10bcfcb2227850803', 200, 1, false, '\x6e0d68c47f6655b8');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x5f0296f9521e24b7', 1, '\x01d41b9bc8582d0a9fbe7e9c8b6428a62df4609c390b5d364b2352da2b27f6ce04d76630f085cc5d7b5dfcd011df6c2f7fba6a5f0f31ead4e613af898b');
INSERT INTO vcon.audit_key VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x6e0d68c47f6655b8', 1, '\x012051153c74de8bd2e681efb536486e1064325698359065e3e0a96e41dcdfb4b5e3d7b1cecdb40aea02be10d6566c4ab6201c9b2c423230072e4e84c6');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x92db71faa62eee79', 14);
INSERT INTO vcon.key_use VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x331efada2a54f23d', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', 2, 1795274296624510779);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', 4, 1795274296743361274);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x5f5be78f8ec694d9', 2, 1795274273331564657);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x96765f24bf544b47622548ba1d363314', '\xae9209d8c518001139b09e3c6adb06de', '\x0195d5ba18d80c542ba125f2dc388b53fe62fa5229fb343892c9ecf101ba3e9ae5a8df', 1795274288398269532);
INSERT INTO vcon.membership VALUES ('\x8fbfa4b6428b806e237e0c8ca0830349', '\x46fb33e90c78285745052227167e8d3e', '\x01aea5336a22ee463e38aca8bb62e01b204e9d17ba66ea868d00e25e6862c0f96ca6c07a', 1795274289958362969);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x486222c4e9e0686514243db3815a6715', '2026-11-21', 12, 12, 7404, 1795274296493295817, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x7ea304f6be497a652076faa95567a97e', '2026-11-21', 10, 10, 6240, 1795274296639819352, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x87a5da2e94a3531a');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-21 15:17:51.686609+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-21 15:17:51.71879+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-21 15:17:51.725227+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-21 15:17:51.725707+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-21 15:17:51.727726+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-21 15:17:51.730572+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-21 15:17:51.731081+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-21 15:17:51.736004+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-21 15:17:51.738507+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-21 15:17:51.739066+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-21 15:17:51.740182+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-21 15:17:51.743795+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-21 15:17:51.749236+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '081378ce9f917e68c504b73c7c10ad72');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x9a27b4c70cb9d3279a9a2ca68b16a6625cda6174b5c1910e82333a12cb60fcc2', '\x01e727b7a955c53d44b52fb378a3940906e7fd3f07558462d98f35efcc91a2b266d1ee7737', NULL, NULL, 1795274273344102441, '\x92db71faa62eee79', '\x01c47430a9d61e462b99c36a6f15036501648b2e160bd941d7da65e35641197de42c3f672d62d573ae57942fb01b297495c66443200d3de83b3e69b36e', NULL, 1795274273344102441, NULL);
INSERT INTO vcon.tenant VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x8b1f6334c29c179e24efdf182598d9b098491415b3828366c71f752b01921cba', '\x016bd31756c3e2da45589007429dc53edf1e56988b58cc9b61ef8a023e45e07302ca8cb056', '\x0130fdb2318661c0f9f8e613fa6d9f1c2197d23180e54171bf8712a18596e7', NULL, 1795274278137358248, '\x331efada2a54f23d', '\x0118c7b949957e58bae20af7425bd3e9358e1821bccfd2079a289b6c951fda12e7d86ba52aac8364a7e446293b98f673de1084eb53ab8378ace535ca1c', NULL, 1795274278137358248, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xe1b7389dc1b86fe5');
INSERT INTO vcon.tenant_catalog VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x82e8b936bab965bd');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xae9209d8c518001139b09e3c6adb06de');
INSERT INTO vcon.tenant_lock VALUES ('\x46fb33e90c78285745052227167e8d3e');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x33488eb97de568996292ca6cc83871ff5425fdbd54b71b6caeed488dd4e364e9', '\x01ec533075334faeb11ac2226bf3d5059485b572d3284204b6a62c87d6f59788f72b3a0bd8e71cde9637e062a5', 1795274292173660985);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x96765f24bf544b47622548ba1d363314', '\x01d46a6e8083bb73934f09e5eeb5edf0b37fae2f628dcaa349e4905e91476a7c3034412fe28b58c9c51101', NULL, NULL, 1795274285376828573, NULL);
INSERT INTO vcon.user_account VALUES ('\x8fbfa4b6428b806e237e0c8ca0830349', '\x01e574c1432f94308f6e1e7d90a563c6a4a4a73debdf4bf5b5290347584ab1bbb5724acfcc579ff8056a4f90', '\xb344fb0465cf8a1476f10427d0a4ab418199845f8aa25ac0d49bd4aa59b2bda0', '\x016cbc1a8935b63258d13192e20024792f39833c3e58856141b80d5221da5b0739a86247649cebf6e7793bccfe9010c7fafe9359be5df84d898d92bbce7f7bb6', 1795274286889405193, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x3e91ae6c8c1ee525a5119fa380a6d24521f1c837269c7d2651ad7f49715ff726', '\x015aa9e5ccc6d2e511ddba8c58dff747c1aa59df25ff6687e4adbf47a8f087dbaab224c338d667409b8cb838f7', '\x13a86df30049927d2fb0d66c7b2a2110', NULL, 0, 1795274292011742201);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xf845a63c58bed9d6c877a9bce8be604c76e5c39c4d749836c90ac7843503165d', '\x014a42ee9002f9a22ec693b2f3fd8f9834b6c3a4e3162099de25305772466a12f52bb5aa225e7816ecccded639', '\xab153bd041a333f65664b100f1eb387c', NULL, 0, 1795274292020224398);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x5b682d02f992151b19c118a4a3564c9dd64c3f5ac4514806951f0ccbd7e8e59b', '\x017468c9d199494c762b2f1f3bd56686d54721adff97d6d3f3d0b63aacf89d0ada43d45b45ba5ee9dce4a646fd', '\x449f0c2a02ddcd495bb011f7a9728801', NULL, 0, 1795274292028248453);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xdd469d60ea44d20b7c4a4bb92eee756eeb6867065829eb7cd5cdf2b95746ef71', '\x0195e2851aa9f9944c51019a927745b00314223c7bfc7f5697f02e0f4b2a120ce853f494cb75ced2df4bb55dac', '\xd749839ad7da34808e31bcb96f5e28d6', NULL, 0, 1795274292036204812);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x4d95d5a0147b0508c1758282a7dd7fbe7e94f077a8495d34f10bc498eee6ae36', '\x01e16d4c6cacec8b9b61e5c1f6b98e10fe11faa5b7db80996c510754321a6a14ddd0b7a48c7d9c1d189e9ec060', '\x11c22862c9af250a39ed5051cf4ab315', NULL, 0, 1795274291958618016);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x76bc9d5a69ab0c56db549794cdedc977b8e20eebacc79db600b55c55d29e8aa2', '\x01546ca81c70d9308c1d58a06621d422f2b14a793da019ccf3562512ae69d968a7c1c19e5bbd8742db10de42f8', '\x19303f037846a48eb00caa0af9ff2d43', NULL, 0, 1795274291968631940);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xe9442780cddab7b0d71211947a6bacbd121d2d62c1ea108e253ed3ef1f0509dc', '\x019bd47ba182b3884808c1647c5337922f4823c7ead4a03f7f04bbe912628c9b9a2171ef6d7430424aa8a7b649', NULL, 1795274292058533412, 0, 1795274291977117647);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xb3c0824c919c49882842f024b418842d9e1a673ada6c7bedacdb703877019b4f', '\x01b45ad9e396f08bc5e3ceba4965ac8a1f06f638ad0b185aeeb202791fddd0d758634f1cd708f20b984bacef1f', NULL, 1795274292064740061, 0, 1795274291985328961);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xd8f537014ffe36d3f35f0a0271154644ad0d0f6681940d904b0b816f5cfb41d0', '\x0134814abdbdb8e63ee6d7f5b81b315b13a4399332783117bcba59970c8503581f6d3fdf2ccc3375c6f17b4520', NULL, 1795274292073403657, 1, 1795274291994870326);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x3754b31d7f4e0437986716ad5ce76ebbcdb8de1d929748dc9ec750250769850b', '\x01126c020c7238eec088ef7767657e7b69348b5897fc776be23c17495028c6fd514837a98d78545e3367a040ae', '\x6f5414f3b68fec4d24534211ce1eaa5d', NULL, 1, 1795274292003777888);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xb17f2ed47c4d9741e219e6b3bad4621ab1b84a13c8a6f458a682d69fc92f2022', '\x01d8c70dadc21be678a2af4422693a4fcbd431b45c2c1c6ae8339beeb750e284e6090c5e099360191c060a6a54', '\xd80af39bc74cbcbec9bef220a073c102', NULL, 0, 1795274292120096525);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x0fecbfe670d080d524ea34c2f8b84e3a67ba782c8ef101fd551fdc5b8fda96bb', '\x0173f658040a64296d0538a2dfece42e9a817b17704ab3b6b9a74ce9f111361209badd0c75aa315657a3fd5fd6', '\x9385e49b17fadb56233afa180805b0f0', NULL, 0, 1795274292127785845);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x4293ab16459a40e5675ef8d63412db3738b368f59ca59c5cc3aa910d4019779d', '\x016eedcf99de501d9b7a3675d4f2f0bb3c61392fc4eda4814676f965dbef7012b8b48bc017559243b948b78d2b', '\x2f6d572db1e50423b1ca6f1c470713ce', NULL, 0, 1795274292135671250);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xaa11fe0f6590a58cdf2fc9324d38c2e158e72427c96216a0775187bb9d459c73', '\x0125fa92058df7f01c1150868821e8d7597900fc082902915e5ae1096936f5682633ba2d105954345f3d81feea', '\xbfdfedde205237afa5df9d01aa52ce1a', NULL, 0, 1795274292143617109);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x5edc9f84e4afc2f158ca531c6e6b972a1a76d00fcd6231e4fa2d3d64858820bb', '\x018c3aa2e320b10c2e800134f4edceffe34699ea695cd81092d3373730d9eb472150a1591f485630363f5d6e13', '\xc6feb9fa21397712cfe386dbb485514d', NULL, 0, 1795274292151388716);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x7e13974238491e5ec3539a67a0cb32a52fbb42415ed497589a6ded9fba65bd4e', '\x0150c8401b3fab6ab327f284f2cf0a7de7346036a1fa940b275e3fc7261c89a3bf854c3b2378c9ffc205a2e294', '\x9f72b12eec27fb0359b2e07d6ac654ae', NULL, 0, 1795274292159206380);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x9c84c5a76e306d20e2f172c53cbc8618c9472c0da5da01fd6c83fbe3e827f5e8', '\x011762f333d414bf4d22fc788601c0e1c1a1828d9ab5d2bcdeede273a78d761eee8f3bebf7625104e0236e5798', '\x8f585f4731fb3fe7d1015d189c64d233', NULL, 0, 1795274292087518720);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x688f926ee71cb85219024cb2ac55fbb9e689048b7ec9c6a78ae4164d9f4ad0bd', '\x01c6a6ed53b54e652f70baf22fa902b85f40f5e7d38cd55d6122995e6b58e8e62693e676ad3829f5452fc86608', NULL, 1795274292196443265, 1, 1795274292103943869);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x93ffdc65678b41c929cf31d2ef76b13d4425c980e8cc66653c711c20dd561ebe', '\x012eb5ee87bdce3443e38ea5ad6a1e812f644e73020c2a9b001a45dd41c9387158b3779c70998e3842d3aa6e36', '\xb8c07b0d66e9e95d9c321960dcd1dba3', NULL, 1, 1795274292111919571);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x5dcdf95eeeef09f6ea61878abdc0e2effbb42ea3532c261e5afca25f004d1373', '\x01ec27452eb8b4ae7baae4f2222bc09670748077b37e3d83df4ba307b3d4cadc11b552bc417ce1794281ca8c8d', '\xb177b7e4b740ae575e016adbb4e091d8', NULL, 0, 1795274296852893203);
INSERT INTO vcon.vcon VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xe05f656b7184c104bdebf19a72869da3b93239268fd59e0c94e88bf6bc0a16af', '\x013566270406c75fba9afb6ebb8641e28e9efec02dae7095ea0ec08dac4c29b042d46c0751cae173cacdd9abcb', '\x8ce7054759d13b3554dba77725561896', NULL, 0, 1795274296862101397);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xba09e73f605698c9feb7d84d0764436d2c4af455bd06e7f9b9a08f5b078be2d6', '\x0110844546f820bdf9a674d027ec7642b794ae9e65676d92533999f00d6e8a62a9520abe622dd7b3da1153125b', '\x3085a8236d4dc94fc3720199c905615f', NULL, 0, 1795274296870252488);
INSERT INTO vcon.vcon VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x80bb2dd9f8c93b2442254c7011d5e50ffd14c7b306232062a1ab4d5114cc2414', '\x01eaee9332ec68d61491a48894e152cbc0c5d5963f10df6ff7be541a9e013d5980c17c606fb627102a37524b0c', '\xa51a9c26635c3af10bcfcb2227850803', NULL, 0, 1795274296878278190);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xc5a552d39ad97ad7603c0220027f9144', '\x688f926ee71cb85219024cb2ac55fbb9e689048b7ec9c6a78ae4164d9f4ad0bd', '\x01e783c4fbb4c013df4e03069cdf99e44c7518b90b0ee0eafd58bd8a7b87b670e9b583a1e0', '\x017237992a2f6ad89daac4cfac5224964bccaa5fa98862e1c824a719b3db', '\x01197ed30500fe0417b1f5accc6d226b36881f2229179819f1418eaee0751c3ae16891a82e', 1795274292103943869, 1795274292196443265, '\x331efada2a54f23d', '\x01935103f842f735098433ef8a9eb0e3daebd2ffcfb5336e876b1d6da016f2ebf51c9ac2b054542149024b4448b3ffcbf42566fbbd94f6e3754375fd63', '\x7ea304f6be497a652076faa95567a97e', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x51695d266cb91e42c6d854a284face4d', '\x93ffdc65678b41c929cf31d2ef76b13d4425c980e8cc66653c711c20dd561ebe', '\x01d02cf9eeb9ef5478df706fa423525bb0ed39464b2d4fee5295c9626035cbb1d35e85da06', '\x0173bfd50a9508aa665f5b3c8f37746fc516d0b5d4740a0ea021c9f9c317', '\x0193fb4d77a8e7ec0cbc000dd30420d716f50427fdd7c93ba8db6befed129586911a14b2fb', 1795274292111919571, 1795274292213503689, '\x331efada2a54f23d', '\x0182ba2b89a3015d7ec38000df173ffdb39b5eec8751e072378e4fb116fe31f91dc100e70aabca96b2bac9c8c378f0e7591c4fb1d749f6e50e538d2b71', '\x7ea304f6be497a652076faa95567a97e', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xd80af39bc74cbcbec9bef220a073c102', '\xb17f2ed47c4d9741e219e6b3bad4621ab1b84a13c8a6f458a682d69fc92f2022', '\x01788b96957175215c87b5c4b49916be158a8b6f77e2137ea46e7f5dd017fd0da24fef651d', '\x01553ed20a309e8bf79e3c3f0ed9068929301820216fd578829104ff66f5', '\x015baefe99a338615a3d3338b0c220381748d9d6950c26b33a03593c3942b6789b7d3c34c2', 1795274292120096525, NULL, '\x331efada2a54f23d', '\x014be029d5ce025b2b320045b87bb8cd1c34573720f4a8b49cd0251cb2c87139cdcbac87dfaefbb029eb300044b6c0a5761503a40607dc0b2252a435b9', '\x7ea304f6be497a652076faa95567a97e', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x9385e49b17fadb56233afa180805b0f0', '\x0fecbfe670d080d524ea34c2f8b84e3a67ba782c8ef101fd551fdc5b8fda96bb', '\x01ec0fef31fa8da71cb38fc52be27c07455dc9183e720df099663bbc57cf578eebd3bbaba0', '\x011667816de572df1dd5129f1a84f32d2faf7343e41450e65f229ba1bb6f', '\x01d01c2fb5bcdd68390964aa0e3c316fc26cbf78fbd32b9fc488a06744816bfd0a7f8f782e', 1795274292127785845, NULL, '\x331efada2a54f23d', '\x0184cfed2afd9a3519c97ee1c9aca5c3456f41a6804848856a22a30825ca8aada553f585a64475b2d705d898cbe49815c2ef15ea8229e6d98be119ff0c', '\x7ea304f6be497a652076faa95567a97e', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x2f6d572db1e50423b1ca6f1c470713ce', '\x4293ab16459a40e5675ef8d63412db3738b368f59ca59c5cc3aa910d4019779d', '\x0181efc6e5256eaebd4b9dc1a691c66871f972d16b7e7ba8d05457f9ac2762e79eeca0649c', '\x01ca4ece65f49f5f56c3b2464945486927d789eac7ab43d9558923a2cf31', '\x01bf99e67c5cdef7f095b4c94aecbe5c9aa8af7f17c52a06152219cc316e543ebaec6412cc', 1795274292135671250, NULL, '\x331efada2a54f23d', '\x01245393d891d6ee99a81a6c37b513860eb98981ef238676e2a463ebabdce1fb7c1c1d2135be41f39af98741c8f1e3847fb36e2cc83471222ab8773640', '\x7ea304f6be497a652076faa95567a97e', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xac0323f72e2f01a6c48243679229ddc8', '\x76bc9d5a69ab0c56db549794cdedc977b8e20eebacc79db600b55c55d29e8aa2', '\x0196048260db0bc5960fff049caf498282cf95f0a34dcb08b1de2c3e1827cf544a39fd8ae0', '\x019ac1152d608ec3546153c67611d15f45ab7602c201727a3e878469e1b0', '\x01c97710f7151cc73bd96ef0e01fa8cdd2627e2e33efa34842273ab301ec85be7f87a148c3', 1795274291968631940, 1795274292054594256, '\x92db71faa62eee79', '\x0163847706e3d32b8e46b531bac5c0fd897306d2101ef50381a7832d39b82520c44d9d09a3561ebe46dee8261124ab8514f7e0c1a7e68c9470bdb09965', '\x486222c4e9e0686514243db3815a6715', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xd77070b1f61f4468151bdd57c49c8179', '\x4d95d5a0147b0508c1758282a7dd7fbe7e94f077a8495d34f10bc498eee6ae36', '\x0121d4b9a71ff3f048b9dc15444d849ba605a18768220a9b6611285e8d2b2003c38525fc81', '\x01b8693821624ff57ef3477b75743f95ca94a84b0fae13e6f791196b157c', '\x0134aafc46f0ce8ded109161d7241e9bc9cf8423d9d503aa57756b69c8e3d44f66360d8b1b', 1795274291958618016, 1795274292045553656, '\x92db71faa62eee79', '\x01925f514c4fb26601b4455ff839a91908dc6277a589178351a319bc1e34146400581bf89cbc95d62a48d84604f57f0e919cd05f8d05b05f004c5dab56', '\x486222c4e9e0686514243db3815a6715', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x9c6111c70ce30631f2b1e0d35cd2127d', '\xe9442780cddab7b0d71211947a6bacbd121d2d62c1ea108e253ed3ef1f0509dc', '\x01c08287aefd2f60a6eb5d8c5358e3c14fe3b4bd576ee0c4ca8cb0edaa3b15114e3416af86', '\x0101bee6ba156b297542a460de047ef0c142091f55e470003419639ff1bf', '\x01c669bdbabe7abedc5197605f0073aab1ccccd5adfbcd6f3e9be21e2e29efb81f83e63730', 1795274291977117647, 1795274292058533412, '\x92db71faa62eee79', '\x01d12a22f23d4f92e7b9ae266d8cc1634484f48f4dedaa98f59afb19140c3acf5f1f3dc306c94c6887bc4bddf641f59c50427bc912bba9e138f2fc8587', '\x486222c4e9e0686514243db3815a6715', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xc6e1dfe9f11fdc392c7ade63ba11f83d', '\xb3c0824c919c49882842f024b418842d9e1a673ada6c7bedacdb703877019b4f', '\x01f4a020122faad23f8f6a7e3e216b11d57354fbf622ad1889da6ac42cdbc48fb581ffcb5f', '\x01ccbc7529c4365b6fafbd3bcfee4273c1e03f5e62f416a50226fa2c3a82', '\x011fb044e932ec960a946ef9275fa173939603c3dbc0fb0ec801b8f3766900e09fa2d8975b', 1795274291985328961, 1795274292064740061, '\x92db71faa62eee79', '\x01d00e5bd4a2f30048add8bc3e60785b58257d31798576a6ffb3a5d32448e317a761e1220f05729a27f059fb9076d7bb42397cdbca79063906f89b1b95', '\x486222c4e9e0686514243db3815a6715', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xbfdfedde205237afa5df9d01aa52ce1a', '\xaa11fe0f6590a58cdf2fc9324d38c2e158e72427c96216a0775187bb9d459c73', '\x01cb894ea15a5e00f7c7b97920f03bff4793172e1041aa752d19c0cdcba682230700860120', '\x0199da6a9f60f5866d153c7661deda7a20b3ca4bf7c28d17bbc691e0a1b9', '\x01a57cdfa9173855da556e04ae563f240c097ba6c72741764db3c618514ab155c6f3bbde1f', 1795274292143617109, NULL, '\x331efada2a54f23d', '\x016c8a37f192db97408bef9d92ada5d21e8da992b465a5daeca22988c011318fbe2c5d0d6911fe553ef5e3f68c86a586f03cdb39ea5fdade14c2f8842d', '\x7ea304f6be497a652076faa95567a97e', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xc6feb9fa21397712cfe386dbb485514d', '\x5edc9f84e4afc2f158ca531c6e6b972a1a76d00fcd6231e4fa2d3d64858820bb', '\x01dd543a94b1d3a651c363b65bae21a5ad7172aee77381fd730f291d4a15fa473bb96b2e3a', '\x0137f11fe4a595cb492baf210a9098aefed25ebe6734a7cbbd2651fdbdcc', '\x01a543dbbe563b811cbf8384256d046a89bc3bb1ae5e93c4290a974e475f746aa6c6ba5d72', 1795274292151388716, NULL, '\x331efada2a54f23d', '\x017410afef96d6108f82e3a3ac0d2402426b9818e1b6b498e6b21218fec3d347a1e46326644477a4531b0f4b7075ffdf8152393ebc80d9b66b34586533', '\x7ea304f6be497a652076faa95567a97e', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x9f72b12eec27fb0359b2e07d6ac654ae', '\x7e13974238491e5ec3539a67a0cb32a52fbb42415ed497589a6ded9fba65bd4e', '\x0126e30c55888998c10fc3cea06ec48d1db9cf826a38c7bc18193d7262743d69bd1b3e8996', '\x01be8c97e6bcc42cbadafd24abddf9ebb4b5db8440f8d2b1bb8cfd6af5c9', '\x013a546dc1bd0de63aca6485c5c1b16e4db93fd21bb335270ae24023184c9ff3f0cfcdf24b', 1795274292159206380, NULL, '\x331efada2a54f23d', '\x010042a7da49f75cee6ec64195309cbd58d5915d14d89b9bf5a701f5faee40062c943eebcdf8bef145a5c07e868acc914e26e940d869869fefa2453c21', '\x7ea304f6be497a652076faa95567a97e', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x8f585f4731fb3fe7d1015d189c64d233', '\x9c84c5a76e306d20e2f172c53cbc8618c9472c0da5da01fd6c83fbe3e827f5e8', '\x01af99a69713e097219f2d53050b74ac674348b57ae63853601922beced65086f758862860', '\x01fd11faed3e5eec4cb6be050864c31484a02e31d6a67e74bbe0df616814', '\x0192fdb54b29b230e6f8b45a94128de29fb6ba5cc6f1627e1578b54d800055964674df1697', 1795274292168011127, NULL, '\x331efada2a54f23d', '\x013863a52219b3729ff4afb4bccc05a3d4a1eff5dfa8d7633b340c734e0b6be2246582cd748e53e4d0fe3d289091799cf9a6bd4b08b532e552763b1fea', '\x7ea304f6be497a652076faa95567a97e', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xb8c07b0d66e9e95d9c321960dcd1dba3', '\x93ffdc65678b41c929cf31d2ef76b13d4425c980e8cc66653c711c20dd561ebe', '\x0117e9ee310db1f54d1e4a048681481bf45ef7feb21a7e1c2cae93842d5284de9fd13f70d0', '\x01d9225a93dc76cf60829c1e0f31ca8dec451cec404b238173e46522291b', '\x01810f66920ce50bf36a24fb0793d41814f4e09ef8cff2cb1428aecca3f77bfafe19985020', 1795274292213503689, NULL, '\x331efada2a54f23d', '\x01ac08483eba1080848009fe1d3a387b88d8ee3f01ae64a93424a58684d80080960b3fe78f3ae2b2c075feb97dac1455125bfa13489b34fa518f34c486', '\x7ea304f6be497a652076faa95567a97e', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xe837e126b62a0a86daf9a70db1fa46aa', '\xd8f537014ffe36d3f35f0a0271154644ad0d0f6681940d904b0b816f5cfb41d0', '\x01c0e3679e03df4f786ad031346b42c24df51e2c8b2bfbde537dd88a9c03f00595f736850e', '\x01e980f0fbd3ae4b096dacd2c3e6389cfebc12e5b9a9ef248da45cebcdf0', '\x01f4097e8f5a4769d0ecddb0f80e5bf337e7457c720b623ab39fb92bcaa0fe153d6c442a0d', 1795274291994870326, 1795274292073403657, '\x92db71faa62eee79', '\x01ab9f391d29ae23b7ec4809a582c567ae9efa06c1976f62dc5f80076885a1c522d7a1ae429099f209924e3670dd7267baeb0f5ec08dc6eb7a1036b140', '\x486222c4e9e0686514243db3815a6715', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x6f5414f3b68fec4d24534211ce1eaa5d', '\x3754b31d7f4e0437986716ad5ce76ebbcdb8de1d929748dc9ec750250769850b', '\x01e022cb0d052516e4d9e7a8324a7930ce61200347c0f6765e05d4705c606803d31eb21ec7', '\x010b6e116daa4bf8bce53d091d844f951d2fcce026aaceb75d368f8c69a4', '\x015601f3e5f172fa0bc36815bddb9b61cdb74d87a0a4019b4432e0e4cc26bee928216a51b5', 1795274292003777888, NULL, '\x92db71faa62eee79', '\x018341a73cd7c856ad8c25c96b4de0dc4e8c22ef57a31c2b1d808842bd620c5aa53fd22d08b2ff26c5ae8ca2f11983e96aa9c1ddf68519fb26546e49eb', '\x486222c4e9e0686514243db3815a6715', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x13a86df30049927d2fb0d66c7b2a2110', '\x3e91ae6c8c1ee525a5119fa380a6d24521f1c837269c7d2651ad7f49715ff726', '\x010b56e4ba4bebaaf86cda0f8d5484284058c50855a68517621b6b65c7c425966ba1532e8e', '\x016be6b49ae626c228c49d361092ae2c50670a5bd436034327cd733ddaea', '\x014db3375bde625637aaddfc9bd5cccd30b9bc551cddf72c8cece614e5311241d110f3b916', 1795274292011742201, NULL, '\x92db71faa62eee79', '\x01a8a330dd0c583a4849d439c856d6d50908fc128514d3389c55a349e13e21765273d73a0b37b5cf4f361d4b26e8a1fdeea2dc22c3ec767ef654152b56', '\x486222c4e9e0686514243db3815a6715', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xab153bd041a333f65664b100f1eb387c', '\xf845a63c58bed9d6c877a9bce8be604c76e5c39c4d749836c90ac7843503165d', '\x01e74a1f4a992e364b633c16a65dbd03f388192a4e21d57adc37a89bf1c706f616f401638a', '\x01f58cd86936aaec11e4e047c6261539ccbc411971bcf010eb899483fd1a', '\x01d7cb1b98f06b122d258ac7ff59d0ccb63faeb731241302b1d16c08927be57ad6ae31305e', 1795274292020224398, NULL, '\x92db71faa62eee79', '\x012084832ab19c4e1abdbd4f87c1ac3662e6f054842cc32946f5954b70c547a7b7b07992e888259286ab346c630ec19ea41df952973af5e815c951f919', '\x486222c4e9e0686514243db3815a6715', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x449f0c2a02ddcd495bb011f7a9728801', '\x5b682d02f992151b19c118a4a3564c9dd64c3f5ac4514806951f0ccbd7e8e59b', '\x013d100bbb58bb73b8761b91327827c5c089810aee0391eff4cf4be7bfeb6449d06257ee0f', '\x013a79bb5261c29693e2129a5f2878b9e30039e5d1322cb4a386171dc58c', '\x01fe5ee938ae7a6c606a968ade197a58d52b6a3e5bd2e54dd3ffcaf0292a56dba527f25d37', 1795274292028248453, NULL, '\x92db71faa62eee79', '\x013e71ed1bd7e6a52b5885c223742b0a21316cc1b179aa6c518ce9de332f7fccec4ab70edb5ff5bff9fd60adc7e48b1376602039371a38740f005197f8', '\x486222c4e9e0686514243db3815a6715', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xd749839ad7da34808e31bcb96f5e28d6', '\xdd469d60ea44d20b7c4a4bb92eee756eeb6867065829eb7cd5cdf2b95746ef71', '\x0112edc91b39c4b119a49ac55cb77ba4e37ff3af569535097f9d12b6320b6c5691cca72c81', '\x01429adfb104404a0ef2e4d940915143356814ecbcb1a35723d297ffba16', '\x01b02940f40163b95b73edf79cb247484f3cd89313781ab6bef3d6eb0d294d5cb970d9dd19', 1795274292036204812, NULL, '\x92db71faa62eee79', '\x0117f3f2eca63da2de2c13e5c2a214782b795343497b18090692bb4eebb7b30bf229af9ae206256b503b7d67d75c851b08bd4a145025727885ff2a7c57', '\x486222c4e9e0686514243db3815a6715', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x11c22862c9af250a39ed5051cf4ab315', '\x4d95d5a0147b0508c1758282a7dd7fbe7e94f077a8495d34f10bc498eee6ae36', '\x01ce84baa2323d264a2d9b29be794a8005f85e973cbd8fb6491ab7047c63fe460e930d820c', '\x0153a7189c9cb8227e68187bb7d913803db472682f54e7f6b5103c175d5f', '\x0111fa0d2dfcaefc3e01bf70ef13c1099470601f943c66f4bf7cac1fea28cc1529da406d1b', 1795274292045553656, NULL, '\x92db71faa62eee79', '\x016bdf02bcc78889d3a4dac89ebd6a0a8d4d542cf62d1e6ad8da9198fc9d7aebca231870d20258b0e5e149bf1bd45088d7013c2b57633dd141a0505f47', '\x486222c4e9e0686514243db3815a6715', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x19303f037846a48eb00caa0af9ff2d43', '\x76bc9d5a69ab0c56db549794cdedc977b8e20eebacc79db600b55c55d29e8aa2', '\x01f11f0bd5dab892adc485cda5574656113a89f144e871c0254a8ff511ef51be5f474e8794', '\x01e054d39bfed0f3f3a7bce316bd46abe5936f5c6031be87d15fc7006397', '\x01fc7404b32494c80dcac575e0fbd0f3607fb376dfff17e081ca9e49be1d25bc192753d1be', 1795274292054594256, NULL, '\x92db71faa62eee79', '\x01e70a4bc741cc84c851c67d61ec7602dc3e4e5ca049cfbde1805f3567c83587b4ffcc7c2bfadf0912a84530376299142a3fe728f3b8ab06e4158af6cd', '\x486222c4e9e0686514243db3815a6715', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\xb177b7e4b740ae575e016adbb4e091d8', '\x5dcdf95eeeef09f6ea61878abdc0e2effbb42ea3532c261e5afca25f004d1373', '\x0190b87cd13d8d9a644deae35434549dfc6432e04a095d2269e9562a65ef5f440c5f2e97d3', '\x014e2bfa5fa434c0a5fc6679f85ccdf59ea9d5f0f20b0883f271891d60b2', '\x01e26130be83f1407afad2930459bd1ecf77d653ee419bfb45977bf030f29c0c113931b15b', 1795274296852893203, NULL, '\x92db71faa62eee79', '\x01d90c881b841bb818dcfce05b60c402812a0b963643521133d0d59567517b78ee5886b6e0827824c3f452157bcd7abdbe42e775a424974c1dd50f2e64', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xae9209d8c518001139b09e3c6adb06de', '\x8ce7054759d13b3554dba77725561896', '\xe05f656b7184c104bdebf19a72869da3b93239268fd59e0c94e88bf6bc0a16af', '\x0136354b7535f73444788ccfa292abf0a4cf2652c7f487fe56ee7579414075d28e98859af7', '\x01262a31d36d4efff1f2e53c937add304cdbb16cc40f2e3c23ce0a872832', '\x01917fc5705884b27b8b87e55e8be6fd05c8c15e5720f5fb9ebe7e91208f6a6d0b2e0d7950', 1795274296862101397, NULL, '\x92db71faa62eee79', '\x011de22b04eded11def8fbae628656dd5eb9c7606329cbc9ec7310867e5d02af074bd2737f88e2b977ce5fb4fd0262894c1b4fa57a7d36013df25f13e0', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\x3085a8236d4dc94fc3720199c905615f', '\xba09e73f605698c9feb7d84d0764436d2c4af455bd06e7f9b9a08f5b078be2d6', '\x01967f55bea1dd0c4b8780a87d0f54769b26ae6739f3565e2ec20031cd7654bc9d45daf752', '\x01a3afed88ae75f5e01b9d8b6a010fc3242fa602d5d5430eb636d355a186', '\x01ec07c6a012c67869970cad8b464aa9ba60d89e0dcd7d1ad35ef14d0ea67ac63803b374d5', 1795274296870252488, NULL, '\x331efada2a54f23d', '\x01eea4b234123018684ef09d08458e070e4f77fd299ae91df41118cd0e8ad69045d0619162f68335cd09632501d3cfe18d0c3e56240e22049676ebf8e9', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x46fb33e90c78285745052227167e8d3e', '\xa51a9c26635c3af10bcfcb2227850803', '\x80bb2dd9f8c93b2442254c7011d5e50ffd14c7b306232062a1ab4d5114cc2414', '\x01e9dcfcf9c3d8d25a02ed6b34034ce807ec98366b0fe5040c19fd3435dbfcc4d9dad2a5d7', '\x017f125bb43a9c374142a571c98090725ac0356117731589b10597ed790d', '\x01bea1cf12cadf4d28edf5ffdde6ca4adba92e5affc0032d836f9dc79ecc76520fce4dcc17', 1795274296878278190, NULL, '\x331efada2a54f23d', '\x01e625d0d623dca49aa0bfbf7ecceeaa6e28856c2372d20a694217f6331ac987d3ffab72b79e31e958998a9f56153b92708b00c26561a896080dd611e3', NULL, NULL, NULL, 56);


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


