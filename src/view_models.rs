use crate::models::Tag;

pub struct TagOption {
    pub id: String,
    pub name: String,
    pub color: String,
}

pub fn tag_options(tags: Vec<Tag>) -> Vec<TagOption> {
    let mut opts: Vec<TagOption> = tags
        .iter()
        .cloned()
        .map(|t| TagOption {
            id: t.id.to_string(),
            name: t.name,
            color: t.color,
        })
        .collect();

    opts.sort_by_key(|t| t.name.clone());
    opts
}
