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

INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (2, '2026-11-18T07:02:36.58348329Z', 'cli', 'tenant_create', '\x28080795fe0057b6fa9d625d19dc519cc2c6a9315754062eada2a83508989028', '\x01fba94302757abd9ed12b68b4c412145076cad9d169da9059e8fb3b4bc33f78a660344aad806946fe99e3470629290779189660530a91465675d935c0', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (4, '2026-11-18T07:02:38.529683158Z', 'cli', 'key_create', '\xe5b44d6872d3ec300aa0d96398dca3273b4b6f580c57194b1f9d1a7b52812442', '\x0128c9897dd07f80b36655e4ebd7ed66cd8194f79820e8e0e161ccf8f08b12b61491c0d8382f18a9e2f773aa90', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (6, '2026-11-18T07:02:40.386791631Z', 'cli', 'key_create', '\x0460b7f2641ed8502cb1046af7fe9711c2bfca3c593f2ab920a75ada65b8bad2', '\x016bacbaef3a2839313efbb7e9bd1c767fecde79a07924b2292691273f14f5e9a5d22da812f6b58a449c2a4eb3', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (8, '2026-11-18T07:02:42.109103815Z', 'cli', 'tenant_create', '\xa9d9dbb914c945cb335d727a9b2fca9172b1d2374bdac52adc5d6c4ce01528eb', '\x017016cfbcee06d115c4f9d8ac62f87ef5848f0d2ab2cbc725e00618eb9bec7ed92a0bda882f04627afb1d01bd557083bd3b05e18efa56f9af73735a96', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (10, '2026-11-18T07:02:43.764897207Z', 'cli', 'key_create', '\xd2ed02096cebd6a6d62706b1d2e0305abba618d9c83d998077fbde8aeb2203d9', '\x01bc11e3bb68ee3277c1ff32d4285229d78a4b00e2b05a0afa461306abbaecd7cb27e576d7611551c3b17ec253', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (12, '2026-11-18T07:02:45.355279276Z', 'cli', 'key_create', '\xdefa321f3bee6d0aeea8dac46b15f748ae6b3af0275d0f2735e3b2d11a61e5ed', '\x01f22f65a8042c10c61e95332b34830adaa3553f4894519aacba4900d9998a00c40e8b762c20a5d0df6ee60436', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (14, '2026-11-18T07:02:46.817828795Z', 'cli', 'key_create', '\xbf05aa356cbe24d99c83ab20a54ad4da95821e7e601a974b4d7038be64dfe272', '\x01712e1e4f7268a38faa4a44751919715ece692b3dc04e32782a6d0e6ba487be83b3e8da215827cf9ec28b2072', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (16, '2026-11-18T07:02:49.316037702Z', 'cli', 'user_add', '\x3d99223d66a1ba2d927a506ebd6c322a79ccbf0884617efd87e9044cf2cac2d2', '\x01beb821e7c1a3150d0adf7cd0e31283596d3f801a45b839303e630c661a36bf4c2a2263d94bbebb2a828b8ca0f86feae699f0ae091c82c123248fb737', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (18, '2026-11-18T07:02:50.858817369Z', 'cli', 'user_add', '\xea08d7a5fbc75d80bcd847ce909704080ee645341baa34089029bc917eeb0590', '\x01d89fb3a74fa430679c669abfd78ba96bda84c011688d73a0cb78e038cb88507977c4461702d2cc9ed8e23f4477d791acfda6e21d3b784251c5c399cf', NULL, 201, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (20, '2026-11-18T07:02:52.434669953Z', 'cli', 'membership_grant', '\x6262a2340651150bcd4345022417752b220125042376546701ba2fe604505ed1', '\x0115393c8d534725b87d37477bed4ad209991a3e502858a9ad1fb491e05a44ee9da2c2840a4269f951cbae72adddaf14b01262a99a272e3474e48e78debfa75221ce1f6d66bfa48dfc9f3d898cffa41ba315671d955c41dcdea7e74aa7ddb09dc16e7b3c08', NULL, 200, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (22, '2026-11-18T07:02:54.024526636Z', 'cli', 'membership_grant', '\xf9be73af4cd40ba198d561d8c63c8db12c30c9c0eae77e0edc8fb4a1a00933bd', '\x013c6717e525d335cae9e1446edc9c9c321e69a45b730dd218647448d8ab74dd3ea158125ecdd2816b06fc157741a94a6b3d78cbf0e9a454fa08b62dd04b07e30a85715450c7cd512fe6eec5df36303dfdf0410ceba3e01050a1588346eac4227ba38ac47efc', NULL, 200, 1, false, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (24, '2026-11-18T07:02:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 2, true, '\x7a1d89efe393b0d5');
INSERT INTO vcon.admin_audit OVERRIDING SYSTEM VALUE VALUES (26, '2026-11-18T07:03:00Z', '', 'auth_failed', NULL, NULL, NULL, 401, 1, true, '\x7a1d89efe393b0d5');


--
-- Data for Name: admin_audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.admin_audit_key VALUES ('\x7a1d89efe393b0d5', 1, '\x01fda9f26e4532221f84e0855bf80ed4ae79adbf7eb872e5132150baa1fd11b442aa8618d752b238ceb690ba6f1ab2aa6e324644c4302989a6e4333d55');


--
-- Data for Name: api_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.api_key VALUES ('\x24cc00914e541a9b', '\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xb84a3e181a7be6eee678aca8ae7a95287854186c5a98bd432d15cbb50cf7fedb', '\x012f2061f11306480d44758cf87617aca8a57fae7368aae737ed48f6a2184ae42c', '\x01200d9a836a0b28e7e9bb481776904ea1217fe75cb178200a2c66e6f4086de7dd0ddb119e021beead0d87dd', 1794985360386331493, NULL, 1802761360000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x96a8b3e328340b46', '\x2187366c4fad45f64fa312b2cbc61222', '\x03ea08501842863c8c853ed54f884bbcb49083803d8c5b4137a12d017ce065fb', '\x01193b1137d00d85d860a1fe847850bf5d6b1c373d6ac518e9272e3155f2b2f7aa', '\x01b8813dd56739e7f082f0a18856afdd3659e9da8315589643617e8c752589a870b37047e4e53afbb3d5edc5', 1794985365354882610, NULL, 1802761365000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x29c42ec136814f31', NULL, '\x5221084dc8ed792371afb05762e6b399cf33da4fbc8fdf29280b4db69a7e4c86', '\x01101e3e9eaf8fa5042ab8b3a02da5b6a05b274db83b6ebd04b74d455b1d7040b4a2', '\x01b8beefc8dc66ff3183025366ee630246bafc61dac09fbf229495fd03193502', 1794985366817326574, NULL, 1802761366000000000, NULL);
INSERT INTO vcon.api_key VALUES ('\x6cf9e079f3dbd8f0', '\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x4c3398b0c5e49908d1367b15038311eb4d3165e9a09bd06b9aab61c38870e5f8', '\x01cc623855c017f5281fe541f40e08b0b9608dc96dfb64ca725fe791438414884d371781368b0bd4c32c4a524470e05f2d3530e64c60f8cc8b98', '\x0196cea47bfe443ed048f0dfc1c6e329870d7d14228d87f8d5ef9d984962419485af77d815a5a6330b81d595a163', 1794985358528615605, NULL, 1802761358000000000, 1794985380000000000);
INSERT INTO vcon.api_key VALUES ('\xb5bf7ceaccb173b4', '\x2187366c4fad45f64fa312b2cbc61222', '\xffaf74ef6b10957d7a3e8e6e04e64d73d7f15c9e2e7a43788ef23d559660076f', '\x014bfba7aa9ce669523f229570b31232bf73eeef1be4a1b5b479eff528742ccaaa358726c01e42bceaa4e2bcac965349a6227b5524facaa4e648', '\x0112bb6fde44f478afd71a27623d3f4ffa7f0c8a708d177571604722414198ae5512310e960b959477b65cf5f8d7', 1794985363764498773, NULL, 1802761363000000000, 1794985380000000000);


--
-- Data for Name: audit; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 2, '2026-11-18T07:02:38.537152987Z', 'cli', 'key_create', '\x07f9860074077a01b98f8170ef34b5d5c3bb9a75d816a0e118ccc6748d9dfb87', '\x01bde7952f8801f10ee907989ea465d80ec36e43a0a11c2c248e80203eb123f7d12d0a92413bf3aa2beaefda2a', NULL, 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 4, '2026-11-18T07:02:40.391493507Z', 'cli', 'key_create', '\xd095971dc5a163ff3454f1ca4f5fbe6f4ec5220c975a202a1da6c4c79b41d147', '\x019097ca11935589fd2cb6176e28161141e184e77dd496089cfec653de9a5d40998248468b963079a0fde73071', NULL, 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 6, '2026-11-18T07:02:43.769294426Z', 'cli', 'key_create', '\x30fd5c0fb7f41d3f73a4e7f426d896462e405b1544a1b9daeab7ae7fc1d79516', '\x017d5a5bc8be0249be00659fe01a36fc49018689de5d703133017a68b46846d08e12598ad0a2ed89dd8b0e493f', NULL, 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 8, '2026-11-18T07:02:45.358521239Z', 'cli', 'key_create', '\x6f8d070683ef017f78ba1f95aa5fa7b625cae0a242a1fec4556ed8a7967bec8d', '\x01db06e05132184d357e062177744ed0ae7b2e6c44101746c42dcb4b14cbeeb6c2ba2ea7ffa014b7e4820d1103', NULL, 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 10, '2026-11-18T07:02:52.438151332Z', 'cli', 'membership_grant', '\xd45a293c1740060e9d46561071ddc6e52c4a0a2484c037c9c4c7ce601819483c', '\x013fcc0b9d8fff19de3c919d46fc7a1037039400b4d2028ef6b7b777d79455fd30024932accac51311689caf045b718209401c01a0589c0938a8623a9db37ee96e065217c50b37888bd5eb7ca6ae07e2ce1c5f10041758e0da2f306382ef4b1e7807f94837', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 12, '2026-11-18T07:02:54.027587396Z', 'cli', 'membership_grant', '\x5adc61a3d01912c70a38b699e314e0b959e760331842b046be6a20097c864dd8', '\x014107a7680afcb57e7218b8dc517d55c0a6e409c68b43a9f5936d2adfb99518639afcce7d3dfe20fc8b23ad677d7daa7af7f66c5dc6c8aebafafd274ca2760bd8bd6742de724acb3ddc75077c334efdf8784ef0a64719a165e0639fbb32d94a9e172855f280', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 14, '2026-11-18T07:02:56.005891157Z', '6cf9e079f3dbd8f0', 'create', '\x3f1567253a1a40e9cb841a2b3be1585288c4fc78cf85c58da21bff5100ccad97', '\x01412a1cad94302bbd4dd6fdf37a45b7540c83fb170b1d20b5a98ecd996cd4718e61e98facc09a33a400c5c618a61bac9c7095c3f010866f5ef3f9e37ac73445d0', '18afecd8a57dcf5dd67532372e060514', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 16, '2026-11-18T07:02:56.015851035Z', '6cf9e079f3dbd8f0', 'create', '\x9ea985abbd42e1f0df5be76687bc9ef1b2a28dd0c34076057378ae21c6dfbc38', '\x016f634977354c5c6d6efb8a3a906eb5ec3804c486aeb0be9dd72a2c767f1f1515b75bb965f6880d1ed80c0f46f336b2164da834dab96f649ea0ac427ce1a957cc', '018947c65a48ba581c78c683ecaf94bb', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 18, '2026-11-18T07:02:56.025707721Z', '6cf9e079f3dbd8f0', 'create', '\x0038132fd705d55b10034d6e6762337d0fda6f27424743111f03621be20cac64', '\x01d483620b6668e13ba226845a62c345ffe7e0471177d409b7460df512c6854948a845383e2265f2c0b5cb17c17d3458ccdb776e9df61140bc82839895fcba80b8', '4dc747157eeb6ec67a97905902833387', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 20, '2026-11-18T07:02:56.03429977Z', '6cf9e079f3dbd8f0', 'create', '\xb647834584e19b44d8ab65448f4310a0e4f76af0a743073867780886b5d7b95b', '\x013e95f576c9165deb83d253991c60794f06bcbe6fabc896e0b93384fed42e1c3e2d28c0e39d2aa9826bd017df438f61facc6f89638d1ef6abd8e0e5ab8f3f6e0e', 'ecc80bdd5287e3aa7a0e2fb25166e2ff', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 22, '2026-11-18T07:02:56.045533617Z', '6cf9e079f3dbd8f0', 'create', '\x23b36925239bfcaa9a5462aa12d8ca6f5622fa5a133605287da408423b18e7c9', '\x016de19d9922167399b68db1b52ea76b88b3959a6d1348819ba0be07d922feb22b24c3cae4747580da85c58667ef7302b91d9780e44a4cdb04b2600d0b597705c1', '2ead207d8f702865da171d08b8ec1808', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 24, '2026-11-18T07:02:56.056254069Z', '6cf9e079f3dbd8f0', 'create', '\xc5d76b8305c1151fc026244ee86603582f4c0d431056af6d6a4a1b0a2f3d19ac', '\x01245244d1fef6f49cd376e286f6b3015f829a1fe47acdca42b35de6ee9cbf6db2b7d7178297aea6455d31916156a41ce5004ba291a26266c97bcd9226dc45fa6b', 'f4e54bd7748a6aec780162e9fecce55b', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 26, '2026-11-18T07:02:56.064991682Z', '6cf9e079f3dbd8f0', 'create', '\xedda7e9e6fe491b1f59843f95037a3f8d9a737a040da8f29b64bb06698e49e55', '\x01c18bfd26a86a6ac495bb30ff5effa04db01d43ed11c6a84383acfd1bd99a72fb58a28a3e9d3a94a6af69c0250128962d61725d71539a324ba49aa2eab8200c17', '9330150147a350b084894941b759272f', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 28, '2026-11-18T07:02:56.073695787Z', '6cf9e079f3dbd8f0', 'create', '\xced0f66cc9dd508808347ce15b02825bc579dbb8405bf06421d150b3196deff2', '\x01775d3c0c9a72a7fb8ade106c25a40dcd8d08200c2912e3ebce734037efd59fe4417f1422f45151a116321da77e6c694fd25137c6c522a361ca8fdc64234df739', '27c2144f39d7d5e0c4547801921f677e', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 30, '2026-11-18T07:02:56.083193324Z', '6cf9e079f3dbd8f0', 'create', '\xbf9b74c04784430034b937e919d890a1f4553e50e1cb54163d44ad30b5d2822d', '\x01dd6f3f6d4a1e4042026ae06b171036c985655e0e684011e3d8a49a6f5723baf3ca7580491ec02b84f022e0688baf50d338c04a331c116a3c74b57a2fd086bad3', 'b908addcbd423da50d15a55770969160', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 32, '2026-11-18T07:02:56.091439209Z', '6cf9e079f3dbd8f0', 'create', '\x8352f98c5389921b12a12e0b328dfa85212a3bb44ff76a74207a60cfcede2531', '\x017a5fb66b672f11501dd641aec6795b97e40f17dc1965ddcc621414382cd06489ddb269c8272c442cab73366515f512f410ca0ae1e61fff7bba40ebf7cb23d8e5', '7898325204832c41ba5c6fc2b44cf9f1', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 34, '2026-11-18T07:02:56.101524596Z', '6cf9e079f3dbd8f0', 'replace', '\x3f1567253a1a40e9cb841a2b3be1585288c4fc78cf85c58da21bff5100ccad97', '\x01d9b269ecb57ba72ea4d2725beff58ecee8c564c45d6e13c87898c60283ee6c37c9c25e01f32a5cbfec4dc785f25949ca68c5e977e3eee525b417c7c375ae22e3', 'ee705042ce0d8a6dc9108e620b27ff8f', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 36, '2026-11-18T07:02:56.111389494Z', '6cf9e079f3dbd8f0', 'replace', '\x9ea985abbd42e1f0df5be76687bc9ef1b2a28dd0c34076057378ae21c6dfbc38', '\x01c57d1b6fd6706c5ff01234a53f7bcc412d2eaf97aefd324912fedf013152cb1f81cc3f79c3784cfb7e14df3251ff4dedb9d81f33e52acc1de18b2a8e3d12f5ce', '76230137aac4ad5735f91eac7aca6c6c', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 38, '2026-11-18T07:02:56.115394529Z', '6cf9e079f3dbd8f0', 'delete', '\x0038132fd705d55b10034d6e6762337d0fda6f27424743111f03621be20cac64', '\x0168a86913f8ffc5011a855b143912d2d347592992915f0aa2dcbcd86427aca07407617f58bccff8ac03e6ff79cdb1e224c29636149fc2f872a492dec15f46dd7c', NULL, 204, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 40, '2026-11-18T07:02:56.121851761Z', '6cf9e079f3dbd8f0', 'delete', '\xb647834584e19b44d8ab65448f4310a0e4f76af0a743073867780886b5d7b95b', '\x019b595654f8ee14fdd3542c4098b1c3c3dc72d1f938f7bc3c339aa2df479116afe04b4e7a48581d9b3a6caf3070e292075fc39b14ea0b6b31f65aecb8c0f001fe', NULL, 204, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 42, '2026-11-18T07:02:56.12683407Z', '6cf9e079f3dbd8f0', 'hold', '\x23b36925239bfcaa9a5462aa12d8ca6f5622fa5a133605287da408423b18e7c9', '\x013e4683170ef7c97c06dd763ef2fed7230e02e686c1eec7c9895be74db649b1c2b3fc85e19635f98ba77ffa1d22c05427e64620f7093514aa9695f4add9aad0c3', NULL, 204, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 44, '2026-11-18T07:02:56.130344366Z', '6cf9e079f3dbd8f0', 'delete', '\x23b36925239bfcaa9a5462aa12d8ca6f5622fa5a133605287da408423b18e7c9', '\x013463d6218646bc3afe2147086d049a30c7f47b09a82a0876b9b2fb95bf213e111012d018d02504a1ea63b932a61f05f30bd7cd8da839956a078cb9aa928160b5', NULL, 204, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 46, '2026-11-18T07:02:56.135139029Z', '6cf9e079f3dbd8f0', 'hold', '\xc5d76b8305c1151fc026244ee86603582f4c0d431056af6d6a4a1b0a2f3d19ac', '\x018c6e49850c065c6033f8e71206472b5fafaf09ba36dd7cefe17ab841fb4f6654a865a7c1ac192b264813392586ab3dadfe8a36bbf983c243e45b3f276bfa9571', NULL, 204, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 48, '2026-11-18T07:02:56.14372056Z', 'b5bf7ceaccb173b4', 'create', '\x62df4eb9a422f26831650da5f77c889b07e13d68783dc63c3b4595c8108e28e4', '\x01c985293f8925edbc6eafdfea95f4c4197fbb3e9f3a755d52feab8be099ce9e3a67566ef0f7a082a0c978a5b5ef3dde93a8c8d6ae5849888be95b03d52ffabbbe', '0e6bcf75ceddf0ce9611b14f9ce91503', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 50, '2026-11-18T07:02:56.15180438Z', 'b5bf7ceaccb173b4', 'create', '\x40c1595f467625d6e6bf5f569e2c47b25dcc9365a985ce329801976841d84c09', '\x012beb4efd5e1f278ed97cfdd592f1bd4b9440d3969f9046639daa46d885d0b7dfc6cd7d8b81dd635bde6d5e045c2e06ae0eaa2f124ddfc8c32f0256165ac35e87', '92cf19fbaf3f2e70314ff11c707b103b', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 52, '2026-11-18T07:02:56.159592108Z', 'b5bf7ceaccb173b4', 'create', '\x62812e843e2c32791b9e1b85ad9a17181e56b4f4d4ff8eac73816c5047f3d019', '\x01a890f23756efbf68bbfa7d9fafca2ef4a5245241a1fecb0f83736c49547605927977d10f1c38b77beff33e3d171db6791cf867a8a91763ce5c6745c5000d9281', '864e46cb70a60f1b7ed9039bd0ae2783', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 54, '2026-11-18T07:02:56.167504188Z', 'b5bf7ceaccb173b4', 'create', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x012d5c7341c8552ae3ddba31b12fd34d655522a0e72f250f50ac19b0dafce314bcd52cc433a399c1e750b9b1eb5d25445768041cfa5bb4f92826a887db5249cb74', '3be0c6ccfdf0b893d76f61f207bab69e', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 56, '2026-11-18T07:02:56.175315583Z', 'b5bf7ceaccb173b4', 'create', '\x2e213751fd4a090f30fa7442d596e84213a555682bb31f6b852bfbdce6a1f390', '\x0107012ce5fd3f3d266e3620da59ee410add0988188607e96689481a7a4356eab3adbf1d23a06956ce2e00c702ab85ab9de53bb7228a3d4bfbff1f22b6755cf101', '07f2cd753b91774b7cfc885e699888f2', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 58, '2026-11-18T07:02:56.182912783Z', 'b5bf7ceaccb173b4', 'create', '\x3b3e223d01e3c37880301d88cc4f01ab9b6d936e4de29a48347cfdb983fa1536', '\x0136bf3e698ee25058bb6f2752d20be62e60484d3285363ce69dbf1e3e4cf7a145ad8d17c9b40dfe32225c47c0c79028c45f811560318ee7c6d6599ea4fb16dace', '9280879ddd11e10eaeea230130a1c6d3', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 60, '2026-11-18T07:02:56.190701039Z', 'b5bf7ceaccb173b4', 'create', '\x8cf136930a75852f74f7f18cd26fcda1e5ee7fda7a7439ac1c068ff9f5341bc2', '\x012e46770aabf64de42a1c41c3cac8b62ec49186aa43615a4f87228f611cacceedbd930580d712127c8becc6f9cedf23a34712fbefc42c7cfccba9de82663df87c', 'aa03a9c9b37afbea1c89346a782afca2', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 62, '2026-11-18T07:02:56.198646878Z', 'b5bf7ceaccb173b4', 'create', '\x30f594d5abd234877ec335b460986f1e1a8caa1b5993cf32f5e2b67cea2543e2', '\x018df01e13d2b0cff43d42085b7f01f22993d478ea028f79a878a1ab0fa53057c1e727d57b550a6d9b896075cd7f264eaa88bcc4fdfc99163569990b15bb65e679', 'd821b68af704a24dace086e1c9331707', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 64, '2026-11-18T07:02:56.206431596Z', 'b5bf7ceaccb173b4', 'create', '\x8855d493aedb47369b853d4b6eba16e20fb659a985ae9d11ec9508e8271fadb9', '\x01b3ec46b1419e0f5eb0a375f9d16f27eb64a4c755264c3e8f61061b96ee2377c910b03d15c5ebcea841f4692c04f68f0949d920f8e6ea7ff7dbef00d1f3d0743c', '3de02e6c4524df351f157f144027ec19', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 66, '2026-11-18T07:02:56.214100658Z', 'b5bf7ceaccb173b4', 'create', '\xe7e753097374b901cca55f598de1ca2bd282bfd22b65734ef6a97988d29ead5f', '\x01591c8ea3245d0ba47ef663770562ff8135b6b46ff42f4fe64c49fd1e5602393ecc8cf6ec6d36bb9bd5649e75d490ad5654027a44802a3aeced886e5f6af7105b', 'd061d9d8afaf91e5dcfe49cb8f1a67a0', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 68, '2026-11-18T07:02:56.222980633Z', 'b5bf7ceaccb173b4', 'replace', '\x62df4eb9a422f26831650da5f77c889b07e13d68783dc63c3b4595c8108e28e4', '\x01106ce3ebd5853a1ba1138389164b54ea48365369b8f5b8ef9639bec352ea8e0c4e6ccdc82521fa4f184e373515bbafce2824ac5159622a57b7e117d0f80f05d3', 'a9ce7652a6173081d7d7f58c1801e256', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 70, '2026-11-18T07:02:56.226674577Z', 'b5bf7ceaccb173b4', 'delete', '\x40c1595f467625d6e6bf5f569e2c47b25dcc9365a985ce329801976841d84c09', '\x011392df2c1e36b842c3c2a48dd1c8cea7b28cb88a5b441cc2cc905076f19d67c28e5f94f3e46cdad5d6459be71f3341ae2811c0e4ddad06215c9b66ae6ed5ba2f', NULL, 204, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 72, '2026-11-18T07:02:56.228931459Z', 'b5bf7ceaccb173b4', 'purge', '\x40c1595f467625d6e6bf5f569e2c47b25dcc9365a985ce329801976841d84c09', '\x01e25e2ee248830ed13d6eaf5ea3c8f1078e2ccef506ad7749a4d902f7eb854aaa7d678de5fa153aae196ca26198581147d906b724f96143a1bc2b1df7f63be42e', '92cf19fbaf3f2e70314ff11c707b103b', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 74, '2026-11-18T07:02:56.246164503Z', 'b5bf7ceaccb173b4', 'hold', '\x62812e843e2c32791b9e1b85ad9a17181e56b4f4d4ff8eac73816c5047f3d019', '\x015e23b0c27192e20ce16d701972f9c34da88a956bda33b0c5eb990197fafc6e313926ff6df0760f505e3d5ac2c0d900a33fd77ffc9fe3daa1c49ae312a97bdb23', NULL, 204, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 76, '2026-11-18T07:02:56.250073964Z', 'b5bf7ceaccb173b4', 'delete', '\x62812e843e2c32791b9e1b85ad9a17181e56b4f4d4ff8eac73816c5047f3d019', '\x016ae8d43511e28777ada0a659f4da77e4be3f25eaa09d32f777900e38c280d7912849bc1a00298b98628b4cc285c4403f76341a5d98ce775532290ef71fcfbf8e', NULL, 204, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 78, '2026-11-18T07:02:56.254876237Z', 'b5bf7ceaccb173b4', 'hold', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x018d2336121928022cae9a693ce2a6a4cbcc0207aad07f3f3118bda60ae8b6b20ece2d1db75f295f731d0dba10db1800955d6bcc89ebb8956784b50b6e1e5635dd', NULL, 204, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 80, '2026-11-18T07:02:56.26422223Z', 'b5bf7ceaccb173b4', 'replace', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x0132629e5bec74c72cd56573bf7f76ff0a2324efaf057ec8e0519581ea4bb47badaf0a778953e9fea473868a4ab0f731826cd3bcfede84c820fd104a46af1b1179', '324a32c23c1528ba64d3f17fa920609d', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 82, '2026-11-18T07:02:58.062931359Z', 'sweep', 'purge', '\x62df4eb9a422f26831650da5f77c889b07e13d68783dc63c3b4595c8108e28e4', '\x01deadc9cf84d2b44568ccb0ea604fedf3e033f179ace677989d62d5c80dee34c15cc7411e9311865567d8e02b024f15c16e66b05944209a5b327b12b2a2ca120f', '0e6bcf75ceddf0ce9611b14f9ce91503', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 84, '2026-11-18T07:02:58.44317918Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x3f1567253a1a40e9cb841a2b3be1585288c4fc78cf85c58da21bff5100ccad97', '\x0109c12134fd6cdfbecaaea5a3003119539f18d95c4f9a82842d628e6269e53b0c746b4d5367c51c390773f81b5942103416a361455bb9b6a821cab73fb9c7fdf1', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 86, '2026-11-18T07:02:58.450171743Z', '6cf9e079f3dbd8f0', 'legal_read', '\x3f1567253a1a40e9cb841a2b3be1585288c4fc78cf85c58da21bff5100ccad97', '\x014b9bdd93620608cd5e9893ca2aa23850a572a5b1ca74cfd67c7776abd3827b38288daac25fb369826f9425367e226df0152f0b6619c536f365df85738648bf53', '18afecd8a57dcf5dd67532372e060514', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 88, '2026-11-18T07:02:58.455559636Z', '6cf9e079f3dbd8f0', 'legal_read', '\x3f1567253a1a40e9cb841a2b3be1585288c4fc78cf85c58da21bff5100ccad97', '\x016558b94e1882ffc13f1e82f7f68f046d28aad6f8bd59c9c1cc1fa1be9a39c9f825921961560363c78a044bb5a99923fb7200dc7d54182df4ccf4d3abccab0ffb', 'ee705042ce0d8a6dc9108e620b27ff8f', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 90, '2026-11-18T07:02:58.459338819Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x9ea985abbd42e1f0df5be76687bc9ef1b2a28dd0c34076057378ae21c6dfbc38', '\x01c22bf64a977e71e3ef89fecaa1da775ef2b354a99b590efa94b5a8ef519a809eab9d9e15c9dca56a1f7809c5eeb93311662d13aa1d34e05258a971ceb9406832', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 92, '2026-11-18T07:02:58.464401898Z', '6cf9e079f3dbd8f0', 'legal_read', '\x9ea985abbd42e1f0df5be76687bc9ef1b2a28dd0c34076057378ae21c6dfbc38', '\x015dd01361cf97e045a89737c06cc24b0b7cbd55fe742639d18e003a54ecd3cab7b12e87eb27eb2c9efe9395932348c209d01723a6c6a2e4b8148454ace71e1609', '018947c65a48ba581c78c683ecaf94bb', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 94, '2026-11-18T07:02:58.469626374Z', '6cf9e079f3dbd8f0', 'legal_read', '\x9ea985abbd42e1f0df5be76687bc9ef1b2a28dd0c34076057378ae21c6dfbc38', '\x01fa16db7469f1043fcb7a27d7d78dfced2771981be0b0ac1866a87c0aa43fe2a097a6d457086576ee746c08c9e679f48e04337911e7d21d5ae80994d317624168', '76230137aac4ad5735f91eac7aca6c6c', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 96, '2026-11-18T07:02:58.4747116Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x0038132fd705d55b10034d6e6762337d0fda6f27424743111f03621be20cac64', '\x012cd168dc69b1be263bd7d9d2ea6664da4c584529c0e7ca32d1a783f75b037c3a44a6c03326ce33a12ce904531dcd2f1286e964307571a091b0e2fd6c38fc5dcc', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 98, '2026-11-18T07:02:58.479546678Z', '6cf9e079f3dbd8f0', 'legal_read', '\x0038132fd705d55b10034d6e6762337d0fda6f27424743111f03621be20cac64', '\x0154754e6ba221f3a6c986abfcfbacb453cee0204bcdab5ac8d4ff9bbfb93dc67ac220624369e3f200a146ab083477faac6733756574aa192fec8095d9e24fa9df', '4dc747157eeb6ec67a97905902833387', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 100, '2026-11-18T07:02:58.482862012Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xb647834584e19b44d8ab65448f4310a0e4f76af0a743073867780886b5d7b95b', '\x0134c68b8ad8c8346988f04e020d6c08d40f5c00f33a108e369517c7ef47fbd57eb29e81ddb0b536d08e9cc3c4b2eb30a6a59d6a925aa07f6b3cdf296032d5e072', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 102, '2026-11-18T07:02:58.487459027Z', '6cf9e079f3dbd8f0', 'legal_read', '\xb647834584e19b44d8ab65448f4310a0e4f76af0a743073867780886b5d7b95b', '\x0157da0be5e18e3385e8ac62bdee051ed05db59fef162bb44ce0a3e688a66b3b1520b2b81570715ad075db0c7838e92bf2075377119e06280540e9082e2c216bb5', 'ecc80bdd5287e3aa7a0e2fb25166e2ff', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 104, '2026-11-18T07:02:58.490669916Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x23b36925239bfcaa9a5462aa12d8ca6f5622fa5a133605287da408423b18e7c9', '\x01fc9bbdfb4cdcef307bba9ffae49bcf32a54f459e5b9f419f8123082f279a4007b77cb3efc08266309c28161cf200c2c3946eb6c07f0a0618e4b7703406e9519b', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 106, '2026-11-18T07:02:58.495690902Z', '6cf9e079f3dbd8f0', 'legal_read', '\x23b36925239bfcaa9a5462aa12d8ca6f5622fa5a133605287da408423b18e7c9', '\x01b2da4edad34497619c3e25c804baf15041567eafae5bc6346e89d055a24d3d2b2b3b7e5265585a2e62cd86841d51f504d24e115ecb1ecf254ea445fc1405d4ef', '2ead207d8f702865da171d08b8ec1808', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 108, '2026-11-18T07:02:58.498992189Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xc5d76b8305c1151fc026244ee86603582f4c0d431056af6d6a4a1b0a2f3d19ac', '\x01b8fcb65d01a83533db83c87a128e84d9015ee73c62f1a86b6886dd1030c9fd00a37e4bc13be781b75a5b9b39bdefdc670ed0f61eed95ac6ae32c882f395cc116', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 110, '2026-11-18T07:02:58.503775416Z', '6cf9e079f3dbd8f0', 'legal_read', '\xc5d76b8305c1151fc026244ee86603582f4c0d431056af6d6a4a1b0a2f3d19ac', '\x01a346891dca3380a1b6e9c10338905c54f26c425b8f6e88f8b40c3bb0e35989b306d9a500fbc935aedeb680826b08e45762ffde455a7f6ef0fb14140eec56d218', 'f4e54bd7748a6aec780162e9fecce55b', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 112, '2026-11-18T07:02:58.507166888Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xedda7e9e6fe491b1f59843f95037a3f8d9a737a040da8f29b64bb06698e49e55', '\x010f95125d79840645eb57c4255a6ca8b8f0474fb4d1b70ee501e27dca7829f3644ad3b84c75b3782ea997171a9f606700dafb6ba31fdeadbf64b76ee20193e9db', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 114, '2026-11-18T07:02:58.511735866Z', '6cf9e079f3dbd8f0', 'legal_read', '\xedda7e9e6fe491b1f59843f95037a3f8d9a737a040da8f29b64bb06698e49e55', '\x015b906834df6d475f2d36f76d68e75eccd75336df47f5182318491647ef5cc4d9ce2fb5d2880514ff4cec6a6cb3fe30e036b2c992e9c2917522771608905baee3', '9330150147a350b084894941b759272f', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 116, '2026-11-18T07:02:58.515151791Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xced0f66cc9dd508808347ce15b02825bc579dbb8405bf06421d150b3196deff2', '\x01a4270a71f863f2ee9a6f4e699ecd1af63c0f5ccfd60255b6269db9bb91d9d01094165ffc427575f270e4c7827004b4d4da835b34fc786c6f9bbbca47213d6823', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 118, '2026-11-18T07:02:58.519677177Z', '6cf9e079f3dbd8f0', 'legal_read', '\xced0f66cc9dd508808347ce15b02825bc579dbb8405bf06421d150b3196deff2', '\x01e5efe181ee3b3da265056a4b6b4d00830d96bcc0319ffdaff511b5d41c7fc3ccec4797de17710effef9b3d381c67a4f353ef6ba3dbabe9ae6114e831f5a4d0e5', '27c2144f39d7d5e0c4547801921f677e', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 120, '2026-11-18T07:02:58.522986306Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xbf9b74c04784430034b937e919d890a1f4553e50e1cb54163d44ad30b5d2822d', '\x01313aa2d3c2a7ecd1fe6470706f5b55b9caba1f9b0a9fa2297c9ebb6ae36c894949cf11791baec0c5431377c065c63d0e602b8b56291995a688102762ca06d889', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 122, '2026-11-18T07:02:58.527535524Z', '6cf9e079f3dbd8f0', 'legal_read', '\xbf9b74c04784430034b937e919d890a1f4553e50e1cb54163d44ad30b5d2822d', '\x011a0b6611935c031acadd9af7dd59c8003edc2960aa8b08e68a03987ba302673385086e08a891d5903ea4adcaa3dfae812e7b64d98735c5f86fdb30ec54ae6dbd', 'b908addcbd423da50d15a55770969160', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 124, '2026-11-18T07:02:58.530866986Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x8352f98c5389921b12a12e0b328dfa85212a3bb44ff76a74207a60cfcede2531', '\x0120c58239414c9f20005ab350bfd45bf2031675948adad20fc1aff338b5295c59dfdc02dcd08a24c091216ab5ed8cf526134ab6d4606a14bb8defac323812163c', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 126, '2026-11-18T07:02:58.535473789Z', '6cf9e079f3dbd8f0', 'legal_read', '\x8352f98c5389921b12a12e0b328dfa85212a3bb44ff76a74207a60cfcede2531', '\x01871a9f7cd9d8c348301b588647efe9541d6cffbd856647a688009abaa2d0a2801fe7408ae5d6ca49cabcd37892a1d515e36af90fcbebaa0d97d083ce037fd785', '7898325204832c41ba5c6fc2b44cf9f1', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 128, '2026-11-18T07:02:58.538840483Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x62df4eb9a422f26831650da5f77c889b07e13d68783dc63c3b4595c8108e28e4', '\x012ee28ec7c3573a526b9ac6e52d501aafbc867179650d3cdaaa323b0b1ea4ed93df4d5246a0b4be7348f743b340744d308a2ee35801855f414648af05f18b036a', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 130, '2026-11-18T07:02:58.543362868Z', 'b5bf7ceaccb173b4', 'legal_read', '\x62df4eb9a422f26831650da5f77c889b07e13d68783dc63c3b4595c8108e28e4', '\x01f0ec7a6c23da6e3a705ed786a19663061557bff391c042f5d9e36bb3f921e91e8f6d5399319ce3f51f3dbfc652db412b80657f68e73c02b3227cf5f71164b05e', 'a9ce7652a6173081d7d7f58c1801e256', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 132, '2026-11-18T07:02:58.546239971Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x40c1595f467625d6e6bf5f569e2c47b25dcc9365a985ce329801976841d84c09', '\x01bb3bb4e055e7cc744551bfe138d9279f57d00630eb080a4b850bcd642d20929b2af309d223c7453d2a983e23977521f19801429faddb84a55769141073bef181', NULL, 410, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 134, '2026-11-18T07:02:58.549377989Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x62812e843e2c32791b9e1b85ad9a17181e56b4f4d4ff8eac73816c5047f3d019', '\x01f85da2ea26da029a97e1ac78b2c5b884bb40e87e5da0197f875a9f1acd5d752d21dd792a4114948f7065e451cfe847af923051a99f8f571911a62d2b6acc7a6c', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 136, '2026-11-18T07:02:58.553648764Z', 'b5bf7ceaccb173b4', 'legal_read', '\x62812e843e2c32791b9e1b85ad9a17181e56b4f4d4ff8eac73816c5047f3d019', '\x01f4da932b45d4f2c44d69cc034751264cef498d8c4692db9b67d39fbcaf2bf245d98be37fb34736bfa1c1ccda593a021b694758d2357162da955f89fc22a33516', '864e46cb70a60f1b7ed9039bd0ae2783', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 138, '2026-11-18T07:02:58.557001829Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x01e405a02853576d3d97d219240bee0eac7ece3adc78cc9bb81d61956414a8ef06ee5ee2f195726624727d8c8feae97bd0cb63b59364dd2650a1037820f1d60777', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 140, '2026-11-18T07:02:58.561307585Z', 'b5bf7ceaccb173b4', 'legal_read', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x01f70af2cfedfdf3ae43386d9bf1c3546a4ddb8b85596a201f7d7de65dc5a1db44e6a76b857f5eb894843aca56b32186b1a3589093a3f56cc9b1c4fb333353437d', '3be0c6ccfdf0b893d76f61f207bab69e', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 142, '2026-11-18T07:02:58.565747452Z', 'b5bf7ceaccb173b4', 'legal_read', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x01f62b13e93282eca4ce8a4956604a3e62788f891a7becd627d97885beb2a5445b15b8d0d9445a8bb2bca0a7b858e04e67f4f5df886ac326ab589293a6822cf8f0', '324a32c23c1528ba64d3f17fa920609d', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 144, '2026-11-18T07:02:58.568838767Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x2e213751fd4a090f30fa7442d596e84213a555682bb31f6b852bfbdce6a1f390', '\x015d2673813440598b7d1a882d69a35b7dde806d62807090db98a43abcea800dd4da483412c58e9b3eef8321d00c8bf6c68f58e07022ba791b4acff5b852e99462', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 146, '2026-11-18T07:02:58.573263782Z', 'b5bf7ceaccb173b4', 'legal_read', '\x2e213751fd4a090f30fa7442d596e84213a555682bb31f6b852bfbdce6a1f390', '\x015ce3ced45ccc32af64117349d042ccf9b9e83a72459a8f72d92d91ef5338b581bca6f8dca7937155b74bb06b212bdefdbc277bab403835685ebdc92e932493e3', '07f2cd753b91774b7cfc885e699888f2', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 148, '2026-11-18T07:02:58.578466563Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x3b3e223d01e3c37880301d88cc4f01ab9b6d936e4de29a48347cfdb983fa1536', '\x01f134413245893bff26496e85031902f9c905afe9823d27053c26dcff7bc8e3c5ed5fedae9d33660fcfc7af78a69c72a12ac53b2a5eff7c3ba2c0510c7359bf3c', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 150, '2026-11-18T07:02:58.582959681Z', 'b5bf7ceaccb173b4', 'legal_read', '\x3b3e223d01e3c37880301d88cc4f01ab9b6d936e4de29a48347cfdb983fa1536', '\x019a22a3f758679d91899031e80286ff3dfc6e8ba5380e47c2c6507f07a64d1ab26c2a8631c4c77f683aacefbbae71a559cb9224349bb5871f511408b93c2213dd', '9280879ddd11e10eaeea230130a1c6d3', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 152, '2026-11-18T07:02:58.586132376Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x8cf136930a75852f74f7f18cd26fcda1e5ee7fda7a7439ac1c068ff9f5341bc2', '\x01b994cb0bd1e253ab6b9ce3312d2658e32a107ef515c3fcb725ece68c8a632a520f6a97c2d475a04c6cc2eaf65a4cd3cb7351169f4a35093602c9b60567b13a58', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 154, '2026-11-18T07:02:58.590558029Z', 'b5bf7ceaccb173b4', 'legal_read', '\x8cf136930a75852f74f7f18cd26fcda1e5ee7fda7a7439ac1c068ff9f5341bc2', '\x017c8793dc946848a0fc30085cfcd7348bf2b10918aee0f74022ef0da0ee285f10f65bb5460a1e0f420b207df224e7a4b01420792e1a2c3bf136cd267f10ee2c93', 'aa03a9c9b37afbea1c89346a782afca2', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 156, '2026-11-18T07:02:58.593678391Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x30f594d5abd234877ec335b460986f1e1a8caa1b5993cf32f5e2b67cea2543e2', '\x0183a652b29e60e54f0757747cd1a4a50d9bc702c71bdc7464c31a64ca1dae115626553321dc0b75da972b4a79d3d6afe874d342c0597726b754fe24384f12bda3', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 158, '2026-11-18T07:02:58.598094091Z', 'b5bf7ceaccb173b4', 'legal_read', '\x30f594d5abd234877ec335b460986f1e1a8caa1b5993cf32f5e2b67cea2543e2', '\x016b0688870923b881a61bfdc1b1431d943670137412017e2f74d948fd713daa8738ffc662a5c837800805ac4212bacd6f312aadcc2a3293b37980a64a015a8318', 'd821b68af704a24dace086e1c9331707', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 160, '2026-11-18T07:02:58.601142277Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x8855d493aedb47369b853d4b6eba16e20fb659a985ae9d11ec9508e8271fadb9', '\x01394ad8a53743a36e15a89452b5557fd1e3ee1f9990fef8b53546945ec61fecc001af0fc240e6f2815733acd7611f7472e5410351fd6c5596cee3081c70c351a1', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 162, '2026-11-18T07:02:58.605555256Z', 'b5bf7ceaccb173b4', 'legal_read', '\x8855d493aedb47369b853d4b6eba16e20fb659a985ae9d11ec9508e8271fadb9', '\x015423217427bd0423a0aa77e6aae42afc59a29ba3bd287745656617e5c43254a0bde0a7c99e6cad49025b0fa7c7cb54aebdcb67ba6a0a8b58ea6de09ae531f0c5', '3de02e6c4524df351f157f144027ec19', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 164, '2026-11-18T07:02:58.608643496Z', 'b5bf7ceaccb173b4', 'legal_versions', '\xe7e753097374b901cca55f598de1ca2bd282bfd22b65734ef6a97988d29ead5f', '\x016e65bfbfb21f69356e0c7941c8bfc085b87aebafd2934d8d31a64dddf45e19a8e820180d716831b2f1559a17d03e3054d5f144b3b05405956e2128ab37415994', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 166, '2026-11-18T07:02:58.613117594Z', 'b5bf7ceaccb173b4', 'legal_read', '\xe7e753097374b901cca55f598de1ca2bd282bfd22b65734ef6a97988d29ead5f', '\x01d92c00b47d9fa2d3d2c3e1cd6e88dea6e7174973de03566966e989cf1d793b8423543ae6becd382d36aae25350c456491afa7bdab46b47ddce3d5b19b5a0fd37', 'd061d9d8afaf91e5dcfe49cb8f1a67a0', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 168, '2026-11-18T07:03:00.704523227Z', '6cf9e079f3dbd8f0', 'create', '\xba9a3360130c4cd9a3b94eecba1960a0a79306705d2945751b30fd92cc9f6828', '\x015c6c76878ab3b8945cb20265bff8e5095d5f318e6d47db4ffd4d310d679739b1dbaf073a0c33c8d33681d6406bf8bef9d9d4930eeb748f65656ee11b1bcd24e2', '8459150a9349d6da2c3d2e8493bd700e', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 170, '2026-11-18T07:03:00.714704402Z', '6cf9e079f3dbd8f0', 'create', '\x27786d31d7f47b19d6f696934223619f548f1f88bd169544c6cfb346994c6113', '\x0127b545e5cfee38c6d29dd3884b1313c38b4a12b5c141f4da8032d68774609281984570014c522805ba119f06230d7d7b53cb9949f3a3b6594992b6c9fe58942b', '47c07cbf1e504fcbabe77d113524ba9d', 201, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 172, '2026-11-18T07:03:00.724405004Z', 'b5bf7ceaccb173b4', 'create', '\x9787966ac8babf59ee775518327283bbcec8049d084ec94ec0a92a4441998a2b', '\x0123018f1d6be900f527964e87a341501ded8d484767c18e0b1f8c6ac24516e5067efa93200b2686b7bf6acf0165fbc82ecf7bce74e30319c0f8599e456968f6b3', 'fbb0a02aea001d0db954395057375c40', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 174, '2026-11-18T07:03:00.733454913Z', 'b5bf7ceaccb173b4', 'create', '\x3d9f618502d3ad79d8b6b3272f14d2f312e3fc81f48ee894dc28967a209095c6', '\x0138b3c9562bb9018e3c04b3c0e62d207199dab6b3205c2afcdeeeb4d4fc2f80ab4e63065151145b6f1b01b192ffd4048a08170d60cfee6e5d1b1f32b90b52f812', 'a9a2b96980f24f0460236329f3485b96', 201, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 176, '2026-11-18T07:03:00.73736931Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x3f1567253a1a40e9cb841a2b3be1585288c4fc78cf85c58da21bff5100ccad97', '\x0190a63e2806c297bafc85a889fba79e447695bffc03a4b13ec8618e5983096cb6b77b1f35fc36ff5c8c5bacd3efe5b7ec952f8840473f05d5d6907c46f0f6b926', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 178, '2026-11-18T07:03:00.743640004Z', '6cf9e079f3dbd8f0', 'legal_read', '\x3f1567253a1a40e9cb841a2b3be1585288c4fc78cf85c58da21bff5100ccad97', '\x0100e5416dfc0b09120a27af529e920fa617f62c0416dd15c7a595dd09d6fb2a02b004448263a6c56ee4bc41bad0dba94e90a917037f7703dc9924b0d507337ecd', '18afecd8a57dcf5dd67532372e060514', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 180, '2026-11-18T07:03:00.748425861Z', '6cf9e079f3dbd8f0', 'legal_read', '\x3f1567253a1a40e9cb841a2b3be1585288c4fc78cf85c58da21bff5100ccad97', '\x01dc61eb2ce5795423a3ba4d361a7c1b26d57efe7153e3fb2300d29aea84e88b5cd828a1847cd87bb265ffe3047cfb6c47af7a07a011f41007249cb13f8afef27c', 'ee705042ce0d8a6dc9108e620b27ff8f', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 182, '2026-11-18T07:03:00.752027378Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x9ea985abbd42e1f0df5be76687bc9ef1b2a28dd0c34076057378ae21c6dfbc38', '\x01d28b444ae2b786d823f30e21fb163e495126387192b064b75387252f1a96f97bcdb73d1b031baee577e18cbb727a1856eb8f827f691c7ac83c00c9be919f4627', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 184, '2026-11-18T07:03:00.756803976Z', '6cf9e079f3dbd8f0', 'legal_read', '\x9ea985abbd42e1f0df5be76687bc9ef1b2a28dd0c34076057378ae21c6dfbc38', '\x0122f0e766d11d5eebfe62ccd8aeef43c53d445d63e10b23fbdfbba727e3d460ab4e1b8e79e76f74baca7c0abfbee6fa5bd96dd7d165b7cce34591c86280292721', '018947c65a48ba581c78c683ecaf94bb', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 186, '2026-11-18T07:03:00.761245473Z', '6cf9e079f3dbd8f0', 'legal_read', '\x9ea985abbd42e1f0df5be76687bc9ef1b2a28dd0c34076057378ae21c6dfbc38', '\x01029a2f09350cdaec107fd90e9808785572572f8ef05407ddd5e71f2a77b4afb6c96da8f39ab1ec874513e83f2460485342ec790254e7c352d531e520283d1076', '76230137aac4ad5735f91eac7aca6c6c', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 188, '2026-11-18T07:03:00.764747963Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x0038132fd705d55b10034d6e6762337d0fda6f27424743111f03621be20cac64', '\x0175f8b96d479f6bb7a4b8f50ba8948fd913ddda38f892b0f3af5c8413748e8f370e59aaec0051912285ec0a2059fe36428afed9973b79f4af617869058179eed2', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 190, '2026-11-18T07:03:00.769252514Z', '6cf9e079f3dbd8f0', 'legal_read', '\x0038132fd705d55b10034d6e6762337d0fda6f27424743111f03621be20cac64', '\x01ce5f0238a24b8069d27cdad5ccc17a4bd226a20d8826ed6067ee3fd06e5a23239fdf2aac4b0a5224060b4c322f3de83c7e3515633b250e00888f7d3fdef51847', '4dc747157eeb6ec67a97905902833387', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 192, '2026-11-18T07:03:00.772678921Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xb647834584e19b44d8ab65448f4310a0e4f76af0a743073867780886b5d7b95b', '\x014ebe724142636b77791888b9a9bebc44ab954a8a316c90662f9e23b9c865239516c03117fc3b8ae9eeb9902ca838f0f8836ae1adb470ef00d958da9dd1fbad3c', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 194, '2026-11-18T07:03:00.7785872Z', '6cf9e079f3dbd8f0', 'legal_read', '\xb647834584e19b44d8ab65448f4310a0e4f76af0a743073867780886b5d7b95b', '\x01fb438ad11a3c9cc1f8857147877b3824b7e70f2def271ef2f53cc8ae3d4a505bb7aab48e577a7245800cb4c639afbda52cb7dfc53b5c655732605278dcd9678c', 'ecc80bdd5287e3aa7a0e2fb25166e2ff', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 196, '2026-11-18T07:03:00.782076043Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x23b36925239bfcaa9a5462aa12d8ca6f5622fa5a133605287da408423b18e7c9', '\x0152ffe4c8e6e8bde3b066463ecf5d99c476f1dd1aa99e35ec8c6502a89c6f490105e325f4eb207a8db9015d872a8b5d8c8f4d1cdfcb666c9f1563acbecde1c0b4', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 198, '2026-11-18T07:03:00.786533178Z', '6cf9e079f3dbd8f0', 'legal_read', '\x23b36925239bfcaa9a5462aa12d8ca6f5622fa5a133605287da408423b18e7c9', '\x011160981e643d8d39c04fc562dc20b7f14dbf53523479b6686d91b45ae0fcecb87b4e75ebd97fce7525374544b0fdd9f96986928ef5611dfe819a980d91f940c8', '2ead207d8f702865da171d08b8ec1808', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 200, '2026-11-18T07:03:00.789961677Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xc5d76b8305c1151fc026244ee86603582f4c0d431056af6d6a4a1b0a2f3d19ac', '\x01f9eca6b202a34f86c0d83684ea7a1c3e36b313688e51a773b24c84ef00d77932380de434f4d3b905e1767605d7f74242c027bebb8f5710d2294d742359d1800e', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 202, '2026-11-18T07:03:00.794308091Z', '6cf9e079f3dbd8f0', 'legal_read', '\xc5d76b8305c1151fc026244ee86603582f4c0d431056af6d6a4a1b0a2f3d19ac', '\x01c6e3e9417e365999e762465d447525b54fec5c3371bd35152a335d9d74d84da4813ef8ee73e0a30d7ba0dd5ff916d817c3aff357a09d01456ff730dec0139c38', 'f4e54bd7748a6aec780162e9fecce55b', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 204, '2026-11-18T07:03:00.798283089Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xedda7e9e6fe491b1f59843f95037a3f8d9a737a040da8f29b64bb06698e49e55', '\x0113fe8078b1defe126875473318db07d66abc3b5326befe8dea850768c33e322a99731603a6390611ae6000de0f37d2d2197b3b51e23ceeb5614302f30e90ecd9', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 206, '2026-11-18T07:03:00.802626104Z', '6cf9e079f3dbd8f0', 'legal_read', '\xedda7e9e6fe491b1f59843f95037a3f8d9a737a040da8f29b64bb06698e49e55', '\x013d5adbd14ba684a8a5c6e53154a2d867400f0b52ae14827ff98188a5746a5606ce1d4cdbb0532ed75ca77a6ac80e9fe932698e69224354b91fb6127c2ba61d00', '9330150147a350b084894941b759272f', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 208, '2026-11-18T07:03:00.805967984Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xced0f66cc9dd508808347ce15b02825bc579dbb8405bf06421d150b3196deff2', '\x011b891ca74fce046788c8a25ef87c69d087d89e2782ca13703562615fd3f60934a9aa64a7a1fdd87c3954c4f6c397506629eba8e3626a3fe372fa05432241c6f1', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 210, '2026-11-18T07:03:00.810240898Z', '6cf9e079f3dbd8f0', 'legal_read', '\xced0f66cc9dd508808347ce15b02825bc579dbb8405bf06421d150b3196deff2', '\x013b03d4b6e0a2c99e0389df39b20e70c6262bdae1620429403cf9220b5273090d4135c0502db1c30631a28d3494097939859396c96e28a7182a3dd1cb74335f76', '27c2144f39d7d5e0c4547801921f677e', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 212, '2026-11-18T07:03:00.813522258Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xbf9b74c04784430034b937e919d890a1f4553e50e1cb54163d44ad30b5d2822d', '\x013cb65cc1c67cb03bdede96a4eb2b109a7bfe2102558ae6eb169298b3e8d5c3c2fa18261cdcc601bfd86d3acb448a74c031a7c31438e742e865597c1f07b0e88e', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 214, '2026-11-18T07:03:00.817869394Z', '6cf9e079f3dbd8f0', 'legal_read', '\xbf9b74c04784430034b937e919d890a1f4553e50e1cb54163d44ad30b5d2822d', '\x019f4f9b7cf020392cf7b3aca45844b819037acb4d880789dc0e3c890568eed6a443b73ee06777e2fb58043607a894ab1c95f34c2ed8bec737c9f53a9d84d97bde', 'b908addcbd423da50d15a55770969160', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 216, '2026-11-18T07:03:00.821219162Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x8352f98c5389921b12a12e0b328dfa85212a3bb44ff76a74207a60cfcede2531', '\x019f37d3f052249fee27233207a65b83740d8cfcce818b84fbf4c3859babf1c79030dcf3c967232ca3671f29a4779b6508feac12bd95595dc9f4907ef6e0269082', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 218, '2026-11-18T07:03:00.82549402Z', '6cf9e079f3dbd8f0', 'legal_read', '\x8352f98c5389921b12a12e0b328dfa85212a3bb44ff76a74207a60cfcede2531', '\x01c74e246918b1aaf63d5c65592a7369ac51ab4d7a4b7f1bce0455ccdd7abb03b0a60e5a4a30f8ffd92513b84a1a722836f27347f5ef6f887116f13241f78482de', '7898325204832c41ba5c6fc2b44cf9f1', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 220, '2026-11-18T07:03:00.828780909Z', '6cf9e079f3dbd8f0', 'legal_versions', '\xba9a3360130c4cd9a3b94eecba1960a0a79306705d2945751b30fd92cc9f6828', '\x01a0af06332ea28500aaa01d3b9fe92104dfda0ff400ff0d210f6bd9cc6e8ca82b9b89a75a57a22da3e90e1c460383d3f6e4ef3d93416f746e6176ea226ef61d5b', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 222, '2026-11-18T07:03:00.834521643Z', '6cf9e079f3dbd8f0', 'legal_read', '\xba9a3360130c4cd9a3b94eecba1960a0a79306705d2945751b30fd92cc9f6828', '\x011bef641147bc62e23ebf561dcf32869ecbe26058731ea97c4a63914eb5ab4d42e6755c6461389e05e320fa0d6912591542e7d518270466a6e9bd68832f411820', '8459150a9349d6da2c3d2e8493bd700e', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 224, '2026-11-18T07:03:00.837940957Z', '6cf9e079f3dbd8f0', 'legal_versions', '\x27786d31d7f47b19d6f696934223619f548f1f88bd169544c6cfb346994c6113', '\x01aa800604c42d426949ae6b87058f1491769cf7e9ce737f081f9f98fce0e0ad40a69f35f49678cf499704806f2d1e01551801e5cf078223b55e9a8ced49de0ba5', NULL, 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 226, '2026-11-18T07:03:00.842499046Z', '6cf9e079f3dbd8f0', 'legal_read', '\x27786d31d7f47b19d6f696934223619f548f1f88bd169544c6cfb346994c6113', '\x01875f036cce3be4b09df536572940bc3cb6e462a8fce4f10109d54a146e29a8dc7282c049c81a706d09e151c09485208a482a9b1ee5f64b62c2b4e659c49d98dc', '47c07cbf1e504fcbabe77d113524ba9d', 200, 1, false, '\xe5369867d757fbc2');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 228, '2026-11-18T07:03:00.845842592Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x62df4eb9a422f26831650da5f77c889b07e13d68783dc63c3b4595c8108e28e4', '\x01e40afcf73b07100c511db4e1c4caa78ec45ec142a7ce36c56feb88e177e40d790428f07dd41eed4253eb0b57ffc49a1920c1b3a881a9ac65234a46549fbd7686', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 230, '2026-11-18T07:03:00.851159706Z', 'b5bf7ceaccb173b4', 'legal_read', '\x62df4eb9a422f26831650da5f77c889b07e13d68783dc63c3b4595c8108e28e4', '\x014c53c00daffb763af71954c831c1bdc845b73a4c0b6ae8b5120b8fcdcb97ded13044d7c0412af7a612531dfa706f6e01f351a71a5b972dbb1795c84555603734', 'a9ce7652a6173081d7d7f58c1801e256', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 232, '2026-11-18T07:03:00.854239383Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x40c1595f467625d6e6bf5f569e2c47b25dcc9365a985ce329801976841d84c09', '\x01cb17e8cdad4dcee078e4e1a4e096f08c12a8c3a323a53f9cfea88bed3c1ceb4306592fe0533767550f400dcd48383268c86d03ab3a4678af0f0087e7662782e4', NULL, 410, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 234, '2026-11-18T07:03:00.857561669Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x62812e843e2c32791b9e1b85ad9a17181e56b4f4d4ff8eac73816c5047f3d019', '\x016893280e93f228e0b5714539a2a66b0895f1e504f2f28b8c953b6a5efab34f9f67521dc8d9573eb68f5aad80963dfc737df4f3d7b3ddfd26ce1747dc20440b35', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 236, '2026-11-18T07:03:00.861998046Z', 'b5bf7ceaccb173b4', 'legal_read', '\x62812e843e2c32791b9e1b85ad9a17181e56b4f4d4ff8eac73816c5047f3d019', '\x0162315d19d1310f3ef51a8289c024a578967e96e18558cb8c5868c8d272998219807e9f6cbfcfa9c196ccd14e481d5d6a616f08539ec06f24444f104b5a9d8da5', '864e46cb70a60f1b7ed9039bd0ae2783', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 238, '2026-11-18T07:03:00.865263092Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x0172c50af4fc270149833ebf78430318fcdf969a4383362565b96f5b33f4952468a9fa5b5846110d4ed1544ac0652f62b75045ee7e1925e4d574168afa50792bd5', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 240, '2026-11-18T07:03:00.869404414Z', 'b5bf7ceaccb173b4', 'legal_read', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x01a193e260afa559e4f5fd07e5aa1ac90616b14fc2326beb09479a122bdf09ee25f9273e316123678808c477005acb0bd8f2f28944a01385990f6c8b4143fef326', '3be0c6ccfdf0b893d76f61f207bab69e', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 242, '2026-11-18T07:03:00.873662114Z', 'b5bf7ceaccb173b4', 'legal_read', '\x8fd055b11b0cd32cbe3667af5a6fb9103c540d24467097a131db5f6b31022ed1', '\x011507e888a5a007e85778393885c0eaa5c025e3c6ffdc1a1c36dbb5b1f8b42c3cd88894a85eb9c7f652bae35f666645b09fdd29eba1d533866bc4397e3407c55b', '324a32c23c1528ba64d3f17fa920609d', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 244, '2026-11-18T07:03:00.877111799Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x2e213751fd4a090f30fa7442d596e84213a555682bb31f6b852bfbdce6a1f390', '\x0129ed67fa55dae378b080c83ff21bbd8488ecc2ddf3b727721fd4fda50470eb0ac43820c7a869ec7947f5769814d8e77a7e190ebf154e32957411ea7bec447c80', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 246, '2026-11-18T07:03:00.881316Z', 'b5bf7ceaccb173b4', 'legal_read', '\x2e213751fd4a090f30fa7442d596e84213a555682bb31f6b852bfbdce6a1f390', '\x013d5ccd63609921ea46366a653479ec0bb0136cec6861f03dbb2eb4785b96eb2629b21bb6ba1e3ab68f8bbd3dec101fca561376a6981c4ef18a6e4defa89692c4', '07f2cd753b91774b7cfc885e699888f2', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 248, '2026-11-18T07:03:00.884547463Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x3b3e223d01e3c37880301d88cc4f01ab9b6d936e4de29a48347cfdb983fa1536', '\x017070c69761b9be71eff42fd4b43f5b7dcdc2028865bb943fbf4aea709d1ad09d62054d1f9484e455b3f58b8881274e8c14cb99ab6de98098cbeb34fcee3a0a17', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 250, '2026-11-18T07:03:00.888826849Z', 'b5bf7ceaccb173b4', 'legal_read', '\x3b3e223d01e3c37880301d88cc4f01ab9b6d936e4de29a48347cfdb983fa1536', '\x01927ac23570e3a70e547b43b018c33559efe81bfd7537e87ead0dcb4c29e6ed74b832b116148f0428fbf3d8f5449770e23571665e3a66efef4113c4159d3d5152', '9280879ddd11e10eaeea230130a1c6d3', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 252, '2026-11-18T07:03:00.89206571Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x8cf136930a75852f74f7f18cd26fcda1e5ee7fda7a7439ac1c068ff9f5341bc2', '\x01f13a1c289b85e351ac5a1bcb7ccd9503a234805702aa61af455b666e7315f1f265d847db669bfefbed6564d61a39d045741c8d082bc5d0386dca97b45dbe6cf6', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 254, '2026-11-18T07:03:00.89637204Z', 'b5bf7ceaccb173b4', 'legal_read', '\x8cf136930a75852f74f7f18cd26fcda1e5ee7fda7a7439ac1c068ff9f5341bc2', '\x013ddeb591ae129b3214a44edb2d5c2241dd3bf4d97a8757be743104aade787d526621a08ca9aa4cc12f45a9d417a4295902f5f6709c2f8be40bedd311c7d16f55', 'aa03a9c9b37afbea1c89346a782afca2', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 256, '2026-11-18T07:03:00.899586466Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x30f594d5abd234877ec335b460986f1e1a8caa1b5993cf32f5e2b67cea2543e2', '\x01a8619673960774e5248c005b0897b67615047c53eeea49d75bf69a4f4af2fed6ddea8f45c22d2480f9c4c04054700b37cdf92b3612e874a2cf7c6dec3eb0ffb8', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 258, '2026-11-18T07:03:00.903761908Z', 'b5bf7ceaccb173b4', 'legal_read', '\x30f594d5abd234877ec335b460986f1e1a8caa1b5993cf32f5e2b67cea2543e2', '\x0112959e914fb72268da89dfeca006a3d6e9c2b4f5c2c82b9cc2072fa2d470c6d3536b59c4dce7733716a09201ac19e3e50fc5bb117056eb2eecd413bdcaa81f61', 'd821b68af704a24dace086e1c9331707', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 260, '2026-11-18T07:03:00.907186176Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x8855d493aedb47369b853d4b6eba16e20fb659a985ae9d11ec9508e8271fadb9', '\x019d42855a5ad65bfc80c63255dd83deef37df2d1d64c32c5b1ae0ac90dba543436a6fc3b5eda4cdeb401d7dc13ac6797e6f85642bf2b52bf7a72ab4174f781dcb', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 262, '2026-11-18T07:03:00.911487108Z', 'b5bf7ceaccb173b4', 'legal_read', '\x8855d493aedb47369b853d4b6eba16e20fb659a985ae9d11ec9508e8271fadb9', '\x015a13daad1e15add3176f9b4e8a041712b190f31a51b8ee0abd5a6cb4a94428ea5a3749ca0e8ab2b23b1ccdc0369235e3c66e2a6f73eeb824b05c4785b80831d4', '3de02e6c4524df351f157f144027ec19', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 264, '2026-11-18T07:03:00.91471546Z', 'b5bf7ceaccb173b4', 'legal_versions', '\xe7e753097374b901cca55f598de1ca2bd282bfd22b65734ef6a97988d29ead5f', '\x01ec3b71d3a46793f809e6474a330432b9c86304ebe47c37cab463c92757198addad3dda2c33c067733b2f81c8b5aa226a1950f98bdadab66b0309172354076427', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 266, '2026-11-18T07:03:00.918985476Z', 'b5bf7ceaccb173b4', 'legal_read', '\xe7e753097374b901cca55f598de1ca2bd282bfd22b65734ef6a97988d29ead5f', '\x013440fca47dff0147fc403e8c967ec1d582eea82d403903d2dc94a7c3b7b1b478f74adc085394c95261a02c1d837ea915c98f2c8b3390b1770091a68277c2089b', 'd061d9d8afaf91e5dcfe49cb8f1a67a0', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 268, '2026-11-18T07:03:00.922204272Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x9787966ac8babf59ee775518327283bbcec8049d084ec94ec0a92a4441998a2b', '\x0131abb0bff8f29fe90c07213aa573e6311bb37b66d0f0ce870c08d24cdd5f9f18b65c131f15b439bb9461383c1cb0f855e5643620826ce2b9f9a69e75ca9ae898', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 270, '2026-11-18T07:03:00.926621185Z', 'b5bf7ceaccb173b4', 'legal_read', '\x9787966ac8babf59ee775518327283bbcec8049d084ec94ec0a92a4441998a2b', '\x0105a29653594f0abb83cc8407dcbd98db014921c6ba2cedc09a9df8ae6e9af701205337c65349b2e6480ba8f681012fc285c855b39b2b4f75361ccc297ffb8b52', 'fbb0a02aea001d0db954395057375c40', 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 272, '2026-11-18T07:03:00.929924806Z', 'b5bf7ceaccb173b4', 'legal_versions', '\x3d9f618502d3ad79d8b6b3272f14d2f312e3fc81f48ee894dc28967a209095c6', '\x0178fec2a8599d049ea917d9d12b70e705875cdc8392174d5d4a7151c246e864ac0e78f56f3f0945473075af01333de7bc4f12377f6c9f8efc45d888adc8a6d6c8', NULL, 200, 1, false, '\x3caabaac14424288');
INSERT INTO vcon.audit OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 274, '2026-11-18T07:03:00.93435045Z', 'b5bf7ceaccb173b4', 'legal_read', '\x3d9f618502d3ad79d8b6b3272f14d2f312e3fc81f48ee894dc28967a209095c6', '\x01d60261781e5a482975fb6c2610384496604b10d5f6c7ca7be70af651b104a37402bdfe7238666983d922ec499bbc469f54aac3e653f7b56b5f798d469060ba53', 'a9a2b96980f24f0460236329f3485b96', 200, 1, false, '\x3caabaac14424288');


--
-- Data for Name: audit_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.audit_key VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xe5369867d757fbc2', 1, '\x013d367ba9bb27f712946e431725982dc2baa3ac047f07409a5e4b999fe0fc7f6ef8d7ad284fddb7ec31879a880e74b250775c3c20ad8962fad7501a9c');
INSERT INTO vcon.audit_key VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x3caabaac14424288', 1, '\x01574eed807268ab334fe5b6e8d4bfee45c953e7e77fc9ed7183cd09cf0cecc10dd6a0cba2e695fe5ae0ef68fa5e1c660fe5152f454d26993cd81265c3');


--
-- Data for Name: key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.key_use VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x2396b3a1d7ac1412', 14);
INSERT INTO vcon.key_use VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x13747e4b4547674e', 14);


--
-- Data for Name: manifest; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', 2, 1794985380463513424);
INSERT INTO vcon.manifest OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', 4, 1794985380586550795);


--
-- Data for Name: master_key_use; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.master_key_use VALUES ('\xe316b3bcb25b76c0', 2, 1794985356559641395);


--
-- Data for Name: membership; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.membership VALUES ('\xf396dddf89c631bd71a9132e5827aaf3', '\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x011d1282c9a50554153f988d05db11c401c43ddc8911d9674076605c9d95a780a933f0', 1794985372433537503);
INSERT INTO vcon.membership VALUES ('\x4caa90a113decea6555fe0c3fb4e6219', '\x2187366c4fad45f64fa312b2cbc61222', '\x01ae7ff685842f91151f6ed06e70127a2b1cdd414cc200adceb125ede17ad6a73bae1990', 1794985374023781101);


--
-- Data for Name: parquet_file; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.parquet_file VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xb5545bd7fa6cec1f9a1caaebebd7281f', '2026-11-18', 12, 12, 7404, 1794985380331412665, NULL);
INSERT INTO vcon.parquet_file VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x20aa7c90df9922f506a209d078166e14', '2026-11-18', 10, 10, 6240, 1794985380480009738, NULL);


--
-- Data for Name: purge_pending; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: registry_key; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.registry_key VALUES (true, '\x3ca1139744f499da');


--
-- Data for Name: schema_version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.schema_version VALUES (1, '2026-11-18 07:02:34.881678+00');
INSERT INTO vcon.schema_version VALUES (2, '2026-11-18 07:02:34.924233+00');
INSERT INTO vcon.schema_version VALUES (3, '2026-11-18 07:02:34.933518+00');
INSERT INTO vcon.schema_version VALUES (4, '2026-11-18 07:02:34.934153+00');
INSERT INTO vcon.schema_version VALUES (5, '2026-11-18 07:02:34.936343+00');
INSERT INTO vcon.schema_version VALUES (6, '2026-11-18 07:02:34.940673+00');
INSERT INTO vcon.schema_version VALUES (7, '2026-11-18 07:02:34.941282+00');
INSERT INTO vcon.schema_version VALUES (8, '2026-11-18 07:02:34.945895+00');
INSERT INTO vcon.schema_version VALUES (9, '2026-11-18 07:02:34.950275+00');
INSERT INTO vcon.schema_version VALUES (10, '2026-11-18 07:02:34.951409+00');
INSERT INTO vcon.schema_version VALUES (11, '2026-11-18 07:02:34.952687+00');
INSERT INTO vcon.schema_version VALUES (12, '2026-11-18 07:02:34.956681+00');
INSERT INTO vcon.schema_version VALUES (13, '2026-11-18 07:02:34.963354+00');


--
-- Data for Name: session; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: storage_claim; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.storage_claim VALUES (true, '721ae11c43ecb1da73609755d09c7975');


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x408d8027782ba010053bb8210a3537f4c5274c6cc1c639c3431ae81e24195055', '\x01a86ed7ae4332ca1d9a9e7d3338524800766683984e1b93ccc5df402c212c349c8c55f989', NULL, NULL, 1794985356583006615, '\x2396b3a1d7ac1412', '\x01b87f0f6c3a931f256a161e6fb2e084d22735d6d2aaaa4a95f21f44907424a40631ac8c5ff2150de6c83c8ab29b73f0f9791c1b6cf1da92e118b8d7a0', NULL, 1794985356583006615, NULL);
INSERT INTO vcon.tenant VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xe9cb2a2c2404932e1c53ca5ebdf2cfd8b11137cd3ea93ba0ea3ad73f4cf2a967', '\x015faf31d5e2aa0543894e2c3b15b7e431482b4379b818078ebb7b1e01ecbd7e42caf82664', '\x01b24e9f3e7130da703767b83615d6eeee96d13f47e6d0ef56948d179a3935', NULL, 1794985362108809177, '\x13747e4b4547674e', '\x0137502a29c0be59bb737bf73a5103b646505f9d91f96f2b5c13cbd47cd1cfebc9d8efaee435e16119e8a9df1b1dd182a2b3877ae7e3ccea0230109663', NULL, 1794985362108809177, NULL);


--
-- Data for Name: tenant_catalog; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_catalog VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x7605e374c3348133');
INSERT INTO vcon.tenant_catalog VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x29b6d852f42aef0d');


--
-- Data for Name: tenant_lock; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tenant_lock VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db');
INSERT INTO vcon.tenant_lock VALUES ('\x2187366c4fad45f64fa312b2cbc61222');


--
-- Data for Name: tenant_setting; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--



--
-- Data for Name: tombstone; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.tombstone VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x5db2dc8fa508c20f8f16b4b381553016e631c3c878a0f4f1b18c716b67392de5', '\x019bc88a23daa0ee7033d22e3292a76436b50e1aeb43c0d12c6bb49e9407b09eb161d64c80639464813b6b988a', 1794985376227787675);


--
-- Data for Name: user_account; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.user_account VALUES ('\xf396dddf89c631bd71a9132e5827aaf3', '\x017f6ea0ea11505985ea862f5031abf14164e54b71d4ea33c16222a42a3d261878e1f5a8e3ea381f3c3e87', NULL, NULL, 1794985369315358462, NULL);
INSERT INTO vcon.user_account VALUES ('\x4caa90a113decea6555fe0c3fb4e6219', '\x018f79e7e718e00a71b03cee7fb7cd844f58d56bbe1a645337ea700c5776b7c34c9dfc4b0b5af2b65264dd4c', '\x14ad43c6c320bc8ab6b1a701010a4e8c7e03859237c35088d4a8ea8e21b6d74a', '\x0109bb4c93913f3198ed933638330f362499089d276e2103957c687f84b1b0e60c11cb864b3718aca64331f08a1caf29fbfabf269c9fb8edfc4f2bf86d4af45b', 1794985370858516712, NULL);


--
-- Data for Name: vcon; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x6c3088497dbeb1f00b88716d0c389cd1a0a9d6e35fe21741811a9588fb1f8c66', '\x012abd25eeeeb7025d78fb31a25275f2a52ca209a161aae29e726b69b1dac2e121da14c7e8c81ffed091351915', '\x9330150147a350b084894941b759272f', NULL, 0, 1794985376064623924);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x1eef51e1c8460afe1f1f098a54f632267a61e9995f8029cf87448c4049c35a99', '\x017b3f69f2fe39a011146e4c8431940176191f4cc168fff3a9655f724c6bf453a46c75c3627a24c9f1f63bd135', '\x27c2144f39d7d5e0c4547801921f677e', NULL, 0, 1794985376073335556);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x4cbdd19179ce16635005475404e38828bbd0a18610219da8d221faf59fc93c07', '\x01d2bbd60973a0aa9284b201b7fcb23ae7b32bbd65ee65e9d33ef74242526abedb8324c72a33892d0ce99c27ed', '\xb908addcbd423da50d15a55770969160', NULL, 0, 1794985376082834418);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x2f4298aba6e4eff19809159789f4c156da9196bbb5a91063f18e152b4fc61cfb', '\x01ee0e514377c4850eb66329ba114b6df98691d07223735c139f545f61730e1c24c4967d028297f81d0133d98a', '\x7898325204832c41ba5c6fc2b44cf9f1', NULL, 0, 1794985376091089071);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x0a5bde0b82ba5409d233e5e84a84b4c06abd2cd6d3663c72ad29ba7f306d0681', '\x017c2ba402faeb5c15b48d29e970b6fbd7015e1c8c106c1177e9e84422500e1baab9ac888faddcc25436e2e0a5', '\xee705042ce0d8a6dc9108e620b27ff8f', NULL, 0, 1794985376004635456);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xfee55b3f4c512fa9a11352474d5c2073cab7d1a44562dacd62d4be65c2827495', '\x01948aecc1ab004ca731a1063e20d1847c7c71dbfec9f4805372637c4df5f947c42a7a721bbd0cc2fb7e1d8e05', '\x76230137aac4ad5735f91eac7aca6c6c', NULL, 0, 1794985376015448102);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x2565a8b66cd2d9cf09491337dc0343c629d41c10a296b06ebc0bbb45aa488542', '\x01ec21d8875534fe546319003a02b9529ba3ab37f0da7456e074530dcc62b3280849edd06e516b753a72659dc9', NULL, 1794985376115050975, 0, 1794985376025325564);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x8d42b6d49732e2c80e485dbac02de1f98e874aef45b8c867efbdaa21cee8ad12', '\x01837244b10811cbdc4a3ee6e62571612d83b92926c157f55ffea008d5d77edee958f1f9aa418cb5a4f793e600', NULL, 1794985376121504318, 0, 1794985376033931789);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x6c6df4437e714b13714a01864803e4b10b58e205d2448d6320fba1829dad7403', '\x0108bcb97c8ec65b52c0686c108d5765bf9a2fdb14f29a4cbd1fbd36aec340b9f0773fd6a414e8735a60bde892', NULL, 1794985376130009274, 1, 1794985376045088618);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xd340b2da1fefad514c2287d1ef8de29675ce3f4c4a43a71beb573264f02539f9', '\x01299a29dfdd3e007c2cd1e9808b9984a72405a524ff2e70bc55748d68e3427de968c5f6d1b41d0e1ffab38958', '\xf4e54bd7748a6aec780162e9fecce55b', NULL, 1, 1794985376055861616);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x4619fe496d368c88ae16f4db79279f9b44784ef9521df3bc89554f98b45f8cde', '\x018b941eeb56cdd87b1142f9237001ec633e616bd54245f6af77f0d4bc08cba351edd62282d11642e617989360', '\x07f2cd753b91774b7cfc885e699888f2', NULL, 0, 1794985376174975519);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x69d5ed9d1594cd5b7e4c3e1d806d1efe033e56775edabae0a0829bc711d3b7d3', '\x0161b9fba0c6c8f9f327bc4f768e69dcecd7e1134e55605fbf884a5ebc94bbc9273a4fc45f8cad1b5120c1f556', '\x9280879ddd11e10eaeea230130a1c6d3', NULL, 0, 1794985376182575562);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x79218aceaace18f63a8ad20945bcd0b57e64866ce3dbd83db54c4e0ab9e827a5', '\x01936da25c67b0c630425078589c1c7976fe2aebd6c00a1c5d37df4f04440c203db197beac89ba51f57a8c4e44', '\xaa03a9c9b37afbea1c89346a782afca2', NULL, 0, 1794985376190359235);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x68873e35dcac0fdf26ac3f0b6ea8d9e827a3af67e1ca64a800ac7ef65af75031', '\x0168392d56d89777b159a7254a64617af367e0fc4a19413c9c8afd8fe76bce0d9a9cd9ce17ffb817baf9aec273', '\xd821b68af704a24dace086e1c9331707', NULL, 0, 1794985376198307323);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x3d17ccd71ad5b63595e04a1d01ac0e7349663249a5218d1c0d78c1470f3bf941', '\x011acec956c49498342c6c03c8d90455239cb209015334dd5ba1b667fb6695acbcc087dc55f423eb456dfc6a44', '\x3de02e6c4524df351f157f144027ec19', NULL, 0, 1794985376206089597);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x9b077f51fb435cacc459748f644158b3c41ac69456328b3c302333a49dc86664', '\x013fe1425cac19469c550607edd6c349d8a194b294ef7f52ed4b9a2ee19325b582d9c244ef651c26e1b60d8114', '\xd061d9d8afaf91e5dcfe49cb8f1a67a0', NULL, 0, 1794985376213733826);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xfc742a900fe0dc61f5a490ce8ba78d61e6fbdb7bd2baf84a3d88f101bdf0b448', '\x01b6e32d135eb55c6117d044e8b4b66363d2b421490b5b0af1ac11385f7992db32789d697723dc9f9fde387df3', '\xa9ce7652a6173081d7d7f58c1801e256', NULL, 0, 1794985376143357839);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xf846ac1652cc74bb24d70353b85a2752a2a595a91c48ab48d51f02a81a334ef4', '\x0141c6a1325ddffa365734f425b1e8b7c91b6d6168335137f8206c049d63ff1adaf9f98d65322817fd8d4f71cc', NULL, 1794985376249697067, 1, 1794985376159250748);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x4fbc4cddfbaddfe0059f2887531f96d1991532665d25c969624a6385ad2bec03', '\x01648be57b8944649e492af80675bc1df665710bf158333c225c873bfdae9d4aabd18647c64988a05bf190a93d', '\x324a32c23c1528ba64d3f17fa920609d', NULL, 1, 1794985376167156383);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xd65b037917db9ecf5e6ba1c0e9ed19f5ea47f77694e2933ebdfffbbef2ebc434', '\x0198a3493ffe7b351643e0938dc94c3a0a8c4ae8884ebd79d5abf3338019c8ab83473e99f0e63e88272ef0cebc', '\x8459150a9349d6da2c3d2e8493bd700e', NULL, 0, 1794985380703861109);
INSERT INTO vcon.vcon VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x296f44be6621345fbdf12e545fdbbcb3f74b3b3db67506ae2a462facbadcf96e', '\x012c07caacb6db0308bf2227021ba233538214a63a11d228f1140ac317256579accd2d31469b51f486518b2299', '\x47c07cbf1e504fcbabe77d113524ba9d', NULL, 0, 1794985380714261024);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xe4f9b35f07a1d0aad82bfe74ed88f13d6173c86b62afde9a861d09a9e6311c8b', '\x012acbece279e1828f815d7430b1d393ec464f4f2ce8c763afd36d85bc42802174f4a5867ef69424aab9b1bbae', '\xfbb0a02aea001d0db954395057375c40', NULL, 0, 1794985380723964422);
INSERT INTO vcon.vcon VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xe69385ba432d3954f7df61ae2e076ff4a5bd92d8caea1bd9a64682775d3b0b95', '\x018c6fb32bbccd03c6fc5f3dc8dcd95248dc9519c1d2318a7b2f0a857aa47ae131c78d9366bc72aa1819a3a9a5', '\xa9a2b96980f24f0460236329f3485b96', NULL, 0, 1794985380733022553);


