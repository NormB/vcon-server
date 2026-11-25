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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-25T20:20:55.337729297Z', 'cli', 'tenant_create', '\xfaab29c53c4c15dd57299ee3150cd159f610cc94831a4a92f98eee7508f9538d', '\x012b9312312e240abbbabfc7ff2431abad467a7404a2954715a24b2bc3f2dfbc161db7b6fd080cadb5edae9317cff92afd82643a518b68424d0887acb2', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-25T20:20:57.349523377Z', 'cli', 'key_create', '\x014071f03eced34253c5ed258158da504327ad6f121686fc8d82ce8680b28f88', '\x01d4ec92e8cbf58da02bdba8000acea0bf444eb9450abb672b5ebd1380039d6c1ca4ac00368abe2068c4d38f57', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-25T20:20:59.343270783Z', 'cli', 'key_create', '\x1b2c6f5e0a5852c79cac34a7ebb150302fa99fdb93d3b74605b53a1b4b2fa63a', '\x016c1e36cde779684cb999ea44d120cb23448b95a56666c2d6451111b80a85d9f280220579a3178d74f6393e5f', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-25T20:21:01.295065355Z', 'cli', 'tenant_create', '\x48e727b78046a72fcdc532c8ff2fffc0c144d244c9c44dbd13e9896856ee78b5', '\x01c97e8a02f69c970851b33ede22b6910386bd18f43b16e2d46cd12b4c4546bdce0c8a7facbea68fbd09f1745ea2cf8ff3923df1fba88b03820776a157', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-25T20:21:03.612438791Z', 'cli', 'key_create', '\xd47b65c33057d828718f3b8f0d583738a6c4437a985239ef289680ca97fe8f84', '\x010972f3d2e5de314f153252531b699b90db3ab5f93503220b213e39f6ff2cb16bd1092519b9fc5c1e76f9e758', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-25T20:21:05.492393376Z', 'cli', 'key_create', '\xb5da3e5833895248d4efdb6a5dae9374c51106ae55fb083c54ccbf92a62b1916', '\x01f8c00aff95eafdfb53443623b5cf8f3fbe5669f3af8155e2cf404f8985cbe02e25fb26014f758052b15ee1bc', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-25T20:21:07.398528506Z', 'cli', 'key_create', '\xef4f2d21de8a5f9cd3698553775e2cba5fb5512fe27be10e79f3ebf982d8deba', '\x0116a12c947d0d7fe1a3055a6f6a1777b654284abd6dd6548f39cd0261d33e76902ab746e1d2ff44f49c5cc7a9', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-25T20:21:10.410783864Z', 'cli', 'user_add', '\x0724b055c2d3cacfcc758b165319c514a54cd0bf48a7d78672d69661cad154cf', '\x013706b0197f959226021eaba93b888175a416bdeb563d04ff8208566aa6c9b90102983b7e3cccc543df2b29aa873d470fcc1ef25270b10c412d8b588c', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-25T20:21:12.298510477Z', 'cli', 'user_add', '\x1bd8e17ed142934b2e7b7e64dd14ada06ae7ef03021e21f7704f9850e2925515', '\x017c50f57880342a91beee91c838864c232554bfa774cc9d77ffc766f308133432bfd43c5250266218575eb7912e14f3ef2f290505d98db67ab346b433', NULL, 201, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-25T20:21:14.291096637Z', 'cli', 'membership_grant', '\x33d52625c2766a671ff5d8232139ea18a49eabe5b39bba077995ce27c926b202', '\x0150f365503db4e98700d67edf1aa2ae1389190cb0bc6d38f68233bf47bce5fbee1ff84f59a9bc29d222fe7c88f8ec46bf0b11b1f1f716df2ef5e18892bea5c30a9830321511ab969e60aef858b11a46aea9cefe321af9c990b6b1d78c9bcdf9257c6875c3', NULL, 200, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-25T20:21:16.243115551Z', 'cli', 'membership_grant', '\xa1b0746eda567e8281c1fa17c74ad68d1c540085208700386935b0d25b97103c', '\x012dff61514058c21e81bfbdd086e769d158be561aa0d17ca0df2a4496bdb8e39b4d8810bf6eba229505eba76f71c99c788dd90bd4fd6d6c90f63f0ad965e3d23872b3098fb232da970b68b1eac007115c8185da6acec60bbdd476766dc2588577a3e118606b', NULL, 200, 1, false, '\xa7fad78d440e3443');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-25T20:21:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 3, true, '\xa7fad78d440e3443');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\xa7fad78d440e3443', 1, '\x01ff254e912d4a945f25bd831713504db20c5403ba2a54e4aad544e497e41d20712751e85805a6bcdcd13ab7c4394f93bcba74714a34ab7d3cee02a772');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\xe8ad3479363514cc', '\x52b7c793d8ed0394f3e77d034bed56cc', '\xdc1ebd7c369321b184ce566b2edec6444004f63e5abc1d9c851688d10dc318e9', '\x018aeadb0c7e5319bacef863d6dffa658088b04e9891d5de99f2d595c1369d6e47', '\x019207c0bd79fda592a56ce6dd27f369466e180f883f52787afc2155c352625898c8c298dcf0ad998d34e8ed', 1795638059342851431, NULL, 1803414059000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x5493d2bd9e7a90a8', '\xe5cd4acc6f03e69568766fd93846c156', '\x95c9a2f7e63529c8793acf819813e5e973d9b35455647d374c26ea219bc8e90f', '\x019dc87ba0c469d9e079040c9dd47bbe2a3042fed75c4e2b91b8c15b7abfe81179', '\x01cbd57361616234b5ff64483e9ad1a48b702f6916510fd62989f2b9472e902827f288dfe3e0bdc964f554ea', 1795638065491987458, NULL, 1803414065000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x1f15b9fc1d63a750', NULL, '\x34f63f27c8e8b283b889f070332010f52373a556259bd13ceb4fb4da88a592cd', '\x01b0462ede3224f1cc123e395bac5bac0a8f84746cd5ef0658c8e1c6a84f435520d4', '\x01ee3ad20b8b7f6742a6703713331fbd41d2f8551d73f683b6110b4c654c6c61', 1795638067397985126, NULL, 1803414067000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x2f97d4f778b37bcb', '\x52b7c793d8ed0394f3e77d034bed56cc', '\x742ec83ea310b3de647bf78b0278ebf1b70f1213d985755aedc1ca7fc8ca23cc', '\x014cf8a806093efbd668647f6e900b9db1615a0a0a1b6458fb2b2d2e0803033589a584d48f6ff2d0e33377178d8e984d16e8796a9b3621d18eba', '\x01061348247e415e3ebc23d2de9c7a5398f7cf7961b7530ae594a64bb5215415f1aa2ec7f8bcf3adea593e7fcc83', 1795638057348956515, NULL, 1803414057000000000, 1795638060000000000);
INSERT INTO vcon.api_key VALUES ('\x08fcb6f4c97e7d44', '\xe5cd4acc6f03e69568766fd93846c156', '\xe739d6a72acd25014e7a38deb979454ab1455b0bf012881f01466aa951b7c9a5', '\x013dcc33f8e82afa20c3c04197fcdfc085fa5cefa8c73c034e20fbf9127ef51b4833813868bf4d7bc80ec1958c8a486985baaa72b28dba09d290', '\x01280465312087a4b182e89c82c6c22d4a257f2a1b9b8a5fb7de3a7dcc70053af75b7a029d91869d5ff3f1d4cbf2', 1795638063611948290, NULL, 1803414063000000000, 1795638060000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 2, '2026-11-25T20:20:57.352929447Z', 'cli', 'key_create', '\x4354376dac1c37022fd49b206321e792c2df55a4796d088f792b73e963234905', '\x014cce342b511070b7f5b652b8aaa8c229be7a0057f303ca54343a2790fda90bec819ab2202033b74f0dabc9c8', NULL, 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 4, '2026-11-25T20:20:59.346585705Z', 'cli', 'key_create', '\xaeac0bc94ee38c37532287ba2d67152f5620e0bc9ef16c46a3a1fe8592b3e57b', '\x01d342649c0d09fdeefb416ed50431d7bc1ee5b3ab04d8445bef3ed5120b32315fa611891a7d7ff0dd9965347c', NULL, 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 6, '2026-11-25T20:21:03.615924018Z', 'cli', 'key_create', '\x23e74f01e89b73e26dca1e7de037aff378204a46a0ab2d449f8fbdd664fd254e', '\x012ce7125db69fa9d87b4e5aa782d91ceb16f62edff63c5ee2c94199f527ad6ca0fd075ea3d3b28503181a126c', NULL, 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 8, '2026-11-25T20:21:05.495650039Z', 'cli', 'key_create', '\x11c96aae84ebe09c36d849774df9fcc82d8bba0e6c174531b12feafdcf4d8856', '\x01871b45e7375fc9c565b294097b33b489f819d7da24292b2fd8690f0b68462218a8624c59057b7b7f7f07c40b', NULL, 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 10, '2026-11-25T20:21:14.295485069Z', 'cli', 'membership_grant', '\xcb318e9818d9f9b847a20686be96010c43605e360e7ff7733412a294ffc705ff', '\x01521141bc4fb604073ad47a16a26a80897932b2f4d2d523f01634d2152a560019bff3e3608cb71fb6725ee9eed125ad5121d1fd487e645a614d2edf6b2b5995bcd31c3d335ccfbc58ebee77e219d6b05fa67ff88de81208186aa85477a9cef06e7b617443', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 12, '2026-11-25T20:21:16.246216833Z', 'cli', 'membership_grant', '\x605d1c6dc178fbe0c7d6c2d976dfd864e568507eb87350f9be719c147eefd17a', '\x01ca6576b6d0a9d6bb49d7a0512f8dff40889015b28554ac3cffe45f6930e4bbb245fe8e7b8ec6c7dc59c880ddcac849af302e0a7b4afbc7c1f337d20da4f721c31a138cc2eeaba9fdc19a50a9882883fa5f1e7e7b9fabd479b1a284a454df50ce6bbb082da9', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 14, '2026-11-25T20:21:18.623135112Z', '2f97d4f778b37bcb', 'create', '\x328649da467b28c0414c06ca3cd6efca09ffb916d731b5e8b4117282bb6c0237', '\x0118dbe4a06844663ed50a243cfebb03705d2b690168df76e13013768ea673c07f63d15b22bd55864c08adbd7b5a46eec2e7f0bee83af07f69654a23b31d327aed', '4a11ef0ebf82fb498625bdda1474f3a1', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 16, '2026-11-25T20:21:18.680046661Z', '2f97d4f778b37bcb', 'create', '\x0abeb69a0282712d152e990e68e2578189d0de54b8932e1b09c8c467df718c63', '\x0108c8176995cb30559c9c5513c60e90a23ad3c7dcca070a7fd6e1189663e3e3749899f49452a44016b59af334ad52495a69dfa1d7ded2e3cf5af14543f1dd8b14', 'ffcdf416f994ba16120fc7e0c5bf5373', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 18, '2026-11-25T20:21:18.738118952Z', '2f97d4f778b37bcb', 'create', '\xc80a9dec4f9ce95b3bed5eae04f738ba2c8df58a408d8464959f1f023dd86b6b', '\x01fb9ade47c131c306a4ec4a134bebe00b720aae3628903f4a50f64ca56f2a7c9ba3953051232cf62dbe0b8a11f2405aeb0ec682513fc990a17da817e5df0dd121', 'ddc49cf2b794712e93b407b39890edba', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 20, '2026-11-25T20:21:18.782679123Z', '2f97d4f778b37bcb', 'create', '\x3da8674fd352a9242c1e95813054c63a071ceee5693f6f3ae8aaaa6bf3e51dd5', '\x01ff50920d919df7bef7a1bdf70a2ee6ea05eeb89fee7e7706caca34bad807edb32688ca72d9ae9daf39ebd58fa6767ad35d7c266ba6523d59e97afa0aaf08124c', '4ad864faea1b391d370fffe7c95b68c4', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 22, '2026-11-25T20:21:18.8321912Z', '2f97d4f778b37bcb', 'create', '\xb7d30f5b9990fecc2e7b07abbf8c16def7f54de712ece9e8ee326855671e07cd', '\x017c78ba0b4ba382b60a81062d4b8aa02bb4c9db50df938eea60498fabb987f3c1e7758db029bbad92a048069a2d84d29c680ebde688ebecd2ba4f3916158e8e4a', 'ade9c9d0c361c3698f3f023b2245a119', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 24, '2026-11-25T20:21:18.902400243Z', '2f97d4f778b37bcb', 'create', '\x5c16b91791d9d52bbc926e5ae37de230ebd2f45f1e27ba263f866c909c6160be', '\x0138e3401ac54b0e2f47d02e03de06d026a3e1a66a1c77175c8ea0ec1bf3cb574608d17511f8794ac8724fb32d92cd4c1b4543e5ee4efc4b5fa87cc9a3152b9584', 'b1a884cf8cb8d6e341beef005858bf04', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 26, '2026-11-25T20:21:18.968111614Z', '2f97d4f778b37bcb', 'create', '\x38f42690046947951c8771bd52b4ed108aa6753532ff5316975cc9d51962da36', '\x018355f1c7cf616bd054e823dd519446bf5b2d7398eea5e8e76135579bb414481570cd161646d19c9e0254a24ed6ab67f06667d5c3099738a1b3d9d97f95eb3fe1', 'b14f0bf0deec67640b05941eadebf44c', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 28, '2026-11-25T20:21:19.021807483Z', '2f97d4f778b37bcb', 'create', '\x2ed90caae3134e5b7d61835ebb24310630654e40f3e12d88c369e1624048edf0', '\x01e2f5a64cdccac41127f6c7a380c951473ad9d18189685811b045e12872d056ee5c4766939c815e5fa00cdde3e741fa45fe504f63ab0598ce398115de045b83bd', 'ffc789073d539b3cdc9f058e2cd895ff', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 30, '2026-11-25T20:21:19.08622833Z', '2f97d4f778b37bcb', 'create', '\x2e881e128d6217c4bb18d230a557227161436c2fab97e45c030e3538180cdf04', '\x01f37f8aa0c8b9b2f94767754403e5714c2e0fc446ab0a19e4f002845b6578d8611d9c8433c7b575292450cd37df09529821ec989bb3eeb367b3a5baf249dcb8f3', '94f6002a6a45de9931533673dd307353', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 32, '2026-11-25T20:21:19.106012119Z', '2f97d4f778b37bcb', 'create', '\xeaadeb71c7008e19015471f96e53b5ed20e8b1abe02850f0831eb79a386163ec', '\x01f562398fd417343b79d8dc24443e0c04d8c89afaf8e06441bf5c889a787372d38e78aae2e6040ea2ec1a52295467604a8c3814a30ea2f3282016629b498861f4', '746983e4fe27b3dce5929b1115425564', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 34, '2026-11-25T20:21:19.134176773Z', '2f97d4f778b37bcb', 'replace', '\x328649da467b28c0414c06ca3cd6efca09ffb916d731b5e8b4117282bb6c0237', '\x019d9a2bb30012607605c7cd3d5b89e8eb3ddbabaf8297e81ae71d39de1070ea657d6e03d618e9e1ec3f246e5786fc666f5b336bc10dcefa8583bc1492b9cb2299', 'c30d6aa97e3538249a1502342f88fcc3', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 36, '2026-11-25T20:21:19.182688911Z', '2f97d4f778b37bcb', 'replace', '\x0abeb69a0282712d152e990e68e2578189d0de54b8932e1b09c8c467df718c63', '\x01a4f6d2259a4101a5cab1f539b8c7efa7825b6ce193a12fd381a8071c67ffbfa4a9d44536e3c2e56ba6b4df44751f2a532b4ce602315bd172019de170bf0cec41', '87db6510a4daf044591fc601d32e9fd0', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 38, '2026-11-25T20:21:19.188905475Z', '2f97d4f778b37bcb', 'delete', '\xc80a9dec4f9ce95b3bed5eae04f738ba2c8df58a408d8464959f1f023dd86b6b', '\x0128751e8ac4206b8b8684b702cc1a77278087b9fc6b20e8cfeaf44a792587992e1fd2d2517ef48d3fbdf838aea98a56d2953f0038c8fb15798c3a6772f3ca2a0f', NULL, 204, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 40, '2026-11-25T20:21:19.196535228Z', '2f97d4f778b37bcb', 'delete', '\x3da8674fd352a9242c1e95813054c63a071ceee5693f6f3ae8aaaa6bf3e51dd5', '\x019ebbfe58a76f9be0da60c6e2901f162a75b412c6acb3f24bfa9d881f3e372ca33f239e7a159571a1973d1709d22b7050174fbaf4246f0b3d3d6ed46c3eca7b8e', NULL, 204, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 42, '2026-11-25T20:21:19.202671506Z', '2f97d4f778b37bcb', 'hold', '\xb7d30f5b9990fecc2e7b07abbf8c16def7f54de712ece9e8ee326855671e07cd', '\x01ba72009f6304d4578072565c25eea6587d832d0d09827cefce7df525e862d1481c95a5fbfb96bbc61b810150fd41d1dac7cbb2696c9dcfd1dce41518628f3953', NULL, 204, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 44, '2026-11-25T20:21:19.206661753Z', '2f97d4f778b37bcb', 'delete', '\xb7d30f5b9990fecc2e7b07abbf8c16def7f54de712ece9e8ee326855671e07cd', '\x01461230b329ff3fe64d3083de75207c27811c3aea329259290fb0e700df91a119b36e076d1407caa648c0fc4fe07dba385a59fd67d6b78f200317177f1505839b', NULL, 204, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 46, '2026-11-25T20:21:19.211772538Z', '2f97d4f778b37bcb', 'hold', '\x5c16b91791d9d52bbc926e5ae37de230ebd2f45f1e27ba263f866c909c6160be', '\x01c2faa965c3336447edac1ae1c34844cf6fc8df9474a35a9526f2b1159ea72ec2b1157498ef1b7439373691ab2ee859d81a0f237006c5fc17ff76dde7bbb2978c', NULL, 204, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 48, '2026-11-25T20:21:19.260810973Z', '08fcb6f4c97e7d44', 'create', '\xa1675dea503a24efd5b6c1f8d1703f1cc509d7fecf09647a99cf2c0a721c7479', '\x01b048ae0d8f6f5f8b417216ef31b7bbda448ce029dfeb7359381a2e2acccb9a2740c47a2e49549add801b353ce6c0470c6b310b9ac18565f3797306c99e259ea8', '870990728262f07c3029018f10b8f8ec', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 50, '2026-11-25T20:21:19.315519408Z', '08fcb6f4c97e7d44', 'create', '\xd69e0a38afa6c1ace1e91b356138098f7c28d1f626df5545214368e4d0ab2109', '\x013e940e16f2fe6572277ba29daf840ce296b093ccaf13ba6a19dc3ce9412965424b7d728bfedd9cd7680373dafc70959801e539a31a08471217a19955b0add0a0', '6db2620b36b79f26cfcf41139e969b21', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 52, '2026-11-25T20:21:19.336834181Z', '08fcb6f4c97e7d44', 'create', '\x78968066f8513504196f9f9593dba6cdbc250848ada706f263fc57eed08df918', '\x01c2a9facc3a8cf9b8a7ef261fa384ea366d2dd3a20fad97acd909c1ebe6615bf34cdd431fadc9aa68b0aa0adeca4a0477e7520efed21136808cfd724bda5f88e6', 'cb5bb899fc06402b8e53528d7d674fa0', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 54, '2026-11-25T20:21:19.379985886Z', '08fcb6f4c97e7d44', 'create', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x0113e35d455f23292144a7005ea85187f401f4514b4c9a7f4cde9533f5124e4f44e776100fc75e2969a270fc8f95120071563b8777656b27c4ea90191015dbcb6f', 'c167152624a9a247e5ceb0e1967fc149', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 56, '2026-11-25T20:21:19.435269342Z', '08fcb6f4c97e7d44', 'create', '\x3cb9a9e9b9a91696d9da4eaa7131944ca106fd6732bcbfe38b1187e175ed4464', '\x0104d7627e8d6d8dc528cce53878841b380ce32d2e51021496fcb8e7a525e533f24c945b51e7010aff7500d6701702154200f09e281202b559e4aa70ebde99b653', '27e6b59b58db0f5cea62392dff305a8d', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 58, '2026-11-25T20:21:19.494355655Z', '08fcb6f4c97e7d44', 'create', '\x2cdbfa3807eb7800b6f0a11f7c302e1606d8e03f7c6cfa966e166d0fd8ead466', '\x01b55a67fd3f3f66330c8a4349fd03f1857e27af7b343f0a215e38e1b000cf9b2eb09744cb1e61b3df3ecc3867015c7a547c09687eb1cd0a6d25ab91c64a58f5c1', '547efc87c67c19afee06ded2591fd34c', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 60, '2026-11-25T20:21:19.539846328Z', '08fcb6f4c97e7d44', 'create', '\x46363d63d5fc6408ce94b3fd148cd6863e3063a882a01e9653fe21e54c2ef36a', '\x0148563e39dd3a850a3a6401a79b2185ec6ffbc43bc2141a7535e6e47c437b9de1819be46bbdba55174cba204c984191e02352e42ddd82436b3e4fa4fbe9b0f307', '7d273eae2eab1aea06dbf0df48a74210', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 62, '2026-11-25T20:21:19.587188698Z', '08fcb6f4c97e7d44', 'create', '\x5aff14f559214331b670d7e9178cc00cae461ff7ba9e4e76efc71e11c5ab4b22', '\x01c6f77bd3efd0efaf3cf8d76504bb97305df3e1efd53386c714755ce644fba5c1a946416ac812fce3cc7e3cfce4726a6d87f9d2a6f40da5bd7e6399a1934f972d', '8431f898c6e353a873f11871cfd69ab8', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 64, '2026-11-25T20:21:19.631857488Z', '08fcb6f4c97e7d44', 'create', '\x6267173a0b74e52449b69b7025e933077677e927973b8617fbb24846d072bbf6', '\x01f57e2289ac489a897e564642001ccebe5bc62db2dad06f980bdd8d83769dec7c040a3a6d830551374d01ab5c61a560782d3661e8cdf93cef7faf56aa38a834fc', 'aaac2b66b4801d73c4003df6df4b7d64', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 66, '2026-11-25T20:21:19.652470446Z', '08fcb6f4c97e7d44', 'create', '\x033e13b708a398ebd56b56c1724aca5b62e3ba44daf34997072a47e6cb536829', '\x013c2ae2b2587b2586990654a75a02bd8960044efbd5e27b0696acc0985cce16287b5f68698057e431eadf8f9ef3bb79f04c54c7a55034a3e065cf3d37171963ef', 'ed630c63cb698264324f684d11b86500', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 68, '2026-11-25T20:21:19.69923422Z', '08fcb6f4c97e7d44', 'replace', '\xa1675dea503a24efd5b6c1f8d1703f1cc509d7fecf09647a99cf2c0a721c7479', '\x01a18f0938ac2a66fcec87daa9b2ce95ac686cffc6f556df1b5993b24e4baeeca15a3dec85ef1edc50c574b96ca4fe56041afa5d8416528c0aa40ca5351d1ae333', 'b468dea8a0f25f2492a7fc6e0412cb5e', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 70, '2026-11-25T20:21:19.704083727Z', '08fcb6f4c97e7d44', 'delete', '\xd69e0a38afa6c1ace1e91b356138098f7c28d1f626df5545214368e4d0ab2109', '\x0122c9a4caf7a9758bf60de0d4f52378982533670d1854e0162204b593b7f0e47474d569ad4c216fd49968f272983005ab8079edca0610ef6de57720421124c086', NULL, 204, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 72, '2026-11-25T20:21:19.706390499Z', '08fcb6f4c97e7d44', 'purge', '\xd69e0a38afa6c1ace1e91b356138098f7c28d1f626df5545214368e4d0ab2109', '\x019df9a79d34ec53429b8abd55f8d0ddaacf226d736eef30f7a656617674ff9bf0ce89ee0019515d135bdc5d14991723e99c5fa6aa3c49657b6b2fda5f7fe4abc4', '6db2620b36b79f26cfcf41139e969b21', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 74, '2026-11-25T20:21:19.855564789Z', '08fcb6f4c97e7d44', 'hold', '\x78968066f8513504196f9f9593dba6cdbc250848ada706f263fc57eed08df918', '\x01de488b703554c88e3b0bba06e491c8b321752b50b1ec4868d2617ee0cd91dccb3cacf6bbe0c091cd7b6aabd93800436acc2b80adec7ffba89504d069eedd70f3', NULL, 204, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 76, '2026-11-25T20:21:19.859727648Z', '08fcb6f4c97e7d44', 'delete', '\x78968066f8513504196f9f9593dba6cdbc250848ada706f263fc57eed08df918', '\x0134e7485fc2e576cfd5160e6182a8afe26a0733195b1f9353b76eca931daa3db8f8db697bc80d794947a0bbfc42a04a014b2fba20c3aced1bd706c22855a587ae', NULL, 204, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 78, '2026-11-25T20:21:19.865151693Z', '08fcb6f4c97e7d44', 'hold', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x016ed3e206140be3cad2215357b8e778101173e6b8c8a5e243f8b51c3ae1be0a4e9b77a12d7b0e7931409ac0d16f1bef310a12777f7304f62fd587a042d76c6885', NULL, 204, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 80, '2026-11-25T20:21:19.909737075Z', '08fcb6f4c97e7d44', 'replace', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x0184273474291bd7aaf4f7132adba77023a8501577ff6bf3e8428ae7864c900b9059fafdc6d4d80b5e306560cc48de87a141ba3b6250aa802cbfcfa86e489c37fd', 'e1a28c001aecc65186d9fbb16fbe36e2', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 82, '2026-11-25T20:21:23.624862568Z', 'sweep', 'purge', '\xa1675dea503a24efd5b6c1f8d1703f1cc509d7fecf09647a99cf2c0a721c7479', '\x015528a635336d2268b5390b3ee8a600e60214682a4923ff0d981b1012ce96046da38de2ca4362161613ec63fab06aaff34170ccba77622a2f4853b54575024727', '870990728262f07c3029018f10b8f8ec', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 84, '2026-11-25T20:21:23.927010538Z', '2f97d4f778b37bcb', 'legal_versions', '\x328649da467b28c0414c06ca3cd6efca09ffb916d731b5e8b4117282bb6c0237', '\x01323a3cc676ec6fb0cd4450a4c69d44a131d6112aaaf53e2e8d7bffbc534d7bbb4161dacbb18cbcd8674b6b624861a656a7a5844ac70f22472785f204263542e9', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 86, '2026-11-25T20:21:23.939350557Z', '2f97d4f778b37bcb', 'legal_read', '\x328649da467b28c0414c06ca3cd6efca09ffb916d731b5e8b4117282bb6c0237', '\x014725b94edb1d32808c2f1c09a8daf19d82486e2685e72a6ec9f7cb6f445ec3c35892473b5c7c4a1abac298b2b7bd11937d7e0e14875225666de99e7c4b05a53b', '4a11ef0ebf82fb498625bdda1474f3a1', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 88, '2026-11-25T20:21:23.967369312Z', '2f97d4f778b37bcb', 'legal_read', '\x328649da467b28c0414c06ca3cd6efca09ffb916d731b5e8b4117282bb6c0237', '\x014697cce62d05bd10bdb1ba32c6aa9a30051bb7d19e5ed0d065a38bf3a1a7ca280812d082cdcc21b1d5e23c9c85b32bc0fe013bc2a12c9593eb3fe06b1a2d069e', 'c30d6aa97e3538249a1502342f88fcc3', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 90, '2026-11-25T20:21:23.986942407Z', '2f97d4f778b37bcb', 'legal_versions', '\x0abeb69a0282712d152e990e68e2578189d0de54b8932e1b09c8c467df718c63', '\x01f6ef44a9755a983f1330a834b3deab74f90c0a4e87f1b8972890e5ec1c024e0175a6ab4ea30c9a97ae141cd8c68db10be304242116da74d698eda12dfb7afdb6', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 92, '2026-11-25T20:21:23.997110838Z', '2f97d4f778b37bcb', 'legal_read', '\x0abeb69a0282712d152e990e68e2578189d0de54b8932e1b09c8c467df718c63', '\x011ec4c1bfa8880e86a45960724abc9c9c35ce8f2a5c0253a14ab029c200cce1a5550b7481ef832d89e11c341dd57e6c11e5c9a19aea5f4a5b51dd9f426995aeff', 'ffcdf416f994ba16120fc7e0c5bf5373', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 94, '2026-11-25T20:21:24.023619637Z', '2f97d4f778b37bcb', 'legal_read', '\x0abeb69a0282712d152e990e68e2578189d0de54b8932e1b09c8c467df718c63', '\x015b7e0ccee2f92eefb5c7f4d8c50e034a1f0420ef20c43556b51af0f502883070327c5acba2c606240bbb135ed0f2356840c9324be4ba2af1ee2dbe25db2842a2', '87db6510a4daf044591fc601d32e9fd0', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 96, '2026-11-25T20:21:24.034804506Z', '2f97d4f778b37bcb', 'legal_versions', '\xc80a9dec4f9ce95b3bed5eae04f738ba2c8df58a408d8464959f1f023dd86b6b', '\x01ef92f61f023f8526bbf1a463aadb54f3d70668271907d35b3ec95e296a9bffbfa0da6ae3acd99082c5a987525929c64ea057bb270012c3d7d046166bf077c6a7', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 98, '2026-11-25T20:21:24.044332187Z', '2f97d4f778b37bcb', 'legal_read', '\xc80a9dec4f9ce95b3bed5eae04f738ba2c8df58a408d8464959f1f023dd86b6b', '\x014645e92b1cb29a4be128bde54906167238896921fd079c804ffeb8d9113e45b63fa58d804f8379349f4617f77ac27edbd7ead1e294e7caf69c0869cb5db03cdd', 'ddc49cf2b794712e93b407b39890edba', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 100, '2026-11-25T20:21:24.051625161Z', '2f97d4f778b37bcb', 'legal_versions', '\x3da8674fd352a9242c1e95813054c63a071ceee5693f6f3ae8aaaa6bf3e51dd5', '\x0175d5ee708d81ba3ee3c648f75d5abc540a460fb8199c131ed0d25e69c46b32554275b5c0e61460e790bd27c428f0e3fa6791000e40340e23105e1a609b36caa5', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 102, '2026-11-25T20:21:24.065636016Z', '2f97d4f778b37bcb', 'legal_read', '\x3da8674fd352a9242c1e95813054c63a071ceee5693f6f3ae8aaaa6bf3e51dd5', '\x01c36b1368ab8fd7765bbce9b45eb7137a4ebc792f7b4c69203e95da263e00103c374aea1256ff1e19e6903be6d988e32af2b805a3c11e83ec6636609d11c5419c', '4ad864faea1b391d370fffe7c95b68c4', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 104, '2026-11-25T20:21:24.074433844Z', '2f97d4f778b37bcb', 'legal_versions', '\xb7d30f5b9990fecc2e7b07abbf8c16def7f54de712ece9e8ee326855671e07cd', '\x0149a704993ab5b36d0752df9931035d89a17c2efec0dd55bfca013efe43bc9a66d1ba4fba00e9437199d019dca4fd940077a2647329d374014d642cc16baf7c18', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 106, '2026-11-25T20:21:24.086169584Z', '2f97d4f778b37bcb', 'legal_read', '\xb7d30f5b9990fecc2e7b07abbf8c16def7f54de712ece9e8ee326855671e07cd', '\x0197c20a2015e1eee0fac2ede129b2e2f5c65b7fe2812b2e8f92ded9b9d2a7618600bd6c8c144a659c2821b3fb9bd671e1dd8021fba4d1cf55fbeed765de8e2b5c', 'ade9c9d0c361c3698f3f023b2245a119', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 108, '2026-11-25T20:21:24.092446278Z', '2f97d4f778b37bcb', 'legal_versions', '\x5c16b91791d9d52bbc926e5ae37de230ebd2f45f1e27ba263f866c909c6160be', '\x015b79009a872be2eb7833ff5e853cb0f0db06894ab81126230c56630aa8502722b9773c47ffef913a274e1c84b0854d0c315327870f3a7a6a013cca1bbb328ebb', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 110, '2026-11-25T20:21:24.101591125Z', '2f97d4f778b37bcb', 'legal_read', '\x5c16b91791d9d52bbc926e5ae37de230ebd2f45f1e27ba263f866c909c6160be', '\x01cfeb12b70af405cef9fda437cd248e42549a35f8e6088a3b54ce1c1de62efbe1d13e6295b1d814a811144f88af10b08285a4557e8c8c91017cb0f16fe733a732', 'b1a884cf8cb8d6e341beef005858bf04', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 112, '2026-11-25T20:21:24.111397131Z', '2f97d4f778b37bcb', 'legal_versions', '\x38f42690046947951c8771bd52b4ed108aa6753532ff5316975cc9d51962da36', '\x012adcd5e73a6da321f569a2cb03f294ce085099eb2e6bd8a0ab28571dec570207720389a25b3cd9b1ab8e5bd390cda63ee2db270224a017307a6a920e3d4db1db', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 114, '2026-11-25T20:21:24.126199552Z', '2f97d4f778b37bcb', 'legal_read', '\x38f42690046947951c8771bd52b4ed108aa6753532ff5316975cc9d51962da36', '\x0168af49fb76a0181873c0119bb5e053a45e4fc70a3e378cdd7bd11a5bdaa95e32e778443fe8966676b01ebb6cdde4e27987596bb0fa6f0e9f3d917567f547b2dc', 'b14f0bf0deec67640b05941eadebf44c', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 116, '2026-11-25T20:21:24.132271246Z', '2f97d4f778b37bcb', 'legal_versions', '\x2ed90caae3134e5b7d61835ebb24310630654e40f3e12d88c369e1624048edf0', '\x01410ac46e58bc6fc4cda1ae4343400ae11b4b3c3a962ad1b9f8c3f0056b817c102917fa04e402dd128ae8a164312214267ba2d0a42768c8ccfc8f60ad8e07c2cc', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 118, '2026-11-25T20:21:24.148196779Z', '2f97d4f778b37bcb', 'legal_read', '\x2ed90caae3134e5b7d61835ebb24310630654e40f3e12d88c369e1624048edf0', '\x01ebab3fc211ec1b4b5af784444151902bde0e71dc9b6895739defdbe613d7e288c46b9d1ed9220617917d9f72bff3382515c958e60f2290ca3689ae0f314491b5', 'ffc789073d539b3cdc9f058e2cd895ff', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 120, '2026-11-25T20:21:24.161193206Z', '2f97d4f778b37bcb', 'legal_versions', '\x2e881e128d6217c4bb18d230a557227161436c2fab97e45c030e3538180cdf04', '\x017592ff1f427c984a59b9f8b08a61de013aee086a876071bc87d5c9e0c01e1acad2b2c5d6d218fbe0d3ad1f04d696405cde10681483a1e535ff9da2986f28de13', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 122, '2026-11-25T20:21:24.171255239Z', '2f97d4f778b37bcb', 'legal_read', '\x2e881e128d6217c4bb18d230a557227161436c2fab97e45c030e3538180cdf04', '\x019cb331a2df8eaa770982496a215c73f562d1d3f6ab7771999446430437f85e9803e92d848e01cda5b7d52d72ce02e9fa23f2909efffdb0b57cc46fb4202109cb', '94f6002a6a45de9931533673dd307353', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 124, '2026-11-25T20:21:24.178785011Z', '2f97d4f778b37bcb', 'legal_versions', '\xeaadeb71c7008e19015471f96e53b5ed20e8b1abe02850f0831eb79a386163ec', '\x01666f5b3828cba4e4ccd9ac5c7d811ee8e0bc6d2fabfe7d7ade412ebb5254ae08c2442b164f838df01feaca64d414dca80cc42c33e6f473c0facaf791f5cd0e41', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 126, '2026-11-25T20:21:24.190475575Z', '2f97d4f778b37bcb', 'legal_read', '\xeaadeb71c7008e19015471f96e53b5ed20e8b1abe02850f0831eb79a386163ec', '\x01231b62a35f3014652af01c8b6f35336f82450c1a8fea82dc06b46983f6bb7921ef92b78760600a5f9baf8aec50fac679df8aec9701711699d17cb08820a45d67', '746983e4fe27b3dce5929b1115425564', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 128, '2026-11-25T20:21:24.200179127Z', '08fcb6f4c97e7d44', 'legal_versions', '\xa1675dea503a24efd5b6c1f8d1703f1cc509d7fecf09647a99cf2c0a721c7479', '\x018f520183c4b682babf257d1ab1ef6a495a2d8f80b6b8e337e815edf99c7468ae0f6f60f74be242bbf10cfeef608062f8306a53eb5c9918ee9b4ab514ae3fc662', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 130, '2026-11-25T20:21:24.212133293Z', '08fcb6f4c97e7d44', 'legal_read', '\xa1675dea503a24efd5b6c1f8d1703f1cc509d7fecf09647a99cf2c0a721c7479', '\x01dd9d742bad7dfb00a9ca5cf06f3bf378c983c729a063e8f2c790209f2f95778ada95d8c27ffbfcd704e82f620bbbad563c5a1f7357613fd7f8223816da96bb70', 'b468dea8a0f25f2492a7fc6e0412cb5e', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 132, '2026-11-25T20:21:24.222274567Z', '08fcb6f4c97e7d44', 'legal_versions', '\xd69e0a38afa6c1ace1e91b356138098f7c28d1f626df5545214368e4d0ab2109', '\x01ea278d601d2342d30120712d1071260488ad3f1a34a638a758d4b1a16b8c04e9577174898a1200cd13efd7983dafefbc822f5d98cbae41d02fbea5d5b833bc39', NULL, 410, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 134, '2026-11-25T20:21:24.231665767Z', '08fcb6f4c97e7d44', 'legal_versions', '\x78968066f8513504196f9f9593dba6cdbc250848ada706f263fc57eed08df918', '\x01569c330dba6e8f77f4d675d78aad16e0d2c6a895ee0dece0dea42149dd43d6a8af719ed0850e4a8b50d0d818bccb6f748d69c6d7e2ffdc030a4c25584ac90f90', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 136, '2026-11-25T20:21:24.245032176Z', '08fcb6f4c97e7d44', 'legal_read', '\x78968066f8513504196f9f9593dba6cdbc250848ada706f263fc57eed08df918', '\x01c8c4d03b4cd421fa31f2ba75b12815aa7d3eba2d83a045ea522c2540f3786ddc6b266b5a6538a6b4bea94173776817fa34e15c32c0ecee0a5930b34f3e42e187', 'cb5bb899fc06402b8e53528d7d674fa0', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 138, '2026-11-25T20:21:24.254453023Z', '08fcb6f4c97e7d44', 'legal_versions', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x01bc59631acfb4f2eed39936ee77221ffd0cf862fdcd16edb9496c13f2629788701b4fca41f9ec6d8c516683cc0f8ad38aa435e57f17a980d6ec1ebe0f7aea6563', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 140, '2026-11-25T20:21:24.266396608Z', '08fcb6f4c97e7d44', 'legal_read', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x01c4e5d9dcbe25c45366092883a952555437dea35d99a3aa431eb93dd6952b0e7fd1117849633512daea1a9e6ad5f8e25afb03c514c51be311b21e1a4a2be771b4', 'c167152624a9a247e5ceb0e1967fc149', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 142, '2026-11-25T20:21:24.278486015Z', '08fcb6f4c97e7d44', 'legal_read', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x0144c8088fa6a71756a421769f81e396e38f238c0efc09270d644b8cf149c61bfdf7c86f28c5088e10cb4df9a3c47c84477daaa88b8b4a7d616620e8531bb39711', 'e1a28c001aecc65186d9fbb16fbe36e2', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 144, '2026-11-25T20:21:24.288283623Z', '08fcb6f4c97e7d44', 'legal_versions', '\x3cb9a9e9b9a91696d9da4eaa7131944ca106fd6732bcbfe38b1187e175ed4464', '\x01ccdbcff6c72c00495cb80abd60531b96c58707cae0b41d06511ac41ecb2bf98d3ca6a916e52c1b9e532b770c81ec7e81e5a6875e06fc747d8e2005945c7421f4', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 146, '2026-11-25T20:21:24.300069308Z', '08fcb6f4c97e7d44', 'legal_read', '\x3cb9a9e9b9a91696d9da4eaa7131944ca106fd6732bcbfe38b1187e175ed4464', '\x010227f9ff171a0bda3cc551e3b810b0964c70fbfeed1baea57e2a47a4e2d4ebdb1fe4a19f387e8f7adc46e900a1e1b7a96faf01676291a15127e9951105bc1b7f', '27e6b59b58db0f5cea62392dff305a8d', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 148, '2026-11-25T20:21:24.318943624Z', '08fcb6f4c97e7d44', 'legal_versions', '\x2cdbfa3807eb7800b6f0a11f7c302e1606d8e03f7c6cfa966e166d0fd8ead466', '\x010688e961c8265daf7a6a8f63b023566963f294cc2bff26f0ecc9a21f6dc95c6260cdb4d4468ae89650a4abae8e10836465ba038ab6e72136d8974d605482f02d', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 150, '2026-11-25T20:21:24.332710859Z', '08fcb6f4c97e7d44', 'legal_read', '\x2cdbfa3807eb7800b6f0a11f7c302e1606d8e03f7c6cfa966e166d0fd8ead466', '\x0127fa641ea6ba40441134b68918ba632889143f47f0b42a79fff14039566f132d7ef1cfcfce5c409ad1a902ca107316485b3bd97b31d3decf8c2796740ddeeab3', '547efc87c67c19afee06ded2591fd34c', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 152, '2026-11-25T20:21:24.343725265Z', '08fcb6f4c97e7d44', 'legal_versions', '\x46363d63d5fc6408ce94b3fd148cd6863e3063a882a01e9653fe21e54c2ef36a', '\x01955bdb67c3d0869c180705279bcffdf980cac85a28d7d4ef97e912f044965eedf9911d481d197d6c238ef62e77ee90fb2100e9d77839709e6a07fd120f6dc948', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 154, '2026-11-25T20:21:24.353420733Z', '08fcb6f4c97e7d44', 'legal_read', '\x46363d63d5fc6408ce94b3fd148cd6863e3063a882a01e9653fe21e54c2ef36a', '\x01465629f249ff5914b118a84ef7f23f2ab258cf7e9ade5c8a3073310cc0452fd7ce34f12347d2f06cb8df233f4329a45af8fed9a1a645acef5e8c2f2c1699e576', '7d273eae2eab1aea06dbf0df48a74210', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 156, '2026-11-25T20:21:24.364260778Z', '08fcb6f4c97e7d44', 'legal_versions', '\x5aff14f559214331b670d7e9178cc00cae461ff7ba9e4e76efc71e11c5ab4b22', '\x01dcf0fc0acde210cc289690e922aaee190837e548855605748c29ee540ec2c19800f92789ed7f7c1f0d044b151797e4dd69a2c5b26842f57baa68756ff4be4fe1', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 158, '2026-11-25T20:21:24.379131088Z', '08fcb6f4c97e7d44', 'legal_read', '\x5aff14f559214331b670d7e9178cc00cae461ff7ba9e4e76efc71e11c5ab4b22', '\x0114fc018bdd470835348f5841898e1030229730675e713d25cc6a304808161f5bad85a763c90b475ac53e3e7f97735477bec6e5e0e95c46c6c4e94ee03a9580b9', '8431f898c6e353a873f11871cfd69ab8', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 160, '2026-11-25T20:21:24.390430587Z', '08fcb6f4c97e7d44', 'legal_versions', '\x6267173a0b74e52449b69b7025e933077677e927973b8617fbb24846d072bbf6', '\x016a786962e619b8f64ad0737e6bfbd9cb0bf4115b8df2151b74d5a73b633a02bd12053d0ccbfcf6a7295e6069d41ab4e07eb1a302d4d37e27c49d844b2875333b', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 162, '2026-11-25T20:21:24.400078982Z', '08fcb6f4c97e7d44', 'legal_read', '\x6267173a0b74e52449b69b7025e933077677e927973b8617fbb24846d072bbf6', '\x01c3b5021b453002aed7f3c157fafe8dfca8717975a05113cf7f9689bf4db82833870c2792f32bcfc5c17a6051dbbe3f0d6e2f4c64d7ba7374838260431de71340', 'aaac2b66b4801d73c4003df6df4b7d64', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 164, '2026-11-25T20:21:24.407564345Z', '08fcb6f4c97e7d44', 'legal_versions', '\x033e13b708a398ebd56b56c1724aca5b62e3ba44daf34997072a47e6cb536829', '\x01021b8f6a1498c46dadb95188ef65c50075fc15db9d8a2b4eb2dcc569d49b06faf36b21703137d809df80a292a978dc6e0ad0c8a84cd6ffb357001cdc3657f191', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 166, '2026-11-25T20:21:24.415004208Z', '08fcb6f4c97e7d44', 'legal_read', '\x033e13b708a398ebd56b56c1724aca5b62e3ba44daf34997072a47e6cb536829', '\x01c1e88f3cbe9b8335331f274e2a7fe9a4ca314b7bbf432989a7200be55744c6c318bf90033d60bec3f9eff596d277c0b4384153d7248fab49134586f8c736f068', 'ed630c63cb698264324f684d11b86500', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 168, '2026-11-25T20:21:44.240736006Z', '2f97d4f778b37bcb', 'create', '\x4e98b4a88cc52eae36ba22b259320c73a2bbd550b90808056396743c07062fee', '\x012fc77d671b07690fcf6e13bd16a62859f42b497960aafdb085d510d6f7099aab7671cfda0cf8f18a41bb062d34c31266a974071ac8e120662a3ddd5735ac2796', '350d79fa516357cc3ad07633930cc7be', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 170, '2026-11-25T20:21:44.28884544Z', '2f97d4f778b37bcb', 'create', '\xa57569bc54163142d4859fbee992c5d0de3a1737071128817ee73189b67b7811', '\x01c667d19d0b0832094fc7e34d2cab7a0d27c29693b1c1dd71d64fb7257667334ba2843d3a95716e68e86e17d6e618c6c3b0fffbe50e09be17c9f4432afcb43af4', '0ec853e86104b62e88cb4cc003169d16', 201, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 172, '2026-11-25T20:21:44.308225377Z', '08fcb6f4c97e7d44', 'create', '\xc12b1684eb935247907630674cb51e61f1489e6b6bc37ca889ebdfc478ac902b', '\x019e57a3bab46c997eeef20d4b5e4362ccd1af721f207f5d5e3c2fe9d74a7e94f4808a5b368f430c32577d0f21dabc47755a4f5c7c465d06806baa55b89b6b8d32', '017c646e115b0a5115434231cb50456b', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 174, '2026-11-25T20:21:44.327518489Z', '08fcb6f4c97e7d44', 'create', '\x5a10c18cbd3495f09adaa197be09dbf00fb9b3664c222334907e9575715e831a', '\x012deab3e0ecaeb0a3e1a5bf7385f377fab1ef2a15f4c56686b395b2597c6c623cac78eb5e10b3c974c49a871580401c13f313c5db256bc270325dc4112007541a', 'e6e31b82d7cb999f9da60444a4ee670d', 201, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 176, '2026-11-25T20:21:44.331700671Z', '2f97d4f778b37bcb', 'legal_versions', '\x328649da467b28c0414c06ca3cd6efca09ffb916d731b5e8b4117282bb6c0237', '\x018906167b81c4815a69a08fdc86ff07c9d32dd39fcb96221be5ee368fcf83e1578352bec1ea992d8f92298947b1d677ce49410f1f968624929f5a4ec426038788', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 178, '2026-11-25T20:21:44.337880208Z', '2f97d4f778b37bcb', 'legal_read', '\x328649da467b28c0414c06ca3cd6efca09ffb916d731b5e8b4117282bb6c0237', '\x0125f0509ef5f9817fd7331fd407f7954beb40657f89330d7665bff3a5e0d2fe83921fc92f5cad671e615ce6f62e9da2984d19fdbc5d888042390a76f72a6c02bf', '4a11ef0ebf82fb498625bdda1474f3a1', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 180, '2026-11-25T20:21:44.342793715Z', '2f97d4f778b37bcb', 'legal_read', '\x328649da467b28c0414c06ca3cd6efca09ffb916d731b5e8b4117282bb6c0237', '\x0161ed10a48c07a2f64823c23c72dc12c4a7fa69241d5b3ce684406f6aa1cb946899ed3b15be9557ff1619bd6d58a4de5bab60bb644d4c78863505a8a4d6067148', 'c30d6aa97e3538249a1502342f88fcc3', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 182, '2026-11-25T20:21:44.34627686Z', '2f97d4f778b37bcb', 'legal_versions', '\x0abeb69a0282712d152e990e68e2578189d0de54b8932e1b09c8c467df718c63', '\x0100c7c042776e157f7c6faf2cab83a4e1094199fa8845cc99d4641fdf503ac04aae810fa693b364ceb07332377260fbd95f499b946c9cfe3367d991245133ba8d', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 184, '2026-11-25T20:21:44.351283691Z', '2f97d4f778b37bcb', 'legal_read', '\x0abeb69a0282712d152e990e68e2578189d0de54b8932e1b09c8c467df718c63', '\x0125d7d485e2f7caaa1a3df3ee10d4d83b96ba72441e3f91fcc42512c34164385382bbf7fa202be8ae16d8c116472222f34269eab1edf4ca6e8deba6d97747f005', 'ffcdf416f994ba16120fc7e0c5bf5373', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 186, '2026-11-25T20:21:44.356032929Z', '2f97d4f778b37bcb', 'legal_read', '\x0abeb69a0282712d152e990e68e2578189d0de54b8932e1b09c8c467df718c63', '\x01eeca0c37b6e9e798716f1dafa2f0cb36b8e710dc7d0b6606cb29f0874fb0e8e8ef694b217b25079e21758ea307935a87b2e73f6ae9ccc9522217b10f1f85ed6d', '87db6510a4daf044591fc601d32e9fd0', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 188, '2026-11-25T20:21:44.359409713Z', '2f97d4f778b37bcb', 'legal_versions', '\xc80a9dec4f9ce95b3bed5eae04f738ba2c8df58a408d8464959f1f023dd86b6b', '\x011f659068cc6f49cc23bab24c60ad62b523750b27f905d0e53cf3d1252678c7c9873c0ee35b26451f0e8f4290a9c6e3391f4c613684d0f3361275f77f42407058', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 190, '2026-11-25T20:21:44.363903081Z', '2f97d4f778b37bcb', 'legal_read', '\xc80a9dec4f9ce95b3bed5eae04f738ba2c8df58a408d8464959f1f023dd86b6b', '\x01af20fb942cf85c6a683b348611323a82372bad4ac48fc3ee8c2d8dde5004e33b08aed66aaaafcf90b34799bb62a0c712775c7996087e317c98a4145fe9a10d35', 'ddc49cf2b794712e93b407b39890edba', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 192, '2026-11-25T20:21:44.367248077Z', '2f97d4f778b37bcb', 'legal_versions', '\x3da8674fd352a9242c1e95813054c63a071ceee5693f6f3ae8aaaa6bf3e51dd5', '\x0132c95026d8f3da2f332f97f6daf52f6cb4de46303a52e70f4aafcb20df4c2683911c1862d33edfee13065add2f3a7a86fa0dbb659772e40093be894dac5bae44', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 194, '2026-11-25T20:21:44.371632472Z', '2f97d4f778b37bcb', 'legal_read', '\x3da8674fd352a9242c1e95813054c63a071ceee5693f6f3ae8aaaa6bf3e51dd5', '\x0170c97b686f47959cf91ef42abd7f84df1497f4858d60c4e60cbd8a4a6e8154ee05d5c4164f94387f056a209253703cf1ca472a1a3b08ccd3d43acd3233a80b76', '4ad864faea1b391d370fffe7c95b68c4', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 196, '2026-11-25T20:21:44.374901163Z', '2f97d4f778b37bcb', 'legal_versions', '\xb7d30f5b9990fecc2e7b07abbf8c16def7f54de712ece9e8ee326855671e07cd', '\x0107ed6c0b04abb9afde6ebd2208800dcf81d333bc4e6de6e037a85890688bc0267a7a3aa93f64fa83300f42e57a17c60c9c9827f7801ed5e3a60d1111ecfa8974', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 198, '2026-11-25T20:21:44.379233744Z', '2f97d4f778b37bcb', 'legal_read', '\xb7d30f5b9990fecc2e7b07abbf8c16def7f54de712ece9e8ee326855671e07cd', '\x013cdac23cd3d076ed6ec1a63e5d9e5730344fc8a7b190c9270546a42029250fdf740b62bc68c2723102f9d20f28c6c41751106d1f632bc8d8eed5a1db8a19bca0', 'ade9c9d0c361c3698f3f023b2245a119', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 200, '2026-11-25T20:21:44.38239336Z', '2f97d4f778b37bcb', 'legal_versions', '\x5c16b91791d9d52bbc926e5ae37de230ebd2f45f1e27ba263f866c909c6160be', '\x0110cbacfe12c4059a5c90c56fcb0fa22875daaa206418d33a524ecda4f42150a39398ddee76335651a98bdbaf65fbb2e3ffa69ae2b1021158907b0b9ed96cdd37', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 202, '2026-11-25T20:21:44.387014534Z', '2f97d4f778b37bcb', 'legal_read', '\x5c16b91791d9d52bbc926e5ae37de230ebd2f45f1e27ba263f866c909c6160be', '\x01e5c28b1c9ff25b6cd7104b9e3e1c4d700e610c6bfbfbd8e3bf866ba71a9f6a6ce206f743a778143ecc331c503513fb201d964bbfa916849bab2b4d63ff568e8f', 'b1a884cf8cb8d6e341beef005858bf04', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 204, '2026-11-25T20:21:44.390372446Z', '2f97d4f778b37bcb', 'legal_versions', '\x38f42690046947951c8771bd52b4ed108aa6753532ff5316975cc9d51962da36', '\x01b1b9f7adb92121aca3c055856f9f8e8ee8a097aeb9eda150d68b9790ab1c3e7eb4e01ec492d36d1c5a64b6bd31ba4d539e2cddc1e417f01faeb3828e598c2336', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 206, '2026-11-25T20:21:44.394661573Z', '2f97d4f778b37bcb', 'legal_read', '\x38f42690046947951c8771bd52b4ed108aa6753532ff5316975cc9d51962da36', '\x011975df9c3ec513761229d91fb04acdec390c0da1d54af72512849d656caf70504fb01973cde329cdc14682789f517ffc66e1ffb4c23d25c382ca5846ba62c62a', 'b14f0bf0deec67640b05941eadebf44c', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 208, '2026-11-25T20:21:44.397817337Z', '2f97d4f778b37bcb', 'legal_versions', '\x2ed90caae3134e5b7d61835ebb24310630654e40f3e12d88c369e1624048edf0', '\x01d685d3b0d5b6cbb25c52c8c782c00212de370f12ec9fcd9f064778113c5f230742e7bf895e61c7c71bf51f717bd61885dd309c64087675344b7deac856854a5e', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 210, '2026-11-25T20:21:44.4033633Z', '2f97d4f778b37bcb', 'legal_read', '\x2ed90caae3134e5b7d61835ebb24310630654e40f3e12d88c369e1624048edf0', '\x0114657931b2e3a4a17fbbd51df9722491955abf34744f99d046e53f27f970d8d2d148e351d6c8c783336efca75ed7989d353f4a42e4ab8618c568432d6a896208', 'ffc789073d539b3cdc9f058e2cd895ff', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 212, '2026-11-25T20:21:44.406655648Z', '2f97d4f778b37bcb', 'legal_versions', '\x2e881e128d6217c4bb18d230a557227161436c2fab97e45c030e3538180cdf04', '\x016ff89ecb7a72a8acf46f100d1c01318c1ee054c9f0dc9c6e2436712d0c0910290b87d098e233979fca775544c7818ac25fe7ec222dc741666b0c704588b3d576', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 214, '2026-11-25T20:21:44.411165655Z', '2f97d4f778b37bcb', 'legal_read', '\x2e881e128d6217c4bb18d230a557227161436c2fab97e45c030e3538180cdf04', '\x014785ca45049b9e0071c45071ba57e7e65e3f2d42b1c610ba0f75d6d902d665f13e4bdad45f9f41f874389c95e4dba30caf84a008a563e930dd6e0ffa7e1f7b5c', '94f6002a6a45de9931533673dd307353', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 216, '2026-11-25T20:21:44.414312075Z', '2f97d4f778b37bcb', 'legal_versions', '\xeaadeb71c7008e19015471f96e53b5ed20e8b1abe02850f0831eb79a386163ec', '\x011ba8e86cfaa181e7a5c6bf14a40d3e4fc3294fb6c45640caeba603a83f46945f7f0cc97040a373ba1b218ab225ec2b892702abe375e62fdf2d025c92ed5319d2', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 218, '2026-11-25T20:21:44.418680796Z', '2f97d4f778b37bcb', 'legal_read', '\xeaadeb71c7008e19015471f96e53b5ed20e8b1abe02850f0831eb79a386163ec', '\x012804be24f5442b16b4415465b4c0ced84491fb1bbe4014bf859dc6132cff59eff409643f6239428705d46800a17380f1740481d3c686adea47d968c6122721d2', '746983e4fe27b3dce5929b1115425564', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 220, '2026-11-25T20:21:44.422043449Z', '2f97d4f778b37bcb', 'legal_versions', '\x4e98b4a88cc52eae36ba22b259320c73a2bbd550b90808056396743c07062fee', '\x01372f879832216b82929db1b2f8edf3d8445a048ee530ff7421a8dcaef91fdca21bce4ca7fbf925f874e957cea5b5e2797ff81f0f372b9b123c800ab937732641', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 222, '2026-11-25T20:21:44.427575106Z', '2f97d4f778b37bcb', 'legal_read', '\x4e98b4a88cc52eae36ba22b259320c73a2bbd550b90808056396743c07062fee', '\x01a5108e23aa1d8168258ba2558f4cacd622ae4abc8873479857c44584e5e2c71fdf5eec381ea69b26b28ab58eec37e63c8a66691395e4bd1986c77f3871b14764', '350d79fa516357cc3ad07633930cc7be', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 224, '2026-11-25T20:21:44.430811759Z', '2f97d4f778b37bcb', 'legal_versions', '\xa57569bc54163142d4859fbee992c5d0de3a1737071128817ee73189b67b7811', '\x01425845da96a39cf1def6ff2920e303eb0c1d24f5d7eafbbe183d79e5b9dab0216279088b9f7dc37f941a46318acb407ca57a470d5c906a50e6e9fde54bf19609', NULL, 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 226, '2026-11-25T20:21:44.435357228Z', '2f97d4f778b37bcb', 'legal_read', '\xa57569bc54163142d4859fbee992c5d0de3a1737071128817ee73189b67b7811', '\x015b3d1b60eed3ad2d1f7ef2ca4029f73ca529bf545d013f63747bee3fe1c06b7f18b6efbba2ed4ad78f14d182350abb522c8482e8742b3f2eb4bea64480f64c58', '0ec853e86104b62e88cb4cc003169d16', 200, 1, false, '\x8edbac54ae9644a4');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 228, '2026-11-25T20:21:44.438843846Z', '08fcb6f4c97e7d44', 'legal_versions', '\xa1675dea503a24efd5b6c1f8d1703f1cc509d7fecf09647a99cf2c0a721c7479', '\x01273f8f1b5f0211b91b2220b490b1beaac73dfde478d748628f031b7d12f6e073201500c0390a14340354fe5f7bf802595ad5c5c9898741941cec40c752f79d38', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 230, '2026-11-25T20:21:44.444432318Z', '08fcb6f4c97e7d44', 'legal_read', '\xa1675dea503a24efd5b6c1f8d1703f1cc509d7fecf09647a99cf2c0a721c7479', '\x01989816ac68736f314bc8861ba7b41d888017790e3e284a6c263f7c23feb46aaae0f5f038d05e8be7e35e9527cd9ebfd8b14cf6bb92f06019f89a405c0c3de454', 'b468dea8a0f25f2492a7fc6e0412cb5e', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 232, '2026-11-25T20:21:44.447734749Z', '08fcb6f4c97e7d44', 'legal_versions', '\xd69e0a38afa6c1ace1e91b356138098f7c28d1f626df5545214368e4d0ab2109', '\x01efc81f8adfcf4c204aeeb5cb6f4f1db746850d3e46d77d502c80715e94b6293f9acf1e8dfd345ec149c58e6820028c8b260f415002a545b1449cccc3993250d8', NULL, 410, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 234, '2026-11-25T20:21:44.451080772Z', '08fcb6f4c97e7d44', 'legal_versions', '\x78968066f8513504196f9f9593dba6cdbc250848ada706f263fc57eed08df918', '\x01e245aaff269127537da4dbb39abc984390f492ff42a583a801b565f7bfd6c80753bb1441dffb4ed3bb5bdb31ed96c8f8dfd32d7cd78f86c96347604477a0d16c', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 236, '2026-11-25T20:21:44.455748326Z', '08fcb6f4c97e7d44', 'legal_read', '\x78968066f8513504196f9f9593dba6cdbc250848ada706f263fc57eed08df918', '\x0148cd6321906f93a2782ceaabb3f211d560d90d8764ee9b13f0aaf3dd0f8c5bf187547e409c37ccc16841311d91114756933a2bcacb2e7f5fa07f0fad8ec8496d', 'cb5bb899fc06402b8e53528d7d674fa0', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 238, '2026-11-25T20:21:44.459098377Z', '08fcb6f4c97e7d44', 'legal_versions', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x01e771689770ae4e897ca2c2941eed92f26b4123d7a0ddd313635466b3ae44afd1dee0b23e5ec15ffb0287a00b691c246af9d2206295960f93a2de17329591684f', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 240, '2026-11-25T20:21:44.463456366Z', '08fcb6f4c97e7d44', 'legal_read', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x01ea48f64df975a3c52d7caa3f74ee1ca650177725eb75ecb98f0ca2af69d41b2bc8d31d7ebee67e3954704f68dd5451108ba137251096b1ff5b5e3cfc77f4e373', 'c167152624a9a247e5ceb0e1967fc149', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 242, '2026-11-25T20:21:44.46765889Z', '08fcb6f4c97e7d44', 'legal_read', '\x92c63b226f01ff8580a5331a03277e4c57d24697e3de5a439ef69d52e8a2e1a4', '\x01b43553b36e496b782ab0fbd8411f300307b08d1a5afce46ba001889dad442bf310dadc1e4d1434ebf846b1da3b4e47ef9f918dd2c15a06674337d02051710800', 'e1a28c001aecc65186d9fbb16fbe36e2', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 244, '2026-11-25T20:21:44.470879923Z', '08fcb6f4c97e7d44', 'legal_versions', '\x3cb9a9e9b9a91696d9da4eaa7131944ca106fd6732bcbfe38b1187e175ed4464', '\x01e9a50cb7bbd69a9efb7da76c5f22cc65f7977a5208cf311c41b28eddb1f2217eb2cc7837ba02e1128b26c998ad7ee25a960003487f22e17cb23f06c40614815b', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 246, '2026-11-25T20:21:44.475345504Z', '08fcb6f4c97e7d44', 'legal_read', '\x3cb9a9e9b9a91696d9da4eaa7131944ca106fd6732bcbfe38b1187e175ed4464', '\x010cc52146fe4032b02e0c3a0d078da707b54efbf5bf1cc7a50b6fed0f6dff4b1c044aba7d979796a54e077ab2fb78b5bd411253942b7eea04e90353880ea788d3', '27e6b59b58db0f5cea62392dff305a8d', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 248, '2026-11-25T20:21:44.478841704Z', '08fcb6f4c97e7d44', 'legal_versions', '\x2cdbfa3807eb7800b6f0a11f7c302e1606d8e03f7c6cfa966e166d0fd8ead466', '\x0111818fe6c99a704271461b718673ed825b21021703fe5f2d3c4cf387d0b2ebd5e37aa5032aa1802321eb1df864c31e7e8e1eab5a0e117bb7b629a4575c31ca30', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 250, '2026-11-25T20:21:44.483289868Z', '08fcb6f4c97e7d44', 'legal_read', '\x2cdbfa3807eb7800b6f0a11f7c302e1606d8e03f7c6cfa966e166d0fd8ead466', '\x01154d73fcea74c35a73a76ade3a4ba61386e62dcd039795d3a9713e1f05f5fba937c98a72be958a982b96ecfeb88260fb850b074268a295ce4f55f0e29a22617a', '547efc87c67c19afee06ded2591fd34c', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 252, '2026-11-25T20:21:44.486712725Z', '08fcb6f4c97e7d44', 'legal_versions', '\x46363d63d5fc6408ce94b3fd148cd6863e3063a882a01e9653fe21e54c2ef36a', '\x01eb7bd5af9c9258a161788a057fad266b7b92afd4e8cb564a49119233e398e5d7cc0343dd8bdf831e0ccef7e5d1e4516fda09182e9f18ba2b17793bf7db887125', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 254, '2026-11-25T20:21:44.490869973Z', '08fcb6f4c97e7d44', 'legal_read', '\x46363d63d5fc6408ce94b3fd148cd6863e3063a882a01e9653fe21e54c2ef36a', '\x010a76217dfc1d97f168264bd8a172974cd75b2c088aef5bf6443cfdc60b8a8daca29a36127e2c8ed8bfe4a02870cef37ab1cc2e8027045095b20bc3db8cb78ee5', '7d273eae2eab1aea06dbf0df48a74210', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 256, '2026-11-25T20:21:44.493920264Z', '08fcb6f4c97e7d44', 'legal_versions', '\x5aff14f559214331b670d7e9178cc00cae461ff7ba9e4e76efc71e11c5ab4b22', '\x01ca193003be0b054fd17017cd2583a1fb0d4f20d29d563b4c98d6df03923d936d0f5c3060821dd9b3887085a753b3926ecfc2fa701273b64cdae00f91e27e38e0', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 258, '2026-11-25T20:21:44.498382688Z', '08fcb6f4c97e7d44', 'legal_read', '\x5aff14f559214331b670d7e9178cc00cae461ff7ba9e4e76efc71e11c5ab4b22', '\x01acbc64a4bc20b8b55822cb5bcce73a336634b6e75ca6ff45eeb5ba8e24124c50113bb1b317ff7cb0b7676931dd9f36594a68ae090f994aae1fa2b6943f80fa8c', '8431f898c6e353a873f11871cfd69ab8', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 260, '2026-11-25T20:21:44.501677277Z', '08fcb6f4c97e7d44', 'legal_versions', '\x6267173a0b74e52449b69b7025e933077677e927973b8617fbb24846d072bbf6', '\x01a12f4bf2fd1edb4b463db1e34ba7504e910df9ea57de0e7496174caf75c7e72e061b3d5137aacd3bf50ce44190209db50f1d6d544967b08fb15833b1c1d6c4aa', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 262, '2026-11-25T20:21:44.505867246Z', '08fcb6f4c97e7d44', 'legal_read', '\x6267173a0b74e52449b69b7025e933077677e927973b8617fbb24846d072bbf6', '\x01a76107f338baa6cf03981b6d36332b1aa86e2abdc43f2408ec0961de5578f3a66e0d1548d26eb09984770f2208a70a23b8aed53510aaf10ec00327f308f9b781', 'aaac2b66b4801d73c4003df6df4b7d64', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 264, '2026-11-25T20:21:44.508967353Z', '08fcb6f4c97e7d44', 'legal_versions', '\x033e13b708a398ebd56b56c1724aca5b62e3ba44daf34997072a47e6cb536829', '\x01a8de0685f68da8739a7b7b7c88b75b8c3f2f622a3ac9c12fb16bca46485021a86a51d423a5ede2cb910db90f37208d63b1fe8b6980924f7129e7311304473f68', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 266, '2026-11-25T20:21:44.512980599Z', '08fcb6f4c97e7d44', 'legal_read', '\x033e13b708a398ebd56b56c1724aca5b62e3ba44daf34997072a47e6cb536829', '\x01d2248ac1f060e1881b0d43a04d4988135b0bc8dbe15111f6cb335a27665b3b83d127241535b7cc0ecf874a65e660d53c899c381e6c7f06fce35c0009632dbd8a', 'ed630c63cb698264324f684d11b86500', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 268, '2026-11-25T20:21:44.515967706Z', '08fcb6f4c97e7d44', 'legal_versions', '\xc12b1684eb935247907630674cb51e61f1489e6b6bc37ca889ebdfc478ac902b', '\x01a52c6d5541d9a319027ac2267c273cc9506820cd366970c43cb9f0f700ab89b907f6ba78a7bf076a4c0b7e43232c3021a843b4f77304e2cdbed0662c967ff6d3', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 270, '2026-11-25T20:21:44.520350814Z', '08fcb6f4c97e7d44', 'legal_read', '\xc12b1684eb935247907630674cb51e61f1489e6b6bc37ca889ebdfc478ac902b', '\x01c2408ab2a8a5b8f9a03692af64ca9923d9bf82ccb98775d6bddcd5971cd55785b168b18f5faaeabfbb26892f45406b93cd8c0dee490321af2b7607754b636db6', '017c646e115b0a5115434231cb50456b', 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 272, '2026-11-25T20:21:44.523611088Z', '08fcb6f4c97e7d44', 'legal_versions', '\x5a10c18cbd3495f09adaa197be09dbf00fb9b3664c222334907e9575715e831a', '\x01ce727ef8c764a3b53a5fbdcaa903112b00abf9b4b71b1e9a741de6c9999751c0f0037c202952cfdc176d85955b79fb15fb085eb7d60ee96a72eb03c2beeae479', NULL, 200, 1, false, '\x6bf3fa2a95ce3677');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 274, '2026-11-25T20:21:44.528025197Z', '08fcb6f4c97e7d44', 'legal_read', '\x5a10c18cbd3495f09adaa197be09dbf00fb9b3664c222334907e9575715e831a', '\x01a93c29d3c0510e68918979a07f9c2d636ee91349f4cc36a74ed13d096b7c578f7312fd88913f5a2bb114986e6b3e65577e9812bed460f756810704ef1be80c4a', 'e6e31b82d7cb999f9da60444a4ee670d', 200, 1, false, '\x6bf3fa2a95ce3677');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x8edbac54ae9644a4', 1, '\x012c151119ad2a3cfc5bf6c72bcb0ddea1ddea8453343425921bf8420dec379457279ec19f16a44bd024bdb669c0026635376094188b248b33c5595ca5');
INSERT INTO vcon.audit_key VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x6bf3fa2a95ce3677', 1, '\x01fe4b10805dc60858c159b759a658c3dae5b7c06a558649e696751b936e8be309d65564615087fb3709629aa275764556016c4d1164ce2626c93a3e35');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x612bbb498c17aaa2', 14);
INSERT INTO vcon.key_use VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x6f725c4dffc508ad', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', 2, 1795638102368300701);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', 4, 1795638103470095062);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\xccf49bd6a76e7900', 2, 1795638055327050911);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\x9ccd2f6a154aabc6f18603e991ce0861', '\x52b7c793d8ed0394f3e77d034bed56cc', '\x01e8d3019496a337c5349edd7c4d03d3f6cf33758c9c6e1a19d89784c797554ec70d55', 1795638074289906709);
INSERT INTO vcon.membership VALUES ('\xf502be3c08293741ecbd93544c4e5c57', '\xe5cd4acc6f03e69568766fd93846c156', '\x0125e18278e996af398f3082470acdd746689234c67973fd5ec0a80be2a1457e4d0bc764', 1795638076242356892);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x1c90c72e3a9a651537c80e384c1f5f5d', '2026-11-25', 12, 12, 7401, 1795638100840391306, NULL);
INSERT INTO vcon.parquet_file VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x4ea045c9479655f6b2b4bd52735dc0bd', '2026-11-25', 10, 10, 6237, 1795638102432225782, NULL);


