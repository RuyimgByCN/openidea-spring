use crate::boot_metadata::MetadataIndex;
use crate::data_jpa::parse_derived_query;
use crate::endpoints::{join_context_path, resolve_base_url};
use crate::index::{BeanLookupIndex, ProjectCache, resolve_injection};
use crate::model::{EndpointKind, SpringProjectModel, WiringStatus};
use crate::parser_config::{config_entry_at, resolve_effective_properties};
use crate::spel_cron::{parse_and_explain_cron, validate_spel};
use crate::text_util::{normalize_relaxed_key, position_contains};
use openidea_plugin::hover::{Document, Hover};
use openidea_plugin::inlay_hints::{InlayHint, InlayKind};
use openidea_plugin::{Position, Range};

#[derive(Clone, Copy, Debug)]
pub struct InlayPrefs {
    pub enabled: bool,
    pub value_hints: bool,
    pub bean_hints: bool,
    pub endpoint_hints: bool,
    pub cron_hints: bool,
}

impl Default for InlayPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            value_hints: true,
            bean_hints: true,
            endpoint_hints: true,
            cron_hints: true,
        }
    }
}

pub fn hover(
    cache: &mut ProjectCache,
    doc: &Document,
    at: Position,
    active_profile: Option<&str>,
    is_zh: bool,
) -> Option<Hover> {
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let facts = model.files.get(&doc.path)?;
    let eff = resolve_effective_properties(&model.all_config_files(), active_profile);
    let cp_classes = model.all_config_prop_classes();
    let meta_idx = MetadataIndex::build(&model.custom_metadata, &cp_classes);

    // 1. Config file hover: rich Spring Boot documentation + effective values across profiles
    if let Some(cf) = &facts.config_file
        && let Some(entry) = config_entry_at(cf, at)
    {
        for ph in &entry.placeholders {
            if position_contains(ph.full_range, at) {
                let rk = normalize_relaxed_key(&ph.key);
                let resolved = eff
                    .iter()
                    .find(|p| p.relaxed_key == rk)
                    .map(|p| format!("`{}` ({})", p.masked_value, p.source_profile))
                    .or_else(|| ph.default_value.as_ref().map(|d| format!("`{d}` (default)")))
                    .unwrap_or_else(|| "*unresolved*".into());
                return Some(
                    Hover::new(format!("**Spring Placeholder** `${{{}}}`\n\nEffective value: {resolved}", ph.key))
                        .range(ph.full_range),
                );
            }
        }
        let mut md = format!("**Spring Boot Property** `{}`\n\n", entry.key);
        if let Some(meta) = meta_idx.get(&entry.key) {
            md.push_str(&format!("Type: `{}`\n\n", meta.type_name));
            if let Some(def) = &meta.default_value {
                md.push_str(&format!("Default: `{def}`\n\n"));
            }
            if !meta.description.is_empty() {
                md.push_str(&format!("{}\n\n", meta.description));
            }
            if meta.deprecated {
                let repl = meta
                    .replacement
                    .as_deref()
                    .map(|r| format!(" → replace with `{r}`"))
                    .unwrap_or_default();
                md.push_str(&format!("> **Deprecated**{repl}\n\n"));
            }
        }
        if let Some(e) = eff.iter().find(|p| p.relaxed_key == entry.relaxed_key) {
            md.push_str(&format!(
                "Active (`{}`): `{}`",
                active_profile.unwrap_or("default"),
                e.masked_value
            ));
            if !e.overridden_from.is_empty() {
                md.push_str(&format!(" *(overrides {})*", e.overridden_from.join(", ")));
            }
        }
        return Some(Hover::new(md).range(entry.key_range));
    }

    // 2. @Scheduled Cron hover
    for sch in &facts.scheduled_sites {
        if let (Some(cron), Some(crange)) = (&sch.cron, sch.cron_range)
            && position_contains(crange, at)
        {
            let ex = parse_and_explain_cron(cron);
            if ex.valid {
                let summary = if is_zh { &ex.summary_zh } else { &ex.summary_en };
                let fields_md: Vec<String> = ex
                    .fields
                    .iter()
                    .map(|(k, v)| format!("- **{k}**: `{v}`"))
                    .collect();
                return Some(
                    Hover::new(format!(
                        "**Spring Cron** `{cron}`\n\n{summary}\n\n{}",
                        fields_md.join("\n")
                    ))
                    .range(crange),
                );
            } else if let Some(err) = ex.error {
                return Some(Hover::new(format!("**Invalid Spring Cron**: {err}")).range(crange));
            }
        }
    }

    // 3. SpEL hover
    for sp in &facts.spel_sites {
        if position_contains(sp.range, at) {
            let v = validate_spel(&sp.expr);
            let msg = if v.valid {
                format!("**Spring Expression Language (SpEL)**\n\n`{}`\n\n{}", sp.expr, v.summary)
            } else {
                format!("**Invalid SpEL**: {}", v.error.unwrap_or_default())
            };
            return Some(Hover::new(msg).range(sp.range));
        }
    }

    // 4. @Value hover
    for v in &facts.value_sites {
        if position_contains(v.full_range, at)
            && let Some(key) = &v.prop_key
        {
            let rk = normalize_relaxed_key(key);
            let resolved = eff
                .iter()
                .find(|p| p.relaxed_key == rk)
                .map(|p| format!("`{}` (from `{}` [{}])", p.masked_value, p.source_file, p.source_profile))
                .or_else(|| v.default_value.as_ref().map(|d| format!("`{d}` (default)")))
                .unwrap_or_else(|| "*not defined in active profile*".into());
            let doc_desc = meta_idx
                .get(key)
                .map(|m| format!("\n\n{}", m.description))
                .unwrap_or_default();
            return Some(
                Hover::new(format!(
                    "**Spring Property** `{key}`\n\nEffective (`{}`): {resolved}{doc_desc}",
                    active_profile.unwrap_or("default")
                ))
                .range(v.full_range),
            );
        }
    }

    // 5. Injection site hover
    for inj in &facts.injections {
        if position_contains(inj.range, at) || position_contains(inj.type_range, at) {
            let (status, targets) = resolve_injection(model, inj, active_profile);
            let target_list: Vec<String> = targets
                .iter()
                .map(|t| {
                    format!(
                        "- `{}` (`{}`){}",
                        t.bean_name,
                        t.class_name,
                        if t.is_primary { " **[Primary]**" } else { "" }
                    )
                })
                .collect();
            let md = match status {
                WiringStatus::Resolved => format!(
                    "**Spring Autowired Dependency**\n\nTarget bean:\n{}",
                    target_list.join("\n")
                ),
                WiringStatus::Ambiguous => format!(
                    "**Ambiguous Spring Dependency** ({} candidates)\n\n{}",
                    targets.len(),
                    target_list.join("\n")
                ),
                WiringStatus::Missing => {
                    format!("**Unresolved Spring Dependency**: no bean matching `{}`", inj.target_type)
                }
                WiringStatus::OptionalMissing => {
                    format!("**Optional Spring Dependency**: `{}` (no candidates)", inj.target_type)
                }
            };
            return Some(Hover::new(md).range(inj.type_range));
        }
    }

    // 6. Bean class / @Bean method hover
    for b in &facts.beans {
        if position_contains(b.name_range, at) {
            let inj_count = model
                .all_injections()
                .into_iter()
                .filter(|i| {
                    let (_, ts) = resolve_injection(model, i, active_profile);
                    ts.iter().any(|t| t.bean_id == b.id)
                })
                .count();
            let mut md = format!("**Spring Bean** `{}` · {}\n\n- Class: `{}`\n- Scope: `{}`\n- Injected by: {} site(s)", b.name, b.stereotype.label(), b.class_fqn, b.scope, inj_count);
            if b.is_primary {
                md.push_str("\n- `@Primary`");
            }
            if !b.profiles.is_empty() {
                md.push_str(&format!("\n- Profiles: `{}`", b.profiles.join(", ")));
            }
            if !b.conditions.is_empty() {
                md.push_str(&format!("\n- Conditions: `{}`", b.conditions.join("`, `")));
            }
            return Some(Hover::new(md).range(b.name_range));
        }
    }

    // 7. Endpoint hover (resolved full URL)
    let (base_host, ctx_path) = resolve_base_url(model, active_profile);
    for ep in &facts.endpoints {
        if position_contains(ep.range, at) {
            let full = join_context_path(&ctx_path, &ep.full_path);
            let url = format!("{base_host}{full}");
            return Some(
                Hover::new(format!(
                    "**{} Endpoint**\n\n`{} {full}`\n\nResolved URL: `{url}`",
                    ep.kind.framework_label(),
                    ep.methods.join(", ")
                ))
                .range(ep.range),
            );
        }
    }

    // 8. Spring Data Derived Query hover (JPQL preview!)
    for repo in &facts.repositories {
        for m in &repo.methods {
            if position_contains(m.name_range, at)
                && let Some(ast) = parse_derived_query(&m.name)
            {
                let jpql = ast.to_jpql_preview(&repo.entity_type);
                return Some(
                    Hover::new(format!(
                        "**Spring Data Derived Query** (`{}`)\n\n```sql\n{jpql}\n```",
                        repo.entity_type
                    ))
                    .range(m.name_range),
                );
            }
        }
    }

    None
}

