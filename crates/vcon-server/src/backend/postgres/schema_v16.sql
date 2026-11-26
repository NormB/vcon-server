-- Schema version 16: a tenant's row names the key-encryption key a key
-- backend wrapped its tenant key under, and the registry's key row holds
-- the registry root a key backend wrapped.
--
-- Until this version every tenant key was wrapped under the master key
-- file, and the registry's keys were derived from that key. From this
-- version a tenant key may instead be wrapped under a key-encryption key
-- of its own, which a key backend (a KMS or an HSM) wraps: wrap_ref names
-- the backend's key (its kind and reference, as `kind:key`) and
-- wrapped_kek holds the `key-wrap` blob, format 1. pending_wrap_ref and
-- pending_wrapped_kek are the same for the pending key of a rotation. A
-- CHECK keeps each pair whole: both NULL, the key under the master key
-- file, or both set.
--
-- wrapped_root holds the `key-wrap` blob of the registry root, from which
-- the registry's keys are derived when a backend holds it, and
-- pending_wrapped_root the root a move to another backend makes before it
-- is complete. The application role may write those two columns of the
-- key row; its grant on the row's other column is as version 12 made it.
--
-- Every row there is keeps NULL in each new column: its keys are under the
-- master key file, as every release wrote them, so no row is rewritten and
-- each CHECK holds for every row. A blob is not secret without its backend,
-- so the columns are in the clear; a blob moved to another tenant's row
-- fails to open, because the tenant key under it is wrapped with the
-- tenant's id as its associated data.
ALTER TABLE vcon.tenant
    ADD COLUMN wrap_ref text,
    ADD COLUMN wrapped_kek bytea,
    ADD COLUMN pending_wrap_ref text,
    ADD COLUMN pending_wrapped_kek bytea,
    ADD CONSTRAINT kek_whole CHECK ((wrap_ref IS NULL) = (wrapped_kek IS NULL)),
    ADD CONSTRAINT pending_kek_whole CHECK ((pending_wrap_ref IS NULL) = (pending_wrapped_kek IS NULL));
ALTER TABLE vcon.registry_key
    ADD COLUMN wrapped_root bytea,
    ADD COLUMN pending_wrapped_root bytea;
GRANT UPDATE (wrapped_root, pending_wrapped_root) ON vcon.registry_key TO vcon_app;