--
-- Data for Name: version; Type: TABLE DATA; Schema: vcon; Owner: vcon_owner
--

INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x864e46cb70a60f1b7ed9039bd0ae2783', '\xf846ac1652cc74bb24d70353b85a2752a2a595a91c48ab48d51f02a81a334ef4', '\x01e0a4bf6d461a25b0aa10e8bc8a9baa4abbec176406aca6044925e07b478dff5921d55d95', '\x013261936c5c3dea0e8649c9a2d1cfcd15b96213e485ba7388c4804ce127', '\x01d36612627b4e74f8df6ef43b2a039020e6e4f11356543c9e6016d2833b107b7b66a41c8b', 1794985376159250748, 1794985376249697067, '\x13747e4b4547674e', '\x011f67fea82684a447c08ad764605ba8a465306be8ee30dbca9189b8e857db95b4e614df9c39801600b6a1aed0dd433ef3b333c59c4c1ecb941a834a01', '\x20aa7c90df9922f506a209d078166e14', 0, 0, 30);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x3be0c6ccfdf0b893d76f61f207bab69e', '\x4fbc4cddfbaddfe0059f2887531f96d1991532665d25c969624a6385ad2bec03', '\x015f44d28dee671b135bcc5e698524803d1ce9addf362afc9b8fb8bd2932695b775ba634eb', '\x0149bb1d8f1a805cca49b3a40b2016e697e251efa94fc5246b8da4d6a939', '\x01d0743e0c442d03e1773d325e71d855c9ffe5e9d4b3493f560fc7ca9604b2dae7678ca7cb', 1794985376167156383, 1794985376263646779, '\x13747e4b4547674e', '\x01cc53879137f35c8a7a634f4d74d8287b40998fb6094e01ddba4579f211051349a8df4c4cbbb5f115abc534aea9ff8308060536d9ae533d2eaa85bc60', '\x20aa7c90df9922f506a209d078166e14', 0, 1, 32);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x07f2cd753b91774b7cfc885e699888f2', '\x4619fe496d368c88ae16f4db79279f9b44784ef9521df3bc89554f98b45f8cde', '\x010651d9bbc45a3257289f7f0fa460a484c644fd25c2386ba0cb211df309225109fff4273f', '\x01681a45ca55c32f93c1ae616be6fe4d766fb0af46ca85eee06619d7e24b', '\x012a5ea9ad8066f5d2c5a0a35a48b2b361dece713e92ec13f5460b532659ec3db3eed5b3b2', 1794985376174975519, NULL, '\x13747e4b4547674e', '\x019d16e348759c720bd81790aab52fe96d595ac81b9310249d4c4d122957d88fe36280f4a9f05cadb44a6b83b8cebc2ee835f5e0e5e6738fcb1cd99c8c', '\x20aa7c90df9922f506a209d078166e14', 0, 2, 34);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x9280879ddd11e10eaeea230130a1c6d3', '\x69d5ed9d1594cd5b7e4c3e1d806d1efe033e56775edabae0a0829bc711d3b7d3', '\x011ca579703480a471d84291369d78aa4528e7bf745d9971ba445e261c78dd0e57f01bd3e4', '\x01e6f99f97a785411a518ea46bfad1b1d9fe4f0b2384daf093f3fd95f1d1', '\x0110320582f2aa803cbffd7d09b22f259fc6b7de84cbcf2f8b796f775f03a86fd246942212', 1794985376182575562, NULL, '\x13747e4b4547674e', '\x0134047b2f6bd3b518f834fa787621625b4d2806909611e3601b07206261ec2d69b136e1de4f4ef86218088aedc16e64c795ddf71ee20631c1ca68e29d', '\x20aa7c90df9922f506a209d078166e14', 0, 3, 36);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xaa03a9c9b37afbea1c89346a782afca2', '\x79218aceaace18f63a8ad20945bcd0b57e64866ce3dbd83db54c4e0ab9e827a5', '\x012a67534277a47cc9734e2990f4c71389d44377976469fdd5af7ca1fef0bd6dfc901f6656', '\x01858b33df5b1b5e39a2c1d716a978ed18e4d9a032d004c5883187073ad3', '\x01416944ce796e130177d93f60e72c2baa2f750a390ce005cd01d976ab06934d5dbdd112d8', 1794985376190359235, NULL, '\x13747e4b4547674e', '\x0187fb30d6e212c8471795d0286d34f9e94e56631586a52757a2deb938f80d12e002a605bd1788762b52560fe27a49f4fe509d0d8caa17b7aa1acff0bd', '\x20aa7c90df9922f506a209d078166e14', 0, 4, 38);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x018947c65a48ba581c78c683ecaf94bb', '\xfee55b3f4c512fa9a11352474d5c2073cab7d1a44562dacd62d4be65c2827495', '\x017a30539be28892b4aa3e82879edccf46abf850ab0ee598605037b267173517cdedfbb364', '\x01f8dbe13f7230847aded322604270e54dcf9dc596ad53b24ca33d2ba0ed', '\x01e6fd8e2e0f1e0f7bdf1660dfad019d40f12f84eff03ad6ecccf4730130ca722a4395cf08', 1794985376015448102, 1794985376110813441, '\x2396b3a1d7ac1412', '\x0195e94247735248f9658463b5e12b30d7880f963228739efc0a6ebaf317d5613f92aa30ef72af6db17d139486ca2bcdcfc0d0669089d582ab7266ceb4', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 1, 4);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x18afecd8a57dcf5dd67532372e060514', '\x0a5bde0b82ba5409d233e5e84a84b4c06abd2cd6d3663c72ad29ba7f306d0681', '\x010faa9b36d3b04f6a60dc5898fe203f83f1eeb9f215f5d60925996632e578f0104eec72e5', '\x01ae111f90969bac9c1013b62648f7ff83ce7d7136dd6e2ac190d7263958', '\x01785268eebeef7ccd62e7f811faed2c184a1fb562e23e68a318674be205837ab64b8fa8b9', 1794985376004635456, 1794985376100860126, '\x2396b3a1d7ac1412', '\x0155d6f1aac721e0a9d0c0401fd342a5f542161d5e0611c5b1d4b5e898b3b624a8290524994fd5279a72143b9d07b2b10cb3978affff65f8184d2a78bd', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 0, 2);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x4dc747157eeb6ec67a97905902833387', '\x2565a8b66cd2d9cf09491337dc0343c629d41c10a296b06ebc0bbb45aa488542', '\x01a5d1fb71b79fb21a1bfd441546cfdcdce2499a90df6c900bdad38b51b90db216dd5615d5', '\x0107bd07174dd427a0b20477bfbde52a4dff3cb686a17a0f7961ff19d14b', '\x01aab137e7f197bf4a43b6db073baa9259bc7101d011928e7f8665e683bdf6065148713a6b', 1794985376025325564, 1794985376115050975, '\x2396b3a1d7ac1412', '\x0109e2d7f1773393dedb565f0921907ce917f746e671994ca2cf9d431a0dc0408dbe35f138fd6200a0369ab9694dffe78728ea36753146a5ae51b8b457', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 2, 6);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xecc80bdd5287e3aa7a0e2fb25166e2ff', '\x8d42b6d49732e2c80e485dbac02de1f98e874aef45b8c867efbdaa21cee8ad12', '\x011785161642476764802eb19fc684139b1eca0a0a100b22c9dfd0aa7bcbb2888885f73829', '\x01d6c509b46c96fdb538ce9ec7beec73904df6af3178cdde87b4814bf326', '\x011c1a71abc3872d473194a4adffd5886a6edd0aad3f02b2dc99649075894b3fa37ff1b829', 1794985376033931789, 1794985376121504318, '\x2396b3a1d7ac1412', '\x01a9099d2ed59d5aeb996a93ad9a23cae4d65af396275949e01bc66589bf7560a5339331e516cc85274573d6a6f067dc3d235e95605e78ef867ce75976', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 3, 8);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xd821b68af704a24dace086e1c9331707', '\x68873e35dcac0fdf26ac3f0b6ea8d9e827a3af67e1ca64a800ac7ef65af75031', '\x01b2af0c88cc7cd63105859ea13599b6c80326833e5834438ffff39ab78094a429b90cc49f', '\x01a08252dd36cd71315c54541c927b82a2057cf4e7e3053bd247f01cbf06', '\x019e4b33ff3611ff0ad94b51a1b8f6f08f1eee4d819fbcfe7e90a83d187fb9ffd343a7727d', 1794985376198307323, NULL, '\x13747e4b4547674e', '\x01ce85c5b449677f434c66296bb5429b1265bf8b0a6727b840e41789f156528ac9cb0b94ab1a81f4df7a1d6f294c55b7978b12ba9fb0a53bf9f4bf96cc', '\x20aa7c90df9922f506a209d078166e14', 0, 5, 40);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x3de02e6c4524df351f157f144027ec19', '\x3d17ccd71ad5b63595e04a1d01ac0e7349663249a5218d1c0d78c1470f3bf941', '\x011db11e64997ebd51b9718f5bc64b5ca5c3e0b7fbabf581a6ca38a2865e87cb1c68c97e18', '\x0127eb27c60937ee8b730ff3a7754199203aca20ec3ba66cac2f46e4fe00', '\x01af3ae8ecef3cdc0fdb9198f2235ae4b18572195ff6ac0d509cdfe7fa4c92a7d5e8529015', 1794985376206089597, NULL, '\x13747e4b4547674e', '\x01d9c86f56cfe31bde3ac246570c74a1bc089cb1cea04b3862151ad8b3928271f0390f73a5c0595149b2815f3741a3a62bd922332269ad9a4d1d916b84', '\x20aa7c90df9922f506a209d078166e14', 0, 6, 42);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xd061d9d8afaf91e5dcfe49cb8f1a67a0', '\x9b077f51fb435cacc459748f644158b3c41ac69456328b3c302333a49dc86664', '\x0148d2c583e29a5eec995d06087de94aa804f7a1a46fc1a4e9f4478007b93ad0c8b243efb4', '\x016e84f2b7b0d14550b640306a0b4f200d5ea14c2f86e6271bbd9250ca1f', '\x018241d3490a4a4b4d6f5c92c391fe65bd50b394723568a1ba6cb62ad8d4362df44d6e1410', 1794985376213733826, NULL, '\x13747e4b4547674e', '\x018c1181f1aaea13b135f30a73b1161568c6544034cd65f7441585696ffd736f8ba55bee17b7d6826917d2b834c6459a46798841b0bacc32949f15361d', '\x20aa7c90df9922f506a209d078166e14', 0, 7, 44);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xa9ce7652a6173081d7d7f58c1801e256', '\xfc742a900fe0dc61f5a490ce8ba78d61e6fbdb7bd2baf84a3d88f101bdf0b448', '\x01c29a0f30d981577c113ddb31cbee6e6325ff9c693c4faa6a1e26a0840a3a0f85f0a30b53', '\x0169380f5ba17f101753a5bdd6b49bbb6a241eba823c4fdc46433a99137e', '\x01eff17ad15251499a89c73727a835e01f3fe3cbebf86e3a1283bc2d4bb1487da4c02be4c0', 1794985376222405626, NULL, '\x13747e4b4547674e', '\x0188f0d02d6f4f365dfdcee639a7083d049143a0f23f44d6805bc168c7a6cb5117e2e8b81435d03d973c768fa9700a80288c3044485dfbf969c680e664', '\x20aa7c90df9922f506a209d078166e14', 0, 8, 46);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\x324a32c23c1528ba64d3f17fa920609d', '\x4fbc4cddfbaddfe0059f2887531f96d1991532665d25c969624a6385ad2bec03', '\x014e606350ffc47cfe8051c2a481de4302d94735080f3fe742300cf8f3e070611dd38b4931', '\x015843e8f936103c3223a4f91f3bbdf2e4dd22a05a84ee248c6b87ffbdf1', '\x01f50516491900709fb6758567a14e9062f98ae8fe9db693c8c6e5f636c0a646342c0e153e', 1794985376263646779, NULL, '\x13747e4b4547674e', '\x01006c7bee99bc0e5287fed8dd2a330f5ea288a9c9a210141bb3ab4644c12bb5144798c0e59a0de4e9b37d837523d9670ce0a8d8e8e0dffb7803c7e981', '\x20aa7c90df9922f506a209d078166e14', 0, 9, 48);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x2ead207d8f702865da171d08b8ec1808', '\x6c6df4437e714b13714a01864803e4b10b58e205d2448d6320fba1829dad7403', '\x013a584f27bd25363c729046068f382598694cb4621ac2c989c1be5766216cc6ccc6ab9205', '\x011819266f0be0a661250d231bf3dc0423c680825c19f15db568bd62a33e', '\x018582a71145b9a7e3a7a4be6813ba6030da3d4ef19dd89bcb08aa25d8d77d12e7d7de8f4b', 1794985376045088618, 1794985376130009274, '\x2396b3a1d7ac1412', '\x01e42d0a7c5aee74481dbcfc9b5a25f4ee6bc0794fb9c87e36879f19a2af850a1cb82f32f3c3cdd044b1eaed5c77ac1ecc148bd0bfc9a9359134d69d57', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 4, 10);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xf4e54bd7748a6aec780162e9fecce55b', '\xd340b2da1fefad514c2287d1ef8de29675ce3f4c4a43a71beb573264f02539f9', '\x01bd8194778b46c05ea2713230316489cf8a75e30f2a9c136e3b5d21d2615b06b4459ae034', '\x01fddc7a8353334310b7beef9eacf3488215cafb0141043dfdd789fbd14a', '\x014192f9695e0547f5308f22ae3385932b6d58c3694e0be453fdf3486b1bdf578343a96caf', 1794985376055861616, NULL, '\x2396b3a1d7ac1412', '\x01165e3e2a14849c6aaa345ba27b8d1ca785858838e59037624ed9c6e6b6065bbe55b6dfdef78cc7aa0a2db439a245a9b2fec4a44c3dc16e2c8f63258a', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 5, 12);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x9330150147a350b084894941b759272f', '\x6c3088497dbeb1f00b88716d0c389cd1a0a9d6e35fe21741811a9588fb1f8c66', '\x0170a4bc65e860b952e701a0ab49ac78449acdb2acb8239ccd2067afc31d811193cc223095', '\x01ba0c5ab28073e189bf5812717304f8869a15c864ddc467f63eb0fe3838', '\x014fc43c74dd998b78336d38597ac11bbc6b306ce32c79b8183b628a62e55d81a1484843ff', 1794985376064623924, NULL, '\x2396b3a1d7ac1412', '\x012e2090c8145cfdeafedf77ed51060cac6751d8cdb254c26b1ce3e679fcad2685feed53c36e10113ec7619b30a36f078f437e2bea2548560ce714b4cf', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 6, 14);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x27c2144f39d7d5e0c4547801921f677e', '\x1eef51e1c8460afe1f1f098a54f632267a61e9995f8029cf87448c4049c35a99', '\x018feadb8371480cc45181d09a9418e9c2c7ee22be326ad934eced9a0be70b3ac4df025170', '\x0152bce9d8c9d8bfd39ca5845f31a84cc1d300a6207eb93e105a0d1d7a5a', '\x014f530ca625b4f35021c9ad90f6341e9f393520fd70498a2435e3eeb1175062616da41b1a', 1794985376073335556, NULL, '\x2396b3a1d7ac1412', '\x017e14bbcd7d861385ebbc8dfe94ed55e055adaf08b8bdadc63e4bd5626eede6071e235acc5a08497333eb6450de00d6df2338bc696603653ed8da7e2a', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 7, 16);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xb908addcbd423da50d15a55770969160', '\x4cbdd19179ce16635005475404e38828bbd0a18610219da8d221faf59fc93c07', '\x01196d49a651419072afdcb43d4349e87234b935c306cef3e0048326e96598d887ae2a812a', '\x015ead76834e19987313a80d0f1cc8b1de322558eaeee52dc7cae0a744a1', '\x01f363593f16cf6c228247310e3ea91f0b6e366bbf70a5a3cf3ee75d16e94d630af76d7276', 1794985376082834418, NULL, '\x2396b3a1d7ac1412', '\x01119fcc8387f06ab41c1e07741cb1a4813f4ee70060276165bdc29f89e75380b49295f62e36c71da31fbde823076d3c46dcfe0d76c343d3c8a180d3ae', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 8, 18);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x7898325204832c41ba5c6fc2b44cf9f1', '\x2f4298aba6e4eff19809159789f4c156da9196bbb5a91063f18e152b4fc61cfb', '\x01eb103b952b9aeb86d523c0eaa02f620d139ef791e47a8562d05bc97cac18a411e1e133d9', '\x01755bbdb49a2af09ca6ffcacacb7ab71ca522947f5a5560eda189d40925', '\x010701dafd6147c7726b92b70d06b39ab1b36008a29c7db70f804ddca857744341305646da', 1794985376091089071, NULL, '\x2396b3a1d7ac1412', '\x01e20e0b2bbbe7679fc0f3723cbc466c1d51dcfaba99e46478fa5f041bee2ef3e2921e5efdec35b5fabeb7e52309b786ef897107d2ca303dcecbf915fe', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 9, 20);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\xee705042ce0d8a6dc9108e620b27ff8f', '\x0a5bde0b82ba5409d233e5e84a84b4c06abd2cd6d3663c72ad29ba7f306d0681', '\x01845c0272481e579bbb939d3e9d07fe0b0877855e8906038122338eca79a08b99cbeafed3', '\x015215a1f557cf27258ae6aa5de2fe46d2188ebc8a7016e864f01ecda4ad', '\x01d101917bbf71430b91247b86d2564c682643c7790add4ab1e3ee47e9c9588c082115d146', 1794985376100860126, NULL, '\x2396b3a1d7ac1412', '\x01cf25e6c53a5d126b7deb3e280e3749ea5c8d663672aee3ccc441fc3485ca8cc60bc1b3e31442863cc6dbfc6071eb553fa393a47b1b0cdc2469187388', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 10, 22);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x76230137aac4ad5735f91eac7aca6c6c', '\xfee55b3f4c512fa9a11352474d5c2073cab7d1a44562dacd62d4be65c2827495', '\x01cbfc3b3b1721b9fb1bdca54ac3698c77ea1e93e9e4cbbb243a5c5638f66e139d6489d1ef', '\x012d7dc48a720efff31dfe2ddd8ad3e959c2b505cf88196f7572553ab2d9', '\x011fcb036a934fd4b13a8d033a65d287cd2ae2b52aac9ea6754cdd7a287426ae93c227a675', 1794985376110813441, NULL, '\x2396b3a1d7ac1412', '\x01f91b8f1bd69bc1ee03d2f73e2675e7777690de9ae977d662b14925050d281e3cd7bc6440032d5affedac9bf0cb9a9afce78da66e22aea79b3b719d0d', '\xb5545bd7fa6cec1f9a1caaebebd7281f', 0, 11, 24);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x8459150a9349d6da2c3d2e8493bd700e', '\xd65b037917db9ecf5e6ba1c0e9ed19f5ea47f77694e2933ebdfffbbef2ebc434', '\x01a9bdeb3ceeb67921660aa0ecde20091c0589cc65e058e88c168a693a2b607f7a5f2eac9c', '\x0138f8ab91aa59271f3b892746c78a9c47cc71376fd80f06ab8e0896489f', '\x0177da78f22cc72bb5ea8be1c4274e0537de1cd69f0e43156664c45b92a5fee2971b1e8578', 1794985380703861109, NULL, '\x2396b3a1d7ac1412', '\x011a42c20652978d18006676daef430089e8a1d0200f080ac81a12eba95d2c6136fd91602e9824bb1411c9c7b561ef8f72b1dac7a78821410e70376beb', NULL, NULL, NULL, 50);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\xb6b8e37320fabe32f55a4c8a3b9ea5db', '\x47c07cbf1e504fcbabe77d113524ba9d', '\x296f44be6621345fbdf12e545fdbbcb3f74b3b3db67506ae2a462facbadcf96e', '\x01b39a1199e0d59aa62fd653b0d5ebb1e2d5faf704def0317953af1abb8dfa78804b9cf091', '\x01742a1e110687c1e0b444152fecf9187c1229df32861aadda508c98a19f', '\x01e179d4fe34a0c3cc9b6062bd23ddbfff424fde09084acdd2c8498c3332ee0c63fa0e6ac1', 1794985380714261024, NULL, '\x2396b3a1d7ac1412', '\x013fc3627ee6070775dfee69c6f7405d31815dd041c3e6419fe4af900839c748f856bdf450fdbcf31d1120b998aca717d2eab867df2a0ddc1146ecc4ba', NULL, NULL, NULL, 52);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xfbb0a02aea001d0db954395057375c40', '\xe4f9b35f07a1d0aad82bfe74ed88f13d6173c86b62afde9a861d09a9e6311c8b', '\x0191aeba3c934b11bd21f0d3aa6664a7aad372b4ba5e45176e3d6ded894514a53126f55914', '\x013f981623fd297d5e2e4a6cd3f476746d8d5fe08b5d28bdf307d0df9f26', '\x016ed045fe2e5e6bd5e935c59ee423dd2a74095e60f89952494c05984199750e0ac01d8bf3', 1794985380723964422, NULL, '\x13747e4b4547674e', '\x01a794c5b786b11eb88f621c245597b8da3a43294bdf460355ce9e6bb5634ee5190c90e4934a2461a20b0f15a38ac3153251a122e797fa679c46590eff', NULL, NULL, NULL, 54);
INSERT INTO vcon.version OVERRIDING SYSTEM VALUE VALUES ('\x2187366c4fad45f64fa312b2cbc61222', '\xa9a2b96980f24f0460236329f3485b96', '\xe69385ba432d3954f7df61ae2e076ff4a5bd92d8caea1bd9a64682775d3b0b95', '\x01c446c4d3ad8a94a9ae8245231f132335e90cee6c0deb59e82b6e7c014160aa5fcda6ae0b', '\x01799b321936ef255b7347bd7e06eef4d2ed79c845dc45918327cc682d92', '\x01b32e259ced4f4db6307f0724fb35282998667f9c9faec7a6a231d1462f00ce67c71fa381', 1794985380733022553, NULL, '\x13747e4b4547674e', '\x01ab3e5482cbbf4baf58ab4da02f63932f93b80772e9dfb4b16781e8741adbf24ec611199fff74a2d6189d9e082345ffb68d6c3e911b067367d06572aa', NULL, NULL, NULL, 56);


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


