use crate::boot_metadata::MetadataIndex;
use crate::data_jpa::derived_method_completions;
use crate::index::{ProjectCache, resolve_injection};
use crate::model::{EndpointKind, EventKind, SpringProjectModel, WiringStatus};
use crate::parser_config::{config_entry_at, properties_to_yaml, yaml_to_properties};
use crate::parser_java::convert_field_injection_to_constructor;
use crate::text_util::{LineIndex, normalize_relaxed_key, position_contains, word_at};
use openidea_plugin::providers::{
    CodeAction, CompletionItem, CompletionKind, Document, LineMarker,
};
use openidea_plugin::{Error, Location, PickItem, Position, Range, TextEdit, host};
use std::collections::BTreeSet;

pub fn completion(
    cache: &mut ProjectCache,
    doc: &Document,
    at: Position,
    _trigger: Option<&str>,
    active_profile: Option<&str>,
) -> Vec<CompletionItem> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let cp_classes = model.all_config_prop_classes();
    let meta_idx = MetadataIndex::build(&model.custom_metadata, &cp_classes);
    let line_idx = LineIndex::new(&doc.text);

    // 1. YAML / Properties completion
    if matches!(doc.language.as_str(), "yaml" | "yml" | "properties") {
        let ls = line_idx.line_start_byte(at.line);
        let cursor_byte = line_idx.pos_to_byte(&doc.text, at);
        let before_cursor = &doc.text[ls..cursor_byte.min(doc.text.len())];

        // Value completion after `:` or `=`
        if let Some((left, right)) = before_cursor
            .split_once(':')
            .or_else(|| before_cursor.split_once('='))
        {
            let right_trimmed = right.trim();
            if let Some(ph_start) = right_trimmed.rfind("${")
                && !right_trimmed[ph_start..].contains('}')
            {
                let prefix = &right_trimmed[ph_start + 2..];
                return complete_config_keys(model, &meta_idx, prefix, active_profile);
            }
            let full_key = resolve_yaml_line_key(&doc.text, &line_idx, at.line, left.trim());
            if let Some(meta) = meta_idx.get(&full_key) {
                let mut items = Vec::new();
                for val in &meta.allowed_values {
                    if right_trimmed.is_empty() || val.to_ascii_lowercase().starts_with(&right_trimmed.to_ascii_lowercase()) {
                        items.push(CompletionItem {
                            detail: Some(meta.name.clone()),
                            documentation: Some(meta.description.clone()),
                            ..CompletionItem::new(val, CompletionKind::EnumMember)
                        });
                    }
                }
                if (meta.type_name.ends_with("Boolean") || meta.type_name == "boolean") && items.is_empty() {
                    for b in ["true", "false"] {
                        items.push(CompletionItem::new(b, CompletionKind::Value));
                    }
                }
                return items;
            }
            return Vec::new();
        }

        // Key completion
        let prefix = before_cursor.trim();
        let full_prefix = if doc.language == "properties" {
            prefix.to_owned()
        } else {
            resolve_yaml_prefix_for_completion(&doc.text, &line_idx, at.line, before_cursor)
        };
        let replace = word_at(&doc.text, &line_idx, at).map(|(_, r)| r);
        return meta_idx
            .complete(&full_prefix, 60)
            .into_iter()
            .map(|m| {
                let insert = if doc.language == "properties" {
                    format!("{}=", m.name)
                } else {
                    let seg = m.name.rsplit('.').next().unwrap_or(&m.name);
                    format!("{seg}: ")
                };
                CompletionItem {
                    label: m.name.clone(),
                    detail: Some(short_java_type(&m.type_name)),
                    kind: Some(CompletionKind::Property),
                    insert_text: insert,
                    snippet: false,
                    replace,
                    sort_text: Some(if m.deprecated { format!("z_{}", m.name) } else { format!("a_{}", m.name) }),
                    filter_text: Some(m.name.clone()),
                    documentation: Some(format!(
                        "{}{}",
                        m.description,
                        m.default_value
                            .as_deref()
                            .map(|d| format!("\n\nDefault: `{d}`"))
                            .unwrap_or_default()
                    )),
                    additional_edits: Vec::new(),
                }
            })
            .collect();
    }

    // 2. Java / Kotlin completions
    let ls = line_idx.line_start_byte(at.line);
    let cursor_byte = line_idx.pos_to_byte(&doc.text, at);
    let before = &doc.text[ls..cursor_byte.min(doc.text.len())];

    // Inside `@Value("${...")`
    if before.contains("@Value") && before.contains("${") {
        if let Some(pos) = before.rfind("${") {
            let prefix = &before[pos + 2..];
            return complete_config_keys(model, &meta_idx, prefix, active_profile);
        }
    }

    // Inside `@Qualifier("...")`
    if before.contains("@Qualifier(") {
        let prefix = before
            .rsplit('"')
            .next()
            .unwrap_or("");
        let mut names = BTreeSet::new();
        for b in model.all_beans() {
            names.insert((b.name.clone(), b.class_name.clone()));
            for (q, _) in &b.qualifiers {
                names.insert((q.clone(), b.class_name.clone()));
            }
        }
        return names
            .into_iter()
            .filter(|(n, _)| prefix.is_empty() || n.to_ascii_lowercase().contains(&prefix.to_ascii_lowercase()))
            .take(50)
            .map(|(n, cls)| CompletionItem {
                detail: Some(format!("Bean ({cls})")),
                ..CompletionItem::new(&n, CompletionKind::Reference)
            })
            .collect();
    }

    // Inside `@Profile("...")`
    if before.contains("@Profile(") {
        return model
            .all_profiles()
            .into_iter()
            .map(|p| CompletionItem {
                detail: Some("Spring Profile".into()),
                ..CompletionItem::new(&p, CompletionKind::Constant)
            })
            .collect();
    }

    // Inside `@PathVariable("...")` -> suggest `{var}` from route
    if before.contains("@PathVariable(") {
        if let Some(facts) = model.files.get(&doc.path) {
            let mut vars = BTreeSet::new();
            for ep in &facts.endpoints {
                for seg in ep.full_path.split('/') {
                    if let Some(v) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                        let clean = v.split(':').next().unwrap_or(v);
                        vars.insert(clean.to_owned());
                    }
                }
            }
            return vars
                .into_iter()
                .map(|v| CompletionItem {
                    detail: Some("Route path variable".into()),
                    ..CompletionItem::new(&v, CompletionKind::Variable)
                })
                .collect();
        }
    }

    // Inside Repository interface -> derived query methods & `@Query` `:param`
    if let Some(facts) = model.files.get(&doc.path) {
        for repo in &facts.repositories {
            if at.line >= repo.line {
                if before.contains(':') {
                    let mut out = Vec::new();
                    for m in &repo.methods {
                        if (at.line as i32 - m.line as i32).abs() <= 5 {
                            for pn in &m.param_names {
                                out.push(CompletionItem {
                                    detail: Some("Method parameter".into()),
                                    ..CompletionItem::new(pn, CompletionKind::Variable)
                                });
                            }
                            for (ep, _) in &m.explicit_params {
                                out.push(CompletionItem {
                                    detail: Some("@Param".into()),
                                    ..CompletionItem::new(ep, CompletionKind::Variable)
                                });
                            }
                        }
                    }
                    if !out.is_empty() {
                        return out;
                    }
                }
                let trimmed = before.trim();
                if trimmed.is_empty() || trimmed.starts_with("find") || trimmed.starts_with("count") || trimmed.starts_with("exists") || trimmed.starts_with("delete") {
                    if let Some(entity) = model.all_entities().into_iter().find(|e| e.name == repo.entity_type) {
                        return derived_method_completions(entity)
                            .into_iter()
                            .filter(|(label, _)| trimmed.is_empty() || label.to_ascii_lowercase().starts_with(&trimmed.to_ascii_lowercase()))
                            .take(40)
                            .map(|(label, sig)| CompletionItem {
                                label,
                                detail: Some(format!("Derived query on {}", entity.name)),
                                kind: Some(CompletionKind::Method),
                                insert_text: format!("{sig};"),
                                snippet: false,
                                replace: None,
                                sort_text: None,
                                filter_text: None,
                                documentation: Some(format!("```java\n{sig};\n```")),
                                additional_edits: Vec::new(),
                            })
                            .collect();
                    }
                }
            }
        }
    }

    Vec::new()
}

