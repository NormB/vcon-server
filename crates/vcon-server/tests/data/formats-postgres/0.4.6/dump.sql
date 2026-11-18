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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-17T04:52:49.557544319Z', 'cli', 'tenant_create', '\x98e424f728c6718e9c672426fb26703b11f1bba4da6d17cfef6a10cb9dd52a22', '\x01680856f0964f0b85ea86feea1de7dd1cfa378644f0e94801855c3be72fd0c814c48302f397826bab52751ba1e6ad6b15816815214c21e87c41913c10', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-17T04:52:51.993879679Z', 'cli', 'key_create', '\xd8bb85e5ee53a119211ec5e63131c927e4713e2ed4bf64373e8cea7247855b80', '\x012f90e75505b7fb514a14dda5e996e9a51d339c1cbb137bb0532b19018c51a0fa4545b3c7f1dd2082aa4130f9', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-17T04:52:54.325890601Z', 'cli', 'key_create', '\xe7b54921d734a3d902b7f59f55a108914b06eaa76974b0b00c18d22ac5695747', '\x0118c1bd8c07f28fa35010b8f8b3a2c53f2d0ef32b6732e63910f2ff62baa313ef929ee8f4cb0168456516a7db', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-17T04:52:56.632333197Z', 'cli', 'tenant_create', '\x20dae62e9327f4d27efbdfc454e826abd901113f3bd7f637fbb8a13a44d1c878', '\x0155c2210bebac57a1df3e23857527a9bf957be6f40cb6866e399331db2fa1f5b41ffc30fa710d4d88e49d1e1ba5c0291f9edee7459806b46eeecdf97c', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-17T04:52:58.775654755Z', 'cli', 'key_create', '\xcb77e052758b9f41400c5c1f490ba88dc7cb241b69abe0b69581c752040abbb8', '\x019156e0d12909b2c7751c114cb75625b6134ad33f6bbea2af569c2544e40ef064d612c8b3769216c9034bbc46', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-17T04:53:01.360016873Z', 'cli', 'key_create', '\x0e1693f2f569d4c94536fe5af677877ca0f635fd607eb6c7b4643102ff03bc34', '\x011312048c6dc79c454a461c026bc2c4d476ea10edf1dc7670f1e2741cfe74099e8526c5c5130b4cff5888b66f', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-17T04:53:04.849564477Z', 'cli', 'key_create', '\x8f339229c0449dcb7bd53ff8b377aebdc5b3e9ab63fb6ac6af65039429c9dc0c', '\x015761343e3b1778a74d7683810aa05659fd1830fd0b46d8ff67809e034d831b3df2b36931916a7fa7adccc25f', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-17T04:53:10.967260773Z', 'cli', 'user_add', '\x24967ded8071f47cd0c59c2c1f8849beb25afca1e26146b5744175706ebfb341', '\x01185fd398fa87fffc6880ea9b851fb4a571eb840da9455d517301163155fbe6d9de68c0286b5632747544fa2aedd8f2616362dfaa92ebb4c19610de80', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-17T04:53:16.270417891Z', 'cli', 'user_add', '\x640192bcc0de1cf88ee9e797b094b0fdbd31413eb8b3a8e6a6ccdd06698d5c8e', '\x0160c7f8f55020371cb3ddacd3d3b1d1369a3c9c433a60837c109151320aedd0d66a24f2a397f7099312823fa377f7ab6631c19e70d99baa9431e027e6', NULL, 201, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-17T04:53:19.68721787Z', 'cli', 'membership_grant', '\x9a517a67b42278c83229ab17d3dadd3d464e99d5a28c4831fd5d601a7ffde7b5', '\x01f3da268a763e722a4cb2d266e9505ac1740a4f235c4ebd1247cfca3ffeb79a5475844b964c3baba2ca0ec8c8fff7db3f9fc7ab31c0511b74f196a3a16a80ee97f4fcfc13b07d77e9af58f1a9b0c641b35ceeb7052e5bae9b09faf7019ec9150ee229579a', NULL, 200, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-17T04:53:23.339286525Z', 'cli', 'membership_grant', '\x3f675e62c01c341f6037918d663a6ab76e51c6e4b7c517e6361db71b7e00af65', '\x016aa204a50755a5804a22e4ff49a1b331d54ca68c854a070c483ebd90e3be17d82a5774e78b2f617f6efda45fcf60a90c1b0589a81cd5c1a39b8b544516feebb48117b2ac67e2e63453f403b994dc7a913f8137f33c00e577cb094ab5a1f176e5e4a7b7dd57', NULL, 200, 1, false, '\x1c83ac1c4a6046e8');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-17T04:53:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\x1c83ac1c4a6046e8');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x1c83ac1c4a6046e8', 1, '\x01624b861150f3d819a7aefe21149dd67d8fec6ccecbd464394f2dbe94ace311b69dfb2cc8a6c3b021febb52e9e4f968604968bd7dd83e2942f52b077f');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x89e1588bfc5445ba', '\xbf26f82a77cb38b279f2c7d7a2586375', '\xc4990cd0b3465bb03798cb18cfa0ee6151f909eca0a9e37c6be5d6bd914771c5', '\x017652631ac03bff10cd8be969483a20dbcd5618110be7e411c65c969cf4d02e4f', '\x01ff2f01ea7dccbb0583de5dc8e9a5a712f3f2a617e5a9a4ec5cbbafdbd7c1d883c0e21d1e88e76dd09e7d4a', 1794891174325264934, NULL, 1802667174000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x4e98e658e5348856', '\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x6242055e582d649d51ff0e0e9629d4e211e6f9e7a2d1e702f4b61612a9227e7f', '\x01a32921adeb7780f2c8eef8dc36295a872703bc7592b9e4931ee902f1f53eafe6', '\x018848562527bde7c75e844affe95997bbb07dc392db2a0ac23d04eb75de7213ebc4bb055a17171d7875f1f3', 1794891181359487772, NULL, 1802667181000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x1113bd8fbd86e71f', NULL, '\x47bb38eb64ca2509ceaaf095e70b311afce2ee8be0d127c6ab35fe7df6c7c519', '\x01656ec18d930ebcb18a5eb541bff1e0516075bc59d505197069d4f6ec6add572eb6', '\x01f14ed1dfdb9da6e3178f1f3a326844ae8c2e564da9c31be536d4e1cea1d0b6', 1794891184848363663, NULL, 1802667184000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x77a6dd7e93f0b38b', '\xbf26f82a77cb38b279f2c7d7a2586375', '\x2ca9a8d213a62a89b74b2352e2f5aeaf92c1f5f1f5ca87f7e012d80432406903', '\x0169f680e49889fdea32d71e4d55fa050cacd5f23f0f63d66a2075a97d63def94f177ac6155edb6af994162d6e10a92e4c801944c978949832ab', '\x01128047001c2bcab7c25b326ebeb12339c78d7b29e2ce937316b42289c2273ed7d439de5b8e29fac81c0d1724f7', 1794891171990971483, NULL, 1802667171000000000, 1794891180000000000);
INSERT INTO vcon.api_key VALUES ('\x3ae1d9d4e3d6281b', '\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x00081fbfcdb711ef6269dabd65e71a34c34b830615e7130b5fb74ea8f7907c41', '\x0175c37acb6736e97118bbbc8c4ff5244bd748db594b453c6dc1f10704341d9e894a94fc6a3da3246e0d1e5a126b8fc06c340632d31a3ea2d583', '\x0171cb33b4b214f2d5c6d8095b2f21c01d0fb47ba8202d06b9f836923887331af9478c658332eff421003b4e061f', 1794891178774949208, NULL, 1802667178000000000, 1794891180000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 2, '2026-11-17T04:52:51.998591208Z', 'cli', 'key_create', '\x62e4f761376e17048d5f12a59fd6d0c8fdcde5b74b53a2ff8e04d2dfa7a5e5ed', '\x01dcffd8ff0bd22c523ab923511ba0a5b0f4512df9748349f0a217fa30203c81f0228c3858f8a43160f27f954c', NULL, 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 4, '2026-11-17T04:52:54.335646714Z', 'cli', 'key_create', '\x093aee5c3aaac364840d2e48c5f0d7551a4f19b5edf9dda9ca3bf289a35d9f4f', '\x01585752e8b40f78d11387096dbdbccfce0e01f20f650f0c298500c3ab9a4cb5207e649e4e6c7ebb979809c6c1', NULL, 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 6, '2026-11-17T04:52:58.787324182Z', 'cli', 'key_create', '\x14f84c4c956016df25f57259de58fd52e1ed67c638f332db9f77aefa1ca44371', '\x0166f9e9cadba874d24ddc7421fd3b8a1b34e9fac87d48ac769c3eaea24c542ac73010b91e51e87115e26ba53b', NULL, 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 8, '2026-11-17T04:53:01.364210235Z', 'cli', 'key_create', '\xa1795daa3a8bb41bc3394e2aa94b0a6482ec4819aa1260b76abf8e3f67f1452b', '\x01728ece8bb2d28dfe8784a92c069b416ec6ae8e39bc3fe4c2b9265d29a9538c0d3fb9ac67f552fb3ce7919beb', NULL, 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 10, '2026-11-17T04:53:19.69263826Z', 'cli', 'membership_grant', '\x03367cc61329419a20918ec08d272eb9934065e605e7909f6d923da4709ebd35', '\x0173d7ae68c5c8dcb0fa03fd8724ab12353230544ec3d2d70ec0bbccd9dadbab02a56c75173141ba3e40e459b24a05bfe62835be0d25f724f0d7dd26164cebb015b02490a33cd1f50db88e18f113d551c5f5d10701eb00d8316b2f693ad011cd7e1d2fed11', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 12, '2026-11-17T04:53:23.344037628Z', 'cli', 'membership_grant', '\x80d57e5dedb8d8ca39a2f6aa9ef0eb4dd573d3c0cd6d787116c72559d33aa3f2', '\x0106ebf37281f95a85a971e80f03648946da3e72a991d45718a78126119ad709b75806c80af5fcb27b5f098de74a887d5728b6573664e5971802c4229ca97defeb55b8f503f3a698cced3847374660fca4a3215929c7270f66d4af6312d274959853577a6628', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 14, '2026-11-17T04:53:27.754375141Z', '77a6dd7e93f0b38b', 'create', '\xcd6a2e8f87d800e0d6bdd4887c702f56f8c6db5fd91268f3054d84ae46cc47fc', '\x01e36e4d6ae08ada08c2e6ee9b25bd14d2170ee205e2cc9168f88953ebcc40b906767fc30f6a85e317e9ba0d2723ba7d9d8886e2c9ed0517ac37bc9683e299ab1f', '1cc656669d2e8c654e9a3cbfec9744f2', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 16, '2026-11-17T04:53:27.843745604Z', '77a6dd7e93f0b38b', 'create', '\x3948846358dc2d6e7d692169771d3974905001530b95ca9ec083234b92d26ec9', '\x0143ba27fb547796be958e37fdf229d048860e713c88e28524d4f5d5325dbcf38a37a3bf7221458c20cae27128eda8d38047d477fdae975337ed2689d4973d9a4f', '01ff81867813c2419019ceea87646ffb', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 18, '2026-11-17T04:53:27.941300883Z', '77a6dd7e93f0b38b', 'create', '\x33abfb5f93897bd2b8e83936a8f8b0fac71a5290ac5e9e503f7271be0d08dce9', '\x01e714e8911c2b886b392ef6b2737d4b8aeedc2f067f7d72c2e4f8f8c28805ace52efc13109aa1810b08d7770795afcebd947837179d699046318bf862450e153a', '4cb778b948cfc6b601c91949addf3a05', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 20, '2026-11-17T04:53:27.997473647Z', '77a6dd7e93f0b38b', 'create', '\x5f26cd1ee080d5a0d1934910238acc88e1869b23d05379e0c620c6da75d3eca7', '\x010a82815febd88420dd0af2bf04781f1df1a248f4191adc125adcb509a5b53714053731abfce4b6bc599d09fd0a228610b57a1f3da13ad6a9b66d5e31a9562991', 'e2dbe23b5c8210ac6247bb26bf3d8d78', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 22, '2026-11-17T04:53:28.028569003Z', '77a6dd7e93f0b38b', 'create', '\x60ff166b7012c069bc9fd66a8d4d2dcf6e3fd65b2e39f7ae2f6f35b37b4355dd', '\x017f49fd0c40b37e36f05aa620ea5f9fb65670747d8c6f0a927decb4846aa55c4d97bacbcf67b875c764ec01ac310e7152be51f5b7cc8802ac0d72057741767f2e', '52c6b2c07319a83ab011e4a24cefa6c7', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 24, '2026-11-17T04:53:28.074161787Z', '77a6dd7e93f0b38b', 'create', '\x5fa432261d9daf6aa70e374b3845b257e4a79bcf4306c057120ae37d278c984b', '\x0131fa299992c028dbc1bbba045bcdb466eca566bf52a51073485aeb6b2f00be23b7694e8483d96c24b7d36f9ea4eb8569bb95ca866753d22ac1ea8097330667bc', 'c8f6ee689273cb0a608c0f199526dc61', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 26, '2026-11-17T04:53:28.088153845Z', '77a6dd7e93f0b38b', 'create', '\xba430a9b1ea2bc6988a20721b0d659a5148ced9967c72caf3e4611cf3d34e34b', '\x01e7367929cc3b0c77c10968d0c496673a2f24bfd9f2a29357097bd794b2aa724574acb164c502b906b605e8ee2eeaa27d2c6e703cad0b1bd6594e53c2800163a6', '6aa82c46b270f6250e19b61b2954f507', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 28, '2026-11-17T04:53:28.102410078Z', '77a6dd7e93f0b38b', 'create', '\xe4e4b067d57f315028da89db5ab9bb934ded30096931efc4180e05be37d96a71', '\x016f4774dce1b47316c255e291b07b5ee3c60f579647f66b7090186b9049ce1f70da26e933db9e7059ca3566bcce9728f2fb720820eebd97baad78ca2dbec38d4d', 'ac70dbd2ad890bb04d4bd3d60e0d4865', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 30, '2026-11-17T04:53:28.119210803Z', '77a6dd7e93f0b38b', 'create', '\x6f711b5f172c3c2dd8ef85898949f0713e497e9735577244d9079acc82f922f2', '\x0130afe1d91350a52bf4e20556c15f8ec33fabd5b43fcba128bb0aac07937220bc7386c6cd75ed71bb13b5f51d26c1b4b32248e258594cbf236cc77b78b89892fc', 'd98e6d1d1c079164b855379a8670f583', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 32, '2026-11-17T04:53:28.13550424Z', '77a6dd7e93f0b38b', 'create', '\x3a5a2349694eb56efab1d16608c285597d5fba9140be89d75d3748734f65ea8a', '\x01710146d898485fb256c417a77d0cece9844004c5437cb84a2f138841c584e33a545b5df4c9e2df46fc44c892e8a6deb84cfa7a5df32ab3846779d677f9c41b15', 'ec3b204b80c92e700c92707d459ba5ee', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 34, '2026-11-17T04:53:28.160167651Z', '77a6dd7e93f0b38b', 'replace', '\xcd6a2e8f87d800e0d6bdd4887c702f56f8c6db5fd91268f3054d84ae46cc47fc', '\x01cf9db6e622e3d598957993787934b4a146fa89447aa7e0b3ee44a9f9375dd43b69980ae76a9366757ce377ca5e95b531db32538ec57617712b5796965563aa1d', 'b0407c188382e10ec32f6d983dcc1439', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 36, '2026-11-17T04:53:28.188516238Z', '77a6dd7e93f0b38b', 'replace', '\x3948846358dc2d6e7d692169771d3974905001530b95ca9ec083234b92d26ec9', '\x012e30d812f9555e874b5bea666095a2aeda12a8e59bda126f0b282ca8529b69d62cfbbfe5eae90f7666c567104846e9668988747fae48d55621c1905f792477d4', '6064fcbf60fa1a26253016bb9ff6f581', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 38, '2026-11-17T04:53:28.23240218Z', '77a6dd7e93f0b38b', 'delete', '\x33abfb5f93897bd2b8e83936a8f8b0fac71a5290ac5e9e503f7271be0d08dce9', '\x017cbfc7b08726703f0f2e0438af25786d490b19a8a6d0802263de933feeb3a7abe30c4afcc79225746259ffab75c2f16f364779236986c1a061d77807e05c842e', NULL, 204, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 40, '2026-11-17T04:53:28.249135358Z', '77a6dd7e93f0b38b', 'delete', '\x5f26cd1ee080d5a0d1934910238acc88e1869b23d05379e0c620c6da75d3eca7', '\x01abfb074c68c64f43390c16ff30bbe358500a966f60ddd5aff99a73fda48298656f02a9e3698b14fc010a8f3735d70508063eb362a2068ff812fadd948aef9f0a', NULL, 204, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 42, '2026-11-17T04:53:28.302153401Z', '77a6dd7e93f0b38b', 'hold', '\x60ff166b7012c069bc9fd66a8d4d2dcf6e3fd65b2e39f7ae2f6f35b37b4355dd', '\x0176bb7ecfc45683bf5dd474f3587d053f01a98a183381de8d76ceeca6ba8b16dd03b9cf2b41c02bacdfc01caece3637674d1e86acd7788a56799b1445365044e8', NULL, 204, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 44, '2026-11-17T04:53:28.343398572Z', '77a6dd7e93f0b38b', 'delete', '\x60ff166b7012c069bc9fd66a8d4d2dcf6e3fd65b2e39f7ae2f6f35b37b4355dd', '\x01be6061ae33c9a34ece3f43dc5b523f9e4124e6d5cec2dc2c3595bb87b4d1f10045d70db4afca91fc5e096740fb1b91a3de1b06f1367f8c5d0bba1336fce7e94c', NULL, 204, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 46, '2026-11-17T04:53:28.450600101Z', '77a6dd7e93f0b38b', 'hold', '\x5fa432261d9daf6aa70e374b3845b257e4a79bcf4306c057120ae37d278c984b', '\x01d1609caf57a4a7181be1f36e66823b87e5ae654f5e95fd96556a1cdeea79a9f027d846020356e05ed0d96bc9f1c42433dbc9d50644ea81ab38f22c0fe38bb25c', NULL, 204, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 48, '2026-11-17T04:53:28.544010425Z', '3ae1d9d4e3d6281b', 'create', '\x89dfbd797a72ac5d5da19975007de3d213e5dd469e0ec854a561a2846765f802', '\x01f674f254a0ae0bba25d6246e678156c15778b7ed59cd9905b9b904595c0775a7e9677b958fba4a3aa1bb726762475a36ba0bb1149ffb34f9a7a304dce81374e2', 'cf6b950002b3bbea92e8f883901c7326', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 50, '2026-11-17T04:53:28.634991554Z', '3ae1d9d4e3d6281b', 'create', '\x4e0b9a99b2bb1cb24a2184a69d2b3dbffd4bc2c2c3f2568d1b2c07548db84805', '\x01edd727a7b4af960a1cf3d3e8161d4f9d43c2d1a28f513d12ba1019e02a52f929377b44c9a3e907b2e9e90fc5d327eafe824763b0da2f484d0737732e5d702d9b', '4370cc5686d84091a0635948fe0e006b', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 52, '2026-11-17T04:53:28.660767744Z', '3ae1d9d4e3d6281b', 'create', '\xa472ace27570c8c9289ff9a8a19d530f1903052b7592b2961310b254fcbcfd3a', '\x0143f29d3a0a2c9bd930e9a2be941dee5199f8de33fd934a2cc1bfbd87df13cafeb1bd3f2ac2bae12a22aa8501c04078f86ff122b3fc3582aa6f2b1b193044afec', 'c5cb1e428a88d6c2573e9349e8ae7862', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 54, '2026-11-17T04:53:28.687156932Z', '3ae1d9d4e3d6281b', 'create', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x0108cef2b2f959a82bc301c7c2deb5904ed15083b208bd55af9238e6e80b12b9624d73aa5ca22a92e37be794342601885c8207a8bd7fb6eea7acc3f252e60a3e28', 'd0c91914a20d29efe5939fecb531da80', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 56, '2026-11-17T04:53:28.710508871Z', '3ae1d9d4e3d6281b', 'create', '\xfcea3c859c8decf3f4dd4947036d794ccac7f71cce1f18fab4ad84bb52597cb5', '\x01fe76498b3a781a7c36f5fc0c68287d2dcc1395cb498bedf02ee479644b8d6c3b62c75234c36ba0ae544dc8f61a93267a9423b83f58ebd6af2d2da0946893449a', '36fc4f049f519fab944463013cdadf14', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 58, '2026-11-17T04:53:28.737982384Z', '3ae1d9d4e3d6281b', 'create', '\x6262279bc3dc8931897251711b4bbeb063ba9942f698192ac4d65813d96bcdf3', '\x010c339f0931ec1fc65f9547bfbec854b42de41135c6ba0ebf7936723c4b0de912807ae277a6876e3d880462edf327cec3edd7cce980135cf14fa37c2f3d0981db', '7aa16790a19930c29ad9bcc5ef8933b0', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 60, '2026-11-17T04:53:28.754387071Z', '3ae1d9d4e3d6281b', 'create', '\x9af076b432811ea2e8f76e9d862671bee5b9b55bcc50c9cdb6fa5069025847b8', '\x01b4aa653838e6e3a2aa1110bba266c73ca4e8f209d51cd9113fd531f064136015b6da0367550fe49560a5185ecbe4c7e414d205ce52538e8bab026314c49a695f', 'b9d82401f7910fdeb30b4144d940ff0f', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 62, '2026-11-17T04:53:28.788586354Z', '3ae1d9d4e3d6281b', 'create', '\x1ddc18203b145092125aaf87db8fa7d06263a4f6a3af443157e90c3fab3fdb89', '\x01a4b67272b679ad8cd9c1c7d2ed1b0076651c451ae97558f5b4c9bea52bd05a7cf06740c698172af37e23481ac90b31b2f253494229cdf7bd6fcd700427fb277f', '6a0490e808757e00d633522bb402a8d6', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 64, '2026-11-17T04:53:28.805841958Z', '3ae1d9d4e3d6281b', 'create', '\x9af1a7e461ff416df6859d74fc9f0f464b8c8249dd9dfb2b2ac4e7464297a4be', '\x01467238897fa5e2a7471962c3ae6ed81b5bc054d4b1ec14e8de774e397f10d46e34833f941f04286470e2c5ffe5645644989a2ef73ff8553e52a043cb65945e51', '9b9c874b0736932b74da45debce5f5d6', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 66, '2026-11-17T04:53:28.828660045Z', '3ae1d9d4e3d6281b', 'create', '\x90eeea35a136ffb2aaa7fd930881513879c2f06b99c366fdbb6e88cd71ac278d', '\x01d1e053aa3355d43465d4e16bc909271b1e7e4386b3b4be60689b70b68d7808e4c54c3783e99b8966b1f14d7d972651f2de5a33e253cfd3eab8521770cd167b7b', '732d08b988225eefb74588e77ca3c464', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 68, '2026-11-17T04:53:28.874385653Z', '3ae1d9d4e3d6281b', 'replace', '\x89dfbd797a72ac5d5da19975007de3d213e5dd469e0ec854a561a2846765f802', '\x0107d8c23fd65becfa22ab8f006e035ae7ce6582c7d43937ae9360d663d980483dc9e3d7e690296620a9fad4e57c5ad79f8ed9fe719dcf7bea6e8981b8fc745110', 'fcf14e1bdaf8178558478c7c0bff4a6c', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 70, '2026-11-17T04:53:28.883219858Z', '3ae1d9d4e3d6281b', 'delete', '\x4e0b9a99b2bb1cb24a2184a69d2b3dbffd4bc2c2c3f2568d1b2c07548db84805', '\x01fd60c73c3321f907273fa59d4db0571c55ca61972e9c6666bc264f382bb257f8715c06bc3413a410c597fd62296a9769d12489e00f03c8690d73e48cef2ee313', NULL, 204, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 72, '2026-11-17T04:53:28.889897146Z', '3ae1d9d4e3d6281b', 'purge', '\x4e0b9a99b2bb1cb24a2184a69d2b3dbffd4bc2c2c3f2568d1b2c07548db84805', '\x0185a6617423b079760aa229b335294333301751be8d179e60a8f6cae6c36c1320cdd4a62e2ad621f3d4f4e16124d62f5f9027fb21b5c0ee7edcac6f947a6ebcc7', '4370cc5686d84091a0635948fe0e006b', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 74, '2026-11-17T04:53:28.933870706Z', '3ae1d9d4e3d6281b', 'hold', '\xa472ace27570c8c9289ff9a8a19d530f1903052b7592b2961310b254fcbcfd3a', '\x01ad49294b7bdc34d68c0845a28796d2b8d178f49b6f03530cdb8adafa319b681ecf42863dc2b2bd9f59eefeb245302b032b85746a966a76b0a2f2856fb8ef2469', NULL, 204, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 76, '2026-11-17T04:53:28.946991105Z', '3ae1d9d4e3d6281b', 'delete', '\xa472ace27570c8c9289ff9a8a19d530f1903052b7592b2961310b254fcbcfd3a', '\x01439ae9f4b80382f2509c4b7b2b2eed45bdb1772c185f93f89b8ea8318a1e277157e05b4bf94619b5638df46089249ae3a86698b768106125538cf8223c7e77de', NULL, 204, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 78, '2026-11-17T04:53:28.957981357Z', '3ae1d9d4e3d6281b', 'hold', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x0111d65573328b3d6e7e6a0a178bf6f4c9b19ae7ef34109b6bc62465e2c8e664e2396da6ab4e1320b0ab8c326b1bef8fca51c1761c97305fc6081da66db0178248', NULL, 204, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 80, '2026-11-17T04:53:28.980634488Z', '3ae1d9d4e3d6281b', 'replace', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x01c2c814e73f68e594923ea01c4411b847a21be87b90574eafbb98d8b2f2e6fc80276eab579ca45e8ece6383747a82759fcd944a5d8c5489a9622f38b8fbb80a36', '64ee4cf326c2c19a75012d222b5a7fe2', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 82, '2026-11-17T04:53:32.561148306Z', 'sweep', 'purge', '\x89dfbd797a72ac5d5da19975007de3d213e5dd469e0ec854a561a2846765f802', '\x011777182b6eb36957c10b8ea666c15b6b656a832b53eff54ed3cbb5098c1279438a511fc455209c9f6969a6e75cb10b3bb461e7ee83232d05d950c33de93f89af', 'cf6b950002b3bbea92e8f883901c7326', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 84, '2026-11-17T04:53:32.710365684Z', '77a6dd7e93f0b38b', 'legal_versions', '\xcd6a2e8f87d800e0d6bdd4887c702f56f8c6db5fd91268f3054d84ae46cc47fc', '\x0199f89526302f24d4ecc5ef8f516b5d57379c802daeb1d6b5024e70089a8c44b56146160cbec674fb749951f3a27712184497a612c6c52ebc5ca296bc4c6a841d', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 86, '2026-11-17T04:53:32.722681175Z', '77a6dd7e93f0b38b', 'legal_read', '\xcd6a2e8f87d800e0d6bdd4887c702f56f8c6db5fd91268f3054d84ae46cc47fc', '\x01abb0e4b8c0c45b29c003e8e464de086ed0eebb07c0367d7515ff33424a70e8be8bde92a9cd7fd37acf7b81d3817886db15c4405ca03461889296ff7dd7b5ef97', '1cc656669d2e8c654e9a3cbfec9744f2', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 88, '2026-11-17T04:53:32.738143464Z', '77a6dd7e93f0b38b', 'legal_read', '\xcd6a2e8f87d800e0d6bdd4887c702f56f8c6db5fd91268f3054d84ae46cc47fc', '\x017bcbb92fa20e7c37434658841c0726ef4ff6ed2e4c312f32039b42ae4310d1b18c7a1d82ebbbe2df3ae45c15a68f6b4d0dbf15ffb1f760cd2cbe607c983c6916', 'b0407c188382e10ec32f6d983dcc1439', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 90, '2026-11-17T04:53:32.771191994Z', '77a6dd7e93f0b38b', 'legal_versions', '\x3948846358dc2d6e7d692169771d3974905001530b95ca9ec083234b92d26ec9', '\x014c6b754771896114cb653e9343df342ab3965f38faa7227ee1746370cd919901e48416dda76d06338c4bf2312e97d2b61f66c3ca7bc5ab72c848daa082303cdf', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 92, '2026-11-17T04:53:32.792707969Z', '77a6dd7e93f0b38b', 'legal_read', '\x3948846358dc2d6e7d692169771d3974905001530b95ca9ec083234b92d26ec9', '\x01334cf81d477902fd015dc069badb1fab904f7aa48b5ab69574a74ab71bd4464cefd67d4c10d565fbfaa043185cfac088d0b479cd0cbf94e11e55e2eff6f59251', '01ff81867813c2419019ceea87646ffb', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 94, '2026-11-17T04:53:32.813956341Z', '77a6dd7e93f0b38b', 'legal_read', '\x3948846358dc2d6e7d692169771d3974905001530b95ca9ec083234b92d26ec9', '\x01aef9c87905636f65583c7d0f03d674d5c8de19510d68689698a0ca9c854b2a608a9fa076af775c42602a733bfa9cbd6b3c8d4906b14f79b88d64190b21453ea9', '6064fcbf60fa1a26253016bb9ff6f581', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 96, '2026-11-17T04:53:32.822374037Z', '77a6dd7e93f0b38b', 'legal_versions', '\x33abfb5f93897bd2b8e83936a8f8b0fac71a5290ac5e9e503f7271be0d08dce9', '\x01ea158d013b43e32b3c8c4566645c04da95f2d34ca48070c4398dbebb2392dd5b55949d70a6f157900dfa44e3b03a19545a3bcd6ac0149b41f1fe603e5e2a6269', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 98, '2026-11-17T04:53:32.832349964Z', '77a6dd7e93f0b38b', 'legal_read', '\x33abfb5f93897bd2b8e83936a8f8b0fac71a5290ac5e9e503f7271be0d08dce9', '\x010c9ee6c2bcee84d16a0e424a83b075d1bbd93a4c9d9be971cbfa4783a6e964915cfdf93b5498ceaba80550e5438d0dd8a3dc5a6f775cd360563b38046719107e', '4cb778b948cfc6b601c91949addf3a05', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 100, '2026-11-17T04:53:32.8453814Z', '77a6dd7e93f0b38b', 'legal_versions', '\x5f26cd1ee080d5a0d1934910238acc88e1869b23d05379e0c620c6da75d3eca7', '\x0164fd4261de5f40c55db07c7fb94ed29c06391d952714eaf1a5ac05fdbffdbac30712c47df2825cb901a4b1d3871ac45951d1e558b1bf4b7b66b3a22e93f82384', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 102, '2026-11-17T04:53:32.862534346Z', '77a6dd7e93f0b38b', 'legal_read', '\x5f26cd1ee080d5a0d1934910238acc88e1869b23d05379e0c620c6da75d3eca7', '\x014a1dbd64d11a0e5490a18212bf58a2a39abeeab74c08b7f84fe36564731f4efc706b4ffeaad3321be90e43aa3a3b84230c75338066e4bf33162e0ca4c78b1e4f', 'e2dbe23b5c8210ac6247bb26bf3d8d78', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 104, '2026-11-17T04:53:32.877495023Z', '77a6dd7e93f0b38b', 'legal_versions', '\x60ff166b7012c069bc9fd66a8d4d2dcf6e3fd65b2e39f7ae2f6f35b37b4355dd', '\x01665da9bda7ab1ec6da4f6ea05751dbe06b19e22358b3a65641e253e590936dcbf9762acf2181d3d8c91afe1fa8cef0650bf0754f599b86d1977f77d497b107be', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 106, '2026-11-17T04:53:32.899540081Z', '77a6dd7e93f0b38b', 'legal_read', '\x60ff166b7012c069bc9fd66a8d4d2dcf6e3fd65b2e39f7ae2f6f35b37b4355dd', '\x01e033a6d80470bfeb770a2569fca98cf48012fd75c6258f20a1eb7f3decbb5f6404c5da5f89b68705f5b0831846469923df3e222c7f51dbc6e45ee4abfb3a6870', '52c6b2c07319a83ab011e4a24cefa6c7', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 108, '2026-11-17T04:53:32.906476702Z', '77a6dd7e93f0b38b', 'legal_versions', '\x5fa432261d9daf6aa70e374b3845b257e4a79bcf4306c057120ae37d278c984b', '\x0175938f1055df6d25673130cb7af270745f2011211f653d8f78337ba9ec9a1cf4befe6348b6071c02345b4c4b7dbe4f9fd5996b5c1bb480f48e65500f7bf9ef15', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 110, '2026-11-17T04:53:32.914479323Z', '77a6dd7e93f0b38b', 'legal_read', '\x5fa432261d9daf6aa70e374b3845b257e4a79bcf4306c057120ae37d278c984b', '\x0118afdcbed5455bb1d405d9b1c0e49cb5349104f87ae72db2d0c9b261873efe57338b7cd48f27a942484a52ced38d8d93aae853429279b99bb82998b13a922de2', 'c8f6ee689273cb0a608c0f199526dc61', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 112, '2026-11-17T04:53:32.921806027Z', '77a6dd7e93f0b38b', 'legal_versions', '\xba430a9b1ea2bc6988a20721b0d659a5148ced9967c72caf3e4611cf3d34e34b', '\x011d7f8c54d71794f8a960170ed1ef034c75820197f95a2d204793491f05ec67a1004726036444103fabf17b49ee83e6f34bf5484f34d4496b6b66b098080d48df', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 114, '2026-11-17T04:53:32.952230345Z', '77a6dd7e93f0b38b', 'legal_read', '\xba430a9b1ea2bc6988a20721b0d659a5148ced9967c72caf3e4611cf3d34e34b', '\x010887dfac6f625670363a7fad9f40220746f5412053d05ce2f231e1c12110be9ae77b386a431eac9cd716e473831b5fea3b058961b247e4ea58af914e5b050230', '6aa82c46b270f6250e19b61b2954f507', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 116, '2026-11-17T04:53:32.963411346Z', '77a6dd7e93f0b38b', 'legal_versions', '\xe4e4b067d57f315028da89db5ab9bb934ded30096931efc4180e05be37d96a71', '\x0101de656da8f62f27590d4baf6be95971c9ed01016c023eff81a127926fef42029a3cccb3ff9155c81e831683dcb27d19ccfe21a7f61d99e9e0e2bc2b73d68c47', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 118, '2026-11-17T04:53:32.971718958Z', '77a6dd7e93f0b38b', 'legal_read', '\xe4e4b067d57f315028da89db5ab9bb934ded30096931efc4180e05be37d96a71', '\x018a33f3c03216017a9d67687c729a23cb5e08a663b54a283a5ce9d68384f9591b95d4c40ae1da79a695408e5f33c16bc59edb5c0a2ec7ef2e9e346b0184601985', 'ac70dbd2ad890bb04d4bd3d60e0d4865', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 120, '2026-11-17T04:53:32.978392791Z', '77a6dd7e93f0b38b', 'legal_versions', '\x6f711b5f172c3c2dd8ef85898949f0713e497e9735577244d9079acc82f922f2', '\x01c9a59104e8476b1b408de4b766486bf4aed3ecbf8f5de615f0efbe81802bf79d2143a9b76d19b349b9dabb6229e2726a32ce3fcb95578eea242760a1b91b19dd', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 122, '2026-11-17T04:53:32.994253478Z', '77a6dd7e93f0b38b', 'legal_read', '\x6f711b5f172c3c2dd8ef85898949f0713e497e9735577244d9079acc82f922f2', '\x013f7a7cd09bd15bf7306898b58d9a308c6df9679c901e0d86a096d22db04b980c7d3fc09b6f947432a5fe0945f911f6ddda59d509b3d1a4b195e8eb019817ef5f', 'd98e6d1d1c079164b855379a8670f583', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 124, '2026-11-17T04:53:33.001294118Z', '77a6dd7e93f0b38b', 'legal_versions', '\x3a5a2349694eb56efab1d16608c285597d5fba9140be89d75d3748734f65ea8a', '\x015b27e7c004c953c21dc597f753eac80eb4f1b13bd921b4d8688776f5a1ec109b20fd4e826d66dc684c16967d61ef49136f77d63c0a3daca6241ea00f42d9b288', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 126, '2026-11-17T04:53:33.009258508Z', '77a6dd7e93f0b38b', 'legal_read', '\x3a5a2349694eb56efab1d16608c285597d5fba9140be89d75d3748734f65ea8a', '\x0167f836a68a7cc3b23f90a31bef758304f932f84cc6c544153410351a9547f2001b5a4c8fa806a5622dc7ad98be0b25def354acdebec379b6d4f1faa3ee8eb9c7', 'ec3b204b80c92e700c92707d459ba5ee', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 128, '2026-11-17T04:53:33.020719018Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x89dfbd797a72ac5d5da19975007de3d213e5dd469e0ec854a561a2846765f802', '\x01c93a7f16e60da44562f5acd986bf9c1fd5904b485618c6a062326ca2278575a457f810c53fb38d48e35eb99bc889d48c394a51f85db85a6bf87d9725110898cc', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 130, '2026-11-17T04:53:33.048080587Z', '3ae1d9d4e3d6281b', 'legal_read', '\x89dfbd797a72ac5d5da19975007de3d213e5dd469e0ec854a561a2846765f802', '\x014b5ce1ec8f1f6478c62683b10e320d7dab51d1903baf4c756ca53871bb120ec93ba1eca8bbd2b61dcecb2da45430e15ad45bdb65960ee2d9309c2dfae2b9c937', 'fcf14e1bdaf8178558478c7c0bff4a6c', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 132, '2026-11-17T04:53:33.060249495Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x4e0b9a99b2bb1cb24a2184a69d2b3dbffd4bc2c2c3f2568d1b2c07548db84805', '\x01d9e0e612658c90c8ceef2325dc57c3ff60db93fa50ad115d69e6d4debc6212c058dbe57e2b7d16f4aa1fce7d9c576ec1c598f7cbe359fdb8d7b159f2d89d53e5', NULL, 410, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 134, '2026-11-17T04:53:33.070138024Z', '3ae1d9d4e3d6281b', 'legal_versions', '\xa472ace27570c8c9289ff9a8a19d530f1903052b7592b2961310b254fcbcfd3a', '\x01b972b58e62860b17b02f03c878e3b2728c81b7967f5b625ba81a9507e200dff768dca16d3a08e3efc1e2b162e249f888eee257888c1b1146fedbea8a4e5922ed', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 136, '2026-11-17T04:53:33.078024452Z', '3ae1d9d4e3d6281b', 'legal_read', '\xa472ace27570c8c9289ff9a8a19d530f1903052b7592b2961310b254fcbcfd3a', '\x010331f24d6c70a529e7e0d821cfb48a1f0902fab2f412c5d224516f97ee4c0bb65cf2a3beb96d60b4864d41b9a9c155bc3abab7a8d6e8a7fbb80a9d8e4f36ab92', 'c5cb1e428a88d6c2573e9349e8ae7862', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 138, '2026-11-17T04:53:33.083080544Z', '3ae1d9d4e3d6281b', 'legal_versions', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x0189f0538166939e8e088f742627b75ae1b0d45686c61078c5e67650a01f311e40c006bbd22215d65730239d64299b0cd84154e182bb9765d5991e418fa683ff54', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 140, '2026-11-17T04:53:33.094067528Z', '3ae1d9d4e3d6281b', 'legal_read', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x01cce5b148fcbd16e007e69b5ff4ce76a94cf3cb30ab4eed3201af054cf7f06ce7e7171ea41c529e2e6798ecc934a9583d8072086c2602b34ff4751f17375463a6', 'd0c91914a20d29efe5939fecb531da80', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 142, '2026-11-17T04:53:33.108032817Z', '3ae1d9d4e3d6281b', 'legal_read', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x0121ea14a1ab407946c7ab4a1a49be21b458c8f9dcba99936515a9b6f5d935047837fe03a8aca4a3105cefe30d195fe01d3a307e6a95bcff02d8e5fadaa82de110', '64ee4cf326c2c19a75012d222b5a7fe2', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 144, '2026-11-17T04:53:33.122626041Z', '3ae1d9d4e3d6281b', 'legal_versions', '\xfcea3c859c8decf3f4dd4947036d794ccac7f71cce1f18fab4ad84bb52597cb5', '\x014d5524ebf4257606559035e983687d9cfd5612f57b2171872a4a0b3f23c79ca8aede5b71451795eb9422fef2a996546aeb00d4926ce686f2b6777e6eea68db6d', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 146, '2026-11-17T04:53:33.178092307Z', '3ae1d9d4e3d6281b', 'legal_read', '\xfcea3c859c8decf3f4dd4947036d794ccac7f71cce1f18fab4ad84bb52597cb5', '\x012205253f7e3b6b8e4af02c310159bda71705a5e790b30f0c73ee300a515c8c47ce5d45a3846f40e2e6ef1decc7f665f937c0989a274a4929cf91417a2bd1eae5', '36fc4f049f519fab944463013cdadf14', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 148, '2026-11-17T04:53:33.188158854Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x6262279bc3dc8931897251711b4bbeb063ba9942f698192ac4d65813d96bcdf3', '\x01071c50f8f0c4b820344e0bb7512b8e0016e3c121a7601c4c630e2c1cc7488711098be1836767a9b1b8c3aef2256b8a9be957efd7f48813c8626df6435efe3450', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 150, '2026-11-17T04:53:33.264590383Z', '3ae1d9d4e3d6281b', 'legal_read', '\x6262279bc3dc8931897251711b4bbeb063ba9942f698192ac4d65813d96bcdf3', '\x01165ef80b504dcbeeba96517eea26c766977a250b325c3bc49bf442efd6570304a09d19af85e9013fc011235e36a10aa1e841059818f28e732cf980aed9736a5e', '7aa16790a19930c29ad9bcc5ef8933b0', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 152, '2026-11-17T04:53:33.312251822Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x9af076b432811ea2e8f76e9d862671bee5b9b55bcc50c9cdb6fa5069025847b8', '\x0104b6a41c09f31835350a50a6d1d2f741cb48fe055d64cbd668547f630074078de0118008615fc0c766a4d08f18ee8dbdeef7ac7e94d0710e9c35ae6647045a5b', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 154, '2026-11-17T04:53:33.331993778Z', '3ae1d9d4e3d6281b', 'legal_read', '\x9af076b432811ea2e8f76e9d862671bee5b9b55bcc50c9cdb6fa5069025847b8', '\x010b51892685881fa49f0cf47034d2da04570ce43cd4fb1239b446174ab2b6dae2299dcc094b226b8dc37e5f65b2b97244e674368c2f94703c68c0cb56350339dd', 'b9d82401f7910fdeb30b4144d940ff0f', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 156, '2026-11-17T04:53:33.339693427Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x1ddc18203b145092125aaf87db8fa7d06263a4f6a3af443157e90c3fab3fdb89', '\x01163bcf3020ff2579d171a170838ad8915ff8e4d471c199d5ca69f65c4ba9d347ae461fa6707d0b3ef955e44752cfaa3f147c3cadcf09924fac1368e1d5f873a1', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 158, '2026-11-17T04:53:33.352192984Z', '3ae1d9d4e3d6281b', 'legal_read', '\x1ddc18203b145092125aaf87db8fa7d06263a4f6a3af443157e90c3fab3fdb89', '\x0182f36747b7d3d16445240af30d161770d92e6f6ffd8d4051c30d785456082b6243f4e1689a240f2c5bc82423d3498753aab8c67d34ceb110e3a209658322df4d', '6a0490e808757e00d633522bb402a8d6', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 160, '2026-11-17T04:53:33.35828129Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x9af1a7e461ff416df6859d74fc9f0f464b8c8249dd9dfb2b2ac4e7464297a4be', '\x01688cd7bb24fe697cff3222a0c2daa4f1d828b86bda1ee76047b4665242beaba28562899342a83273f61b04814edabdfccc8c0479343aca19123c71545b1d98d7', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 162, '2026-11-17T04:53:33.365784272Z', '3ae1d9d4e3d6281b', 'legal_read', '\x9af1a7e461ff416df6859d74fc9f0f464b8c8249dd9dfb2b2ac4e7464297a4be', '\x018af55aa726a8221cb4505b5bf14bb6f800c4102e382be53b78e5c3c100d3146ca1a8e38b977e26399f3dc232118e9151debc5fe86fc3734133ea83cfc94e10f6', '9b9c874b0736932b74da45debce5f5d6', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 164, '2026-11-17T04:53:33.371446837Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x90eeea35a136ffb2aaa7fd930881513879c2f06b99c366fdbb6e88cd71ac278d', '\x01cfe95f910e76a9ad4a9c19e2b407fb1247b33e756d246e26c1bf8225a7c84fec7dc761b9f4da7fbc1085d652bc2c6a6acc515d838202de51792791c5580873d8', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 166, '2026-11-17T04:53:33.3817872Z', '3ae1d9d4e3d6281b', 'legal_read', '\x90eeea35a136ffb2aaa7fd930881513879c2f06b99c366fdbb6e88cd71ac278d', '\x01c55343d98ac324d4f741c2d2b217ff1b63b54edc076ff50fa985d752e57c5b115596c0a5820d47604b410f7a1cb35c0d6ccba249f7abf8ca32c55c5bb9d65b5d', '732d08b988225eefb74588e77ca3c464', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 168, '2026-11-17T04:53:43.26010777Z', '77a6dd7e93f0b38b', 'create', '\xced1b4e0c77bd70f4f2ac2e2114a750ccbd1208c8f2181a8ca168082414af8c8', '\x01b6f74705d2e53e6e95d335ecb2f8d765bc960a955fdf5670752d7fd01c134bc497047e577fbbc9b47921026f9fd52cf88a03edcdcb43340f49e1ebd0062109eb', 'f94381743ec355e2c5c55f61d93c5569', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 170, '2026-11-17T04:53:43.335878682Z', '77a6dd7e93f0b38b', 'create', '\x1aa3d5e5ef2f19fb1f6068453f911cd890c80a94564e3391b7a131390741865a', '\x015e9749fd76e12e18052019b8ff3dc5e77345b164c48463272fb7b6225fa7b3e6b52e6b5fded096f75ca4ab4f5725ef9448b7aee6f1092cebff27d9a93afbf0a1', 'e936ff4b3f46ccbe515242061d17a31c', 201, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 172, '2026-11-17T04:53:43.397131122Z', '3ae1d9d4e3d6281b', 'create', '\xde37cfb31b51475fe1ff4d0070db69253326e2b54d6d7e4f941a92e0d210735e', '\x010f6561e9df8c4aad76c03e1b825c4e50a163002cc9f50c7075900bf31849bb745bc35b8202a279722c6fd66c8d32f7f4c1b33452a99e6e6df5c6f12a5e885d0d', 'b8498ee8c6b06929ab963191c38a1947', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 174, '2026-11-17T04:53:43.421941152Z', '3ae1d9d4e3d6281b', 'create', '\x703d2e1ce11d670aa163dee6f5492bba9ed43f97c8677dcd31b763930142ebf1', '\x01097579322ca28f43f4ddfde0ddee725fa1a673acd218b06fb10fff378873fc3fc91e603cdebb89357fa1917387947f07fa1a919f6f3a0ed8de9506c98494ad76', '802df3e64de4c65d67594a47fec84fa6', 201, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 176, '2026-11-17T04:53:43.430965441Z', '77a6dd7e93f0b38b', 'legal_versions', '\xcd6a2e8f87d800e0d6bdd4887c702f56f8c6db5fd91268f3054d84ae46cc47fc', '\x01775adaddb043b9f3d4160ee891eb05e5638490f40f2a73259a375d0aa68938ef1cb2a4dc57e2d211f76c24e92c01411e1af1bd02716f02203e66c5b0cddbaadb', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 178, '2026-11-17T04:53:43.460299443Z', '77a6dd7e93f0b38b', 'legal_read', '\xcd6a2e8f87d800e0d6bdd4887c702f56f8c6db5fd91268f3054d84ae46cc47fc', '\x012e6360657aec99df8a0c168f7991e16e6cf9dce8dd97688d4f292146b39bb67dd68f7310948c4a7e1b6814685bdd043c3ba9c0cd6b1120c95a726e5cefe95300', '1cc656669d2e8c654e9a3cbfec9744f2', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 180, '2026-11-17T04:53:43.484270131Z', '77a6dd7e93f0b38b', 'legal_read', '\xcd6a2e8f87d800e0d6bdd4887c702f56f8c6db5fd91268f3054d84ae46cc47fc', '\x0178a17b0b7be73a0a40a4152a1e5a4378c4234648b12fb5558582608c92d323a77aa1185927de17c079302beb8b1b8f4db50ef9cbe37276d83d199406f86edc42', 'b0407c188382e10ec32f6d983dcc1439', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 182, '2026-11-17T04:53:43.509887829Z', '77a6dd7e93f0b38b', 'legal_versions', '\x3948846358dc2d6e7d692169771d3974905001530b95ca9ec083234b92d26ec9', '\x01e747bf835670a4939c4459f1b6277d87423514485d250063ce6b3267fa114961952c507845e8a9f1d73998f25b4e128684ad6c6f3b5625ab8abd741e9c70f1bb', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 184, '2026-11-17T04:53:43.521156867Z', '77a6dd7e93f0b38b', 'legal_read', '\x3948846358dc2d6e7d692169771d3974905001530b95ca9ec083234b92d26ec9', '\x0163137e1040b2640a8c3230cee0cb89aff9ae1ba55d873e22d89d3ffb959d6e08c27fd56e8481d22d92d6137dca99ea12d3e07888405f0dbb6bc1e4fe359078ce', '01ff81867813c2419019ceea87646ffb', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 186, '2026-11-17T04:53:43.535195036Z', '77a6dd7e93f0b38b', 'legal_read', '\x3948846358dc2d6e7d692169771d3974905001530b95ca9ec083234b92d26ec9', '\x01c9cfd43f87e26df877b508727ae1df0b98d2ce9977f3e4ad965035c3f71998c0fb4e4f52f6891122e8d2172af93b2e00adb1c6890240cd381a3913b69b7797fe', '6064fcbf60fa1a26253016bb9ff6f581', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 188, '2026-11-17T04:53:43.551828556Z', '77a6dd7e93f0b38b', 'legal_versions', '\x33abfb5f93897bd2b8e83936a8f8b0fac71a5290ac5e9e503f7271be0d08dce9', '\x01d8633c31dc5dadacec401e5059a6a186cf1b321d04142629e9341ccac9c183d33089a107a6c84888906e842190ddb11a2c0c353cb0d2b54b0155fbbf8825854d', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 190, '2026-11-17T04:53:43.561536177Z', '77a6dd7e93f0b38b', 'legal_read', '\x33abfb5f93897bd2b8e83936a8f8b0fac71a5290ac5e9e503f7271be0d08dce9', '\x014ee29c549f6436bfefbb1eef4223b624feb9f60cf350e726ce8fa2f513f1975d2f62a47e103939c21ff1d8faa9688227116ad3caa3bddfd938bf929909bd33d8', '4cb778b948cfc6b601c91949addf3a05', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 192, '2026-11-17T04:53:43.572824725Z', '77a6dd7e93f0b38b', 'legal_versions', '\x5f26cd1ee080d5a0d1934910238acc88e1869b23d05379e0c620c6da75d3eca7', '\x0123eb1f06283f72ed38e3df9448113869d33618c54426d2bf83fb7b1ab38d62839ffa0fb89e84f5ff38b49b381a7bca8771eddba8b518927293e74a9850391e46', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 194, '2026-11-17T04:53:43.585899087Z', '77a6dd7e93f0b38b', 'legal_read', '\x5f26cd1ee080d5a0d1934910238acc88e1869b23d05379e0c620c6da75d3eca7', '\x0192950ab7436826044c5bd211d636e7831ecaf13d62f3a3836c0d7377c0d558d9268c7ff9112d948c83b440d67e29b883aa5cf4743a91f763855eb2f32a0ed683', 'e2dbe23b5c8210ac6247bb26bf3d8d78', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 196, '2026-11-17T04:53:43.603972663Z', '77a6dd7e93f0b38b', 'legal_versions', '\x60ff166b7012c069bc9fd66a8d4d2dcf6e3fd65b2e39f7ae2f6f35b37b4355dd', '\x01b9eae2a3bb7fd644c12329b825baabace4260492c324023e815e931fbe825a34bef5f77f0465efb1da0cd28c786003b9772654b3ce719fc2b420599bcfc13935', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 198, '2026-11-17T04:53:43.613782433Z', '77a6dd7e93f0b38b', 'legal_read', '\x60ff166b7012c069bc9fd66a8d4d2dcf6e3fd65b2e39f7ae2f6f35b37b4355dd', '\x01222a1d9e834086fc8803db5419c151d210a74ca930b894812839eef371720fa376d3b2463ad3a9e58608b262098c327557b7d861d8e370d991eacd05bd5f81eb', '52c6b2c07319a83ab011e4a24cefa6c7', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 200, '2026-11-17T04:53:43.630741249Z', '77a6dd7e93f0b38b', 'legal_versions', '\x5fa432261d9daf6aa70e374b3845b257e4a79bcf4306c057120ae37d278c984b', '\x01bb6e7d976a00bf905b59a9f4c8c857570eeaf9d893265ea13754fc53f9a85401cda99f47d7d0977cd62b1564d1ef8b7b73f2c35e8e0098dfa1f060eea31c219d', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 202, '2026-11-17T04:53:43.64336714Z', '77a6dd7e93f0b38b', 'legal_read', '\x5fa432261d9daf6aa70e374b3845b257e4a79bcf4306c057120ae37d278c984b', '\x01e8d6df05a5f4a2e27e9bd81283be4d9e796d065e484342ebc5823cd873faaf99a70d469766c4bb1ddc5a94cc9457794a6aa4afa5d700c6e0aa873ad573409244', 'c8f6ee689273cb0a608c0f199526dc61', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 204, '2026-11-17T04:53:43.651645723Z', '77a6dd7e93f0b38b', 'legal_versions', '\xba430a9b1ea2bc6988a20721b0d659a5148ced9967c72caf3e4611cf3d34e34b', '\x01e866aa94090f555d8e8a86fef4a530b37a359f2c72c80533897ff95a2e4056f0c0070d52ad22f8190abd4b3b6deacbb037ca61a7daba515ca172905b91cb06e4', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 206, '2026-11-17T04:53:43.666760224Z', '77a6dd7e93f0b38b', 'legal_read', '\xba430a9b1ea2bc6988a20721b0d659a5148ced9967c72caf3e4611cf3d34e34b', '\x011a9ec50e0a6e9ba1418f9c82f6370c00e2e0e3bf545914c01bb2b3f8f548b2c89871815356eff3104e01932b5f021d63137380cd121a1c10e6ad79d978fe6483', '6aa82c46b270f6250e19b61b2954f507', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 208, '2026-11-17T04:53:43.677479678Z', '77a6dd7e93f0b38b', 'legal_versions', '\xe4e4b067d57f315028da89db5ab9bb934ded30096931efc4180e05be37d96a71', '\x01886b85fe41e612464a4f871c76562d9323b9004c5e77d925de058cc335f86c6c194b3b596531c5e6bf377d4d2a38c2ff22178c5acfd3a0fe849cb18055dceef8', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 210, '2026-11-17T04:53:43.684676735Z', '77a6dd7e93f0b38b', 'legal_read', '\xe4e4b067d57f315028da89db5ab9bb934ded30096931efc4180e05be37d96a71', '\x01e88013d3873da274191d5f1e5767aab107f637cc242dbff6b6b71d87f4aeeab906444b0fddd1a87d26ceb1010452c824c2c3e763b01c80a589e3875a8287b425', 'ac70dbd2ad890bb04d4bd3d60e0d4865', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 212, '2026-11-17T04:53:43.694111356Z', '77a6dd7e93f0b38b', 'legal_versions', '\x6f711b5f172c3c2dd8ef85898949f0713e497e9735577244d9079acc82f922f2', '\x01539d929475769d448f881a86e69c59511735dca613c73921fa7f028a611d7a022ab4cc3180a1b9ac63abfcc24c08b8c65500c50f77ef0a1cfba33668c5bef5cf', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 214, '2026-11-17T04:53:43.706311153Z', '77a6dd7e93f0b38b', 'legal_read', '\x6f711b5f172c3c2dd8ef85898949f0713e497e9735577244d9079acc82f922f2', '\x0173872619cc5fefd93f25659d9baed278a7c6895ea24a6933873af4bb312191da1da0b19adc80cad197c30de20bb1a42ce0c6d7a77feb9a2d283d5cbbcc9fb4f1', 'd98e6d1d1c079164b855379a8670f583', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 216, '2026-11-17T04:53:43.728765007Z', '77a6dd7e93f0b38b', 'legal_versions', '\x3a5a2349694eb56efab1d16608c285597d5fba9140be89d75d3748734f65ea8a', '\x01e1421f7eced21530b4f3562ec13ba2777d5ffe0d2d518aeaf7153fe33dc5f96b6e5c30b8d204b1417ba47a94df66ef4ddc679a25518c5a6268ddf24b270da0a2', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 218, '2026-11-17T04:53:43.773354527Z', '77a6dd7e93f0b38b', 'legal_read', '\x3a5a2349694eb56efab1d16608c285597d5fba9140be89d75d3748734f65ea8a', '\x016909569211211b3cb4933515b116c5dbe3454d7f83c3e57feab8bae9f4bb3ecfe3638913a2bbabd34c8723c1fc6025a512e2576442cb9ec0387da5ffcb797eba', 'ec3b204b80c92e700c92707d459ba5ee', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 220, '2026-11-17T04:53:43.779097334Z', '77a6dd7e93f0b38b', 'legal_versions', '\xced1b4e0c77bd70f4f2ac2e2114a750ccbd1208c8f2181a8ca168082414af8c8', '\x01b0f69497a02af4d900afc20b9985e134d67a9ba28e5af428c1626630cd41dacf9235c72ff2ed507c8da7ac674dad6ebb107ebf2afe14b6e644f71184ae1aa300', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 222, '2026-11-17T04:53:43.7879619Z', '77a6dd7e93f0b38b', 'legal_read', '\xced1b4e0c77bd70f4f2ac2e2114a750ccbd1208c8f2181a8ca168082414af8c8', '\x01507b4eec5381aa37754f13e1dc3c08d0137ab497023f45fd895602a450c10e32ffb0797d877fb523c7271b09edf9cacc09d3c289783194b6d0e63eeb3bba1393', 'f94381743ec355e2c5c55f61d93c5569', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 224, '2026-11-17T04:53:43.793316085Z', '77a6dd7e93f0b38b', 'legal_versions', '\x1aa3d5e5ef2f19fb1f6068453f911cd890c80a94564e3391b7a131390741865a', '\x015331ac55abead6939d37fc8b66e57de293f0f0548e848fd148bb57dcdab1c83ca719d44b11ced1949a15750e661074f909b5490ea115b4b2a4a1a0a828e97296', NULL, 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 226, '2026-11-17T04:53:43.80083951Z', '77a6dd7e93f0b38b', 'legal_read', '\x1aa3d5e5ef2f19fb1f6068453f911cd890c80a94564e3391b7a131390741865a', '\x01a816ab373baf71e644e126cdcfda7ea7bfa165ba7043d47b8cb44c505acf7b17a29ec81f549d845ac4b1b0546d539d8ee0b5eea666f2e03c78224d6e02cfb1ef', 'e936ff4b3f46ccbe515242061d17a31c', 200, 1, false, '\xcfed381c741e4649');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 228, '2026-11-17T04:53:43.805771039Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x89dfbd797a72ac5d5da19975007de3d213e5dd469e0ec854a561a2846765f802', '\x0171e8029cf6428f04e2a8a0d063b091e8c15421a0215fb2009df59160ad5393fd08394c24790bee262c5507d576c88d50c443dd70dc8ad6010413f71fcd773be1', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 230, '2026-11-17T04:53:43.814179446Z', '3ae1d9d4e3d6281b', 'legal_read', '\x89dfbd797a72ac5d5da19975007de3d213e5dd469e0ec854a561a2846765f802', '\x013248522d39a2ec82e39a0cb70bd51ab986b32c6276a9ee56dab4710fef232a3551882cf27f7f78d4584ff524906c9ceeb76b54f126f09affe7af1ed6e9c3ba19', 'fcf14e1bdaf8178558478c7c0bff4a6c', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 232, '2026-11-17T04:53:43.826594476Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x4e0b9a99b2bb1cb24a2184a69d2b3dbffd4bc2c2c3f2568d1b2c07548db84805', '\x01cf29e8aa1a373700b2813476c533c3f55333b26b7e8cee89bec60a206f1359eb94c9cdbebde085ab9e8bef37ac3356eb68cd2422bfde6e07172386a9c5f0acd2', NULL, 410, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 234, '2026-11-17T04:53:43.835577847Z', '3ae1d9d4e3d6281b', 'legal_versions', '\xa472ace27570c8c9289ff9a8a19d530f1903052b7592b2961310b254fcbcfd3a', '\x01812c394b5fe6609eb0ea631d23f6e8355b54d2f3b75f87bb8ec762ad7524108e90c535631a0f4c7a28014b888e7485cd9e369eb19773e6adeda4ca4cebe0d7e7', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 236, '2026-11-17T04:53:43.869478395Z', '3ae1d9d4e3d6281b', 'legal_read', '\xa472ace27570c8c9289ff9a8a19d530f1903052b7592b2961310b254fcbcfd3a', '\x01ceb6b431dcc700a0a6c41799ad58a7d63b45090ed0688e4259ab0b93963e0c53f0995c37c021a769f21678bf78c7d74219a546f893b7e7312a1a0ce1614e2856', 'c5cb1e428a88d6c2573e9349e8ae7862', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 238, '2026-11-17T04:53:43.877076914Z', '3ae1d9d4e3d6281b', 'legal_versions', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x01663eed7e93d7104965dd6d70351944967fbaf5d6c659c43f6f6ddd0eaf1eb1a969df8a83ed036f232e33c194d314777546e6a50092aa364f3dc5cd4ab6622965', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 240, '2026-11-17T04:53:43.889724915Z', '3ae1d9d4e3d6281b', 'legal_read', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x01d678c20173e0cef99d412df53549eeb868431c4e2f2a40800d924fcd6f223119745d212f2db226c172d26ca10526eeadbd8a42316d173f0317bf6767b4e8d7b9', 'd0c91914a20d29efe5939fecb531da80', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 242, '2026-11-17T04:53:43.898723823Z', '3ae1d9d4e3d6281b', 'legal_read', '\xd1848afa7b5f2682b0afbad6c48503331d8dfa7351fb8fe59060195101feeb8c', '\x01e922af64c7477425d22715ca7eb9a2bba1a62f41115a470c6edca900dce8e91aa68c631c26ecc15e8792bd4fdf462da2166704c285abdeb6f8f1cd1ec6d075e3', '64ee4cf326c2c19a75012d222b5a7fe2', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 244, '2026-11-17T04:53:43.905741675Z', '3ae1d9d4e3d6281b', 'legal_versions', '\xfcea3c859c8decf3f4dd4947036d794ccac7f71cce1f18fab4ad84bb52597cb5', '\x01a8fb7f7f8be127cb69b6c5e930dc35792ea78e137fd8cdefa8f155570bdc560548b95f67b0ba9d89843418ab0ec2a7fac21b9b12617b46be2d0a3183e60ca50c', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 246, '2026-11-17T04:53:43.914133537Z', '3ae1d9d4e3d6281b', 'legal_read', '\xfcea3c859c8decf3f4dd4947036d794ccac7f71cce1f18fab4ad84bb52597cb5', '\x01aff5fa1d40803f6cc631a5649c495e075e49dd226c49bf626ceb78b318c6ee8a9997d3d39d3eb5a06160a98fbeafad46efa6c91e5099852c9c92bfe216c17cf9', '36fc4f049f519fab944463013cdadf14', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 248, '2026-11-17T04:53:43.920117537Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x6262279bc3dc8931897251711b4bbeb063ba9942f698192ac4d65813d96bcdf3', '\x014262de152e2f912e79a2600d47d24c2ba79c79e0e4d6cf42c40fdafee937d35e733111c67f998b58f64062c69c95f859213610d191856da37b9744a7b712811c', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 250, '2026-11-17T04:53:43.927313982Z', '3ae1d9d4e3d6281b', 'legal_read', '\x6262279bc3dc8931897251711b4bbeb063ba9942f698192ac4d65813d96bcdf3', '\x01bddc0cd571b739b2c77fa2ada7e57d3c449685df71415fde27019559b3d305f3d9d5a489013d3da40bb93ffa31c743d583dfa2c4fc052d2e13adba48c16ce801', '7aa16790a19930c29ad9bcc5ef8933b0', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 252, '2026-11-17T04:53:43.962316031Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x9af076b432811ea2e8f76e9d862671bee5b9b55bcc50c9cdb6fa5069025847b8', '\x010fa16e630fdaae8d94ab4c3471617128f1ed5f202f56b2121743905cd3dbf9c411e07c34aa3281d264eff486c52dcb7712671fc0668689d9f494567c6adb87df', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 254, '2026-11-17T04:53:43.993211182Z', '3ae1d9d4e3d6281b', 'legal_read', '\x9af076b432811ea2e8f76e9d862671bee5b9b55bcc50c9cdb6fa5069025847b8', '\x0170aeba793aadb0b4de38e263bda6044cafb1d026dedbbe200962bdfe626d6e5245ee19823a531044f716eb0da851918c037abf947c0c7a33041571357ad64e4e', 'b9d82401f7910fdeb30b4144d940ff0f', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 256, '2026-11-17T04:53:44.023967721Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x1ddc18203b145092125aaf87db8fa7d06263a4f6a3af443157e90c3fab3fdb89', '\x01651f27bd53086e40c38478bb6dd86124cc77cb666209883efab3216108d535ee923417b58bc8034af647ae1359960ca6af8ce6867e833c8baa7debdf5100ee56', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 258, '2026-11-17T04:53:44.034700908Z', '3ae1d9d4e3d6281b', 'legal_read', '\x1ddc18203b145092125aaf87db8fa7d06263a4f6a3af443157e90c3fab3fdb89', '\x011043733a6414de2b10f2c7b2675d1b7b8e83c4db07f70a2e0d22a8a85e3b969807d1cc8a664e345b7e4acd7f7a81524e6126f9b7de1d5ffae0b80df155209aad', '6a0490e808757e00d633522bb402a8d6', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 260, '2026-11-17T04:53:44.04441652Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x9af1a7e461ff416df6859d74fc9f0f464b8c8249dd9dfb2b2ac4e7464297a4be', '\x01f358c5c17c52dc1404dac9b3ebfd24528f68d20436b17b0b47f4f14264aa4ff5af53b6050f3e973130901a8c3f1fb1bdd2129cc2cdebd842fa160075e2727eb3', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 262, '2026-11-17T04:53:44.062888392Z', '3ae1d9d4e3d6281b', 'legal_read', '\x9af1a7e461ff416df6859d74fc9f0f464b8c8249dd9dfb2b2ac4e7464297a4be', '\x01d3474e2858a9583e72b055c280cfa4a9c9f95d2dc1afc1f058a9ac714cebd66b0c037504c549372a8430e0c9fcda5daa17b1403156e6faeb7e2bc52524bd478c', '9b9c874b0736932b74da45debce5f5d6', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 264, '2026-11-17T04:53:44.076596134Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x90eeea35a136ffb2aaa7fd930881513879c2f06b99c366fdbb6e88cd71ac278d', '\x016c44a40cb1482ce2eb7a6baa0881bb6f4dc5566cdf6c7c7d593b390ac90ed366900354e4b902791988796b798af98705b916fd484b186e9c90f183cf6d96052d', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 266, '2026-11-17T04:53:44.089241644Z', '3ae1d9d4e3d6281b', 'legal_read', '\x90eeea35a136ffb2aaa7fd930881513879c2f06b99c366fdbb6e88cd71ac278d', '\x017f8868c65238c13a14c5fa9b9f348b438eeaecb21432298ab2bb24bf07c46eee748f56c9060739ad318608f7458f06cb88aafe3353705e8ceccf328785bb9f82', '732d08b988225eefb74588e77ca3c464', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 268, '2026-11-17T04:53:44.103014451Z', '3ae1d9d4e3d6281b', 'legal_versions', '\xde37cfb31b51475fe1ff4d0070db69253326e2b54d6d7e4f941a92e0d210735e', '\x01fc1da50cafb54d641952ec4508b508138a53e84ef6b00cf81b3e522502628c96072f2aca8993f90845ddf44f0da91d5dade533bd4ec82d46085d24fd15844351', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 270, '2026-11-17T04:53:44.115960675Z', '3ae1d9d4e3d6281b', 'legal_read', '\xde37cfb31b51475fe1ff4d0070db69253326e2b54d6d7e4f941a92e0d210735e', '\x016f7f3bab8227f9ec4b846dd230feff5dfeb3faacc512bb76c31e9ea0d1372b599d06c63ca2b45df819fac3c088f85d4a5753af1f47bfdad590d6b3a6afedbd71', 'b8498ee8c6b06929ab963191c38a1947', 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 272, '2026-11-17T04:53:44.123446334Z', '3ae1d9d4e3d6281b', 'legal_versions', '\x703d2e1ce11d670aa163dee6f5492bba9ed43f97c8677dcd31b763930142ebf1', '\x01703f8c1faa06e0f305c218b6bd3dd45f512bebc2e609f4f17fdb27a7e45938888ba5ffb0a4a305ee4ebddb0251c2915eca7adbdc6d83a8b4d26e6a440aad14a1', NULL, 200, 1, false, '\x7e538910913a7f79');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 274, '2026-11-17T04:53:44.139528863Z', '3ae1d9d4e3d6281b', 'legal_read', '\x703d2e1ce11d670aa163dee6f5492bba9ed43f97c8677dcd31b763930142ebf1', '\x01f126ba979c2c7beafc5af345ee2a6e9adc5de453559f5463ccd8ffd724ef6ddc67246e2968233d55aa0fed2af53f054e58b545e1c186a30d4b6f547ac218726c', '802df3e64de4c65d67594a47fec84fa6', 200, 1, false, '\x7e538910913a7f79');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xcfed381c741e4649', 1, '\x019f4fe8eaef7a76a45eb79e0b37e471b948db522172e98d1a8ba48a0de427c90e22f45d9ed7e7eed464f21281a4570eda8a8eeb9471b0aba32ee2cbbc');
INSERT INTO vcon.audit_key VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x7e538910913a7f79', 1, '\x01159ae0e6733c83fad3e62cb995c11fe299a2a79bafe617cc298b633c7845cace3a84ad2a0e246758b6c2e998b0d15f61c8282555d2a55dfcb86b99a3');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x8c27a0f6ded858d8', 14);
INSERT INTO vcon.key_use VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x82b55e2c5a9a7e54', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', 2, 1794891221148978506);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', 4, 1794891222033287720);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x6bb101f453ecab11', 2, 1794891169542009501);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x948ec27298a38917237a321f2652c9bb', '\xbf26f82a77cb38b279f2c7d7a2586375', '\x0192deebfbb8ba7d39594b8398af461a9f2cfd2ecd806691a72d16b2b91c486850ec16', 1794891199681918231);
INSERT INTO vcon.membership VALUES ('\x51d98fef98f8355c4cdd189a8114b160', '\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x01faf8b40a16969e1039cf64552938473fda71f0896c2ce55636f8da32d104334b57cd7a', 1794891203338109285);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', '2026-11-17', 12, 12, 7404, 1794891220274292451, NULL);
INSERT INTO vcon.parquet_file VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x7dc25d25b6ce4bc7f314f09f1627a203', '2026-11-17', 10, 10, 6240, 1794891221268259729, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x6849199f0f656743');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-17 04:52:46.975776+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-17 04:52:47.028656+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-17 04:52:47.040233+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-17 04:52:47.04639+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-17 04:52:47.049141+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-17 04:52:47.053092+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-17 04:52:47.054667+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-17 04:52:47.063588+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-17 04:52:47.066634+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-17 04:52:47.067314+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-17 04:52:47.068682+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-17 04:52:47.073123+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-17 04:52:47.080003+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, 'e6047d6e852776341bba4ea78dad32f0');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xeeef02cd8fafce1342cf06960d8c9aba835fd23c3e16bbc9a8e5795c25eb44a5', '\x0174bf8a2e4901adc434c6d295c364181e4052bdfe4e801270931bf0b1eae8efe8df9d51a2', NULL, NULL, 1794891169557096513, '\x8c27a0f6ded858d8', '\x015a9f7e823d8c2e32da504a9d2db1353021fcc823d03e31988712185a4806a9546022571cacfe14d30f24acd5a0470ec5dcc5ab8df9e2b6a050d7240c', NULL, 1794891169557096513, NULL);
INSERT INTO vcon.tenant VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x1b472c2bafe217e3c15a1a46afd440bdb29a1b0307855ce9a085926d80820d66', '\x017aebfd11c37cd9f3ab9691c76e29335345fa57c1becf37515baead91d677f31bbd3b104a', '\x01b7b37a843d9ed6c66dcc3022f1509d4be10f93498d4adc9743ba281855cc', NULL, 1794891176632025373, '\x82b55e2c5a9a7e54', '\x0159d5ad30fdfab2f1a7f146809bdc8d8cca62267a28ea767c9bc700c51e08fb1da4f2d58a0054cefbeaf200e9761311c06d5bb997840484a880f0a9a7', NULL, 1794891176632025373, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x031e01a5012e4195');
INSERT INTO vcon.tenant_catalog VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x7932606525c90b9c');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375');
INSERT INTO vcon.tenant_lock VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x5dea9dc5f7e1911045b9a1bde29312feb7320d3b5bb912b333f4dd397c17d87d', '\x01fb5c923a61bbe5e4d56938cd0363a2847c0d33ce7c0940dbb5347e7a8be140d23b7b7a43cd5a0c96a14e87df', 1794891208887452378);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x948ec27298a38917237a321f2652c9bb', '\x013154c4902ce033be650c95d0d1799a0a868a97e0bff205c3e91f9994fbf17939e4e39f303bfecbaf7281', NULL, NULL, 1794891190966279828, NULL);
INSERT INTO vcon.user_account VALUES ('\x51d98fef98f8355c4cdd189a8114b160', '\x01c6015c46af9f5d5c54802e5e0be3d40da25ab87d14fb0249e82229f9fe6566d543788e124bc66914583056', '\x529df375d5a687a5fe3caf8ba10614305a4f42b4038cfc96941f3487fbd5a3e1', '\x01a68cdfd94618eb6bbaa5d2b7ea8df6501e4734c3cbeabc6512e7bfeb71afeefcd22410e2a1ab9d1bfbb0c5150708c8556f58e128ba0ab4149ce0f5ccde3e77', 1794891196269875780, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x5cf9d3453c5fa0a733c69327f1cc3178afa8316a92900fc559c4c53bea873194', '\x0116cc8e404a33d101926c92b45ec476c23d6cd3c5d42bd010da60efcf0f74a45a14e9e1cc866fa2fefdd2cba3', '\x6aa82c46b270f6250e19b61b2954f507', NULL, 0, 1794891208086473048);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x97c9e01812bbe7f1d8a6684de4d36bc832bed3eee856021ba5d1a1fdfbe808bc', '\x013b5a4aa5e832f307e8d6dc500356adcb08c7b122c08a8e10fafa7b057577212999b5ec9ed22c36525976b45b', '\xac70dbd2ad890bb04d4bd3d60e0d4865', NULL, 0, 1794891208101823615);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x47a5988316a62a82d52aa2bfbd203ef5c55faf7e2c52e02baed03d8329af0939', '\x019077b6e151eb75bda7532ac7f70f68e6d29b16a40e3ef31b1ad71e5713ea80291ec5118678d5aa0fb129eb66', '\xd98e6d1d1c079164b855379a8670f583', NULL, 0, 1794891208118725516);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xf29f743c8acfa40eec6474114c712751eada4795edb83017f9313cd1da9861ba', '\x017752a8c2afac3bc751bb860418336c60558afe6d934186fa3f7a3ab5e184b522d699d1d2ce9cb60f5c350cd1', '\xec3b204b80c92e700c92707d459ba5ee', NULL, 0, 1794891208134921685);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x05873b2a6bfe88c0782365f46655a50849cc7d28eb039821d51e4e7b14799e0f', '\x0171c2bba43e5bb113c3fac0f9979d5a1f08177a67773b23c2893e4e3e51ada5222e58c801d8f5b723b5a37fe1', '\xb0407c188382e10ec32f6d983dcc1439', NULL, 0, 1794891207743875011);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x9ff28a2218e707f39541504b97926c2684963696e1c07cfbf808af094bddf813', '\x0113dabb04c7975c36b910e7eff5ffc7de091df3b354b5bc91252f3a3ed83681182697096675ee5c2b0c84dc34', '\x6064fcbf60fa1a26253016bb9ff6f581', NULL, 0, 1794891207842787983);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x0e317a36e4425a7718a56d3eeb99c46a5df1d9fffe8f077e35d3de50b2434b66', '\x01bd2fcae40180dd37ef3527085a6d1008738713a3b69b075c72ecc050e8a222635fb9353234c3cfe2fae06d74', NULL, 1794891208230934069, 0, 1794891207940087272);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xf76bf50cf6320baab7c03e7c1ddf3d50af62248644b617774c49ede760724e9b', '\x012d6f7fa465d00a6effa53337ea0e3b16535c1fa7a904991ea774112380ee10450641016d73524c6a65b27e4b', NULL, 1794891208248538923, 0, 1794891207996919295);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x78037477d437604d1097496d48b2ee4ef161e3cb7616f8f34d93808f39371fe6', '\x0135c6c9d5c8c7d44cc28e114e3cd966b122d57c450e69a06e5923e59cbfb50dc7b272b86b3499c1236ee2ef4b', NULL, 1794891208342187822, 1, 1794891208027871818);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xe095628173a0beadaab99f1dd73d094d94e3012a712a89a99198393001f2d196', '\x014b691f23847b9ee51a8a465f3766497b156f0ee030ea7ca8a525f5284ca1ca76c17d5b38201527b74188a411', '\xc8f6ee689273cb0a608c0f199526dc61', NULL, 1, 1794891208071478065);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xf388288521c63060e71c120fe6a9db8f320793acca44ee3aee0c93ead7057bef', '\x01073ad824e490cfb86c656bfa91f1c196032ad73dfdbe0f5e2910c21c2c43abff6fc597d11385d50d3c44cf2e', '\x36fc4f049f519fab944463013cdadf14', NULL, 0, 1794891208709712084);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x0f64a850ae8e979b5d6da2d566cd16f001653a3646154376ad69623fc91c253e', '\x0106bf45edb91a90b83c6fe5037d133381da59504906d431b4b749ab941da4f56d3f8cf1ea4d95ac8af32d2d77', '\x7aa16790a19930c29ad9bcc5ef8933b0', NULL, 0, 1794891208735940032);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xb8db8c08a7277cb50f963c7985f62072cfe8df4769e68b23fbd87cca3e591a59', '\x01ef8af2abe854fca90f76c5511f0dc0bf04ea2a9365c07dae0de7f658b6e29a9eb2e93d811ededb6019f0918c', '\xb9d82401f7910fdeb30b4144d940ff0f', NULL, 0, 1794891208753781201);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x5f9cfe7eaaccbdc969371eed1232be8c79780cda97811b507a193725a05ef1c8', '\x012615629dfd02ed50a773d5fdd47ed3133b260aad510851415120d7b2cb5b54ecd403ed90f756fc3a38d353e4', '\x6a0490e808757e00d633522bb402a8d6', NULL, 0, 1794891208787973197);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x9a24181dd07873ba1fc70d3847149fb91d7da4faea919fe7f565853fd60e5046', '\x0152a8227847d54c3d7fb9bef3cac895f2ea8cac6c2355b608bf9ff9ceafa1319ae7d97af196e27fa27b7de410', '\x9b9c874b0736932b74da45debce5f5d6', NULL, 0, 1794891208805280255);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xc45d10d78a57e592d2943dcaad92fe32a4dc591f73990eb63ce3c06d4a723347', '\x01266be18ce2e6d1f02567ff4f6ce44be4ca2e89376003769ccf5e9dc9f9b46d3765d62c78f99d07ed1f10c39c', '\x732d08b988225eefb74588e77ca3c464', NULL, 0, 1794891208827993740);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xeda5a7515850a2468219b6496515d4388a72f9232e2f312e2aa5c662b50b01cd', '\x01143607f7a00fcefad169dae96e071649d05372e165247eb26dbbc130ce5ce3222cac21f728d6eace35c1e062', '\xfcf14e1bdaf8178558478c7c0bff4a6c', NULL, 0, 1794891208543103453);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x4d354efd9993bea86e7aab4935bf2539e9a3dd197b1761bc660bf38fa6a00256', '\x012df1b3ab541dabc840fd389e5f28a992750b07328c3043f2f32eded35878fe3de4e79c86aaa5ab47a39f2617', NULL, 1794891208945436290, 1, 1794891208655743623);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xde0d9824fdbede0b0b5e010ab130787a5b115f37f4495b3cfd1cee2035bdb98e', '\x016eac3fe1a465bd1181db3e71edf6596c8a51325a0d7e637295f1cbd34a02a164f8402e048d7bf3c5a0ad95eb', '\x64ee4cf326c2c19a75012d222b5a7fe2', NULL, 1, 1794891208686036349);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x5cd8c42fec2c392b14b5b265f4e21d2712ec53042714c7eeac9e173d00487c63', '\x0127f065b8773a7bedb40e7f1183ffa4ff61ae086734db0bc6883700bbad01f8cefd01da081927c757962b4dd9', '\xf94381743ec355e2c5c55f61d93c5569', NULL, 0, 1794891223258902557);
INSERT INTO vcon.vcon VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x14ccc0818c2aa3f925ff323bda0c9f015e57fed62415dd6b7e22441de85d36f8', '\x01bc80625d345dfa20a75a0d24652503b45d8df2bb4248e2ae9ca3f44c7d6afdacc037433baac8b92e944e12af', '\xe936ff4b3f46ccbe515242061d17a31c', NULL, 0, 1794891223330655903);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x7840e982c2b10bddb43b4c00a5bd86b99ac236bb65949a866be4cca59fb82020', '\x015f473ea6ed62352c987ecc076ab2b2b55e0fd08ce6f9ea74b7255aa2c63b96e343ca1617f74e7c68a586d52d', '\xb8498ee8c6b06929ab963191c38a1947', NULL, 0, 1794891223391027205);
INSERT INTO vcon.vcon VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x83899e61ad3375dbcda01b0d5e2bd9f45aeecfc9f1c8f156325294d2e2ba1d16', '\x01d71c7759b5dc9124547612af44ea76c4efbac103507cee2d0c3f9050b341ffbf1461c9896f02f3ee63caadea', '\x802df3e64de4c65d67594a47fec84fa6', NULL, 0, 1794891223420990513);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xc5cb1e428a88d6c2573e9349e8ae7862', '\x4d354efd9993bea86e7aab4935bf2539e9a3dd197b1761bc660bf38fa6a00256', '\x01e13ab9b1dc1be6b7de6f09e8fd311bc97490e03a429e1317df84c0926bf60e97b1ea2caa', '\x0120120b5ff393cb5a530a486154e4d321b90a5f4a4de87f5d5be699d259', '\x01ac97d3be07337aac7753420876c11e6aef4facc05975df5d3a077e9e7087184848a06921', 1794891208655743623, 1794891208945436290, '\x82b55e2c5a9a7e54', '\x018228acc2ee6ee40cbd90670b3920cc3cc419a51b4a8d5718f0c0068629375eadcfbc9ae6a1deb5e8d7ec1e57bd1f19eedf7a80eee15cc8f2cfc9fc09', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xd0c91914a20d29efe5939fecb531da80', '\xde0d9824fdbede0b0b5e010ab130787a5b115f37f4495b3cfd1cee2035bdb98e', '\x0115557eeebd06ad50b1e9ceaf6a69e1d257329302f2b166aa71645bb156fc388830c18421', '\x01e4a15b3dfacd705f7c33cf92b6d450310fc06707410c0e815f9c32bdeb', '\x017d9ab1ff6c5b2f99425e0767448c559a6058d6ff8f1c6df2cd128481682118e7ed287d07', 1794891208686036349, 1794891208979755266, '\x82b55e2c5a9a7e54', '\x01f71889d742053afecfbc3cecb0d3c363da204da36c2042898906126122d58ca248edbdd9e00d544fb8090085469992f6a2f1ec2f0bb709f35b531655', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x36fc4f049f519fab944463013cdadf14', '\xf388288521c63060e71c120fe6a9db8f320793acca44ee3aee0c93ead7057bef', '\x01bb1fe3f5bbbba8a6cf279c6b942955cf1a8451011f5bb15932dfd750a6bc6bd820040a39', '\x017831c57157ff852776328413661a119d684482bfa43751b8c64e4d0a10', '\x010d5dd169cc1f00f227eb6e7efaa31be095503006f98cdb73b36d5159825069f3f101f914', 1794891208709712084, NULL, '\x82b55e2c5a9a7e54', '\x017598964b48b3443702260460c0b2f74c3bd8fef7450f4a383f4e0c64058bb0f680e9525390f579ac703fc50247d2c8bb1f88ebea944b5a22aaeba487', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x7aa16790a19930c29ad9bcc5ef8933b0', '\x0f64a850ae8e979b5d6da2d566cd16f001653a3646154376ad69623fc91c253e', '\x01d3b033ee6a354b6258fb856d49857a99c41b966de281e34ec658628986da4c4c28bbc9a1', '\x019dbbcd7ffa3eced441dbee7434fb3802bbe52f1ec6311001d2d34db4f8', '\x01dd493d81e8a8d0fefc3a2b07c781bbee371221c2437db593466d46c1b771c069a409954e', 1794891208735940032, NULL, '\x82b55e2c5a9a7e54', '\x019871d71e0949d2aa5e07676b723d5b4fe5c6a382252b16416b3a7dddb681c35113f94b24772e3c72fa64dcf2bb7cb047a47457ed33d12b936552cf01', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xb9d82401f7910fdeb30b4144d940ff0f', '\xb8db8c08a7277cb50f963c7985f62072cfe8df4769e68b23fbd87cca3e591a59', '\x01b489cba0552271cf9be4257bbfaf65787340fea00d0d59db63141d7f718c01dbba166bba', '\x01983780a0a23473053cb17222118e50f125fbcc5e1377d864b63a2659ac', '\x01719af99e4a4240dad716dbbd821817b16ec9d77c25aaaa0e21c1009a7049720b1761fa97', 1794891208753781201, NULL, '\x82b55e2c5a9a7e54', '\x01e40fdabf83ad9081d2378e8fbb39d085cc8094dc1f5192508fefe7342224c9d855d55695806b4857bbea68fa7caa7687f22623e097a21cd0f3535f56', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x01ff81867813c2419019ceea87646ffb', '\x9ff28a2218e707f39541504b97926c2684963696e1c07cfbf808af094bddf813', '\x0143161e658af429e5449e2ef99d1cd1197ffc284047b3efd689c206620aaf013dea55ae9d', '\x01f1d9fc9f1a0b6f5ad5d3607142ceef242a0af88475e980fa45caff7cec', '\x01e99d386e189b8bd1c9f8cae749cf276299ffffdf234fde38a6822f76c410af14c1f62e32', 1794891207842787983, 1794891208187150924, '\x8c27a0f6ded858d8', '\x01bfd2736cb04067740265625e21ab3a92ff0bf9e65a4e81dfe8e7294942ee72a1ecd9a285612a5f3f465b5dd005f8d7d8532bea3f3bd09bb5792ae368', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x1cc656669d2e8c654e9a3cbfec9744f2', '\x05873b2a6bfe88c0782365f46655a50849cc7d28eb039821d51e4e7b14799e0f', '\x011b110a98048147db3289e7d8f18d4a5bb84f702c38e913326134a44f054b0c7c15e68d1b', '\x01d73f90052dfa0bbebc08a47b561366d6d68f54d7e810722a67427f9813', '\x01c93599deb95383c8057b65030dbb251a018011591026d81d653e8bb22067488f191feed9', 1794891207743875011, 1794891208158741022, '\x8c27a0f6ded858d8', '\x012029f3e025641d64c1874541fa777d4ea45b36e573a93d9320a1feaad8d37499ab2ab29493dc41b1c59cc5e02f9fbf71d07c2eab3a7cdee8a6a2cf86', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x4cb778b948cfc6b601c91949addf3a05', '\x0e317a36e4425a7718a56d3eeb99c46a5df1d9fffe8f077e35d3de50b2434b66', '\x01d4fad0e5fdef8668bb4f783829bf13521acca66b4caab00e01740688eee351175561f1f9', '\x010bb0e5ed7886cb8bf6b4008b856c8b2058939102d3aca8517c0ff85732', '\x01a06d326004929a0bde9083837bb6c99a4747f43f105789f0d7d0643164b75872ba85e809', 1794891207940087272, 1794891208230934069, '\x8c27a0f6ded858d8', '\x01539a9411ffbd66db7813545dfaa25704841c39c4fb52f8cbe773a3fded8afee4e5064d1c1777bf1a14905580cfedb8f22f5bbb94fc1dea99d2987978', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xe2dbe23b5c8210ac6247bb26bf3d8d78', '\xf76bf50cf6320baab7c03e7c1ddf3d50af62248644b617774c49ede760724e9b', '\x01744777a0393a7db971c9231efdc5f6b576b38046b8c77e7f43c5d20a7b3ba4028f2f81ad', '\x0183a541b4803ef2809d40d1982b7405f030a5cd773436a9e35e942b410d', '\x01b1627ed868940ce6564b131a4e83f26a2f05906be051e597bce25479c259d411e17e5276', 1794891207996919295, 1794891208248538923, '\x8c27a0f6ded858d8', '\x01eaa000b7ad15812dc6e14fc25e93b24af8e87ae4630c14037c0b3c531b86672d7e4c981f103e4708034a4b3448b9be62ff2a2599600405f91dceac4e', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x6a0490e808757e00d633522bb402a8d6', '\x5f9cfe7eaaccbdc969371eed1232be8c79780cda97811b507a193725a05ef1c8', '\x01d15ed41d4be6cf4876df6ba0f58f07dff287a8d688a380d398b0e3894ef642644c6d069b', '\x01f287bf33447ad36999d2d2acf865196fe26183ec6296ee5f622d840e49', '\x01feab295a8b0e83774bbd9337dd89937edd507172f59e94e9d7d3998db4205c199e740b5c', 1794891208787973197, NULL, '\x82b55e2c5a9a7e54', '\x010bfd6730830c44d8ee870a271f7f6c45a42a8112ff25df9416adcf7fcda75501e257e7629a6e1b6c578273630d77f4f042e861a32cff4d59ed5312c5', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x9b9c874b0736932b74da45debce5f5d6', '\x9a24181dd07873ba1fc70d3847149fb91d7da4faea919fe7f565853fd60e5046', '\x01f6bb5fe12dcf388143e6a09559f685417aed39532b066079d4c06a1b57268c0edd176d9c', '\x01294970a6125b06c4d4f85111b6e93fe9c6751c27bb6dbbf9cb7c61f049', '\x01c5d0a2eeec88176da76b05bf783f5e5700953d6be7fc5cfb4b7a69f705bd506243d75e24', 1794891208805280255, NULL, '\x82b55e2c5a9a7e54', '\x0141ccaab2dc2093a5f49b460f4d43f0c306e045fff24ae7e324c3c54c9994b7c09775ee05d2fcb0274d55f80eb565d9969f2755dace043f0e18adc2f7', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x732d08b988225eefb74588e77ca3c464', '\xc45d10d78a57e592d2943dcaad92fe32a4dc591f73990eb63ce3c06d4a723347', '\x01b68ac01b92b141cdbf798eb64edd3260b5fe946b5440fb94764f6898d2f5ade7d8154dd4', '\x014b34bb3ac65e069c162ff686cb20ddf894ede513a7ee37665dd2569853', '\x01fcc1684ef570dffc1fdba96ebba3080344e8a7ac6417fcb63bd275e495f111d1a5df4148', 1794891208827993740, NULL, '\x82b55e2c5a9a7e54', '\x01fda01db9d0267245c7c46864f3529fdba05483bf5062bcba224ae7e7996cf49eab664450d33a71cfd0e8c911da0dfb8124a41fe6083bf6c0340d9ec9', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xfcf14e1bdaf8178558478c7c0bff4a6c', '\xeda5a7515850a2468219b6496515d4388a72f9232e2f312e2aa5c662b50b01cd', '\x018605fd2e358334af76da0c1959fa7183b11b6891090826da976e68c90cd2e7b9dcf58811', '\x017a04c1c05d2e0e52c3f12146f2af3420b4df7be5845132f32e737d1a6d', '\x015b88a7f486f98bccd1dd27e4008a6648269ddff2032f1c1855ad8fa4da509ed166982fb3', 1794891208873163061, NULL, '\x82b55e2c5a9a7e54', '\x0164b6e9144247236e17a71c9d1f4658ba81cfd9669c982087c0b1b61ad49907260a67ec214b2e00a26f5e1d40844cb5615e29190b3c3e32df10685543', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x64ee4cf326c2c19a75012d222b5a7fe2', '\xde0d9824fdbede0b0b5e010ab130787a5b115f37f4495b3cfd1cee2035bdb98e', '\x017ec1aebd21c22be57f2a39cd1b7af64129897e046266881c74b615b2449ee5e1025b1757', '\x011cbd1f4bbe00fab9cbaa72e175ed98a6059889a97744e3db4c07410e4a', '\x01ac5e13027bfd8ee038c5b78cf9b6f9eb1d472880ef793d3761d3674a8678db8b4992d5dc', 1794891208979755266, NULL, '\x82b55e2c5a9a7e54', '\x01a6c7078b3af61b8fdcab9c17a7bf8827a9f35d696ef09fe5e2ac6bb3a12e1ba2221fe5c27d6906fb80d91f51665b4b118a8c73d0785598da220f658c', '\x7dc25d25b6ce4bc7f314f09f1627a203', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x52c6b2c07319a83ab011e4a24cefa6c7', '\x78037477d437604d1097496d48b2ee4ef161e3cb7616f8f34d93808f39371fe6', '\x01045e07e2d438e0d0caa2ebb42a0e425274a60a4d59468a9187661aa96c44a74421483abe', '\x0187e7f31c24b7e0257dc59ebb8be45f07c99c639036b66b2310631bb346', '\x01105b9bcc1a960006e384ab5a8a847bc8ac428f7a7bdfefaf9880f6fca80c0c1f5924d85b', 1794891208027871818, 1794891208342187822, '\x8c27a0f6ded858d8', '\x0165760d2bbb2552155d49c78d8d2e35017b75eb6d4e79772342eb492d6898f5cf3afe7068be6d50d98015233e5e129d008aa90de0223797e06f140dea', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xc8f6ee689273cb0a608c0f199526dc61', '\xe095628173a0beadaab99f1dd73d094d94e3012a712a89a99198393001f2d196', '\x01a35dd75c870fd46891584228d9e6c0c320177f9713638fd42b86e7a79ad30ca903f5e49d', '\x01647d4b3b237fcf926de45062fc394d1faf5c95d48ca5bb1656e8821db7', '\x01bd375e5329158ff20f7e659950c8999621a4c167adf122ea57d71fa51f858f447fd84dee', 1794891208071478065, NULL, '\x8c27a0f6ded858d8', '\x01f6522e844f3afc80566f6b7d5ffc9e4c8aec94d175145abeac3f06cc77d5fdf6b628c141314820cfd0b39e9e20f617fd9f8095d91b7b40d724f54eef', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x6aa82c46b270f6250e19b61b2954f507', '\x5cf9d3453c5fa0a733c69327f1cc3178afa8316a92900fc559c4c53bea873194', '\x01cd83e9d3ed87144b4a7a899ff4827500ee0cd7b62b41325d15a649cc0a0d1e3d81019603', '\x01ec063e627094b683b4de227665de6e1641d836edf4b7fe361555a90d8e', '\x01a2ba4d158557762c59988ff064b39a3756a94c88ceddc51a1dce9228449cb96444cfb6ce', 1794891208086473048, NULL, '\x8c27a0f6ded858d8', '\x01aa9beb19b41608050cdc109b5a62b4a3449f7d7a13a27dce8ef67f30bed8d50ce7952949c646afe3a710045cb06162ebcc7e1ced84d4b8d2a293635b', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xac70dbd2ad890bb04d4bd3d60e0d4865', '\x97c9e01812bbe7f1d8a6684de4d36bc832bed3eee856021ba5d1a1fdfbe808bc', '\x010e010e3266d02bd29067c9bffd744bf8045b24c348652b2cef529ca30c35a3aa82be46d7', '\x0186f9c36a1fbc49463957d1a8cc714f1e7e6c1a181def808fddda35fb41', '\x01768f4acca84cefc30bca9dee5e328288f2a615e10141c78a8577e40adc1196bf7dbcd631', 1794891208101823615, NULL, '\x8c27a0f6ded858d8', '\x01dc2d9be6aff8962164e2455712c9d0187125f6f82b72ff9b2d794f183bf35f67423a32ad5cb400009c23ff4bc29049796e47ffc2c3ca405a467dd89f', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xd98e6d1d1c079164b855379a8670f583', '\x47a5988316a62a82d52aa2bfbd203ef5c55faf7e2c52e02baed03d8329af0939', '\x010aaaf0de310910d13af6f96d72f0f37ac05e0673cfa7069a1c834bf585625c4b56354128', '\x018fef65407cc5eaf9c0552e23aac4a76ff1174fc44a72711e693d1ebc34', '\x01efbb07732bf3d3d0525d4f16cc0009e9918e0470fad14f8f5dce629cfe13737ea4dab0a4', 1794891208118725516, NULL, '\x8c27a0f6ded858d8', '\x01b2996fae54c0b996b83e620cbbdd3a5abf42d4c255fba5a3b71cf15ae8739a334f569c2b75383de809079b3fcc5a75b853b44bb8aaab74315b6125d1', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xec3b204b80c92e700c92707d459ba5ee', '\xf29f743c8acfa40eec6474114c712751eada4795edb83017f9313cd1da9861ba', '\x01d595aa8ab761071f935c6b20141dcdb323c4846fe1b2c0f088f8f88014b8cba3dd570283', '\x0179e34d2a9e5d87f4c3116feb4cf1541bc6d358e40d27d44bb19ec6f713', '\x0124c328e924a9c78fbcef8b5c0e30c3b5e850543bb6d46c35e14ed89dd797112725b594ce', 1794891208134921685, NULL, '\x8c27a0f6ded858d8', '\x01fab1ce4550b5770e99dfa68752c41cdd42c9aebed005ac6cb6819e1dd489f40d5b693a0887d0a07e0372d3db3a119b43c8573c3cc61da9fe4d2dea57', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xb0407c188382e10ec32f6d983dcc1439', '\x05873b2a6bfe88c0782365f46655a50849cc7d28eb039821d51e4e7b14799e0f', '\x019559526efef1a47ec7508b6c9a796efb8376d44b815c4067f323630fc1fb08c24b900abb', '\x018c771d4e075513af54469cc2c57475f23bdf60fc248f3f384f9893f076', '\x01e9bf261129334dc18367c3a1331c170756eee3473dd5a97c2cd6eb67648d860081b4b246', 1794891208158741022, NULL, '\x8c27a0f6ded858d8', '\x013dadf278333356a0b5816755f9bb76b476ed4417af3a1fd3aaeceb44facf0909697d3085460ccb7841bba1dbb3524ef78bd4ebe03c2e9ca25ba66b03', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\x6064fcbf60fa1a26253016bb9ff6f581', '\x9ff28a2218e707f39541504b97926c2684963696e1c07cfbf808af094bddf813', '\x0185b39ab7e1d38185b5bdfe867a581961adbe3fecdd851cd5d7e0dfcba3cde2ac6544a67a', '\x015237a235b5d5e94d71e88fbf18118b7ca62053709f3db8a8daa8eb59ae', '\x01826073ca79eb58c5fc9d4142919c08c039eee7c73211876d9477ae758e1315ddf57e349b', 1794891208187150924, NULL, '\x8c27a0f6ded858d8', '\x01612c6f94f9620b7ce490ca0242a3f9da32e2387fd6b44f5c5d523f8edd4f25983a4638e9489bfeaf5cc93a433a18ecc30f1bd27aecc4a19e5cee8cd1', '\x9ba1d3a53a6baeb9f27ef4e7979e7c11', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xf94381743ec355e2c5c55f61d93c5569', '\x5cd8c42fec2c392b14b5b265f4e21d2712ec53042714c7eeac9e173d00487c63', '\x018d28b6da057d77e32a07d15c7d91826089f3cfb6db913f75467432aa2edc80cf747a0af5', '\x01f0795adf7318d4c924827af24abb3ddf2983b3adaae299e283be47e6d3', '\x01f173c74adab5381c10ab1e3a0eeec7a25b9e94c85ca8d1d1547ecb4da9c9243f4c7945b6', 1794891223258902557, NULL, '\x8c27a0f6ded858d8', '\x01d411a1dba277a55dca738980b886da6fec0ed86d7e2188253fb4c1123b800771d28b4d4fb89a0e443539d416e3ee628307d533e49e6812eaff87ee95', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbf26f82a77cb38b279f2c7d7a2586375', '\xe936ff4b3f46ccbe515242061d17a31c', '\x14ccc0818c2aa3f925ff323bda0c9f015e57fed62415dd6b7e22441de85d36f8', '\x01377bddad90dc46a56e7fcd9f029c5463226e5a49a73ae386e0869cd34443441cd6a0806d', '\x0144f5469e6c6ef0bd3dfca4ae0f6a637ed16ff143a0c080c7729d4efcf6', '\x01cc8256e86957007c009f176ebe190059faadae01567278a15edbd788ba9b3f95d20de106', 1794891223330655903, NULL, '\x8c27a0f6ded858d8', '\x0191265ad7274f8ac5ac932eec9ab1ccd1da6ebc1ba817ffe5a94a13111cfe908d9327dea2da28e8fb18496ca8887ffbc2e687879ccfc255a9bdcec4ca', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\xb8498ee8c6b06929ab963191c38a1947', '\x7840e982c2b10bddb43b4c00a5bd86b99ac236bb65949a866be4cca59fb82020', '\x01ad78025909cfe8743274bed5f15c5ee59786b5ecb42251ee3acd8727e46078698e3d5e03', '\x0114ccaab218f6a420d890b3fc29e3ec12714e8eccf58c1bb3932438f535', '\x01139eaec1c4542a5b039d78c35655afbe02b630761a328d9d96ef00bebb076a1e3d0bdb01', 1794891223391027205, NULL, '\x82b55e2c5a9a7e54', '\x0153c4f40242cdb525d02119c36ed2773409d6e230a7bcc0b0d853c4106cb8fb835216589540c94b2bf3d7ca6aa5411910e7e53aba7e20fddc79ca4fcd', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xbcc3988aaf8d5e7cbbb1794ed520c3f3', '\x802df3e64de4c65d67594a47fec84fa6', '\x83899e61ad3375dbcda01b0d5e2bd9f45aeecfc9f1c8f156325294d2e2ba1d16', '\x01861c9d3d9b2cfbfcdbad84e0f6e3113328c78ed194e054f9ba21c8dcebe32508a5a1617a', '\x01b1cbe299eec994c0914f12b5abe9b65ec5129415f09af822125f317d2d', '\x01b0b81f290d2633459b88c270dd94b8523bcdc273cd33dea846ea1b1db421f8c3d74299ff', 1794891223420990513, NULL, '\x82b55e2c5a9a7e54', '\x01e18c8bdbc910f8cebd472d4c433d3a660067dc5168b3b8df691926c2497049ab28c5bbb7e2144b49235d3cadce52e1bd5ebe2f4e186a1cfef3c25fae', NULL, NULL, NULL, 56);


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


