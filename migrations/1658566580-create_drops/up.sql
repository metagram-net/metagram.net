create type drop_status as enum ('unread', 'read', 'saved');

create table drops (
    id uuid primary key default gen_random_uuid(),
    user_id uuid references users(id) not null,

    title text check (title != ''),
    url text not null,
    status drop_status not null,
    moved_at timestamp not null,

    created_at timestamp not null default now(),
    updated_at timestamp not null default now()
);

select manage_updated_at('drops');

create index drops_moved_at on drops (moved_at);