--
-- Data for Name: pending_write; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x2e9fbfcf17839dee');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-25 20:20:53.316426+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-25 20:20:53.34961+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-25 20:20:53.356122+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-25 20:20:53.356668+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-25 20:20:53.358728+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-25 20:20:53.361696+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-25 20:20:53.362284+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-25 20:20:53.366476+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-25 20:20:53.368812+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-25 20:20:53.369302+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-25 20:20:53.370377+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-25 20:20:53.375614+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-25 20:20:53.3809+00');
INSERT INTO vcon.schema_version VALUES (14, '2026-11-25 20:20:53.383229+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '3739774de34efa6f83630f1bf0679751');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x6cd36e3805fe4fe7cff47a1d07411050b14e368e9e173f18ab17b3b81da2b80d', '\x010ebbc8354414b511b74d395a71ffc9199cd34c7dabbe5d0c347dbd786bfba32f3fdba5b4', NULL, NULL, 1795638055337458221, '\x612bbb498c17aaa2', '\x017eef3955be11eae6962c835afdd2053bf8640f8deb9f463735ad95662147f8c1ed68a64c0ec7e229187e75274f6c8e3300bb985cee25cbcf2715cea4', NULL, 1795638055337458221, NULL);
INSERT INTO vcon.tenant VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xe3c6327098d61e540288495850609a794d35ca2fa7975d5e8736af0cab0cf51a', '\x0167d4cd2d647bd404319161284ac29ecf1f849fd34ddd90591f58c06f024c3cfaa74d2263', '\x0178a5c66eba936a3885ecf3693d3aded1e3387b4be2d51d35d94d06973781', NULL, 1795638061294838743, '\x6f725c4dffc508ad', '\x01b83e0484da4fdf020f2c29f50e32af3d3a2f5e0a88d37d3634470a1b0cbcbdf6935d63a273d650d345553a7fa2836189433d8df2dcaf3a88d11288bf', NULL, 1795638061294838743, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x76daa9609f49e482');
INSERT INTO vcon.tenant_catalog VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x98d4ee02e72185ae');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', false);
INSERT INTO vcon.tenant_lock VALUES ('\xe5cd4acc6f03e69568766fd93846c156', false);


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xb4f17e3b19adf075a847b73e3bc913abd8197a1963309eb37bd7b553c15fbaf8', '\x01f61c44ed17305a164e0b4cbac11986999e4f7a8fd8aa9f28c343482b70d05d590971eda5e4d6bf9e4edaf344', 1795638079705214581);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\x9ccd2f6a154aabc6f18603e991ce0861', '\x01a81605a9478769484eced1726c10e78c5e5aef10bbd6d6aaa6de9afcfcfe3bb03df8c447dbf65430494a', NULL, NULL, 1795638070410135113, NULL);
INSERT INTO vcon.user_account VALUES ('\xf502be3c08293741ecbd93544c4e5c57', '\x01f5b25f43f4c2b054d21943f7364a5f34260ea5b4a11c42e4ae05ecf2ada0e58c7c500f08dc762cd474bc62', '\x85dcd2dd9015fad03d79024858d27047ba79dea3a8064b13ebb189f42567d9df', '\x01766794a9184505fbb1c277a25947fc3b0d80164cd89cc76cb0af193bf57080abb9e17779b5367e1079bdd69e0206d4d62f3bf8d36d4f73061161fd60c644d6', 1795638072298173115, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xa4bc8d19d9944553eef886d96697de091ad41a70303827ad47327bdad34ee7cb', '\x0132eea5f4e3ab95b648e57c999d5e612b3abbe10062fe36f26fe29dcefeeaa0a5844a9b81758399bae3d50e59', '\xb14f0bf0deec67640b05941eadebf44c', NULL, 0, 1795638078967681169);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x01863e0240ec08fdcd7c669dc0fdf5b9475ab342d6a7206855ba846bc10eae54', '\x01285a237595f8e66b59b1345956a3be8d6ce5f2a8c347069221fdb880ae3cd60f41c992efc3dbdf692ac8c07c', '\xffc789073d539b3cdc9f058e2cd895ff', NULL, 0, 1795638079021363325);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xcd0a02cf8ac0061b786c5f2d6c18b1275a9672c05e489f2bd1eca58876164702', '\x014ef054c2ad7c2829a67dc5b6ca519536679ede3ff231c8b8c5de77d981c15e3dbeb79343c12e08b03439c036', '\x94f6002a6a45de9931533673dd307353', NULL, 0, 1795638079085766903);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x5ce8bdf556c2d1b86380f946de246ba8cb71990939c03591c8331385b537c710', '\x011112fcbc27a56de5f8c4b1078156302a22c558f0950844750dc16790857142c61329aa980161c53dd87399f2', '\x746983e4fe27b3dce5929b1115425564', NULL, 0, 1795638079105597258);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xd71954b5f55ddd95e32810f8b1e2ab1fd14247d65a3ffbd4ca721ace2e6d7078', '\x01c91b5042af430c225a7ff58ce6148e28022f9c3480204cf064b200247e3471f0791bb782aa75132bac8371b9', '\xc30d6aa97e3538249a1502342f88fcc3', NULL, 0, 1795638078621585840);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x0c49ed63f2cf7a41142971ce978c98cd6d6edf2a05968eb5febb0d2e2bb3807e', '\x0199754bded02187dc6d6eb84bb53e1348e6a0c75bd16b3d8ddb60f6375cf78d7c0b25c337decc39b1dcd7a91d', '\x87db6510a4daf044591fc601d32e9fd0', NULL, 0, 1795638078679591317);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x727618e4b85c2be651ccf0758179c29fc61aa8c84cc45140613201085045c4ec', '\x01d37cf164956f811cae932aad35c255c301705fc1ce725117afb1b34ce97c3b04ea018ab1a34051fd753e5b0a', NULL, 1795638079188551383, 0, 1795638078737526849);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x7c30a11ef273fd5f80a080ac196b69a95b76440009cf9565f22ae6fabf68d256', '\x01083b31243e73a84f0cb51bd677f64fdc0909bd4d4f29d03934f0b200c93675c86ac75a024fed7161a31bbd3f', NULL, 1795638079196069866, 0, 1795638078782261409);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x3714ee5d517637a7ba1c6be505e6af8f5ebdcc3ff4a38d10ed628d25f37108a8', '\x0106ab2d5436fdfc3708beba40002df75ad872779f2278012ec682dc59633507ba8d041c7b334956ab4a421297', NULL, 1795638079206309558, 1, 1795638078831771384);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x9deba1cbc84b3b8c81144ec82b9434b476ae8a3e7a8eae023f39c1ab2e4d0815', '\x01e92563a752cf05729041a15b8b4da50df1d432db62d5786f73cb37b3a088505e36f6f487b09ec79c0fba392c', '\xb1a884cf8cb8d6e341beef005858bf04', NULL, 1, 1795638078901959613);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x482ce46c0d79705da8311c03d242294d532906c5fbe350d6214a18659f6580f3', '\x0155eaa0407c538ca7b1b55cc538f374be86ac735d7ba387356797d0968deacce70a3d9dabfd7eac354a7113cf', '\x27e6b59b58db0f5cea62392dff305a8d', NULL, 0, 1795638079434852036);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x00930328d519086d192760e55b6799a893e0ebe5ecf5169479f965a714f23f97', '\x01d1fbac1e2115f7a1d5ac36719e8c6a881f657e0a4e9077a1ddf14bc2901958db7c647090dee20ef26feca15d', '\x547efc87c67c19afee06ded2591fd34c', NULL, 0, 1795638079493958276);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xdde40c6673b36462dddc2b7c4d3fafd4b28a7ba0f5a4cccd6e5fd8423047b347', '\x015be2c184112eb16cc9d20443b10e40d39b8ef059870f6ae7296ee0e1d420212ddf3929e2a4e16eae109dde88', '\x7d273eae2eab1aea06dbf0df48a74210', NULL, 0, 1795638079539431142);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xbe3e78306dbc141c859e9c910ef2ca105652a9b690c85c99baa860d42fb3942a', '\x01ed3eef2e8c8a0981d6dedc8f20683a2d3b2aed128b05645ec82b3791d788eabfe93e3dda6eedfb323caf6bec', '\x8431f898c6e353a873f11871cfd69ab8', NULL, 0, 1795638079586766512);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xeeb7cb31f812a1b910fea37a8da38f8daa09cbf0528ad122d7cd3928072312bf', '\x01b9accb5023223b20d915c4ab801f70a42b40d5c6630dfe88c4facb1bedfabe407ad177e6c57b0a97d5152e33', '\xaaac2b66b4801d73c4003df6df4b7d64', NULL, 0, 1795638079631480137);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xf540275dcd3443b837ecd48eeb28a010c973da832e642d4ad14c4040a60a98cc', '\x014eb32622c514bebd79a3dc60fe37abd17b48009079f126fd1bcb00a8888c21c0e0b47c95c5bce911949c0633', '\xed630c63cb698264324f684d11b86500', NULL, 0, 1795638079652084325);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xfbcc69d59024be2a5b3cb6f33538699ba0b11bdf6585b0cd7b3775b4001f5a8f', '\x018d8dead299b38f85f156df2e37a5c1d52d844a8ba80888262d45c19b0dc47dd1ce2fcd21b4f076a08ee8f122', '\xb468dea8a0f25f2492a7fc6e0412cb5e', NULL, 0, 1795638079260390973);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xf3526afeb19a4bc8ed0fad4e29aece9a5229e00a7bbe2914157f6e5123c8a29e', '\x01f1e4d4a279003f004c261087d26a0dac836213967009d7b1c2f249fb61d38ddd4f960a7024143d84038b798b', NULL, 1795638079859374656, 1, 1795638079336440773);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xb1ec08b95e4c904cc203ce4590ff726660927864b39109170a47481daa2ba4e1', '\x0103e80e8d237917f34f1327bcb1cf80f3f39c5dee0df97ad8024f034b2e3275f8de0a688b7bdc37241e2b7842', '\xe1a28c001aecc65186d9fbb16fbe36e2', NULL, 1, 1795638079379562718);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x50798757e03c041a29340b663fd323c0e230a741cf162a7c238710b970e377b8', '\x01675aafda285a695baec3d00dccfa1119c19b4b763ab6d17e80179c08b8c07347a60c5a20b4832a345fffc846', '\x350d79fa516357cc3ad07633930cc7be', NULL, 0, 1795638104240118922);
INSERT INTO vcon.vcon VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x951d0908b03f3cb28575a7922d595de41573da590dfacc77ef1af9ccb8e3c114', '\x0162598bac6f750019e50f865f95f9a0c8b32827b398f3a86e1572cb60c02db02b19004edf36a0a20d60898790', '\x0ec853e86104b62e88cb4cc003169d16', NULL, 0, 1795638104288409180);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xc3ecd3e774b0a0a0f29364951566c0ef9c1ceff0550c9f1d59f78907ce6442d8', '\x01335d5197cbc045cd70510d49c9874385f02bb5ef9d37af6b808e9c0cd21720a3669b442ad9a3caad60201aca', '\x017c646e115b0a5115434231cb50456b', NULL, 0, 1795638104307829644);
INSERT INTO vcon.vcon VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x86a48952e9c5d8277ec1c9de9fb7deba1e7e03451f7e3fb839b1b5750d0b5f19', '\x012e3146645bc3c7f5805b114e3d8d97e9fc28195973fd94898e29f47261c9a453da4c7b2f6cc9305036ea33a9', '\xe6e31b82d7cb999f9da60444a4ee670d', NULL, 0, 1795638104327131091);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xcb5bb899fc06402b8e53528d7d674fa0', '\xf3526afeb19a4bc8ed0fad4e29aece9a5229e00a7bbe2914157f6e5123c8a29e', '\x01a7fb19bd7a1186122a459d947b95fe78cc0ad47ce8ae1cc7890ac283f7a442eeb052d953', '\x01a2bf7735516eaefb4bfcf3348b07d3bcb03db6b87a5260e4ce39e9345a', '\x01dcbafba1730abde1840582d8b78fc4f76a35863edf63a7832cf189d0b2dba8e9e04e25c9', 1795638079336440773, 1795638079859374656, '\x6f725c4dffc508ad', '\x014c9cbb924f8efcbf798bc84bbaf210845b17d3b03b2df2e2bddd10e210ee45d31ecc3f840860f139ecc4b47c761a7bd3dcbd401a712918b8cc3170b4', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xc167152624a9a247e5ceb0e1967fc149', '\xb1ec08b95e4c904cc203ce4590ff726660927864b39109170a47481daa2ba4e1', '\x018efb6eb81b72b0bb65600d9512df9869d0ec1bff18a83f4faae089206eb80e5441b0dada', '\x0166d20b22c5f8dc6a2a0851a1b09b6aec4db1e12c85f9305b2ffc0ee45a', '\x01b8be21987dba0093c8586045c8617d7357f4c0be82c7e4dce25d03860919502fb8269c9d', 1795638079379562718, 1795638079909123074, '\x6f725c4dffc508ad', '\x01047c1d9fd8c7280a7b3c892969fad8611db9eee30c0ea7db11cf4db6c1188108fb794b64bb44d94ed98b0e34f94ef314211a1901d8ee79403917cf56', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x27e6b59b58db0f5cea62392dff305a8d', '\x482ce46c0d79705da8311c03d242294d532906c5fbe350d6214a18659f6580f3', '\x01bed10de68ebf3fd0bb180812bb3fd0d856e87592492cf3982528aa8325c9e37078cc74a5', '\x01a59f994566e073a99b6ae93969aa6605ff0b1cbe5381edc76355e19ef0', '\x01f3f5bb93e22b7a0e9e4f761baf77e14471bbd96ffd742a718c168d8ea05dad9b30807a4f', 1795638079434852036, NULL, '\x6f725c4dffc508ad', '\x01ce366fd6dd76879bf0cbddfb119008873c7cf74050c483d00dd50b1783ea1f2f3c5239c3e3168a4a2002f5eb93c986d6ad10dcd414cb692aa619afb4', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x547efc87c67c19afee06ded2591fd34c', '\x00930328d519086d192760e55b6799a893e0ebe5ecf5169479f965a714f23f97', '\x01ea7a61ef548541ed21804abbc8f3f36d7267d33acd9d1cf5cf9ab0eaad356996893d1bda', '\x01d520fa7384a56d73e62697db65056e882a09dc8147346128648c4eb034', '\x01875282a1be65d94f2a44bdd43a883d90b89041b1b9f30e3d951a67411807f308d999dfce', 1795638079493958276, NULL, '\x6f725c4dffc508ad', '\x017a2fafae3556a276c5ab9c713c531a6662e46f94868327edcf73b0e99f24fe88c44f85faac5d2263cef12889aef3dbc1c96154cca331a0c8f127e1e0', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x7d273eae2eab1aea06dbf0df48a74210', '\xdde40c6673b36462dddc2b7c4d3fafd4b28a7ba0f5a4cccd6e5fd8423047b347', '\x0171da0e0483df06fe851e4b862c0331674589268d84573d1eb13f6c98d211856dccd9f079', '\x015238ab9f9f2745e67c1c5629384947a5f037db33674565b3d28c5bad8f', '\x01774a212036e8c111456028e7890e84244e0474b35f4fe7db66f3cf8fdccf59cef613e46c', 1795638079539431142, NULL, '\x6f725c4dffc508ad', '\x01e576682d92af578364b0a98e51a0c9dc90b974cdc7dee7500a5b451de5d988da2212cca3f764393cff9ad0afd2e17456344b85b868fca742191fffd3', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xffcdf416f994ba16120fc7e0c5bf5373', '\x0c49ed63f2cf7a41142971ce978c98cd6d6edf2a05968eb5febb0d2e2bb3807e', '\x011e6f8b7078e269aee9da803504a2d5dd4071932daced4e54cb98a6b339f6f51ab9bd0312', '\x0131a88936c154b443bb1ef1abf6d15b1b88a46f9950cc0776c5ea83659d', '\x01a4ba0a427b50797daddf0727990ee4063c077433f7032b530aff035297008dbc1e84d7a6', 1795638078679591317, 1795638079182058141, '\x612bbb498c17aaa2', '\x019b263683757806a92a35d2bf4be8ec043b15b5d53f9b40ebd88e28a4c80c7a1a8f306347fe43efe74f81255a75383fa9a031e1890a90464a16631e3d', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x4a11ef0ebf82fb498625bdda1474f3a1', '\xd71954b5f55ddd95e32810f8b1e2ab1fd14247d65a3ffbd4ca721ace2e6d7078', '\x016bf13f1adc670fe7a9c900f8559e68595be4bd68a913a47634e3d64af0475a3ddbbf843a', '\x0127906e329df50779819679358d566acba8b43ce7d548c0da7e44e18781', '\x01f141b65cfa540e452cb0011d1e4c034e21a6a3e084a89cb8abe37ea4ec1c5e07fe00e610', 1795638078621585840, 1795638079133460614, '\x612bbb498c17aaa2', '\x015f678ffb8bc4ecdf63b5dd5a67a737088fd2785687a3a176ac27426dc9bc09148b02fd9034e8f0d44aeef097b56de828b18250944e2604150aacc1e3', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xddc49cf2b794712e93b407b39890edba', '\x727618e4b85c2be651ccf0758179c29fc61aa8c84cc45140613201085045c4ec', '\x01e9cae46094791e35d99ba25b7c27c62d31f3c494a4b98bf325e69713c98f3e9fcca3788d', '\x017be67ddf225bf9432b4c6c3996d44c5a850bbb935bbf1e303441f0987a', '\x019ff090e8e3633acb31eabdc72561c3bb40eb5db3dbc5aa3ef2bcb5006120bc2ed1f27365', 1795638078737526849, 1795638079188551383, '\x612bbb498c17aaa2', '\x015795a0d9f2f09b75fa3e70055e6b07d8b3ef6a5fd4ba4f747e5bab7d7b19682929c5f66cea31143af27439f62cf1c765799c855453918981ad6c3fba', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x4ad864faea1b391d370fffe7c95b68c4', '\x7c30a11ef273fd5f80a080ac196b69a95b76440009cf9565f22ae6fabf68d256', '\x0132fa5bb6f7e0ab3c6489174689d93cc2469a5aa1f9c9c5e58bc6bd25c282c139f0a155df', '\x019c11e8c7b97c921fe20460c88b0c6c18bf34560797fe2019c7e8a06ce7', '\x011903493aee7379f5163b854f230aa6b25cd065de6f56c606253583298364f1307bd76977', 1795638078782261409, 1795638079196069866, '\x612bbb498c17aaa2', '\x0155bd01f33e8aeab19deb2bf4b3b7696468a5720889427a322e96ab75323e2e9698bbdf0699b1943f47c65e847f22c38b43aa35b3c45547e23ebaf9ef', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x8431f898c6e353a873f11871cfd69ab8', '\xbe3e78306dbc141c859e9c910ef2ca105652a9b690c85c99baa860d42fb3942a', '\x01b47adec4fc327fb07e50fc7f9bb749f4afc0d1913ec3e559766f8f6a4eede11ec70de8cb', '\x01f1f1631ed5a9d24b41db2f7ee0e2b9d4a94cdbacc1fe05439d5cfa33f4', '\x0129f71399790e684d39e342cd9af7efb37979e658757f1f9d993c6591c0e401923e4c11ac', 1795638079586766512, NULL, '\x6f725c4dffc508ad', '\x0193983f71fab03c49bfa7d720e4a7652334dc6b8009861790189f76991ff02a90ffb873c65aa80e7e0c6ac5344f1f0a91fc5acd5423f298c34f0e29e2', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xaaac2b66b4801d73c4003df6df4b7d64', '\xeeb7cb31f812a1b910fea37a8da38f8daa09cbf0528ad122d7cd3928072312bf', '\x01a91c828d8339ecfbf8c5dc14cade99030a11c98218ed5b9cf05f624516e5860cdea6ddaf', '\x01622e89045e42af7e86fe5ce84e1643c01aaf95e20b61f496b36c0950ef', '\x012d5dc30c0c150118c3528f385418f0696ba3a0bc7a01eb33065002ad446b86c256e6125b', 1795638079631480137, NULL, '\x6f725c4dffc508ad', '\x018698d9e07a8b312a4578402e9345a7f925b7c1a4aac11cee5242b5d5865d92aa3425309075878bacd3e56698b2ec9547264908e7d7f64efc07021ac1', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xed630c63cb698264324f684d11b86500', '\xf540275dcd3443b837ecd48eeb28a010c973da832e642d4ad14c4040a60a98cc', '\x014cac106e5799c65c97bd1b622873ed49cfd65a90ba02bcbe65e3821d9a35832517b9e002', '\x013a5432cdd3c24e133d564961451b0ab286c16a214818a3dac697dab567', '\x0103e13cdefb6abe6bb3fecb990c1b2f93eb948075ba5a18e5585f7616b5b3a339e3484057', 1795638079652084325, NULL, '\x6f725c4dffc508ad', '\x01f465e0eb119a1cc6ec9b14bfe76501eee3ce5048e21de2c1bcbac3d158ae39c23aec8293b7e5b50736e8102680540fb89bba99e94fad0360f2a163ab', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xb468dea8a0f25f2492a7fc6e0412cb5e', '\xfbcc69d59024be2a5b3cb6f33538699ba0b11bdf6585b0cd7b3775b4001f5a8f', '\x018eef24bd617143775e1c0988598c49c369edf5297c8697e4568c36ce711ec7c0e5ac5126', '\x019bd356cc3768adc3f907552a927b3077b76b3fa4ac00f14cb141574804', '\x010a8e1ccd47b70f1a7dc8343e64df58c21cf2ffbd1d5c47c01f74d25de837a0fe120b6c35', 1795638079698616923, NULL, '\x6f725c4dffc508ad', '\x01e4191d50f49679c11c993ab806542306822cc8d1a181317b4f26f58e6a9be00f4035752a61c2f8741202b0929094eb07a6a93a0abf4caec9b3f9d1b5', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xe1a28c001aecc65186d9fbb16fbe36e2', '\xb1ec08b95e4c904cc203ce4590ff726660927864b39109170a47481daa2ba4e1', '\x011bc3b081a2621753c6eb2162bb3dfb6ed1524597e8343088feab83136a2461294b4a3f07', '\x016a6f0f3d26e3a88a64d7cdcfa3c4e8f43ddb93a9105382f30de834115b', '\x013a447ca2c412ab15c018cc1fbf85dd0761b4bac812028982053d97869414db6e552c5d76', 1795638079909123074, NULL, '\x6f725c4dffc508ad', '\x016853e8af81f36138308484a34cf6a44195afc736443a55a96a3fda14c8f95d0d3b307f6a4721302eaeae46a85945248ab0f5c5a31893bb978e2534c9', '\x4ea045c9479655f6b2b4bd52735dc0bd', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xade9c9d0c361c3698f3f023b2245a119', '\x3714ee5d517637a7ba1c6be505e6af8f5ebdcc3ff4a38d10ed628d25f37108a8', '\x01cfda6988a96713213d5edf9309f16f87fd44806f92b413a6393d5619b0e41f318e68d213', '\x014f56abbbe25d364a0a15f5c9433b4ec6250df32331f181fb7635c7a572', '\x0132ac87579b997be282b6c5b3da340132d0178a2f8c3e5067f7b7f5784f9e8b2b2782d2ed', 1795638078831771384, 1795638079206309558, '\x612bbb498c17aaa2', '\x01906a3f4521b751a4da4544a610c47e33600a8a3d68974d303832eee24dbd73ba6506baf70be2537489a973a7c864313e5fb93a81bceb2c56e20bf90e', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xb1a884cf8cb8d6e341beef005858bf04', '\x9deba1cbc84b3b8c81144ec82b9434b476ae8a3e7a8eae023f39c1ab2e4d0815', '\x019a4d59bc7aa8a77c31b4f89bc0d2c90dbbbfd70bb5756a6d66abc5bd5f82fef34d7c6503', '\x0145afaef13ae4dffc23e1f985fd40c5664d2ed69b5e024c764f27457324', '\x0196e8f27031d8983b161b02a16f2359a5e608eb3ea8c5c212c71e03263c69a82f30d34a66', 1795638078901959613, NULL, '\x612bbb498c17aaa2', '\x014a75c4d876100ea35b176882cd6668226a687ec91b9a77a8ef4319f3c7e48a117db49ae7f028514acf19e71e824fcdb4a1ac09de2f13cc56c9f1160a', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xb14f0bf0deec67640b05941eadebf44c', '\xa4bc8d19d9944553eef886d96697de091ad41a70303827ad47327bdad34ee7cb', '\x01706cf511140ccc185e6ca58c83965fa437761bacb1cdbd14f6aff246333c20d9a95fb344', '\x0168d6fc62d1682c38a9211f1885cd7c4456ec9196bd125199e9ed3cc0cf', '\x0187acda5d406314e92c6c04f3cef3ffc53526701a83925eb99c12de5f767bfa8c58a72bfc', 1795638078967681169, NULL, '\x612bbb498c17aaa2', '\x01983c0c63c0929ce0467d1bedaec9cd554cb2c1e5b6dc26597b1982ebe50622faa85af96dc96ba63782a6732387e9e5fa13500e8820515af276c02d9e', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xffc789073d539b3cdc9f058e2cd895ff', '\x01863e0240ec08fdcd7c669dc0fdf5b9475ab342d6a7206855ba846bc10eae54', '\x01b8a95a231e1d0ac361978708046700bee0c063c2aa360c7479eb6c39be6aeba2ce14f25d', '\x01d731c9382375c3d64f30ecc765f1eab3155c5e1687359ecc746dcd9043', '\x01b798554dfdea512aef67fb587de47c7b5aeae7dd2c2118fff19f7183ee2fdb1887417189', 1795638079021363325, NULL, '\x612bbb498c17aaa2', '\x0157791c1f8edf726c059d2293325824b6566b6a2819310e59712ebc1b78d602458694f745881f06f6a81b126212cb7996f4070fdd53b8cfb6d0ac4bbc', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x94f6002a6a45de9931533673dd307353', '\xcd0a02cf8ac0061b786c5f2d6c18b1275a9672c05e489f2bd1eca58876164702', '\x0114f65139e52e20d9acea2aac16d6b0fb79a8f4fc0986b61070eed9947fab3ca6a712fa92', '\x019e7d10afe2d08e49185aea01c346b71d8e066abbaba8a1a2dde2b20a4b', '\x0167be5af6c45dbbdaaf5695d9dc162d277039cf2f6735a0e7eaf3bb9b2a102bb3bdbe8d4e', 1795638079085766903, NULL, '\x612bbb498c17aaa2', '\x014bb53488acad804e3d63410a17f43a155761dcbcd80e0fe7c49c873ba9c763d2b8c022d6f6b8234a9454ee40054e4ff5a6fb0f76fb2bdfb30bfcd57b', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x746983e4fe27b3dce5929b1115425564', '\x5ce8bdf556c2d1b86380f946de246ba8cb71990939c03591c8331385b537c710', '\x013258b3e35645722bf0fbb93000060985b55838ced5f732bce18822eafdde4ea7129174cc', '\x01765b937cd505bae9b2af8ecff936429ce1b1afd48b3e8ff97949e6db2f', '\x01f9cf514ab8883898f757335060e033219a8e9deb7cae60c103f1e0670978c8dc758c1ec7', 1795638079105597258, NULL, '\x612bbb498c17aaa2', '\x01a55365cd250abf7c087ea449bd1abf913c3846335907049d66c5fd63a4ced07306e21c84a5f02e3f44b9440854e4258acdd0baf34b69103800aec245', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\xc30d6aa97e3538249a1502342f88fcc3', '\xd71954b5f55ddd95e32810f8b1e2ab1fd14247d65a3ffbd4ca721ace2e6d7078', '\x01301a934fb8f2a1868ca5e7f54ff833e75a6d7a28af490377e9e7746a1c34aec020fe0f74', '\x016fa61f98639167b45071bac37172a23195fea9ac2ff3bff22ed8bd94d3', '\x0136ebdcb58016f672719fd530483d2ce42cebf4f87c9a8470a07469bdd80cf846a548307f', 1795638079133460614, NULL, '\x612bbb498c17aaa2', '\x01119d2637e8b92d1a95f29a6530993bb5fcceb0b8bb2a4f5082bc0966cf367175a2e1fdea05aca36a1c6042595c1cba365441f6880bcd7da2e3e82a4d', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x87db6510a4daf044591fc601d32e9fd0', '\x0c49ed63f2cf7a41142971ce978c98cd6d6edf2a05968eb5febb0d2e2bb3807e', '\x01e05e7b6e9cdf528179ba4fffd39d5d7898080e9eeebc47a3bc0f01faf18b205fe25f2106', '\x01c518b3e887b80206e0c2b8ce754115c95906d324edfa640c9460edfab0', '\x01888c29259cf226b505b2eb0e3b9fb709dbdd3793a5f3027a5bfdd1497661dbb1838800b4', 1795638079182058141, NULL, '\x612bbb498c17aaa2', '\x01be3e0c712978a7c2b5b9a60bb74fe5c7bbcea42f2d014c501e61c0e906f2f35b638dae18a47935e3f108867cb88f01ac3690fc6509b8ad9f1dc25236', '\x1c90c72e3a9a651537c80e384c1f5f5d', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x350d79fa516357cc3ad07633930cc7be', '\x50798757e03c041a29340b663fd323c0e230a741cf162a7c238710b970e377b8', '\x01ef86694bda188e804e67996597d592a0f5ee8c8a38619f6511108440dcf5d26b8eeb831e', '\x0154e2a02225b11eb36b94f7e0bf10db358d311da9839bc2b747527d28a6', '\x010faae3591377b6f26651bb79f2944377c8b30c4c35dbccf301f8edfcb68912012bb865fd', 1795638104240118922, NULL, '\x612bbb498c17aaa2', '\x0149586b1e471aabdb388537734362926067e92f502da68a3764970d3d716f7a4480a4e0c2376bd5b98ce7935bc74cc4dff7755620f012a8eb904db3f2', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x52b7c793d8ed0394f3e77d034bed56cc', '\x0ec853e86104b62e88cb4cc003169d16', '\x951d0908b03f3cb28575a7922d595de41573da590dfacc77ef1af9ccb8e3c114', '\x0172e687d08f0a630c1526cc6d3e9936d94f05cc2345cb362ed41c99830d0a8253d4732848', '\x01f3aaeb6e80c0b81264600460185f45e23a5de9bd671d5201e2f51900bb', '\x01bc8075264be3d3cefd8712a8c4a6b708eb86aa78d20ca64d62038d721abe566c382c3051', 1795638104288409180, NULL, '\x612bbb498c17aaa2', '\x01c9634b45046f66f67fef00e69dae6fa20f5f7713137063908075e42598d872f4acb4a235b90531c44a469fd1e1ba9e194c69cf8097afb98f159dbb9f', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\x017c646e115b0a5115434231cb50456b', '\xc3ecd3e774b0a0a0f29364951566c0ef9c1ceff0550c9f1d59f78907ce6442d8', '\x01ac88fb3eff761d324a64e642a4a3bb91c75b7f9fe49c52af531418c9b45174efe2aad979', '\x0149432691af65b6b091a8922234c318ebd39db3689f694925db464bd3f8', '\x01912f06ae0c5fbd4def1af1480c3095dfdc1844f93f9697e6c03dddc499a2a3aba9a8645d', 1795638104307829644, NULL, '\x6f725c4dffc508ad', '\x011d49406f44af62ad37f6352d77892f0496b3658f1661ee8770e7f89c8f44517e36ca080beb342ed64fd4db0ad032fbf8cd4c344135be4ea1703d0bdc', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xe5cd4acc6f03e69568766fd93846c156', '\xe6e31b82d7cb999f9da60444a4ee670d', '\x86a48952e9c5d8277ec1c9de9fb7deba1e7e03451f7e3fb839b1b5750d0b5f19', '\x01acdabdd23cc5016452ad6bd09c0b7e583c445bfbd3b2624af854d5b2f56c28b215a038ce', '\x014c3fa01b6042893fb2d381d813a5ce79bacd9fdf96a5adf233afe820b6', '\x013cf4264cbf84a15aad43765497ffd56ee909f0c517d160efd3793a060011c3f4a847fa35', 1795638104327131091, NULL, '\x6f725c4dffc508ad', '\x017d4b9743ea75da117c6bfa415c3b64bd3e2fd8024314bfd7f3748c9c18e0f59e6645db5ef8631fb57c624fa8aa467ca480197ba333a2c3af8aa6bc6f', NULL, NULL, NULL, 56);


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