fn complete_config_keys(
    model: &SpringProjectModel,
    meta_idx: &MetadataIndex,
    prefix: &str,
    active_profile: Option<&str>,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    let mut seen = BTreeSet::new();
    let eff = crate::parser_config::resolve_effective_properties(&model.all_config_files(), active_profile);
    for p in eff {
        if (prefix.is_empty() || p.key.to_ascii_lowercase().contains(&prefix.to_ascii_lowercase()))
            && seen.insert(p.key.clone())
        {
            items.push(CompletionItem {
                detail: Some(format!("= {} ({})", p.masked_value, p.source_profile)),
                ..CompletionItem::new(&p.key, CompletionKind::Property)
            });
        }
    }
    for m in meta_idx.complete(prefix, 40) {
        if seen.insert(m.name.clone()) {
            items.push(CompletionItem {
                detail: Some(short_java_type(&m.type_name)),
                documentation: Some(m.description.clone()),
                ..CompletionItem::new(&m.name, CompletionKind::Property)
            });
        }
    }
    items
}

fn resolve_yaml_line_key(text: &str, idx: &LineIndex, target_line: u32, leaf: &str) -> String {
    let target_start = idx.line_start_byte(target_line);
    let target_end = idx.line_end_byte(text, target_line);
    let target_indent = text[target_start..target_end]
        .bytes()
        .take_while(|b| *b == b' ')
        .count();
    let mut parts = vec![leaf.to_owned()];
    let mut cur_indent = target_indent;
    for l in (0..target_line).rev() {
        let ls = idx.line_start_byte(l);
        let le = idx.line_end_byte(text, l);
        let line = &text[ls..le];
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t == "---" {
            continue;
        }
        let ind = line.bytes().take_while(|b| *b == b' ').count();
        if ind < cur_indent
            && let Some((k, _)) = t.split_once(':')
        {
            parts.push(k.trim().to_owned());
            cur_indent = ind;
            if ind == 0 {
                break;
            }
        }
    }
    parts.reverse();
    parts.join(".")
}

