use clap::{Args, Subcommand};
use lazy_static::lazy_static;
use regex::Regex;
use sqlx::postgres::PgConnection;
use sqlx::{Connection, Executor};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::{env, fs};
use tabwriter::TabWriter;

#[derive(Args, Debug)]
pub struct Cli {
    #[clap(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Init,
    New(New),
    Renumber(Renumber),
    Status,
    Migrate,
    Undo,
    Redo,
}

#[derive(Args, Debug)]
struct New {
    #[clap(long, value_parser)]
    id: Option<i64>,

    #[clap(long, value_parser)]
    name: String,
}

#[derive(Args, Debug)]
struct Renumber {
    #[clap(long, value_parser, default_value = "false")]
    write: bool,
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        match self.command {
            Cmd::Init => init(),
            Cmd::New(args) => new(args),
            Cmd::Renumber(args) => renumber(args),
            Cmd::Status => {
                let mut conn = connect().await?;
                status(&mut conn).await
            }
            Cmd::Migrate => {
                let mut conn = connect().await?;
                migrate(&mut conn).await
            }
            Cmd::Undo => {
                let mut conn = connect().await?;
                undo(&mut conn).await
            }
            Cmd::Redo => {
                let mut conn = connect().await?;
                redo(&mut conn).await
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Migration {
    id: MigrationId,
    name: String,
    path: PathBuf,
}

impl std::fmt::Display for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.to_string_lossy())
    }
}

impl Migration {
    async fn up(self, conn: &mut PgConnection) -> anyhow::Result<()> {
        let path = self.path.join("up.sql");

        let sql = std::fs::read_to_string(path)?;

        if RE_NO_TX.is_match(&sql) {
            conn.execute(&*sql).await?;
        } else {
            conn.transaction(|conn| {
                Box::pin(async move {
                    claim(conn, self).await?;
                    conn.execute(&*sql).await
                })
            })
            .await?;
        }
        Ok(())
    }

    async fn down(self, conn: &mut PgConnection) -> anyhow::Result<()> {
        let path = self.path.join("down.sql");

        let sql = std::fs::read_to_string(path)?;

        if RE_NO_TX.is_match(&sql) {
            conn.execute(&*sql).await?;
        } else {
            conn.transaction(|conn| {
                Box::pin(async move {
                    unclaim(conn, self).await?;
                    conn.execute(&*sql).await
                })
            })
            .await?;
        }
        Ok(())
    }
}

// Migration ID has to fit in an i64 for Postgres purposes, but it should always be non-negative.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct MigrationId(i64);

impl MigrationId {
    fn width(&self) -> usize {
        // TODO(int_log): self.0.checked_log10().unwrap_or(0) + 1
        format!("{}", self.0).chars().count()
    }
}

#[derive(thiserror::Error, Debug)]
enum ParseMigrationIdError {
    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),

    #[error("negative number: {0}")]
    Negative(i64),
}

impl std::str::FromStr for MigrationId {
    type Err = ParseMigrationIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let i: i64 = s.parse()?;
        if i < 0 {
            return Err(Self::Err::Negative(i));
        }
        Ok(Self(i))
    }
}

// TODO: Allow configuring migrations dir.
const MIGRATIONS_DIR: &str = "migrations";

lazy_static! {
    static ref RE_MIGRATION: Regex = Regex::new(r"^(?P<id>\d+)-(?P<name>.*)$").unwrap();
    static ref RE_NO_TX: Regex = Regex::new("(?m)^--drift:no-transaction").unwrap();
}

async fn connect() -> anyhow::Result<PgConnection> {
    let url = env::var("DATABASE_URL").expect("DATABASE_URL");
    let conn = PgConnection::connect(&url).await?;
    Ok(conn)
}

fn available_migrations(dir: &str) -> anyhow::Result<Vec<Migration>> {
    let mut paths: Vec<Migration> = fs::read_dir(dir)?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.is_dir() {
                let m = RE_MIGRATION.captures(path.file_name()?.to_str()?)?;

                let id = m.name("id")?.as_str().parse().ok()?;
                let name = m.name("name")?.as_str().to_string();

                Some(Migration { id, name, path })
            } else {
                None
            }
        })
        .collect();

    paths.sort_by_key(|m| m.id.0);
    Ok(paths)
}

