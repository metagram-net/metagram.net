use clap::{Parser, Subcommand};

mod invite;
mod seed;

#[derive(Parser, Debug)]
#[clap(name = "Metagram Dev Tools")]
#[clap(author, version, about, long_about = None)]
#[clap(arg_required_else_help(true))]
struct Cli {
    #[clap(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Generate fake test data for local development.
    Seed(seed::Cli),

    /// Invite a new user by email address.
    Invite(invite::Cli),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Seed(cmd) => cmd.run().await,
        Cmd::Invite(cmd) => cmd.run().await,
    }
}
