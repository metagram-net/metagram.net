-- The original Go version of Drift has now been rewritten in Rust and renamed
-- Squill. So it's time to change the function names!
--
-- The old names will be deleted in a later migration.

-- _squill_claim_migration registers a migration in the schema_migrations
-- table. It will fail if the migration ID has already been claimed.
--
-- Squill will call this at the start of every "up" migration transaction. For
-- migrations that cannot be run within transactions, it is the migration's
-- responsibility to call this.
create function _squill_claim_migration(mid bigint, mname text) returns void as $$
    insert into schema_migrations (id, name) values (mid, mname);
$$ language sql;

-- _squill_unclaim_migration removes a migration from the schema_migrations
-- table.
--
-- When iterating on a migration in development, it's useful to have a down
-- migration to reset back to the previous schema. Squill will call this at the
-- start of every "down" migration transaction so the "up" migration can run
-- again.
create function _squill_unclaim_migration(mid bigint) returns void as $$
    delete from schema_migrations where id = mid;
$$ language sql;

-- _squill_require_migration asserts that the migration ID has already been
-- claimed in the schema_migrations table.
--
-- Call this from within a migration to ensure that another migration has
-- already run to completion.
create function _squill_require_migration(mid bigint) returns void as $$
declare
    mrow schema_migrations%rowtype;
begin
    select * into mrow from schema_migrations where id = mid;
    if not found then
        raise exception 'Required migration has not been run: %', mid;
    end if;
end;
$$ language plpgsql;