#[derive(sqlx::FromRow)]
struct MigrationRow {
    id: i64,
    name: String,
    run_at: chrono::NaiveDateTime,
}

async fn applied_migrations(conn: &mut PgConnection) -> anyhow::Result<Vec<MigrationRow>> {
    let query = sqlx::query_as("select * from schema_migrations order by id asc");
    match query.fetch_all(conn).await {
        Ok(res) => Ok(res),
        Err(err) => {
            if let sqlx::Error::Database(ref db_err) = err {
                if let Some(code) = db_err.code() {
                    // undefined_table
                    if code == "42P01" {
                        // The expected table doesn't exist. This is probably because we haven't
                        // run the first migration that will create this table.
                        return Ok(Vec::new());
                    }
                }
            }
            Err(err.into())
        }
    }
}

async fn status(conn: &mut PgConnection) -> anyhow::Result<()> {
    // TODO: There's definitely a more efficient way to do this, but 🤷

    let applied: HashMap<MigrationId, MigrationRow> = applied_migrations(conn)
        .await?
        .into_iter()
        .map(|row| (MigrationId(row.id), row))
        .collect();

    let available: HashMap<MigrationId, Migration> = available_migrations(MIGRATIONS_DIR)?
        .into_iter()
        .map(|m| (m.id, m))
        .collect();

    let applied_ids: HashSet<MigrationId> = applied.keys().cloned().collect();
    let available_ids: HashSet<MigrationId> = available.keys().cloned().collect();
    let mut all_ids: Vec<MigrationId> = applied_ids.union(&available_ids).cloned().collect();

    all_ids.sort();

    let mut table = TabWriter::new(std::io::stdout());
    for id in all_ids {
        match (applied.get(&id), available.get(&id)) {
            (Some(row), Some(_)) => {
                writeln!(table, "{}\t{}\trun at {}", row.id, row.name, row.run_at)?
            }
            (Some(row), None) => writeln!(
                table,
                "{}\t{}\trun at{} (missing directory)",
                row.id, row.name, row.run_at
            )?,
            (None, Some(dir)) => writeln!(table, "{}\t{}\ttodo", dir.id.0, dir.name)?,
            (None, None) => (), // This is impossible, right?
        }
    }
    table.flush()?;

    Ok(())
}

async fn migrate(conn: &mut PgConnection) -> anyhow::Result<()> {
    let applied: HashMap<MigrationId, MigrationRow> = applied_migrations(conn)
        .await?
        .into_iter()
        .map(|row| (MigrationId(row.id), row))
        .collect();

    for migration in available_migrations(MIGRATIONS_DIR)? {
        if applied.contains_key(&migration.id) {
            continue;
        }

        println!("Running up migration: {}", migration);
        migration.up(conn).await?;
    }

    Ok(())
}

async fn undo(conn: &mut PgConnection) -> anyhow::Result<()> {
    let migration = last_applied(conn).await?;

    println!("Running down migration: {}", migration);
    migration.down(conn).await?;

    Ok(())
}

async fn redo(conn: &mut PgConnection) -> anyhow::Result<()> {
    let migration = last_applied(conn).await?;

    println!("Undoing migration: {}", migration);
    migration.clone().down(conn).await?;

    println!("Redoing migration: {}", migration);
    migration.up(conn).await?;

    Ok(())
}

async fn last_applied(conn: &mut PgConnection) -> anyhow::Result<Migration> {
    let applied = applied_migrations(conn).await?;

    let last = match applied.iter().max_by_key(|row| row.run_at) {
        None => return Err(anyhow::anyhow!("No migrations have been run.")),
        Some(last) => last,
    };

    let matches: Vec<Migration> = available_migrations(MIGRATIONS_DIR)?
        .iter()
        .cloned()
        .filter(|m| m.id.0 == last.id)
        .collect();

    match matches.len() {
        1 => Ok(matches[0].clone()),
        0 => Err(anyhow::anyhow!(
            "No migration directory found for migration ID: {}",
            last.id
        )),
        n => Err(anyhow::anyhow!(
            "{} migration directories found for migration ID: {}",
            n,
            last.id
        )),
    }
}

