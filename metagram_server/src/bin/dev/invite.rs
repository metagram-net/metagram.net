use clap::Args;
use serde::Deserialize;

#[derive(Args, Debug)]
pub struct Cli {
    #[clap(long, value_parser)]
    email: String,
}

#[derive(Deserialize, Debug)]
struct Config {
    stytch_env: stytch::Env,
    stytch_project_id: String,
    stytch_secret: String,
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        let config = envy::from_env::<Config>().unwrap();

        let stytch_config = stytch::Config {
            base_url: config.stytch_env.base_url().unwrap(),
            project_id: config.stytch_project_id,
            secret: config.stytch_secret,
        };
        let client = stytch::reqwest::Client::new(stytch_config).unwrap();

        invite(client, self).await
    }
}

async fn invite(client: stytch::reqwest::Client, cmd: Cli) -> anyhow::Result<()> {
    let req = stytch::users::CreateRequest {
        email: Some(cmd.email),
        ..Default::default()
    };
    let res: stytch::users::CreateResponse = client.send(req.build()).await?;

    println!("{:?}", res);
    Ok(())
}
