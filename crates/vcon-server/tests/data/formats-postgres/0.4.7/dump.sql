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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-17T04:54:48.601666909Z', 'cli', 'tenant_create', '\xd1a822f00328dc1b465f9b1aa6c4f515532509a9c1c1578d3994225ecb284724', '\x0175ad4d2b6cd4fc0b8b000982d240ce6adb771fba27912197469c70b000c359262f8c5033cf2366b24d13ad11c77e4094cb2e755500dede07f4463aae', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-17T04:54:50.753751547Z', 'cli', 'key_create', '\x386b0328d30a909ed30a337b4c69b39759bcb1d68aafd13a2bf28b53c8c1dd4e', '\x01a9cdaf366af4798390cdfa233534da793bdc89a2765105d3906767333231a50cf8c85eb63b807f94ce2f59a9', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-17T04:54:52.933320545Z', 'cli', 'key_create', '\x8e39236f979fe1ce3aadb1baabd1611bf8286273f2b4f58cbc0491fdde14fc90', '\x012e6f3ad72d358ecba38dae7c71a7480e74cef8c1fe51f0c7b0d8cbf76063628855229e20f2a14c55399bfd1f', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-17T04:54:55.406280254Z', 'cli', 'tenant_create', '\x70d72798b6de43da553aa7cca20753210255a553d0eb07b251a8c37ffdc6592d', '\x0142f790ee96c9caaf01e2329a3f1fc2787517dde902cd91d3bdc374520078d751c3a47bc15ccb5690c6489ec7f264ba69638fb34e235d2fc51c45b6ba', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-17T04:54:58.222950462Z', 'cli', 'key_create', '\x8f715418bff245406673ec77b8f65501cd432c9422c769b691ef72e5cbbd1162', '\x017cc91b671169adf460d638d1efb6ab09698616b621958922f8d2fa8212287cb1567b4a57873b7163cf25c2e7', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-17T04:55:01.894697799Z', 'cli', 'key_create', '\xf3519b1c56c4c15feb6ba8effa671409d52cbfe22b9ba51365566c05bdbcce3a', '\x0134bc8d2403a1ecdf32694ec6941f04ce3bf8cb5cbdf7e09cc82c650f5ba34330eb167bbb42f0c892e0ac85ee', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-17T04:55:05.346837895Z', 'cli', 'key_create', '\xd7681c4eddfa3a0d77292534c41225489146c167483ac8ec34ecc5d74ec8157a', '\x018ad29f5ddefe6fbb3445cee0e6197a9a81baf3c6b64d10bdebcce8b9a6d2f6d048d37053db8d57ab11c77b5a', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-17T04:55:10.939026789Z', 'cli', 'user_add', '\x01fa1bf2030f143ee32b6f34727ba3e0be4e99101bc456e9c267f94d97823e65', '\x01809d13f1c0a3a96d79ae6b5fb6ef53238076cc053ad45df6e0a41feaeb2a620ad7ac14a7b0fdc0769f21fb2ab0a18f875604df73f907c6b60ab2bce2', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-17T04:55:13.770262949Z', 'cli', 'user_add', '\x91b951fe24a47fde0ae91a93cfacbc08794eccf68d1c0202151dc6a81b152315', '\x01912bcb9171eea8b2a63efb82a3cdfbbc3f61567584e9846cec4e2464c0b8482c4efe6de39faff5e6d79a3a58f04629b623f8fb41fcb03d1375e590f6', NULL, 201, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-17T04:55:17.61695798Z', 'cli', 'membership_grant', '\x1d404160a1057d6d35fae3ce3f915139041b03587bcf20c8588c498f6009e4af', '\x015ebe320ebad38ecaa6e0790905c8a61aed7167cc010e1dcf75370b40d00d5b4c827aa3d39d880bffc378525946c5a0bf52b22119a93ab3dfb4635fdf727ab3aac07e751a15c4c3e56f24949f9f867ae92b4fdc8606511e2db89bb32f5d03429a25fefe8c', NULL, 200, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-17T04:55:20.49040488Z', 'cli', 'membership_grant', '\x948b399080dcc477bbaf25c1de9e413d5a1feff04b65328b1ea698c195943d4f', '\x01befafe792e081553100c4eeb441ddc2de99a6f35d25aa3ac5e55dff4e9d8dc8f01726ea26b815261030af48f1fcbc358198fb54eddb7af51af309603d6efb5a5a2ca345e8a6f04f1e7ca851801f23809628691825a59af5416b9f5bbd397db767e7c032722', NULL, 200, 1, false, '\x9725587dde3cbef4');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-17T04:55:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\x9725587dde3cbef4');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x9725587dde3cbef4', 1, '\x01a32424d83367daf7b1acecbb8d76589654856d31c605d827c596cf1cc510d0b72804f964a036884c904445219fc7c3b61c80469d7c16daf13513b9b0');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x568887478576e2a5', '\x5aa55f9947674e0eb8acc35d5dadd218', '\x5b79e70f6d077502da11750e7858ce24547eaf20cfcc3f0e3bb8ca76e8933cf2', '\x015223df667989e1ceaa553b5ca8a1f2fdc04c5792f84a3a854ed95a647a028581', '\x01de7d0a47fbb62d954a1473c006fd18d3f6ef1fd8d4d72c322cadf2c6b88048d8b77cfb3c51e17f5d596e97', 1794891292932791434, NULL, 1802667292000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xc5875e5d779954fe', '\xb50dd7af538c83a987d0889f76e31d4f', '\xb19e03bd956131abd4e152b475da41865d751ea049c9a9a29c38c37e159bb6c2', '\x01addf88f3869a322fcab29efba78358ef5038ab651b2178e5553f5a9630ac1bfd', '\x01cc7dee1360c24d654ed6a650a2207f6a2aa20e897465a36819518027865f00aeacff0391cbd6ec59a98b22', 1794891301893877643, NULL, 1802667301000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x2a4c2f580a02a1b6', NULL, '\x305f0806cd25f801802a598ac621489527f412a4f1505bd1d35e8cef9975a0f8', '\x0115f22ecaf1d4588fc33d426ccb81390d1a2d9db1d582e12ee758bed719a30ffad6', '\x012ec595c6a97f39107b3a8d32a47dc14e9f55aa5af898de3a8f9469541c13bb', 1794891305345893719, NULL, 1802667305000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\xb1b795d3306e8f08', '\x5aa55f9947674e0eb8acc35d5dadd218', '\xabbb2fe374a16e286a35dd87ad48523175441ba7e43886fe6f33f24979d02bb9', '\x0197dd8da9dad6ed98af3e69a9f5e4b5226cc912013c9b52f080a3993d00bd279041cbe5c4d0be2548640e7e0ebee18f46a8cdbf22596a0b618a', '\x01f6de340e9a9a032b4691ed67166ee43953cdb7b74a66e0b86927608d0529f268e3c4c3d0c2e541c6ff186bf26d', 1794891290753064353, NULL, 1802667290000000000, 1794891300000000000);
INSERT INTO vcon.api_key VALUES ('\x0f6551ac70bc3575', '\xb50dd7af538c83a987d0889f76e31d4f', '\xcd78920b57e96841356bd33fc7335b9d0ec2be8c66498b73a21a1ebe0010a590', '\x0101687945af3fe383132052e21cfcdf35dbe13138733574ddf7988af02674d57a259393cb5ca0e267e831394acfdf7be8aa0c8a64d3971885a8', '\x01673072e6eb79c97bab9b0781d7895e335429b2add30ad523a237a5b1e73c88b68f424cfa29f987b3b14b952791', 1794891298222025286, NULL, 1802667298000000000, 1794891300000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 2, '2026-11-17T04:54:50.757907769Z', 'cli', 'key_create', '\xba45a6586518c6add4549f612e62448a5830ffed68e3e4ad5798885351e08c2d', '\x017e036efd257c0f792732d60a4eb3ed58a86c65dff134b2ee842aab36c26901ae192f0df9407d0dbda81d2643', NULL, 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 4, '2026-11-17T04:54:52.938445526Z', 'cli', 'key_create', '\x46bca81cfe339c1b89ca8716cc14d38cc06b45dabb0980c821b552e2ba76ac24', '\x01c5afb7a3534f51c7a8037a5c826cedc158b9d9f56a66ae3a02a02e4a6cc20852f9d589b5d496ae5007cff6da', NULL, 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 6, '2026-11-17T04:54:58.22806686Z', 'cli', 'key_create', '\x7400f176cbc996f39263829538dede98c49c8882dc436fd9c0e43e301c47a10c', '\x012a573309cd4e14bdf5a9104107fb0bda03dce4718566591b32c1cdc02ca6a61e0f10b1b7f2860db72232c9bc', NULL, 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 8, '2026-11-17T04:55:01.901093846Z', 'cli', 'key_create', '\x75a3e8149283f3de55dac41c9aa07a05084986de5039b25bde18e9f29762694d', '\x01739b59ac1b2819b59645cf92d61048c697ae2202effe3b5321dd4b5014f2cf4f7cff9eb8a9ce69c6bef7b4f8', NULL, 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 10, '2026-11-17T04:55:17.621711395Z', 'cli', 'membership_grant', '\x7272cb9cc202889e3b72166218400a8f1bd15a6dbc98ed9e2e43b0d78dbff615', '\x013d2c86925fec0a23c7c3019dc409e3251358ea24840af5c6fd753934da54702b75ef87ce0216f5c2bcd43e0ac588fda2c15e2a1be4b1a9229edf53e91f3a9fc7ac42770d1ca0249754a299af335b4efd773666fb4eaec9cb3fbddbdd0d00059ebfe203a4', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 12, '2026-11-17T04:55:20.498382676Z', 'cli', 'membership_grant', '\x804690ad5c2f8402c3ba54dae7923952c61c0e3dac231ef8c55f5d0987c02a56', '\x011f1201b1e004305d7b95dcc4df362fda2a01795c3526c3924c9fd21adf45fa9ba6ee31604b0b432c3bd3c8faeb1a0924de4863692098da79d5ef3fcda6d5b5d21b020cf9ab4db41bba8e46a09c168027511fb88499cbc06f7d68b5266d15b262fca4529b20', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 14, '2026-11-17T04:55:23.152976563Z', 'b1b795d3306e8f08', 'create', '\x1b2aec2bba13324e4fe8338285293424d3c6d127f400afd5dc3d3d0074c28f9e', '\x019da8f805d2c3355ca4faaa0b3d96b3b68d4bc4b9c6d0f802414d71882ac8e6d858184afa540b098fec3d838f870ad70a048ad2cb2622fa75037d0710870dbe4c', 'db457c53adaf5d2e806e5aa0ad4f926e', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 16, '2026-11-17T04:55:23.206039151Z', 'b1b795d3306e8f08', 'create', '\x0ba172685ff4115ebff559724743b25287ec4f0413b5012a647d746c1bcc7f93', '\x0164e5a9409da4b45049380e1965205b7e28422ebe63c9631e42e78e3d1d5b8feb6b790eea7837258fe0fe7dc801d47652bfc9334cf1261c01c1e597ef40a87560', '1238424ea556358b8906fb4b359897b7', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 18, '2026-11-17T04:55:23.228172511Z', 'b1b795d3306e8f08', 'create', '\x7d969b9223d17ada8689508b63e2b5da9cbff7e3d1548e957aae0cced7b1fbf4', '\x01bd1fbe0791a97ef0f2eda7f91ac2f5f0ca2604afc51b2b94552408e7d2045b760b85f491272585f04ce5f660c81e5e1b849bf52706c72e9f196fd56b6fe8bafa', '876eac2ea831d883cc5abd404ccea6ea', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 20, '2026-11-17T04:55:23.268552849Z', 'b1b795d3306e8f08', 'create', '\x3013f8cf483637d69b4a079cedd48027e075e605f72da7ffd2a1604cbf1d22f0', '\x0138fde7f627a546ba795bb5ede31d94a3b48af4e4ecea9824d9f964bd0bd347c857715c3d813d93f7d2a6d436816859fd062c892a4cdb67b2cf7691f221fc1f83', '7b11f380e6508b39570a276f2036efed', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 22, '2026-11-17T04:55:23.29391881Z', 'b1b795d3306e8f08', 'create', '\xdb5950483f659addd2326443d4714f974c71b2d159ec6f2214c5d4734349e47f', '\x012908c79a8e2c3374fe0912bf12d25e5f1ca969c2554d7b3a48adf35bab7e7c8d78859b9d0a0e5dc9f042f7f440dd6a306b2a9fd9a407f837e52adf3e59ae49d2', '8553505b7cb6e246a181cd3649808305', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 24, '2026-11-17T04:55:23.324749863Z', 'b1b795d3306e8f08', 'create', '\x5ff19f373b22a884daa28f15f55635186bf192b843464bc10836aaa80436e346', '\x018d906725e1646207c910c7a435fbefed81ae745b80e5ea25f4fbd6f0a24d3d97d5d51706dff93145d7e829fe3c2c722b3b2c57836af2392ba3de2c19ccc57ee4', '0d69ab79c8bb6164dbd661abc2957eb6', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 26, '2026-11-17T04:55:23.365123462Z', 'b1b795d3306e8f08', 'create', '\x603331f2eb7ecc6ae108fb4b5c90a21f4e9f4ea2d96cd3a2b1fb986cd9f19c9f', '\x01a622f711e0bbda1d786661284c48b35cc3bb4d5fe0cb1cd323cc4bcb9a40dcc14013c2126c44414f724fc84222286ff3ea21c4ca52586d3eab166463c516d09d', '8c132144e4c30e3a6e0be6509f91d692', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 28, '2026-11-17T04:55:23.386028952Z', 'b1b795d3306e8f08', 'create', '\x8628228e413cd3690b97c23939093e6140303c11c4b14d19adb80432676860b0', '\x014911bea6b0c86e2012f7d4fff978f59f1c1c648fda8f34d36801a33c1f6ef658c537d3c156ad2d7d7d24af00b0d2bf0e9208bde0ff2fc1771a565f2b2ce05f4b', 'b2a5467f7b1855c7254a63b762a3f831', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 30, '2026-11-17T04:55:23.425630316Z', 'b1b795d3306e8f08', 'create', '\x71f75083892aa29c849d56d9d052ec2588546a7a72d688ea5c665175c01b213b', '\x01bb606f015266c1138bbd14b3dd09678877873df8f7aea6f0ef423b95f3820f0631d0bf5737bb17709742da43fdbc29d0eb2e88d4eee04b43caa0986f6fb7a588', 'f46fccf7a533c223df31986b12e6a9be', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 32, '2026-11-17T04:55:23.449149712Z', 'b1b795d3306e8f08', 'create', '\xdaa316e48fae2735eb70e00682013a3da585c1b416326f49df4d5548d6fa9322', '\x012aff9df7277ad4393ac83bcfa83765903592f42a2f9cffbf6d43e53882d9d63b6bb2419f0924e2b7d70c66bcccc56aed29115a6ba1fe24d7e33383bd5cb9ab50', 'e9f31220be813ceb8725bf7ab4b8f7cb', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 34, '2026-11-17T04:55:23.482407578Z', 'b1b795d3306e8f08', 'replace', '\x1b2aec2bba13324e4fe8338285293424d3c6d127f400afd5dc3d3d0074c28f9e', '\x01c309faf395b7d51c9e41a2012cc0a8e41f00278b4db3b245ad6dc378deba5c4a8f56372e54b9cf9400a3407bfc7258887e519524fe6bba513407c01f3fe987fe', '031d555152e2909feb97b4d55dce6b42', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 36, '2026-11-17T04:55:23.521236879Z', 'b1b795d3306e8f08', 'replace', '\x0ba172685ff4115ebff559724743b25287ec4f0413b5012a647d746c1bcc7f93', '\x01f485ad684c41e51f65c57dd1427db8c82ec885dbcc43ed539cdeed59bc775131484f4edd92319c433c4807123bbfee551775260e5a7199d77f1233c42adf8799', 'f73cd8cfa9d89b53f94da0326adf113c', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 38, '2026-11-17T04:55:23.551885394Z', 'b1b795d3306e8f08', 'delete', '\x7d969b9223d17ada8689508b63e2b5da9cbff7e3d1548e957aae0cced7b1fbf4', '\x01d7b06bb7f1dda86d7794d659dc7d43ece7c97dbaa9af8cec6cf7972dc194955007a9a2af6e695c9579f97a27b4a1735a1270d3765ad0e1ac4d131f44c3ccb2af', NULL, 204, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 40, '2026-11-17T04:55:23.578458687Z', 'b1b795d3306e8f08', 'delete', '\x3013f8cf483637d69b4a079cedd48027e075e605f72da7ffd2a1604cbf1d22f0', '\x0111106572a05cc300a827fe9b0d04d0744657ce8a2fd286957621acf125bf5e72ae11a8083305b280237fee9020ba1a91f5ca82a5112c794ebafbc215abbbb910', NULL, 204, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 42, '2026-11-17T04:55:23.588002603Z', 'b1b795d3306e8f08', 'hold', '\xdb5950483f659addd2326443d4714f974c71b2d159ec6f2214c5d4734349e47f', '\x01e77dc37758137f306a8bd1c3d5b97a2e4736b8219ba40b0765d87ff598e34214592825c66fbce56083478bd9349afbf5b934bd465b6b948e76228a102cda7328', NULL, 204, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 44, '2026-11-17T04:55:23.594123416Z', 'b1b795d3306e8f08', 'delete', '\xdb5950483f659addd2326443d4714f974c71b2d159ec6f2214c5d4734349e47f', '\x0100e3492e20fda19a60f9bd1915a0c9f7e7e2032da8e0dddc5f895e420d6afc179b98d47cb465dae05ffd9b96d1f36f353ed5eb50f91b438df34c5983d3fedddd', NULL, 204, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 46, '2026-11-17T04:55:23.601744916Z', 'b1b795d3306e8f08', 'hold', '\x5ff19f373b22a884daa28f15f55635186bf192b843464bc10836aaa80436e346', '\x01cbaff8c07eb13755d0e023734c546851adfcf4a500788ee585b5477e64d9307e058ef5b6ff09517f83ecf63dabc405385f3cdaaafa7437ac7c5e1a5a57e748d1', NULL, 204, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 48, '2026-11-17T04:55:23.634105032Z', '0f6551ac70bc3575', 'create', '\x7e24ed1f05d10be3135c33f92eabff5f413457cce5f9d5d7df65d3d061451c56', '\x01fdd95900fcc6043a018e778adba1159da7346428bf45acbabc46e8b3546d7fdc7f7993ca6a2ac8d2e2429520788f40c5eebbb02c00c77ea835068f0ce2f09014', '7e77e67822cade4b94c44ec83c2951ee', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 50, '2026-11-17T04:55:23.665861805Z', '0f6551ac70bc3575', 'create', '\x03ca3130a7d3e2011febf32a6219389a0fed4501ef7aeb76906d1408d66a7474', '\x01e373856a6b6a2c966a2bee1fdd53fcb6ec7596b5ede6ecf7c3dc059bb485c474512a538e4a1b0c63068f541132383fa73da5860d90bf1b67c50bfac75bf2cac4', 'a465da5476283d79f77a5bc3e6b2be00', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 52, '2026-11-17T04:55:23.696923311Z', '0f6551ac70bc3575', 'create', '\x7d166687403f8479b95991813e6bc74a2c4a18f3d5735701c2a1556abe8f8de6', '\x01557ae191a8d77157e7d411f5545aedf7c0053fec81a140d3ab1a8974fa482acb054f56f4feef6c230d449f2d2859e281e71a9c8ff92c050c13183726bee812d1', '5645241fbc1cb4ce92aa1a55c77dbcb7', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 54, '2026-11-17T04:55:23.755615119Z', '0f6551ac70bc3575', 'create', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x012c3f296b2bf00a9047440efc8ce0251b78f61b31f1ffa1caebbba3d25e1867df9e5301fbd91b32f501ffcd781f01aa191fcc8eabe56feda890f4244617a85e44', '26033b4803d02d168c6216cb8200e98a', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 56, '2026-11-17T04:55:23.789692476Z', '0f6551ac70bc3575', 'create', '\xc55c1fee933134319d547e40157a202fd93e21bf3403d3df0c6874c991cc226d', '\x01de69fef10ff52dd16c83a016e711ccb9787104d86aebdb1a0a42120c9131dabe78450e3df0bd790a68e8f233fa45ab13f14c2d2db5777286cd2c944c7bc39fde', '5b3de6a3567e01ae208b67ae181dc6aa', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 58, '2026-11-17T04:55:23.822866991Z', '0f6551ac70bc3575', 'create', '\xd4bf1d7c88504e9fea648ccca58b2d03cebfd68c67270e1cfed179ed67f166f9', '\x017782fa0e568f937d0565c5fbf9737af96574e5b11b65347b8d0dfe0e194a5da7532c61257a00a5e7d14ca9715c2a64f636f0d8387c9ef1cdbb7f80c38c35177c', '1e297f90b3e7e1ac6689dfb2fceb3278', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 60, '2026-11-17T04:55:23.855656153Z', '0f6551ac70bc3575', 'create', '\x1d63ae5031147227289dad8893c831738aadb63f6a7994dadc750a777309c4d2', '\x012b6e503df8327285efba64bf66fc79dfeb2514bd5e305a5016ba4f2e49513f20c5c502a7579c8025654eb819a072a442f49781479922fe7a6156c9b99f4313d1', 'da747f567345590fd555e3a0e1f307f8', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 62, '2026-11-17T04:55:23.877806012Z', '0f6551ac70bc3575', 'create', '\xa78bde54008e62bcc5a78d086d64a149172031d1b71543dbd8e3563b6a9bf497', '\x014037bbbe6d9a13297d1f0238ad7c7973e73853b4cc1d013041f4b50bb21f90c892bb1e95ece7da6e617959ccb310f2d3e8ff76a3a50248e917a2b2393a9c73aa', 'dec093f719f1bf93fbfaf63ff7397008', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 64, '2026-11-17T04:55:23.920327498Z', '0f6551ac70bc3575', 'create', '\x54d1b5655ef7eec99ede00a76987d24f023596fcf534f7ed8b7fe0a7fbb67c4a', '\x01bbc528d05131d8e178c4c49129e3839225ec2c51dc61321d82f5059462b5aa4495aaa112441b49579ae9c59aa5e3be8d94ed483ac8dbfdfa0ab021e6db25ce2b', '5fbb5769c3445e106b634f9fb8fc52fe', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 66, '2026-11-17T04:55:23.949963274Z', '0f6551ac70bc3575', 'create', '\x3000a6883365eb79b5b6ee28e5fb89f2e266a671f80345a3224daa3b20bf693d', '\x012a0a96446416a14c7386eb345a9451a6b803f0d66cc283eaba85955853f74217837fcc7cad652500be8a01618106d553f86f4c57a5ce73ad84d2442e8770028a', '103fc9c1d38be9417b3ed49275d30a21', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 68, '2026-11-17T04:55:23.980203401Z', '0f6551ac70bc3575', 'replace', '\x7e24ed1f05d10be3135c33f92eabff5f413457cce5f9d5d7df65d3d061451c56', '\x0109f73d2c6c4b7cb905d8446b3b3c99411dd3f4ca83ef735e579e5f2278a6570885c946dc0cb2e031832e2f0e2ac679ef344c4bf8d1efef0a06212479aa468479', '71d3eec5c672127f4c3d9fe486ca7245', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 70, '2026-11-17T04:55:23.997236159Z', '0f6551ac70bc3575', 'delete', '\x03ca3130a7d3e2011febf32a6219389a0fed4501ef7aeb76906d1408d66a7474', '\x01b0ae2b48a958f09099a3598be6f7f5802da8639f7119a88e4330e60ee7386ab4568aa25cb6d285c8ec8e4abb5fc54a0c21dd5f22a1d7c4ac2f0e66b9e5923365', NULL, 204, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 72, '2026-11-17T04:55:24.000964511Z', '0f6551ac70bc3575', 'purge', '\x03ca3130a7d3e2011febf32a6219389a0fed4501ef7aeb76906d1408d66a7474', '\x01ad96851134156d4c92d993bfa706ab0fc5cba0e38154eebac240eb259614c79888eaa3a2ac6655534ea121162e9a0dbb28ac3fe0808a9a16fd844b84a27bf125', 'a465da5476283d79f77a5bc3e6b2be00', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 74, '2026-11-17T04:55:24.081168596Z', '0f6551ac70bc3575', 'hold', '\x7d166687403f8479b95991813e6bc74a2c4a18f3d5735701c2a1556abe8f8de6', '\x01cf855ae7e4a5079b17a268fd683288def511b5293ad20e1803e9d4d03c4ff2c99fd34563ef8ab35de1a1b0d1cabdba14579c73d837a51c049a05021ba08983a2', NULL, 204, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 76, '2026-11-17T04:55:24.100556512Z', '0f6551ac70bc3575', 'delete', '\x7d166687403f8479b95991813e6bc74a2c4a18f3d5735701c2a1556abe8f8de6', '\x012fac874b79ebba53f55e7cbc7682f480952c1c7a33dd34de040f915afafb1c187ef576bcca1739d1ae6f8fab990149e2a1946449e8c37c572b01aedecbcb3648', NULL, 204, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 78, '2026-11-17T04:55:24.128672416Z', '0f6551ac70bc3575', 'hold', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x01cada1f7db7aafc695de1ea53dae7dee11873b8ef6d75a340b99e2ad5acc5b2bd99aac9dbda6b5799415f0eb256843c4864b80c2aa78f12ab95fd10003d03aa0b', NULL, 204, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 80, '2026-11-17T04:55:24.17424756Z', '0f6551ac70bc3575', 'replace', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x01d61e173005c54f3ba466ba6d0f0f0a9509544aece7b669665ae5ffb1426fd621e52e1066a233a532290a17e261935bdf622170dbf4e2011c5161241f2623159d', '395d03ceac111c9eef347d6fa604d4c7', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 82, '2026-11-17T04:55:27.992766978Z', 'sweep', 'purge', '\x7e24ed1f05d10be3135c33f92eabff5f413457cce5f9d5d7df65d3d061451c56', '\x01e73e70409ead446c124134d2c51c46ee0f4b254638e28a727e4468cc69079a17346f5f65b41ae8b6b04655d7c9f33d50887d0e805c6a3b4dcb604bf15de6139d', '7e77e67822cade4b94c44ec83c2951ee', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 84, '2026-11-17T04:55:28.048683491Z', 'b1b795d3306e8f08', 'legal_versions', '\x1b2aec2bba13324e4fe8338285293424d3c6d127f400afd5dc3d3d0074c28f9e', '\x014034f9436c3bac7d639d3fc4563bc9dbb3be83f915f7f96646c27aa700588c10c1bda38c22032e52ecd552883b8d61843d811c96270b60f2ae2aac2a8e72c27d', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 86, '2026-11-17T04:55:28.060059518Z', 'b1b795d3306e8f08', 'legal_read', '\x1b2aec2bba13324e4fe8338285293424d3c6d127f400afd5dc3d3d0074c28f9e', '\x0108145cee05338f2fd5f2a1787a6f7ad5c64e02eb66466578d8ffedf0e5800720d11ddb7d2d9a0780d8f2a54199d47ba5a1dde87c8086d8023186dec017d11ddc', 'db457c53adaf5d2e806e5aa0ad4f926e', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 88, '2026-11-17T04:55:28.091537876Z', 'b1b795d3306e8f08', 'legal_read', '\x1b2aec2bba13324e4fe8338285293424d3c6d127f400afd5dc3d3d0074c28f9e', '\x01eec0ef11f8cc48e2189c5b967d260802b7d5e412acd07107fb4fce84e00394032680fbb644aa36757d9eedca83f60d3652ce4c9df8e28d096ec81ad53fb9964a', '031d555152e2909feb97b4d55dce6b42', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 90, '2026-11-17T04:55:28.109972652Z', 'b1b795d3306e8f08', 'legal_versions', '\x0ba172685ff4115ebff559724743b25287ec4f0413b5012a647d746c1bcc7f93', '\x01fbc4f42654d9b1642a9ba38c2d47c39b9cabeb91dee271d0dc20069d24b3f9b941167d3efbafae3a13d78c61640f8890bbb4bf1a7e79f1cdf019bb1293f9312c', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 92, '2026-11-17T04:55:28.197897403Z', 'b1b795d3306e8f08', 'legal_read', '\x0ba172685ff4115ebff559724743b25287ec4f0413b5012a647d746c1bcc7f93', '\x01c3777a27693b8997a8cf3627f16c1c5bb03da48f919a4e9eb18b63cd99135ae627d9d97eee41f1de37dcb5febd8b6c88a23fcfbb2fa136466c366c8668da8007', '1238424ea556358b8906fb4b359897b7', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 94, '2026-11-17T04:55:28.27955858Z', 'b1b795d3306e8f08', 'legal_read', '\x0ba172685ff4115ebff559724743b25287ec4f0413b5012a647d746c1bcc7f93', '\x0184e4b32eeeb11c8fee9880a56ae4df1ddc1b68760121c0822b4306436fa91e88df164bd55ceae7a0ba09eb531fb8f5902e9434b47d6a7c8f7676eb067e2d8d32', 'f73cd8cfa9d89b53f94da0326adf113c', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 96, '2026-11-17T04:55:28.307391531Z', 'b1b795d3306e8f08', 'legal_versions', '\x7d969b9223d17ada8689508b63e2b5da9cbff7e3d1548e957aae0cced7b1fbf4', '\x0151cede3d5239a3f3f9a4cbad99b06923ed273ec20506c81bd64931feb70fdf9842ec5375c40016bf2619c9723521f0569988179ecb0d18c1b506aab7c4975ebe', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 98, '2026-11-17T04:55:28.344798053Z', 'b1b795d3306e8f08', 'legal_read', '\x7d969b9223d17ada8689508b63e2b5da9cbff7e3d1548e957aae0cced7b1fbf4', '\x01632980050cc8f15b8e38514252feceda9f76283a1310e529ca95d7020033f60413795c17094ddfd2a3878e66546512d690899b99e3f824a061b2c6c34610b67b', '876eac2ea831d883cc5abd404ccea6ea', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 100, '2026-11-17T04:55:28.366414078Z', 'b1b795d3306e8f08', 'legal_versions', '\x3013f8cf483637d69b4a079cedd48027e075e605f72da7ffd2a1604cbf1d22f0', '\x013853895e4e074b46c1d5d3bb9226ae74deb40e9eb10d16cebbf3ce11bfd80868f4bbbadd00b99803279bdea719b5a1dbf028afb10e768142ae18ef781bdfba18', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 102, '2026-11-17T04:55:28.387791159Z', 'b1b795d3306e8f08', 'legal_read', '\x3013f8cf483637d69b4a079cedd48027e075e605f72da7ffd2a1604cbf1d22f0', '\x01949f8cf4c1a9f57ee5e3342d66c52f144e70fbb5d04321993f05caa33696fba48ccd6ca6fcc47ca5f82f5c4571001b5fe97500965981e2829e4a545b38be85e8', '7b11f380e6508b39570a276f2036efed', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 104, '2026-11-17T04:55:28.407650758Z', 'b1b795d3306e8f08', 'legal_versions', '\xdb5950483f659addd2326443d4714f974c71b2d159ec6f2214c5d4734349e47f', '\x013e86067900c0bad76ddf1d959460622862d9e551204f7ddeff50e8f182365a145d360f05f8ef7402da945696be534033028c0d08aef084a07f1825c4336efeed', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 106, '2026-11-17T04:55:28.438154338Z', 'b1b795d3306e8f08', 'legal_read', '\xdb5950483f659addd2326443d4714f974c71b2d159ec6f2214c5d4734349e47f', '\x01bf60ed4bb3000bc279439ba81200ce8bfeddbe74d67e8f199580689b0b6e9f7322ebb79ed764d5f569c62e49b0bc0459e4f2d8dcdb06e660ea7ac96b85e0599c', '8553505b7cb6e246a181cd3649808305', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 108, '2026-11-17T04:55:28.451648502Z', 'b1b795d3306e8f08', 'legal_versions', '\x5ff19f373b22a884daa28f15f55635186bf192b843464bc10836aaa80436e346', '\x018f7d8015a75c90290ff9d7e22800f579d5bd6dac4fc29b40679928f1f306cc63b4635a13ae952a8f482eecfe96440353d8cb6362e0ac723330ac06e06731100d', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 110, '2026-11-17T04:55:28.471395379Z', 'b1b795d3306e8f08', 'legal_read', '\x5ff19f373b22a884daa28f15f55635186bf192b843464bc10836aaa80436e346', '\x012f4a9908ac688edbd450b2136aadf7a854b592a2c0be9d7006fd3df9ba49f5d435dcfe3ae93e6975e66f9568f0bbfcd9f1123f5b91047b9b3c0560d19bb371e4', '0d69ab79c8bb6164dbd661abc2957eb6', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 112, '2026-11-17T04:55:28.482623701Z', 'b1b795d3306e8f08', 'legal_versions', '\x603331f2eb7ecc6ae108fb4b5c90a21f4e9f4ea2d96cd3a2b1fb986cd9f19c9f', '\x015f5cdbd636c795dc6e764c184334d183b734a98ac70fe603ffe297b2eea3ef8030b2397d73486b80ba2c44ec7938ca44503a035330cdcf469265922d5327284f', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 114, '2026-11-17T04:55:28.518182503Z', 'b1b795d3306e8f08', 'legal_read', '\x603331f2eb7ecc6ae108fb4b5c90a21f4e9f4ea2d96cd3a2b1fb986cd9f19c9f', '\x016508c599aa66f3d1c6467c03e21fc0db64154068a44a11eb6f03a364ff1780b326d97564380c8e619e0311a7507c7fbd840d20764b7851a2ec3c3dc53780fd86', '8c132144e4c30e3a6e0be6509f91d692', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 116, '2026-11-17T04:55:28.524138734Z', 'b1b795d3306e8f08', 'legal_versions', '\x8628228e413cd3690b97c23939093e6140303c11c4b14d19adb80432676860b0', '\x0100297278507e214ff72a160c53cdfba771e3c3ed02c8e913d327cac9a7ab0cf7493fb246c76d4827e7a29e0d1cea55fa3283c6a0b7108940774305e169f46934', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 118, '2026-11-17T04:55:28.553157767Z', 'b1b795d3306e8f08', 'legal_read', '\x8628228e413cd3690b97c23939093e6140303c11c4b14d19adb80432676860b0', '\x016ed12fbe3425c0cf009282efacfe8aaa7a2378d2a573d47e21ddf6956b34b7c1caf3e2ca4cd1812799b4903fa2dae4631ea8780f0c0ceead3eac6314844be4ba', 'b2a5467f7b1855c7254a63b762a3f831', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 120, '2026-11-17T04:55:28.574696153Z', 'b1b795d3306e8f08', 'legal_versions', '\x71f75083892aa29c849d56d9d052ec2588546a7a72d688ea5c665175c01b213b', '\x015ddceb2eda7ba19623135e49440744167e7c886be957fb036d6c3bfce6a0e80830dbeb78039dfe2fad726e4af52f5fadd452ca58e52ec2f4bd3f75c0c20cb131', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 122, '2026-11-17T04:55:28.613958907Z', 'b1b795d3306e8f08', 'legal_read', '\x71f75083892aa29c849d56d9d052ec2588546a7a72d688ea5c665175c01b213b', '\x019bf2ca30d9f577a0f5baf71990c9925109c171964d972ac1a7a7b11f8b63d456f9c629b16be0e0f3a33f80680bb9e3ed94defb4410bd892dea61cd322b516e80', 'f46fccf7a533c223df31986b12e6a9be', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 124, '2026-11-17T04:55:28.630332617Z', 'b1b795d3306e8f08', 'legal_versions', '\xdaa316e48fae2735eb70e00682013a3da585c1b416326f49df4d5548d6fa9322', '\x0129cd7ba491416c9277923fa3924f13198e008c1ec25e563c6fa3ea1fc7a3e3c89f4695cf7e5020bca48f72309c1d1d909ec41f12d27dccc90d4bd572d3bef2ad', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 126, '2026-11-17T04:55:28.653894579Z', 'b1b795d3306e8f08', 'legal_read', '\xdaa316e48fae2735eb70e00682013a3da585c1b416326f49df4d5548d6fa9322', '\x019c0893c1bffa444ee72e1551dae6fa1608ebf8f63e5b9b2c94a3f026f347d3318c306139b40f5df67da028dcdffb6c730ad8716f0204758c27e8c762beeb61de', 'e9f31220be813ceb8725bf7ab4b8f7cb', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 128, '2026-11-17T04:55:28.661487087Z', '0f6551ac70bc3575', 'legal_versions', '\x7e24ed1f05d10be3135c33f92eabff5f413457cce5f9d5d7df65d3d061451c56', '\x01fd5548081a7211706b2aa850043eaf5ecc996356eccb612fece8cb9bbd3453539642d5f6550b2b407a95ce5f1340af4b56ed1e268350e39116845924abceb138', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 130, '2026-11-17T04:55:28.672663707Z', '0f6551ac70bc3575', 'legal_read', '\x7e24ed1f05d10be3135c33f92eabff5f413457cce5f9d5d7df65d3d061451c56', '\x01111c06b3ac0b5050330a01389802d95ce84c499374372dfd14104069d12acc4d62596dcb2b5251ae2181ab01dca952be552dcc572643ff542ae6193d8e63341e', '71d3eec5c672127f4c3d9fe486ca7245', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 132, '2026-11-17T04:55:28.688275298Z', '0f6551ac70bc3575', 'legal_versions', '\x03ca3130a7d3e2011febf32a6219389a0fed4501ef7aeb76906d1408d66a7474', '\x0107633bbf2f2bf0a7700d0a3907ef2f81f5c1645ea79a59f6d83710963f5609028b601e64cd905aae4d4408977f565e2a3c4c308b0c1cfc6053a86af6a2bc7276', NULL, 410, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 134, '2026-11-17T04:55:28.705351398Z', '0f6551ac70bc3575', 'legal_versions', '\x7d166687403f8479b95991813e6bc74a2c4a18f3d5735701c2a1556abe8f8de6', '\x01dd8e797ad7cef065762ad91c9b7551e13fe5d753ccd34f8c8e81e3f38a554a016b20add80323e8fbc64b35f4480bc655dfc8ab646f432452655abe9dbefc4547', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 136, '2026-11-17T04:55:28.725245054Z', '0f6551ac70bc3575', 'legal_read', '\x7d166687403f8479b95991813e6bc74a2c4a18f3d5735701c2a1556abe8f8de6', '\x01b11f5bd0c652c5e8347a3f6eda7b2371ebe66aef18c27ad27ce989455ef26761c8cc060c2d39934a1dd338b2bb606f3b0a59ba98ac3a03a62c575d27528c5337', '5645241fbc1cb4ce92aa1a55c77dbcb7', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 138, '2026-11-17T04:55:28.737372988Z', '0f6551ac70bc3575', 'legal_versions', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x01a79a92c677ae7ad521451fd86654b6c7757c274ed042f8ce3097afe6f2e54761c67e172f4189a81e83e37e18dc6df2a11f1a00f4a7f205ddd336cbb8f58493a7', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 140, '2026-11-17T04:55:28.770774318Z', '0f6551ac70bc3575', 'legal_read', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x0111346c5ba53d2cbd69c1b23a79b9e3aec773ce9ef1f9cfa99e2d3dc4537f39b1177fb0e457125295be990d16c9b7cb920a3101ec86dff10d52ce12056f9135a5', '26033b4803d02d168c6216cb8200e98a', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 142, '2026-11-17T04:55:28.784937057Z', '0f6551ac70bc3575', 'legal_read', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x016cb8918f3e75d2491482ce1df2ee251c20e0d11795becd5450097124fbfc7471873e233e8653e335d499a85fead2b3b4dd4cd6e810ed7f6736a05131a6ff513e', '395d03ceac111c9eef347d6fa604d4c7', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 144, '2026-11-17T04:55:28.802539787Z', '0f6551ac70bc3575', 'legal_versions', '\xc55c1fee933134319d547e40157a202fd93e21bf3403d3df0c6874c991cc226d', '\x012f202021c876260c52798cc25bfe700dc8644b51f3c7721b03e1a1fa9c7beaa74e5e6dc8be1f8ebaf5284b798c5ae8d263f2b761f51fa5ca060bfd708b2c716b', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 146, '2026-11-17T04:55:28.84105633Z', '0f6551ac70bc3575', 'legal_read', '\xc55c1fee933134319d547e40157a202fd93e21bf3403d3df0c6874c991cc226d', '\x011204c45e8ba0a2c0f1252c8782e162efe6ae6f720bf9d1ecc42b8c26a5a4ea05fb8d75619147242da5cc3c2c74ed4c171573f825fdf8933eec6310b9e1655a01', '5b3de6a3567e01ae208b67ae181dc6aa', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 148, '2026-11-17T04:55:28.847975681Z', '0f6551ac70bc3575', 'legal_versions', '\xd4bf1d7c88504e9fea648ccca58b2d03cebfd68c67270e1cfed179ed67f166f9', '\x01dffce8c5abf301394621794fe4c13555d30307e2d274a1e709b35295276b540dafa1b0bd710a3751ef8ac4779fa313f77e757d89cd9b721e93eeb12c612af23c', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 150, '2026-11-17T04:55:28.856337217Z', '0f6551ac70bc3575', 'legal_read', '\xd4bf1d7c88504e9fea648ccca58b2d03cebfd68c67270e1cfed179ed67f166f9', '\x013d72337debf0069075deace446020d6d560cef6c2f942b525bf697ac10139b5516212ec500e5278e90c902c24d7c46559529ac605a60e2e018c7736648f727ef', '1e297f90b3e7e1ac6689dfb2fceb3278', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 152, '2026-11-17T04:55:28.871909346Z', '0f6551ac70bc3575', 'legal_versions', '\x1d63ae5031147227289dad8893c831738aadb63f6a7994dadc750a777309c4d2', '\x0148212795b982574d4bd3527d61826bca73234d4233ed6dfd0fa1e6e8abd356eab14ad0d25ed9992e4c7b40d4fe0ca407f1df66e61b60208b4e7dfd7ea2020cae', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 154, '2026-11-17T04:55:28.885728002Z', '0f6551ac70bc3575', 'legal_read', '\x1d63ae5031147227289dad8893c831738aadb63f6a7994dadc750a777309c4d2', '\x01d509381bf22c63e47405fe02c37a2bb48f7b9ed100840ad0baf1bd2a74c17b08bdb73f5d91cedd81610259dc36574ffe95a60ca98f5dd7141de5842e744c6412', 'da747f567345590fd555e3a0e1f307f8', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 156, '2026-11-17T04:55:28.907161425Z', '0f6551ac70bc3575', 'legal_versions', '\xa78bde54008e62bcc5a78d086d64a149172031d1b71543dbd8e3563b6a9bf497', '\x0192a94ebb33aa23fec75069dec889b9a01d92d799e4b007486ee57388a62a3e3cc821013d4b622048bc0d5173a651d843c9e4616624b82b41b141cca6d842590c', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 158, '2026-11-17T04:55:28.927142682Z', '0f6551ac70bc3575', 'legal_read', '\xa78bde54008e62bcc5a78d086d64a149172031d1b71543dbd8e3563b6a9bf497', '\x012ef81facf8b296d2d88f8253ff67d867dba5a19a3764e2a6e0425da49ca8f465a32eb8c2e7111b66f972700b91bd4eb7daf28833795c774f01ba24fa365305fe', 'dec093f719f1bf93fbfaf63ff7397008', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 160, '2026-11-17T04:55:28.937732366Z', '0f6551ac70bc3575', 'legal_versions', '\x54d1b5655ef7eec99ede00a76987d24f023596fcf534f7ed8b7fe0a7fbb67c4a', '\x01a959fab33477d77f27a95b18e6fc297105738c6369c6e749bd835d7351f179999016910aad33fb51c2c643ae1d517407866c1ae6b255ff26a280f57ac458b90b', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 162, '2026-11-17T04:55:28.953853169Z', '0f6551ac70bc3575', 'legal_read', '\x54d1b5655ef7eec99ede00a76987d24f023596fcf534f7ed8b7fe0a7fbb67c4a', '\x01971f8f8461d2e796fb316967fd2e349b21ace76d113e68e9a04cd90e513a8e75f888a604966fd836ec17ea865be09098e0f56233f25cd83a6cd748898fb0ddcf', '5fbb5769c3445e106b634f9fb8fc52fe', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 164, '2026-11-17T04:55:28.968201946Z', '0f6551ac70bc3575', 'legal_versions', '\x3000a6883365eb79b5b6ee28e5fb89f2e266a671f80345a3224daa3b20bf693d', '\x012f0eb87d6e7a0fdf589cb4d6fcebbd53cea98a56c83aa22c0822e0669f24627094cd40774ac64e9239cd5827cdc31b747f3bcfcf5f2d74d50f389a69e048d3ee', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 166, '2026-11-17T04:55:28.976105019Z', '0f6551ac70bc3575', 'legal_read', '\x3000a6883365eb79b5b6ee28e5fb89f2e266a671f80345a3224daa3b20bf693d', '\x011e22dc2ee30a01654172634aab9cc462746e365e44bdb6519abdd334ab26e12e28659b4b08ae34cf66f9ff4e5243ec7a0f625d9d58afce1b84b49864c683a454', '103fc9c1d38be9417b3ed49275d30a21', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 168, '2026-11-17T04:55:36.02154731Z', 'b1b795d3306e8f08', 'create', '\x2ef283ac494e09d50a39443fb0cfb15f2e8bc6279fcaaf1d05c354e2c46a6dfe', '\x014a25e83dd8661d77525cd345c9c9dbadf9063f23982e4ce322bd169511eeb0f3461151d994e7791621da79047f9c6f31ac81f3dfddf9217096874bb86e1d3818', '5e4e4d7877f9b81d32d68f9e1d101e08', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 170, '2026-11-17T04:55:36.117498741Z', 'b1b795d3306e8f08', 'create', '\xc673365a5c9458be56e1910d7250994f9fdf2b5d4eda2ea4b148a2e1e23fc985', '\x01499d34ed6df79091fa59a6ea4e943bd56fc3e3fad2228be64ce7f4f16a303123e756f088b1a66fc22a2622fc668ba430db0ce5299ba2d6a4f68cc5e884a2e76d', '0b05d77fd6278ea50430aef83a85def1', 201, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 172, '2026-11-17T04:55:36.182326733Z', '0f6551ac70bc3575', 'create', '\x3459503683e930983e2bfd2b00627e943471cb2ba1cd6ac311f81b902fd7e6d5', '\x014ddf555622c38716d6e1f85af503c397d5d231aa8b2567b9ec5a4024f89f6a264ab0cc5b99833fc51fcb94405680cb2ddfff89e760cfd206c747a838117e3e1a', '6541c46cfabd21c9df52e8b578ded2fe', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 174, '2026-11-17T04:55:36.265210017Z', '0f6551ac70bc3575', 'create', '\x01f4a299896db3a5eae673dddec22cb061563f90b3bfcc73acfd8c1021eaeb9c', '\x01e18607e8f6093170e49fae6974337897bca54805d97c0993a79ca62aae648a8e8b725047724ed4e11d7b6a48782f53f071e80884174618312ac004fb6f1495f2', 'bb7a9b768258642ccfa07af7f2085ffd', 201, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 176, '2026-11-17T04:55:36.280531977Z', 'b1b795d3306e8f08', 'legal_versions', '\x1b2aec2bba13324e4fe8338285293424d3c6d127f400afd5dc3d3d0074c28f9e', '\x016442b154e99996bee59f6317f7a62c6b9143d8930dbeb5cd1eaf1b019baa80d317dcf786dbf85b0302716d5e1e69f7c71c8a95036936c15f4b4ab2c1c6bffa4c', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 178, '2026-11-17T04:55:36.321763972Z', 'b1b795d3306e8f08', 'legal_read', '\x1b2aec2bba13324e4fe8338285293424d3c6d127f400afd5dc3d3d0074c28f9e', '\x018fe501262360c7496006d52bbc647526c5abd62c460cc9b622fd2eb4ea1b99f89afa7fb3b1ed2d4d77b0f7b6d3514ee122a542181505c0e9f7e287b5c2ff2e54', 'db457c53adaf5d2e806e5aa0ad4f926e', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 180, '2026-11-17T04:55:36.347022477Z', 'b1b795d3306e8f08', 'legal_read', '\x1b2aec2bba13324e4fe8338285293424d3c6d127f400afd5dc3d3d0074c28f9e', '\x01b31b4f2761fcadef886a7b09e7ec550d983208e79c825336e5154551fc46bc3048e0c45712ac1db66209e7667a8e59e4bfca0c213b457e816f91c20b62691729', '031d555152e2909feb97b4d55dce6b42', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 182, '2026-11-17T04:55:36.362886585Z', 'b1b795d3306e8f08', 'legal_versions', '\x0ba172685ff4115ebff559724743b25287ec4f0413b5012a647d746c1bcc7f93', '\x01ccb7e1323980674de6262ebb23e21dfb5e867c25b862bd2c5587a83973c13e126ea1203527123d31b8e184d850389034ee0f5dcbc0f03160e6334e2dc592f1fc', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 184, '2026-11-17T04:55:36.385127406Z', 'b1b795d3306e8f08', 'legal_read', '\x0ba172685ff4115ebff559724743b25287ec4f0413b5012a647d746c1bcc7f93', '\x01b58618e853804aab29ea3ec1a1f58dbfd91af096ada9bcec6141b089fda1d6237c1b4983c2564ba0248802c8bb80513ba9ae2e1c2f0efea479c2542812fbaa28', '1238424ea556358b8906fb4b359897b7', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 186, '2026-11-17T04:55:36.417072031Z', 'b1b795d3306e8f08', 'legal_read', '\x0ba172685ff4115ebff559724743b25287ec4f0413b5012a647d746c1bcc7f93', '\x01aa3037ee5ba82c298a7423f48aeb6893a9937983191ab58715df080fa1de39d4c510d69fa49c3e412678683ccd88a6ff73dbe372314b4657ba9a8f2b7656b0ee', 'f73cd8cfa9d89b53f94da0326adf113c', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 188, '2026-11-17T04:55:36.468326923Z', 'b1b795d3306e8f08', 'legal_versions', '\x7d969b9223d17ada8689508b63e2b5da9cbff7e3d1548e957aae0cced7b1fbf4', '\x0154f0711f00e2be15f88957c0ade4e74b2bdb7bc261fdd1b61e8609ba7f3a6bff5325468f403c77803d74ad1aed0aed219560c47ce97f65240c8b3822f04a89d8', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 190, '2026-11-17T04:55:36.484070457Z', 'b1b795d3306e8f08', 'legal_read', '\x7d969b9223d17ada8689508b63e2b5da9cbff7e3d1548e957aae0cced7b1fbf4', '\x01379273409aea6425bf9ec646da4ac0d3326f09f93e015c21a9d84c3c4a205cc2225c1f2d8f710d938d2d64cdede7d53168fa3267036a37961be5e11b6911af25', '876eac2ea831d883cc5abd404ccea6ea', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 192, '2026-11-17T04:55:36.505804667Z', 'b1b795d3306e8f08', 'legal_versions', '\x3013f8cf483637d69b4a079cedd48027e075e605f72da7ffd2a1604cbf1d22f0', '\x015cc966f1d9f4bc7722447bb4b5607fc443e1fbeab4c48876b0bf8b85afb986c203d3d73d612c1962101cae8b72ddbfbd71179c5727b99db6d8db54d757e9fb8b', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 194, '2026-11-17T04:55:36.526455692Z', 'b1b795d3306e8f08', 'legal_read', '\x3013f8cf483637d69b4a079cedd48027e075e605f72da7ffd2a1604cbf1d22f0', '\x011dde38caf21dc006b85b5b0f85c79017699056c926ee918e06ab5343d19fbc22c0963e461ee2ee7d674577076407d07057756c25815a3d3387c920a29828eea0', '7b11f380e6508b39570a276f2036efed', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 196, '2026-11-17T04:55:36.544890932Z', 'b1b795d3306e8f08', 'legal_versions', '\xdb5950483f659addd2326443d4714f974c71b2d159ec6f2214c5d4734349e47f', '\x0118bae81aac5ee1b7183045a43e40ccec8f953bb7c45b719d150b4cc483e6e9718f5bf382e9406974c9ec84c954d4d581931c4dfb4e40189d065572a8b56753f7', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 198, '2026-11-17T04:55:36.572447567Z', 'b1b795d3306e8f08', 'legal_read', '\xdb5950483f659addd2326443d4714f974c71b2d159ec6f2214c5d4734349e47f', '\x019ed51dbfcb5a6ddb4c40ee16f145341c77d35ba6302ed40acda55f107750c3cc3cc5bb37b2c5d253693c37ed3a803f2372fce11549ff695ec2207737c3701e80', '8553505b7cb6e246a181cd3649808305', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 200, '2026-11-17T04:55:36.5848309Z', 'b1b795d3306e8f08', 'legal_versions', '\x5ff19f373b22a884daa28f15f55635186bf192b843464bc10836aaa80436e346', '\x01106e4abcbc9e0dc04b5103f9b5e9d9dd23ef9ed7ff1992bed712e924896a65bc99d29d1cd45f2f3c6f096c2e07cc9a91206de798f65fbf6e842e5508c30d51e0', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 202, '2026-11-17T04:55:36.630516654Z', 'b1b795d3306e8f08', 'legal_read', '\x5ff19f373b22a884daa28f15f55635186bf192b843464bc10836aaa80436e346', '\x01a475bfa7d4c6161b98f98f61dfa3043c7ddf8c972f00445805122ea9509cca5434f30b378dbe836cecb4b8e8ab385f24389c045114454242316c089f3f88169f', '0d69ab79c8bb6164dbd661abc2957eb6', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 204, '2026-11-17T04:55:36.645438625Z', 'b1b795d3306e8f08', 'legal_versions', '\x603331f2eb7ecc6ae108fb4b5c90a21f4e9f4ea2d96cd3a2b1fb986cd9f19c9f', '\x01fba305fd615266c0fec554fa11fbe32e1e65aebf2f41b4b8d8a92947ebe84bcc7976761437b45a9825f95db7db7fb0257a9f16bdc5bf8e46b257ef7f151cc269', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 206, '2026-11-17T04:55:36.668953224Z', 'b1b795d3306e8f08', 'legal_read', '\x603331f2eb7ecc6ae108fb4b5c90a21f4e9f4ea2d96cd3a2b1fb986cd9f19c9f', '\x01ae47fcec241e69b70ddcd0f532faddd4a41b0cc037ff780186b586bcee56a0f416a5d866fa765a0406d1ed432abfe0b95f7dd43cb35be08556da867f3c55b83b', '8c132144e4c30e3a6e0be6509f91d692', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 208, '2026-11-17T04:55:36.686703842Z', 'b1b795d3306e8f08', 'legal_versions', '\x8628228e413cd3690b97c23939093e6140303c11c4b14d19adb80432676860b0', '\x018168877d4882a13bac044dd557eecca8a85f57916c0071c811512026800eb8b43556bc491302f67cd2ba9df892a5193fe41425f41dfe5783b902f7acd41b2c23', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 210, '2026-11-17T04:55:36.698636156Z', 'b1b795d3306e8f08', 'legal_read', '\x8628228e413cd3690b97c23939093e6140303c11c4b14d19adb80432676860b0', '\x01885bc036f9223fd9ddc0b7de882983a629cd4de12cbc501b29c6ceb6a88e57efe5b7defac94ffb0a4e251f948e60c4f765dbed7bc7526bd65750b163edcad325', 'b2a5467f7b1855c7254a63b762a3f831', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 212, '2026-11-17T04:55:36.731823042Z', 'b1b795d3306e8f08', 'legal_versions', '\x71f75083892aa29c849d56d9d052ec2588546a7a72d688ea5c665175c01b213b', '\x014a1c30d0ae1c90547882c8d5b4a9e83030e9d4833113715ab55589fd9aaca50034aa69005935850b300edb17ae1f25820bef400e17dc98c8e09a71dff5c95272', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 214, '2026-11-17T04:55:36.747260151Z', 'b1b795d3306e8f08', 'legal_read', '\x71f75083892aa29c849d56d9d052ec2588546a7a72d688ea5c665175c01b213b', '\x01cea8cf6969a33c9d3b079618ce17f559d95efbd0b0fcdf45538d2a2df8307fe1b2343dcfb1b818c7b0d27cd3c156333794c91578e29b7b3735ce88b4b8144ca5', 'f46fccf7a533c223df31986b12e6a9be', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 216, '2026-11-17T04:55:36.763328612Z', 'b1b795d3306e8f08', 'legal_versions', '\xdaa316e48fae2735eb70e00682013a3da585c1b416326f49df4d5548d6fa9322', '\x01707dce7f414d8d5836ef6a8bc2d8764d2abf497db048ce70dc879a2d459b08efcdf96dd2492c8d89be97afd72476ad724c104d4d583aca936cc67521b0486d13', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 218, '2026-11-17T04:55:36.789350145Z', 'b1b795d3306e8f08', 'legal_read', '\xdaa316e48fae2735eb70e00682013a3da585c1b416326f49df4d5548d6fa9322', '\x01185db1784e9ef8b5082c14e4e7d6caaa327b2a6635bc9eee9ea3dea7ce4a71817533e6313d6ec9ebdadd156c71087c632d55642138e2f32c450545e06aadf0e9', 'e9f31220be813ceb8725bf7ab4b8f7cb', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 220, '2026-11-17T04:55:36.802891384Z', 'b1b795d3306e8f08', 'legal_versions', '\x2ef283ac494e09d50a39443fb0cfb15f2e8bc6279fcaaf1d05c354e2c46a6dfe', '\x01d0be2b8582d3dfef84f5d34d205466dcd10edc29df5e51587fd9d31b525c06bf587cc64c5267f2aab10f817b656bae94bb4497d0c513565a060efcfd2113623d', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 222, '2026-11-17T04:55:36.870290495Z', 'b1b795d3306e8f08', 'legal_read', '\x2ef283ac494e09d50a39443fb0cfb15f2e8bc6279fcaaf1d05c354e2c46a6dfe', '\x01d9fc8c30b4ac0b2901803e33ef605b10d1a0acb890a31407ed0f2f6c7313c6b657b4027d5917e04358261b2bb9be5f6de0cd275825dbb0a0ab092dd2ccf1db6c', '5e4e4d7877f9b81d32d68f9e1d101e08', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 224, '2026-11-17T04:55:36.910184405Z', 'b1b795d3306e8f08', 'legal_versions', '\xc673365a5c9458be56e1910d7250994f9fdf2b5d4eda2ea4b148a2e1e23fc985', '\x01739984f66b5056d1bc601fd74d5d53675ad49a3d08f05ec21f8dd4491f352bd486e676d6e175d47ebdf81bbebc03ae5f70ee47a63bfb77b884227770a26fae22', NULL, 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 226, '2026-11-17T04:55:36.962048917Z', 'b1b795d3306e8f08', 'legal_read', '\xc673365a5c9458be56e1910d7250994f9fdf2b5d4eda2ea4b148a2e1e23fc985', '\x0193849a9222208fc36f42b3fb4664a906e9c088b42908fabbac35c5601412cd64b8d7a3cce01b8783b42d80e92f6f8bd75b2b5d76e3bc738a08dfc1b687d3149d', '0b05d77fd6278ea50430aef83a85def1', 200, 1, false, '\x9ff9d0d3d83cc6a1');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 228, '2026-11-17T04:55:36.976491026Z', '0f6551ac70bc3575', 'legal_versions', '\x7e24ed1f05d10be3135c33f92eabff5f413457cce5f9d5d7df65d3d061451c56', '\x01408f622fadf2e66aa704c2f4c0f7312cc0789d8cf4336a05e268dd838313a31150599a034689cead33aee18ed337123a6032f82af04f2fabf2b71c75689b41c5', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 230, '2026-11-17T04:55:37.009891272Z', '0f6551ac70bc3575', 'legal_read', '\x7e24ed1f05d10be3135c33f92eabff5f413457cce5f9d5d7df65d3d061451c56', '\x01c125c8ed8b4c3fe3a182b70dd28312a4be31e8ce5c32812737550832f6d3e76302d904716b8190ad6041de9c7a5c60dbb46668b57191cdf4af3960c79171c33e', '71d3eec5c672127f4c3d9fe486ca7245', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 232, '2026-11-17T04:55:37.035852915Z', '0f6551ac70bc3575', 'legal_versions', '\x03ca3130a7d3e2011febf32a6219389a0fed4501ef7aeb76906d1408d66a7474', '\x0134f929956f8c65c3614f14b1cc9c390ecc40fb916972200e6b85c81255849874d780fe11724b4aecb374567db5f0bf5767374a499423736213b6999b36b7e7da', NULL, 410, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 234, '2026-11-17T04:55:37.068532531Z', '0f6551ac70bc3575', 'legal_versions', '\x7d166687403f8479b95991813e6bc74a2c4a18f3d5735701c2a1556abe8f8de6', '\x01cf9f6efa0f4b33c10716397af7c3c2e63cfa30fe1c31c3164628a59f1b15c061c6477138626fe28975a30368fb658b838424b0fc91a9722ff40c73b9363b0a91', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 236, '2026-11-17T04:55:37.095092277Z', '0f6551ac70bc3575', 'legal_read', '\x7d166687403f8479b95991813e6bc74a2c4a18f3d5735701c2a1556abe8f8de6', '\x01c45e1b3394a0394a011f226e493fa6400d30b4f4fee6757077af14cbb5c3d238e8172f391838ef6723e7d5794b31a86dd7c85680a3e0b975aebe08757077df96', '5645241fbc1cb4ce92aa1a55c77dbcb7', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 238, '2026-11-17T04:55:37.106969609Z', '0f6551ac70bc3575', 'legal_versions', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x0119cd5e197ebab7380744c4de7e40cf4bc8dc43e4384ce1018aa09d725b16259e943140bb206241e1e85f8c9a79c4401a99434d08a7954e9abfcb418cb8879fa6', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 240, '2026-11-17T04:55:37.135939938Z', '0f6551ac70bc3575', 'legal_read', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x01a308e6f6bee391fbeb727ececa094c007e6eb3988d3c3f2ec358d27ad2b3fededba37033ba6288948f4c4d4ccf8cc264a5612d61bdc462cb89a9b4ecb84400fa', '26033b4803d02d168c6216cb8200e98a', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 242, '2026-11-17T04:55:37.179743211Z', '0f6551ac70bc3575', 'legal_read', '\x1362da0f1fe0d763995e2c0fa0ff2c94611a41a99169683728257b9a1b024da6', '\x0148156875fda3cd9f52023c12cda85cc72932618412219e6d79cc3436a8324d2a9473d1eb5222e9ca3c8d4017379e1a77da64ad19429b8f944e217e28b642c2a7', '395d03ceac111c9eef347d6fa604d4c7', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 244, '2026-11-17T04:55:37.198034219Z', '0f6551ac70bc3575', 'legal_versions', '\xc55c1fee933134319d547e40157a202fd93e21bf3403d3df0c6874c991cc226d', '\x012a0175d088a857e41514cd8df2d5c2bb30203abf882e887ec58715e39e39c9d7858e3a9a66fbfc11b1e3f2d2c1939128f9f03b5c4f42b43997775258dc8d89d2', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 246, '2026-11-17T04:55:37.211158587Z', '0f6551ac70bc3575', 'legal_read', '\xc55c1fee933134319d547e40157a202fd93e21bf3403d3df0c6874c991cc226d', '\x0185f6bfa66eb17e0e6cefe2bd7c051180fd2f0a4d743577cacc19f18cb990ba45251b3d2d5553c5c3726612bc35aaf6e7be7abf9ecca291501ac055165fc66303', '5b3de6a3567e01ae208b67ae181dc6aa', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 248, '2026-11-17T04:55:37.221186735Z', '0f6551ac70bc3575', 'legal_versions', '\xd4bf1d7c88504e9fea648ccca58b2d03cebfd68c67270e1cfed179ed67f166f9', '\x01f505bf4ffffac47ba52e00daa9312fb451eeb0f9947615a781f2030fc8f0c07c4b4aee37095880576875c6250de8f7786da6a0a4c88ef555c7981920bfbb7087', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 250, '2026-11-17T04:55:37.240333417Z', '0f6551ac70bc3575', 'legal_read', '\xd4bf1d7c88504e9fea648ccca58b2d03cebfd68c67270e1cfed179ed67f166f9', '\x017bc4c69cca64f4ca1e1f6522850d7657ac2906cfabda3457c37a55b651d9a3c9428a655d575c6cebc7061dd58a0b719bb84ad0fb9ee2d0ec55efb8dcb7141ad5', '1e297f90b3e7e1ac6689dfb2fceb3278', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 252, '2026-11-17T04:55:37.247794454Z', '0f6551ac70bc3575', 'legal_versions', '\x1d63ae5031147227289dad8893c831738aadb63f6a7994dadc750a777309c4d2', '\x0144dee0be7a551de2161ad03b3b12f776c7f6ff04f1876ff5cbcddcec9bd3bd50b7ecf4ea34b7a8780ab64ab3e05a65b93fa2016eaba2bdcaffa1c160f0fb11aa', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 254, '2026-11-17T04:55:37.257246147Z', '0f6551ac70bc3575', 'legal_read', '\x1d63ae5031147227289dad8893c831738aadb63f6a7994dadc750a777309c4d2', '\x01d223c82ee182f4881c071cf80831ce21ec50beae72b7c1797ebc627a57e801920f53f8199280d3a7adb88eafeb2baede8467c4f915e9245cf1326922245889c0', 'da747f567345590fd555e3a0e1f307f8', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 256, '2026-11-17T04:55:37.264391359Z', '0f6551ac70bc3575', 'legal_versions', '\xa78bde54008e62bcc5a78d086d64a149172031d1b71543dbd8e3563b6a9bf497', '\x01dc4ef61e8d08fd93312bf45df3668f469c945cd943b657b5e978b893227ffc71daf27adb8b184617afad0f2155bdd13bfff850566e1d41e534c86c4b1e1a0f97', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 258, '2026-11-17T04:55:37.277513256Z', '0f6551ac70bc3575', 'legal_read', '\xa78bde54008e62bcc5a78d086d64a149172031d1b71543dbd8e3563b6a9bf497', '\x017c46c1a11efc90f6ee1df060827820ec90fb01254354e367876e5cb84de1d8256fa3152650b0a7564cec3a28a2fbcb138d92c86817f921f4fd5dce0d2e7291d5', 'dec093f719f1bf93fbfaf63ff7397008', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 260, '2026-11-17T04:55:37.297873642Z', '0f6551ac70bc3575', 'legal_versions', '\x54d1b5655ef7eec99ede00a76987d24f023596fcf534f7ed8b7fe0a7fbb67c4a', '\x01bf426640ca002cd9689cae1477dd168349202c432724b081f6bf879301ab870d15bc550863329d079af223a7b4d00b4c72b3da00d6254dfe329438cbd09fc7b3', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 262, '2026-11-17T04:55:37.333715408Z', '0f6551ac70bc3575', 'legal_read', '\x54d1b5655ef7eec99ede00a76987d24f023596fcf534f7ed8b7fe0a7fbb67c4a', '\x015573be1c61f90740093e09bdc62772ba4fcfe6835b64a03c58895258cdf7359494e4d078f942fd4b3129e73669b07e29067fff3592dc8a4ccdea40a4b937ec70', '5fbb5769c3445e106b634f9fb8fc52fe', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 264, '2026-11-17T04:55:37.358657498Z', '0f6551ac70bc3575', 'legal_versions', '\x3000a6883365eb79b5b6ee28e5fb89f2e266a671f80345a3224daa3b20bf693d', '\x01be192d28995b400c9fb3078e3a58d201a9f0a0d70956c173c396e996039f1497ae90fee65db05435194c7a981d8d6dcef7229744524c9d0a4a892f4af354fb63', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 266, '2026-11-17T04:55:37.391601651Z', '0f6551ac70bc3575', 'legal_read', '\x3000a6883365eb79b5b6ee28e5fb89f2e266a671f80345a3224daa3b20bf693d', '\x01e6e60ccf912c571828f04bc7a399a40307e4df0c6ae7c732f6dbb1cb73bbe787bc448f03744a30442357cf83f5f6aa68931221ed45c7f365a082eee853bd06c5', '103fc9c1d38be9417b3ed49275d30a21', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 268, '2026-11-17T04:55:37.413592453Z', '0f6551ac70bc3575', 'legal_versions', '\x3459503683e930983e2bfd2b00627e943471cb2ba1cd6ac311f81b902fd7e6d5', '\x0131dcd829412523527081700c59d6d2b2580f57a447d45c12fde2ad0967db8623a44466b30da169c88dd239f3096749f5fcfe66a4f9d5c902c402c07b0bcb32ee', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 270, '2026-11-17T04:55:37.438291274Z', '0f6551ac70bc3575', 'legal_read', '\x3459503683e930983e2bfd2b00627e943471cb2ba1cd6ac311f81b902fd7e6d5', '\x019e9ff143f00a1c86de3cca47c5b432b3244355f6a430a16fd7c5a68cf7ac701b42e891c80f8be3ecfa4679f76d23cf8667b6b7944dbf1478c39e08441bebdc99', '6541c46cfabd21c9df52e8b578ded2fe', 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 272, '2026-11-17T04:55:37.46938689Z', '0f6551ac70bc3575', 'legal_versions', '\x01f4a299896db3a5eae673dddec22cb061563f90b3bfcc73acfd8c1021eaeb9c', '\x0115317d464975ffb6d67e6128b4464839be7a294f675805431f2be8c841d4431d78fcb237fe9df365d5f9d5793d923f53f401c959a2cc9a11599d2107fe7922ee', NULL, 200, 1, false, '\x7f5e7c91cd6020ef');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 274, '2026-11-17T04:55:37.495547062Z', '0f6551ac70bc3575', 'legal_read', '\x01f4a299896db3a5eae673dddec22cb061563f90b3bfcc73acfd8c1021eaeb9c', '\x017fb49db17a2a40103c21a0039e61f9aff90ce6576143cf5a656fb7add052ef3fe5d97d59a89af571370ecc197e86aa4e48c6976abad726df2a9fed338483163c', 'bb7a9b768258642ccfa07af7f2085ffd', 200, 1, false, '\x7f5e7c91cd6020ef');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x9ff9d0d3d83cc6a1', 1, '\x01dfc3d8e2da91366f45847e71d2c545967f4875d350340d8a0265e60f69ee65429bff0239e6222b318d1ed978af6113ee26c14472d934e4c08a6c2dee');
INSERT INTO vcon.audit_key VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x7f5e7c91cd6020ef', 1, '\x01a1ab24ffa30d1cbf5e33ea4f2c8ea403119a78470fcc82ac79233b1780423bad0b986c8e63e2c6bc80ba04dc7d2535acf19d025eb633f9b237ee3495');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x946f17fae0008e89', 14);
INSERT INTO vcon.key_use VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x14fa3b0212c67360', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', 2, 1794891334133614563);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', 4, 1794891334995253761);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\x59c39d8edaebbab6', 2, 1794891288581139688);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\xeaa017918212158343ae6b4ab4050941', '\x5aa55f9947674e0eb8acc35d5dadd218', '\x01d570f2cdd111822683dc9918e773f02470a662260e73dc17ec487781d4686f5ea5ad', 1794891317615569684);
INSERT INTO vcon.membership VALUES ('\xf800bd8dfad0f31fefabc583b33efbaf', '\xb50dd7af538c83a987d0889f76e31d4f', '\x011adba0711513f34c3b0784fef32e375d9b93c05ff098953e3230842725994b1183e9ad', 1794891320489157334);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x7764dbf56e35264e5e9f4150d9aae31c', '2026-11-17', 12, 12, 7404, 1794891333856658750, NULL);
INSERT INTO vcon.parquet_file VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x821c58287f69b6fd62029a09acb03558', '2026-11-17', 10, 10, 6240, 1794891334207156788, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x71d7f4bab41c858c');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-17 04:54:46.554278+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-17 04:54:46.593269+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-17 04:54:46.600688+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-17 04:54:46.60138+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-17 04:54:46.603973+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-17 04:54:46.607587+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-17 04:54:46.608294+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-17 04:54:46.61618+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-17 04:54:46.619135+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-17 04:54:46.619776+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-17 04:54:46.621067+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-17 04:54:46.625283+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-17 04:54:46.632729+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, 'b557f7618436aaf37854ebeb1b815bb7');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x6720ae2f8e44c0d554cdc33381192871db83a30ed7b33c3732d2bf3154d85d59', '\x01500c0075667e5f9f4bf255c35e592da24dfa89916715dae07c93c6e29c25c79eca6673be', NULL, NULL, 1794891288601149307, '\x946f17fae0008e89', '\x01011b71bef794583b0f6e6e5b7ce2b173c519312d3d3b6f3a06c42e069413667369cd1591009c20621e8f08836eadd6915c0b9f90e327537dedd67aac', NULL, 1794891288601149307, NULL);
INSERT INTO vcon.tenant VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x8640ff0c4b15f078613a4aa874267d949efc032092506bb156b6ecff5c24659f', '\x01c02e08099d46a83c6ef7c8781a9c4f6676096b5eefc60553e9ce953fb34c8771cfc99704', '\x01086301a8e7c717e3d8da62bbe71fea05c5c433a773dffc41441aa99f7754', NULL, 1794891295402786809, '\x14fa3b0212c67360', '\x01077382f823568646bad5877ebe74a2d56381824084489b58f889e6ead143d4bbe413610398afa4ddf6336ddfddcd3f10d396876c63af2f60fd839142', NULL, 1794891295402786809, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xd7b2a390d4a47b30');
INSERT INTO vcon.tenant_catalog VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x428cd50f3e0d129f');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218');
INSERT INTO vcon.tenant_lock VALUES ('\xb50dd7af538c83a987d0889f76e31d4f');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x5bcbfbf41fe827af375cc44f07832ab3f73c9896612355f14d772fa0510a6fb1', '\x0172448111d5f50d5f6cfdf1146be28d41fea11a1c29dc2156d51ae002ee28e6e64ccd3ce8d85962a13f648b75', 1794891323999327233);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\xeaa017918212158343ae6b4ab4050941', '\x0196bee126745fff9a358a81546f99ec389cb2e72ca6662c730405bbb85adc418460553744d28b244c7109', NULL, NULL, 1794891310931612697, NULL);
INSERT INTO vcon.user_account VALUES ('\xf800bd8dfad0f31fefabc583b33efbaf', '\x013088870c7f9e1949a8b1c68eda548477cfd620d016f6693297e4bf4ab3bf36804528bc0eab1d24b9ec1c3b', '\x327bb54154f705c1c9b4b8ff161110e8e01097d0e59e549aef9f57dc614efc82', '\x01f0b168481701843f2b1237b5332e25ee1e1aec844b1c9837bec9244eb557cd1414c296ef575e7722018d891ab00836fc31b7bf9d4ec22b9961193dfb6a677f', 1794891313767261356, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x99286fabb79ccc165c254350ae30615400ef1eb99998bcf28f2ca19122fef98a', '\x01723288084a46ecb3f6e6eaaed6e952360ace72499644711fa366259d81b79e2a5e85b302e6a2bfa05dd04448', '\x8c132144e4c30e3a6e0be6509f91d692', NULL, 0, 1794891323364120990);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xdc8061d5891106b258c7ebc9deccce59a4eb22051d5ed9845119f73b25537fd7', '\x01eec04775cf6437ea80a4ec3bc8795a7118de4083420279bf3e8cf22b6b1f695ddc91b016e3c80d423fa9500b', '\xb2a5467f7b1855c7254a63b762a3f831', NULL, 0, 1794891323385335350);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xf490ab4dfa3b0c9a26f55fa65b02955b182c1da6b6875a393dd2adf5713522be', '\x01cb19e12159131f0f015c9ea1a0bd6a4e7e85b292d89bd5ac5d962a15266af9c64651890236d8f231406d7afb', '\xf46fccf7a533c223df31986b12e6a9be', NULL, 0, 1794891323421237790);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x8cd3b5d9b1df1c3ca972bc20004194b1ca32789c4db997b974cc0473fd6caec0', '\x0152d2b8c1fb4cdf4c73e7ea9da27a1c3eeb4588664a93e618b73be20494799203749234e341adcfd3b58e1fe8', '\xe9f31220be813ceb8725bf7ab4b8f7cb', NULL, 0, 1794891323448219620);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xbe17e886baf63efae0a263c5a4c76c95d753fa2bed6fff43c2160dbce4eeaaa1', '\x01edac8c35e62966e6fe43d156caba011a771f7fb2acb6bb9a5cee8249e2ad54e635a368c1497ad0f703ff14ef', '\x031d555152e2909feb97b4d55dce6b42', NULL, 0, 1794891323150396415);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x3ff3676cbb51a7c9f24e7f4a7cff8272cdd32286aec3ded719848deb9645a541', '\x01522b891e6e8c27bebf89786327477d827d0ca951ce39d9cf59c5a4a9216d69db30d556100b3e71bb54bb1935', '\xf73cd8cfa9d89b53f94da0326adf113c', NULL, 0, 1794891323204693994);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x4e90b62569969561ad3fb47f16f5483c18f1a70f2bf355c77ddf1c58e306304a', '\x01195adb94cfde801e03000cb1f1f463f4f1ea04e4253d548cae699a095dbac71fbb0e29f61f202e2aa2bc1485', NULL, 1794891323550980339, 0, 1794891323226335427);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x911b6d192f412f4dddbec54fe6761b83d729289d308be2042231d3af21c10e73', '\x0197c531202a2e6149624b723ab561cdf754b1e07dda6317e1f2345c02dbdb4069fccf833e23cb137adf55b42c', NULL, 1794891323577558974, 0, 1794891323267394147);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xf603161ca3e20e84aa47bcdb5da23319052c99493fd907360c279fe617046497', '\x01a8cf5cb2cf0d41bff0722cde624e4088a6bf4f76d3506ddcd941726405c234e5714774d057ac7189c7d7fe89', NULL, 1794891323593598204, 1, 1794891323292218441);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x5a3ee76b53557e7a6c25a7d3ff50df1f3e55dd40d0aa82527865aeeb7a23fc73', '\x018ddf4ddcb99adb1b7d62f2b1b0dfb3eb2720dade2725261204b10055eefed7ec5ce56292c552ef5647df98e5', '\x0d69ab79c8bb6164dbd661abc2957eb6', NULL, 1, 1794891323315824948);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xbf8ad7a74d00fed5c7cc2e57acc6407c24b8c9fc2f6572bd640d1a4e35c1b06b', '\x01b041a7c02fc20ab5937191c2a1bcd571d9dc603deb3e0afee820fbde58ecf13ebeb9c9f25781381d9c45698f', '\x5b3de6a3567e01ae208b67ae181dc6aa', NULL, 0, 1794891323788469105);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xd1ea16949b8a9e824637139f64b8e5db9aaa676e704c5a97b35fce52ec848819', '\x01f0ef4ffc1b927e956c24cc2f39787133242012ad886cea5c9528762fd927b9e5ff699e50e52067b657fd9705', '\x1e297f90b3e7e1ac6689dfb2fceb3278', NULL, 0, 1794891323816268611);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xc74631a347ec5705c5ea8a59f7aca68fcad5ab9a7b5c27af07e412660dd8a0c9', '\x019039f4d5a4f384128eb8788290300359bbac6bb297e192b9a758b247a4218636b2163fef6450de336adfbb09', '\xda747f567345590fd555e3a0e1f307f8', NULL, 0, 1794891323854948736);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xa71e1f181df0117360a08712d04e755654c66954ee64ad23269d898f0409e2d7', '\x015661f30967c14dfea20f393e195f8625f81d047f1a0abf040aa839dc22d685233535e283c4a8346564d2fadd', '\xdec093f719f1bf93fbfaf63ff7397008', NULL, 0, 1794891323876977678);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x64fc174fa0a65ab658c93237ccfc32a1a836a1829ee97f860fa3d944c3bcf76d', '\x0181e26256999d1a3d2733a231029d8d8cfbe33db821d10c539e1354ab34cf5829ad298123757985f87a94ca42', '\x5fbb5769c3445e106b634f9fb8fc52fe', NULL, 0, 1794891323919578360);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xbc0996376dcfe8694261d92f46f1d39127c80d99aae0a643d74466a4ea26b85a', '\x011e24a57323cf48d025f63b62dfd017dc0561a7065112f676d534a38137b87a0ca29f0c1a32a478c13c093ed2', '\x103fc9c1d38be9417b3ed49275d30a21', NULL, 0, 1794891323946883774);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x0c217285f168ac49ef56e02cd94f093d17f047bfe2f5a3d865e0289a10850a0d', '\x01834a7f8f54754ff9d865e85e061cdc1118983191f6eed3ac81ae13832e64b55fd76ffa4a09e7ea65c646124b', '\x71d3eec5c672127f4c3d9fe486ca7245', NULL, 0, 1794891323627730635);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x186b0c351d8c47a63a1c3b4454f3dfbe0606fb153e67b680ad1c2a8c013681d3', '\x01de9f1292e3a7cf34edbb70b618c4798fced4437cd7e44c82e316098f206a8d07777581784b2436dea636d534', NULL, 1794891324099765234, 1, 1794891323696155913);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xf6ade42a672cf89cda36adb3faf5685ce9eb61cbacf500c914ed9719de3dfd7f', '\x0119d47821fba4dff989590c0143b91d6d889608a39c4d8a26e68d367daf589df6d6c123c7ed556983fdd22401', '\x395d03ceac111c9eef347d6fa604d4c7', NULL, 1, 1794891323754622563);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xf70a9d04a72622e213ee0123da1fc622deffcc3fea97aeb80a5c484a2d9b5a36', '\x01783c25f55456a8f4456807fb5de17fc85a9ba633f0582fbaed1745d51c4aa1138fef7f78a0c8539751bbf15b', '\x5e4e4d7877f9b81d32d68f9e1d101e08', NULL, 0, 1794891336020358108);
INSERT INTO vcon.vcon VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xaf02aeaa5b048c401d5239fa8ca8f0fb7e75b7a40fe6ba0eae015af1a9300de0', '\x0111240595dd7c092cf805ef5d8ff9de1d2149b1a5fa84d11f93886c9c0964a9794c40639ce972f4abc8f14dbc', '\x0b05d77fd6278ea50430aef83a85def1', NULL, 0, 1794891336113868826);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xf897bb3f720e01bbaa572227352d064e36540ec0f96efcce8610404ee3a97fd0', '\x01b3adac356cde66cef5aa6df524d25ce6bf851269821166b7fba3a19ea82b228cdd82de41633fc4a0b9f803d4', '\x6541c46cfabd21c9df52e8b578ded2fe', NULL, 0, 1794891336180100677);
INSERT INTO vcon.vcon VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x30e11a3f3671e2c1ebe9df5d0763991ba8871343a5028f924ddfdfca69ec774e', '\x01615d0e565d07d0a722e2237bdb988195cd5abc93b140c83f584fded630d11a542983699c6c0b6b5088bd3161', '\xbb7a9b768258642ccfa07af7f2085ffd', NULL, 0, 1794891336262467415);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x5645241fbc1cb4ce92aa1a55c77dbcb7', '\x186b0c351d8c47a63a1c3b4454f3dfbe0606fb153e67b680ad1c2a8c013681d3', '\x01067fe15ca745e0a01c6fdaf249cfb1fbcd613e3ecfa3ba31318e7113900967681676ef7e', '\x018b7ed28c63336f6e6985297c06242232437022fb3ebc01ddff50002a38', '\x01b51d5a35efd9f8783567e00ddfccb096753e872b325ca3e30b773469c01a8bd5d9131c23', 1794891323696155913, 1794891324099765234, '\x14fa3b0212c67360', '\x0134c7bd7f3563d45c8b5aba963da34af9cd4d0f031be510e0c3fe5b875de6b9596b6a03cac039c5b78918e0f35c1a7cdb50825bd6502de6f8f5d00fa4', '\x821c58287f69b6fd62029a09acb03558', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x26033b4803d02d168c6216cb8200e98a', '\xf6ade42a672cf89cda36adb3faf5685ce9eb61cbacf500c914ed9719de3dfd7f', '\x010e83b2ec3841018d22e193f841b896adc55693baeca2175deeaaa5ea06a8ec742edaad82', '\x01fed66c735e8bd3d6dd87585452faf635dbe495fffca2b97c1854432f51', '\x0120e7a770f9cabb539dc4618e249ea1fbd9fe349362671ebc8fa43a0dd898a477896d1e51', 1794891323754622563, 1794891324170181893, '\x14fa3b0212c67360', '\x0114b9790d31720f25b008ec7171c0008ecbd8263748a3b6db8bd70cd6f81b92fa0ad88ea1e773425cb3fd991a2c685e56a154ffedb19a564887d116b9', '\x821c58287f69b6fd62029a09acb03558', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x5b3de6a3567e01ae208b67ae181dc6aa', '\xbf8ad7a74d00fed5c7cc2e57acc6407c24b8c9fc2f6572bd640d1a4e35c1b06b', '\x01fe11cdb64b508397473dc653c2fe07afa66062dcaedac9f6562f2ee9235934cadc33f4c8', '\x0136c806bcefe9b771639897fc3ef5c4fd450129cda7b51539ab23562e28', '\x010d13ae4a05894329afd9450c61f0cc5b5046e2b27b8ab85a4d9aacf2dabccd2c3141e6fa', 1794891323788469105, NULL, '\x14fa3b0212c67360', '\x014e4aba9f06182b4bf092f42e9f53d12271e79d6c9347c7bf4e2b24617e407bee7e5ed0540216be572ee28cfc46d1f258a0fe13de4c8869529e6928ce', '\x821c58287f69b6fd62029a09acb03558', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x1e297f90b3e7e1ac6689dfb2fceb3278', '\xd1ea16949b8a9e824637139f64b8e5db9aaa676e704c5a97b35fce52ec848819', '\x019c382af4d14bf0c980d344fe36b9dd98f1b0e8b017f0f69b271919562a19f4317b508d8c', '\x01855e1b9850eaced333f259b7770b80a992ef5d5c2906234a7f51dca338', '\x017777411e6b43d0ccec107a59645b0ca86614e73b85ae13313cb390ddba63efff7129274f', 1794891323816268611, NULL, '\x14fa3b0212c67360', '\x01f4fb929ffecdc04e1db2d90a7bbfc19b2e0c0d74cea2368574f71dadfad88ec62c3488ad572744c888c92fa793354a08ba323de6913244588e9cf919', '\x821c58287f69b6fd62029a09acb03558', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xda747f567345590fd555e3a0e1f307f8', '\xc74631a347ec5705c5ea8a59f7aca68fcad5ab9a7b5c27af07e412660dd8a0c9', '\x01f5d371a0f6db99ff3f26bdbfcc5a711972ea05ddebb5d730d35b54e1022963a5a50fba5f', '\x01c87ad7841cd4880f482c3cf2042f94abb764ef02349902c3c3a5b22d54', '\x01d9a45b34ffac6ffa7dd9a615218460e1ea8836ea58009113cafdb34ce586a859bb96344f', 1794891323854948736, NULL, '\x14fa3b0212c67360', '\x016b28e6b7b1abeb5dbbc0ca24c789f7320df8abae7db0a2c0490a955c15cca3ed2ebb1bce0d041e8a9b29fa54880c633159adaa9ef2f75d6a7bd79d2f', '\x821c58287f69b6fd62029a09acb03558', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x1238424ea556358b8906fb4b359897b7', '\x3ff3676cbb51a7c9f24e7f4a7cff8272cdd32286aec3ded719848deb9645a541', '\x01a6a97ff117b187711fd1c9ef2d3f36225f3fc8c957cec3060d87fce93d2639bd61ccbefa', '\x01b03f3deed0d89418107112d50146b9de986c556df8ffb22aa3e9887cb7', '\x01e129674687796f60f8afeb301ee63c8afce44f4b0a0b7473f7172603503e40af122ebdde', 1794891323204693994, 1794891323520050518, '\x946f17fae0008e89', '\x01a2b4e778971460633d8bf979250ef0e02bddf3eeb535eabb8519c29d22886168a428446564937b6c62c5ef64b5ce18460d2dcc38cacca6a2633df60c', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xdb457c53adaf5d2e806e5aa0ad4f926e', '\xbe17e886baf63efae0a263c5a4c76c95d753fa2bed6fff43c2160dbce4eeaaa1', '\x014c75a45f478252410946a375e90a96db66933834a520a44db36dfc8cb3df88876c508d2b', '\x01e317e64c0a444301edc582d9462d7acfe8c84a3a5b788d701bffc2db1e', '\x0128bcd7c6ed1446a7f7658fd3019435ac29fdaf110c1c17e8bf2ca242edfcb18de1c5d089', 1794891323150396415, 1794891323479924440, '\x946f17fae0008e89', '\x012c5c69f1c6add571bb97c580c9a333ccfa9f07740a5ffa3e9d721ba7db79d31f84f9e3ac2bad32ffdf9f1ceb1952f3ce29e6f3a3096d4437cff913da', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x876eac2ea831d883cc5abd404ccea6ea', '\x4e90b62569969561ad3fb47f16f5483c18f1a70f2bf355c77ddf1c58e306304a', '\x012b119d5b2522a58ade89aedfb8b110ff36dd3be04bf1efe198e19f65d7548b359c33259e', '\x010941aa155c857d246a2822efc63ca8046beadb9b8aab6e9d1abdded257', '\x013824c2392d4c719e0353e2637f2574f91471a52b4a5a8cda2ce1df6f2fdeeb15e59c6400', 1794891323226335427, 1794891323550980339, '\x946f17fae0008e89', '\x019f280d70334ace721378b5913331ec13174ce31eedd4ad83ed1fa12c92a3cebc127e8a317e682606fef308ac3a6aa6614d580abbd815c56a2e298e7d', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x7b11f380e6508b39570a276f2036efed', '\x911b6d192f412f4dddbec54fe6761b83d729289d308be2042231d3af21c10e73', '\x01636b91113b62f833d2f9c17f0a51b3293b6c302fb737a0e8add56c2fe8787097eb91abf9', '\x01bbf32d95a3d23bb98583e80a21c65ad61aa6b517fd1c60545e0ad7bbea', '\x01b1e6705c751441d811ac5118407195ace94afe003dccc64cde957a2af39edff08bb3d7b5', 1794891323267394147, 1794891323577558974, '\x946f17fae0008e89', '\x011e3b2a16fb6d26b225d9163f289a876d22fa9f576323c9264aea0aa846b88331d59d48bdccb6702f59d1caf22a526079ba88c703de78ce0a4f5492df', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xdec093f719f1bf93fbfaf63ff7397008', '\xa71e1f181df0117360a08712d04e755654c66954ee64ad23269d898f0409e2d7', '\x0102792321ce458591aa4e02cafb33417ede358053718802e208626d141b193119895b29f7', '\x01167be1ab246753faaa86f0b40eddb9bbc8902927c5e8240d112c7ab0b9', '\x01dc5ffe55a8cde1778c05e3767111bedd1f61ec5d11c7be6f4abe0e00cb2e9a95215f1d1f', 1794891323876977678, NULL, '\x14fa3b0212c67360', '\x01eaed1f6376d891889db8f287643bcd4e7035f336ce63490c032fb541b3e8f6585b4b0f08882eb285cf025e06aa2999a803d0cf4d5f132d325d0a8449', '\x821c58287f69b6fd62029a09acb03558', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x5fbb5769c3445e106b634f9fb8fc52fe', '\x64fc174fa0a65ab658c93237ccfc32a1a836a1829ee97f860fa3d944c3bcf76d', '\x0114bb4178797b8a0fc4f75496aac444001fb8abb0d57342d17a5e75bac8e8f7a242abf91e', '\x01ac356d557390be39ee036d07695be5891e25606950e84828f86c985be5', '\x0181220935ecc724ca8509b2076a8dc0deb254ea000051b87e7c19a37baf27dad07e7ef2d7', 1794891323919578360, NULL, '\x14fa3b0212c67360', '\x01c1b278a1d6abee0ad981892e30d2788bca42fe3f25b7d87245be724b3b90397b5ebd72db73cabfe9115e9ca1ad31bbcc021fe3f15ccc4f2e4f1f89bf', '\x821c58287f69b6fd62029a09acb03558', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x103fc9c1d38be9417b3ed49275d30a21', '\xbc0996376dcfe8694261d92f46f1d39127c80d99aae0a643d74466a4ea26b85a', '\x018be501afc5cbba75b9f5fae2efea1a21aa00fa8b404b12bed183b8d8ef91cd3d9d3a2da4', '\x01d65f39acd7bb68fddf7a16fa2c09cc188ae6ac4ce9c268d4716f9e1e19', '\x0174254ce8c4be974dc5bd90643b6e1e51dab6889db2ca36dfa5744779e39c8fa9b954bba8', 1794891323946883774, NULL, '\x14fa3b0212c67360', '\x015f4b636509b726e44beea96c8864c58a20e0f4d2ef00825bc811950b63ba995afb3476eb2776763b2c46d16539813dbd629b62958139e3fdf25e689f', '\x821c58287f69b6fd62029a09acb03558', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x71d3eec5c672127f4c3d9fe486ca7245', '\x0c217285f168ac49ef56e02cd94f093d17f047bfe2f5a3d865e0289a10850a0d', '\x01a8f17c512d342dd3e16ab735df01fc3cf0a28186c387fee18a12fd454a109ee7fe5511e9', '\x01899fc5b28849c217f435ef50eed7a0c8dfc8f996e7363d253c267a57e6', '\x01bec695f3634ad712314f0efb1991bfe3628653c0a150564141218e555f04490dda8b8743', 1794891323979072531, NULL, '\x14fa3b0212c67360', '\x016c05e5068f17a6d98b88c9b81966ac06aafbbfc7e0c0d5a93ea2da9209b0390f36989a22700d19aa484d68caf96af7cfc3ff438f45fc71b11010a382', '\x821c58287f69b6fd62029a09acb03558', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x395d03ceac111c9eef347d6fa604d4c7', '\xf6ade42a672cf89cda36adb3faf5685ce9eb61cbacf500c914ed9719de3dfd7f', '\x01a9a5f5925261fcb5248e8bcd8a9035ef3ba70deca3a2f2c342f33ba7307d1f531df5c3f4', '\x01a9739d29485bc3557d13e7becd65596e9fb1ae312f95de99108033c5c9', '\x015fcc87373ed35ce7a45e92a19a5b871e18c0dd737337651468190feb51252f9dd880e3c8', 1794891324170181893, NULL, '\x14fa3b0212c67360', '\x010ca965882246f90f9a6c7b3dd6ec2288cc8fdf062df96d5b69d8c7bfad95d0491f7389573b9cddf8ffe29d79f55ac8a767c1db9aff3d9e8683137390', '\x821c58287f69b6fd62029a09acb03558', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x8553505b7cb6e246a181cd3649808305', '\xf603161ca3e20e84aa47bcdb5da23319052c99493fd907360c279fe617046497', '\x012852f34120d310a0b04ba985353c23e619674aed516ff9c5405f778dfd836f15658ef47a', '\x0114b2743c0b600550e10241daf65016d7f9abc53d544dade65aad58c3d4', '\x01b0053939d67ed5c204449ee2cc8084bab9582edea0320e40da4bc9770e62fa2a131d5678', 1794891323292218441, 1794891323593598204, '\x946f17fae0008e89', '\x0116abdeabba34f71cd183f722fade57307bc6df92072ce51608b7601aeb8a5d294f6e5163f3278e8047bbe4a7bf4da3594ee59ca448d74c56bb803f81', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x0d69ab79c8bb6164dbd661abc2957eb6', '\x5a3ee76b53557e7a6c25a7d3ff50df1f3e55dd40d0aa82527865aeeb7a23fc73', '\x018e0b9e27f5eb4eec86735b48e8b37ebeac409d8bb814980fa08ce91b5fd81285a5879cec', '\x01476b2347b1d63a71f92d94b0797c2ba5f77dbd4d8e63734bf8b7ab14c8', '\x012bc8164bfa9031a4841c254958b5875ac9a7c20e33046f718524675ad663875db245c6f6', 1794891323315824948, NULL, '\x946f17fae0008e89', '\x01c8e657b0d02cfb4e98fcd341d1a70c65b4e377794540f3d0aea6da9993c6545719edd8e9bbe8e3ad72f2d517bfd737005b677b4760cd76dd52c3db27', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x8c132144e4c30e3a6e0be6509f91d692', '\x99286fabb79ccc165c254350ae30615400ef1eb99998bcf28f2ca19122fef98a', '\x01cb2374b231b2b24c1e76efb21945e1d6394231f15c519f769c6593c50880d7e5b0a3846c', '\x01f2268cb74b218ef78aec3f6e387b23ec433203a71d15738a4a21683cc3', '\x016be9f4e63a1c0897bd68f3d913dc262577508ed46f5b86c734f83b5a79134e10017ec2d2', 1794891323364120990, NULL, '\x946f17fae0008e89', '\x01d35a00bbb5c5fb7a06a29c932ae9963cef06215edc7e8be8d50fab8c175c9befa3e0e419767cf8d1d9282aee92ad709217a35b72b30f7b19fa977ada', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xb2a5467f7b1855c7254a63b762a3f831', '\xdc8061d5891106b258c7ebc9deccce59a4eb22051d5ed9845119f73b25537fd7', '\x01acf992a2e54417e49a536e571563b42da335203656c0e43a6b69e68d9a0374721beca291', '\x0150c02c0107709ce9efd6148ba4c4c6b14c26314651e2145f2b09b5802a', '\x01f68f0b08906d9ec8ba48b516a7d047164ce11cf2251b3b7e87b1caf23cff7c63c3aebe49', 1794891323385335350, NULL, '\x946f17fae0008e89', '\x01044631c81ea16d7e8fb649952065cdd9d1d67624fb8c9b68f29f521b45504c3919ac4f9b542cb6438e912259b84b4c2a0d13e6353956b0c8853da129', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xf46fccf7a533c223df31986b12e6a9be', '\xf490ab4dfa3b0c9a26f55fa65b02955b182c1da6b6875a393dd2adf5713522be', '\x016c3233301b84531995b3313de5a5d7ed914e967400de48057525a50757da5a1f7f1340f1', '\x018175f4df908cb9af9e4cb48bbd24c62897482d63005b33d05810378d87', '\x014793e71eb086d545b91b04d88d1b5148ab3ef5c007e6b4541dca3e7c207f1713cfe4aaa6', 1794891323421237790, NULL, '\x946f17fae0008e89', '\x01ceaf1c27a2dac717b38a38c6256e321814a084acbcfd72a648bcc680db355e67f457f254283284fb50cf5d64afe71f2fb6c79c4ff7fd9682b588565c', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xe9f31220be813ceb8725bf7ab4b8f7cb', '\x8cd3b5d9b1df1c3ca972bc20004194b1ca32789c4db997b974cc0473fd6caec0', '\x0119c7b1827fce7b156e22a7c35cf7027b0c1ad2568546952f975bf16694d662484cc96d94', '\x019bcfa2cd77a2b3203fa2c35694110a92f823012d1b87dd7c2933daa84b', '\x0169b428283763acb6576c5b1b82f6a4da87d6bf245d9d36cc08804c872d02660cf86d40d4', 1794891323448219620, NULL, '\x946f17fae0008e89', '\x016ccf8305d27d3aea655feea10980ffa70bdc44d8f2ea3a53be41eb144243ab0bacc4d23360682d51d85b5eef1b0fdb4c27dc4be85b7fef7b51058a66', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x031d555152e2909feb97b4d55dce6b42', '\xbe17e886baf63efae0a263c5a4c76c95d753fa2bed6fff43c2160dbce4eeaaa1', '\x014e5706d47ad41762d69cbf47ba58f30106b526759047388b5c89527b901170781f157f68', '\x01532415b29f5de99fdec853d484a8d44085b980c8f6cccffbfed00a239d', '\x01a8b5ce0a65afd73ffd7223e8ac4fae8701257cc26b49c2d1217466174e90ed709f0aaddb', 1794891323479924440, NULL, '\x946f17fae0008e89', '\x01338071722cfabc5adb06cfa50749a1160c1fae0e6dffac08e4931a4cf16f84ceccdf904f228dc242f27c366b8f20f80a48457a73681645b629b14971', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\xf73cd8cfa9d89b53f94da0326adf113c', '\x3ff3676cbb51a7c9f24e7f4a7cff8272cdd32286aec3ded719848deb9645a541', '\x01f488e3cf5b6f3a413df101e7685e74afed7328b32831a8ee5cd14fad2e7767917f244031', '\x01833c3b81aa9cafa8f14773b98e40c0691a5e5c3cd29533f91a262f68f3', '\x0127752ada3ab40b6c05257f60acb482ab9ee5e314915908f2fd92636d4c8f9ff1566df7a9', 1794891323520050518, NULL, '\x946f17fae0008e89', '\x01438f3f2a31a4060a86a82dd0334b11179ceb110a79c20c765a45a2d79a0da179e4261d414a7dc1336f360a8eb5075cb7fb6c792a99839c8ebeb63ece', '\x7764dbf56e35264e5e9f4150d9aae31c', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x5e4e4d7877f9b81d32d68f9e1d101e08', '\xf70a9d04a72622e213ee0123da1fc622deffcc3fea97aeb80a5c484a2d9b5a36', '\x01b9e923a1151c400fbffd05ff1a960aa1254132a233b8007b7b7c7127de34220ab73a5bed', '\x01c7b3a3b0fb5336f076bd76f244a5d599beb1d0219a88ede93c7caaaa73', '\x016e0a6bb7ec6b175582e62396856199cbf82ea574522b89daa3075c7109c65c1a1ceb2753', 1794891336020358108, NULL, '\x946f17fae0008e89', '\x01fdbcadc571bbb44510cc1fab8bfa450e40f63d965c055a7d8b73ffeffb6177da1022dbad8c233124fb61e094ba7a7e0efbe75a6625c1454f079e2e2a', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x5aa55f9947674e0eb8acc35d5dadd218', '\x0b05d77fd6278ea50430aef83a85def1', '\xaf02aeaa5b048c401d5239fa8ca8f0fb7e75b7a40fe6ba0eae015af1a9300de0', '\x017500d26c56a2f54f6de0665125dd9b76721e08a8f4296f294975e5f1780458c32a2a0f04', '\x01fcc475d09d17f7405fce0e002d8b7d988864562b32e45c6830f6664c14', '\x01591ae48b9a39948f29208f8793c6a9ac1b62263072f4ea01213efb426250e76129dce4e6', 1794891336113868826, NULL, '\x946f17fae0008e89', '\x015ce7541375b504fc8535b786a89f8c5c05aedc6c642184f8f26d0efbbd5689f712bdef1bb675410e53bdcacc9df656d662f1cfe1e9122720b8a1bc02', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\x6541c46cfabd21c9df52e8b578ded2fe', '\xf897bb3f720e01bbaa572227352d064e36540ec0f96efcce8610404ee3a97fd0', '\x01a5971ac4834d5404fa99d6e17802e3aee0739d14e61c783dde30a6cde7c23547dec8b6d3', '\x017a67dfe2347314ac2ef1111010f87ac8cdc00efc9695ba22e2a3f93520', '\x01461ae207bc39d69007e1eeaf72799dda204a57cc8a7b267f5058ad0026d79ba6caf3c4ea', 1794891336180100677, NULL, '\x14fa3b0212c67360', '\x019946be806e16da4ccf835b697b91f8a91741c1cc555bf1d390d6337a9c59b1b72efd47dff6941f729ece3a7c25435e3782764f4cd3d9ee2a892faeab', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb50dd7af538c83a987d0889f76e31d4f', '\xbb7a9b768258642ccfa07af7f2085ffd', '\x30e11a3f3671e2c1ebe9df5d0763991ba8871343a5028f924ddfdfca69ec774e', '\x0143063f9574989473901db1344473b92dfb533157d779a44591004975844c8cba35c6665b', '\x014efc4247f44cce09e771f4c4d840620d403a5ec73898cfd55b5d1d4341', '\x01f5cbb752c6ed77eccc738bd0bf4a0e0d4019a3912c36bc1334db292a6f14074ae1e8bd72', 1794891336262467415, NULL, '\x14fa3b0212c67360', '\x01f4761faf3db8432fd9bec897032af2c8d90da2cd6151fce70565a3e82047836183c0762be76e8f4827ad4707e5add4f81d48b814096129751f94c91a', NULL, NULL, NULL, 56);


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


