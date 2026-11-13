-- Schema version 10: a registry transaction reads the lock rows of every
-- tenant.
--
-- A tenant that has a catalog has a row in vcon.tenant_lock. A server
-- removes the catalog of a tenant that has no row in the registry: a
-- create or a removal that was cut off leaves one. To find them it lists
-- the tenants that have a lock row. The one policy of version 1 showed a
-- lock row to its tenant's transaction only, and the policies are forced
-- on the owner, so no role could list them.
--
-- From this version a SELECT of vcon.tenant_lock passes in a registry
-- transaction for every row, and in a tenant's transaction for that
-- tenant's row, as before. The registry holds every tenant's id already
-- (vcon.tenant), so the list shows it one thing more: which ids have a
-- catalog. INSERT, UPDATE and DELETE stay the tenant's own: each asks for
-- the tenant, which raises in a registry transaction, and so does the
-- row lock a tenant's write takes (SELECT FOR UPDATE reads the policy of
-- an UPDATE too).
--
-- The read's rule is a CASE, which gives an order: vcon.tenant() raises
-- in a registry transaction, and two policies joined by OR have none. A
-- transaction with no marker, or with one that is neither the registry's
-- nor a tenant id, comes to vcon.tenant() and is refused by it at the
-- first row; the planner does not evaluate a CASE, so on a table with no
-- row such a read ends with nothing and no error.
--
-- No other table is changed, no privilege, and no row.
DROP POLICY tenant_rows ON vcon.tenant_lock;
CREATE POLICY lock_reads ON vcon.tenant_lock FOR SELECT
    USING (CASE WHEN current_setting('vcon.tenant', true) = 'registry' THEN true
                ELSE tenant = vcon.tenant() END);
CREATE POLICY lock_inserts ON vcon.tenant_lock FOR INSERT
    WITH CHECK (tenant = vcon.tenant());
CREATE POLICY lock_updates ON vcon.tenant_lock FOR UPDATE
    USING (tenant = vcon.tenant()) WITH CHECK (tenant = vcon.tenant());
CREATE POLICY lock_deletes ON vcon.tenant_lock FOR DELETE
    USING (tenant = vcon.tenant());
