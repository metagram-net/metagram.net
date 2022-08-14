use clap::Parser;
use diesel_async::{AsyncConnection, AsyncPgConnection};
use fake::{faker::lorem::en as lorem, Dummy, Fake};
use metagram::{auth, firehose};
use rand::{distributions::Uniform, rngs::StdRng, Rng, SeedableRng};

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

const NUM_DROPS: u8 = 50;
const NUM_STREAMS: u8 = 5;
const NUM_TAGS: u8 = 10;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut db = {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        AsyncPgConnection::establish(&url)
            .await
            .expect("database connection")
    };

    seed(&mut db, args).await
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

    let tags = seed_tags(db, &mut rng, &user).await?;
    for tag in &tags {
        println!("Created tag: {} {}", tag.name, tag.color);
    }

    let streams = seed_streams(db, &mut rng, &user, &tags).await?;
    for stream in &streams {
        println!(
            "Created stream: {} {:?}",
            stream.stream.name,
            stream.tag_names()
        );
    }

    let drops = seed_drops(db, &mut rng, &user, &tags).await?;
    for drop in &drops {
        let tags = drop
            .tags
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<String>>()
            .join(" ");
        println!(
            "Created drop: {:?} {} {:?} {:?}",
            drop.drop.title, drop.drop.url, tags, drop.drop.status,
        );
    }

    // TODO(hydrants): seed hydrants without spamming real sites?

    Ok(())
}

async fn seed_tags(
    db: &mut AsyncPgConnection,
    rng: &mut StdRng,
    user: &metagram::models::User,
) -> anyhow::Result<Vec<firehose::Tag>> {
    let mut tags = Vec::new();
    for _ in 0..NUM_TAGS {
        let name = capitalize(lorem::Word().fake_with_rng(rng));
        let color = Color::random(rng);
        let tag = firehose::create_tag(db, user, &name, &color.css()).await?;
        tags.push(tag);
    }
    Ok(tags)
}

async fn seed_streams(
    db: &mut AsyncPgConnection,
    rng: &mut StdRng,
    user: &metagram::models::User,
    all_tags: &Vec<firehose::Tag>,
) -> anyhow::Result<Vec<firehose::CustomStream>> {
    let mut streams = Vec::new();
    for _ in 0..NUM_STREAMS {
        let phrase: Vec<String> = lorem::Words(1..3).fake_with_rng(rng);
        let name = capitalize(phrase.join(" "));

        let tag_count = rng.gen_range(0..3);
        let tags: Vec<firehose::Tag> = rng
            .sample_iter(Uniform::new(0, all_tags.len()))
            .take(tag_count)
            .map(|i| all_tags[i].clone())
            .collect();

        let stream = firehose::create_stream(db, user, &name, &tags).await?;
        streams.push(stream);
    }
    Ok(streams)
}

async fn seed_drops(
    db: &mut AsyncPgConnection,
    rng: &mut StdRng,
    user: &metagram::models::User,
    all_tags: &Vec<firehose::Tag>,
) -> anyhow::Result<Vec<firehose::Drop>> {
    let mut drops = Vec::new();
    for _ in 0..NUM_DROPS {
        let article: Article = Title(1..10).fake_with_rng(rng);

        let title = if rng.gen_bool(0.9) {
            Some(article.title)
        } else {
            None
        };

        let tag_count = rng.gen_range(0..3);
        let tags: Vec<firehose::TagSelector> = rng
            .sample_iter(Uniform::new(0, all_tags.len()))
            .take(tag_count)
            .map(|i| firehose::TagSelector::Find { id: all_tags[i].id })
            .collect();

        let drop = firehose::create_drop(
            db,
            user.clone(),
            title,
            article.url,
            Some(tags),
            chrono::Utc::now(),
        )
        .await?;

        let status: firehose::DropStatus = rng.gen();
        let drop = firehose::move_drop(db, drop, status, chrono::Utc::now()).await?;

        drops.push(drop);
    }
    Ok(drops)
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
        // TODO: add some randomized punctuation: end=!? inner=,:&
        let title = capitalize(s);

        let base_url = url::Url::parse("https://example.com").expect("base_url");
        let url = base_url.join(&words.join("-")).unwrap().to_string();

        Article { title, url }
    }
}

fn capitalize(s: String) -> String {
    s[0..1].to_uppercase() + &s[1..]
}

#[derive(Debug)]
struct Color {
    r: u8,
    g: u8,
    b: u8,
}

impl Color {
    fn random<R: Rng + ?Sized>(rng: &mut R) -> Color {
        Color {
            r: rng.gen(),
            g: rng.gen(),
            b: rng.gen(),
        }
    }

    fn css(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}
