pub fn pluralize(count: usize, noun: &str) -> askama::Result<String> {
    let singular = noun;
    let plural = noun.to_string() + "s";
    let word = inflect(count.try_into().unwrap(), singular, &plural).unwrap();
    Ok(format!("{} {}", count, word))
}

pub fn inflect(count: i64, singular: &str, plural: &str) -> askama::Result<String> {
    match count {
        -1 | 1 => Ok(singular.to_string()),
        _ => Ok(plural.to_string()),
    }
}

pub fn yes_no(b: &bool) -> askama::Result<&'static str> {
    match b {
        true => Ok("yes"),
        false => Ok("no"),
    }
}
