-- Run as a PostgreSQL administrator after Platform's `01-database-and-roles.sql` has created the database `ratatoskr`, and before the first sites start. Set the password separately with `\password ratatoskr_sites`; no credential belongs in this repository.

do $$
begin
    if not exists (select 1 from pg_roles where rolname = 'ratatoskr_sites') then
        create role ratatoskr_sites login;
    end if;
end
$$;
