create table tags (
    id uuid primary key default gen_random_uuid(),
    user_id uuid references users(id) not null,

    name text not null check (name != ''),
    color text not null check (color ~* '^#[0-9a-fA-F]{6}$'),

    created_at timestamp not null default now(),
    updated_at timestamp not null default now()
);

select diesel_manage_updated_at('tags');

create table drop_tags (
    id uuid primary key default gen_random_uuid(),
    drop_id uuid references drops(id) not null,
    tag_id uuid references tags(id) not null
);

select diesel_manage_updated_at('drop_tags');
