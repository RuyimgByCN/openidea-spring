use crate::index::{ProjectCache, resolve_injection};
use crate::model::{EndpointKind, SpringProjectModel};
use crate::text_util::position_contains;
use openidea_plugin::file_actions::{FileAction, FileOperation};
use openidea_plugin::language_features::{Document, DocumentLink, GutterAction, Injection};
use openidea_plugin::{Location, Position, Range};

pub fn implementation(
    cache: &mut ProjectCache,
    doc: &Document,
    at: Position,
    active_profile: Option<&str>,
) -> Vec<Location> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    for inj in &facts.injections {
        if position_contains(inj.range, at) || position_contains(inj.type_range, at) {
            let (_, targets) = resolve_injection(model, inj, active_profile);
            return targets
                .into_iter()
                .map(|t| SpringProjectModel::to_location(&t.file, t.range))
                .collect();
        }
    }
    Vec::new()
}

pub fn document_links(
    cache: &mut ProjectCache,
    doc: &Document,
    active_profile: Option<&str>,
) -> Vec<DocumentLink> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut links = Vec::new();
    for lt in &facts.link_targets {
        let cleaned = lt
            .raw_target
            .trim_start_matches("optional:")
            .trim_start_matches("classpath:")
            .trim_start_matches("classpath*:")
            .trim_start_matches("file:")
            .trim();
        if cleaned.is_empty() {
            continue;
        }
        // Match against known project files by suffix
        if let Some(target_file) = model
            .files
            .keys()
            .find(|p| p.ends_with(cleaned) || p.rsplit(['/', '\\']).next() == Some(cleaned))
        {
            links.push(DocumentLink {
                range: lt.range,
                target: target_file.clone(),
            });
        }
    }
    links
}

pub fn gutter_actions(
    cache: &mut ProjectCache,
    doc: &Document,
    active_profile: Option<&str>,
) -> Vec<GutterAction> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    for b in &facts.beans {
        actions.push(GutterAction {
            line: b.line,
            icon: Some("icons/marker_bean.svg".into()),
            tooltip: format!("Inspect `{}` in Spring Dependency Graph", b.name),
            command: "inspect-bean-at-caret".into(),
            argument: Some(b.id.clone()),
        });
    }
    for ep in &facts.endpoints {
        if matches!(ep.kind, EndpointKind::WebFlux | EndpointKind::HttpExchange) {
            actions.push(GutterAction {
                line: ep.line,
                icon: Some("icons/marker_endpoint.svg".into()),
                tooltip: format!("Copy cURL for {} {}", ep.methods.join(","), ep.full_path),
                command: "copy-endpoint-curl".into(),
                argument: Some(format!("{}:{}", doc.path, ep.line)),
            });
        }
    }
    for sch in &facts.scheduled_sites {
        if sch.cron.is_some() {
            actions.push(GutterAction {
                line: sch.line,
                icon: Some("icons/marker_scheduled.svg".into()),
                tooltip: "Explain 6-field Spring cron expression".into(),
                command: "explain-cron".into(),
                argument: sch.cron.clone(),
            });
        }
    }
    actions
}

/// Language Injection: embeds `sql` inside Spring Data `@Query(...)` and `@NativeQuery(...)` literals.
pub fn sql_injections(
    cache: &mut ProjectCache,
    doc: &Document,
    active_profile: Option<&str>,
) -> Vec<Injection> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for repo in &facts.repositories {
        for m in &repo.methods {
            if let Some(q) = &m.query
                && !q.query.trim().is_empty()
                && q.inner_range.start < q.inner_range.end
            {
                out.push(Injection {
                    range: q.inner_range,
                    language: "sql".into(),
                });
            }
        }
    }
    out
}

/// File Actions: offers creating `src/main/resources/application-<profile>.yml` for referenced profiles.
pub fn file_actions(
    cache: &mut ProjectCache,
    doc: &Document,
    selection: Range,
    active_profile: Option<&str>,
) -> Vec<FileAction> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    let existing_profiles: std::collections::BTreeSet<String> = model
        .all_config_files()
        .into_iter()
        .filter_map(|c| c.profile.clone())
        .collect();

    for pref in &facts.profile_refs {
        let clean = pref.profile.trim().trim_start_matches('!');
        if !clean.is_empty()
            && !existing_profiles.contains(clean)
            && pref.range.start.line == selection.start.line
        {
            let target_rel = format!("src/main/resources/application-{clean}.yml");
            actions.push(FileAction {
                title: format!("Spring Boot: Create `{target_rel}`"),
                kind: Some("quickfix".into()),
                preferred: false,
                operations: vec![FileOperation::Create {
                    path: target_rel.clone(),
                    text: format!(
                        "# Spring Boot configuration for `{clean}` profile\nserver:\n  port: 8080\n"
                    ),
                    overwrite: false,
                }],
                edits: Vec::new(),
                open: Some(target_rel),
                data: None,
            });
        }
    }
    actions
}