pub fn inlay_hints(
    cache: &mut ProjectCache,
    doc: &Document,
    range: Range,
    active_profile: Option<&str>,
    prefs: InlayPrefs,
    is_zh: bool,
) -> Vec<InlayHint> {
    if !prefs.enabled {
        return Vec::new();
    }
    let model = cache.model_for_doc(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut hints = Vec::new();

    if prefs.value_hints && !facts.value_sites.is_empty() {
        let eff = resolve_effective_properties(&model.all_config_files(), active_profile);
        for v in &facts.value_sites {
            if v.line < range.start.line || v.line > range.end.line {
                continue;
            }
            if let Some(key) = &v.prop_key {
                let rk = normalize_relaxed_key(key);
                let val = eff
                    .iter()
                    .find(|p| p.relaxed_key == rk)
                    .map(|p| format!("= {} [{}]", p.masked_value, p.source_profile))
                    .or_else(|| v.default_value.as_ref().map(|d| format!("= {d} [default]")));
                if let Some(label) = val {
                    hints.push(
                        InlayHint::new(v.full_range.end, label)
                            .kind(InlayKind::Other)
                            .tooltip(format!("Resolved value of `{key}`"))
                            .padding_left(),
                    );
                }
            }
        }
    }

    if prefs.bean_hints && !facts.injections.is_empty() {
        let bean_idx = BeanLookupIndex::build(model, active_profile);
        if bean_idx.has_beans() {
            for inj in &facts.injections {
                if inj.line < range.start.line || inj.line > range.end.line {
                    continue;
                }
                let (status, targets) = bean_idx.resolve(inj);
                if status == WiringStatus::Resolved && let Some(t) = targets.first()
                    && t.class_name != inj.target_type
                {
                    hints.push(
                        InlayHint::new(inj.range.end, format!("→ {}", t.class_name))
                            .kind(InlayKind::Type)
                            .tooltip(format!("Injected bean `{}` ({})", t.bean_name, t.class_name))
                            .padding_left(),
                    );
                }
            }
        }
    }

    if prefs.endpoint_hints {
        let (_, ctx_path) = resolve_base_url(model, active_profile);
        for ep in &facts.endpoints {
            if ep.line < range.start.line || ep.line > range.end.line {
                continue;
            }
            if ep.kind == EndpointKind::FeignClient {
                continue;
            }
            let full = join_context_path(&ctx_path, &ep.full_path);
            if full != ep.raw_path {
                hints.push(
                    InlayHint::new(ep.range.end, format!("{} {full}", ep.methods.join("|")))
                        .kind(InlayKind::Other)
                        .tooltip("Full resolved Spring endpoint path")
                        .padding_left(),
                );
            }
        }
    }

    if prefs.cron_hints {
        for sch in &facts.scheduled_sites {
            if sch.line < range.start.line || sch.line > range.end.line {
                continue;
            }
            if let (Some(cron), Some(crange)) = (&sch.cron, sch.cron_range) {
                let ex = parse_and_explain_cron(cron);
                if ex.valid {
                    let s = if is_zh { ex.summary_zh } else { ex.summary_en };
                    hints.push(
                        InlayHint::new(crange.end, format!("⏱ {s}"))
                            .kind(InlayKind::Other)
                            .tooltip(format!("Spring Cron `{cron}`"))
                            .padding_left(),
                    );
                }
            }
        }
    }

    let _ = SpringProjectModel::to_location;
    hints
}
