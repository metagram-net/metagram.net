create table users (
    id uuid primary key default gen_random_uuid(),
    stytch_user_id text unique not null check (stytch_user_id != ''),

    created_at timestamp not null default now(),
    updated_at timestamp not null default now()
);

select diesel_manage_updated_at('users');
