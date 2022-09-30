create table jobs (
    id uuid primary key default gen_random_uuid(),
    params jsonb not null,
    scheduled_at timestamp not null default now(),
    started_at timestamp,
    finished_at timestamp,
    error text
);

create index jobs_scheduled_at on jobs (scheduled_at) where finished_at is null;
