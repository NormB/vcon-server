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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-26T11:39:45.978521Z', 'cli', 'tenant_create', '\x9eae33c716fcbe8a01967e0c3d160784d017a7f427ab3e6c6b31c35155104b5e', '\x01a9a3cddb2a1afeb0ad0dd5d08b9a11a1956b3e2b46bef369812cc7d985d5273a2b0b806e6022098065611c0c9289f3f1307881b6acb41851e730489b', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-26T11:39:47.867811Z', 'cli', 'key_create', '\x1842097da874cb32bcba8c0b0691e09e5de47806e25fa22e7077b5aebd6f575b', '\x01161f64b1664c22860a3f5cfd3e85e1d9f83c54d399513af63b46250e746a0e109d83caab6b97a5ae1bc25d49', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-26T11:39:49.664758Z', 'cli', 'key_create', '\x26cbca4a9442ec7c205470c6880e4fa0c92840b917856ee40d8c58e53ec19275', '\x019352185bfb83e36c2ee2b8d405128bf3d185732a1aca4f50d21bc17064585216d5073ea5176b5e7df01810bb', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-26T11:39:51.587966Z', 'cli', 'tenant_create', '\x1de714dcbdb9e654139ff16b64de0bbb7372f3cfa6c515db8362079f83be6a92', '\x013f26cbc815240f7e673490e3f39ef1dacae63a6dcb98af43f35f60db95615ee7b22731d2394104e2739459fdf1b7c2851df89ddc4d4426488ecd107e', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-26T11:39:53.457482Z', 'cli', 'key_create', '\x078aefa9c066e3c77ce186bb7478620421913b5f8eadb43e76b8ba632c34a81e', '\x016d2cd6cd5d1f84aca731d7c2804f897a10fd1e36a34415870c73332247c1f483954cb2b5547c0f90533e5cd0', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-26T11:39:55.301858Z', 'cli', 'key_create', '\x2b992088d3ee8be55ec016a097153fc2eb14b602edc148c92a314c6162e14f5c', '\x0104329d7ba3133ce236ccd4eb40ce942383307a4ea1277455bcd35a7130d517906eac9ba26bae15c9b085c2ac', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-26T11:39:56.958573Z', 'cli', 'key_create', '\x0fc679cf055ba9ef7a84663c9b9f6c14df1394e975538952aa09bd7714bfda27', '\x01205c109d93fa6c76b306ccbb35b6474867ac69f8c5adb2d43001fabb3c39966dd06f80bd696f4c8ca3fa7075', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-26T11:40:00.013427Z', 'cli', 'user_add', '\xfb33e8f936fe2386f1e1350a4e2d834574b1435c06424285ce79851d826b2e83', '\x01a1d3afdfa07d2ad35d1e906c9aa749f7efec1945527fafeedbbae29dba522e8df7d7b1ba76fede972728f73003e36c43d7b216d4c0a57efb70cf17f8', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-26T11:40:01.745195Z', 'cli', 'user_add', '\x3eb67315582ced8fa38f8e553e13838cfb66cc410666a4e5fe03fcdd9cd73386', '\x014d6d7c3a023241617ed011e30a3f2527b155f65e8e7f5ee300a9ba15a748642724c47688304f01ea4ea281bae9aadefe4da228029d51d6e076c35f0e', NULL, 201, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-26T11:40:03.528531Z', 'cli', 'membership_grant', '\x8cf68028123d85dce44c8a480491190118783a4e07c5502fd506ea141a6b773b', '\x0150f533150e36eecbec12432c450403e3435782aa486e75a5711a0ffd17096d86417bb8be6438403c9fef9ca38863c9de739a769a8ebaf50c6a1051f1162d52adb873c809c289a2f0354bf53459d1a4ddd292198449093758ec59e491f9fd8fe698231504', NULL, 200, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-26T11:40:05.364704Z', 'cli', 'membership_grant', '\x05485bcd88794a3bf5ece475a5b8828f3dfc241461e3eb76dd58bf4c840b85eb', '\x01365f1ca645399d767ac3b98a944ac5b22a3de6e184b5bcf46f545a87e759ff2a7d25632668546e834fa2080ea70093628d64afb4626eb285bc3a65b0e6f670cc11d8f2f41666647103afd090f7701475db9fdb1a0473e13435e62782f45719b6a21cde4018', NULL, 200, 1, false, '\x99d3ee5b8bfa4449');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-26T11:40:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\x99d3ee5b8bfa4449');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x99d3ee5b8bfa4449', 1, '\x01ee16723d57e2e03eb88055587951caed872b24f85e973608fc56b5f94797c61102135db15fa6aa8e00fa64e2935a38860f33c8f98532bf57af7af1f8');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x7ec44cb034dfa357', '\xa349c63b8362b611d1acea6ca60d9f25', '\xfc35aefd76cb2f8e685b169c7f3e5d8de3c9ec691b5e503ec9ac62255f85af69', '\x01a1756dd564b100db40bea2f2dbc90cbf01ab30e13a57feadea4a0514872874a9', '\x0139ec102149d7f4afda02f33109edc4f9f5c122eae05262e84120ba0a635dd4c5ef5e5bc4ab9ee2d252a281', 1795693189664758000, NULL, 1803469189000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xda0f5eb7e0b5e3b5', '\x54b706a00d2bd04602f13a627df3df9b', '\xfa83822a00578110d5054c9623395b00f5817eaa99dce9efa116438b0e40eca0', '\x013583ae9c53f63b98ef4b2f2bda6ed524c1be41fda605cda9ad42fb2a6965bd04', '\x012978425768548e17f7d194ec70786e0a5b77f891adf737faa3015353c04d80c6bfcbede303f073a42b57c6', 1795693195301858000, NULL, 1803469195000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x2d1352b46d240873', NULL, '\x9af582cb7c5a1870af01784213c8fb933a8fe83b3e32757365179ce239fa4661', '\x016dbbf6a87768dd5e3a0974e3f73294057f481b302c6bd2f0e10b1e3cba6740ae5f', '\x018e0febf47f4b14fdb979a64bf1d7069519fa1e63b59459bbc15bd9d17c2b18', 1795693196958573000, NULL, 1803469196000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xeb5cf27e08b0c427', '\xa349c63b8362b611d1acea6ca60d9f25', '\x58715dbd4d9a1a8241cc68490f49bfa47f95fa6856faf0b1197a4bb0603159ae', '\x01301939eb07acc2d514520d3d72dbd6f647e833b049b4dbce1950a27bbf30b0261d2fd13a823ea1a9793a5ee06a97e9d8d4f91323d0a5d630a0', '\x011399d4e06c1cfb29a2eeaefa23b587d308c9db7b3059f87f54046b73bc61013ce9d72d0feae706538e37602161', 1795693187867811000, NULL, 1803469187000000000, 1795693200000000000);
INSERT INTO vcon.api_key VALUES ('\x3d756ee725602f74', '\x54b706a00d2bd04602f13a627df3df9b', '\x45d64551885f06fa09087751254aaa76e27a01be81c77555ab6669ca5af2ab48', '\x01dcec867a1406cf18cead476591977f5c6b3a423abbcde61df3c9de50f0db3cfba1826eb3d504812b7f0f01bd654a6536297e51f7c832c106c9', '\x01b6ca81ea8b8ce91c0b898b69a65f524d52a56a0983ee61686be8a9ca777d164ab6edffdae29d7e9c7555cba92e', 1795693193457482000, NULL, 1803469193000000000, 1795693200000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 2, '2026-11-26T11:39:47.873426Z', 'cli', 'key_create', '\xc5e76253e1ebe3af6bd5d68b86fbadb8b74ab1fb03c4d7d9f5bb4f105592ac2c', '\x013a557b5db904c853f3f4a0a15f19634e4166cc06f5c33c8b6194046c0c705c19f2270d003152386d2f370159', NULL, 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 4, '2026-11-26T11:39:49.670116Z', 'cli', 'key_create', '\xcf151bf22aec6ea49985a8438e146095799983d17def12d33cd7a8b47865d6ac', '\x01c1812813128bf1001f9241184f041c9a8c1796698ebcb6f2a2ba49bb17de64aa6ca7f9bbe49e130d88c493f2', NULL, 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 6, '2026-11-26T11:39:53.463011Z', 'cli', 'key_create', '\x19915671c7cb69f5d8a98d46c7f1be9c2a5644ffae4f7becab2ceb9922e5f3f3', '\x010852dcf71dccb2cc21416f29818537837553bc3f3d51cb6db68bbf01349a48ea72502b14c4e312dae0e9233f', NULL, 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 8, '2026-11-26T11:39:55.308425Z', 'cli', 'key_create', '\x61f085e02fbab636788b84b631ebc7faa8222b6511cb16f42b8e2128203ed851', '\x015279c0005ba7d0b64e9a9bc8d0be9b527927f6447f2fb474a297edf9ea2f8c260969fdebae914bad8d80f360', NULL, 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 10, '2026-11-26T11:40:03.535952Z', 'cli', 'membership_grant', '\x02238313b1492270080c32ae524b15e857110653e877c8ebbc5dbf7a08a76088', '\x01dcb5c5561f1b0ee3d21eb60647fa6a37c1d09d57ea92b333fc1d180c8fb16bd7520550e6b6fe31b5f2c25df33463530d297bd42b3342d62377ba5b8a1c2c10706574b5e7cf039373fc2cce63f6d2d2b314c2dcc024ba1f1ea3299dbc4da42d4aaa343656', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 12, '2026-11-26T11:40:05.371331Z', 'cli', 'membership_grant', '\x8c846f736f842819a2460231bfab2ab5da58f45244623910b988025b7c87853e', '\x014ba4ff1076a1e1959a5d5a387c0452f9cd9018471c3988d44db565406a3fd240c5d88ad5376ded14f11a42ac544b03cb147f9eb642c6388503f5fea4a8d92e9c0ff8eaea85fef7b301d16918f8752cfa27dc33567a851f1ab337cd50d9dd92979991ab6b8f', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 14, '2026-11-26T11:40:07.741715Z', 'eb5cf27e08b0c427', 'create', '\x5804d057460e980e4b49354a371e6098bf6733f27050aa9a8702812cd1b431a0', '\x0132b97c4cacb78b77ec5e24e577eb72a40e681799dbc33bbbc91c252682a462b19a67d995e491143065c11d3d5cc42c30684355c3fa5ec13347ac81f6a439932f', 'bfd39f58e4ebd92141af728660727e2b', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 16, '2026-11-26T11:40:07.798458Z', 'eb5cf27e08b0c427', 'create', '\x95bda802986bcfe10ba4794d93def77de11530970180c817d122fb87cda8bedb', '\x01ec5f800decf819e8a955070a3f798634c7d3e24d9e3016c0f2d73023fb878cc4908c3759fa9cdffb0d46bf9ec4e96bacacba1678b8b03251cd09316552143678', '6246bc2083a7353a50d150438d224096', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 18, '2026-11-26T11:40:07.821544Z', 'eb5cf27e08b0c427', 'create', '\xd079795dfd631393c4b898a157794e7c6b65d5dc0f15f06dba3ae4f4a7448284', '\x01b44ff0bbbda9521a40a0a93893e68ff1a73e6efedf2407995811cc9171bf507d6446cdd7087e18f1db0b1fc238089c1171e5627f6762acdf3e478a83c615c8fb', 'f097132ee6dbf7cebcb9354a78252957', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 20, '2026-11-26T11:40:07.843832Z', 'eb5cf27e08b0c427', 'create', '\x97be58cd34f9d3d2c72db1ba9b868a376c004a94f6613eeaefefbba7de1ad032', '\x011d3f6e3ebf4f17ab1b396eca5d21a13bfbdba0a895a584fe0a9c40098ac6c25b9ff566b97256f0c7f70e2eea10d16645b1c1798a694bde36e024a8966fa087d5', '2bf236e1ddd779e057896707f00358c2', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 22, '2026-11-26T11:40:07.866618Z', 'eb5cf27e08b0c427', 'create', '\xc0ed995b15768d249678a8293ab1e2e2412e8a023f802489379b2d354ef43eba', '\x01ba21db35d357ec48a3cd2f6e7dccae9b972808afe35c24ff11e5355974885368716a58e40d9f459859cf8f265f2b1cc63f083b3427f44e498e29a420b0f555c8', '7f957b5192b5dd9fc54f4e23c30f1dfd', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 24, '2026-11-26T11:40:07.892236Z', 'eb5cf27e08b0c427', 'create', '\x9851e2288b637e0e44e489b5b80e583dae21a8529e70e0317c0e381e39283d88', '\x0133f8a7e2a40f75530bbaddc1559421a66083bf4b69095d1b6e2aedf5c23656e352568273cac92349e0a0fc89a98235975389de1491fb7bb5e1b7f3bb1b5944bf', '7221e8e6318e893f7868a47a05a30a58', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 26, '2026-11-26T11:40:07.914204Z', 'eb5cf27e08b0c427', 'create', '\x7860bb531041861cd32199a284d997cbc971817d2f7df3c68031fa691fc6c664', '\x0193b698530235099b8f270211efa3c63a47429dedc6db6374e0f0ac4965c502d2cecf6591908e48a4837398d6e8f9c07c7c794d92ee224aaca272292813d9774c', '92a8f5107a4441ee90de882b1466bd18', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 28, '2026-11-26T11:40:07.963884Z', 'eb5cf27e08b0c427', 'create', '\x94bcf0c719ddc45c61fe9b236dd461e4a744004d4c60a0902f8cb51845d54e83', '\x01afdbb6847d16c452b0f297d6c89f1b10477e42e196202ae584de9527ca06737c6a64467cf4bb1ab3323498b5ea8529f1ec9cf882fb045b59848991b6650e3134', 'ea904d7aa2fe8bea8098b254dcee5d05', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 30, '2026-11-26T11:40:08.012771Z', 'eb5cf27e08b0c427', 'create', '\xcaf33f2f9a2df1087eb7364bab2362b8ebb2d9f9797408a279b302233613f86b', '\x01175bb1712089e1696b66a68ebfa1171b75b5dd4c3dda463ece04ed21f9a1e444261671caa5e4a00a9f4c04c488ba28660efab316543c324bc3369d3076338393', '61d71f7586e9fed62b558458439dc9cb', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 32, '2026-11-26T11:40:08.04725Z', 'eb5cf27e08b0c427', 'create', '\xbd356548a66cd3a35a036b55a574ea924dc95bf4e8c21799f15c460ed44e41b4', '\x0170f494cc94718a5fb071ebe4a32066851f6fd55280795a7097c6ba28788a8f6768fc7ac68c01d1d46839541a2da56cc8e3fd1464d97b8ac2f001be78297eacfd', 'b5edeb6ece5f8b963c560ea2eea4bc8a', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 34, '2026-11-26T11:40:08.093598Z', 'eb5cf27e08b0c427', 'replace', '\x5804d057460e980e4b49354a371e6098bf6733f27050aa9a8702812cd1b431a0', '\x012d4a66a236229af33848d28e20da57efd775defd561b58e9d324cfb551d7f33e8fb3087e6c9afb0efa1dabe75694974d25442263c55d56019478026572321f81', '8e4eb45c99dd81f2364038c46fe1c5a6', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 36, '2026-11-26T11:40:08.135065Z', 'eb5cf27e08b0c427', 'replace', '\x95bda802986bcfe10ba4794d93def77de11530970180c817d122fb87cda8bedb', '\x01879d03e24491e825a963f4efb80a42d20f9d2eb539398ca0f9d89bf2e4a9ede9514ab3fae534397e57741320e31bf37ae63af74217e47d95d48df56597964f37', 'a8ae3fd61d6cd83e7abcdc66dd9ba65b', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 38, '2026-11-26T11:40:08.141206Z', 'eb5cf27e08b0c427', 'delete', '\xd079795dfd631393c4b898a157794e7c6b65d5dc0f15f06dba3ae4f4a7448284', '\x016b71aa23b070a3218f00eb827558c3eb74d5aa7277eec53185e9b5691af3c7a9bd2c143a00c78a36563fbb251ac581f98c279d3067cb9622158ae843ec9eef2c', NULL, 204, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 40, '2026-11-26T11:40:08.147475Z', 'eb5cf27e08b0c427', 'delete', '\x97be58cd34f9d3d2c72db1ba9b868a376c004a94f6613eeaefefbba7de1ad032', '\x01eb579bba8de583eb1e68422b0678c81dcbdead383f47e7c038c02362708e946b85e69f7ded97bebb3c665db5a641a2fd55f9228255c6307f381cafe6ec17977a', NULL, 204, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 42, '2026-11-26T11:40:08.152799Z', 'eb5cf27e08b0c427', 'hold', '\xc0ed995b15768d249678a8293ab1e2e2412e8a023f802489379b2d354ef43eba', '\x01aa407e69cf055b8de4ab0de7646658a5f3965e1740d7e619b327d321652c5efce76c1e1df50bb02f671b9989b0da9b324a3e7715db9c3e3c403bfc5ab8149f76', NULL, 204, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 44, '2026-11-26T11:40:08.156328Z', 'eb5cf27e08b0c427', 'delete', '\xc0ed995b15768d249678a8293ab1e2e2412e8a023f802489379b2d354ef43eba', '\x01fe707b7fddadb760ba00cc45993d65e89a6c18b0b9fa52794e958581cc5e196c845ff25c0df6a192a50db2994fa3263b758023bc93cd602304e96b55070e56c9', NULL, 204, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 46, '2026-11-26T11:40:08.161167Z', 'eb5cf27e08b0c427', 'hold', '\x9851e2288b637e0e44e489b5b80e583dae21a8529e70e0317c0e381e39283d88', '\x01d4b4c20c5b2676f18bb2e96b6092cc7435a2b55590279e2a88f3d3af6245ebabbe5f8739c6306d0c2076c7f248769f5ad0aade66d2495b642f1f91f0a5542cff', NULL, 204, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 48, '2026-11-26T11:40:08.184708Z', '3d756ee725602f74', 'create', '\x58ca37ad5467fce9f861ce8068a4e24de3327e344f3135307e80f3fad6a763dd', '\x0104d68b63d097ce5283cf2e06b51d35f043062640fdab7cfe7126b5953c39880931c5f9c2d492d31800046833fda3d6e7b2947bb37c579e952a3eb75f59075e0e', '8b5ef1a8dc64cf16b88eeefe3892a6c2', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 50, '2026-11-26T11:40:08.230474Z', '3d756ee725602f74', 'create', '\x5af272e0e1c5026e513a267126335b420f6eb13c2df0d8a9632726c2b9b022cc', '\x01e1ca3f061bbd5ac986b810dae467a87b4fcced4f731b1039cd26f8529a4cfccfb5a60b96084ef77ebc2940ba8c824c1f4c43a7ab68d946e1d587539c6ec3d311', 'f48b7fe942ebe8c5a3ff9f35674c9742', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 52, '2026-11-26T11:40:08.251118Z', '3d756ee725602f74', 'create', '\x8a25765094daf43ae7aa207d6158dc9ce083cb77e59c91e5ae87b42c938d7f87', '\x01b9d8808bf9bb6466acc70f6c80b26f04476c0c89a61c2cda5e705337b28fa1bd9b17a7179a0ce05def3e9d317b003164d3806d6ad464dc3f3ab41ea5b8ed8f0b', '5ceaf7b849cb94b27df822fec9f2d850', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 54, '2026-11-26T11:40:08.282832Z', '3d756ee725602f74', 'create', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x0159d6c998dfdfff58b938e387e4ceb114ace7539eb343a8b5f24fd8f8f156e7ad2d8e51048ad0505c1d0e66662c5688234365abcd3b6fc46ece4c6072b7e23c70', '88ca692bd359b1cde20637d4b04ebdd2', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 56, '2026-11-26T11:40:08.30601Z', '3d756ee725602f74', 'create', '\x308458ccd4b0de18faac74580162e3018074efd83c51b7454241c09de9765273', '\x018d3e3af7d1416d7ed5413fa81b6f4250a8409702b0606e7a5799023a11386dfdc1603acbfd8109269f1152ca8e6819d1b7d3afe3c625ed0f64354bcfdabe1525', '717c8b6115289ec19f7f018f853928d5', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 58, '2026-11-26T11:40:08.331552Z', '3d756ee725602f74', 'create', '\x17c07f6b6d0e96f65f14613feda03eb723653067fadfd6ecf609542f9e9af4a7', '\x014a36823f893879d86dd7ca1d0c9a4eae70d5558e718df841f60c34e0f0f21e555f4bbc53c2040c388471e72333f938519df4475afc4d2c1a1f8c932383fc75cb', '8132c76dab5d75b36cd4c765555857c8', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 60, '2026-11-26T11:40:08.351417Z', '3d756ee725602f74', 'create', '\xb4ebb19f01276836bb307d178bfce65ffdcf1ee9abb585e283ad1e4585ba65bb', '\x01506cbbc0f0571d092d203f17b529b67f4f463bf982d690346aa7a4971eada59f7da331a306b360f3c3582790c8136ff803aae7bd7bc7f7404f28c10d56d50053', '9fca4ad107febee8c31b676b554b492b', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 62, '2026-11-26T11:40:08.382601Z', '3d756ee725602f74', 'create', '\x3d6c3e3425f976c06dd690a2ef5ec298f6b9c43ba7c8224e33e20a53080fbb78', '\x019b9be84f0bde7694dc1aff6c95238ef510fce879afb2c44896d5dd1de0f5213d5e6f2d1249ee71547f2d62afd74751d15d20711ce8f8241a2525ec0da9cc517d', '7202586671ad94f2e9a0cc84b6f97a2b', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 64, '2026-11-26T11:40:08.402576Z', '3d756ee725602f74', 'create', '\xb2b3ca3c403527258f691389a95333098c2d02d3f5d47304da989ea974f0b49f', '\x01b6d0cf0a09075ed6841feadbe7a08196e8ff1fa549c440acab45f92cc763eedc68dcb1472c3435de056884339427913d7d02512dbceb79741e7614fb37aa1e5b', 'bc456004050e07752009d9c4bf0f082b', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 66, '2026-11-26T11:40:08.425337Z', '3d756ee725602f74', 'create', '\xfe85f6d08696f1de3e12ff1d17059212ab34ac200cb603e94603e00eaf0e12b6', '\x015918ef0ed099c20a0d25d525df351202b5b037c432d52e678fd20f629d523c0403cb30786dde1b101df08be381478631245dc81d0ff8038f699e73546dcbda47', 'cbe4df4ff51e700d57b2b3886c586a15', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 68, '2026-11-26T11:40:08.457325Z', '3d756ee725602f74', 'replace', '\x58ca37ad5467fce9f861ce8068a4e24de3327e344f3135307e80f3fad6a763dd', '\x01bdbbf6a2380fa0b3a0bd787ad1308266bb387275b79a291bf122ceda9347c5645782ab25fee3379ab4527dd052ad274609b6077ca70777b61a0c7a47008ee092', 'd91ddf8fd2309dc4ed2fad69cfa8c694', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 70, '2026-11-26T11:40:08.462846Z', '3d756ee725602f74', 'delete', '\x5af272e0e1c5026e513a267126335b420f6eb13c2df0d8a9632726c2b9b022cc', '\x0105800e0fb023aa4d82d9cb6a650f70aaf268264715c73d7b12def944e10c2b8ba6fdcad56c25455b23f3b738a70f4fa7f3afcef0f14c6089089bb6021dedc647', NULL, 204, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 72, '2026-11-26T11:40:08.46464Z', '3d756ee725602f74', 'purge', '\x5af272e0e1c5026e513a267126335b420f6eb13c2df0d8a9632726c2b9b022cc', '\x01afc72eec448e270be86a16debf69844c54fac5c51bf4bdd72aa120c8e6971fd935573c2245057d10058a463ed46b35dd70765cc2ec9c3b43d2e53d70f411dbcf', 'f48b7fe942ebe8c5a3ff9f35674c9742', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 74, '2026-11-26T11:40:08.563569Z', '3d756ee725602f74', 'hold', '\x8a25765094daf43ae7aa207d6158dc9ce083cb77e59c91e5ae87b42c938d7f87', '\x01326f303a254ca6295a7de2b9617d534cfb4c20f8ed0fa754263b9cdc3c6de8e105a8f4e1a335bc0415e60db306812aff216307e7058108fea7a059320135870c', NULL, 204, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 76, '2026-11-26T11:40:08.569367Z', '3d756ee725602f74', 'delete', '\x8a25765094daf43ae7aa207d6158dc9ce083cb77e59c91e5ae87b42c938d7f87', '\x01e068dbe1a0e703a62f6db4c48d443f0adf6beb13d74d78d9f5cade736ede7a68cc9af918f4a925ad62bad3551a69a07403863dc0805e7a56a5fbf0b8407573c3', NULL, 204, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 78, '2026-11-26T11:40:08.575883Z', '3d756ee725602f74', 'hold', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x01df9e0c74dffa2ea7b24d5b980423fe2194de5cb3522d7f691624302cb5bdabc2dc01f3d8e90d29cd84a86e75613e7f8e7fe1624a80e73f54cbc502743292d226', NULL, 204, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 80, '2026-11-26T11:40:08.619927Z', '3d756ee725602f74', 'replace', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x015942c96adac6e9fcd4470d472bf6fd6bb47ec606bae2d6295c30a4d845468b1cce3ace6e6ae00b76c24e74e1335c78c3d6e5fec2b34d829a620bee322e3ae4fa', 'ff4f5c254e8377a713f39bab49dee7b4', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 82, '2026-11-26T11:40:10.979428Z', 'sweep', 'purge', '\x58ca37ad5467fce9f861ce8068a4e24de3327e344f3135307e80f3fad6a763dd', '\x01bed926efc750abbebf9f46b59112ea4c3e55b4556df8f345de47f0dcacb790f6290ac588bffcfa2f032ffa870e039ea70259ac1baeca128bb057493724a6b770', '8b5ef1a8dc64cf16b88eeefe3892a6c2', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 84, '2026-11-26T11:40:11.336575Z', 'eb5cf27e08b0c427', 'legal_versions', '\x5804d057460e980e4b49354a371e6098bf6733f27050aa9a8702812cd1b431a0', '\x01b6c86eb4405e9d9a19cedeb405fdecb13393fd84bd7c8a770bc84dc149d2deefafe201b183376014620fe39c65d68642f1f9ff4666f0579befa9813244c22c39', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 86, '2026-11-26T11:40:11.340808Z', 'eb5cf27e08b0c427', 'legal_read', '\x5804d057460e980e4b49354a371e6098bf6733f27050aa9a8702812cd1b431a0', '\x0188e48cb9171272c8ecdb21437ffbba57b8a3c3fbe90bef47e079e4180952ced2c65342565b06b86789a3be3879a9dd8dd4f836c54302e1308c4108093904d595', 'bfd39f58e4ebd92141af728660727e2b', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 88, '2026-11-26T11:40:11.349403Z', 'eb5cf27e08b0c427', 'legal_read', '\x5804d057460e980e4b49354a371e6098bf6733f27050aa9a8702812cd1b431a0', '\x0182de04a874cf1914b6865515d47c0514a73f95e2b2679b7989b7c3227f8d71e74a07372ab3481a1e6118c3c211524596f42210db695e738615cb95b401b5088f', '8e4eb45c99dd81f2364038c46fe1c5a6', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 90, '2026-11-26T11:40:11.354556Z', 'eb5cf27e08b0c427', 'legal_versions', '\x95bda802986bcfe10ba4794d93def77de11530970180c817d122fb87cda8bedb', '\x016694d1ed58af3d5f6a5b6508b94f85d4957b4d340654cb5464019c927b617a5e950253d4d458b31708ba6ef38d285dc9d0fc588c5540e36b7f2dd6b4ca7a1bf4', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 92, '2026-11-26T11:40:11.358107Z', 'eb5cf27e08b0c427', 'legal_read', '\x95bda802986bcfe10ba4794d93def77de11530970180c817d122fb87cda8bedb', '\x01ce47eff1056ea0974c97fc96a606077af8fdd8c0b0fe52b72031ca3a1e318019fd9c27a952f39d83285b8c80a10356b4d0da1b768aa3d05b28d0ff49c8426935', '6246bc2083a7353a50d150438d224096', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 94, '2026-11-26T11:40:11.363Z', 'eb5cf27e08b0c427', 'legal_read', '\x95bda802986bcfe10ba4794d93def77de11530970180c817d122fb87cda8bedb', '\x0188e011994aa206d80183b3cb8c40da7fb5e300fa351028ce1974140a63585111c214f00c41c958a3607e69238e8a2cfe20f51d3b2ad30e9d32352b3fadabcacf', 'a8ae3fd61d6cd83e7abcdc66dd9ba65b', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 96, '2026-11-26T11:40:11.367676Z', 'eb5cf27e08b0c427', 'legal_versions', '\xd079795dfd631393c4b898a157794e7c6b65d5dc0f15f06dba3ae4f4a7448284', '\x017ff9ef5077243f4aa38d461b31df68e1de50a57ba88ec85832b5ad1b14f940ae347c4080ce018d36251f510931e2f42e626acb050ea40801206d01053ac43ba3', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 98, '2026-11-26T11:40:11.371291Z', 'eb5cf27e08b0c427', 'legal_read', '\xd079795dfd631393c4b898a157794e7c6b65d5dc0f15f06dba3ae4f4a7448284', '\x0149f811b3eb603e8dc75aa7e63c514f8384e276a9cc350556b94e1d8b048250eb3b577e86d9a8a79bc8dbf32aa3fd1fe2c4c7cf90ba5919e0986d9c37ac59cfbc', 'f097132ee6dbf7cebcb9354a78252957', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 100, '2026-11-26T11:40:11.375986Z', 'eb5cf27e08b0c427', 'legal_versions', '\x97be58cd34f9d3d2c72db1ba9b868a376c004a94f6613eeaefefbba7de1ad032', '\x011f83a15718d80aef015b52c060c4b00f50ae4d71592027fd0bf8715e8b810ab99674c840ecf669a6dcd8a200b44b38e5ca9b740d954d883434719264d385ffe5', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 102, '2026-11-26T11:40:11.379189Z', 'eb5cf27e08b0c427', 'legal_read', '\x97be58cd34f9d3d2c72db1ba9b868a376c004a94f6613eeaefefbba7de1ad032', '\x01f4e622aa4bfdaf10d53a6b5c557337f30b723a456701227a799fe225a38841f3bc648c3078d797ea4e718f285d392178600155b05bce956e3b02c13a8d40de41', '2bf236e1ddd779e057896707f00358c2', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 104, '2026-11-26T11:40:11.383658Z', 'eb5cf27e08b0c427', 'legal_versions', '\xc0ed995b15768d249678a8293ab1e2e2412e8a023f802489379b2d354ef43eba', '\x01ad0a492d987927cc816d15ebf2012eee0028292253c52be37bceae33c8d222da77883baa4a2e707d853c57f8247af7e326e48635d5ec28405d0039ab8a58f879', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 106, '2026-11-26T11:40:11.386935Z', 'eb5cf27e08b0c427', 'legal_read', '\xc0ed995b15768d249678a8293ab1e2e2412e8a023f802489379b2d354ef43eba', '\x01e5f0564628c39141c839ef6a2dabb75c487c1edaa8bf2ddc31fdcd609a3f3b1171a84d67b80e7c88fa07ade28143b8f4c89cb30364fd5cff14ba92ece8de7847', '7f957b5192b5dd9fc54f4e23c30f1dfd', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 108, '2026-11-26T11:40:11.391493Z', 'eb5cf27e08b0c427', 'legal_versions', '\x9851e2288b637e0e44e489b5b80e583dae21a8529e70e0317c0e381e39283d88', '\x01850bd682757afa05e86f38878ca3602b6da60344825e9ab7fb5140045928b28a4ba30fff39b0f72103e171d446e6934f1faab3be0049db3884c79c333ab4d4a6', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 110, '2026-11-26T11:40:11.394935Z', 'eb5cf27e08b0c427', 'legal_read', '\x9851e2288b637e0e44e489b5b80e583dae21a8529e70e0317c0e381e39283d88', '\x01f7f5d14cf7f4e897ea71f7ee21ff7c76e0c03015ee6a2ccfe017a54c996ce0f76a4a05955f146ab98b05218b2b07d766b8bea2be5b09ffbf24c4a6a5a45ec155', '7221e8e6318e893f7868a47a05a30a58', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 112, '2026-11-26T11:40:11.399403Z', 'eb5cf27e08b0c427', 'legal_versions', '\x7860bb531041861cd32199a284d997cbc971817d2f7df3c68031fa691fc6c664', '\x01105b363f3bb21d4b90ffad878e275d2d8d288b2f7133e9daee07726821dd7609d04ba897df7403f0ac6de721aa3433d4bf8744f850d9f8f41c1c9f48966c758e', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 114, '2026-11-26T11:40:11.402522Z', 'eb5cf27e08b0c427', 'legal_read', '\x7860bb531041861cd32199a284d997cbc971817d2f7df3c68031fa691fc6c664', '\x01dc55aa6ae07c6bf1f4189a3f309cf3320cc9a1fa0ae44b769afe6d099dbbc06cd357b07ec63ecde4137907cfa9319023ddf3b454ff9e5440be3b477832d5dece', '92a8f5107a4441ee90de882b1466bd18', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 116, '2026-11-26T11:40:11.406903Z', 'eb5cf27e08b0c427', 'legal_versions', '\x94bcf0c719ddc45c61fe9b236dd461e4a744004d4c60a0902f8cb51845d54e83', '\x017b5b0a5b5db120be16d32a9f5627984b43416b50ca35149b8ca9ba402234159e7c2af7052410c84bb0f5c1e8710a230866bc946efa5e9729ce32b1c6a3d8bdea', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 118, '2026-11-26T11:40:11.4102Z', 'eb5cf27e08b0c427', 'legal_read', '\x94bcf0c719ddc45c61fe9b236dd461e4a744004d4c60a0902f8cb51845d54e83', '\x01c27a09ed852217cee29111b298682ac3914d9b655d5bf4dd74f92c0542e5f8d6503527b099c0dbec0ea42a59de3f879b293d0b7c1f768f59a526674de2af176a', 'ea904d7aa2fe8bea8098b254dcee5d05', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 120, '2026-11-26T11:40:11.414525Z', 'eb5cf27e08b0c427', 'legal_versions', '\xcaf33f2f9a2df1087eb7364bab2362b8ebb2d9f9797408a279b302233613f86b', '\x01ea9033931fb1af45de139f9da7ea9033937340a61874e5c0c408f00115a59d369f107e31c00290619cc925f00d73c6413a1c38643682b4cba3d570af4cc2cfe6', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 122, '2026-11-26T11:40:11.41762Z', 'eb5cf27e08b0c427', 'legal_read', '\xcaf33f2f9a2df1087eb7364bab2362b8ebb2d9f9797408a279b302233613f86b', '\x01de1250329013b251aa03be1ae3348158e0178f73af184057081102220994d3f23315102ca7170f71d5134fac824b727e00c94ea463d7f3b07b5ae45b08e451ec', '61d71f7586e9fed62b558458439dc9cb', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 124, '2026-11-26T11:40:11.422132Z', 'eb5cf27e08b0c427', 'legal_versions', '\xbd356548a66cd3a35a036b55a574ea924dc95bf4e8c21799f15c460ed44e41b4', '\x010a276d2b18b047fe6e9844c2945f282aa6976349f194bdc9ab478a672b4696141ac9599a2ea572222d3cad82c448135630918fa2642ff14d3f8d6361c275ebfd', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 126, '2026-11-26T11:40:11.425184Z', 'eb5cf27e08b0c427', 'legal_read', '\xbd356548a66cd3a35a036b55a574ea924dc95bf4e8c21799f15c460ed44e41b4', '\x01b8a666353647a7c13eb917aca9772f355518aad18752063ab26a63942752e82802279a760db89b1b6d1141f182fd671d26a46405e07f041d74cfd70f2ccd9eed', 'b5edeb6ece5f8b963c560ea2eea4bc8a', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 128, '2026-11-26T11:40:11.429567Z', '3d756ee725602f74', 'legal_versions', '\x58ca37ad5467fce9f861ce8068a4e24de3327e344f3135307e80f3fad6a763dd', '\x016e4ebc8939b079c59baa0ab182265c3eb7827f619bf90a20b0a4f5487b135f6b51939d0cc3fed121861560e1473b7fbe4920bf2ec8be188c524405263156d636', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 130, '2026-11-26T11:40:11.432581Z', '3d756ee725602f74', 'legal_read', '\x58ca37ad5467fce9f861ce8068a4e24de3327e344f3135307e80f3fad6a763dd', '\x018c0fe6c271a02dcb1568d5aec51ef9c390395962cf91a9d71e3510b90c1c6d89577d2d32350c0be9aecdab5695a429cb4ae97734c881a66b8b5c323d83105c87', 'd91ddf8fd2309dc4ed2fad69cfa8c694', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 132, '2026-11-26T11:40:11.436988Z', '3d756ee725602f74', 'legal_versions', '\x5af272e0e1c5026e513a267126335b420f6eb13c2df0d8a9632726c2b9b022cc', '\x01fe2b4d091f5c735143b86c6b17f1384121ecc39c5a99149312306daa41667c37dfd49d95d20da425895af7d58a559fcf188769810fb686420ca689886d023843', NULL, 410, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 134, '2026-11-26T11:40:11.439687Z', '3d756ee725602f74', 'legal_versions', '\x8a25765094daf43ae7aa207d6158dc9ce083cb77e59c91e5ae87b42c938d7f87', '\x01bd17f97c2e52983471b5795b14c62016c9b140f67d00db30cae8a8ae62f375074fb445edf4e6d9766e411fd28e79f4e1e2765a8db903a041f1a67206763ed60a', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 136, '2026-11-26T11:40:11.442622Z', '3d756ee725602f74', 'legal_read', '\x8a25765094daf43ae7aa207d6158dc9ce083cb77e59c91e5ae87b42c938d7f87', '\x01dd1c88e1605586be12e7e7c5d8c500af7eaeb3d00b240e4776987baca07e232fbb37dcfe9736fc3dae986ded6973dd5fbb30e36a583b2be898f1d77d65a932c0', '5ceaf7b849cb94b27df822fec9f2d850', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 138, '2026-11-26T11:40:11.446832Z', '3d756ee725602f74', 'legal_versions', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x01604321edbfbef3fcb4c0933984d64a467d8fb3350b670293f4f4c31a812adfe53bd8885cfaab38e436c90438799bfc22f135fbf18c466fec73158da4267598e5', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 140, '2026-11-26T11:40:11.449733Z', '3d756ee725602f74', 'legal_read', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x010f84d4f87161e9ecaed982b15717383b99ea3dd66869bde9c00b44ff8625bcee4a546e4633dd28efe82d2a43cc55b933f5399ac3d312363e932e32bb1a29375d', '88ca692bd359b1cde20637d4b04ebdd2', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 142, '2026-11-26T11:40:11.453971Z', '3d756ee725602f74', 'legal_read', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x019a19c0da8e91a4b0228e2bf6eacbc9dab7cf229c02af3437d3ee66e9a21682be9ece6ec7ea9780fa64cb9692441116d000f2b1d69ebafd18f219236ae0d0bc7d', 'ff4f5c254e8377a713f39bab49dee7b4', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 144, '2026-11-26T11:40:11.458472Z', '3d756ee725602f74', 'legal_versions', '\x308458ccd4b0de18faac74580162e3018074efd83c51b7454241c09de9765273', '\x012fa9706d6d40eb1c21ede3db0ee407c24993b82b36e84fc06fd528c9cb7dbe7f3786b929b934eaa87c552affb3f9184645b495a419bb078893822b8f155b00c1', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 146, '2026-11-26T11:40:11.461636Z', '3d756ee725602f74', 'legal_read', '\x308458ccd4b0de18faac74580162e3018074efd83c51b7454241c09de9765273', '\x017b21d50c02113f8ce15cac712897c4af2d4bb4ad07edfccfd17a3bd682a2f4409cd17c97f912bb6afac7a9f4472a13513da93f75bc2789f2664360daf907b156', '717c8b6115289ec19f7f018f853928d5', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 148, '2026-11-26T11:40:11.466155Z', '3d756ee725602f74', 'legal_versions', '\x17c07f6b6d0e96f65f14613feda03eb723653067fadfd6ecf609542f9e9af4a7', '\x010fe24afc9200ab220cba53121b17facdea3ec54eab53b9b848305a73663da9eeb76b5b2cec9d547b616427f9d9505f14765be15bb3542cf461ae8a8877ad9736', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 150, '2026-11-26T11:40:11.469097Z', '3d756ee725602f74', 'legal_read', '\x17c07f6b6d0e96f65f14613feda03eb723653067fadfd6ecf609542f9e9af4a7', '\x01ced78177a190dd46a07a255bb5dd63f653b5823c7125eaf3029e401b44cd874278f6b592c8e9909c6820f0035f5316667927411028bbde6465f52c0ba863974f', '8132c76dab5d75b36cd4c765555857c8', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 152, '2026-11-26T11:40:11.473271Z', '3d756ee725602f74', 'legal_versions', '\xb4ebb19f01276836bb307d178bfce65ffdcf1ee9abb585e283ad1e4585ba65bb', '\x0191682c898b2f1d54c2725eb1b36acde48889f249a162b30065d484284b4e75bbe35754fcdacb9760cc6863cc18a51a155e26b4770f9f93ecb8396772bb05919c', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 154, '2026-11-26T11:40:11.476258Z', '3d756ee725602f74', 'legal_read', '\xb4ebb19f01276836bb307d178bfce65ffdcf1ee9abb585e283ad1e4585ba65bb', '\x01debbe9c89041afc6fedfb31bb8c281e014f9c9648a1b400d18eed3bb00b2adb361339b87cde07c46ecad6b467d301c0c99dc54dcc46126f35e0f9665f090e96d', '9fca4ad107febee8c31b676b554b492b', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 156, '2026-11-26T11:40:11.480316Z', '3d756ee725602f74', 'legal_versions', '\x3d6c3e3425f976c06dd690a2ef5ec298f6b9c43ba7c8224e33e20a53080fbb78', '\x010e3838e2c82fc3d8006e7f0ebac0e244b196682b86d9a299ce4fd609ed8add4304fbb7680169af2d30e34563e26f2151901d6ebd5c2d74db27912f68d7a30ffa', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 158, '2026-11-26T11:40:11.483232Z', '3d756ee725602f74', 'legal_read', '\x3d6c3e3425f976c06dd690a2ef5ec298f6b9c43ba7c8224e33e20a53080fbb78', '\x019c8dc720be662ee609897f60703b839e8020795981b8e059eb46a8c411c2ff5ee312820ad38ac37575f09969de6c19273ce4ffd590c242e1337f117b978d23e1', '7202586671ad94f2e9a0cc84b6f97a2b', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 160, '2026-11-26T11:40:11.487343Z', '3d756ee725602f74', 'legal_versions', '\xb2b3ca3c403527258f691389a95333098c2d02d3f5d47304da989ea974f0b49f', '\x012152bb76a862127b2d838192b7b27873abb4af952b3d879aeee32ea0caff8b2405054a4268d0120fe850ff6823672c31fe08102cde8519df32c1fb756b1e96f8', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 162, '2026-11-26T11:40:11.490217Z', '3d756ee725602f74', 'legal_read', '\xb2b3ca3c403527258f691389a95333098c2d02d3f5d47304da989ea974f0b49f', '\x01d002a75739527b5d2e8c46346135f03bef0605c83fc4b78099fe0b7613ec8687d2eb4057212c85f8bf8ce30e22e85946f1503e8284c0b2f128eb7b8297525b1e', 'bc456004050e07752009d9c4bf0f082b', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 164, '2026-11-26T11:40:11.494698Z', '3d756ee725602f74', 'legal_versions', '\xfe85f6d08696f1de3e12ff1d17059212ab34ac200cb603e94603e00eaf0e12b6', '\x01d3cb9b06c90147cecf8e795fe4d58fef4453daf6e35117271c2efb26a253cd09576707af20308d7766508250b3b69a3f95edc63cbad257746d99ebad11145f9b', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 166, '2026-11-26T11:40:11.497994Z', '3d756ee725602f74', 'legal_read', '\xfe85f6d08696f1de3e12ff1d17059212ab34ac200cb603e94603e00eaf0e12b6', '\x01c214aeba8dcc06ee7a3180d7c0508f95e29d04550942b013337c82603ef038dbaddba3bfa4272694a67c4dfc3d6f092bd230767b686c8849b8bcb6b5788c2345', 'cbe4df4ff51e700d57b2b3886c586a15', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 168, '2026-11-26T11:40:16.136702Z', 'eb5cf27e08b0c427', 'create', '\x568ebb1405cdb605f15d10618e90d988c2a3dccea694a25a2a592dd1a3043444', '\x01eb4ed28a3f2fe8cf18c74b78c4b748d695f5b4b2018e62cbe0a7da2ed1b7976fcdcc49cc4b43127642565d818158d2a568667a93f3b040ec4bfdbe17e6c6d8eb', '6c1962aba5d06fd7ee758ec7ebb7a9b0', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 170, '2026-11-26T11:40:16.169338Z', 'eb5cf27e08b0c427', 'create', '\x27fd44e1804f914a12aa9c42f721d67ecfb9cd7dc3378b3d3ca2f3276ccce9a0', '\x01a7e91e6abcf94a68ed16a8d6b94e190d32249d28a48ee2e4fb8dda5ac2d61c41f3056d0ff28dbf8427eb1bd0b3a3f5f5604137a85543b6491b047ec9da6f5e9a', '88f6a6b60abe2c0d0a53533638a82e19', 201, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 172, '2026-11-26T11:40:16.188764Z', '3d756ee725602f74', 'create', '\x6e2d05e38736dc6b98b1a39970a085066bfa822b92fd6ecc852246bb58dde22b', '\x0112d38f847a2bc006d1be90a720bc86179b9d141f38923ba0714329ab4da1136ee4c447710d4d90133756cdc978625739648adb667981adb6b88fb7a64ed0eeb3', '0fc04bfcfcb335fa24e353f52421efd2', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 174, '2026-11-26T11:40:16.212923Z', '3d756ee725602f74', 'create', '\xbdd4c10c4a87ac5333a30a31f96d4982f2f1cfb13b50a14715cbd213b3e3642a', '\x014d03e49535466d8e06cf891d72ec42b6fc70b578ca9f54d2e1da1542b0c2faa47722cf98356ec9269fc35b06fce0ea38ed8272ead5aa2ffa056278d3b8e23317', '6adc66621df985dea6c41d7852f6df60', 201, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 176, '2026-11-26T11:40:16.217401Z', 'eb5cf27e08b0c427', 'legal_versions', '\x5804d057460e980e4b49354a371e6098bf6733f27050aa9a8702812cd1b431a0', '\x012b5650fb96c6e9734978c1bb26c35787686fc6fcbb87f9f6a69f7a224a95b5e8efa053886595a68bf5b1019c68495d52312b04fb9a9fa3d751e31ef06483a328', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 178, '2026-11-26T11:40:16.221256Z', 'eb5cf27e08b0c427', 'legal_read', '\x5804d057460e980e4b49354a371e6098bf6733f27050aa9a8702812cd1b431a0', '\x014287b525c018cbe80adde74368d52da8513e28448abcc68a8c68f24cf4bb70dacc4f5ed849ff158c0c367d20042e9f26fd77eeffeeb9c19f83692fb64417f1e5', 'bfd39f58e4ebd92141af728660727e2b', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 180, '2026-11-26T11:40:16.2285Z', 'eb5cf27e08b0c427', 'legal_read', '\x5804d057460e980e4b49354a371e6098bf6733f27050aa9a8702812cd1b431a0', '\x017d5a36514586b9cc3e6c644daefa403353f75408e54bfdcbb2b3369d6f793efe61d335a35ec6935fe874e2b880946a54fcb8e841f3c2bfbe55487879245ea85c', '8e4eb45c99dd81f2364038c46fe1c5a6', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 182, '2026-11-26T11:40:16.233194Z', 'eb5cf27e08b0c427', 'legal_versions', '\x95bda802986bcfe10ba4794d93def77de11530970180c817d122fb87cda8bedb', '\x016687fa9525028518cbe70006700baee75cd19820b93363e266633ee0713d1de94d076605c04e90ff8fd68925d66e13ef567a8722949508c4be38123980ab8497', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 184, '2026-11-26T11:40:16.2368Z', 'eb5cf27e08b0c427', 'legal_read', '\x95bda802986bcfe10ba4794d93def77de11530970180c817d122fb87cda8bedb', '\x0178b33bab5ac57e70e7f4e21570090e1cdcd46004e4e0f1a92e03a5b18b729c091b433f04f04d273a0011ce023f01ec254f71f501924461e7af0f6fe76f86b5dc', '6246bc2083a7353a50d150438d224096', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 186, '2026-11-26T11:40:16.241255Z', 'eb5cf27e08b0c427', 'legal_read', '\x95bda802986bcfe10ba4794d93def77de11530970180c817d122fb87cda8bedb', '\x011921d2090e9ff739977d0fce4cc386724d8c579dd3eb616c216eba8be4427a1119001f14c85ba4369398c8573b9dd609984556b5b9eafcb43eba54c7b3cd08ad', 'a8ae3fd61d6cd83e7abcdc66dd9ba65b', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 188, '2026-11-26T11:40:16.245991Z', 'eb5cf27e08b0c427', 'legal_versions', '\xd079795dfd631393c4b898a157794e7c6b65d5dc0f15f06dba3ae4f4a7448284', '\x01215160c18257b51a4559d62c1630b8863e26cec9b23503a1ed1b45219c1d776f0826e870d2f62979223bf2ba1ed89e611dbb73934a0d82c6c6dc28a59dda3924', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 190, '2026-11-26T11:40:16.249299Z', 'eb5cf27e08b0c427', 'legal_read', '\xd079795dfd631393c4b898a157794e7c6b65d5dc0f15f06dba3ae4f4a7448284', '\x01254fc81ea0b350b1d8b632e4e4ff8e720dcef08753d7c29f93bf07302f61e63eaf6d065775ef8a2d74562d35b142ebf3e75ec27cd900365d4508d98650712079', 'f097132ee6dbf7cebcb9354a78252957', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 192, '2026-11-26T11:40:16.253701Z', 'eb5cf27e08b0c427', 'legal_versions', '\x97be58cd34f9d3d2c72db1ba9b868a376c004a94f6613eeaefefbba7de1ad032', '\x016e722fead8bdda2b8101dbf638da51931f9ec7f649851ccaa08acc846b3a677d6fb98e17de81683b69802fc67e69a7cf5ef7f01eea3728a852f77c1c8675accf', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 194, '2026-11-26T11:40:16.25693Z', 'eb5cf27e08b0c427', 'legal_read', '\x97be58cd34f9d3d2c72db1ba9b868a376c004a94f6613eeaefefbba7de1ad032', '\x010e3aeeb61e41d1bd7fa939a9a9f73f38ad9497b637a55249ce8f223bcc56b9171302aad934d125c8b034c47276041cae0b6209daef28a4eaef6426f77ee13424', '2bf236e1ddd779e057896707f00358c2', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 196, '2026-11-26T11:40:16.261171Z', 'eb5cf27e08b0c427', 'legal_versions', '\xc0ed995b15768d249678a8293ab1e2e2412e8a023f802489379b2d354ef43eba', '\x012421f47b787426943d588f38b860537ae5033b65fbded470bd2a7165b5fd3944620041d06171db2994bf83fe0f5e8f0b63c51a12636bc55f850e10488f8c3e19', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 198, '2026-11-26T11:40:16.264773Z', 'eb5cf27e08b0c427', 'legal_read', '\xc0ed995b15768d249678a8293ab1e2e2412e8a023f802489379b2d354ef43eba', '\x017d504366cce8e09aa691f1a730125709915497301f6a6b044acd9752e73f3c0da23828887163cba6f643b30fa19b736aad9303d5889b919e80cca39de3fdf5a8', '7f957b5192b5dd9fc54f4e23c30f1dfd', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 200, '2026-11-26T11:40:16.269177Z', 'eb5cf27e08b0c427', 'legal_versions', '\x9851e2288b637e0e44e489b5b80e583dae21a8529e70e0317c0e381e39283d88', '\x0197b7812e3349df95d46053db5a35d7b4fa953d72ff2aec8d3582643bdbcf215f54f0ff4cd3d0d178092c281843f62ec4a625afd9877e1cfc6dab1de930eb2b62', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 202, '2026-11-26T11:40:16.272397Z', 'eb5cf27e08b0c427', 'legal_read', '\x9851e2288b637e0e44e489b5b80e583dae21a8529e70e0317c0e381e39283d88', '\x01cf620371c525ad0871c2bfa491ab9b0c84e5b0ccab9cbc5fd851d0256b319bce41b61c4bcf0e499dd9a0f411a97c63c712f4219e5c90f8e9a6a8e04db1840c95', '7221e8e6318e893f7868a47a05a30a58', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 204, '2026-11-26T11:40:16.276611Z', 'eb5cf27e08b0c427', 'legal_versions', '\x7860bb531041861cd32199a284d997cbc971817d2f7df3c68031fa691fc6c664', '\x01e93b830562d8b18c3a1470a7117fbaaa887bd7fe0deee7de20fcb4c655bfbcb5cdd7c438ad9e9adf18f906dfd7b4016e3084fda4ad381f9ae20efb62c78efb3c', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 206, '2026-11-26T11:40:16.279708Z', 'eb5cf27e08b0c427', 'legal_read', '\x7860bb531041861cd32199a284d997cbc971817d2f7df3c68031fa691fc6c664', '\x0122db73ffea9b393fb09df3deb722938270a68e3cf51b5085ac09b52bd1f45911fe1913c94ec2782a088ff04327de2554185200289acc281c912e8f1880e600de', '92a8f5107a4441ee90de882b1466bd18', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 208, '2026-11-26T11:40:16.283805Z', 'eb5cf27e08b0c427', 'legal_versions', '\x94bcf0c719ddc45c61fe9b236dd461e4a744004d4c60a0902f8cb51845d54e83', '\x01059af7525b7600924ff0a3be8fc7e16260b363a09bb38286c1e3676555dbe1660348e6ae9a8bf87eac4301a05ff13aaf8eefb7994d06df7fa9060333f1142356', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 210, '2026-11-26T11:40:16.286957Z', 'eb5cf27e08b0c427', 'legal_read', '\x94bcf0c719ddc45c61fe9b236dd461e4a744004d4c60a0902f8cb51845d54e83', '\x01735d994bb4e004d170d90167f29411aba9abcb6868dc6f92439bbbcb5519e990eab2723f144a6ec3eb67088d621eaddb270999518a15b88df847dc62de7acc0f', 'ea904d7aa2fe8bea8098b254dcee5d05', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 212, '2026-11-26T11:40:16.291079Z', 'eb5cf27e08b0c427', 'legal_versions', '\xcaf33f2f9a2df1087eb7364bab2362b8ebb2d9f9797408a279b302233613f86b', '\x017f281359ae4e7eec94d7ae0491682728f2469794f579ca3caf865ad1fb8c6c19bf43edd076f08c6a08c0bdfef068babc9cd34ead3f43a66f7dc1c1147fec4cf1', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 214, '2026-11-26T11:40:16.294168Z', 'eb5cf27e08b0c427', 'legal_read', '\xcaf33f2f9a2df1087eb7364bab2362b8ebb2d9f9797408a279b302233613f86b', '\x018fa4f25bcd7f4d7a56f21bc8dd8240ea160141f823178ee5a379cbe8f1d822a008697796b94c97fda0313181158b23ec51a0728a03938551174b5612f87a9607', '61d71f7586e9fed62b558458439dc9cb', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 216, '2026-11-26T11:40:16.298522Z', 'eb5cf27e08b0c427', 'legal_versions', '\xbd356548a66cd3a35a036b55a574ea924dc95bf4e8c21799f15c460ed44e41b4', '\x017d05be2bd6837ae305f606777dbac1800d785fe092c5f52dc9f4a7c07b59caedca5bbe2a0b254638d21bf48084a48cb1edb95ae919d56a7edb249c8fd78ee906', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 218, '2026-11-26T11:40:16.301594Z', 'eb5cf27e08b0c427', 'legal_read', '\xbd356548a66cd3a35a036b55a574ea924dc95bf4e8c21799f15c460ed44e41b4', '\x01d00db8a8504501ec6446449abdb68e2ce70aa84647b4214ef2de328f726a427b4e0f97a48cc28ad17694ced09f756833791d4bd3907fcba884dba8d540327731', 'b5edeb6ece5f8b963c560ea2eea4bc8a', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 220, '2026-11-26T11:40:16.305955Z', 'eb5cf27e08b0c427', 'legal_versions', '\x568ebb1405cdb605f15d10618e90d988c2a3dccea694a25a2a592dd1a3043444', '\x01755265e53fa72411d7d70f1c4f75943eb024413cdadfc55fc9d6a098917384e637598e72924ed39eb7c72aad78de3238a3645b66cc5cbc8e9ea2b5d1ac4c7ef7', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 222, '2026-11-26T11:40:16.309996Z', 'eb5cf27e08b0c427', 'legal_read', '\x568ebb1405cdb605f15d10618e90d988c2a3dccea694a25a2a592dd1a3043444', '\x01d9b9bf71d770ccdeb9e3d0008d0827bf0f8793b6c753c9c08bfcb8744508d39c77d4687bfa25859213b25f66b77111bcbb596ce7b616fd54c12cc754886a2966', '6c1962aba5d06fd7ee758ec7ebb7a9b0', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 224, '2026-11-26T11:40:16.314503Z', 'eb5cf27e08b0c427', 'legal_versions', '\x27fd44e1804f914a12aa9c42f721d67ecfb9cd7dc3378b3d3ca2f3276ccce9a0', '\x01bdf59e218312dd9983d1b2172d99dacf506400c1b505755767ebc3351ec317dd04c8edf0aeccf0a90cf85374524473a1974abee24e5f745fd8176a0adeea3a55', NULL, 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 226, '2026-11-26T11:40:16.317599Z', 'eb5cf27e08b0c427', 'legal_read', '\x27fd44e1804f914a12aa9c42f721d67ecfb9cd7dc3378b3d3ca2f3276ccce9a0', '\x01bd0eba8404b749b1c9042dc82f55783f7e8959ab271d0b2b51493e8e17ef895aa69ee903509fc1ac4660effdabcf012502e532b1fb3b3e83d6d5a23bbb91aaf1', '88f6a6b60abe2c0d0a53533638a82e19', 200, 1, false, '\x20b364ed34c70b40');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 228, '2026-11-26T11:40:16.322624Z', '3d756ee725602f74', 'legal_versions', '\x58ca37ad5467fce9f861ce8068a4e24de3327e344f3135307e80f3fad6a763dd', '\x010b96de4d3ae9ffa27d2b894caae4a938dfc8c0fd877859f86b592e2aa8a9954ca8d839418c669d5a07eb1a2b222da4c35ed3eb95bafd2b961c96073b5a8f3600', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 230, '2026-11-26T11:40:16.326325Z', '3d756ee725602f74', 'legal_read', '\x58ca37ad5467fce9f861ce8068a4e24de3327e344f3135307e80f3fad6a763dd', '\x01dcd367920cb81be882366792a7ca0fef89180747b5baf90da60f85313393665ffaa4b0476ed134459c0c399f6199b7cab427e070bb018dc4585556f85b8e1f31', 'd91ddf8fd2309dc4ed2fad69cfa8c694', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 232, '2026-11-26T11:40:16.332039Z', '3d756ee725602f74', 'legal_versions', '\x5af272e0e1c5026e513a267126335b420f6eb13c2df0d8a9632726c2b9b022cc', '\x01bc87778cc0384165ec860f7099946b04aecfa9ae84482ea27d583080d4b21d38eef49a59b2a2df7b71a8b8cacfb80d843198df92a03aaeee181392e67b41f48d', NULL, 410, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 234, '2026-11-26T11:40:16.334887Z', '3d756ee725602f74', 'legal_versions', '\x8a25765094daf43ae7aa207d6158dc9ce083cb77e59c91e5ae87b42c938d7f87', '\x01809eb6dacefea6f1de9e950a09521de694a0f79e4ce4e95e83c0fe6f08baeb8b1aaf79b0e58ae6110c3145ba78c318a4066b25aa52a644dc930e8e7a73ca04ef', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 236, '2026-11-26T11:40:16.338316Z', '3d756ee725602f74', 'legal_read', '\x8a25765094daf43ae7aa207d6158dc9ce083cb77e59c91e5ae87b42c938d7f87', '\x013bdcef3e870c04bf8cab578f3950a87daf6179f74eede85c21b62bd465aded5594517d958e2e570dcb341949912a686977629a03bdadd1b2d4abdcfb71494a33', '5ceaf7b849cb94b27df822fec9f2d850', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 238, '2026-11-26T11:40:16.342693Z', '3d756ee725602f74', 'legal_versions', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x01b9f533c5bf1c6d46ed15ace37bebbd6dd2df6c08dc6973d0e85e8b61dbdefd60146ba76db2a09646a76ee0c07a6cfa920e8e2657eaa6ba383fe6d0b5e1dd6e5f', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 240, '2026-11-26T11:40:16.345826Z', '3d756ee725602f74', 'legal_read', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x01d89f53bdd2ecec0b2ead59460540207a4c0466208cad2b71989d6a2cfa1804a30ae28714ba3340780451a7793d4b3648547db6679690cfc03eae69e1d6c39d03', '88ca692bd359b1cde20637d4b04ebdd2', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 242, '2026-11-26T11:40:16.350109Z', '3d756ee725602f74', 'legal_read', '\xce8de5e4cd29d0d0114a58f34c87ee33bd10b3bc8b85e97655734a32ebfd159c', '\x01173681306a8da37e6899eddd226b130a7a6e2966c92da6993e50bc261d80a8817a68bbc2ccbc0353007f7e22c7ce91fb0274ccd62b7ad5c1779c7800fe8959c0', 'ff4f5c254e8377a713f39bab49dee7b4', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 244, '2026-11-26T11:40:16.354622Z', '3d756ee725602f74', 'legal_versions', '\x308458ccd4b0de18faac74580162e3018074efd83c51b7454241c09de9765273', '\x01f24eb9ef3573b25eb65ecb798ce356d20f3aee34f0baff0f76421e8f33cedcdc2f837b2e1a5a41e3fb4aea930aa4b720693ecd2407140c29d8d89ba6b38bf537', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 246, '2026-11-26T11:40:16.357722Z', '3d756ee725602f74', 'legal_read', '\x308458ccd4b0de18faac74580162e3018074efd83c51b7454241c09de9765273', '\x01297685950c6a29bfdbd7f1720349b2c0cf8a4cdd0a1d9892caef56e036d4dcba42e21538d3ca92d5040e729c30800e5e2e90ff8e0bd3ff640eed5bc2f039ff7a', '717c8b6115289ec19f7f018f853928d5', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 248, '2026-11-26T11:40:16.361966Z', '3d756ee725602f74', 'legal_versions', '\x17c07f6b6d0e96f65f14613feda03eb723653067fadfd6ecf609542f9e9af4a7', '\x015de305e9c1e8946d51d7c3175d8bcbfca2af412a2473f5c599dbbdf8c38ee168aaa2b424f5b21233a6af271d6f1246e192bd7719ae0cc7a53f5aca67850d0719', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 250, '2026-11-26T11:40:16.365Z', '3d756ee725602f74', 'legal_read', '\x17c07f6b6d0e96f65f14613feda03eb723653067fadfd6ecf609542f9e9af4a7', '\x0155155bfbfe7b7eceb44029c428cf71137c3e68595d78f45a69066355f13e57bd68a3390e39a68938d699d43d3dcaa3e5dcbb6aa8761250fed6428cc18468f86b', '8132c76dab5d75b36cd4c765555857c8', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 252, '2026-11-26T11:40:16.369047Z', '3d756ee725602f74', 'legal_versions', '\xb4ebb19f01276836bb307d178bfce65ffdcf1ee9abb585e283ad1e4585ba65bb', '\x0178cc2c74990a6b537db61c85e5510226dec10b50f68e2756f202407c609258f62e67ef8d176889f7fb815449a133caebe61e0ef840caae05d132803420790d11', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 254, '2026-11-26T11:40:16.372141Z', '3d756ee725602f74', 'legal_read', '\xb4ebb19f01276836bb307d178bfce65ffdcf1ee9abb585e283ad1e4585ba65bb', '\x012cd3bdd18d563a178e28994c18223591cab1dece2b5cb9eb38ab7e9178045993a95699ec06fba58731ad32fc292d90d95d76af7dd50ec5b4d7f41811886b33b3', '9fca4ad107febee8c31b676b554b492b', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 256, '2026-11-26T11:40:16.376457Z', '3d756ee725602f74', 'legal_versions', '\x3d6c3e3425f976c06dd690a2ef5ec298f6b9c43ba7c8224e33e20a53080fbb78', '\x0150ff973c145ce360a1c74bbbcf132cc1b22e510a506d6f9860bbc8ca2d804a49776520f433e2e67e4d26efd9921dd29808cfa14077cbd7604eb34b9f0c76b44c', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 258, '2026-11-26T11:40:16.379484Z', '3d756ee725602f74', 'legal_read', '\x3d6c3e3425f976c06dd690a2ef5ec298f6b9c43ba7c8224e33e20a53080fbb78', '\x015ff86e4ea15df8a060ecedbcf8b961575833f2349763812ac9463dab22051a2e5303ec093644e13509a6d4856ccd8446dda8d965c3b38b854bbfde0ac5570fa3', '7202586671ad94f2e9a0cc84b6f97a2b', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 260, '2026-11-26T11:40:16.383803Z', '3d756ee725602f74', 'legal_versions', '\xb2b3ca3c403527258f691389a95333098c2d02d3f5d47304da989ea974f0b49f', '\x016e496384115319e0d9f848ba4abc12744b7b6a0fc2198077a93a864f86a4afd3e370c71aca5dec3e4ad6b74342b6d17736540f775405d54c5492b55b6ed7d829', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 262, '2026-11-26T11:40:16.386713Z', '3d756ee725602f74', 'legal_read', '\xb2b3ca3c403527258f691389a95333098c2d02d3f5d47304da989ea974f0b49f', '\x01872094d2f7cf1faaad79f9bcd54d120011c4176b67b5ddc5a310bbeb138aea61a9fea42e626079cf04f1a49c58039a6ed6c99eb21be8c613722c54071516e271', 'bc456004050e07752009d9c4bf0f082b', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 264, '2026-11-26T11:40:16.390697Z', '3d756ee725602f74', 'legal_versions', '\xfe85f6d08696f1de3e12ff1d17059212ab34ac200cb603e94603e00eaf0e12b6', '\x01be3051a0f5ae8372d511a4b8dd3efbe8368671e87888d918180d7d6113b103e97ee14769f2a8872a4adfb9948267770ce51e65cc9f9010e1f0430672fb46cddd', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 266, '2026-11-26T11:40:16.39358Z', '3d756ee725602f74', 'legal_read', '\xfe85f6d08696f1de3e12ff1d17059212ab34ac200cb603e94603e00eaf0e12b6', '\x0109dd164c5c87b29aad34277efa6869ed53260fd376182c10f8971e554e70025854b3075001b2c0a5f57181cfa8b7a1cf5489c3c05358266aec5ee066c141e504', 'cbe4df4ff51e700d57b2b3886c586a15', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 268, '2026-11-26T11:40:16.397665Z', '3d756ee725602f74', 'legal_versions', '\x6e2d05e38736dc6b98b1a39970a085066bfa822b92fd6ecc852246bb58dde22b', '\x01e4524fec7ec7749e8528353f7dbdfeeb8534f547a457197c5bb410a26505a3acfae0e22d69ea3d2083b32d055f1b53a5c8cc43be4ec6dffee6bccc1f4831b5ec', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 270, '2026-11-26T11:40:16.400547Z', '3d756ee725602f74', 'legal_read', '\x6e2d05e38736dc6b98b1a39970a085066bfa822b92fd6ecc852246bb58dde22b', '\x01cd9346df63e7df2dd77a514165f5b3a21fcac4efbecc2cbbc4c240c4a33de7ab7734983484d66149cbe07dd7c1ea89b6882767d319cb20bf9467f589e5be1ca7', '0fc04bfcfcb335fa24e353f52421efd2', 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 272, '2026-11-26T11:40:16.404778Z', '3d756ee725602f74', 'legal_versions', '\xbdd4c10c4a87ac5333a30a31f96d4982f2f1cfb13b50a14715cbd213b3e3642a', '\x01d3ff2507ffcea428b060247d0c65397596ec766fa6b21ca3239feebe3f296a17341b39a31c9cbc013f1db99cb888730c3bba4bcc176c2416d15e3f9dca0d4fc0', NULL, 200, 1, false, '\x86984f9bb67cf8fa');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 274, '2026-11-26T11:40:16.407663Z', '3d756ee725602f74', 'legal_read', '\xbdd4c10c4a87ac5333a30a31f96d4982f2f1cfb13b50a14715cbd213b3e3642a', '\x01d05e960558d22e281693cca0d9bba608cc404763493b4d2131bd3bbb84986a048d085621eae176300d6a71776c933f80b2a9114455a202ef3b7d098ad75fbeda', '6adc66621df985dea6c41d7852f6df60', 200, 1, false, '\x86984f9bb67cf8fa');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x20b364ed34c70b40', 1, '\x0157a94c0e1f463a154572217858e441f72ac6a5002c23ce1f0defe98142f04cc06778e4079944ffd8e5acc25aa049c96ccc33b2f2ae78aa478aa5ff22');
INSERT INTO vcon.audit_key VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x86984f9bb67cf8fa', 1, '\x015aaab4b5d562ed0d7fd545ede1c06d6c7e4f734a125be521e1fee66abcb58e58014dfda51d5a3c80565e2fc95d391ee91286c22e1219d3ee34ae83b3');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xaeeb773ca95d587a', 14);
INSERT INTO vcon.key_use VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xe086b9f00dca7921', 14);


