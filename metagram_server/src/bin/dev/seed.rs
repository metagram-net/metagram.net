use clap::Args;
use fake::{faker::lorem::en as lorem, Dummy, Fake};
use metagram_server::{auth, firehose};
use rand::{distributions::Uniform, rngs::StdRng, Rng, SeedableRng};
use sqlx::{Connection, PgConnection};

#[derive(Args, Debug)]
pub struct Cli {
    #[clap(long, value_parser)]
    stytch_user_id: String,

    #[clap(long, value_parser)]
    rng_seed: Option<u64>,
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        let conn = {
            let url = std::env::var("DATABASE_URL").unwrap();
            PgConnection::connect(&url).await.unwrap()
        };

        seed(conn, self).await
    }
}

const NUM_DROPS: u8 = 50;
const NUM_HYDRANTS: u8 = 3;
const NUM_STREAMS: u8 = 5;
const NUM_TAGS: u8 = 10;

async fn seed(mut conn: PgConnection, cmd: Cli) -> anyhow::Result<()> {
    let mut rng = {
        let rng_seed = match cmd.rng_seed {
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
    let user = auth::create_user(&mut conn, cmd.stytch_user_id).await?;
    println!("Created user: {}", user.id);

    let tags = seed_tags(&mut conn, &mut rng, &user).await?;
    for tag in &tags {
        println!("Created tag: {} {}", tag.name, tag.color);
    }

    let streams = seed_streams(&mut conn, &mut rng, &user, &tags).await?;
    for stream in &streams {
        println!(
            "Created stream: {} {:?}",
            stream.stream.name,
            stream.tag_names()
        );
    }

    let drops = seed_drops(&mut conn, &mut rng, &user, &tags).await?;
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

    let hydrants = seed_hydrants(&mut conn, &mut rng, &user, &tags).await?;
    for hydrant in &hydrants {
        let tags = hydrant
            .tags
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<String>>()
            .join(" ");
        println!(
            "Created hydrant: {:?} {} {:?}",
            hydrant.hydrant.name, hydrant.hydrant.url, tags,
        );
    }

    Ok(())
}

async fn seed_tags(
    conn: &mut PgConnection,
    rng: &mut StdRng,
    user: &metagram_server::models::User,
) -> anyhow::Result<Vec<firehose::Tag>> {
    let mut tags = Vec::new();
    for _ in 0..NUM_TAGS {
        let name = capitalize(lorem::Word().fake_with_rng(rng));
        let color = Color::random(rng);
        let tag = firehose::create_tag(&mut *conn, user, &name, &color.css()).await?;
        tags.push(tag);
    }
    Ok(tags)
}

async fn seed_streams(
    conn: &mut PgConnection,
    rng: &mut StdRng,
    user: &metagram_server::models::User,
    all_tags: &Vec<firehose::Tag>,
) -> anyhow::Result<Vec<firehose::CustomStream>> {
    let mut streams = Vec::new();
    for _ in 0..NUM_STREAMS {
        let phrase: Vec<String> = lorem::Words(1..3).fake_with_rng(rng);
        let name = capitalize(phrase.join(" "));

        let tag_count = rng.gen_range(1..=3);
        let tags: Vec<firehose::Tag> = rng
            .sample_iter(Uniform::new(0, all_tags.len()))
            .take(tag_count)
            .map(|i| all_tags[i].clone())
            .collect();

        let stream = firehose::create_stream(&mut *conn, user, &name, &tags).await?;
        streams.push(stream);
    }
    Ok(streams)
}

async fn seed_drops(
    conn: &mut PgConnection,
    rng: &mut StdRng,
    user: &metagram_server::models::User,
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

        let tag_count = rng.gen_range(0..=3);
        let tags: Vec<firehose::TagSelector> = rng
            .sample_iter(Uniform::new(0, all_tags.len()))
            .take(tag_count)
            .map(|i| firehose::TagSelector::Find { id: all_tags[i].id })
            .collect();

        let drop = firehose::create_drop(
            &mut *conn,
            user,
            title,
            article.url,
            None,
            Some(tags),
            chrono::Utc::now(),
        )
        .await?;

        let status: firehose::DropStatus = rng.gen();
        let drop = firehose::move_drop(&mut *conn, drop, status, chrono::Utc::now()).await?;

        drops.push(drop);
    }
    Ok(drops)
}

async fn seed_hydrants(
    conn: &mut PgConnection,
    rng: &mut StdRng,
    user: &metagram_server::models::User,
    all_tags: &Vec<firehose::Tag>,
) -> anyhow::Result<Vec<firehose::Hydrant>> {
    let monthly_feed_url = {
        let base_url = url::Url::parse(&std::env::var("LOREM_RSS_URL").unwrap()).unwrap();
        let mut url = base_url.join("feed").unwrap();
        url.set_query(Some("unit=month"));
        url
    };

    let mut hydrants = Vec::new();
    for _ in 0..NUM_HYDRANTS {
        let name: String = Title(1..3).fake_with_rng(rng);

        let tag_count = rng.gen_range(0..=3);
        let tags: Vec<firehose::TagSelector> = rng
            .sample_iter(Uniform::new(0, all_tags.len()))
            .take(tag_count)
            .map(|i| firehose::TagSelector::Find { id: all_tags[i].id })
            .collect();

        let active = rng.gen_bool(0.5);

        let hydrant = firehose::create_hydrant(
            &mut *conn,
            user,
            &name,
            monthly_feed_url.as_ref(),
            active,
            Some(tags),
        )
        .await?;

        hydrants.push(hydrant);
    }
    Ok(hydrants)
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

impl Dummy<Title> for String {
    fn dummy_with_rng<R: Rng + ?Sized>(t: &Title, rng: &mut R) -> String {
        let words: Vec<String> = lorem::Words(t.0.clone()).fake_with_rng(rng);
        let s = words.join(" ");
        capitalize(s)
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
