create table hydrants (
    id uuid primary key default gen_random_uuid(),
    user_id uuid references users(id) not null,

    name text not null check (name != ''),
    url text not null,
    active boolean not null,
    tag_ids uuid[] not null,
    fetched_at timestamp,

    created_at timestamp not null default now(),
    updated_at timestamp not null default now()
);

select manage_updated_at('hydrants');
