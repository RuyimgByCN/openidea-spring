use crate::boot_metadata::MetadataIndex;
use crate::model::SpringProjectModel;
use crate::parser_config::resolve_effective_properties;
use openidea_plugin::search::{SearchItem, SearchPage, SearchTarget};

pub fn search_category(
    model: &SpringProjectModel,
    active_profile: Option<&str>,
    category: &str,
    query: &str,
    limit: u32,
) -> SearchPage {
    let q = query.trim().to_ascii_lowercase();
    let max = (limit as usize).clamp(1, 200);
    match category {
        "spring-beans" => search_beans(model, &q, max),
        "spring-config" => search_config(model, active_profile, &q, max),
        _ => SearchPage::default(),
    }
}

fn search_beans(model: &SpringProjectModel, q: &str, max: usize) -> SearchPage {
    let mut scored: Vec<(i32, SearchItem)> = Vec::new();
    for b in model.all_beans() {
        let name_l = b.name.to_ascii_lowercase();
        let cls_l = b.class_name.to_ascii_lowercase();
        let score = if q.is_empty() {
            10
        } else if name_l == q || cls_l == q {
            1000
        } else if name_l.starts_with(q) || cls_l.starts_with(q) {
            800
        } else if name_l.contains(q) || cls_l.contains(q) || b.class_fqn.to_ascii_lowercase().contains(q) {
            500
        } else if fuzzy_subseq(&cls_l, q) {
            200
        } else {
            continue;
        };
        let prof = if b.profiles.is_empty() {
            String::new()
        } else {
            format!(" · @Profile({})", b.profiles.join(","))
        };
        scored.push((
            score,
            SearchItem {
                id: b.id.clone(),
                title: format!("{} ({})", b.name, b.class_name),
                description: Some(format!(
                    "{} · {}{prof}",
                    b.stereotype.label(),
                    short_rel_path(&model.root, &b.file)
                )),
                target: SearchTarget::Location(SpringProjectModel::to_location(&b.file, b.name_range)),
            },
        ));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));
    let has_more = scored.len() > max;
    let items = scored.into_iter().take(max).map(|(_, it)| it).collect();
    SearchPage { items, has_more }
}

fn search_config(
    model: &SpringProjectModel,
    active_profile: Option<&str>,
    q: &str,
    max: usize,
) -> SearchPage {
    let files = model.all_config_files();
    let eff = resolve_effective_properties(&files, active_profile);
    let cp_classes = model.all_config_prop_classes();
    let meta_idx = MetadataIndex::build(&model.custom_metadata, &cp_classes);

    let mut scored: Vec<(i32, SearchItem)> = Vec::new();
    for p in &eff {
        let kl = p.key.to_ascii_lowercase();
        let score = if q.is_empty() {
            20
        } else if kl == q {
            1000
        } else if kl.starts_with(q) {
            800
        } else if kl.contains(q) || p.masked_value.to_ascii_lowercase().contains(q) {
            500
        } else {
            continue;
        };
        let desc = meta_idx
            .get(&p.key)
            .map(|m| format!("{} — {}", p.source_profile, m.description))
            .unwrap_or_else(|| format!("{} · {}", p.source_profile, short_rel_path(&model.root, &p.source_file)));
        scored.push((
            score,
            SearchItem {
                id: format!("cfg:{}:{}", p.key, p.source_file),
                title: format!("{} = {}", p.key, p.masked_value),
                description: Some(desc),
                target: SearchTarget::Location(SpringProjectModel::to_location(&p.source_file, p.key_range)),
            },
        ));
    }

    // Also include matching @ConfigurationProperties fields
    for cls in &cp_classes {
        for field in &cls.fields {
            let kl = field.full_key.to_ascii_lowercase();
            let score = if q.is_empty() {
                10
            } else if kl == q {
                950
            } else if kl.starts_with(q) {
                750
            } else if kl.contains(q) {
                450
            } else {
                continue;
            };
            scored.push((
                score,
                SearchItem {
                    id: format!("cp:{}:{}", field.full_key, cls.file),
                    title: format!("{} ({})", field.full_key, field.type_name),
                    description: Some(format!(
                        "@ConfigurationProperties · {}.{}",
                        cls.class_name, field.name
                    )),
                    target: SearchTarget::Location(SpringProjectModel::to_location(&cls.file, field.range)),
                },
            ));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));
    let has_more = scored.len() > max;
    let items = scored.into_iter().take(max).map(|(_, it)| it).collect();
    SearchPage { items, has_more }
}

fn fuzzy_subseq(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut n_chars = needle.chars();
    let mut cur = n_chars.next();
    for h in haystack.chars() {
        if Some(h) == cur {
            cur = n_chars.next();
            if cur.is_none() {
                return true;
            }
        }
    }
    false
}

fn short_rel_path<'a>(root: &str, full: &'a str) -> &'a str {
    full.strip_prefix(root)
        .map(|s| s.trim_start_matches(['/', '\\']))
        .unwrap_or(full)
}
