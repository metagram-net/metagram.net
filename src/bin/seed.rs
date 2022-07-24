use clap::Parser;
use diesel_async::{AsyncConnection, AsyncPgConnection};
use fake::{faker::lorem::en as lorem, Dummy, Fake};
use firehose::{auth, firehose as fh};
use rand::{rngs::StdRng, Rng, SeedableRng};

#[derive(Parser, Debug)]
#[clap(name = "Firehose Seed")]
#[clap(about = "Generate fake test data for local development.", long_about = None)]
#[clap(version)]
struct Args {
    #[clap(long, value_parser)]
    stytch_user_id: String,

    #[clap(long, value_parser)]
    rng_seed: Option<u64>,
}

const NUM_DROPS: u8 = 10;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let mut db = {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        AsyncPgConnection::establish(&url)
            .await
            .expect("database connection")
    };

    seed(&mut db, args).await.unwrap();
}

async fn seed(db: &mut AsyncPgConnection, args: Args) -> anyhow::Result<()> {
    let mut rng = {
        let rng_seed = match args.rng_seed {
            Some(seed) => seed,
            None => chrono::Utc::now()
                .timestamp()
                .try_into()
                .expect("Unix timestamp in u64"),
        };
        println!("RNG seed: {}", rng_seed);
        StdRng::seed_from_u64(rng_seed)
    };

    // TODO: What if this actually had someone auth by email?
    let user = auth::create_user(db, args.stytch_user_id).await?;
    println!("Created user: {}", user.id);

    for _ in 0..NUM_DROPS {
        let article: Article = Title(1..10).fake_with_rng(&mut rng);

        let title = if rng.gen_bool(0.9) {
            Some(article.title)
        } else {
            None
        };

        let fields = fh::DropFields {
            title,
            url: article.url,
            ..Default::default()
        };
        let drop = fh::create_drop(db, &user, fields, chrono::Utc::now()).await?;
        println!("Created drop: {}", drop.id);
    }

    // TODO(tags): Create a bunch of fake tags.
    // TODO(tags): Create some streams out of some of those tags.

    Ok(())
}

struct Title(std::ops::Range<usize>);

struct Article {
    title: String,
    url: String,
}

impl Dummy<Title> for Article {
    fn dummy_with_rng<R: Rng + ?Sized>(t: &Title, rng: &mut R) -> Article {
        let words: Vec<String> = lorem::Words(t.0.clone()).fake_with_rng(rng);
        let s = words.join(" ");
        // todo: add some randomized punctuation: end=!? inner=,:&
        let title = s[0..1].to_uppercase() + &s[1..];

        let base_url = url::Url::parse("https://example.com").expect("base_url");
        let url = base_url.join(&words.join("-")).unwrap().to_string();

        Article { title, url }
    }
}
