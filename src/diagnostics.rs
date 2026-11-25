use crate::boot_metadata::{MetadataIndex, validate_property_value};
use crate::data_jpa::{parse_derived_query, validate_repository_method};
use crate::index::{BeanLookupIndex, ProjectCache};
use crate::model::{EndpointKind, WiringStatus};
use crate::parser_config::resolve_effective_properties;
use crate::spel_cron::{parse_and_explain_cron, validate_spel};
use openidea_plugin::diagnostics::{Diagnostic, Document};
use openidea_plugin::{RelatedInformation, Severity};
use std::collections::{BTreeSet, HashMap};

pub fn compute_diagnostics(
    cache: &mut ProjectCache,
    doc: &Document,
    active_profile: Option<&str>,
    diagnostics_enabled: bool,
    warn_field_injection: bool,
    warn_unknown_properties: bool,
) -> Vec<Diagnostic> {
    if !diagnostics_enabled {
        return Vec::new();
    }
    let model = cache.model_for_doc_with_graph(&doc.path, &doc.language, &doc.text, active_profile);
    let Some(facts) = model.files.get(&doc.path) else {
        return Vec::new();
    };
    let mut diags = Vec::new();

    // 1. Core DI checks
    let project_has_multiple_files = model.files.len() > 1;
    let bean_idx = if !facts.injections.is_empty() || !facts.beans.is_empty() {
        Some(BeanLookupIndex::build(model, active_profile))
    } else {
        None
    };
    let any_spring_beans = bean_idx.as_ref().is_some_and(BeanLookupIndex::has_beans);

    for inj in &facts.injections {
        if warn_field_injection && inj.kind == crate::model::InjectionKind::Field {
            let mut d = Diagnostic::new(
                inj.range,
                Severity::Hint,
                format!(
                    "Field injection for `{}` is discouraged; prefer constructor injection.",
                    inj.field_or_param_name
                ),
            )
            .code("SPRING_FIELD_INJECTION");
            d.source = Some("spring".into());
            diags.push(d);
        }

        if any_spring_beans && let Some(bidx) = &bean_idx {
            let (status, targets) = bidx.resolve(inj);
            match status {
                WiringStatus::Missing if project_has_multiple_files && !is_framework_type(&inj.target_type) => {
                    let mut d = Diagnostic::new(
                        inj.type_range,
                        Severity::Warning,
                        format!(
                            "Could not autowire. No bean of `{}` type found in project.",
                            inj.target_type
                        ),
                    )
                    .code("SPRING_MISSING_BEAN");
                    d.source = Some("spring".into());
                    diags.push(d);
                }
                WiringStatus::Ambiguous => {
                    let names: Vec<String> = targets.iter().map(|t| t.bean_name.clone()).collect();
                    let mut d = Diagnostic::new(
                        inj.type_range,
                        Severity::Warning,
                        format!(
                            "Could not autowire `{}`: found {} candidate beans ({}). Add @Primary or @Qualifier.",
                            inj.target_type,
                            targets.len(),
                            names.join(", ")
                        ),
                    )
                    .code("SPRING_AMBIGUOUS_BEAN");
                    d.source = Some("spring".into());
                    d.related = targets
                        .iter()
                        .map(|t| RelatedInformation {
                            location: crate::model::SpringProjectModel::to_location(&t.file, t.range),
                            message: format!("Candidate bean `{}`", t.bean_name),
                        })
                        .collect();
                    diags.push(d);
                }
                _ => {}
            }
        }
    }

    // Duplicate @Bean method names
    for (bname, brange) in &facts.duplicate_bean_methods {
        let mut d = Diagnostic::new(
            *brange,
            Severity::Error,
            format!("Duplicate `@Bean` name `{bname}` in the same @Configuration class."),
        )
        .code("SPRING_DUPLICATE_BEAN_NAME");
        d.source = Some("spring".into());
        diags.push(d);
    }

    // Circular dependencies touching beans in this file
    for b in &facts.beans {
        if let Some(cyc) = model.cycles.iter().find(|c| c.contains(&b.id)) {
            let readable: Vec<String> = cyc
                .iter()
                .map(|id| {
                    bean_idx
                        .as_ref()
                        .and_then(|bidx| bidx.bean_by_id(id))
                        .map_or_else(|| id.clone(), |x| x.class_name.clone())
                })
                .collect();
            let mut d = Diagnostic::new(
                b.name_range,
                Severity::Warning,
                format!("Circular Spring bean dependency detected: {} -> {}", readable.join(" -> "), readable[0]),
            )
            .code("SPRING_CIRCULAR_DEPENDENCY");
            d.source = Some("spring".into());
            diags.push(d);
        }
    }

    // @Transactional / @Async / proxyBeanMethods = false issues
    for issue in &facts.proxy_issues {
        let (code, msg) = match issue.kind {
            crate::model::ProxyIssueKind::PrivateMethod => (
                "SPRING_PROXY_PRIVATE",
                format!(
                    "`{}` on private method `{}` will be ignored by Spring AOP proxy.",
                    issue.annotation, issue.method_name
                ),
            ),
            crate::model::ProxyIssueKind::FinalMethod => (
                "SPRING_PROXY_FINAL",
                format!(
                    "`{}` on final method/class `{}` prevents CGLIB proxy interception.",
                    issue.annotation, issue.method_name
                ),
            ),
            crate::model::ProxyIssueKind::SelfInvocation => (
                "SPRING_SELF_INVOCATION",
                format!(
                    "Direct self-invocation of `{}` method `{}` bypasses the Spring proxy.",
                    issue.annotation, issue.method_name
                ),
            ),
            crate::model::ProxyIssueKind::ProxyBeanMethodsFalseCall => (
                "SPRING_PROXY_BEAN_METHODS_FALSE",
                format!(
                    "Direct `@Bean` method call `{}` inside `@Configuration(proxyBeanMethods = false)` creates a new instance instead of returning the singleton bean.",
                    issue.method_name
                ),
            ),
        };
        let mut d = Diagnostic::new(issue.range, Severity::Warning, msg).code(code);
        d.source = Some("spring".into());
        diags.push(d);
    }

    // 2. Web inspections
    let mut route_counts: HashMap<(String, String), usize> = HashMap::new();
    if facts.endpoints.iter().any(|e| e.kind != EndpointKind::FeignClient) {
        for ep in model.all_endpoints() {
            if ep.kind != EndpointKind::FeignClient {
                for m in &ep.methods {
                    *route_counts.entry((m.clone(), ep.full_path.clone())).or_insert(0) += 1;
                }
            }
        }
    }

    for ep in &facts.endpoints {
        if ep.kind == EndpointKind::FeignClient {
            continue;
        }
        for m in &ep.methods {
            if route_counts.get(&(m.clone(), ep.full_path.clone())).copied().unwrap_or(0) > 1 {
                let mut d = Diagnostic::new(
                    ep.range,
                    Severity::Warning,
                    format!("Ambiguous mapping: `{m} {}` is declared multiple times across controllers.", ep.full_path),
                )
                .code("SPRING_DUPLICATE_ROUTE");
                d.source = Some("spring".into());
                diags.push(d);
            }
            if ep.has_request_body && matches!(m.as_str(), "GET" | "HEAD") {
                let mut d = Diagnostic::new(
                    ep.range,
                    Severity::Warning,
                    format!("`@RequestBody` on HTTP `{m}` endpoint `{}` is non-standard and may fail in HTTP clients.", ep.full_path),
                )
                .code("SPRING_GET_REQUEST_BODY");
                d.source = Some("spring".into());
                diags.push(d);
            }
        }

        // Validate @PathVariable names vs `{var}` placeholders
        let mut placeholders = BTreeSet::new();
        for seg in ep.full_path.split('/') {
            if let Some(inner) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                let clean = inner.split(':').next().unwrap_or(inner).trim();
                if !clean.is_empty() {
                    placeholders.insert(clean.to_owned());
                }
            }
        }
        for pv in &ep.path_vars {
            if !placeholders.contains(&pv.var_name) {
                let mut d = Diagnostic::new(
                    pv.range,
                    Severity::Warning,
                    format!(
                        "`@PathVariable(\"{}\")` does not match any `{{...}}` placeholder in `{}`.",
                        pv.var_name, ep.full_path
                    ),
                )
                .code("SPRING_UNKNOWN_PATH_VARIABLE");
                d.source = Some("spring".into());
                diags.push(d);
            }
        }
    }

    // 3. Spring Boot Configuration diagnostics
    if let Some(cf) = &facts.config_file {
        let cp_classes = model.all_config_prop_classes();
        let meta_idx = MetadataIndex::build(&model.custom_metadata, &cp_classes);
        let eff = resolve_effective_properties(&model.all_config_files(), active_profile);
        let defined_keys: BTreeSet<String> = eff.iter().map(|p| p.relaxed_key.clone()).collect();

        for (dup_key, dup_range, _) in &cf.duplicate_keys {
            let mut d = Diagnostic::new(
                *dup_range,
                Severity::Warning,
                format!("Duplicate configuration key `{dup_key}` in the same document."),
            )
            .code("SPRING_DUPLICATE_PROPERTY_KEY");
            d.source = Some("spring".into());
            diags.push(d);
        }

        for entry in &cf.entries {
            if let Some(meta) = meta_idx.get(&entry.key) {
                if meta.deprecated {
                    let repl = meta
                        .replacement
                        .as_deref()
                        .map(|r| format!(" Use `{r}` instead."))
                        .unwrap_or_default();
                    let mut d = Diagnostic::new(
                        entry.key_range,
                        Severity::Warning,
                        format!("Deprecated Spring Boot property `{}`.{repl}", entry.key),
                    )
                    .code("SPRING_DEPRECATED_PROPERTY");
                    d.source = Some("spring".into());
                    d.deprecated = true;
                    diags.push(d);
                }
                if let Some(val_err) = validate_property_value(meta, &entry.value) {
                    let mut d = Diagnostic::new(entry.value_range, Severity::Error, val_err)
                        .code("SPRING_INVALID_PROPERTY_VALUE");
                    d.source = Some("spring".into());
                    diags.push(d);
                }
            } else if warn_unknown_properties
                && !MetadataIndex::is_known_dynamic_prefix(&entry.key)
                && !entry.key.starts_with("app.")
                && !entry.key.starts_with("custom.")
            {
                let sug = meta_idx
                    .suggest_similar(&entry.key)
                    .map(|m| format!(" Did you mean `{}`?", m.name))
                    .unwrap_or_default();
                let mut d = Diagnostic::new(
                    entry.key_range,
                    Severity::Hint,
                    format!("Unknown Spring Boot configuration property `{}`.{sug}", entry.key),
                )
                .code("SPRING_UNKNOWN_PROPERTY");
                d.source = Some("spring".into());
                diags.push(d);
            }

            for ph in &entry.placeholders {
                let rk = crate::text_util::normalize_relaxed_key(&ph.key);
                if ph.default_value.is_none()
                    && !defined_keys.contains(&rk)
                    && !is_common_env_var(&ph.key)
                {
                    let mut d = Diagnostic::new(
                        ph.full_range,
                        Severity::Info,
                        format!("Placeholder `${{{}}}` has no default value and is not defined in indexed config files.", ph.key),
                    )
                    .code("SPRING_UNRESOLVED_PLACEHOLDER");
                    d.source = Some("spring".into());
                    diags.push(d);
                }
            }
        }
    }

    // 4. Spring Data JPA / @Query diagnostics
    let entities = model.all_entities();
    for repo in &facts.repositories {
        for m in &repo.methods {
            let errs = validate_repository_method(repo, m, &entities);
            for err in errs {
                let mut d = Diagnostic::new(m.name_range, Severity::Warning, err)
                    .code("SPRING_DATA_QUERY");
                d.source = Some("spring".into());
                diags.push(d);
            }
            if m.query.is_none() && parse_derived_query(&m.name).is_none() && m.name.starts_with("find") && !m.name.contains("By") {
                let mut d = Diagnostic::new(
                    m.name_range,
                    Severity::Info,
                    format!("Derived method `{}` is missing `By` predicate clause.", m.name),
                )
                .code("SPRING_DATA_DERIVED_SYNTAX");
                d.source = Some("spring".into());
                diags.push(d);
            }
        }
    }

    // 5. Cron & SpEL diagnostics
    for sch in &facts.scheduled_sites {
        if let (Some(cron), Some(crange)) = (&sch.cron, sch.cron_range)
            && !cron.contains("${")
        {
            let res = parse_and_explain_cron(cron);
            if !res.valid
                && let Some(err) = res.error
            {
                let mut d = Diagnostic::new(crange, Severity::Error, err)
                    .code("SPRING_INVALID_CRON");
                d.source = Some("spring".into());
                diags.push(d);
            }
        }
    }

    for sp in &facts.spel_sites {
        let res = validate_spel(&sp.expr);
        if !res.valid
            && let Some(err) = res.error
        {
            let mut d = Diagnostic::new(sp.range, Severity::Error, err)
                .code("SPRING_INVALID_SPEL");
            d.source = Some("spring".into());
            diags.push(d);
        }
    }

    diags
}

fn is_framework_type(ty: &str) -> bool {
    matches!(
        ty,
        "ApplicationContext"
            | "ConfigurableApplicationContext"
            | "ApplicationEventPublisher"
            | "Environment"
            | "ResourceLoader"
            | "MessageSource"
            | "ObjectMapper"
            | "JdbcTemplate"
            | "NamedParameterJdbcTemplate"
            | "EntityManager"
            | "RestTemplate"
            | "WebClient"
            | "RestClient"
            | "StringRedisTemplate"
            | "RedisTemplate"
            | "KafkaTemplate"
            | "RabbitTemplate"
            | "DataSource"
            | "PlatformTransactionManager"
            | "PasswordEncoder"
            | "AuthenticationManager"
            | "MeterRegistry"
            | "Tracer"
            | "CacheManager"
    )
}

fn is_common_env_var(key: &str) -> bool {
    key.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}