async fn claim(conn: &mut PgConnection, m: Migration) -> sqlx::Result<()> {
    conn.execute(
        sqlx::query("select _drift_claim_migration($1, $2)")
            .bind(m.id.0)
            .bind(m.name),
    )
    .await?;
    Ok(())
}

async fn unclaim(conn: &mut PgConnection, m: Migration) -> sqlx::Result<()> {
    conn.execute(sqlx::query("select _drift_unclaim_migration($1)").bind(m.id.0))
        .await?;
    Ok(())
}

const NEW_UP_SQL: &str = include_str!("./new.up.sql");
const NEW_DOWN_SQL: &str = include_str!("./new.down.sql");

fn new(args: New) -> anyhow::Result<()> {
    let id = match args.id {
        Some(id) => id,
        None => chrono::Utc::now().timestamp(),
    };

    let name = slugify(args.name);

    let dir = PathBuf::from(MIGRATIONS_DIR).join(format!("{}-{}", id, name));
    let up = dir.join("up.sql");
    let down = dir.join("down.sql");

    println!("Creating migration directory: {}", dir.to_string_lossy());
    fs::create_dir_all(&dir)?;

    // TODO: Allow custom NEW_*_SQL templates.

    println!("Creating migration file: {}", up.to_string_lossy());
    fs::File::create(&up)?.write_all(NEW_UP_SQL.as_bytes())?;

    println!("Creating migration file: {}", down.to_string_lossy());
    fs::File::create(&down)?.write_all(NEW_DOWN_SQL.as_bytes())?;

    Ok(())
}

fn slugify(s: String) -> String {
    lazy_static! {
        static ref RE_SEP: Regex = Regex::new(r"[\-\s._/]+").unwrap();
    }
    RE_SEP.replace_all(&s, "_").to_string()
}

const INIT_UP_SQL: &str = include_str!("./init.up.sql");
const INIT_DOWN_SQL: &str = include_str!("./init.down.sql");

fn init() -> anyhow::Result<()> {
    let id = 0;
    let name = "init";

    let dir = PathBuf::from(MIGRATIONS_DIR).join(format!("{}-{}", id, name));
    let up = dir.join("up.sql");
    let down = dir.join("down.sql");

    println!("Creating migration directory: {}", dir.to_string_lossy());
    fs::create_dir_all(&dir)?;

    println!("Creating migration file: {}", up.to_string_lossy());
    fs::File::create(&up)?.write_all(INIT_UP_SQL.as_bytes())?;

    println!("Creating migration file: {}", down.to_string_lossy());
    fs::File::create(&down)?.write_all(INIT_DOWN_SQL.as_bytes())?;

    println!("Run the `migrate` subcommand to apply this migration.");

    Ok(())
}

fn renumber(args: Renumber) -> anyhow::Result<()> {
    let migrations = available_migrations(MIGRATIONS_DIR)?;

    if migrations.is_empty() {
        return Err(anyhow::anyhow!("No migrations to renumber"));
    }

    let width = migrations.iter().map(|m| m.id.width()).max().unwrap();

    let mut table = TabWriter::new(std::io::stdout());
    writeln!(table, "From\tTo")?;
    writeln!(table, "----\t--")?;

    let mut renames = Vec::new();

    for m in migrations {
        let old = m.path.clone();

        let new = m
            .path
            .with_file_name(format!("{:0width$}-{}", m.id.0, m.name));

        writeln!(
            table,
            "{}\t{}",
            old.to_string_lossy(),
            new.to_string_lossy()
        )?;
        renames.push((old, new));
    }

    table.flush()?;
    println!();

    if args.write {
        print!("Renaming files...");
        for (old, new) in renames {
            fs::rename(old, new)?;
        }
        println!(" done!");
    } else {
        println!("Skipping the actual renames because writes were not enabled.");
        println!("Add --write to do the rename.");
    }

    Ok(())
}
