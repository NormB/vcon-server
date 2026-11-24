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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-24T07:07:53.787871726Z', 'cli', 'tenant_create', '\x9f9617d3dfdd9e27d0a853bda0f0fa8a70060f826a35275bb2b0767102048600', '\x01a27a96b6e49ce87fbc9a58ab073d0ddd99543d8da090f7e5cbdb38c619d9877112f1f2aff14b7fafeba88d4d489b93f922eb88ff04a9823105416681', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-24T07:07:56.162938498Z', 'cli', 'key_create', '\xc1f7d818dd195ca959b2a497fa38704b92bf8443f3007b48bfeb869b999332b0', '\x012776a2ed01fb95766a51593c297c8e01cb6d9ede51f8f84bc770406ac2cd3f2b500525b9cafbc284757e1394', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-24T07:07:59.066556308Z', 'cli', 'key_create', '\xa9b4faa7ad8b031631cbc3ed76ad95b85d58d928ad203a7666d19abe939cccbe', '\x01676837f1dc804b408d71859853cbfa2e4c586dbc399b438a22eb503889bbddfef510b2e025a3cb02b94c0de5', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-24T07:08:01.979137025Z', 'cli', 'tenant_create', '\x4499c5498afbf6bea4b6d72b9d25af900a796940c7fd77ca3b4b39fcf1c5f138', '\x0143421c3cd4ad79d02e346a2f9c9c6e78ef599ed2f2b748ee75973e75aa83873bcf7c0dec0cb89f18134ca4356ca92e8eae7f1fd85e9c78b457d3d0f5', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-24T07:08:04.819078574Z', 'cli', 'key_create', '\xf11569f12f86aee9aa5666415fefd22a20d8bdd7aff8c8b4656604ba001257a3', '\x01b5443ad944af7e5c47aba883467902cf053d8636305672e344ca75fef7c670649f4e8831bf30f5a29d8f57d8', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-24T07:08:07.035939896Z', 'cli', 'key_create', '\x77ef99d1601503344313ae63ddabf4708819f1bde355ce21bcfb4cbd232a47ff', '\x01d41b506860cc77f7b4d1227eac3b0c685a7aff1332fe34547b07627d503e7f93b892ff11d34afe0de4c96751', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-24T07:08:10.242862542Z', 'cli', 'key_create', '\xd3af2a5b8762d077f6e57c1aa4edca5276c139c26e0f91a7c483ab55c4ed7786', '\x01d88bade2655d602432a2de5cca9569e8427633545ef18336a35f01141cf5c28978d97ed398c895e7bdf30d61', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-24T07:08:14.081871969Z', 'cli', 'user_add', '\xc16c9ef8c4981e437eaae129365273167e1a02a4fada2e279d88796047bc0618', '\x018aad2ad7031ee63447c3b1b28a91a9f40542b85a7ce73747a15741f18e4bb3a5f8bd3fd25acd508f86c49554b0ffe33f4c0d008e9896ada936789cc3', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-24T07:08:16.717076577Z', 'cli', 'user_add', '\xd9201908fc9e6182413e2c4df12a5310d909d4db48ea156be19648966c8c6117', '\x012d158f881f38b4ce97c9b4be3cd48aca790c40383dfe42c7135c0143b8f39be6dde5d356aa4a8892e0644e8b0f0312bbb96d2ddba55db9211fac0463', NULL, 201, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-24T07:08:20.598394007Z', 'cli', 'membership_grant', '\xc046c84b9bf5f3e2907821da3d8406574bca5f540076dc191e5238bcefeb24c0', '\x015d45ec29ba685ee09eafe9f04cb994e05cf62fcabd09aac5d0d83d10f8d44db8b243c499e5cfc3972c5a0ae178c3142c53b144af7b7735a8373fd02c3fe556f601a1c0dc59cad9e65a64f40b9d694273b6cdd01759ccf29fda2aa5278a85ce7e01e5f5e6', NULL, 200, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-24T07:08:23.067964478Z', 'cli', 'membership_grant', '\x30975595689e9324cafedaae58e0bca9b28dc1375e1ee540751f96835e97960f', '\x01c05e5f2e9e6abd3484ebf718d903a31d02ecba7a6bd4bcaf2eb561447665a0f5cccb869ec0dbd83d6119e1c12e8813394226a3f36e79c64f665f2c3a737877c384cc8c20631338d2fcae879d59a5bdc23d89be823e469a67e3c03a716b7915bab721767a35', NULL, 200, 1, false, '\xc26e5d1d6c4e896c');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-24T07:08:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\xc26e5d1d6c4e896c');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\xc26e5d1d6c4e896c', 1, '\x018d8b42c23ffec237b6c1d43d5350677574b5432e9d9412a483c60da1c18e2179527e62758e0df7f11a1bea7ddf93297b734bd005861bf05b2f84db0b');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\xf82eff76b41f45b7', '\xa08b91edfb9326d51465c6015964f940', '\x25aa3d10e3d087f08d70c9f426ed792efdf500ce9dc3ad1e08e31af62ebffe9b', '\x0101385622c5bd43aa4c5530d01badd4906abe2a3e388597843e6a173fa08b24ca', '\x01dbec2a39e3b88b3931a20545f7aeb253e8ae6ec61bbfdbf60e116b1bb0ae29b28475e27ab00e0cff9541b2', 1795504079065588767, NULL, 1803280079000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x7a7cb7472ca59c5a', '\xb3bfb876578ea80322efde7742260ad3', '\xaa0b4da7a1761fdb212c5176b9476658838102e058533350781b8b6a7a3b38b4', '\x01202e8278d7803aa7e4ecefb8b833f465899e7b1717a9f4069eb74ddfd157d5a7', '\x01ea385ded890d50288449bd21d837cd3209c2f72ccc7cfff5fa6bf34bbfb62f3b39b58563029911816c8efe', 1795504087034969187, NULL, 1803280087000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xbfe12b81a4263bf4', NULL, '\x32899ae4fb0c2031a0c06de1737042f058564062a8e16c8cf07d5fc5d132f420', '\x017bbac9b899e7e12a4ef858a2a69400a656ae272e6b81eb778bd3181409d77f2dd4', '\x01836fd59177f0318433ad782fa19f4fb7f3d49c35894bd487a4e1c99c00cbb3', 1795504090242064131, NULL, 1803280090000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x550bb5a41d7834d1', '\xa08b91edfb9326d51465c6015964f940', '\xef93e1a42f904a018265e9beb2ee1621b8a83dd1b662a3d3389c099c46751c26', '\x01186f674f7d1302b7925ea87b6da917fbec5bf3a94174fff6e64d6770b98c35e6ab65b816c0734d77ebbdd996c24a6afe9dc55a8e2f3b367e24', '\x0123b29171b3c9984b3a9b56412a1ef09cdbcc2fb6d027f97a725852cf2def2ed1925d7cd088c20e402cd2532285', 1795504076162240931, NULL, 1803280076000000000, 1795504080000000000);
INSERT INTO vcon.api_key VALUES ('\xb93636aafbcf3853', '\xb3bfb876578ea80322efde7742260ad3', '\x5ac7213e5145e97902ce1b597965767eef0c70c45af530c6e1234b1424212755', '\x0186ff30800c6642f4a87bcbb5d93ee54b2797fb9e2c5f718e9644f595d2adef85e14d1fbd7b9a81e2fb59e202b3cd5eb1d1ee3580e2c999c410', '\x013b22072b8c9b9b64386424cb2a2871da922ecca3446585030b586f072a1ec2229960ff2a39af05f83aa60a2a95', 1795504084818335811, NULL, 1803280084000000000, 1795504080000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 2, '2026-11-24T07:07:56.167772631Z', 'cli', 'key_create', '\x9c41c1dea541594af089ef590ad141cd515019f0287e93ca1c71108b0fa0b5e5', '\x018f4858b7d29c5fdecb4dc6c732c6de26da8cf8f1101aab2e5829e2ccbf3035ff38879e2d8bc797f477a495b0', NULL, 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 4, '2026-11-24T07:07:59.078669896Z', 'cli', 'key_create', '\x74b6e979293947c27b9843081c8f3d884788dcc42b7cb5ca654d001f1df0aa02', '\x019d400549db079c91fb0e5d3fe7cf997f732d95aba9a6d5c12729262ae84f617bbd294b6ab17b8d2ee772529b', NULL, 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 6, '2026-11-24T07:08:04.826095992Z', 'cli', 'key_create', '\x1fe77a8614bed0c3868f4cc3acb06e240fe453004aad772e4fd1fccb90e3be96', '\x0180b68e3b47843a9c3696a23225e4341eeb6b09d1cf8740b4838014333cb66de72284ee1920f49ecaed4bb3e3', NULL, 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 8, '2026-11-24T07:08:07.040498665Z', 'cli', 'key_create', '\xacdca2f8b7e216acfec3107ff98552e64a7800bfc2760153bee374421d722f40', '\x018cf2f818ddc936201164b57328ef205a1d892437c2d91717795513946f830a70e2c86d490a27a40d3e8745f4', NULL, 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 10, '2026-11-24T07:08:20.605128664Z', 'cli', 'membership_grant', '\xddc73dc622ff7976c0efd9fda5b9ab24f643fad4fe1ffad28fda5cfbc692ffa3', '\x01e5293632cbc80dc24e597b9df1174ed933741abe2631f3a39a152fa8010149c9c626cae3ca48b8d69e7c7ec3fee3a8cbe75c4e1941161c49adf3b2f054db1021f6a9e7b17e312f8d985121dbb00460017efa813e150447c6255634195988c45df491f6a5', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 12, '2026-11-24T07:08:23.074078179Z', 'cli', 'membership_grant', '\x36ba5a53479277c9ae9fce9bbd06066f39bea68ebd50f696072f4b291db70d49', '\x01ed846de4d896163f90b1bfba201a3c1ffebe9563486b9444476933f7c539a2317c2b0ffa0fc9619a7c88e0a3b31e4897d056b8c3bc797bb5ebeeac1c6da7e2831743485682b3c4be86e13b1d8bcdc879323afa9c9778cfa37acee845bc43af494fbc46359e', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 14, '2026-11-24T07:08:26.105871208Z', '550bb5a41d7834d1', 'create', '\xe46e1b6465dfa7b31020f5573c69c89094a27254767d16c05b52ef8116d2877a', '\x018ca28bcfcfd18ca84c97642b81a2c9bb9387d80122b20c0f6af974192a60db498a6613f9b2926d5b5bdbf60293846021ede75d5f407b6fa393192cf0ddb1f9da', '9a13d4d8ef63cb902e4fd33bd350d84b', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 16, '2026-11-24T07:08:26.147011681Z', '550bb5a41d7834d1', 'create', '\xf75fd37fe6d641bb2282467513fde559f816e1d170e7801a242d38053b62b9d8', '\x01a406e0e5d2329597ff57e2a7612f751e29ada9bbaf4ad401a360be2a2d8fe55d50099e373c94c4394b5553b6c67e8f1ebc40067ed41d4274e95c91bb81b4eb73', '8a025907bb97a06a57f70457ba5f9ef3', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 18, '2026-11-24T07:08:26.18330954Z', '550bb5a41d7834d1', 'create', '\xa521138c9cfb5ed74f3fce783285656fa7dd5aea512c0bd36f4e51d10bfe1d64', '\x013326082cdeba274987708de9123ae369fb1e893c7f3187cae0b3283233ce77f1d8e38fd278dcb7725afc08dff1c495487608b341aa5da2b2e238ee7dfb08ed66', 'abdf402a4e5288860397aa1fc839f367', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 20, '2026-11-24T07:08:26.233712161Z', '550bb5a41d7834d1', 'create', '\x222a65fa491ea10bc14db860711746486cb7a66ee0dd10af9f36369de0c412ab', '\x014605adbd46892e00109f72b893cb38564b7b963f0f5916dd4bcd792f7f1c7bb0600cc7f4ad5b40347bb241a796387218e4b647dee43aac7dcd6cfac886e75d54', 'ed4c676b0b7771e33c261b3ce32fa58b', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 22, '2026-11-24T07:08:26.276059675Z', '550bb5a41d7834d1', 'create', '\x6f1165628a5687619e6704276c0aa28eb54049feaa6af002b7fef3fedd19ab62', '\x012ffa4c323c5452468146b083c0ef2cc1e2c7159bcd2c1fed6f500c7e94c2f7f5e43c3514028f00a5b4f95efa12b79d746ebbae2b5224e5f75d242ec33ec10c52', '10aa06e250f70b4638e0539aa3f126e2', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 24, '2026-11-24T07:08:26.32740918Z', '550bb5a41d7834d1', 'create', '\x7f3cf6fb65d26be3445c8c8aa5f6144ce73266cde6bf49f8bee69f48249b2a8a', '\x0125fe9879b5a486ac599a849c41bc1691ba1f24ca1972ca089e157222d4b6c89c70a8738658067b1564e529b395f780c1b3c2eea07acec8092c17fc8670cbf595', 'a291e5035008459ebb6166a6b971c92a', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 26, '2026-11-24T07:08:26.374305165Z', '550bb5a41d7834d1', 'create', '\x5d6692275c96b213ec69d9837c98bbe39d6e91ebca7a93a5e758ea67533fe945', '\x0119d42409f48e1f46189b5473d4cb6d37f953eb61477c8e018b0d1547e9f302b812c2416bb72786a845a8befa116bf3ced0b8f34b3a7200b970c6a67e0e954330', 'fb3c57d6c8601242d25cb84ec9adb7c1', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 28, '2026-11-24T07:08:26.404903216Z', '550bb5a41d7834d1', 'create', '\x785af0885a5169d79804133c4149d60e1b0cff0b57635f85b82fa02c5b450a20', '\x01d1762abf54c5ae621b8c4814b74850395a1a6dfd8830990e3045df486a150be81f6151ceacbcbff375cd7491c0193736188eb0f39e7c34b8dd407957aa939ffd', 'a002ce886587f75a6a367d13ba9bb11a', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 30, '2026-11-24T07:08:26.450212686Z', '550bb5a41d7834d1', 'create', '\x8a65d1c05776546b76f6044bf285bb932290a53d7d07808c1d1e333c9486903a', '\x01126fd8a0fc0e319eb375fcb2e85e79d8f6aba8c8e5bedd8d15d0866684c05ec6ae4deb769c86931fd2c80ce1ef54872d8f5ab2e380b40337501c1c5fb76d1464', 'ac0633d5dc955d6a6b6f4eef5bd2efc6', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 32, '2026-11-24T07:08:26.491528483Z', '550bb5a41d7834d1', 'create', '\x678f24a35e91112646f2ce09ae44440894612ad8badde2ebdd499e3646e1226f', '\x01e22c0cf9fb64ee23e1f16e3a71850779edb05112ed202747a996f6ec050cdc2435c950be9651f9436e917aaa79d9a55aeda2d5c626b636ce64855cf2084640ae', '7855312e79e46f001770d9f87ffb4a37', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 34, '2026-11-24T07:08:26.525281017Z', '550bb5a41d7834d1', 'replace', '\xe46e1b6465dfa7b31020f5573c69c89094a27254767d16c05b52ef8116d2877a', '\x01eae5665511b3ff4851b6b28e57fadd04acc45d052daee12f59340f313061639e57e61b3f492d4cf9b13b377403bde626a5adc3d48db318a3b4b10c435db4ab7d', 'cd98d783a25bd2979f49f595956fa291', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 36, '2026-11-24T07:08:26.600409979Z', '550bb5a41d7834d1', 'replace', '\xf75fd37fe6d641bb2282467513fde559f816e1d170e7801a242d38053b62b9d8', '\x010026f789f97011fe162b173df31a6119dda0d903232f3f20926c7c7d49fe9c6f0437f682b8dd6468372c658d4e9d468be95c497b25f4c0e99718e531583b0521', '7dabc86a35385fb078d8dde7292c72bc', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 38, '2026-11-24T07:08:26.613034864Z', '550bb5a41d7834d1', 'delete', '\xa521138c9cfb5ed74f3fce783285656fa7dd5aea512c0bd36f4e51d10bfe1d64', '\x01ecc0c9c2ce638cfe321e2ae8910a6d1e15039dfa7e3ce480d96c0122a2cf7d907099530dc78b0d7770c98e5d2a0a968d73edfb159ec51ff282f68c7331940717', NULL, 204, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 40, '2026-11-24T07:08:26.624705615Z', '550bb5a41d7834d1', 'delete', '\x222a65fa491ea10bc14db860711746486cb7a66ee0dd10af9f36369de0c412ab', '\x011ea8224fa8447eb7c97a6909c800f4d7eb976729c9ec66529e144ba7efc9674eeb366f9cbff372cea72e967f03ecbcdae9e02e37ad04411fe54cfaea7ed44235', NULL, 204, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 42, '2026-11-24T07:08:26.636488339Z', '550bb5a41d7834d1', 'hold', '\x6f1165628a5687619e6704276c0aa28eb54049feaa6af002b7fef3fedd19ab62', '\x01ead162e616fac530e06fd6c86295b3685f2faa241f41faffd5a1fa28de445f7cf29ab3ea473b8bd5a257f6e916bd1b784c9d403328eaf27bf7419d2fb047b6ab', NULL, 204, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 44, '2026-11-24T07:08:26.643327237Z', '550bb5a41d7834d1', 'delete', '\x6f1165628a5687619e6704276c0aa28eb54049feaa6af002b7fef3fedd19ab62', '\x014a3efe7fdf577e56d76bfec6fa39d8bf7f28a0d85138e30b769d027e06673e53d9fb3d226e15aefd28b1134dcd06a2140a4e9af47afd212e86e5256228d6048f', NULL, 204, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 46, '2026-11-24T07:08:26.652305339Z', '550bb5a41d7834d1', 'hold', '\x7f3cf6fb65d26be3445c8c8aa5f6144ce73266cde6bf49f8bee69f48249b2a8a', '\x0142ef041e846961b3a3f0d9e6c040962e5b1d6d5fb15362d706ad51f079b4fd2c7975dd8be18b81c5b8664938da1e5aff58db7c402aa4b0630e0082c385d7a7c7', NULL, 204, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 48, '2026-11-24T07:08:26.688002207Z', 'b93636aafbcf3853', 'create', '\x65845f49e961a9568485daecaf5dedfead22b53e045787b844c758524407f2db', '\x0120952c3fcb1742da9c8f4ec143f1651823c28d2a00e10a6f1508819df20c0c532fbecbfe22ca41564029f7af432808bc2e1ee56013027bca856f77933364e9b1', '26b33b388614574976f2ff2732c4f268', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 50, '2026-11-24T07:08:26.725730111Z', 'b93636aafbcf3853', 'create', '\x9caa423a3c012226fc61ac82ff22432d669a1fbf051f24bd13ded13edfebf8e7', '\x01cc135c0c3080d8f98fa0f7d5953b4bfb762d67de5044f1e676d5fb220c351538e67870b0c7d184826c3738ed348aa0ac7110517d929d3cc84dc1d4de4d708c85', 'e9d80514b547e61aae052961e074d107', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 52, '2026-11-24T07:08:26.762701411Z', 'b93636aafbcf3853', 'create', '\xc3ba7f37391b77e50fd4d4eadf416614424b7fb41727012f9fb5274f30d0d9e7', '\x01f732cbc32d080ff5863cdd31617902b97e9e5459142782da33bcfa2165fcac418392a244adbe2b41f03db12fc6c4ce2b30f17034480b7c4652b603c1a1641d38', 'f0a8e979bea54ad28d05a68c50d62304', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 54, '2026-11-24T07:08:26.83003815Z', 'b93636aafbcf3853', 'create', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x01bf5d83bdb60146bd80bda0305adcbd96c25b89bf49294138820d58ff5760ac5c313e4074e8ea742e3a605340b9af1646b905e8504492d17fd1dfd32456f8b94c', '244032c92551a179ad4bf10d988c4811', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 56, '2026-11-24T07:08:26.870263425Z', 'b93636aafbcf3853', 'create', '\xb158ad95062b112de065a29a74014e65384ab1c6d151a4791ec2a0bcee1628ff', '\x01ca9384e1c70a741be5d2c72147a3c6a824f580ec244210cd51f1dd964e68bff9ee21090559bf427bdb0d7bd5b5c4d0c6211005fa3a43f538d425adbf9346acf0', '957d5d95d211e69f049a82719a9c6c49', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 58, '2026-11-24T07:08:26.903999654Z', 'b93636aafbcf3853', 'create', '\xa490f90879b3ea4f9abd4b4b457baaa7061dc8c5c252bbb1b46dfc91e50d8790', '\x01b79910e4683541fbe5018aa06a714ccdc23be33422655e29d89af523944eb701f9bbdd8b0e18950c7f3eabb68e34748ca649859bcaa8e22c6e06f961c5b2a940', 'c48496b5372837975a307c6752b1845a', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 60, '2026-11-24T07:08:26.936010691Z', 'b93636aafbcf3853', 'create', '\x8c2cd054b7010c1b646abdb1e4fd9af67e3ad8c937cb409eb5b4400aee6f1ccb', '\x01f2b60e8aba749b311e8b357a7b2b07d31b8bfbc072b30d8ed96f08000c1a59450278e69a49c1b47a126a4a416ddedf80b928429f947e39469c6cd68011e60b6c', '8e315ac0b67e962d123313e02a61a6b6', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 62, '2026-11-24T07:08:26.971619363Z', 'b93636aafbcf3853', 'create', '\xc0cef42c5acd6648d7b05f51020ab16dd4e57763e986ffb37c4c1abed03982ed', '\x01326e69eeac7a17896f15389da8ed8b0255c0cd73910c12849e6e098a7edf5cf1ed2e58ef7ed39cbb040e727de02cfcfaa91da1bddec31940fe9f05bcaf3b73b1', '34db5e45ca01e104b2da8402f9156e30', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 64, '2026-11-24T07:08:27.005778187Z', 'b93636aafbcf3853', 'create', '\x3934e25f809551182a9c95809513808185fa8f4dff1afa45c3f9a43352d0928b', '\x01f110876fe7289b09d506114b9f2d439619ce6919de0eca0a9b5620139dd053eb4638c906a1b76fe82b94e2036cafd9968d53ea8d4cf81db804103e5c71910f89', '8e43fbb6d44817a7fa5241f8c90eb766', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 66, '2026-11-24T07:08:27.056270947Z', 'b93636aafbcf3853', 'create', '\x23539626d668dc7ce69a63007b84bc379c0b204a9cdc92e4258595a17bfc94d9', '\x011c919cd64d59c60c519c7fdc0a34fd1c3903e7db18351f4a5b79651e92523ec0156c5be7e23c2efdcc0200755350ebeaac04488b30116c2ccdfe97fcda37327e', '7abe8eecd801432fbe62f61d16701b5d', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 68, '2026-11-24T07:08:27.132379828Z', 'b93636aafbcf3853', 'replace', '\x65845f49e961a9568485daecaf5dedfead22b53e045787b844c758524407f2db', '\x01537c2f6ccccd6776654c6b228d249775ca1252e6214d6f1eeded4cad0b25ae2c921f5ab51d793f131c4a98ce13d09f83eca242e0162c1ca53979d9adc666a283', '4059b5f0afca2d95db040bd7c204e6ca', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 70, '2026-11-24T07:08:27.142470906Z', 'b93636aafbcf3853', 'delete', '\x9caa423a3c012226fc61ac82ff22432d669a1fbf051f24bd13ded13edfebf8e7', '\x01a11116035733041bfe0a6cc295a086ab6f67044dccd405bbe51cc93719b12af3dad504758510d3170b7b98af52597ff71e6809e3dd863b9df87044a67f9dac08', NULL, 204, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 72, '2026-11-24T07:08:27.145541113Z', 'b93636aafbcf3853', 'purge', '\x9caa423a3c012226fc61ac82ff22432d669a1fbf051f24bd13ded13edfebf8e7', '\x0149538c619f8b7cc8bd51d0e18720490eb011da3f20963b2dfd38ba44ceae2e9b12df52ea1c4c0142421677a673ac1e6ed23d72dcadb1119e7f9f072df1aefd37', 'e9d80514b547e61aae052961e074d107', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 74, '2026-11-24T07:08:27.256472034Z', 'b93636aafbcf3853', 'hold', '\xc3ba7f37391b77e50fd4d4eadf416614424b7fb41727012f9fb5274f30d0d9e7', '\x018eef9351bcbbbe5a0f9d88c1eeff2148b7a352b3ecdf78ddf682bc828226b7e9cb3f6a396b3114de54cb01a39dd07e1bed497aa49f88f599cee8030652704c0b', NULL, 204, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 76, '2026-11-24T07:08:27.269700301Z', 'b93636aafbcf3853', 'delete', '\xc3ba7f37391b77e50fd4d4eadf416614424b7fb41727012f9fb5274f30d0d9e7', '\x01b8d4bc7ca3ca4c3070736617123349edf3e3f891c0566b6461423c716f5edf80bd505dd86e180bc07cfcdd86d80ea6105bdb2a206e685699a78538a9030fe362', NULL, 204, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 78, '2026-11-24T07:08:27.281534646Z', 'b93636aafbcf3853', 'hold', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x014f96b11b9bd9f6c0629d27a7cfccddd26097a8ef9e405d6699838ca9e77b78c2181e92eb1f7c9eb0bf75538a1ded00c288b4918bb23c4414ceea59fde7ab5934', NULL, 204, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 80, '2026-11-24T07:08:27.31924543Z', 'b93636aafbcf3853', 'replace', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x01797f1c20ab6b355c2862c13fd6920b116ba752e916120c9c9e3e1438b61059b803cf20f11919da2ddb6aa9669611a82f8fde04bace6e9cc5b5270187b6d44392', '23bcde00e08ff12a90065ba58a27c605', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 82, '2026-11-24T07:08:29.822034284Z', 'sweep', 'purge', '\x65845f49e961a9568485daecaf5dedfead22b53e045787b844c758524407f2db', '\x01c1a38276ea63d5facc02a7ed8921a18b573978d4e7c1b4208116a2e6b043f7a19e31a9282b11542c0f41be6322a25145493430d838d2ef7072037f944718f11d', '26b33b388614574976f2ff2732c4f268', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 84, '2026-11-24T07:08:30.14377257Z', '550bb5a41d7834d1', 'legal_versions', '\xe46e1b6465dfa7b31020f5573c69c89094a27254767d16c05b52ef8116d2877a', '\x01e0c7a27b4bbac7af14217d5910fbafdcc8cacdc2a7298c0a955a5e69d01da017845e1c7140f6a87c0e8476fd0567424975019d80d4261831d9ecbafd2a77d786', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 86, '2026-11-24T07:08:30.157205217Z', '550bb5a41d7834d1', 'legal_read', '\xe46e1b6465dfa7b31020f5573c69c89094a27254767d16c05b52ef8116d2877a', '\x01a5f5e4b9c11f9a13f3268704007774e9285c07ce9429629ba092abf1bae99c2e53b2bee85d78b0612ad43b7fac9030b4f631c3c1ffced39b9c102c5cc110ff36', '9a13d4d8ef63cb902e4fd33bd350d84b', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 88, '2026-11-24T07:08:30.169175469Z', '550bb5a41d7834d1', 'legal_read', '\xe46e1b6465dfa7b31020f5573c69c89094a27254767d16c05b52ef8116d2877a', '\x01d64e7e9ed554f5ec87f4ce5df058f7f17e1d4642f7ca5fa7b41277eab8967c05793aa4f88c11fe935895525c3bcea95ebba3dbfdacdc009cce372e1f836e080d', 'cd98d783a25bd2979f49f595956fa291', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 90, '2026-11-24T07:08:30.179396233Z', '550bb5a41d7834d1', 'legal_versions', '\xf75fd37fe6d641bb2282467513fde559f816e1d170e7801a242d38053b62b9d8', '\x01d9392b50ed573e13496b5b3da71333340dcfb0d653ee0c784c2a9c921ff470efa47322c525baf75b7528bffc4d1233021d22b7f8dbb56b870d0ab3625f333334', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 92, '2026-11-24T07:08:30.191381189Z', '550bb5a41d7834d1', 'legal_read', '\xf75fd37fe6d641bb2282467513fde559f816e1d170e7801a242d38053b62b9d8', '\x014c261ff32c44549eabaaf52cd37442e0c5e2776bfc66bef1f22d3d520483bae12eafa648ed1970efaa6390fc1f36062ef31c329f2a4a50f1d55ccf504fb88512', '8a025907bb97a06a57f70457ba5f9ef3', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 94, '2026-11-24T07:08:30.202563893Z', '550bb5a41d7834d1', 'legal_read', '\xf75fd37fe6d641bb2282467513fde559f816e1d170e7801a242d38053b62b9d8', '\x01182eb8ac5a8b5fb63a82d5fb78b5afa714da3fc5e70c4269936620ddab5e6c287f06d1533fc579d0b7d3385d7491354a26b8ab38c2380cfebb910a67e354b0f0', '7dabc86a35385fb078d8dde7292c72bc', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 96, '2026-11-24T07:08:30.211667235Z', '550bb5a41d7834d1', 'legal_versions', '\xa521138c9cfb5ed74f3fce783285656fa7dd5aea512c0bd36f4e51d10bfe1d64', '\x01c0fe6dec46b2b1cfa150ca9347db9497776501094f21dc3ced6c872d4234ebeacdedb4bddb1ef1536be30352bf12f613437da0bba4a0794f19e015f13482b2c6', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 98, '2026-11-24T07:08:30.228734397Z', '550bb5a41d7834d1', 'legal_read', '\xa521138c9cfb5ed74f3fce783285656fa7dd5aea512c0bd36f4e51d10bfe1d64', '\x0152305044017d4b29ab06747964449cbc31c72f35120197a35a4d58e0f3772e392e29acf3b2db62c500bea93351965ba27b425b61ba3bcfa1d257a90bfe28b66a', 'abdf402a4e5288860397aa1fc839f367', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 100, '2026-11-24T07:08:30.236602169Z', '550bb5a41d7834d1', 'legal_versions', '\x222a65fa491ea10bc14db860711746486cb7a66ee0dd10af9f36369de0c412ab', '\x01829c716b0fa2c5b9ec047f16f0e661ce5065948918eea3f9f43e5eb918904dfe2e3515df03cc7cd26a7abdc8e144ea5a1fe5bcaa7f9dc50252967ffa6159bdd2', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 102, '2026-11-24T07:08:30.245015713Z', '550bb5a41d7834d1', 'legal_read', '\x222a65fa491ea10bc14db860711746486cb7a66ee0dd10af9f36369de0c412ab', '\x01612c62b01eee7ae3180c74e0e3caff9243519a23dad2f101a7bdc387bda6b14e2278f4fb1bd183a89f2426717cf27d96ca34cfdc4134a873a31da413201d49f3', 'ed4c676b0b7771e33c261b3ce32fa58b', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 104, '2026-11-24T07:08:30.253151514Z', '550bb5a41d7834d1', 'legal_versions', '\x6f1165628a5687619e6704276c0aa28eb54049feaa6af002b7fef3fedd19ab62', '\x01839f942bf3f2c0f0b7bded3143c8dd392c0a2d100f0d461bf2c509c690fffd380ece03e139655a723ebd335133bb54dab9e11779e54178fd338b8c5b73239df8', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 106, '2026-11-24T07:08:30.265732731Z', '550bb5a41d7834d1', 'legal_read', '\x6f1165628a5687619e6704276c0aa28eb54049feaa6af002b7fef3fedd19ab62', '\x018a96cccb7147a3bf5a04c33ff56b2c11375023ea7b9f9e95e1e60837b79aa6095d60f5380037393650766965b767be3c1b0c1c665a778c84ea6faeaaea350784', '10aa06e250f70b4638e0539aa3f126e2', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 108, '2026-11-24T07:08:30.279344204Z', '550bb5a41d7834d1', 'legal_versions', '\x7f3cf6fb65d26be3445c8c8aa5f6144ce73266cde6bf49f8bee69f48249b2a8a', '\x0159eb3737a97ebfcb305cf9c3c8849ae6634d29b015b0930d5d8cb7fc6af9e62b355c677f3c7fa5fcc9b0149543a5fbcfbcc92b77fd2d9efb529cba164f73ea76', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 110, '2026-11-24T07:08:30.287527246Z', '550bb5a41d7834d1', 'legal_read', '\x7f3cf6fb65d26be3445c8c8aa5f6144ce73266cde6bf49f8bee69f48249b2a8a', '\x01c6d500ba28d0376a1a6eef7198728ffd6bcd97a5929a0c9af2160bc901d8ca009fa4679f1d0d5fa915bf4259f2b3cf6d153ebb2ecdcdd97ff7cff07e27e42dd4', 'a291e5035008459ebb6166a6b971c92a', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 112, '2026-11-24T07:08:30.293647614Z', '550bb5a41d7834d1', 'legal_versions', '\x5d6692275c96b213ec69d9837c98bbe39d6e91ebca7a93a5e758ea67533fe945', '\x01211568c63ef1d1c17377d58d15d9c84dce4fe672d4f669cd213dbc5ce6c04dedf1cbeed6950cbdc772d1d229dee8e0c4739c962a6f4667a2fc52e25853824cf2', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 114, '2026-11-24T07:08:30.302051545Z', '550bb5a41d7834d1', 'legal_read', '\x5d6692275c96b213ec69d9837c98bbe39d6e91ebca7a93a5e758ea67533fe945', '\x01fdcbf875b6b4bfdf8210d5060a60610f4d62132c2d7cd9fa8dcd1cb64e527f4b62f834699071a293b19994caee76f2e97cc767ba9060ff3f2730ffd3ceb3a8b8', 'fb3c57d6c8601242d25cb84ec9adb7c1', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 116, '2026-11-24T07:08:30.309670057Z', '550bb5a41d7834d1', 'legal_versions', '\x785af0885a5169d79804133c4149d60e1b0cff0b57635f85b82fa02c5b450a20', '\x018aa8d223fbf4eb910705e6552d0ada991c2b6f741dd8f0d8b69410ae9f898e0ff7d19be605027d1875fc8e71cb3397bf498efbf14c7ffe0440a718f058ae4b38', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 118, '2026-11-24T07:08:30.318770751Z', '550bb5a41d7834d1', 'legal_read', '\x785af0885a5169d79804133c4149d60e1b0cff0b57635f85b82fa02c5b450a20', '\x01a04105145c6dde784ca32f12299b13c58150916bfcc973bfffcb106eca2caa44eb121f4ada8304635d28d8ceb4c29af6bdd2aa5d72e72afb44dad031402ebded', 'a002ce886587f75a6a367d13ba9bb11a', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 120, '2026-11-24T07:08:30.326217985Z', '550bb5a41d7834d1', 'legal_versions', '\x8a65d1c05776546b76f6044bf285bb932290a53d7d07808c1d1e333c9486903a', '\x01ff38d19047ddb643193c6f9132e92a9973e62c27ee4167e2e9185c944c82fe93b64a26cf33c707c1e37cdcc109b1304d96a70bc316e01d2692cb5c503435da78', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 122, '2026-11-24T07:08:30.342594282Z', '550bb5a41d7834d1', 'legal_read', '\x8a65d1c05776546b76f6044bf285bb932290a53d7d07808c1d1e333c9486903a', '\x014f17841da695f7ec8bc687398458ec27db61627071c686d888f4082f18b8a5eb42f8a1f089f4ec7d123d1dbc6f045990fb9a6892993c3c183bc015d90a190c82', 'ac0633d5dc955d6a6b6f4eef5bd2efc6', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 124, '2026-11-24T07:08:30.350357953Z', '550bb5a41d7834d1', 'legal_versions', '\x678f24a35e91112646f2ce09ae44440894612ad8badde2ebdd499e3646e1226f', '\x010f2e2521a5c1540d8f2a64ab3f154adc7c0382631ca50c7dd662240e4dfbaaa2d64ed18fe3c2f28b9d2c4fea18eec6d70800638dcb597b6a7f3537c875e43e15', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 126, '2026-11-24T07:08:30.360075492Z', '550bb5a41d7834d1', 'legal_read', '\x678f24a35e91112646f2ce09ae44440894612ad8badde2ebdd499e3646e1226f', '\x01f2a18c357cb9a8c2ded3f8b8973660ab068e4c7d681b13fd894e3bb55d41e4611b033caa51d21e858be5152167c60cdd53dac2afdf05c22df573f44d1c7c0ef0', '7855312e79e46f001770d9f87ffb4a37', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 128, '2026-11-24T07:08:30.368910018Z', 'b93636aafbcf3853', 'legal_versions', '\x65845f49e961a9568485daecaf5dedfead22b53e045787b844c758524407f2db', '\x01a34d28255b4d97fa2fbae78edf7f7c658a6080507ad902a0b71046bdbe25e6d5989b6d2118bfde62a8c6e4aa85bdbd6f7c3d96d8c0d027734cb5be0fd2869563', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 130, '2026-11-24T07:08:30.378686131Z', 'b93636aafbcf3853', 'legal_read', '\x65845f49e961a9568485daecaf5dedfead22b53e045787b844c758524407f2db', '\x017608787654a59089beaa207abdbdd7cf54283f982c18090ca00dda1014191b86e0f4318f5db8153c6ed5ac00e32f93c3cef910ebcaf69ddd99800f77cbcfa912', '4059b5f0afca2d95db040bd7c204e6ca', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 132, '2026-11-24T07:08:30.387181009Z', 'b93636aafbcf3853', 'legal_versions', '\x9caa423a3c012226fc61ac82ff22432d669a1fbf051f24bd13ded13edfebf8e7', '\x01ffb6293990475640dd3564b12b167678a5320f57bcac254ab760a3ec95284bdf6ace9cce332b9d0ce0406bbf2914ca80000855fe1d5e6eafdebfd66f5b08f0d6', NULL, 410, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 134, '2026-11-24T07:08:30.393809962Z', 'b93636aafbcf3853', 'legal_versions', '\xc3ba7f37391b77e50fd4d4eadf416614424b7fb41727012f9fb5274f30d0d9e7', '\x019b6f968446a8dea813b6bed8a8f4e741b5b05fda87f075b57bedaaf15599fb1f9f4edc18fd08d6fe4bb23b798bef9f6ce7373fb93e8c419b5f17fb1d7c7c7987', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 136, '2026-11-24T07:08:30.402144485Z', 'b93636aafbcf3853', 'legal_read', '\xc3ba7f37391b77e50fd4d4eadf416614424b7fb41727012f9fb5274f30d0d9e7', '\x01e8ba688b3f15a90ba79a104223cd5358b2c2aa5747bdebf8ab0686b6175469f51f5c95afbd3544b94ee41514c615b01480532ef6c8e5d51c66855b2e17caf644', 'f0a8e979bea54ad28d05a68c50d62304', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 138, '2026-11-24T07:08:30.408237103Z', 'b93636aafbcf3853', 'legal_versions', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x01d27f81458a1ee5072f8f37e4debfff3bbe9262ffb4cc6b6321b9ec11d118e78b993295f3222b017a947f677cffbf6443d2b7583f39ed5b52cd6d9b94c6ab3891', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 140, '2026-11-24T07:08:30.417166287Z', 'b93636aafbcf3853', 'legal_read', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x01b14ed9e30695bba315ec8df11793e526dedab91031826ab53ea38e12dcf9f191760175169ea27256f6690782d454ef2c63af29b494812a96b71f210986d32b16', '244032c92551a179ad4bf10d988c4811', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 142, '2026-11-24T07:08:30.428420471Z', 'b93636aafbcf3853', 'legal_read', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x01a210b1e335224988dc6c900d2db5847db1dc5a0aae72b5de465931ad0bca5ed2b48b4a06d8040003a5e2acce31976e72e2b4962d681d679cf5324080190e3fc7', '23bcde00e08ff12a90065ba58a27c605', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 144, '2026-11-24T07:08:30.440568196Z', 'b93636aafbcf3853', 'legal_versions', '\xb158ad95062b112de065a29a74014e65384ab1c6d151a4791ec2a0bcee1628ff', '\x0132ac2f96b515a90a612570cbae79b66dd26314335e8fb288c0fe2233010afc3d252dfb55b679ed2f34fd9c06ef17c4349a07be6d8971d6dade88da28835d421a', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 146, '2026-11-24T07:08:30.448715109Z', 'b93636aafbcf3853', 'legal_read', '\xb158ad95062b112de065a29a74014e65384ab1c6d151a4791ec2a0bcee1628ff', '\x014e71f4490e9cf926812d0d3f391406fb40994fd0d3e21e520fd087c5c711d2a7adb89fa47ee5865368eb5f50c6f6dd9c0b79ebc9071f40197fce48175c18eb0f', '957d5d95d211e69f049a82719a9c6c49', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 148, '2026-11-24T07:08:30.458204396Z', 'b93636aafbcf3853', 'legal_versions', '\xa490f90879b3ea4f9abd4b4b457baaa7061dc8c5c252bbb1b46dfc91e50d8790', '\x01cde104232266d201fc70348249b1d7066ebf40b0c32d13d095fe057ebace00eefc885beb6d645d05e57e219e9c23c3254d8cc74a25cf91f194bb69f53231afcd', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 150, '2026-11-24T07:08:30.471833498Z', 'b93636aafbcf3853', 'legal_read', '\xa490f90879b3ea4f9abd4b4b457baaa7061dc8c5c252bbb1b46dfc91e50d8790', '\x0158baff88df579d002dcd3fba47d3a0b206b116e3a36709883873a58ff1d213014b61c12cf93178846fbcad692a3a721e107e83694d63c2962cc10077ab56d6f5', 'c48496b5372837975a307c6752b1845a', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 152, '2026-11-24T07:08:30.47928849Z', 'b93636aafbcf3853', 'legal_versions', '\x8c2cd054b7010c1b646abdb1e4fd9af67e3ad8c937cb409eb5b4400aee6f1ccb', '\x011bea9743f5511d88ded8a4d18453d5466998069d31163cc3a9e23dbe2cb3a7abc365717a37bace9ebf7072231cdb3808fa992e3ae630447b0f8e9029b7889f13', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 154, '2026-11-24T07:08:30.492841351Z', 'b93636aafbcf3853', 'legal_read', '\x8c2cd054b7010c1b646abdb1e4fd9af67e3ad8c937cb409eb5b4400aee6f1ccb', '\x01b7ea27086127478aba3c596633c4671fac739134c133be22a98b8ad4b531d174faf8b5f76d5b85628ebb38e3e9c6e46d48d5a53d3915c01ac8199f66e9cd09a3', '8e315ac0b67e962d123313e02a61a6b6', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 156, '2026-11-24T07:08:30.504276074Z', 'b93636aafbcf3853', 'legal_versions', '\xc0cef42c5acd6648d7b05f51020ab16dd4e57763e986ffb37c4c1abed03982ed', '\x0164bec222a8bb2e488a61b66fcdfed4379000412c12cb6b713831d936d0ebb6311afd647538448108a8a0ebae8f638e7e5e4a37ba8c8217d0958142d8f5735145', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 158, '2026-11-24T07:08:30.514080586Z', 'b93636aafbcf3853', 'legal_read', '\xc0cef42c5acd6648d7b05f51020ab16dd4e57763e986ffb37c4c1abed03982ed', '\x01c0a2fbd55a6f8e05cc837ee246254fdeb1e34aa4abb3f9b159a80266b84ec43fd8b451296145c5d09b9558e28587a09a2c9dbf1894be99b5dbe7e09b457ca51c', '34db5e45ca01e104b2da8402f9156e30', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 160, '2026-11-24T07:08:30.523952727Z', 'b93636aafbcf3853', 'legal_versions', '\x3934e25f809551182a9c95809513808185fa8f4dff1afa45c3f9a43352d0928b', '\x01baa26384d757b1cced2c4b91bbf236ca116943840dce39cea89e3495ba5598175170bb8d5a2fbfe8eb2c105045d5d3927ebd177250e8c78bf707b0f4b1550409', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 162, '2026-11-24T07:08:30.536970649Z', 'b93636aafbcf3853', 'legal_read', '\x3934e25f809551182a9c95809513808185fa8f4dff1afa45c3f9a43352d0928b', '\x01e6e2d60f544da01927a170808722e1652d5ecb1fd73577eecfa9a95b08539f45e75bb1dee9c968875a6b3b99911491fa926aa34bc7babac510bad9ce6e46a46b', '8e43fbb6d44817a7fa5241f8c90eb766', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 164, '2026-11-24T07:08:30.543795778Z', 'b93636aafbcf3853', 'legal_versions', '\x23539626d668dc7ce69a63007b84bc379c0b204a9cdc92e4258595a17bfc94d9', '\x010a1cd2b4eb2175ff879f82b2e09272a14d26742c29e0c233afcc9938601884a2d80fe0fce40873333f45dc3d90e333d8ed4fd40e801311d6180dc00e91161b3c', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 166, '2026-11-24T07:08:30.553919561Z', 'b93636aafbcf3853', 'legal_read', '\x23539626d668dc7ce69a63007b84bc379c0b204a9cdc92e4258595a17bfc94d9', '\x0123a280fd61f5857618cc7e56134c31d93d511ce8951e93ce09c0de668a0407f4cd0081b20f19bc2dd042ed38926eaf4afbb7ef98a804c30f556a5aacae4aaeae', '7abe8eecd801432fbe62f61d16701b5d', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 168, '2026-11-24T07:08:35.330428102Z', '550bb5a41d7834d1', 'create', '\xcf496d9f22fce17c60ba49c037b58c0af813d27fc06a62c87fe373a7658d53c6', '\x01dd737c7600b28ae426261a5279330707940e5b20b615585ae68aaad0055bf7383c22c5329431778155c5d7c477e4aabcc3b1c3c5ae7b114e65fc31fef937786d', 'c595dc9cc01901cdcf2b92a92a1ee4f2', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 170, '2026-11-24T07:08:35.355661332Z', '550bb5a41d7834d1', 'create', '\x0b85d0ada4736875ea87a5b30936da0a87d454e3ca4439dc3bb1c17077ae9b58', '\x01e8db2eea0117026201a04e4e2c8eb72ae48cc2259f195f70578c26810736737c617804f7e0199b2921e33493a44315a5c13a5f5acdc36a43d5c50fb395c5759b', '3acb2014914b61564ab5fd9dfdf94475', 201, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 172, '2026-11-24T07:08:35.386568669Z', 'b93636aafbcf3853', 'create', '\x37549ea96abc26ea6c2dcae5b3a94532a60e102a8a715fcbf3c0f353b9bbe2cb', '\x018ce6beec9f235d8b627cb722daef07f99ba8270b39df8164e4bf9ae83eb6ea0474e030e2df67c7f72a678f5ec75f435b0f2ca15d11931e296c22c092859b992f', '9123470233f518ba808689b8310425df', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 174, '2026-11-24T07:08:35.4233739Z', 'b93636aafbcf3853', 'create', '\x1dab80fc04440caedfb04470ea84ae4a47893fa1d19e49993b7fceb3648fb364', '\x01963f3a096db028c93394e6c6245cf962a96d6d78f994b2bc83f76210223dd657e62b18d6cbf22852a2f696f09ee8384c41ef146eac4bf7cc9636900d31affb31', 'ff4a8413b26f55592e46851c79855121', 201, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 176, '2026-11-24T07:08:35.429820685Z', '550bb5a41d7834d1', 'legal_versions', '\xe46e1b6465dfa7b31020f5573c69c89094a27254767d16c05b52ef8116d2877a', '\x01daf2d6dc54fe5d3bf3632169e847130fe99e6470a5e3f47a17ece9e043056f8599eefe358b9c17209a65c230473db0bd71ad08fb6a9bd124aad62cb1291ee7a7', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 178, '2026-11-24T07:08:35.439697929Z', '550bb5a41d7834d1', 'legal_read', '\xe46e1b6465dfa7b31020f5573c69c89094a27254767d16c05b52ef8116d2877a', '\x019621a964e52cb4c7379238defdeb7e42110a92c420d0e99b4a2f055159a04fb198c9e1aa927d9d55c2e70c73f817f6b95152db0a8f799d674a15843561a8cb50', '9a13d4d8ef63cb902e4fd33bd350d84b', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 180, '2026-11-24T07:08:35.45407444Z', '550bb5a41d7834d1', 'legal_read', '\xe46e1b6465dfa7b31020f5573c69c89094a27254767d16c05b52ef8116d2877a', '\x012b923cde9495efc84aedbf7656b0535768b16c965be4830bdc1d078d9917f62fb410c6e5e61b7d364847041ab119af6a6ee0a7533da40df9b7f922b0e866df77', 'cd98d783a25bd2979f49f595956fa291', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 182, '2026-11-24T07:08:35.460200307Z', '550bb5a41d7834d1', 'legal_versions', '\xf75fd37fe6d641bb2282467513fde559f816e1d170e7801a242d38053b62b9d8', '\x019092d61878aa4379ef519fcaa62a4333621859b02410b5c08e025d56f8daf11c29b264e0233f100fb5a1c0dfcb3ab73b76ed6a5372910c111cb7d5fc4763a917', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 184, '2026-11-24T07:08:35.469175492Z', '550bb5a41d7834d1', 'legal_read', '\xf75fd37fe6d641bb2282467513fde559f816e1d170e7801a242d38053b62b9d8', '\x01d71833024c4ba1b8afd1765154b0113651545d2bd24a6de0b8a9411db08f6a0190c324be1884504e91bd7fe23b36bbc514a55b8808e299b2c4e4281ca630da76', '8a025907bb97a06a57f70457ba5f9ef3', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 186, '2026-11-24T07:08:35.477170513Z', '550bb5a41d7834d1', 'legal_read', '\xf75fd37fe6d641bb2282467513fde559f816e1d170e7801a242d38053b62b9d8', '\x015c3b5a60bfe0e5aa323d623e0923b0cd6ab4a33a378e83cf949a930c2e220924295ea209f2ed85e31ca058c7f4a389666eec4b15d8ce578e34507025989e976e', '7dabc86a35385fb078d8dde7292c72bc', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 188, '2026-11-24T07:08:35.48335703Z', '550bb5a41d7834d1', 'legal_versions', '\xa521138c9cfb5ed74f3fce783285656fa7dd5aea512c0bd36f4e51d10bfe1d64', '\x0159d2c80d0284819ad18ae6e3ee8dab66944e492188d07ad69113fcd3c9325f169981c940c9dc78c6152bdeb7f8ad5fe7d4297af58cfef140a1cc6e29ef1f09cc', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 190, '2026-11-24T07:08:35.490847625Z', '550bb5a41d7834d1', 'legal_read', '\xa521138c9cfb5ed74f3fce783285656fa7dd5aea512c0bd36f4e51d10bfe1d64', '\x01f83c0a3dddbc46164e789a1039728f0c594e9b70ebdefbc63ad6b0fa4f0d52e9e2a47e3b05d4bf0acb02159c85c882e57527579bb3f60e3b9c9584481d71075a', 'abdf402a4e5288860397aa1fc839f367', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 192, '2026-11-24T07:08:35.49997467Z', '550bb5a41d7834d1', 'legal_versions', '\x222a65fa491ea10bc14db860711746486cb7a66ee0dd10af9f36369de0c412ab', '\x01711c2d04c5e51cf9142831dfeab19a2efa1715322462adf69d78b51e5aa0de2f7fa13ef80dde52990a7947d58b0e08f6458b677c5272d1d783863c002d9acbca', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 194, '2026-11-24T07:08:35.508804586Z', '550bb5a41d7834d1', 'legal_read', '\x222a65fa491ea10bc14db860711746486cb7a66ee0dd10af9f36369de0c412ab', '\x017dd0123b85435a5151a6ef88258fd947b15ca466f128dcc69bcc911c6558cec351b58c81ece93fd5a06de855bad73379f263db37641660419c06f203978baf2d', 'ed4c676b0b7771e33c261b3ce32fa58b', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 196, '2026-11-24T07:08:35.518965887Z', '550bb5a41d7834d1', 'legal_versions', '\x6f1165628a5687619e6704276c0aa28eb54049feaa6af002b7fef3fedd19ab62', '\x01ae9e1ac67e20a65ede8ace7254b90e1144df8c83306e096d6983f9ae554746a4e2dce8181f625deeb313e4a49a2eeb788a4dae3331743467eed01c3afd88c1f3', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 198, '2026-11-24T07:08:35.526790196Z', '550bb5a41d7834d1', 'legal_read', '\x6f1165628a5687619e6704276c0aa28eb54049feaa6af002b7fef3fedd19ab62', '\x01af7c676b71813e65a3a511bf7a129afb07ff6ef775ab76bd3d0b78eea234a7149a53cb5b0de5d6636d9081a30d591f0274b580a0b0679d44d5a692ef393529cb', '10aa06e250f70b4638e0539aa3f126e2', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 200, '2026-11-24T07:08:35.532465525Z', '550bb5a41d7834d1', 'legal_versions', '\x7f3cf6fb65d26be3445c8c8aa5f6144ce73266cde6bf49f8bee69f48249b2a8a', '\x01d42bf1ef8e9ce223bb49df6aee8fc2e2e622418d93e88fda0ee619d87f7987922757bc3fea26f871c09b21e4ff45835e29cc6d62d66bea1e226a8dbc7555a204', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 202, '2026-11-24T07:08:35.539802786Z', '550bb5a41d7834d1', 'legal_read', '\x7f3cf6fb65d26be3445c8c8aa5f6144ce73266cde6bf49f8bee69f48249b2a8a', '\x019542a5e87c8e47657ff77fdcdde834ae0ec6fd8355016b3797921705692c26001ed2ca5a42584562b9d8ebf52d5d6232210c71bbe66907ea09d9dd999c10df91', 'a291e5035008459ebb6166a6b971c92a', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 204, '2026-11-24T07:08:35.545715124Z', '550bb5a41d7834d1', 'legal_versions', '\x5d6692275c96b213ec69d9837c98bbe39d6e91ebca7a93a5e758ea67533fe945', '\x01bfb5718a5193b30f518a32c5ab012efd7686d2c55cb8954ac5639a9e96d52aaaf12b233b7d5045e847e388163078596a3ce49d6d485ff0dd7249986d762f54b6', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 206, '2026-11-24T07:08:35.554459068Z', '550bb5a41d7834d1', 'legal_read', '\x5d6692275c96b213ec69d9837c98bbe39d6e91ebca7a93a5e758ea67533fe945', '\x01998bd6859b699a0d40cbf2883f01932299766640d40693eefadf41bc0b9453d49f60d962b5893dc041814271a1e4705123b7316650110d96f21d16fb9e921afe', 'fb3c57d6c8601242d25cb84ec9adb7c1', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 208, '2026-11-24T07:08:35.563171075Z', '550bb5a41d7834d1', 'legal_versions', '\x785af0885a5169d79804133c4149d60e1b0cff0b57635f85b82fa02c5b450a20', '\x0108b113cf7138427f5c96b0e309a62859c9675b976bb2391dc4dc6787e44107eee17f3e957048167321d53880e3be2e41a2e68eaa14d8b84f0d42363dd0d04f41', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 210, '2026-11-24T07:08:35.570355975Z', '550bb5a41d7834d1', 'legal_read', '\x785af0885a5169d79804133c4149d60e1b0cff0b57635f85b82fa02c5b450a20', '\x01936396ceda5c41d30f0434dc96ccd8e5794b0651011ae8434fcb18e1179b9aaf9fb3a4037c5ed1bf72461caf8671ad5b2a6046c9b1bbd10f47f54bec50de2787', 'a002ce886587f75a6a367d13ba9bb11a', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 212, '2026-11-24T07:08:35.575887858Z', '550bb5a41d7834d1', 'legal_versions', '\x8a65d1c05776546b76f6044bf285bb932290a53d7d07808c1d1e333c9486903a', '\x019f7c31811113c20e81cc8fd535739f72c28dfe6102160cbb258d8232ddd6ed6cc7a7378a809e48a22d583923b2dcd82d72fdbde0c93dc6dd165690487bed9bfa', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 214, '2026-11-24T07:08:35.583643084Z', '550bb5a41d7834d1', 'legal_read', '\x8a65d1c05776546b76f6044bf285bb932290a53d7d07808c1d1e333c9486903a', '\x012507f2b3f350a96b37fbf635db271939d4faf8ef7358d58e9841f12fbd7170f53250a645076866b6178ba81bb2aa83f1514dce033794294f58c190c5f6f31233', 'ac0633d5dc955d6a6b6f4eef5bd2efc6', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 216, '2026-11-24T07:08:35.589377792Z', '550bb5a41d7834d1', 'legal_versions', '\x678f24a35e91112646f2ce09ae44440894612ad8badde2ebdd499e3646e1226f', '\x01edc070c2120ec2309a8566051730b9faa08f0fb1dcc79998f708407ecf945390ee93bdf740b61e24d8fdefd8a547b40375fb89a88d7953ff6ad9525494dcf467', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 218, '2026-11-24T07:08:35.598205217Z', '550bb5a41d7834d1', 'legal_read', '\x678f24a35e91112646f2ce09ae44440894612ad8badde2ebdd499e3646e1226f', '\x019b0863340efdfecd312d9f181f4d42838e69c32ea82a1d88fbef343b7a83bc875d5f7958786173f68f486a53fc15bbebe4a62e9ef250fab13d9085232252e2e4', '7855312e79e46f001770d9f87ffb4a37', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 220, '2026-11-24T07:08:35.603669202Z', '550bb5a41d7834d1', 'legal_versions', '\xcf496d9f22fce17c60ba49c037b58c0af813d27fc06a62c87fe373a7658d53c6', '\x01af5cef38f67e54012f9941c1baf54a5adf359c8b1c3e3f4dcd7a3d0fd04a700ecd01f4d7eccd5266f04dd0fada87979421660d90e33139bdf20f1190446cda96', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 222, '2026-11-24T07:08:35.612667553Z', '550bb5a41d7834d1', 'legal_read', '\xcf496d9f22fce17c60ba49c037b58c0af813d27fc06a62c87fe373a7658d53c6', '\x0146a9ce36ecc24bb20d7558eafb6208d488ee471364894a5f359dd7c988241102f18972ef44b0102bca5c7f85f47056b0f4420a2df7b071d01324fcfa23e5015c', 'c595dc9cc01901cdcf2b92a92a1ee4f2', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 224, '2026-11-24T07:08:35.620505678Z', '550bb5a41d7834d1', 'legal_versions', '\x0b85d0ada4736875ea87a5b30936da0a87d454e3ca4439dc3bb1c17077ae9b58', '\x01e848f3836fae5f66c8353faff0c175d7db0240b94b26a43fbc05782014db89b2c15a8e13a8764c8b2698892c77aa9e524ac4fb4af9ed0a5b09f79ac328c690d4', NULL, 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 226, '2026-11-24T07:08:35.627583317Z', '550bb5a41d7834d1', 'legal_read', '\x0b85d0ada4736875ea87a5b30936da0a87d454e3ca4439dc3bb1c17077ae9b58', '\x01b212b520d82101bb3c04376777e7086f7f791a8ae71e558db6b1a9062271e997f498433dc6728a7dd11e2a8c0305703e3d10bc90641efa8dca97a070d126066a', '3acb2014914b61564ab5fd9dfdf94475', 200, 1, false, '\xde6f704d5c9003f3');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 228, '2026-11-24T07:08:35.635847499Z', 'b93636aafbcf3853', 'legal_versions', '\x65845f49e961a9568485daecaf5dedfead22b53e045787b844c758524407f2db', '\x0177542217620cc2f2ff2a5d3e05ea8434fd76eb8713a4c4b2cd8857e229fd5d5f47c4c175ac49dbfee1d30ae8f988d9e1d56221f8e61a623bd360c12a7280e8bf', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 230, '2026-11-24T07:08:35.644277644Z', 'b93636aafbcf3853', 'legal_read', '\x65845f49e961a9568485daecaf5dedfead22b53e045787b844c758524407f2db', '\x01093f33304b0535a549234d4286ceacc56b255cb728e6da04436cef07e856736cbac04619a513fed55c066361630dd156cfe352f121f22787c9012cd560389a5e', '4059b5f0afca2d95db040bd7c204e6ca', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 232, '2026-11-24T07:08:35.653877146Z', 'b93636aafbcf3853', 'legal_versions', '\x9caa423a3c012226fc61ac82ff22432d669a1fbf051f24bd13ded13edfebf8e7', '\x01ba85f84b03bfb109166d6402f5a88ef5c467c85dccd14ccd83711c407d375e3861c27e73237c9fd28552e4ded8da7380fcd0b17c9ce4ff66dfb03bcb9d1cdc60', NULL, 410, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 234, '2026-11-24T07:08:35.663109397Z', 'b93636aafbcf3853', 'legal_versions', '\xc3ba7f37391b77e50fd4d4eadf416614424b7fb41727012f9fb5274f30d0d9e7', '\x013da931fa2d7debdead6fd3787ffded41e0007aa72fe523cfab34acf9bd3f096c9caca2fd66c009a7e1ce75b541cea8f41b979f529686500d60cb90bfb036ca13', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 236, '2026-11-24T07:08:35.675916921Z', 'b93636aafbcf3853', 'legal_read', '\xc3ba7f37391b77e50fd4d4eadf416614424b7fb41727012f9fb5274f30d0d9e7', '\x01ca276a8f676204a417bb0c79aa81b74dae788752268af54b5f99067a44a46d2601c6e82dd6985a2b216eaba0d5835ed9e44e7fcb6b23df07a48f4330c72d14fc', 'f0a8e979bea54ad28d05a68c50d62304', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 238, '2026-11-24T07:08:35.681351933Z', 'b93636aafbcf3853', 'legal_versions', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x0193ec9e3cf8fbe9197b5688fc1d4a4ed7a564ab038e5698dfb541ec7a1b602b192c5c1ea9551354d2de87a2387b0b6dcd56e8818ed447b95cbb3c685409fe1b01', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 240, '2026-11-24T07:08:35.688859798Z', 'b93636aafbcf3853', 'legal_read', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x016fd4e1bdb10119e1e47bd147ea15ca06fccbdb9c93f10f998cdf73f9288c46c0066519f37947554e4acfb4bb8bebb34b223be721aad725095376beb47a503fad', '244032c92551a179ad4bf10d988c4811', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 242, '2026-11-24T07:08:35.696398606Z', 'b93636aafbcf3853', 'legal_read', '\xf1c922d684c3004a93d8cf3c6f6e472c12a402a07114e2e3f2743117ac126ecc', '\x017ce80e25f3149cd054a7c38608a507a38fb3cd2077950d3c3a5dda03c7a4efa3f4b4ae9914013c26c4318a2f879f22f6adcead96fa411b4161c122332f740779', '23bcde00e08ff12a90065ba58a27c605', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 244, '2026-11-24T07:08:35.706634128Z', 'b93636aafbcf3853', 'legal_versions', '\xb158ad95062b112de065a29a74014e65384ab1c6d151a4791ec2a0bcee1628ff', '\x01a247c48d27cb99891e51d5192909611a3a369aa8c5c3abc470a4e2d634597d6085fe1513efe26ae0cc713a5a552b9ecf9deb180d02a04e2bdb1a3b18fbe78c66', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 246, '2026-11-24T07:08:35.719585514Z', 'b93636aafbcf3853', 'legal_read', '\xb158ad95062b112de065a29a74014e65384ab1c6d151a4791ec2a0bcee1628ff', '\x016c2a391142dfcf818a704cb7dd4b2e84eb5c9fd07a6afb3e09364e83cd00ad55e96542ce74e2e9e1c2b5ec3a9f3886e3c1ecfa0fa73db757c2b20343407b838d', '957d5d95d211e69f049a82719a9c6c49', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 248, '2026-11-24T07:08:35.729935899Z', 'b93636aafbcf3853', 'legal_versions', '\xa490f90879b3ea4f9abd4b4b457baaa7061dc8c5c252bbb1b46dfc91e50d8790', '\x010972dc37636cc8e3a36171339845f560627fc85977fe6f142929b338c8971249b1ef1657f4f191d404c70a3c40313a02165cbb01a960bebe878b9af5ddac47ab', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 250, '2026-11-24T07:08:35.747297978Z', 'b93636aafbcf3853', 'legal_read', '\xa490f90879b3ea4f9abd4b4b457baaa7061dc8c5c252bbb1b46dfc91e50d8790', '\x0199ca6a0b5d067183254dc9e4cd48d7f4dcc6c12ce8261a5bae73dce76f13077be4b8d70400587051d9f2224fb850fd73c602542a316d683a1b20287b18a69c62', 'c48496b5372837975a307c6752b1845a', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 252, '2026-11-24T07:08:35.762420381Z', 'b93636aafbcf3853', 'legal_versions', '\x8c2cd054b7010c1b646abdb1e4fd9af67e3ad8c937cb409eb5b4400aee6f1ccb', '\x014b719170ad22f3ace3ac5cc381712d9c5cbbdf26fa739ef3aa2ef3c9c879a27b9b19ace402c4f3736e0d893fe33d721486285a3c8e711026b1845556dc167ae0', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 254, '2026-11-24T07:08:35.781208929Z', 'b93636aafbcf3853', 'legal_read', '\x8c2cd054b7010c1b646abdb1e4fd9af67e3ad8c937cb409eb5b4400aee6f1ccb', '\x016216a3420d1286561f69fd71bcdab2c8b2cfa96989055100d5ad11f42c5fc3e8e8eb6980d79a46b4457db1fa196d0ed79d27d494d6b43b96f43ffcf5a7a5c186', '8e315ac0b67e962d123313e02a61a6b6', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 256, '2026-11-24T07:08:35.792591604Z', 'b93636aafbcf3853', 'legal_versions', '\xc0cef42c5acd6648d7b05f51020ab16dd4e57763e986ffb37c4c1abed03982ed', '\x01502635cb96a6e00eb03947916327158d72047942df262dbb5f19923ab9c6d0e6320bf6349231d800d31fcac01f263cfbcf51d7063a6fe9e00cb5b88fbf69685e', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 258, '2026-11-24T07:08:35.805547193Z', 'b93636aafbcf3853', 'legal_read', '\xc0cef42c5acd6648d7b05f51020ab16dd4e57763e986ffb37c4c1abed03982ed', '\x01c4e814f686f575de98e8576f9cd42ed6231cb09c653f777975d22d84c0b2a37c67a32a21c254a5e3a7d2f1faca44bf5788d5e0dda05cbf1001a7cb9c890d06ad', '34db5e45ca01e104b2da8402f9156e30', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 260, '2026-11-24T07:08:35.814903101Z', 'b93636aafbcf3853', 'legal_versions', '\x3934e25f809551182a9c95809513808185fa8f4dff1afa45c3f9a43352d0928b', '\x01d0eb9a8ec07c317314d73a23e4dc3335d939e9407df9b52aec4246f99ea286a42105b3f135934b49a567827f9a65db47cb4a2c083d79558de5c4495acdad3549', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 262, '2026-11-24T07:08:35.825754571Z', 'b93636aafbcf3853', 'legal_read', '\x3934e25f809551182a9c95809513808185fa8f4dff1afa45c3f9a43352d0928b', '\x0110d1a8ef67c54290fc75fc22cd712948cbf0a5f4ffbffa7d31692f813c74e3c2a5d81c6ef6a18ae6eec7eafc2ad781f7cfcf2b2a6f1efd5c46947f00b9e25dd5', '8e43fbb6d44817a7fa5241f8c90eb766', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 264, '2026-11-24T07:08:35.835552785Z', 'b93636aafbcf3853', 'legal_versions', '\x23539626d668dc7ce69a63007b84bc379c0b204a9cdc92e4258595a17bfc94d9', '\x01abbdffcb1f9398c1a3428548d9563f26e6f25c7b2f5b02ac43a7a23618c7eacddd051fc4421263e77a4eb99239822a89453c2f2a4bb371ca2657bfef5fd93893', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 266, '2026-11-24T07:08:35.847500824Z', 'b93636aafbcf3853', 'legal_read', '\x23539626d668dc7ce69a63007b84bc379c0b204a9cdc92e4258595a17bfc94d9', '\x014a2b9a6ccff64c1c008700b373867762e7a90fe275bd73f43e1db067ec9567ac0f51276a7373c37ba2ba9be9ffd7e188b59023230b5531b01c86b647595c336f', '7abe8eecd801432fbe62f61d16701b5d', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 268, '2026-11-24T07:08:35.854675677Z', 'b93636aafbcf3853', 'legal_versions', '\x37549ea96abc26ea6c2dcae5b3a94532a60e102a8a715fcbf3c0f353b9bbe2cb', '\x017b87b142b016c84e7f35bd9e99296b68faf1285b2c91d84d5fcfaf72e07217ce8cb1c3a763367a498f4ac72872edbd3ae1e43b3310f056a98f8691a03a14081b', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 270, '2026-11-24T07:08:35.864035289Z', 'b93636aafbcf3853', 'legal_read', '\x37549ea96abc26ea6c2dcae5b3a94532a60e102a8a715fcbf3c0f353b9bbe2cb', '\x01106f2957483578e5f9a311dbc8fd087a402d1a393d76d76e649e2002634d8513b4a48388055b33c5ba7c1f2e5313c569f7f1b55d148729c3bee2bb840c138f7e', '9123470233f518ba808689b8310425df', 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 272, '2026-11-24T07:08:35.872557591Z', 'b93636aafbcf3853', 'legal_versions', '\x1dab80fc04440caedfb04470ea84ae4a47893fa1d19e49993b7fceb3648fb364', '\x01017ae69447043bd6f989ec94a0efa66ee9595e6319d94aa409271504485e410cb30e8d5bd3cbbd706f75cfbce969d72f15f726d070ebb369d515592ac0d433a7', NULL, 200, 1, false, '\x05348fb7acd3e90d');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 274, '2026-11-24T07:08:35.885441328Z', 'b93636aafbcf3853', 'legal_read', '\x1dab80fc04440caedfb04470ea84ae4a47893fa1d19e49993b7fceb3648fb364', '\x0102230a439c9f9747c201c66c713d5f0db11304ad75a1ab3630cfcb6607d4f568a2412246c01b509cb9924b1c49c0ced2a82a54599fa139a6523416710d3667ea', 'ff4a8413b26f55592e46851c79855121', 200, 1, false, '\x05348fb7acd3e90d');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xde6f704d5c9003f3', 1, '\x012f9ac3a4e43413791e940779082e0dedb6d6000fa701065a4fbbef5d36f01e0bfcbb3375021fbfb9c5df935d086a564f0cca6de514b2bf0f7e54e258');
INSERT INTO vcon.audit_key VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x05348fb7acd3e90d', 1, '\x01dbd1b133275f90603a1ed967cd13f789ed094299d6a02e92ce4a992686239982f13849514df0f434ee94f4953166c31978df26ee57e2645aae640290');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x2814231d5624c683', 14);
INSERT INTO vcon.key_use VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xdda8d728d499a435', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', 2, 1795504113871393353);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', 4, 1795504114579529874);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x39a050c157acdffd', 2, 1795504073741292244);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x83dbf4e0e57439380cf6e547f10a677c', '\xa08b91edfb9326d51465c6015964f940', '\x01bbc925d090c15ce6f9c91e36ef66423e0646d3a412ef1863114d22bf213c6e023696', 1795504100596391749);
INSERT INTO vcon.membership VALUES ('\x2b0088b41807dae32b3def63725d0df5', '\xb3bfb876578ea80322efde7742260ad3', '\x015c9f33e751828d2d5b65a2ccb09ac4d6f984903b81c27e754372a9bdab9b14b2b685d3', 1795504103066825408);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x97ec4d4e594d2dddf4be589563208c22', '2026-11-24', 12, 12, 7401, 1795504113018995593, NULL);
INSERT INTO vcon.parquet_file VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x02c6b828b2b822061d1903b7bdd41a3c', '2026-11-24', 10, 10, 6237, 1795504113934199494, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x7370f3511a7766d2');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-24 07:07:50.829487+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-24 07:07:50.874073+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-24 07:07:50.880537+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-24 07:07:50.881182+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-24 07:07:50.883504+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-24 07:07:50.88964+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-24 07:07:50.890325+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-24 07:07:50.89522+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-24 07:07:50.898205+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-24 07:07:50.900022+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-24 07:07:50.902071+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-24 07:07:50.906236+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-24 07:07:50.91481+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '78fe21aeaa0e0b0db459988bf11048a7');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x7c35049bb6656de681f5269547447b61dbdde0cfa598834e09776a4b483883fd', '\x01bb69b7e1d8dca14dc1d509b2cb32b011be40a851f5633ed66a4c6a5072fe72870c3b2b12', NULL, NULL, 1795504073781947544, '\x2814231d5624c683', '\x01dfe3ddf0c437e36193c2104df6c38a658feeb3d802d03df1865e072618756ccc89605995c346541cd7c2d52090e8829caaaaaffbd3fc87bbc27424a9', NULL, 1795504073781947544, NULL);
INSERT INTO vcon.tenant VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x22ffc85d6114b0d7e26a18a6e45c93ef505822a1bb20262f531ac15f4797dce2', '\x01e407cdae4849088c593c48dd84989577bfac8d2f65320048a833ac7b57522b48753a9c65', '\x01406ec99ecefb21a949e18444f397d13d063acf31330404f325efef6eb5fe', NULL, 1795504081978787941, '\xdda8d728d499a435', '\x01253a61907d3760ef1b9be090f51fabf3413f4996ddf100446acb99be61c9f03ce24e381ccbce678a64da4f00ec232ebc32fe228f9bf0449b5989191b', NULL, 1795504081978787941, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x257e5c205b0575ff');
INSERT INTO vcon.tenant_catalog VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x6b8dd1c5d69104f9');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xa08b91edfb9326d51465c6015964f940');
INSERT INTO vcon.tenant_lock VALUES ('\xb3bfb876578ea80322efde7742260ad3');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xd0cc2c4adc115fb509e988bb1905c6dd768c95a9c68c56e6b05427a3ee8556ad', '\x0140f711603924a8bea6ece08fb8a73cc27c5d80479ae2502a348b7f319334edf4fa5d2f8ed9e4c87fb3a48137', 1795504107144180395);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x83dbf4e0e57439380cf6e547f10a677c', '\x01a26d7e4d3630d383b9402db93f932302fcee91bc9a81277b7384a3291f6adb443f138fa007052030958e', NULL, NULL, 1795504094080852252, NULL);
INSERT INTO vcon.user_account VALUES ('\x2b0088b41807dae32b3def63725d0df5', '\x01bac609a242e176b5174738fe4a364ca6aede53ce1d2bb8f6318a9618db92a7d494ab69eed1dcff726e0815', '\xfa6fbd8a8421ee73f470ed5627cb77464d5439fc637c3ad90a7572767c81d187', '\x01659079fc0bca90b18ba066009dbdc811c6a059689de1338aa41148f022b5f2fb67b6326994b935871cdd99a1ff1ab44ef1ddd220b9ffd72f8b52f02d68586a', 1795504096716627122, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xba03022f03aa86d5eed8f4dc92b53a95fff4e9c9f0a07d5c73656700c73640cb', '\x01c8f275867e521516710bf03145e39dcd461efe10d0bd29a7d6a13a29eb8009a5823fb4f3b11785b229058c46', '\xfb3c57d6c8601242d25cb84ec9adb7c1', NULL, 0, 1795504106372827317);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x0677c7dd99b66bf2d9ac07775b152cba388a75a072baebb1f0f901b698b91b79', '\x014e66cd424c27c8cedf929794c9ac8ef3e9614dd01fd7db993609f637167ead8f8c0802c853406f12b298b17d', '\xa002ce886587f75a6a367d13ba9bb11a', NULL, 0, 1795504106403390895);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x4d3a9822c9d92582d21ad49ae29885a87780e1191bbf4340d08387145e9e5249', '\x01b4452b039bbf68b10db9c29cf76d427f6d01858824ff10aea5519224ce40261481560b311ba7adf36851e47d', '\xac0633d5dc955d6a6b6f4eef5bd2efc6', NULL, 0, 1795504106448720254);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x53c166008abbfe26d1fa8dd3faabe3bae760ff3bddd9da2a1ab3027d959db9b5', '\x01a86bcb255feb23ae0db41eea3f514180d5c59815f29e161ac4fb3318712619eb98c1cc3d384d412eb96e369c', '\x7855312e79e46f001770d9f87ffb4a37', NULL, 0, 1795504106486422434);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x45b04f977d79b08ff6194c67235e8cdfab55b3d3ec96e72b515ed12869e25ea4', '\x01372d55cd278c68f259ea17fa3856b3461f9a73d65c3c781c45d3430a9ac5e57713efac33ea5eb090e8f69328', '\xcd98d783a25bd2979f49f595956fa291', NULL, 0, 1795504106098591992);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x294a93105ca9f7f9c0fefb2be47fdc26519fa5428cb8f5e1a8f9babea2cbfc9a', '\x01d1a7daa670fbc42adbe552861cf817cca4088a62a120d5612d5582219035f671eede8688716e478139800072', '\x7dabc86a35385fb078d8dde7292c72bc', NULL, 0, 1795504106145892046);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x68eff676c167c4a88e06ff8a1f50ffefce676c8d08e8820067e71d4339810591', '\x0177cab14fbe1a1345eb5c13a17b2661185757712cfd62881db19517988202c0d7b62b04c5b29995b7ac95921c', NULL, 1795504106612329324, 0, 1795504106181658024);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x7c00ed76b1387a03edd75b4925d66e7272dd8d6d5091f1dbcfa5c0afc6576ba3', '\x0108fe1a66eb8305e14735d04a755c0729b39a66ea4198b84fe253a9acead307e5c344ee7ba08844a98f9663bf', NULL, 1795504106624092001, 0, 1795504106232511619);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x5f232624784e47b53a7eca8f6c6714d677bde136b9b2d87cf17a6e4e76e2ac05', '\x017649790f81fec90e019c41125abffdbd765fa6aa8fbe86f7900e228c96e01181f1e83f2fa4fb7b8b399f97b9', NULL, 1795504106642833939, 1, 1795504106274098500);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xf365f4de729e42c804013fe759be4028145f4a6f7ee5f15180f9e7d6f112579d', '\x01ae2171205fa55d4c63635aee79e8cd944c761430766834f57f4eaf4ecff12b43467da9df57348b5a91caea51', '\xa291e5035008459ebb6166a6b971c92a', NULL, 1, 1795504106325953127);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x0617bcabcc071affe530e9739a5500d104235a5b4bccd9f0e0878f504a1f84db', '\x01f2bc66c07295c2c6724378c348f44f00e8e4d644c186c121cd3b44fa731011e2b9331d1d5baee6c079139a58', '\x957d5d95d211e69f049a82719a9c6c49', NULL, 0, 1795504106869129179);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xc8db11ba7964377d4331544360aeece2d7bc034807fc603995d4fa44cfb569cb', '\x01e7b73090ef03819795796b79460346e55bc04348af48b34839ad5194f1dece1fae4be3e8bb4255ab5ef8b7d9', '\xc48496b5372837975a307c6752b1845a', NULL, 0, 1795504106902787751);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xbf20ba41b791fcf33604fee18ccf2248f417696d09f16d73ba4659d76fc3a40e', '\x019f478354d9c5cd5b2825d8c407e13643479537a2a7b4e1871fa3c8730304430b1b4aeaaf0093c0f400cb071c', '\x8e315ac0b67e962d123313e02a61a6b6', NULL, 0, 1795504106934247249);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xf30b1342b052e9d49acea0aaa7a89701d73693be6cc126fe15767b386d26777a', '\x01ebb200209cb15a9ca6ca62ccfa67899f6c3a38b6a5dcaab96e16e00e90f1b2a16b9ca01a9e2c30ebc220c6ba', '\x34db5e45ca01e104b2da8402f9156e30', NULL, 0, 1795504106969898421);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xae55a4efdfee72b8e9a44c31724c282463f26c3e389dd4076bfab7b6c27ad80c', '\x01a559e4d399a746987d02b2cf8fcd2f049e95ec1bd0492ebea2bfba72359a9b8246cb059066715950f5ec6dc4', '\x8e43fbb6d44817a7fa5241f8c90eb766', NULL, 0, 1795504107004362181);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xd369f0261656ff0e401fcdf65aa36443a6abf3189a0727fcecfc2e28c1aeb470', '\x01ea8b546e40dc6c2b07aa5b617d541015ac43815f34515c0e8fa74e07d706a4e311f2d3c058c35deb6c32c788', '\x7abe8eecd801432fbe62f61d16701b5d', NULL, 0, 1795504107055030747);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x936f73f1999bc715061f62ddaeba51d3cc830c11a5fa9c85d08ffcd72b1ac1bc', '\x014405eb0c4ae520be6b5f55630f84db7afde201629fa600b564eb182d8a1a90885a07a7a11fcf847b014336fb', '\x4059b5f0afca2d95db040bd7c204e6ca', NULL, 0, 1795504106687129166);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x9febcb95fc5649a2fdab9b2c0d07bc1ad29a733bd8b9d795c91ef91a72e6c4e2', '\x011da11e7dd52a9448b334876a9cbe1c2f0d21e25a21bf507a9459455f5fdea2b6b5cc2f6af455a68fb2670e4f', NULL, 1795504107269116012, 1, 1795504106761132710);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x47c942d3cd26c2abeb5e092f80d1d3fbb71ce96d8676ebd8afb1a81a2ea650bd', '\x012b55b2cba5f1bbc2446cf6dd737d4c6196a398a4cff2b424ab24fe653167596b3de8aeea1e1aa3c99fc55e3b', '\x23bcde00e08ff12a90065ba58a27c605', NULL, 1, 1795504106803805403);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xa628dc305c14075ce776e0140ac9111a566bf86d224e2f26ae6ce9ba18720fc0', '\x01fe78926c996daeaef6a216092e18d03ecac13715867f79bbdcbf28e38b9e1b573a88d619aa79f2d48c59526b', '\xc595dc9cc01901cdcf2b92a92a1ee4f2', NULL, 0, 1795504115328310574);
INSERT INTO vcon.vcon VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x587510797b15a66ddcd751d56e0c6f3807bfee9178b7104ce17d30494d9f1592', '\x019c0deb3489c40567642c91c61ed35456a78876bb0033815aaad6cd04123b7bf45666d077428525d96a8a73cc', '\x3acb2014914b61564ab5fd9dfdf94475', NULL, 0, 1795504115355058034);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xf9f715e5ae3cfb4e07150d23627a65bf2bd994e1cfb58c20fe3cf90f4fcc77dd', '\x019bab851b0c73f0aaec30dfa868f655bfe9afa4e83db013c222ad8f1fb196eb60618df18314b928114733ee2c', '\x9123470233f518ba808689b8310425df', NULL, 0, 1795504115383986177);
INSERT INTO vcon.vcon VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x55aea36344a595e7087eea06a79746c648f81c9bb138352a984a2294da52683c', '\x0104bc60c79ff55a69b01bc4352addae5281d16f3afedfc003a990780ce959e2adfbf24b173d1b27e3ebe9b18e', '\xff4a8413b26f55592e46851c79855121', NULL, 0, 1795504115422432378);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xf0a8e979bea54ad28d05a68c50d62304', '\x9febcb95fc5649a2fdab9b2c0d07bc1ad29a733bd8b9d795c91ef91a72e6c4e2', '\x011c5dc6c725b5c2427e9d3c83c554126f232fd6b46e06b55d23593e515cbdc0b63fccab9a', '\x017e7b84b9b6dc5b3e6ae5252015855378d8cab9809a5755461e038e5732', '\x01ef31a877c2ada9043b77b03bbf0041ca6b8b5790e4c46fe9f6f95864bfc4da922268f0c2', 1795504106761132710, 1795504107269116012, '\xdda8d728d499a435', '\x01d634e5183befaac3e36544a342783655656cffc1f2e6252a0b0ae4a9cf0485d80cab654987993cb54c9cb4f4c207a6e330c41ff216c0b5fd249a6c28', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x244032c92551a179ad4bf10d988c4811', '\x47c942d3cd26c2abeb5e092f80d1d3fbb71ce96d8676ebd8afb1a81a2ea650bd', '\x0109b9216b5d299f6e432b3b5cb42d2bf5ba2e3e1df69017cbdf3ad3cf48132706b02ce4a5', '\x01ddd03f9d2690bae640e1f7b9af0dfa8da1b0ca3840a40e570b4ac4d150', '\x0171f3dd08e319c8b5c52cb120585262caaae68e6a46cc5a9aabde228fba13100e1f891ffa', 1795504106803805403, 1795504107318376306, '\xdda8d728d499a435', '\x01d261e36043f0e81acac927573efa265c903c4fd60db9aa47bc52b40cb404ec9c41a5d30a27a4a06589ef5ec36405206027c198b2380056e7642a0d69', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x957d5d95d211e69f049a82719a9c6c49', '\x0617bcabcc071affe530e9739a5500d104235a5b4bccd9f0e0878f504a1f84db', '\x01479f4ab9f267f3054e1194cfae8a8897e1a5a8d99308314ecc9f4f7db7aea9963a82d260', '\x01a93fe58361eda3033ec9f283107e782a5e463482847ac05bbb736be189', '\x01ec2548b90086909e1cf17d9d2950c5991ffff66e36cee783096e91927d2c9360df8f227a', 1795504106869129179, NULL, '\xdda8d728d499a435', '\x0133e48003804b639e34c476b2dc08c593b00425510a6aa27e0118e277d1bbc189b98fb8aa166d52e73aa9c91ac347eca73f71031b6e67bab9ac98bb26', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xc48496b5372837975a307c6752b1845a', '\xc8db11ba7964377d4331544360aeece2d7bc034807fc603995d4fa44cfb569cb', '\x016b0da55d5a27d20a718daaa4cd94f80883be4ee0bc773efbca9d1e10e7a805e8b40ed9e8', '\x01c463872eede3020cab36adb80c22b7fdd69219be3e3516734d60b76395', '\x01718a6ca61404edeb75c27c9b3a7a8db86da0615f386e701f8646abffdb627a84996ef43b', 1795504106902787751, NULL, '\xdda8d728d499a435', '\x0131e28956f300afbad075953c8aa4266a6cdce4ced370dc6d29840cc89221a655c063fa7fcd7ec4130c68e1e4e12c4c21c43b53e59a8a0aff7c3684ca', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x8e315ac0b67e962d123313e02a61a6b6', '\xbf20ba41b791fcf33604fee18ccf2248f417696d09f16d73ba4659d76fc3a40e', '\x01f7a974d75b2501db651b48281d3b3127d330d62db22a4e101cf4c1ddebece8a0b5d5f7b9', '\x01661bde9574430d8a91e28040ca9f664ff15506a6fede83763c8765df61', '\x016214c83892913f772d2417ff62ae0b9b5dceb6bf4d4e2f184807821cd442c33b0b339cda', 1795504106934247249, NULL, '\xdda8d728d499a435', '\x0128d9d4d841b885fd92ec67d56e88a02f6cc8a40b90aba5e3b533a04bda384653bc4e9125bec7b8abb36454f1cc13771c965865e7f51efc92d931acb4', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x8a025907bb97a06a57f70457ba5f9ef3', '\x294a93105ca9f7f9c0fefb2be47fdc26519fa5428cb8f5e1a8f9babea2cbfc9a', '\x01ddb67f1de099d930a240a500eed851a63aed2b7ed0ad1ff06439a6c6a8154c162b4743b3', '\x0101063b27f677d7fff92c84f72f0f1529ca9745898cf04ad77d9088f842', '\x0140d6ab415ad2efd24fe60740568d09685c6d1249694a5714a566343a23fbac683a53fd6e', 1795504106145892046, 1795504106597794801, '\x2814231d5624c683', '\x01577b68feb39e2790611002802e17dd099057ece9d068ad9daf4b4641eebe1a55a1826c855a5461c6be9d24f23ee8a2a98c93d8004227d190be376e0c', '\x97ec4d4e594d2dddf4be589563208c22', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x9a13d4d8ef63cb902e4fd33bd350d84b', '\x45b04f977d79b08ff6194c67235e8cdfab55b3d3ec96e72b515ed12869e25ea4', '\x016e6a0519940534d55cc797c87aa88e2b1ac69db5d6dbb697df036aaa9525bfd04e615177', '\x01e2325025bed1286d2bfac8a14368ed2cba5aad4b959eef584b6eaada99', '\x01b38fa70cb854547b5c54ce9f505a1d3a62819fcfacd3dfe06fcdddc73e433d1d83e230a8', 1795504106098591992, 1795504106522825295, '\x2814231d5624c683', '\x017b52e7f61b821cb35a83a6b1a7918e0ad19eae151325213349177bec38fa35722d6a47765062a4141d5aa276acc4f920e0014f3145af1446bc34ddcc', '\x97ec4d4e594d2dddf4be589563208c22', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xabdf402a4e5288860397aa1fc839f367', '\x68eff676c167c4a88e06ff8a1f50ffefce676c8d08e8820067e71d4339810591', '\x01939906eb0f4cf8c004e0e4214cf9093bc28293c99b239d0577440a1109556d44ea0fbde4', '\x01c9cbd7977ed50d6d64c999e5cf6606fc16ea7c8594d6b5fb4c9199038b', '\x01fbed6f464bf3d05300f70f086a993ecc4ca01ec3587bf6683591459151727dc301ca47c2', 1795504106181658024, 1795504106612329324, '\x2814231d5624c683', '\x01a48cd85707f8da64b47c45f745298bf82e2176e4b28666b13829db230c4f8b6a51bd238c7140630868597137b51cea8b942e2344cb865dc6732424a3', '\x97ec4d4e594d2dddf4be589563208c22', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xed4c676b0b7771e33c261b3ce32fa58b', '\x7c00ed76b1387a03edd75b4925d66e7272dd8d6d5091f1dbcfa5c0afc6576ba3', '\x01de25e771d5e245b7340e98fa3e82a4c1c8c57d1ea83e0018288ed6bea2036c923edebdab', '\x010e2010055eb7503e25a0b52830c465792c39057f403818c376263f7a7d', '\x01d1184a274876a68b299c0e2db6784401036b614acbee5f8d5ea6f87ad740b6717744d810', 1795504106232511619, 1795504106624092001, '\x2814231d5624c683', '\x01767aebf448bc2b43de4dfe136dbc3baf7c49a6ac977c47820f239fc7923fc6a142e69842eedeabac71fe6aa3413497d732edfe3db8bac7c77a9115c8', '\x97ec4d4e594d2dddf4be589563208c22', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x34db5e45ca01e104b2da8402f9156e30', '\xf30b1342b052e9d49acea0aaa7a89701d73693be6cc126fe15767b386d26777a', '\x0144ddb4be646cf50910eb052bdf42c2fac98173d4ec531bc0554f7c060783dc0c9045d95f', '\x01329eb5f91f88b38f45c7dfdefcb57ce73fdf2cd8bc0d18e848218652fa', '\x016bc527a939be39312e9dc15bb579afd96f5b7c2d5fd8f7b0fcddda415c82dc5bb731e567', 1795504106969898421, NULL, '\xdda8d728d499a435', '\x0163fc47d9d2fd8a01342a402faece4b815affe2aaa9680c350e3dd8886c49c6f9f179a7603d00c331d43a25987974cc647effd5618c52d53b0e9c6e37', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x8e43fbb6d44817a7fa5241f8c90eb766', '\xae55a4efdfee72b8e9a44c31724c282463f26c3e389dd4076bfab7b6c27ad80c', '\x0104ad1c23e77be6e9b7f8243bab87f153f9375f1f1e240d23de7fbec40c98c0e6c3c723e1', '\x0103d81b0adeed100d3943039ca53c4e8b922588efed552a8405c4e5841d', '\x01742c539ca561a1eab078459dd4646dc446f4e93c8ace106a99560e92d8776ca4a637306f', 1795504107004362181, NULL, '\xdda8d728d499a435', '\x01c362e6cba6ebc85effbb2f654b3287a6d12044faad1c3a3b48e4d93366462d53d5603f148445f136266470e4247ae75b68daf42289600815d3c1a694', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x7abe8eecd801432fbe62f61d16701b5d', '\xd369f0261656ff0e401fcdf65aa36443a6abf3189a0727fcecfc2e28c1aeb470', '\x01692db0b9d3026ff9311355077671fb4cb06363106ce792362bcb03c7e4309e62a90b822b', '\x01bd468b8ba828d27b1a2c13842e8799776d3936dbc96c05e2cc8c8c1d3e', '\x0172fab28896cabb7647e5f72d72b87e1ef71c7cd7e1a8f38016e2ef852c7e55404e860b82', 1795504107055030747, NULL, '\xdda8d728d499a435', '\x01d333ae851b8c0b62668dc6f181fda8e0a2e82d4c1725c83ff57aeb07358f630e80ccf8b091a6b052b95b46ebf9d7dbde6816198485d4811ba53301fe', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x4059b5f0afca2d95db040bd7c204e6ca', '\x936f73f1999bc715061f62ddaeba51d3cc830c11a5fa9c85d08ffcd72b1ac1bc', '\x01d5c43f3cb9209dc0c1d7c5b69970882c94b3029b987a35ebdb40f9dca3596295b266c304', '\x0152ff16ea3c7426d83cb377b84bb40d2a6e52270d476620505be5e14288', '\x018500eb80772c1b57932fb03f50036f670a04b1ff2fd75d3500bc43d8208dfcd2351e736e', 1795504107128337200, NULL, '\xdda8d728d499a435', '\x01851a14eeb2439c47b3c63d6392f32fa73a726c1e8073361719804477a3389bbf26ef5553f286e63130be3a06b8992e385f7ca3c8f01259619941ad26', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x23bcde00e08ff12a90065ba58a27c605', '\x47c942d3cd26c2abeb5e092f80d1d3fbb71ce96d8676ebd8afb1a81a2ea650bd', '\x0176a312d6a1e8db32d22fde23e6d103e726cd7c141f22227ce7edd6947c2f08a9ea055c99', '\x01ffe3109afaac6537e4dfad9f0cf04cb10172513240148910b268146ae5', '\x01cf1fd462401e19625953e6cdea270bbaa65a9c9736a63724c8cc0e855a1eae685621b69b', 1795504107318376306, NULL, '\xdda8d728d499a435', '\x0129d57c60056bc918c855682a8e6efdd5658b5d8ce9d16f489542a5fcc02ee8a3850573cb53bd9da8f5dfa88c783e244fde3df23f0093ce3094ae5592', '\x02c6b828b2b822061d1903b7bdd41a3c', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x10aa06e250f70b4638e0539aa3f126e2', '\x5f232624784e47b53a7eca8f6c6714d677bde136b9b2d87cf17a6e4e76e2ac05', '\x0190ce34a53d829cd8d2904499bc7ba606baca48556338c215afff9d486301ba5411b35922', '\x01769fad989a489d72d9c1dd3007ca0f0d88d539557309ca1202b9007eb9', '\x014cae372c2356d5be9fb2f9b7f3294a7be3d797be1a5281c2ab1f63ffd991a6b53555e74c', 1795504106274098500, 1795504106642833939, '\x2814231d5624c683', '\x016050abb6ab1aaeda505c3dbeb2f288b0c4841222929654a889e3a3feaeafa6b3a6f9a1330f54f1e3791866f2d2a0ba4f1a813931b6658c9394a7d77b', '\x97ec4d4e594d2dddf4be589563208c22', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xa291e5035008459ebb6166a6b971c92a', '\xf365f4de729e42c804013fe759be4028145f4a6f7ee5f15180f9e7d6f112579d', '\x01f65afa2bd2df35ee1e9b338f85ff13e4c5ea1b53efa4b1641f969b220a56e7d6c82bdee7', '\x0174db774b29fea485601f2d2a102a2beecdfab07bee79071c129867920a', '\x01d1574b500d5a60e325330ef97fdf7bdd7dc4afd5746557f3fffb0697c238fac8d425be5f', 1795504106325953127, NULL, '\x2814231d5624c683', '\x01377955af8380269c161018fabaa7dfb2d5dba2e8b4efca66465fd5e9e4991861271fd084fc2cf083f77991610cc57ab4515ce9d0afee9c6fd00dd036', '\x97ec4d4e594d2dddf4be589563208c22', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xfb3c57d6c8601242d25cb84ec9adb7c1', '\xba03022f03aa86d5eed8f4dc92b53a95fff4e9c9f0a07d5c73656700c73640cb', '\x01a60b5c19c8f0853ed024e5501ff06a7592506af93d948cb65f520d146ba1ed87c4b10e22', '\x010e7174f196c192db717dca31a34f01013d18b0cf306434fe14ca98c06e', '\x01fe87e15045b7064437ba96f53efb33199b64643a457c8854bd8ec6309ba0e0895b4e45cc', 1795504106372827317, NULL, '\x2814231d5624c683', '\x01f1aa8ffd540fac417b1d0ca3a1dfbe6ac00c27723279e3a14ee5be520015f25dd8cc08973ca7e4d70fcc768a73627e995e07edfc65b0ec673898b26f', '\x97ec4d4e594d2dddf4be589563208c22', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xa002ce886587f75a6a367d13ba9bb11a', '\x0677c7dd99b66bf2d9ac07775b152cba388a75a072baebb1f0f901b698b91b79', '\x0154b9c943a6261b79ed8e618323571f4b09ada8f864075db1c4d85d953525720277c11fcd', '\x019369342263981b3e253a190c96d3af08b31f1b64cfd961a66df24eed69', '\x01a785f8dd579bb8ad4601df80b7d94cdcd0e21f2ef557f227709e59f495140435952e2d0e', 1795504106403390895, NULL, '\x2814231d5624c683', '\x0192efddf8feb2b080ec0096e4a52dd501af90718d076cca16a15cffef8b05af6bf9ae3f36c604c0c6634f5fdaf3a53410bd5d98700d40f3681f2b5bd4', '\x97ec4d4e594d2dddf4be589563208c22', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xac0633d5dc955d6a6b6f4eef5bd2efc6', '\x4d3a9822c9d92582d21ad49ae29885a87780e1191bbf4340d08387145e9e5249', '\x01a7b118a5b242cfa7717fbdae563e1527b8edd7feaa64daaf229cb3903d1c720cd98ab9b4', '\x015952ea733df559fd5cd4186c055250a0f0fa2fe37797257d08cc9f2d0e', '\x01acfe399d194b36c55480c193bb00c03e6826d734c2bfcd1edf45d68ae1076a29230e5bff', 1795504106448720254, NULL, '\x2814231d5624c683', '\x014558508b0c8003b603ae8f9dd9d6f0a020cc11761374f7ab5b4065ceff90626535fa36ea6a070aee87403383359af7fb8b065b1ff60a399b2cf0d5fb', '\x97ec4d4e594d2dddf4be589563208c22', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x7855312e79e46f001770d9f87ffb4a37', '\x53c166008abbfe26d1fa8dd3faabe3bae760ff3bddd9da2a1ab3027d959db9b5', '\x012ad45a2e40ee9a7186e19f129328f435add347bf4ec9cdab86b6ae7a72af83cd0764b316', '\x016a9cba7072c1aa024e4ad9c2e6251d12fd218ceee6fc7679edd6c15bbe', '\x0142f82113372f2d3946635d58ff950edd5ced077056e380703e98e2568bf164f19873a7ff', 1795504106486422434, NULL, '\x2814231d5624c683', '\x01c7d6fd0984082d3267357c5d904b970a6727e58bb1e95702b84e99dd90df562a57ecf519a2e59d2955aec8907b0e2f021ec76de588af1d24b55d8b82', '\x97ec4d4e594d2dddf4be589563208c22', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xcd98d783a25bd2979f49f595956fa291', '\x45b04f977d79b08ff6194c67235e8cdfab55b3d3ec96e72b515ed12869e25ea4', '\x01c375897e95f6fb8260afe084aaa112fd5473a24241ebb5748197a60263bbda29af985562', '\x017b912dbe1c453e6f84b817d404af9fd003facde0a54181be6de0279bdc', '\x013768796cc286f50b94bbac830c324696da1dafbbfa7a1369b08ad021d75f58f2a2b261a3', 1795504106522825295, NULL, '\x2814231d5624c683', '\x01e6efec099b82f027ee07674d7350756179f44fe3a24069ed059db5341ea2df18e59f9a662f68800aee2fb7e92ad1c9122eff634a4976788eb17f2801', '\x97ec4d4e594d2dddf4be589563208c22', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x7dabc86a35385fb078d8dde7292c72bc', '\x294a93105ca9f7f9c0fefb2be47fdc26519fa5428cb8f5e1a8f9babea2cbfc9a', '\x01ac4ebbbfc021de025090ad1fba8a70405b3b63e3cb47de6daa8d8e082c30cc9918e2c778', '\x017f4081f4fbb1cf610d0b9fdd55e862c3ed522517009c6262d9001563ea', '\x0156fe84545ce55b7d1739e08e2e954b8bbf743866e8538eb6449a38b2d8dfe153a08d5563', 1795504106597794801, NULL, '\x2814231d5624c683', '\x0134037f98219cd8133721889457642af11dbd28d2928d8bbd11ef41f6fc12a369a4e3317914e6bb682978c1c5a34325d12b129aff2e34a8f619c7e303', '\x97ec4d4e594d2dddf4be589563208c22', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\xc595dc9cc01901cdcf2b92a92a1ee4f2', '\xa628dc305c14075ce776e0140ac9111a566bf86d224e2f26ae6ce9ba18720fc0', '\x012af887a82c7e01ca2f41646b4bac52b3f2503a8e875963e0b41daef08ab0bdd652c428ac', '\x01dfb727bf5a2fbfc75633298a9262614c3354c02a9534acc322701e7cd5', '\x0160314b17ebc8e3e5a93c6a150937297f996e064ac50cf8fe7e2e2bfb40d68a8e027b5ffe', 1795504115328310574, NULL, '\x2814231d5624c683', '\x01c48934a0927e40b030ecb14d69546baee59728172e31c4c0888a4f98d49c9bf2605ee649f8234c8f4e730e565122871b3ca94dc393acb843ab1bd8ce', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa08b91edfb9326d51465c6015964f940', '\x3acb2014914b61564ab5fd9dfdf94475', '\x587510797b15a66ddcd751d56e0c6f3807bfee9178b7104ce17d30494d9f1592', '\x019f8236227290a514db3419997f96f94bf83ea3b288b4a3b452a50ba0d5959663cbc75755', '\x0121349afde61a3dedfa931baef44fc3185b0f7da20dfc78632c2ff5c603', '\x01df99a1053bf69e117735250578b22d39ff4c460d34871c7a43a9fcff000ce698edaca248', 1795504115355058034, NULL, '\x2814231d5624c683', '\x0115a3e17d5a30b7066b675c3fec24f0936f6fe180a29bb3aba94cd5bc18f45955735bbcc51556a5645fa78dc09c31bc223051a82d4ff63ac07fec0bc0', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\x9123470233f518ba808689b8310425df', '\xf9f715e5ae3cfb4e07150d23627a65bf2bd994e1cfb58c20fe3cf90f4fcc77dd', '\x0186acf7169886b890b0a031fda9818b4b34c4fe3f3c948331557afc9612496fb10c302371', '\x01fe754d94e56f8b2b95005350135cbd6d03a871b734c2d3652e52437839', '\x0151ba89d3a7adc42c08bdbbe0def4b61deb2d93530e5c5433feb0087e10e44cd0fec4a298', 1795504115383986177, NULL, '\xdda8d728d499a435', '\x012ff27f30c6157aea7737773759c531c8a3967937ce8569e9a8efb3ebed84e6514b1e7105a5416593885d64de5a21e14ebabab82eff30b18bebb91113', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb3bfb876578ea80322efde7742260ad3', '\xff4a8413b26f55592e46851c79855121', '\x55aea36344a595e7087eea06a79746c648f81c9bb138352a984a2294da52683c', '\x01d22fc50224bc2d8cb0340760c211f715c94f6318b0e2088c11deab585de2cad9dc72e86d', '\x015a2626d27b1cada788b56b5bfb50b47e0723d56d3a8a3fff41a7a05218', '\x01099f7a90ecdf2460d1c2382ade565787013be417ff454338aa15166ca7a271542a22b362', 1795504115422432378, NULL, '\xdda8d728d499a435', '\x0190bef9eee08c1fe6e176c050f8467a3d612549fd53c3492520893841b808cdceb3d649a7bda2c063cc83d184b2a4f682062d2a9db6d5eb994b910e0a', NULL, NULL, NULL, 56);


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