fn resolve_yaml_prefix_for_completion(
    text: &str,
    idx: &LineIndex,
    target_line: u32,
    before_cursor: &str,
) -> String {
    let indent = before_cursor.bytes().take_while(|b| *b == b' ').count();
    let mut parts = Vec::new();
    let leaf = before_cursor.trim();
    if !leaf.is_empty() {
        parts.push(leaf.to_owned());
    }
    let mut cur_indent = indent;
    for l in (0..target_line).rev() {
        let ls = idx.line_start_byte(l);
        let le = idx.line_end_byte(text, l);
        let line = &text[ls..le];
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t == "---" {
            continue;
        }
        let ind = line.bytes().take_while(|b| *b == b' ').count();
        if ind < cur_indent
            && let Some((k, _)) = t.split_once(':')
        {
            parts.push(k.trim().to_owned());
            cur_indent = ind;
            if ind == 0 {
                break;
            }
        }
    }
    parts.reverse();
    parts.join(".")
}

fn short_java_type(t: &str) -> String {
    t.rsplit('.').next().unwrap_or(t).to_owned()
}

pub fn definition(
    cache: &mut ProjectCache,
    doc: &Document,
    at: Position,
    active_profile: Option<&str>,
) -> Vec<Location> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    // 1. Injection site -> resolved Bean definition(s)
    for inj in &facts.injections {
        if position_contains(inj.range, at)
            || position_contains(inj.type_range, at)
            || inj.qualifier.as_ref().is_some_and(|(_, qr)| position_contains(*qr, at))
        {
            let (_, targets) = resolve_injection(model, inj, active_profile);
            for t in targets {
                out.push(SpringProjectModel::to_location(&t.file, t.range));
            }
            if !out.is_empty() {
                return out;
            }
        }
    }

    // 2. SpEL `@beanName` -> Bean definition
    for sp in &facts.spel_sites {
        for (bname, brange) in &sp.bean_refs {
            if position_contains(*brange, at) {
                for b in model.all_beans() {
                    if b.name == *bname {
                        out.push(SpringProjectModel::to_location(&b.file, b.name_range));
                    }
                }
                if !out.is_empty() {
                    return out;
                }
            }
        }
    }

    // 3. Spring Events -> bidirectional navigation
    for ev in &facts.event_sites {
        if position_contains(ev.range, at) {
            let wanted = match ev.kind {
                EventKind::Publisher => EventKind::Listener,
                EventKind::Listener => EventKind::Publisher,
            };
            for f in model.files.values() {
                for other in &f.event_sites {
                    if other.kind == wanted && other.event_type == ev.event_type {
                        out.push(SpringProjectModel::to_location(&other.file, other.range));
                    }
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
    }

    // 4. Kotlin @Value / @ConditionalOnProperty -> config files (skipping Java @Value handled by OpenIDEA JDT)
    for v in &facts.value_sites {
        if (facts.is_kotlin || v.is_conditional_on_property)
            && position_contains(v.key_range, at)
            && let Some(key) = &v.prop_key
        {
            let rk = normalize_relaxed_key(key);
            for cf in model.all_config_files() {
                for e in &cf.entries {
                    if e.relaxed_key == rk {
                        out.push(SpringProjectModel::to_location(&cf.path, e.key_range));
                    }
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
    }

    // 5. YAML/Properties -> Kotlin @Value or @ConfigurationProperties (Java handled by OpenIDEA JDT)
    if let Some(cf) = &facts.config_file
        && let Some(entry) = config_entry_at(cf, at)
        && position_contains(entry.key_range, at)
    {
        for cls in model.all_config_prop_classes() {
            for field in &cls.fields {
                if normalize_relaxed_key(&field.full_key) == entry.relaxed_key {
                    out.push(SpringProjectModel::to_location(&cls.file, field.range));
                }
            }
        }
        for f in model.files.values() {
            if f.is_kotlin {
                for v in &f.value_sites {
                    if v.prop_key
                        .as_deref()
                        .is_some_and(|k| normalize_relaxed_key(k) == entry.relaxed_key)
                    {
                        out.push(SpringProjectModel::to_location(&v.file, v.key_range));
                    }
                }
            }
        }
    }

    out
}

pub fn line_markers(
    cache: &mut ProjectCache,
    doc: &Document,
    active_profile: Option<&str>,
) -> Vec<LineMarker> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut markers = Vec::new();
    let mut marked_lines = BTreeSet::new();

    // Only WebFlux and @HttpExchange endpoints (never duplicate OpenIDEA's MVC gutter icon!)
    for ep in &facts.endpoints {
        if matches!(ep.kind, EndpointKind::WebFlux | EndpointKind::HttpExchange)
            && marked_lines.insert(ep.line)
        {
            markers.push(LineMarker {
                line: ep.line,
                icon: Some("icons/marker_endpoint.svg".into()),
                tooltip: format!("{} · {} {}", ep.kind.framework_label(), ep.methods.join(","), ep.full_path),
                target: None,
            });
        }
    }

    for sch in &facts.scheduled_sites {
        if marked_lines.insert(sch.line) {
            let desc = sch
                .cron
                .as_deref()
                .map(|c| {
                    let ex = crate::spel_cron::parse_and_explain_cron(c);
                    if ex.valid { ex.summary_en } else { c.to_owned() }
                })
                .unwrap_or_else(|| "Scheduled Spring task".into());
            markers.push(LineMarker {
                line: sch.line,
                icon: Some("icons/marker_scheduled.svg".into()),
                tooltip: format!("@Scheduled: {desc}"),
                target: None,
            });
        }
    }

    for repo in &facts.repositories {
        if marked_lines.insert(repo.line) {
            let entity_loc = model
                .all_entities()
                .into_iter()
                .find(|e| e.name == repo.entity_type)
                .map(|e| SpringProjectModel::to_location(&e.file, e.range));
            markers.push(LineMarker {
                line: repo.line,
                icon: Some("icons/marker_data.svg".into()),
                tooltip: format!("Spring Data {}<{}>", repo.base_interface, repo.entity_type),
                target: entity_loc,
            });
        }
    }

    markers
}

pub fn code_actions(
    cache: &mut ProjectCache,
    doc: &Document,
    selection: Range,
    active_profile: Option<&str>,
) -> Vec<CodeAction> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut actions = Vec::new();

    // 1. Convert @Autowired field injection -> constructor injection
    if doc.language == "java"
        && facts
            .injections
            .iter()
            .any(|i| i.kind == crate::model::InjectionKind::Field)
        && let Some(edits) = convert_field_injection_to_constructor(&doc.text)
    {
        actions.push(CodeAction {
            title: "Spring: Convert @Autowired field injection to constructor injection".into(),
            kind: Some("refactor.rewrite".into()),
            preferred: true,
            edits: vec![(doc.path.clone(), edits)],
            data: None,
        });
    }

    // 2. Ambiguous Bean injection -> interactive @Qualifier picker
    for inj in &facts.injections {
        if inj.line == selection.start.line {
            let (status, targets) = resolve_injection(model, inj, active_profile);
            if status == WiringStatus::Ambiguous && targets.len() > 1 {
                actions.push(CodeAction {
                    title: format!("Spring: Add @Qualifier for `{}` ({} candidates)…", inj.field_or_param_name, targets.len()),
                    kind: Some("quickfix".into()),
                    preferred: true,
                    edits: Vec::new(),
                    data: Some(format!("pick-qualifier:{}", inj.line)),
                });
            }
        }
    }

    // 3. @Transactional / @Async on private method -> change to public
    for issue in &facts.proxy_issues {
        if issue.line == selection.start.line
            && issue.kind == crate::model::ProxyIssueKind::PrivateMethod
            && let Some(mod_range) = issue.modifier_range
        {
            actions.push(CodeAction {
                title: format!("Spring: Change `{}` visibility from `private` to `public`", issue.method_name),
                kind: Some("quickfix".into()),
                preferred: true,
                edits: vec![(
                    doc.path.clone(),
                    vec![TextEdit {
                        range: mod_range,
                        text: "public".into(),
                    }],
                )],
                data: None,
            });
        }
    }

    // 4. Config deprecated property replacement or fuzzy typo fix
    if let Some(cf) = &facts.config_file {
        let cp_classes = model.all_config_prop_classes();
        let meta_idx = MetadataIndex::build(&model.custom_metadata, &cp_classes);
        if let Some(entry) = config_entry_at(cf, selection.start) {
            if let Some(meta) = meta_idx.get(&entry.key)
                && meta.deprecated
                && let Some(repl) = &meta.replacement
                && doc.language == "properties"
            {
                actions.push(CodeAction {
                    title: format!("Spring Boot: Replace deprecated `{}` with `{repl}`", entry.key),
                    kind: Some("quickfix".into()),
                    preferred: true,
                    edits: vec![(
                        doc.path.clone(),
                        vec![TextEdit {
                            range: entry.key_range,
                            text: repl.clone(),
                        }],
                    )],
                    data: None,
                });
            } else if meta_idx.get(&entry.key).is_none()
                && !MetadataIndex::is_known_dynamic_prefix(&entry.key)
                && let Some(sug) = meta_idx.suggest_similar(&entry.key)
                && doc.language == "properties"
            {
                actions.push(CodeAction {
                    title: format!("Spring Boot: Replace with `{}`", sug.name),
                    kind: Some("quickfix".into()),
                    preferred: true,
                    edits: vec![(
                        doc.path.clone(),
                        vec![TextEdit {
                            range: entry.key_range,
                            text: sug.name.clone(),
                        }],
                    )],
                    data: None,
                });
            }
        }

        // Convert YAML <-> Properties
        let line_idx = LineIndex::new(&doc.text);
        let full_range = line_idx.range_from_bytes(&doc.text, 0, doc.text.len());
        if doc.language == "yaml" || doc.path.ends_with(".yml") || doc.path.ends_with(".yaml") {
            let converted = yaml_to_properties(&doc.text);
            actions.push(CodeAction {
                title: "Spring Boot: Convert YAML to .properties format".into(),
                kind: Some("refactor".into()),
                preferred: false,
                edits: vec![(
                    doc.path.clone(),
                    vec![TextEdit {
                        range: full_range,
                        text: converted,
                    }],
                )],
                data: None,
            });
        } else if doc.language == "properties" {
            let converted = properties_to_yaml(&doc.text);
            actions.push(CodeAction {
                title: "Spring Boot: Convert .properties to YAML format".into(),
                kind: Some("refactor".into()),
                preferred: false,
                edits: vec![(
                    doc.path.clone(),
                    vec![TextEdit {
                        range: full_range,
                        text: converted,
                    }],
                )],
                data: None,
            });
        }
    }

    actions
}

pub fn resolve_code_action(
    cache: &mut ProjectCache,
    doc: &Document,
    mut action: CodeAction,
    active_profile: Option<&str>,
) -> Result<CodeAction, Error> {
    let Some(data) = action.data.as_deref() else {
        return Ok(action);
    };
    if let Some(line_str) = data.strip_prefix("pick-qualifier:")
        && let Ok(target_line) = line_str.parse::<u32>()
    {
        let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
        let Some(facts) = model.files.get(&doc.path) else {
            return Err(Error::Cancelled);
        };
        let Some(inj) = facts.injections.iter().find(|i| i.line == target_line) else {
            return Err(Error::Cancelled);
        };
        let (_, targets) = resolve_injection(model, inj, active_profile);
        if targets.is_empty() {
            return Err(Error::Cancelled);
        }
        let items: Vec<PickItem> = targets
            .iter()
            .map(|t| PickItem::new(&t.bean_name, &t.bean_name).description(&t.class_name))
            .collect();
        let chosen = host::pick("Select Bean Qualifier", &items)?;
        let Some(bean_name) = chosen else {
            return Err(Error::Cancelled);
        };
        let line_idx = LineIndex::new(&doc.text);
        let ls = line_idx.line_start_byte(target_line);
        let indent: String = doc.text[ls..]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let insert_pos = line_idx.byte_to_pos(&doc.text, ls);
        action.edits = vec![(
            doc.path.clone(),
            vec![TextEdit {
                range: Range::new(insert_pos, insert_pos),
                text: format!("{indent}@Qualifier(\"{bean_name}\")\n"),
            }],
        )];
    }
    Ok(action)
}