--
-- Data for Name: lease; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', 2, 1795693214574427000);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', 4, 1795693215383590000);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\xae38e7a5f0cf54f3', 2, 1795693185967265000);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x404421b94a94c6b301cafd9303b6549a', '\xa349c63b8362b611d1acea6ca60d9f25', '\x01fee034ae652e7164fe80cc5e2354ae92a4b84e51079a2e546656b8254623d26c8e64', 1795693203530346849);
INSERT INTO vcon.membership VALUES ('\x16c3a84301ce5f9c56cc5d315e38b67c', '\x54b706a00d2bd04602f13a627df3df9b', '\x012fc70d2c1620b17a3d87c8485770bbec4194ac92f22891bd25c731b44c6acf92f351bf', 1795693205366320330);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xf4abddf0d88ae3d47a7b29a9ea578784', '2026-11-26', 12, 12, 7401, 1795693213739260430, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x4d982e45fe0cfeb35dfa7151e07222eb', '2026-11-26', 10, 10, 6237, 1795693214614358869, NULL);


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

INSERT INTO vcon.registry_key VALUES (true, '\x1a1d64566907f243');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-26 11:39:44.102204+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-26 11:39:44.135788+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-26 11:39:44.141652+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-26 11:39:44.143617+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-26 11:39:44.145693+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-26 11:39:44.148696+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-26 11:39:44.149236+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-26 11:39:44.15341+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-26 11:39:44.155813+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-26 11:39:44.156301+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-26 11:39:44.157351+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-26 11:39:44.160941+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-26 11:39:44.166551+00');
INSERT INTO vcon.schema_version VALUES (14, '2026-11-26 11:39:44.168888+00');
INSERT INTO vcon.schema_version VALUES (15, '2026-11-26 11:39:44.171275+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, 'aa1f7d98ce392c72cc5c90d6be2dd703');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x31bda1baccfbbbe6da25f1c59e005816a8d2c24509b48493227ca224237259ef', '\x014d050b9f3386ccfc3e1c3b1c33bd123d3577a74842038ae2727a99a618770c0806bc8406', NULL, NULL, 1795693185978521000, '\xaeeb773ca95d587a', '\x010cc8767241fe171643e11b6803c3eaf125a27c0e77dee47cff185807597f07fac4fa84938416635f27b5ed28da1161e8a8707833f7976bfea2c0b284', NULL, 1795693185978521000, NULL);
INSERT INTO vcon.tenant VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xae14125f2e3416bb9d183c15e7f885a743869020b19e64f32f8ae0d057b9520d', '\x011317b222f0d2e477db403687b2ba3204ac503afe21b2c567193d1aa512345e0ab241752b', '\x01d8936f019cb411bb475a932ca676b25da89366f54b9883ca2b3c08743d6d', NULL, 1795693191587966000, '\xe086b9f00dca7921', '\x01e8dfbbd3c54c399e75d850450b2f5ee599b3b46d60fb0ccc248b5253a648af8fc1c13fa69ebd27cb9571722303a0fab602672cfc9a4344abfffe5f68', NULL, 1795693191587966000, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x70f5f60afe7f30ec');
INSERT INTO vcon.tenant_catalog VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xacec3d9578d0655b');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', false);
INSERT INTO vcon.tenant_lock VALUES ('\x54b706a00d2bd04602f13a627df3df9b', false);


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xe4ab86b071c41b94103591b9486170969c88f93fe6e3e9f669a105666f3fa8a4', '\x0116f2a40e473e2a27307a3cb482d2efee33631aacd98707fc30713dc98864d6aa26371727516ec1e98da32bdd', 1795693208464640000);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x404421b94a94c6b301cafd9303b6549a', '\x015175da5bc781be01c19029b573cea66967aec0da62b326d086277de28d37e3f7ffbe7079fd0e82d70c88', NULL, NULL, 1795693200013236854, NULL);
INSERT INTO vcon.user_account VALUES ('\x16c3a84301ce5f9c56cc5d315e38b67c', '\x0124f8156226660ec8853cf165bc0c4ce1370ff78d1482bcbe481c90255f8e856b593703e39bd62ce374477f', '\x6bb649b25977d05a156b4a4d0fe1c1256fd0313d27bcf1b35e401c642468a4d5', '\x01a2a2e204f975a40f4353ce678a9a6bd60ab466453e3e9259bbc64eb2d42287bdf2944e4d978f149f9cda3b8963ec749074d32908ba777945087968ece981b5', 1795693201745821772, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x5563d2e9e6d869d4da235ea2f09628d4a4e42a90239520c6e1eb5423f0386c5b', '\x013b98e3842ca928c980b37fe338399c686e45bdf7aad9c26784487c4fbc8ddca917c330ebc98468df168c4665', '\x92a8f5107a4441ee90de882b1466bd18', NULL, 0, 1795693207914204000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x2204b6842667d061431b80b80dd73d72d7a2286d9b896284fa10c375c546731c', '\x01c234a03e83ae049fd011d800ecaafbbb5f8b95633ad7ea31809e8a252513003cde6243bb0c3cc991299c101f', '\xea904d7aa2fe8bea8098b254dcee5d05', NULL, 0, 1795693207963884000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x77fd8999ac57ea0f19946f1b57b4832d74cb842bad37de8fcb1089a583d69672', '\x01833bc2a89fdf9fec68d056e7f5164eef8c977c8d707340d6f872ee2521532ebaa5a11966b11548af1a598d4f', '\x61d71f7586e9fed62b558458439dc9cb', NULL, 0, 1795693208012771000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x56f68f0b928af0fb150c6b04a3d631ae2fc1e3323473b2ca0994d72877e47479', '\x010426b3367f266093716044afce7e808c06b2555533821f30fc3e9f4f32e2d34ae7560f3aebc8e3ec42ffe1ae', '\xb5edeb6ece5f8b963c560ea2eea4bc8a', NULL, 0, 1795693208047250000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x4790c9cc1cf1ef5e89859608ef4ff09693bb9618ce68f12e7e18707e3bf5a156', '\x01fb60d71bf263b12807136deb112b56eb031d551f5c28a204aecf4f88f7b05d3a05e7e8e93e8bbe50cb1266db', '\x8e4eb45c99dd81f2364038c46fe1c5a6', NULL, 0, 1795693207741715000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x03c15367d7b1ea7d7918bba532de2d29db10cdcf712a0e1eaa316faef61eeea3', '\x0139f686079fd0858682e365c0b7c1f20d52bc79ed8b6f1e5eab99a00f41c77e1a7e7dff7099ae9801a9400f36', '\xa8ae3fd61d6cd83e7abcdc66dd9ba65b', NULL, 0, 1795693207798458000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xda912ce5244bd2c7bbba7d413d850037c509bc909a0e546078273987942798d9', '\x0141a41f09baddc145895a9594a20e80c3c07eb9c486035ab307361a15cf4d7b95cc96239fb155a30de81d4411', NULL, 1795693208141602499, 0, 1795693207821544000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xaf8dcbe99838792f8e6f6ec73152f7280312d4a59acab687d9d8589ba03c14da', '\x01bf53efbd9315fd3954c98222b830476c658049e3aa305d47cecc3868c092340fe4cbc508c4a692f46e820b4d', NULL, 1795693208147897551, 0, 1795693207843832000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x389fa94b662c02f7b0a880ed86c6717509eb31bbd4a5a09ce988b3fcd0e6b347', '\x011950ef973281edaa00f148eda58ae587d54fe7ec7f5354b713dbba69eba28f0458fc0133ef4cb78fc5535971', NULL, 1795693208156653120, 1, 1795693207866618000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x17d5ff088ef94d9e8bb6c7de4ec3d85a6f52366e241226493e762a21f2d5a983', '\x01b6812332926e1227bb61558abbf2497160a9b15b13f30b461bd1062f98e2549ae78dfd2c8a730fa14963832c', '\x7221e8e6318e893f7868a47a05a30a58', NULL, 1, 1795693207892236000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x98de441a922e47f8e5a049bbcae534dc523b21bb4082719b01684a8b6dbfaec5', '\x0126ef70aaaab86dd0de92639fe85a6ce38b9816eadc8417c57415dd86c0303509373b3254f0e91be776ee04e3', '\x717c8b6115289ec19f7f018f853928d5', NULL, 0, 1795693208306010000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x39b548e4aabc0c348d01aee393ca448b5d143c60654bdc84874390b96fa8b1d1', '\x011af3804ea58400a82fa1bf0393594c8905c590704e1b0bc4bf594acd43d1e817725cf8abe9e0aeee3042ea89', '\x8132c76dab5d75b36cd4c765555857c8', NULL, 0, 1795693208331552000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x33aa21db1263ff523a82974ab253997cc6196819f93379e1546ec00cb67a8190', '\x0144c397ae7eaa0b25fa9a75dcef9e8c08683d5a950e179ef96d33652d938d64663f1222ee895b2b45b327a3e5', '\x9fca4ad107febee8c31b676b554b492b', NULL, 0, 1795693208351417000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xe57a5c0d15c8fcc95cae009e5f0248fc33ab0d44e3e07635166ff348e9cc3266', '\x0126a110b8c5cd220f79b1925d04cf536f2fd0feffff3310bfd09e284a669a18a70d1b4080dd2a518fe9797b03', '\x7202586671ad94f2e9a0cc84b6f97a2b', NULL, 0, 1795693208382601000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x94a45bffc0a0413f1447b6f124213bdf767871d197e83c01e3bf9fb5736b931e', '\x01628697553f31a1de7810028406af6de65a33fb6e74f18770089248a58c6b165277f91f5fdfc8719bad610a1d', '\xbc456004050e07752009d9c4bf0f082b', NULL, 0, 1795693208402576000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xa78dd6b843135797c92bf3df965644e906ddf45bbb5d823832a1875f4afa378c', '\x0111f018ddc75eca9998a25ab03b403d491da58a7190466b105d3f5f84652b5a5bad5a6d93dcb612dd0578bf8b', '\xcbe4df4ff51e700d57b2b3886c586a15', NULL, 0, 1795693208425337000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x76ecb16fa4390480f987702a8ce431c46f5f328376a508c4e448b0a2b6845b6d', '\x018c87d9d895de362bdbb33a0d3702a1964841efd8123b994972865edb508ac12f734685484ba20125359a8502', '\xd91ddf8fd2309dc4ed2fad69cfa8c694', NULL, 0, 1795693208184708000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x2ce81752ad6ccbc8be986f7f8bafefd066d04f0a8f40b04cb1e3886091081b40', '\x015017bd6500185bf6b5ce35c2f966e2bba42bef438f6a3821f7b95213b46f837a8d0c767ac0dae51776de8900', NULL, 1795693208569753109, 1, 1795693208251118000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x9b800c0f1db0ea70fed09dd48f8a8b91b13fc632c8a170165b8ecb319b2ee6f4', '\x01509c010686f1d3e27a08e156f62557e071e20cdddd6e2a737267152804c5e4d6c816acbccd58999a29642783', '\xff4f5c254e8377a713f39bab49dee7b4', NULL, 1, 1795693208282832000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x367eee3f5cf2cfe5972448264e1ae8e5b7928483e7e65d29f72928dc5ba2d952', '\x01474389cccc44e48116c3a354eab9c1357f2e624c3c91b5b31c63a2e6d100b59bcdc114ac4ed4e00bd317f2fe', '\x6c1962aba5d06fd7ee758ec7ebb7a9b0', NULL, 0, 1795693216136702000);
INSERT INTO vcon.vcon VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x4f3c0eb12c709ef174eb4c4866ee10e577dfeb85181d7921f2282fd5892b3048', '\x01373a6b2bc4d9f1f8b8d74dfc95fb7b8d4642b2741ed03cd204581e0ff9ea47ae79c8b1b2f345317dd0e78023', '\x88f6a6b60abe2c0d0a53533638a82e19', NULL, 0, 1795693216169338000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x05ffb13abad6ea524effbe2f430b89d678e57d0cb5610aa3d75d0632c3a58a45', '\x017ef9031e76c5a0bff5818e931954c59315a96f4c8c654eb1f4dff85bdc25ee59e86e6fa98948063109b5e6b7', '\x0fc04bfcfcb335fa24e353f52421efd2', NULL, 0, 1795693216188764000);
INSERT INTO vcon.vcon VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xb536a850672c83005efdacfac0f6f77b0ebd30176650dd63272f86b98868fe2c', '\x0139b79992943fc65f6d1448895f09780b3afef11d51856aeaffc57ecab67f3949088744b325c1eac3ec662834', '\x6adc66621df985dea6c41d7852f6df60', NULL, 0, 1795693216212923000);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x5ceaf7b849cb94b27df822fec9f2d850', '\x2ce81752ad6ccbc8be986f7f8bafefd066d04f0a8f40b04cb1e3886091081b40', '\x01f7ca74d7cacb622841f9ac0af8e4bacaf70d63ddd4316c7d429399d480ea8fd12c91da55', '\x01136e6a57cb9d639e862b45e485e26dd9f02969b060ea7ce3341b0317e5', '\x01b9400d7cbeb508742313d71cbee204bbacb239ae7cb89e87d2ee54d98007628f9f921e25', 1795693208251118000, 1795693208569753109, '\xe086b9f00dca7921', '\x011357dbf37e5ae8ea84c929056db83a54bab7c85cc6baa8a179807731340421c668cf71ed7ddae2a04924eb9aa505605fed36780569a7eae68477efb3', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x88ca692bd359b1cde20637d4b04ebdd2', '\x9b800c0f1db0ea70fed09dd48f8a8b91b13fc632c8a170165b8ecb319b2ee6f4', '\x0172553b797569db3b5004dd87be656d36f9031247f523c231faf510df7ba149095b49067e', '\x011809e7b47d364c58f2449473bcc491063097d7050fa972c5e01dfb6b68', '\x011115d6ea8d6b5c08e37deca55949c8a2f41a7bd6f220a0d99da2fc0225adb994bc8fb8f9', 1795693208282832000, 1795693208619927000, '\xe086b9f00dca7921', '\x0119b58d4ea2f59de2f05dcf11f28fb6c08660a1e8bf473d44d798a3c1f690d6f92e7f8663393759449216d8f027ef8eb597ddec2aff72d1c46d733478', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x717c8b6115289ec19f7f018f853928d5', '\x98de441a922e47f8e5a049bbcae534dc523b21bb4082719b01684a8b6dbfaec5', '\x01d4ed00ead530aad4bd736c9b5a0b5d54f1f06f7aee17f702b304ba577795556ae36725d1', '\x01a649d3ebdf4dfc30ac86bb9637ee28497634765bddd694e1f7fb55b8d8', '\x01be90e5666e5975870a6a74894d90c9371d1a6049a294793bcad734ea25554d2fc02b4dba', 1795693208306010000, NULL, '\xe086b9f00dca7921', '\x01886868aee4b0f0ae287bc8bc9fc9b8d823d7596eeb5cab9294d7e87bac5072d15cf2ecde5520173594ca980c696c299e76d58b3c598452b4f0a405dc', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x8132c76dab5d75b36cd4c765555857c8', '\x39b548e4aabc0c348d01aee393ca448b5d143c60654bdc84874390b96fa8b1d1', '\x01002e179a39f9266ac2cacbb074ce6a290496b41395b812eea1c85cbe8c4b179a4cd39759', '\x01b187a6044a7f0ec5c96069ac77e31b0349ea7e60b7d09ff28bf88b2199', '\x01e93ff62eca1a7b0f58a364e15fa17b0f3170f2b4d07f70d72da2436a2f98b2796fe2024a', 1795693208331552000, NULL, '\xe086b9f00dca7921', '\x018a12434cc436190321dc06dc5ae2ce3262822bc7ff7d1630ae81999144674540d30fcfdfa0c4530a0bb5e16e5e5610e5d103b7b9500fbcb83ede60f5', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x9fca4ad107febee8c31b676b554b492b', '\x33aa21db1263ff523a82974ab253997cc6196819f93379e1546ec00cb67a8190', '\x015a4397c1ef3c2c644b0d804467e08ad7cb47e0300122835e3d8966d79c6d146634382c0c', '\x013566965e3345ca4e19b346cdc8c52d081e75edc56bb3e395819d9075d1', '\x01ba0a50f66a9f8113d62aec83b4dd9ae87cb18aa55ef70783fc7ac7bb244e3ecde9905159', 1795693208351417000, NULL, '\xe086b9f00dca7921', '\x01ec9a5e04223727f7b6bbde1dccd9b241c95a5b0c1b8b5eca6269bb4c64184f0ffc990689c4326f88b276e32a0b560986b73cffd03d74bd3b2e3e6511', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x6246bc2083a7353a50d150438d224096', '\x03c15367d7b1ea7d7918bba532de2d29db10cdcf712a0e1eaa316faef61eeea3', '\x01f95d9ac7ae42427c1eacfec2e61a56d00835c07a7e59beececc8f766c393b062b7640788', '\x0159c868c54e1ad1f0934e637eafa2e5c9b08854723612f511bffa6ac5e3', '\x01d2b2e36a4169941cc8445e133361236dd884110c676e948fbe81ebf97bcd2dc324c5d342', 1795693207798458000, 1795693208135065000, '\xaeeb773ca95d587a', '\x0102a00401dcf3e7cbbd93e8433016dc29f6f31f1bae3f0f3c62dc8d4208ea8cb1e43fdddb768e56e5dd0e7a5838c25078fb1e5f50c7c1f8096342e537', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xbfd39f58e4ebd92141af728660727e2b', '\x4790c9cc1cf1ef5e89859608ef4ff09693bb9618ce68f12e7e18707e3bf5a156', '\x01344030b53892ef6e963533ca95e89e5862594613ddf9339fadd9492a8dd7ce7f5c09d04c', '\x01ae478245722413a9d7c77a09de3a0e660f2a4f1aec37f25b19b17b1312', '\x015f39e87450ee8fa3c1609343fad74f14a32af9648afc9adad050b52dabbdf095c969b0e4', 1795693207741715000, 1795693208093598000, '\xaeeb773ca95d587a', '\x013dd6a306d8c88c33f76181e66f804f2ea69a1fbde3b053227f0cc7a0561107d0af663de81a360b2e4955bf41603a4b50d2ff5bf17fe6b9487c8e71b5', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xf097132ee6dbf7cebcb9354a78252957', '\xda912ce5244bd2c7bbba7d413d850037c509bc909a0e546078273987942798d9', '\x016f3559155ad4f47c810cc1172184f075abc64d8c37cc7e75c12c26b860482e84d701d38e', '\x011f0e4f287c361d25e31c16e549c171fb151524a7d8e92844982607d302', '\x014771825e58c2a7a67b7f9e9cbb0f10cec0525f45304fe729d9354e30eaad5cbf13e7ae60', 1795693207821544000, 1795693208141602499, '\xaeeb773ca95d587a', '\x0151693c63a4764ce28eb334ef5a0dc72ad5f06bf9f460985d719e76473a6c34b265bbd505c453456e28b834c8d2b5bc2ac753ec4aa8819ba86e70e026', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x2bf236e1ddd779e057896707f00358c2', '\xaf8dcbe99838792f8e6f6ec73152f7280312d4a59acab687d9d8589ba03c14da', '\x01b9b2ef31d5e647254a0510dd8c9078f016d7d8c0871dfd8293eb0d23e811b9fd90b00ff2', '\x01c6b6a157d9f4854338043dd9d47fe23cfc3a3c836dfa2df5a81ef12946', '\x01e30bc3cbf74ddcef82d23a21165167d10fe330a5bafc9a334ff2bd445b4d837eaaa0ba6b', 1795693207843832000, 1795693208147897551, '\xaeeb773ca95d587a', '\x0159459e9d45fab803ffdeac3b4525e71ac78a914b2b3fe1e1f3f0eebf4b43212138ddc30aab584d3003033bb2934362e45946080a91785e4242a94d35', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x7202586671ad94f2e9a0cc84b6f97a2b', '\xe57a5c0d15c8fcc95cae009e5f0248fc33ab0d44e3e07635166ff348e9cc3266', '\x01caeee46eec755f79a1acbdf122eec605f777f598ea44a79935a113c4c7ad4d54792619c9', '\x010e6e616cdb877ebe3ef88534e3c2de03deddb665ae3e2126b75e57233f', '\x0151cf00aeb0fd24d03c882c2fb45b8ea6149af0e876f8d07f24b0fc2e6c01a4aeaff82f6f', 1795693208382601000, NULL, '\xe086b9f00dca7921', '\x01ea3412741508dc2a918350440ab64644bd8cc05293bd2e34d540db3c9325e5cd9a0033bde47f08ece4bb6dd604b9237033acf0596b38fb92228326ea', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xbc456004050e07752009d9c4bf0f082b', '\x94a45bffc0a0413f1447b6f124213bdf767871d197e83c01e3bf9fb5736b931e', '\x01729ff767ca85610e5fc8f63358268cecb18e7c4df0e90f25938ca0b4703fb433bf2a6166', '\x01263f238edfc51f3177d52f40214afdaf3b9437dfea281224efce26df42', '\x01855f4f3373fb8c0c8347b5d5866536a6d0acea607c17fdd42415c7b148d65b34e87e47eb', 1795693208402576000, NULL, '\xe086b9f00dca7921', '\x014a4db93af6b7cc3fc5d96ee532764422528ff5abbf27069421aed00d44e6f2c04a311850ffbf823049b261cc3b6fc412c4139fee22dd94b985c6b3db', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xcbe4df4ff51e700d57b2b3886c586a15', '\xa78dd6b843135797c92bf3df965644e906ddf45bbb5d823832a1875f4afa378c', '\x012bb670d3229499b926997fe2f8e5f862c9d5d5c97b78374b6d5ffa7614674b01246f97ab', '\x011759bc98be238bbd9a6dd772653a9b8c713b3735c442b4a37cab4b7465', '\x017752ab4f900a96949d0c9a6295ac595267dcbfff6a9f18d29cc6e8182c431a8e2eb3e388', 1795693208425337000, NULL, '\xe086b9f00dca7921', '\x0178506a3e81736ee67bba45842160afa4a1e75ea8e25f1cb3cf4af34942248065cd74ba33fc637584578702b549c7c3aebb304bedeec95d279b29529f', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xd91ddf8fd2309dc4ed2fad69cfa8c694', '\x76ecb16fa4390480f987702a8ce431c46f5f328376a508c4e448b0a2b6845b6d', '\x01d9df6845159b9345ae80178803c5849d364acd2cad2062b158e52efa633b135d73e76b56', '\x0184522d6ce90935805be1df7489cee46a228080033d1e3eb3404bff3948', '\x0179572233b9524fadb643b0e04a1749b704be066c5b01b002f859444db97124ae9c017fff', 1795693208457325000, NULL, '\xe086b9f00dca7921', '\x01f59ae38db529caad5326aa11911997837489ec38f430bced2a67c775c8f28e799800b76d00635a056367a78eb299b187373525a6489d2f1a89928087', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\xff4f5c254e8377a713f39bab49dee7b4', '\x9b800c0f1db0ea70fed09dd48f8a8b91b13fc632c8a170165b8ecb319b2ee6f4', '\x017fdc3b7adddf4afbb3265e6627c2153259ba063755609fe7daf6b153f82c5c5103004702', '\x012df2045c151934edf5d7f59e498996bf534e17b9f98e4665acae36d4ca', '\x0198b4b2c20850aa38c64934fb81e8b6c85df47202b1a958025cb4ad62c22660eca417c956', 1795693208619927000, NULL, '\xe086b9f00dca7921', '\x01f97a3140af0fc0eb1fa831ee0f32a9de143906a9798a7765b47dc992101d71f482be389461a7324cb0d8b1a74904a7893760104f4bd3d3e81179f66d', '\x4d982e45fe0cfeb35dfa7151e07222eb', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x7f957b5192b5dd9fc54f4e23c30f1dfd', '\x389fa94b662c02f7b0a880ed86c6717509eb31bbd4a5a09ce988b3fcd0e6b347', '\x01a72cc55fce0b4122ef6f97eb477b321c5c7180ae926cf311db926fc2a2d3455b8b067e58', '\x0124d8c401cbbb8d9b507367c720e5a62014e9cf27222d97cdeb59223109', '\x0135d0efed3c9d93bb779db7c4abd54f1de27b0b4b6772e05db53b054b5ea1848fee157a3e', 1795693207866618000, 1795693208156653120, '\xaeeb773ca95d587a', '\x0162b6f5ae5ba209dd6244b6815528b4142d1b20eab8654793a6e175317937ad616e60ea1f2bccadbb9f654c887965663b63d96542916f1fbbd052e51d', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x7221e8e6318e893f7868a47a05a30a58', '\x17d5ff088ef94d9e8bb6c7de4ec3d85a6f52366e241226493e762a21f2d5a983', '\x01d84bc2843fe3573a869f13021b4a1459cc96a4f20c27236c1f4db6c76ffa5528225f9655', '\x0161975edc3ca0982a5763744a510744ccefa4ad01d2597b4930c64d8af5', '\x01121bd4b2cd481ea44a053c845962fe520463683cd9973d2a3245c8b8d43b695684b5b423', 1795693207892236000, NULL, '\xaeeb773ca95d587a', '\x016e8c7da0a836c574f9944a2b2fa4fbd384eef999afc6292c2e3cefbd095d06d314233859544ba4fa535934e3d695431d19613da85976b461edba2626', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x92a8f5107a4441ee90de882b1466bd18', '\x5563d2e9e6d869d4da235ea2f09628d4a4e42a90239520c6e1eb5423f0386c5b', '\x017e0db668fe514e56a1daa9f360104997ec4c8870c28a4fde17592e2e54c1b4e52d4fd2a5', '\x01a96404ad418437dcb33ef39453e8a24daa8f23ee73582b535f1c80b8ae', '\x0109a6b56fc0d2c286dc92d510f4b49c77041e33f3333f1085d95a53194e490dded4c6ca90', 1795693207914204000, NULL, '\xaeeb773ca95d587a', '\x010b373c7ad896b4ace486aa934238abdb2d53b30cc3c5ff0abce74e0a462d937759312aceb720eb3b0c99802f094f0e893fd94064f0f0a46059325a15', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xea904d7aa2fe8bea8098b254dcee5d05', '\x2204b6842667d061431b80b80dd73d72d7a2286d9b896284fa10c375c546731c', '\x01ea5471bc78f149d422c5cdd49e2bf9a3fe34fc97c67040842f1197598d4bdf754ac36e6b', '\x0136ab4e7a9e98666142c2bb4946e3cd5491b20fa076582e842ed9e34d02', '\x01b62d3f6f54d2731c5fe3fc5a9c64344bd83b86dfed2cf30d01a5898ab4fa3ea10fa382c0', 1795693207963884000, NULL, '\xaeeb773ca95d587a', '\x0109785482c4c56ced83a55060fb3fb5b296274bdff18d0bd42dd964574301f1c8cb0def76aeb0268d707791de17bf7188153440a0765bd0345f3a5b3b', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x61d71f7586e9fed62b558458439dc9cb', '\x77fd8999ac57ea0f19946f1b57b4832d74cb842bad37de8fcb1089a583d69672', '\x01582f298764654564e89c016db0ab76a7596bf9ea8373441cd788047b9d688671146d8687', '\x0119c413e7b7ee89f5e5e3fa6afe7c753f6471aa94f1a6593ad817977088', '\x01fdabdc6e91ef9d4f4c8bdc742347a88f3e9afe03b09e3d5bac1e65c085a5eb6e82e2a6d5', 1795693208012771000, NULL, '\xaeeb773ca95d587a', '\x01302be175885ebf66b3b98e754949e21a948bef830e29e164c912361d8d74ee4c93b85a8389ad3fa850ef63f48b7e305ebbeeb3381a990fc7ab03a07a', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xb5edeb6ece5f8b963c560ea2eea4bc8a', '\x56f68f0b928af0fb150c6b04a3d631ae2fc1e3323473b2ca0994d72877e47479', '\x011ad75e2ebad713f98d4d108508d5dc151cf4b861b82049b87ac84a1e52aa93da8842e831', '\x0144fdd4f0167749d1ecf7b0f8823a0db3edbd82e7ceec3d19ec0f814bf2', '\x01108108a176a69aa17c58ecfb6a1bb340eea3c693936fcf5c5e7f77815ba2caf91b259877', 1795693208047250000, NULL, '\xaeeb773ca95d587a', '\x01b0424cc4b2509ca7e8a1b1ff463c96f4125475ffbca71daa015bac18468c707ff77f6210556d475410d9b81aff087132490fa69052eddc8b98b63a5a', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x8e4eb45c99dd81f2364038c46fe1c5a6', '\x4790c9cc1cf1ef5e89859608ef4ff09693bb9618ce68f12e7e18707e3bf5a156', '\x0135dbf9a3ad2013eba9942b3213a78ea8ac9c3e3f6331079a3f4080e1ac4ea70ff6f4fde9', '\x010d6349af27afbaa8535407c46a4f5203de3c3bdf46edefa6702f3de060', '\x01dd2c2afdcc757c4225931984ea0a9d21256dfe301d4a44d00717450ace58fbf169233e69', 1795693208093598000, NULL, '\xaeeb773ca95d587a', '\x01851246bfbbf523e75ce14a2485adee9275149cae5a960a9e817d0f08c0ce9f71938081f37e942ba7b7a2123a3b76837dff63a772a3e9f7af2c2a3944', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\xa8ae3fd61d6cd83e7abcdc66dd9ba65b', '\x03c15367d7b1ea7d7918bba532de2d29db10cdcf712a0e1eaa316faef61eeea3', '\x01097c8436d68af57f3f06d25ef92651895ac74ed96141b786aaea3282d0dff4b0e7d93cc5', '\x01fde20ec434e5c0af3a898f08ca49187cb01535bf297e1b3f756baa8aa5', '\x01dd14f5d98cd5c3b65db74a0ca06d596d887cdfb6443706accad1ba16d20ecdc53ae017cf', 1795693208135065000, NULL, '\xaeeb773ca95d587a', '\x01dcceaf4dc07ece8becacd6527ebc218742a44da84186a7338f2934adc201f4040f0c5ca3fad8971b222915f00ff3d3b7e14a042120e89d1d5eb7ec17', '\xf4abddf0d88ae3d47a7b29a9ea578784', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x6c1962aba5d06fd7ee758ec7ebb7a9b0', '\x367eee3f5cf2cfe5972448264e1ae8e5b7928483e7e65d29f72928dc5ba2d952', '\x0153fcdb7d0dc13eba292bf52f89a3a783b8eba3038b690908cb5b609cc95625fdc4348d60', '\x0129a84ab6bf63da6e3958951c354953de8c642700dd7aad118852c3e2f3', '\x013fb210fe10743cc5d3b4e55a8b31ca728207bd0114b3e1b4d7dba3cb106b5dd61a4da971', 1795693216136702000, NULL, '\xaeeb773ca95d587a', '\x0176ecf3843b744ddbd80d28409a6a134e6d9c45948acbc9dabdbe6cabc69736682371d8ac9c737b5a74ddc7afa278861be274bdb57e1c8800c3ea463e', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xa349c63b8362b611d1acea6ca60d9f25', '\x88f6a6b60abe2c0d0a53533638a82e19', '\x4f3c0eb12c709ef174eb4c4866ee10e577dfeb85181d7921f2282fd5892b3048', '\x0102671ec3bdb278ebac97c502789aac47a159d835a0f9ed3aed49e7e8e23576cc79303733', '\x0164d8e59c4b6068db52350a18f6d01c0b55aa5f91f5587b74bd3a0270aa', '\x0196c064a3876c4f8a9b7570e4e038a260dc1ce07266a169c565b81248017d409b26c2cf3f', 1795693216169338000, NULL, '\xaeeb773ca95d587a', '\x014b44d4922f423e10b9411f1f115dc381028a362180ff13819bbdfcd84b8efb466cf785e7554a964de3bbc85230e95144896023d6fd6aa4e79a335aa4', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x0fc04bfcfcb335fa24e353f52421efd2', '\x05ffb13abad6ea524effbe2f430b89d678e57d0cb5610aa3d75d0632c3a58a45', '\x01948480b7e80d99430fc4de63bfbc90dd1fb2926294a29ea07483c1725cfab9168c0008b6', '\x013a3ca24e328cdfae58742b3e06a54e241cad1274f82b9aea84685c7a37', '\x0125c678a3d712b35634f2ad56bd31fc32d6fd76f53dd162c7eab8755d4d75089ab779fbb9', 1795693216188764000, NULL, '\xe086b9f00dca7921', '\x01d8250dcc195f74dbcaf93f51153683cef2c34dc6666fb12e8f00f4b6903cd908d9955262c7121cfd3d2552487fe764efb738d5c6e003baed5a1e17d2', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x54b706a00d2bd04602f13a627df3df9b', '\x6adc66621df985dea6c41d7852f6df60', '\xb536a850672c83005efdacfac0f6f77b0ebd30176650dd63272f86b98868fe2c', '\x013db49312c48849e6e04b3fc0ae9df734020475f8bf8b705b9840b6f772f2516c33a3f205', '\x01051508a816a2c4766479c1d58fc0cb8fcb4d29ab0e0f04b84c2879c9d9', '\x01a69140c9de1af7d674072b144019e715ad78654c084cdcec39815e7c950d7d3cac2b5138', 1795693216212923000, NULL, '\xe086b9f00dca7921', '\x0155cf8c67afc4dd2c95e93e30d8fca24afeedba2c4e7fb835b8688a6d57f0dbc2700eff7eae0eb28b20888a85d972e4717746dddcaddcde55f9f84751', NULL, NULL, NULL, 56);


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


