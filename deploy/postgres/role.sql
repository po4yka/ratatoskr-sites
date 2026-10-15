-- Run as a PostgreSQL administrator after Platform's `01-database-and-roles.sql` has created the database `ratatoskr`, and before the first sites start. Set the password separately with `\password ratatoskr_sites`; no credential belongs in this repository.
-- `create` on the database is all the schema needs: the service creates the `sites` schema on its first start and owns everything in it. The role gets nothing in the `public` schema.

do $$
begin
    if not exists (select 1 from pg_roles where rolname = 'ratatoskr_sites') then
        create role ratatoskr_sites login;
    end if;
end
$$;

alter role ratatoskr_sites nosuperuser nocreatedb nocreaterole noreplication nobypassrls;
grant connect, create on database ratatoskr to ratatoskr_sites;

\connect ratatoskr
revoke all on schema public from ratatoskr_sites;
