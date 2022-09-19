create table streams (
    id uuid primary key default gen_random_uuid(),
    user_id uuid references users(id) not null,

    name text not null check (name != ''),
    tag_ids uuid[] not null check (array_length(tag_ids, 1) > 0),

    created_at timestamp not null default now(),
    updated_at timestamp not null default now()
);

select manage_updated_at('streams');
