use crate::data_jpa::extract_query_params;
use crate::model::{
    BeanDef, BeanStereotype, ConfigPropClass, ConfigPropField, EndpointDef, EndpointKind, EntityDef,
    EntityField, EventKind, EventSite, FileFacts, InjectionKind, InjectionSite, LinkTargetSite,
    PathVarParam, ProfileRefSite, ProxyIssueKind, ProxyIssueSite, QuerySite, RepositoryDef,
    RepositoryMethod, ScheduledSite, SpelSite, ValueExprSite,
};
use crate::parser_xml::extract_placeholders;
use crate::text_util::{
    LineIndex, simple_type_name, to_kebab_case, to_lower_camel_case, unwrap_container_type,
};
use openidea_plugin::{Range, TextEdit};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
struct ParsedAnnotation {
    name: String,
    raw_args: String,
    args_byte_start: usize,
    start_byte: usize,
    end_byte: usize,
}

#[derive(Clone, Debug)]
struct ParsedParam {
    annotations: Vec<ParsedAnnotation>,
    name: String,
    type_name: String,
    type_range: Range,
    range: Range,
}

#[derive(Clone, Debug)]
struct ClassHeader {
    annotations: Vec<ParsedAnnotation>,
    kind: String, // class, interface, record, enum
    name: String,
    name_start: usize,
    name_end: usize,
    decl_start: usize,
    body_start: usize,
    body_end: usize,
    extends_list: Vec<String>,
    implements_list: Vec<String>,
    primary_ctor_params: Vec<ParsedParam>,
    is_final: bool,
}

#[derive(Clone, Debug)]
struct MemberDecl {
    annotations: Vec<ParsedAnnotation>,
    is_field: bool,
    is_ctor: bool,
    is_private: bool,
    is_final: bool,
    is_static: bool,
    return_or_field_type: String,
    type_range: Range,
    name: String,
    name_start: usize,
    name_end: usize,
    decl_start: usize,
    decl_end: usize,
    modifier_private_or_final_range: Option<Range>,
    params: Vec<ParsedParam>,
    body_range_bytes: Option<(usize, usize)>,
}

pub fn parse_jvm_source(path: &str, language: &str, text: &str) -> FileFacts {
    let is_kotlin = language == "kotlin" || path.ends_with(".kt") || path.ends_with(".kts");
    let mut facts = FileFacts {
        path: path.to_owned(),
        language: if is_kotlin { "kotlin" } else { "java" }.to_owned(),
        is_kotlin,
        ..FileFacts::default()
    };
    let idx = LineIndex::new(text);
    let pkg = extract_package(text);
    facts.package = pkg.clone();
    let imports = extract_imports(text);

    let classes = find_top_classes(text, is_kotlin, &idx);
    for cls in &classes {
        process_class(path, text, &idx, is_kotlin, &pkg, &imports, cls, &mut facts);
    }

    // Also scan whole file for WebFlux functional routes (`RouterFunctions.route()` / Kotlin `router {`)
    // and standalone `publishEvent(...)` if outside processed methods.
    scan_webflux_routes(path, text, &idx, &mut facts);

    facts
}

fn extract_package(text: &str) -> String {
    for line in text.lines().take(40) {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("package ") {
            let pkg = rest.trim_end_matches(';').trim();
            if !pkg.is_empty() && pkg.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
                return pkg.to_owned();
            }
        }
    }
    String::new()
}

fn extract_imports(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines().take(200) {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("import ") {
            let rest = rest.strip_prefix("static ").unwrap_or(rest);
            let fqn = rest.trim_end_matches(';').trim();
            if let Some(simple) = fqn.rsplit('.').next()
                && simple != "*"
            {
                map.insert(simple.to_owned(), fqn.to_owned());
            }
        }
    }
    map
}

fn resolve_fqn(package: &str, imports: &BTreeMap<String, String>, name: &str) -> String {
    if name.contains('.') {
        return name.to_owned();
    }
    if let Some(fqn) = imports.get(name) {
        return fqn.clone();
    }
    if package.is_empty() {
        name.to_owned()
    } else {
        format!("{package}.{name}")
    }
}

fn process_class(
    path: &str,
    text: &str,
    idx: &LineIndex,
    is_kotlin: bool,
    package: &str,
    imports: &BTreeMap<String, String>,
    cls: &ClassHeader,
    facts: &mut FileFacts,
) {
    let class_fqn = resolve_fqn(package, imports, &cls.name);
    let stereotype = detect_stereotype(&cls.annotations);
    let is_primary = has_ann(&cls.annotations, "Primary");
    let is_lazy = has_ann(&cls.annotations, "Lazy");
    let scope = extract_scope(&cls.annotations);
    let qualifiers = extract_qualifiers(text, idx, &cls.annotations);
    let profiles = extract_profiles(path, text, idx, &cls.annotations, &mut facts.profile_refs);
    let conditions = extract_conditions(&cls.annotations);
    let proxy_bean_methods = extract_proxy_bean_methods(&cls.annotations);

    // Extract @ConditionalOnProperty value sites & @PropertySource links on class annotations
    for ann in &cls.annotations {
        extract_ann_value_and_links(path, text, idx, ann, facts);
    }

    let mut class_bean_name = to_lower_camel_case(&cls.name);
    if let Some(st) = stereotype {
        if let Some(explicit) = extract_bean_name_from_stereotype(&cls.annotations, st) {
            class_bean_name = explicit;
        }
        let name_range = idx.range_from_bytes(text, cls.name_start, cls.name_end);
        let decl_range = idx.range_from_bytes(text, cls.decl_start, cls.name_end);
        facts.beans.push(BeanDef {
            id: format!("{path}#{}#{}", cls.name, name_range.start.line),
            name: class_bean_name.clone(),
            class_name: cls.name.clone(),
            class_fqn: class_fqn.clone(),
            package: package.to_owned(),
            stereotype: st,
            scope: scope.clone(),
            is_primary,
            is_lazy,
            proxy_bean_methods,
            qualifiers: qualifiers.clone(),
            profiles: profiles.clone(),
            conditions: conditions.clone(),
            interfaces: cls.implements_list.iter().map(|s| simple_type_name(s)).collect(),
            superclass: cls.extends_list.first().map(|s| simple_type_name(s)),
            factory_bean: None,
            file: path.to_owned(),
            name_range,
            decl_range,
            line: name_range.start.line,
        });
    }

    let members = parse_members(text, cls.body_start, cls.body_end, &cls.name, is_kotlin, idx);

    // @ConfigurationProperties
    if let Some(cp_ann) = find_ann(&cls.annotations, "ConfigurationProperties") {
        let prefix = extract_string_attr(cp_ann, &["prefix", "value"]).unwrap_or_default();
        let mut fields = Vec::new();
        for p in &cls.primary_ctor_params {
            let kebab = to_kebab_case(&p.name);
            let full_key = if prefix.is_empty() {
                kebab.clone()
            } else {
                format!("{prefix}.{kebab}")
            };
            fields.push(ConfigPropField {
                name: p.name.clone(),
                kebab_name: kebab,
                full_key,
                type_name: p.type_name.clone(),
                default_value: None,
                range: p.range,
            });
        }
        for m in &members {
            if m.is_field && !m.is_static {
                let kebab = to_kebab_case(&m.name);
                let full_key = if prefix.is_empty() {
                    kebab.clone()
                } else {
                    format!("{prefix}.{kebab}")
                };
                fields.push(ConfigPropField {
                    name: m.name.clone(),
                    kebab_name: kebab,
                    full_key,
                    type_name: m.return_or_field_type.clone(),
                    default_value: None,
                    range: idx.range_from_bytes(text, m.name_start, m.name_end),
                });
            }
        }
        facts.config_prop_classes.push(ConfigPropClass {
            prefix,
            class_name: cls.name.clone(),
            file: path.to_owned(),
            range: idx.range_from_bytes(text, cls.name_start, cls.name_end),
            fields,
        });
    }

    // @Entity
    if has_ann(&cls.annotations, "Entity") {
        let table_name = find_ann(&cls.annotations, "Table")
            .and_then(|a| extract_string_attr(a, &["name", "value"]))
            .unwrap_or_else(|| to_kebab_case(&cls.name).replace('-', "_"));
        let mut efields = Vec::new();
        for p in &cls.primary_ctor_params {
            efields.push(EntityField {
                name: p.name.clone(),
                type_name: p.type_name.clone(),
                column_name: to_kebab_case(&p.name).replace('-', "_"),
                is_id: has_ann(&p.annotations, "Id") || has_ann(&p.annotations, "EmbeddedId"),
                range: p.range,
            });
        }
        for m in &members {
            if m.is_field && !m.is_static {
                let col = find_ann(&m.annotations, "Column")
                    .and_then(|a| extract_string_attr(a, &["name"]))
                    .unwrap_or_else(|| to_kebab_case(&m.name).replace('-', "_"));
                efields.push(EntityField {
                    name: m.name.clone(),
                    type_name: m.return_or_field_type.clone(),
                    column_name: col,
                    is_id: has_ann(&m.annotations, "Id") || has_ann(&m.annotations, "EmbeddedId"),
                    range: idx.range_from_bytes(text, m.name_start, m.name_end),
                });
            }
        }
        facts.entities.push(EntityDef {
            name: cls.name.clone(),
            fqn: class_fqn.clone(),
            table_name,
            superclass: cls.extends_list.first().map(|s| simple_type_name(s)),
            fields: efields,
            file: path.to_owned(),
            range: idx.range_from_bytes(text, cls.name_start, cls.name_end),
        });
    }

    // Spring Data Repository
    if cls.kind == "interface"
        && let Some((base_repo, entity_t, id_t)) = detect_repository_super(&cls.extends_list, &cls.implements_list)
    {
        let mut repo_methods = Vec::new();
        for m in &members {
            if m.is_field || m.is_ctor {
                continue;
            }
            let explicit_params: Vec<(String, Range)> = m
                .params
                .iter()
                .filter_map(|p| {
                    find_ann(&p.annotations, "Param").and_then(|a| {
                        extract_string_attr_with_range(text, idx, a, &["value"])
                    })
                })
                .collect();
            let query = find_ann(&m.annotations, "Query")
                .or_else(|| find_ann(&m.annotations, "NativeQuery"))
                .and_then(|a| {
                    let (qtext, qrange) = extract_string_attr_with_range(text, idx, a, &["value"])?;
                    let native = a.name == "NativeQuery"
                        || a.raw_args.contains("nativeQuery = true")
                        || a.raw_args.contains("nativeQuery=true");
                    let (named_params, positional_max) = extract_query_params(&qtext, text, qrange, idx);
                    Some(QuerySite {
                        query: qtext,
                        native,
                        inner_range: qrange,
                        named_params,
                        positional_max,
                    })
                });
            let name_range = idx.range_from_bytes(text, m.name_start, m.name_end);
            repo_methods.push(RepositoryMethod {
                name: m.name.clone(),
                return_type: m.return_or_field_type.clone(),
                param_names: m.params.iter().map(|p| p.name.clone()).collect(),
                param_types: m.params.iter().map(|p| p.type_name.clone()).collect(),
                explicit_params,
                query,
                range: idx.range_from_bytes(text, m.decl_start, m.decl_end),
                name_range,
                line: name_range.start.line,
            });
        }
        let r_range = idx.range_from_bytes(text, cls.name_start, cls.name_end);
        facts.repositories.push(RepositoryDef {
            name: cls.name.clone(),
            base_interface: base_repo,
            entity_type: entity_t,
            id_type: id_t,
            methods: repo_methods,
            file: path.to_owned(),
            range: r_range,
            line: r_range.start.line,
        });
    }

    // Class-level Lombok constructor injection
    let has_lombok_ctor = has_ann(&cls.annotations, "RequiredArgsConstructor")
        || has_ann(&cls.annotations, "AllArgsConstructor");
    if stereotype.is_some() && has_lombok_ctor {
        for m in &members {
            if m.is_field && m.is_final && !m.is_static && !has_ann(&m.annotations, "Value") {
                push_injection(
                    path,
                    text,
                    idx,
                    &class_bean_name,
                    &cls.name,
                    InjectionKind::LombokConstructor,
                    &m.name,
                    &m.return_or_field_type,
                    m.type_range,
                    idx.range_from_bytes(text, m.name_start, m.name_end),
                    &m.annotations,
                    true,
                    facts,
                );
            }
        }
    }

    // Kotlin primary constructor injection on Spring Beans
    if stereotype.is_some() && is_kotlin && !cls.primary_ctor_params.is_empty() {
        for p in &cls.primary_ctor_params {
            if !has_ann(&p.annotations, "Value") {
                push_injection(
                    path,
                    text,
                    idx,
                    &class_bean_name,
                    &cls.name,
                    InjectionKind::Constructor,
                    &p.name,
                    &p.type_name,
                    p.type_range,
                    p.range,
                    &p.annotations,
                    false,
                    facts,
                );
            }
        }
    }

    // Count Java constructors for implicit single-constructor DI
    let ctors: Vec<&MemberDecl> = members.iter().filter(|m| m.is_ctor).collect();
    let single_ctor_implicit = ctors.len() == 1;

    // Collect @Bean method names and @Transactional/@Async methods for self-invocation / proxyBeanMethods checks
    let mut bean_method_names = BTreeSet::new();
    let mut seen_bean_names = BTreeSet::new();
    let mut proxy_method_names: BTreeMap<String, String> = BTreeMap::new();

    for m in &members {
        if !m.is_field && !m.is_ctor {
            if has_ann(&m.annotations, "Bean") {
                bean_method_names.insert(m.name.clone());
            }
            for ann_name in ["Transactional", "Async"] {
                if has_ann(&m.annotations, ann_name) {
                    proxy_method_names.insert(m.name.clone(), ann_name.to_owned());
                }
            }
        }
    }

    let class_is_controller = has_ann(&cls.annotations, "RestController") || has_ann(&cls.annotations, "Controller");
    let class_is_http_exchange = has_ann(&cls.annotations, "HttpExchange");
    let feign_ann = find_ann(&cls.annotations, "FeignClient");
    let class_mapping_ann = find_ann(&cls.annotations, "RequestMapping")
        .or_else(|| find_ann(&cls.annotations, "HttpExchange"));
    let class_base_paths = class_mapping_ann
        .map(|a| extract_mapping_paths(a))
        .filter(|v| !v.is_empty())
        .or_else(|| feign_ann.and_then(|a| extract_string_attr(a, &["path"]).map(|p| vec![p])))
        .unwrap_or_else(|| vec![String::new()]);
    let feign_service = feign_ann.and_then(|a| extract_string_attr(a, &["name", "value"]));

    // Inspect fields & methods
    for m in &members {
        for ann in &m.annotations {
            extract_ann_value_and_links(path, text, idx, ann, facts);
        }

        if m.is_field {
            let has_inject_ann = has_ann(&m.annotations, "Autowired")
                || has_ann(&m.annotations, "Inject")
                || has_ann(&m.annotations, "Resource");
            if has_inject_ann {
                let required = extract_required(&m.annotations);
                push_injection(
                    path,
                    text,
                    idx,
                    &class_bean_name,
                    &cls.name,
                    InjectionKind::Field,
                    &m.name,
                    &m.return_or_field_type,
                    m.type_range,
                    idx.range_from_bytes(text, m.name_start, m.name_end),
                    &m.annotations,
                    m.is_final,
                    facts,
                );
                let _ = required;
            }
            continue;
        }

        if m.is_ctor {
            let is_autowired = has_ann(&m.annotations, "Autowired") || has_ann(&m.annotations, "Inject");
            if (is_autowired || single_ctor_implicit) && stereotype.is_some() {
                for p in &m.params {
                    if has_ann(&p.annotations, "Value") {
                        for ann in &p.annotations {
                            if ann.name == "Value" {
                                extract_value_annotation_site(path, text, idx, ann, facts);
                            }
                        }
                    } else {
                        push_injection(
                            path,
                            text,
                            idx,
                            &class_bean_name,
                            &cls.name,
                            InjectionKind::Constructor,
                            &p.name,
                            &p.type_name,
                            p.type_range,
                            p.range,
                            &p.annotations,
                            false,
                            facts,
                        );
                    }
                }
            }
            continue;
        }

        // Methods
        // 1. @Bean method
        if let Some(bean_ann) = find_ann(&m.annotations, "Bean") {
            let explicit_name = extract_string_attr(bean_ann, &["value", "name"]);
            let bean_name = explicit_name.unwrap_or_else(|| m.name.clone());
            let name_range = idx.range_from_bytes(text, m.name_start, m.name_end);
            if !seen_bean_names.insert(bean_name.clone()) {
                facts.duplicate_bean_methods.push((bean_name.clone(), name_range));
            }
            let ret_simple = simple_type_name(&m.return_or_field_type);
            facts.beans.push(BeanDef {
                id: format!("{path}#{}#{}", bean_name, name_range.start.line),
                name: bean_name.clone(),
                class_name: ret_simple.clone(),
                class_fqn: resolve_fqn(package, imports, &ret_simple),
                package: package.to_owned(),
                stereotype: BeanStereotype::BeanMethod,
                scope: extract_scope(&m.annotations),
                is_primary: has_ann(&m.annotations, "Primary"),
                is_lazy: has_ann(&m.annotations, "Lazy"),
                proxy_bean_methods: true,
                qualifiers: extract_qualifiers(text, idx, &m.annotations),
                profiles: extract_profiles(path, text, idx, &m.annotations, &mut facts.profile_refs),
                conditions: extract_conditions(&m.annotations),
                interfaces: Vec::new(),
                superclass: None,
                factory_bean: Some(cls.name.clone()),
                file: path.to_owned(),
                name_range,
                decl_range: idx.range_from_bytes(text, m.decl_start, m.name_end),
                line: name_range.start.line,
            });
            for p in &m.params {
                if has_ann(&p.annotations, "Value") {
                    for ann in &p.annotations {
                        if ann.name == "Value" {
                            extract_value_annotation_site(path, text, idx, ann, facts);
                        }
                    }
                } else {
                    push_injection(
                        path,
                        text,
                        idx,
                        &bean_name,
                        &cls.name,
                        InjectionKind::BeanMethodParam,
                        &p.name,
                        &p.type_name,
                        p.type_range,
                        p.range,
                        &p.annotations,
                        false,
                        facts,
                    );
                }
            }
        }

        // 2. @Autowired setter method
        if has_ann(&m.annotations, "Autowired") || has_ann(&m.annotations, "Inject") {
            for p in &m.params {
                push_injection(
                    path,
                    text,
                    idx,
                    &class_bean_name,
                    &cls.name,
                    InjectionKind::Setter,
                    &p.name,
                    &p.type_name,
                    p.type_range,
                    p.range,
                    &p.annotations,
                    false,
                    facts,
                );
            }
        }

        // 3. Endpoints: MVC, HTTP Exchange, FeignClient
        if class_is_controller || class_is_http_exchange || feign_ann.is_some() {
            if let Some((ep_kind, methods, own_paths, consumes, produces)) =
                extract_method_endpoint(m, class_is_http_exchange, feign_ann.is_some())
            {
                let mut path_vars = Vec::new();
                let mut query_params = Vec::new();
                let mut has_request_body = false;
                for p in &m.params {
                    if let Some(pv) = find_ann(&p.annotations, "PathVariable") {
                        let vname = extract_string_attr(pv, &["value", "name"])
                            .unwrap_or_else(|| p.name.clone());
                        let required = !pv.raw_args.contains("required = false")
                            && !pv.raw_args.contains("required=false");
                        path_vars.push(PathVarParam {
                            param_name: p.name.clone(),
                            var_name: vname,
                            required,
                            range: p.range,
                        });
                    }
                    if let Some(rp) = find_ann(&p.annotations, "RequestParam") {
                        let qname = extract_string_attr(rp, &["value", "name"])
                            .unwrap_or_else(|| p.name.clone());
                        query_params.push(qname);
                    }
                    if has_ann(&p.annotations, "RequestBody") {
                        has_request_body = true;
                    }
                }
                let m_range = idx.range_from_bytes(text, m.name_start, m.name_end);
                for base in &class_base_paths {
                    for own in &own_paths {
                        let full = join_paths(base, own);
                        facts.endpoints.push(EndpointDef {
                            methods: methods.clone(),
                            raw_path: own.clone(),
                            full_path: full,
                            kind: ep_kind,
                            controller: cls.name.clone(),
                            handler: format!("{}#{}", cls.name, m.name),
                            service_name: feign_service.clone(),
                            path_vars: path_vars.clone(),
                            query_params: query_params.clone(),
                            has_request_body,
                            consumes: consumes.clone(),
                            produces: produces.clone(),
                            file: path.to_owned(),
                            range: m_range,
                            line: m_range.start.line,
                        });
                    }
                }
            }
        }

        // 4. @EventListener / @TransactionalEventListener
        if let Some(ev_ann) = find_ann(&m.annotations, "EventListener")
            .or_else(|| find_ann(&m.annotations, "TransactionalEventListener"))
        {
            let ev_type = m
                .params
                .first()
                .map(|p| simple_type_name(&p.type_name))
                .or_else(|| extract_class_attr(ev_ann))
                .unwrap_or_default();
            if !ev_type.is_empty() {
                let m_range = idx.range_from_bytes(text, m.name_start, m.name_end);
                let cond = extract_string_attr(ev_ann, &["condition"]);
                if let Some(c) = &cond {
                    push_spel_site(path, text, idx, c, m_range, facts);
                }
                facts.event_sites.push(EventSite {
                    kind: EventKind::Listener,
                    event_type: ev_type,
                    method_name: Some(m.name.clone()),
                    condition_spel: cond,
                    async_listener: has_ann(&m.annotations, "Async"),
                    transactional: ev_ann.name == "TransactionalEventListener",
                    file: path.to_owned(),
                    range: m_range,
                    line: m_range.start.line,
                });
            }
        }

        // 5. @Scheduled
        if let Some(sch_ann) = find_ann(&m.annotations, "Scheduled") {
            let cron_with_range = extract_string_attr_with_range(text, idx, sch_ann, &["cron"]);
            let m_range = idx.range_from_bytes(text, m.name_start, m.name_end);
            facts.scheduled_sites.push(ScheduledSite {
                method_name: m.name.clone(),
                cron: cron_with_range.as_ref().map(|(s, _)| s.clone()),
                fixed_rate: extract_raw_attr(sch_ann, &["fixedRate", "fixedRateString"]),
                fixed_delay: extract_raw_attr(sch_ann, &["fixedDelay", "fixedDelayString"]),
                zone: extract_string_attr(sch_ann, &["zone"]),
                file: path.to_owned(),
                cron_range: cron_with_range.map(|(_, r)| r),
                range: m_range,
                line: m_range.start.line,
            });
        }

        // 6. @Transactional / @Async proxy checks
        for ann_name in ["Transactional", "Async"] {
            if has_ann(&m.annotations, ann_name) {
                let m_range = idx.range_from_bytes(text, m.name_start, m.name_end);
                if m.is_private {
                    facts.proxy_issues.push(ProxyIssueSite {
                        kind: ProxyIssueKind::PrivateMethod,
                        annotation: format!("@{ann_name}"),
                        method_name: m.name.clone(),
                        file: path.to_owned(),
                        range: m_range,
                        modifier_range: m.modifier_private_or_final_range,
                        line: m_range.start.line,
                    });
                } else if m.is_final || cls.is_final {
                    facts.proxy_issues.push(ProxyIssueSite {
                        kind: ProxyIssueKind::FinalMethod,
                        annotation: format!("@{ann_name}"),
                        method_name: m.name.clone(),
                        file: path.to_owned(),
                        range: m_range,
                        modifier_range: m.modifier_private_or_final_range,
                        line: m_range.start.line,
                    });
                }
            }
        }

        // 7. SpEL annotations (@PreAuthorize, @PostAuthorize, @Cacheable, @CacheEvict, @CachePut)
        for ann in &m.annotations {
            if matches!(
                ann.name.as_str(),
                "PreAuthorize" | "PostAuthorize" | "Cacheable" | "CacheEvict" | "CachePut"
            ) {
                let attr_keys: &[&str] = if ann.name.contains("Authorize") {
                    &["value"]
                } else {
                    &["condition", "unless", "key"]
                };
                if let Some((expr, range)) = extract_string_attr_with_range(text, idx, ann, attr_keys) {
                    push_spel_site(path, text, idx, &expr, range, facts);
                }
            }
        }

        // 8. Method body scan: publishEvent, self-invocation, proxyBeanMethods = false inter-bean calls
        if let Some((b_start, b_end)) = m.body_range_bytes {
            let body = &text[b_start..b_end];
            scan_method_body(
                path,
                text,
                idx,
                b_start,
                body,
                m,
                proxy_bean_methods,
                &bean_method_names,
                &proxy_method_names,
                facts,
            );
        }
    }
}

fn scan_method_body(
    path: &str,
    text: &str,
    idx: &LineIndex,
    body_start: usize,
    body: &str,
    method: &MemberDecl,
    proxy_bean_methods: bool,
    bean_method_names: &BTreeSet<String>,
    proxy_method_names: &BTreeMap<String, String>,
    facts: &mut FileFacts,
) {
    // 1. publishEvent(new SomethingEvent(...))
    let mut search_from = 0usize;
    while let Some(rel) = body[search_from..].find("publishEvent(") {
        let pos = search_from + rel + "publishEvent(".len();
        let after = body[pos..].trim_start();
        if let Some(after_new) = after.strip_prefix("new ") {
            let ev_type: String = after_new
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            let simple = simple_type_name(&ev_type);
            if !simple.is_empty() {
                let abs = body_start + search_from + rel;
                let range = idx.range_from_bytes(text, abs, abs + "publishEvent".len());
                facts.event_sites.push(EventSite {
                    kind: EventKind::Publisher,
                    event_type: simple,
                    method_name: Some(method.name.clone()),
                    condition_spel: None,
                    async_listener: false,
                    transactional: false,
                    file: path.to_owned(),
                    range,
                    line: range.start.line,
                });
            }
        }
        search_from = pos;
    }

    // 2. Same-class @Transactional/@Async self-invocation
    let caller_has_tx = has_ann(&method.annotations, "Transactional");
    let caller_has_async = has_ann(&method.annotations, "Async");
    for (target_name, ann) in proxy_method_names {
        if target_name == &method.name {
            continue;
        }
        if (ann == "Transactional" && caller_has_tx) || (ann == "Async" && caller_has_async) {
            continue;
        }
        if let Some(call_offset) = find_unqualified_call(body, target_name) {
            let abs = body_start + call_offset;
            let range = idx.range_from_bytes(text, abs, abs + target_name.len());
            facts.proxy_issues.push(ProxyIssueSite {
                kind: ProxyIssueKind::SelfInvocation,
                annotation: format!("@{ann}"),
                method_name: target_name.clone(),
                file: path.to_owned(),
                range,
                modifier_range: None,
                line: range.start.line,
            });
        }
    }

    // 3. @Configuration(proxyBeanMethods = false) direct @Bean method invocation
    if !proxy_bean_methods && has_ann(&method.annotations, "Bean") {
        for bean_m in bean_method_names {
            if bean_m == &method.name {
                continue;
            }
            if let Some(call_offset) = find_unqualified_call(body, bean_m) {
                let abs = body_start + call_offset;
                let range = idx.range_from_bytes(text, abs, abs + bean_m.len());
                facts.proxy_issues.push(ProxyIssueSite {
                    kind: ProxyIssueKind::ProxyBeanMethodsFalseCall,
                    annotation: "@Configuration(proxyBeanMethods = false)".into(),
                    method_name: bean_m.clone(),
                    file: path.to_owned(),
                    range,
                    modifier_range: None,
                    line: range.start.line,
                });
            }
        }
    }
}

fn find_unqualified_call(body: &str, name: &str) -> Option<usize> {
    let needle = format!("{name}(");
    let mut from = 0usize;
    let bytes = body.as_bytes();
    while let Some(rel) = body[from..].find(&needle) {
        let abs = from + rel;
        let prev = abs.checked_sub(1).and_then(|i| bytes.get(i).copied());
        let is_unqualified = match prev {
            None => true,
            Some(b) if b.is_ascii_whitespace() || matches!(b, b';' | b'{' | b'(' | b'=' | b'!') => true,
            Some(b'.') => body[..abs].trim_end_matches('.').ends_with("this"),
            _ => false,
        };
        if is_unqualified {
            return Some(abs);
        }
        from = abs + needle.len();
    }
    None
}

fn scan_webflux_routes(path: &str, text: &str, idx: &LineIndex, facts: &mut FileFacts) {
    if !text.contains("RouterFunction") && !text.contains("router {") && !text.contains("coRouter {") {
        return;
    }
    for method in ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"] {
        let pattern = format!(".{method}(");
        let mut from = 0usize;
        while let Some(rel) = text[from..].find(&pattern) {
            let abs = from + rel;
            let after = &text[abs + pattern.len()..];
            if let Some((route_path, p_start, p_end)) = first_string_literal_with_offsets(after, abs + pattern.len()) {
                let range = idx.range_from_bytes(text, p_start, p_end);
                facts.endpoints.push(EndpointDef {
                    methods: vec![method.to_owned()],
                    raw_path: route_path.clone(),
                    full_path: normalize_route_path(&route_path),
                    kind: EndpointKind::WebFlux,
                    controller: "WebFluxRouter".into(),
                    handler: format!("Router#{method}"),
                    service_name: None,
                    path_vars: Vec::new(),
                    query_params: Vec::new(),
                    has_request_body: false,
                    consumes: None,
                    produces: None,
                    file: path.to_owned(),
                    range,
                    line: range.start.line,
                });
            }
            from = abs + pattern.len();
        }
        // Kotlin DSL `GET("/users", handler::list)`
        let kt_pattern = format!("{method}(\"");
        let mut kfrom = 0usize;
        while let Some(rel) = text[kfrom..].find(&kt_pattern) {
            let abs = kfrom + rel;
            let prev = abs.checked_sub(1).and_then(|i| text.as_bytes().get(i).copied());
            if prev.is_none_or(|b| b != b'.' && !b.is_ascii_alphanumeric()) {
                let after = &text[abs + method.len() + 1..];
                if let Some((route_path, p_start, p_end)) = first_string_literal_with_offsets(after, abs + method.len() + 1) {
                    let range = idx.range_from_bytes(text, p_start, p_end);
                    if !facts.endpoints.iter().any(|e| e.range == range && e.methods == [method]) {
                        facts.endpoints.push(EndpointDef {
                            methods: vec![method.to_owned()],
                            raw_path: route_path.clone(),
                            full_path: normalize_route_path(&route_path),
                            kind: EndpointKind::WebFlux,
                            controller: "WebFluxRouter".into(),
                            handler: format!("router#{method}"),
                            service_name: None,
                            path_vars: Vec::new(),
                            query_params: Vec::new(),
                            has_request_body: false,
                            consumes: None,
                            produces: None,
                            file: path.to_owned(),
                            range,
                            line: range.start.line,
                        });
                    }
                }
            }
            kfrom = abs + kt_pattern.len();
        }
    }
}

fn first_string_literal_with_offsets(slice: &str, base: usize) -> Option<(String, usize, usize)> {
    let trimmed = slice.trim_start();
    let lead = slice.len() - trimmed.len();
    if !trimmed.starts_with('"') {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut i = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            return Some((
                trimmed[1..i].to_owned(),
                base + lead + 1,
                base + lead + i,
            ));
        }
        i += 1;
    }
    None
}

fn push_injection(
    path: &str,
    text: &str,
    idx: &LineIndex,
    owner_bean: &str,
    owner_class: &str,
    kind: InjectionKind,
    field_name: &str,
    raw_type: &str,
    type_range: Range,
    range: Range,
    anns: &[ParsedAnnotation],
    is_final: bool,
    facts: &mut FileFacts,
) {
    let (target_type, is_optional_wrapper) = unwrap_container_type(raw_type);
    if target_type.is_empty() || is_primitive_or_string(&target_type) {
        return;
    }
    let is_collection_or_provider = raw_type.starts_with("List<")
        || raw_type.starts_with("Set<")
        || raw_type.starts_with("Collection<")
        || raw_type.starts_with("Map<")
        || raw_type.starts_with("ObjectProvider<")
        || raw_type.starts_with("Optional<");
    let qualifier = find_ann(anns, "Qualifier")
        .or_else(|| find_ann(anns, "Resource"))
        .and_then(|a| extract_string_attr_with_range(text, idx, a, &["value", "name"]));
    let required = !is_optional_wrapper && extract_required(anns);

    facts.injections.push(InjectionSite {
        owner_bean: owner_bean.to_owned(),
        owner_class: owner_class.to_owned(),
        kind,
        field_or_param_name: field_name.to_owned(),
        raw_type: raw_type.to_owned(),
        target_type,
        qualifier,
        required,
        is_collection_or_provider,
        is_final,
        file: path.to_owned(),
        range,
        type_range,
        line: range.start.line,
    });
}

fn is_primitive_or_string(ty: &str) -> bool {
    matches!(
        ty,
        "String" | "Int" | "Integer" | "Long" | "Boolean" | "Double" | "Float" | "Short" | "Byte"
            | "Char" | "Character" | "int" | "long" | "boolean" | "double" | "float" | "short"
            | "byte" | "char" | "void" | "Unit"
    )
}

fn extract_ann_value_and_links(
    path: &str,
    text: &str,
    idx: &LineIndex,
    ann: &ParsedAnnotation,
    facts: &mut FileFacts,
) {
    if ann.name == "Value" {
        extract_value_annotation_site(path, text, idx, ann, facts);
    } else if ann.name == "ConditionalOnProperty" {
        let prefix = extract_string_attr(ann, &["prefix"]).unwrap_or_default();
        if let Some((raw_name, range)) = extract_string_attr_with_range(text, idx, ann, &["value", "name"]) {
            let full = if prefix.is_empty() {
                raw_name.clone()
            } else {
                format!("{prefix}.{raw_name}")
            };
            facts.value_sites.push(ValueExprSite {
                raw: raw_name,
                prop_key: Some(full),
                default_value: None,
                is_spel: false,
                is_conditional_on_property: true,
                file: path.to_owned(),
                key_range: range,
                full_range: range,
                line: range.start.line,
            });
        }
    } else if ann.name == "PropertySource" || ann.name == "PropertySources" {
        if let Some((loc, range)) = extract_string_attr_with_range(text, idx, ann, &["value"]) {
            facts.link_targets.push(LinkTargetSite {
                raw_target: loc,
                range,
                file: path.to_owned(),
            });
        }
    }
}

fn extract_value_annotation_site(
    path: &str,
    text: &str,
    idx: &LineIndex,
    ann: &ParsedAnnotation,
    facts: &mut FileFacts,
) {
    let Some((raw, val_range)) = extract_string_attr_with_range(text, idx, ann, &["value"]) else {
        return;
    };
    if raw.starts_with("#{") {
        push_spel_site(path, text, idx, &raw, val_range, facts);
    }
    for ph in extract_placeholders(&raw, text, val_range, idx) {
        facts.value_sites.push(ValueExprSite {
            raw: raw.clone(),
            prop_key: Some(ph.key),
            default_value: ph.default_value,
            is_spel: false,
            is_conditional_on_property: false,
            file: path.to_owned(),
            key_range: ph.key_range,
            full_range: ph.full_range,
            line: ph.key_range.start.line,
        });
    }
}

fn push_spel_site(
    path: &str,
    text: &str,
    idx: &LineIndex,
    expr: &str,
    range: Range,
    facts: &mut FileFacts,
) {
    let base_byte = idx.pos_to_byte(text, range.start);
    let mut bean_refs = Vec::new();
    let bytes = expr.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'@' && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_') {
            let s = i + 1;
            let mut e = s;
            while e < bytes.len() && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'_') {
                e += 1;
            }
            let bean = expr[s..e].to_owned();
            let br = idx.range_from_bytes(text, base_byte + s, base_byte + e);
            bean_refs.push((bean, br));
            i = e;
            continue;
        }
        i += 1;
    }
    facts.spel_sites.push(SpelSite {
        expr: expr.to_owned(),
        bean_refs,
        prop_refs: Vec::new(),
        file: path.to_owned(),
        range,
        line: range.start.line,
    });
}

fn detect_stereotype(anns: &[ParsedAnnotation]) -> Option<BeanStereotype> {
    if has_ann(anns, "SpringBootApplication") {
        return Some(BeanStereotype::SpringBootApplication);
    }
    if has_ann(anns, "RestController") {
        return Some(BeanStereotype::RestController);
    }
    if has_ann(anns, "Controller") || has_ann(anns, "ControllerAdvice") || has_ann(anns, "RestControllerAdvice") {
        return Some(BeanStereotype::Controller);
    }
    if has_ann(anns, "Service") {
        return Some(BeanStereotype::Service);
    }
    if has_ann(anns, "Repository") {
        return Some(BeanStereotype::Repository);
    }
    if has_ann(anns, "Configuration") {
        return Some(BeanStereotype::Configuration);
    }
    if has_ann(anns, "ConfigurationProperties") {
        return Some(BeanStereotype::ConfigurationProperties);
    }
    if has_ann(anns, "FeignClient") {
        return Some(BeanStereotype::FeignClient);
    }
    if has_ann(anns, "HttpExchange") {
        return Some(BeanStereotype::HttpExchange);
    }
    if has_ann(anns, "Component") || has_ann(anns, "Named") {
        return Some(BeanStereotype::Component);
    }
    None
}

fn extract_bean_name_from_stereotype(anns: &[ParsedAnnotation], st: BeanStereotype) -> Option<String> {
    let ann_name = match st {
        BeanStereotype::Component => "Component",
        BeanStereotype::Service => "Service",
        BeanStereotype::Repository => "Repository",
        BeanStereotype::Controller => "Controller",
        BeanStereotype::RestController => "RestController",
        BeanStereotype::Configuration => "Configuration",
        BeanStereotype::FeignClient => "FeignClient",
        _ => return None,
    };
    find_ann(anns, ann_name).and_then(|a| extract_string_attr(a, &["value", "name"]))
}

fn extract_scope(anns: &[ParsedAnnotation]) -> String {
    find_ann(anns, "Scope")
        .and_then(|a| extract_string_attr(a, &["value", "scopeName"]))
        .unwrap_or_else(|| "singleton".to_owned())
}

fn extract_proxy_bean_methods(anns: &[ParsedAnnotation]) -> bool {
    if let Some(cfg) = find_ann(anns, "Configuration")
        && (cfg.raw_args.contains("proxyBeanMethods = false") || cfg.raw_args.contains("proxyBeanMethods=false"))
    {
        return false;
    }
    true
}

fn extract_qualifiers(text: &str, idx: &LineIndex, anns: &[ParsedAnnotation]) -> Vec<(String, Range)> {
    anns.iter()
        .filter(|a| a.name == "Qualifier")
        .filter_map(|a| extract_string_attr_with_range(text, idx, a, &["value"]))
        .collect()
}

fn extract_profiles(
    path: &str,
    text: &str,
    idx: &LineIndex,
    anns: &[ParsedAnnotation],
    profile_refs: &mut Vec<ProfileRefSite>,
) -> Vec<String> {
    let mut out = Vec::new();
    for ann in anns.iter().filter(|a| a.name == "Profile") {
        for (val, range) in extract_all_string_literals(text, idx, ann) {
            out.push(val.clone());
            profile_refs.push(ProfileRefSite {
                profile: val,
                range,
                file: path.to_owned(),
            });
        }
    }
    out
}

fn extract_conditions(anns: &[ParsedAnnotation]) -> Vec<String> {
    anns.iter()
        .filter(|a| a.name.starts_with("Conditional"))
        .map(|a| {
            if a.raw_args.is_empty() {
                format!("@{}", a.name)
            } else {
                format!("@{}({})", a.name, a.raw_args.trim())
            }
        })
        .collect()
}

fn extract_required(anns: &[ParsedAnnotation]) -> bool {
    if let Some(a) = find_ann(anns, "Autowired") {
        if a.raw_args.contains("required = false") || a.raw_args.contains("required=false") {
            return false;
        }
    }
    true
}

fn detect_repository_super(extends: &[String], implements: &[String]) -> Option<(String, String, String)> {
    const REPO_BASES: &[&str] = &[
        "JpaRepository",
        "CrudRepository",
        "ListCrudRepository",
        "PagingAndSortingRepository",
        "ListPagingAndSortingRepository",
        "MongoRepository",
        "ReactiveCrudRepository",
        "R2dbcRepository",
        "ReactiveMongoRepository",
        "Repository",
    ];
    for sup in extends.iter().chain(implements.iter()) {
        let base = simple_type_name(sup);
        if REPO_BASES.contains(&base.as_str())
            && let Some(start_gen) = sup.find('<')
            && let Some(end_gen) = sup.rfind('>')
        {
            let inner = &sup[start_gen + 1..end_gen];
            if let Some((e, id)) = inner.split_once(',') {
                return Some((base, simple_type_name(e), simple_type_name(id)));
            }
        }
    }
    None
}

fn extract_method_endpoint(
    m: &MemberDecl,
    class_http_exchange: bool,
    class_feign: bool,
) -> Option<(EndpointKind, Vec<String>, Vec<String>, Option<String>, Option<String>)> {
    let mapping_defs = [
        ("GetMapping", "GET", EndpointKind::Mvc),
        ("PostMapping", "POST", EndpointKind::Mvc),
        ("PutMapping", "PUT", EndpointKind::Mvc),
        ("DeleteMapping", "DELETE", EndpointKind::Mvc),
        ("PatchMapping", "PATCH", EndpointKind::Mvc),
        ("GetExchange", "GET", EndpointKind::HttpExchange),
        ("PostExchange", "POST", EndpointKind::HttpExchange),
        ("PutExchange", "PUT", EndpointKind::HttpExchange),
        ("DeleteExchange", "DELETE", EndpointKind::HttpExchange),
        ("PatchExchange", "PATCH", EndpointKind::HttpExchange),
    ];
    for (ann_name, http_m, default_kind) in mapping_defs {
        if let Some(ann) = find_ann(&m.annotations, ann_name) {
            let kind = if class_feign {
                EndpointKind::FeignClient
            } else if class_http_exchange {
                EndpointKind::HttpExchange
            } else {
                default_kind
            };
            let paths = extract_mapping_paths(ann);
            return Some((
                kind,
                vec![http_m.to_owned()],
                if paths.is_empty() { vec![String::new()] } else { paths },
                extract_string_attr(ann, &["consumes", "contentType"]),
                extract_string_attr(ann, &["produces", "accept"]),
            ));
        }
    }
    if let Some(ann) = find_ann(&m.annotations, "RequestMapping")
        .or_else(|| find_ann(&m.annotations, "HttpExchange"))
    {
        let kind = if class_feign {
            EndpointKind::FeignClient
        } else if ann.name == "HttpExchange" || class_http_exchange {
            EndpointKind::HttpExchange
        } else {
            EndpointKind::Mvc
        };
        let methods = extract_request_methods(ann);
        let paths = extract_mapping_paths(ann);
        return Some((
            kind,
            methods,
            if paths.is_empty() { vec![String::new()] } else { paths },
            extract_string_attr(ann, &["consumes", "contentType"]),
            extract_string_attr(ann, &["produces", "accept"]),
        ));
    }
    None
}

fn extract_mapping_paths(ann: &ParsedAnnotation) -> Vec<String> {
    if let Some(p) = extract_string_attr(ann, &["path", "value"]) {
        return vec![p];
    }
    // Check array literal like {"/a", "/b"}
    let mut out = Vec::new();
    for (s, _) in extract_all_string_literals_raw(&ann.raw_args) {
        if s.starts_with('/') || (!ann.raw_args.contains('=') && !s.is_empty()) {
            out.push(s);
        }
    }
    out
}

fn extract_request_methods(ann: &ParsedAnnotation) -> Vec<String> {
    let mut out = Vec::new();
    for m in ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"] {
        if ann.raw_args.contains(&format!("RequestMethod.{m}"))
            || ann.raw_args.contains(&format!("method = \"{m}\""))
            || ann.raw_args.contains(&format!("method=\"{m}\""))
        {
            out.push(m.to_owned());
        }
    }
    if out.is_empty() {
        out.push("GET".to_owned());
    }
    out
}

fn join_paths(base: &str, own: &str) -> String {
    let joined = format!("/{}/{}", base.trim_matches('/'), own.trim_matches('/'));
    normalize_route_path(&joined)
}

fn normalize_route_path(p: &str) -> String {
    let mut s = String::from("/");
    for part in p.split('/').filter(|seg| !seg.is_empty()) {
        if s.len() > 1 {
            s.push('/');
        }
        s.push_str(part);
    }
    s
}

fn has_ann(anns: &[ParsedAnnotation], name: &str) -> bool {
    anns.iter().any(|a| a.name == name)
}

fn find_ann<'a>(anns: &'a [ParsedAnnotation], name: &str) -> Option<&'a ParsedAnnotation> {
    anns.iter().find(|a| a.name == name)
}

fn has_unquoted_equals(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'=' => return true,
            _ => i += 1,
        }
    }
    false
}

fn extract_string_attr(ann: &ParsedAnnotation, keys: &[&str]) -> Option<String> {
    let raw = ann.raw_args.trim();
    if raw.is_empty() {
        return None;
    }
    for k in keys {
        if let Some((val, _, _)) = find_named_string_arg(raw, k) {
            return Some(val);
        }
    }
    if !has_unquoted_equals(raw)
        && let Some((val, _, _)) = first_string_literal_with_offsets(raw, 0)
    {
        return Some(val);
    }
    None
}

fn extract_string_attr_with_range(
    text: &str,
    idx: &LineIndex,
    ann: &ParsedAnnotation,
    keys: &[&str],
) -> Option<(String, Range)> {
    let raw = ann.raw_args.as_str();
    if raw.trim().is_empty() {
        return None;
    }
    for k in keys {
        if let Some((val, s, e)) = find_named_string_arg(raw, k) {
            return Some((
                val,
                idx.range_from_bytes(text, ann.args_byte_start + s, ann.args_byte_start + e),
            ));
        }
    }
    if !has_unquoted_equals(raw)
        && let Some((val, s, e)) = first_string_literal_with_offsets(raw, ann.args_byte_start)
    {
        return Some((val, idx.range_from_bytes(text, s, e)));
    }
    None
}

fn find_named_string_arg(raw: &str, key: &str) -> Option<(String, usize, usize)> {
    let mut from = 0usize;
    let bytes = raw.as_bytes();
    while let Some(rel) = raw[from..].find(key) {
        let abs = from + rel;
        let prev_ok = abs == 0 || !bytes[abs - 1].is_ascii_alphanumeric();
        let after_key = abs + key.len();
        if prev_ok {
            let rest = &raw[after_key..];
            let ws = rest.len() - rest.trim_start().len();
            if rest[ws..].starts_with('=') {
                let eq_after = after_key + ws + 1;
                if let Some((val, s, e)) = first_string_literal_with_offsets(&raw[eq_after..], eq_after) {
                    return Some((val, s, e));
                }
            }
        }
        from = after_key;
    }
    None
}

fn extract_raw_attr(ann: &ParsedAnnotation, keys: &[&str]) -> Option<String> {
    let raw = ann.raw_args.as_str();
    for k in keys {
        if let Some(pos) = raw.find(k) {
            let after = raw[pos + k.len()..].trim_start();
            if let Some(after_eq) = after.strip_prefix('=') {
                let val = after_eq.split(',').next().unwrap_or("").trim();
                if !val.is_empty() {
                    return Some(val.trim_matches('"').to_owned());
                }
            }
        }
    }
    None
}

fn extract_class_attr(ann: &ParsedAnnotation) -> Option<String> {
    for suffix in [".class", "::class"] {
        if let Some(pos) = ann.raw_args.find(suffix) {
            let before = &ann.raw_args[..pos];
            let name: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if !name.is_empty() {
                return Some(simple_type_name(&name));
            }
        }
    }
    None
}

fn extract_all_string_literals_raw(raw: &str) -> Vec<(String, (usize, usize))> {
    let mut out = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let s = i + 1;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    out.push((raw[s..i].to_owned(), (s, i)));
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

fn extract_all_string_literals(text: &str, idx: &LineIndex, ann: &ParsedAnnotation) -> Vec<(String, Range)> {
    extract_all_string_literals_raw(&ann.raw_args)
        .into_iter()
        .map(|(s, (a, b))| {
            (
                s,
                idx.range_from_bytes(text, ann.args_byte_start + a, ann.args_byte_start + b),
            )
        })
        .collect()
}

// ============================================================================
// Linear-time Lexical Scanner for Java & Kotlin Classes and Members
// ============================================================================

fn skip_ws_and_comments(text: &str, mut i: usize) -> usize {
    let b = text.as_bytes();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
            i += 2;
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        break;
    }
    i
}

fn skip_string_or_comment(text: &str, i: usize) -> Option<usize> {
    let b = text.as_bytes();
    if i >= b.len() {
        return None;
    }
    if text[i..].starts_with("//") {
        let end = text[i..].find('\n').map_or(b.len(), |p| i + p + 1);
        return Some(end);
    }
    if text[i..].starts_with("/*") {
        let end = text[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
        return Some(end);
    }
    if text[i..].starts_with("\"\"\"") {
        let end = text[i + 3..].find("\"\"\"").map_or(b.len(), |p| i + 3 + p + 3);
        return Some(end);
    }
    if b[i] == b'"' {
        let mut j = i + 1;
        while j < b.len() {
            if b[j] == b'\\' {
                j += 2;
                continue;
            }
            if b[j] == b'"' {
                return Some(j + 1);
            }
            j += 1;
        }
        return Some(b.len());
    }
    if b[i] == b'\'' {
        let mut j = i + 1;
        while j < b.len() {
            if b[j] == b'\\' {
                j += 2;
                continue;
            }
            if b[j] == b'\'' {
                return Some(j + 1);
            }
            j += 1;
        }
        return Some(b.len());
    }
    None
}

fn find_matching_delim(text: &str, open_idx: usize, open: u8, close: u8) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = open_idx + 1;
    let mut depth = 1usize;
    while i < b.len() {
        if let Some(next) = skip_string_or_comment(text, i) {
            i = next;
            continue;
        }
        if b[i] == open {
            depth += 1;
        } else if b[i] == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn read_annotation(text: &str, at_idx: usize) -> Option<ParsedAnnotation> {
    let b = text.as_bytes();
    if b.get(at_idx) != Some(&b'@') {
        return None;
    }
    let mut i = at_idx + 1;
    // Skip Kotlin use-site target e.g. `@field:Autowired`, `@get:Bean`
    let name_start = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'.' | b':')) {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let full = &text[name_start..i];
    let after_colon = full.rsplit(':').next().unwrap_or(full);
    let simple = after_colon.rsplit('.').next().unwrap_or(after_colon).to_owned();

    let after_ws = skip_ws_and_comments(text, i);
    if b.get(after_ws) == Some(&b'(')
        && let Some(close) = find_matching_delim(text, after_ws, b'(', b')')
    {
        return Some(ParsedAnnotation {
            name: simple,
            raw_args: text[after_ws + 1..close].to_owned(),
            args_byte_start: after_ws + 1,
            start_byte: at_idx,
            end_byte: close + 1,
        });
    }
    Some(ParsedAnnotation {
        name: simple,
        raw_args: String::new(),
        args_byte_start: i,
        start_byte: at_idx,
        end_byte: i,
    })
}

fn find_top_classes(text: &str, is_kotlin: bool, idx: &LineIndex) -> Vec<ClassHeader> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut brace_depth = 0usize;
    let mut pending_anns: Vec<ParsedAnnotation> = Vec::new();
    let mut pending_final = false;

    while i < b.len() {
        if let Some(next) = skip_string_or_comment(text, i) {
            i = next;
            continue;
        }
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'{' {
            brace_depth += 1;
            pending_anns.clear();
            pending_final = false;
            i += 1;
            continue;
        }
        if c == b'}' {
            brace_depth = brace_depth.saturating_sub(1);
            pending_anns.clear();
            pending_final = false;
            i += 1;
            continue;
        }
        if c == b';' {
            pending_anns.clear();
            pending_final = false;
            i += 1;
            continue;
        }
        if c == b'@' {
            if let Some(ann) = read_annotation(text, i) {
                i = ann.end_byte;
                pending_anns.push(ann);
                continue;
            }
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let word_start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &text[word_start..i];
            if word == "final" {
                pending_final = true;
                continue;
            }
            if matches!(word, "public" | "private" | "protected" | "abstract" | "static" | "sealed" | "open" | "data" | "internal") {
                continue;
            }
            if matches!(word, "class" | "interface" | "record" | "enum") && brace_depth <= 1 {
                let mut cur = skip_ws_and_comments(text, i);
                let name_start = cur;
                while cur < b.len() && (b[cur].is_ascii_alphanumeric() || b[cur] == b'_') {
                    cur += 1;
                }
                let name_end = cur;
                if name_end == name_start {
                    pending_anns.clear();
                    continue;
                }
                let cls_name = text[name_start..name_end].to_owned();
                let decl_start = pending_anns
                    .first()
                    .map_or(word_start, |a| a.start_byte);

                // Skip generics `<...>`
                cur = skip_ws_and_comments(text, name_end);
                if b.get(cur) == Some(&b'<')
                    && let Some(close_g) = find_matching_delim(text, cur, b'<', b'>')
                {
                    cur = skip_ws_and_comments(text, close_g + 1);
                }

                // Optional primary ctor / record header `(...)`
                let mut primary_ctor_params = Vec::new();
                if b.get(cur) == Some(&b'(')
                    && let Some(close_p) = find_matching_delim(text, cur, b'(', b')')
                {
                    primary_ctor_params = parse_param_list(text, cur + 1, close_p, is_kotlin, idx);
                    cur = skip_ws_and_comments(text, close_p + 1);
                }

                // Scan header until `{`
                let mut header_end = cur;
                let mut p_depth = 0usize;
                let mut a_depth = 0usize;
                while header_end < b.len() {
                    if let Some(next) = skip_string_or_comment(text, header_end) {
                        header_end = next;
                        continue;
                    }
                    match b[header_end] {
                        b'(' => p_depth += 1,
                        b')' => p_depth = p_depth.saturating_sub(1),
                        b'<' => a_depth += 1,
                        b'>' => a_depth = a_depth.saturating_sub(1),
                        b'{' if p_depth == 0 && a_depth == 0 => break,
                        _ => {}
                    }
                    header_end += 1;
                }
                if header_end < b.len() && b[header_end] == b'{' {
                    let body_end = find_matching_delim(text, header_end, b'{', b'}').unwrap_or(b.len());
                    let header_clause = &text[cur..header_end];
                    let (extends_list, implements_list) = parse_supertypes(header_clause, is_kotlin);
                    out.push(ClassHeader {
                        annotations: std::mem::take(&mut pending_anns),
                        kind: word.to_owned(),
                        name: cls_name,
                        name_start,
                        name_end,
                        decl_start,
                        body_start: header_end + 1,
                        body_end,
                        extends_list,
                        implements_list,
                        primary_ctor_params,
                        is_final: pending_final,
                    });
                    pending_final = false;
                    // Continue inside body so nested @Configuration/@Component classes are also found!
                    brace_depth += 1;
                    i = header_end + 1;
                    continue;
                }
            }
            if matches!(word, "package" | "import") {
                if let Some(semi) = text[i..].find('\n') {
                    i += semi + 1;
                }
                pending_anns.clear();
                pending_final = false;
                continue;
            }
            continue;
        }
        i += 1;
    }

    out
}

fn parse_supertypes(clause: &str, is_kotlin: bool) -> (Vec<String>, Vec<String>) {
    let mut extends = Vec::new();
    let mut implements = Vec::new();
    if is_kotlin {
        if let Some((_, rest)) = clause.split_once(':') {
            for part in split_top_level_commas(rest) {
                let clean = part.trim();
                let ty = clean.split('(').next().unwrap_or(clean).trim();
                if !ty.is_empty() {
                    extends.push(ty.to_owned());
                    implements.push(ty.to_owned());
                }
            }
        }
        return (extends, implements);
    }

    let ext_pos = clause.find("extends ");
    let imp_pos = clause.find("implements ");
    match (ext_pos, imp_pos) {
        (Some(e), Some(im)) if e < im => {
            for p in split_top_level_commas(&clause[e + 8..im]) {
                let t = p.trim();
                if !t.is_empty() {
                    extends.push(t.to_owned());
                }
            }
            for p in split_top_level_commas(&clause[im + 11..]) {
                let t = p.trim();
                if !t.is_empty() {
                    implements.push(t.to_owned());
                }
            }
        }
        (Some(e), None) => {
            for p in split_top_level_commas(&clause[e + 8..]) {
                let t = p.trim();
                if !t.is_empty() {
                    extends.push(t.to_owned());
                }
            }
        }
        (None, Some(im)) => {
            for p in split_top_level_commas(&clause[im + 11..]) {
                let t = p.trim();
                if !t.is_empty() {
                    implements.push(t.to_owned());
                }
            }
        }
        _ => {}
    }
    (extends, implements)
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut start = 0usize;
    let mut angle = 0usize;
    let mut paren = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if let Some(next) = skip_string_or_comment(s, i) {
            i = next;
            continue;
        }
        match b[i] {
            b'<' => angle += 1,
            b'>' => angle = angle.saturating_sub(1),
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b',' if angle == 0 && paren == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start <= s.len() {
        out.push(&s[start..]);
    }
    out
}

fn parse_members(
    text: &str,
    body_start: usize,
    body_end: usize,
    class_name: &str,
    is_kotlin: bool,
    idx: &LineIndex,
) -> Vec<MemberDecl> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = body_start;
    let mut pending_anns: Vec<ParsedAnnotation> = Vec::new();
    let mut header_tokens: Vec<(String, usize, usize)> = Vec::new();
    let mut header_start: Option<usize> = None;

    while i < body_end && i < b.len() {
        let next_i = skip_ws_and_comments(text, i);
        if next_i > i {
            i = next_i;
            continue;
        }
        let c = b[i];
        if c == b'}' {
            break;
        }
        if let Some(next) = skip_string_or_comment(text, i) {
            i = next;
            continue;
        }
        if c == b'@' {
            if let Some(ann) = read_annotation(text, i) {
                if header_start.is_none() {
                    header_start = Some(ann.start_byte);
                }
                i = ann.end_byte;
                pending_anns.push(ann);
                continue;
            }
        }
        if c == b'(' {
            if let Some(close_p) = find_matching_delim(text, i, b'(', b')') {
                // Method or constructor parameter list!
                let params = parse_param_list(text, i + 1, close_p, is_kotlin, idx);
                let decl_start = header_start.unwrap_or(i);
                let mut cur = skip_ws_and_comments(text, close_p + 1);
                // Optional Kotlin return type `: ReturnType`
                let mut kt_ret = String::new();
                if is_kotlin && b.get(cur) == Some(&b':') {
                    let ret_start = cur + 1;
                    while cur < body_end && !matches!(b[cur], b'{' | b'=' | b'\n') {
                        cur += 1;
                    }
                    kt_ret = text[ret_start..cur].trim().to_owned();
                }
                // Advance to `{` or `;` or `=` (Kotlin single-expression fun)
                let mut body_range_bytes = None;
                let mut decl_end = cur;
                while cur < body_end {
                    if let Some(next) = skip_string_or_comment(text, cur) {
                        cur = next;
                        continue;
                    }
                    if b[cur] == b'{' {
                        let m_end = find_matching_delim(text, cur, b'{', b'}').unwrap_or(body_end);
                        body_range_bytes = Some((cur + 1, m_end));
                        decl_end = (m_end + 1).min(body_end);
                        cur = decl_end;
                        break;
                    }
                    if b[cur] == b';' {
                        decl_end = cur + 1;
                        cur = decl_end;
                        break;
                    }
                    if is_kotlin && b[cur] == b'\n' && header_tokens.iter().any(|(t, _, _)| t == "fun") {
                        decl_end = cur;
                        break;
                    }
                    cur += 1;
                }
                if let Some((mname, ns, ne, rtype, is_ctor, is_priv, is_fin, mod_range)) =
                    interpret_callable_header(&header_tokens, class_name, &kt_ret, is_kotlin, text, idx)
                {
                    out.push(MemberDecl {
                        annotations: std::mem::take(&mut pending_anns),
                        is_field: false,
                        is_ctor,
                        is_private: is_priv,
                        is_final: is_fin,
                        is_static: header_tokens.iter().any(|(t, _, _)| t == "static"),
                        return_or_field_type: rtype,
                        type_range: idx.range_from_bytes(text, ns, ne),
                        name: mname,
                        name_start: ns,
                        name_end: ne,
                        decl_start,
                        decl_end,
                        modifier_private_or_final_range: mod_range,
                        params,
                        body_range_bytes,
                    });
                }
                header_tokens.clear();
                header_start = None;
                pending_anns.clear();
                i = cur;
                continue;
            }
        }
        if c == b'=' || c == b';' || (is_kotlin && c == b'\n' && !header_tokens.is_empty()) {
            // Field declaration or end of statement
            if let Some((fname, ns, ne, ftype, type_range, is_priv, is_fin, is_stat, mod_range)) =
                interpret_field_header(&header_tokens, is_kotlin, text, idx)
            {
                let decl_start = header_start.unwrap_or(ns);
                let mut cur = i;
                if c == b'=' {
                    // Advance to `;` or newline (in Kotlin) at top level
                    let mut p = 0usize;
                    let mut br = 0usize;
                    while cur < body_end {
                        if let Some(next) = skip_string_or_comment(text, cur) {
                            cur = next;
                            continue;
                        }
                        match b[cur] {
                            b'(' => p += 1,
                            b')' => p = p.saturating_sub(1),
                            b'{' => br += 1,
                            b'}' => {
                                if br == 0 {
                                    break;
                                }
                                br -= 1;
                            }
                            b';' if p == 0 && br == 0 => {
                                cur += 1;
                                break;
                            }
                            b'\n' if is_kotlin && p == 0 && br == 0 => break,
                            _ => {}
                        }
                        cur += 1;
                    }
                } else if c == b';' {
                    cur += 1;
                }
                out.push(MemberDecl {
                    annotations: std::mem::take(&mut pending_anns),
                    is_field: true,
                    is_ctor: false,
                    is_private: is_priv,
                    is_final: is_fin,
                    is_static: is_stat,
                    return_or_field_type: ftype,
                    type_range,
                    name: fname,
                    name_start: ns,
                    name_end: ne,
                    decl_start,
                    decl_end: cur,
                    modifier_private_or_final_range: mod_range,
                    params: Vec::new(),
                    body_range_bytes: None,
                });
                header_tokens.clear();
                header_start = None;
                pending_anns.clear();
                i = cur.max(i + 1);
                continue;
            }
            header_tokens.clear();
            header_start = None;
            pending_anns.clear();
            i += 1;
            continue;
        }
        if c == b'{' {
            // Static initializer or nested class block: skip over cleanly
            let end_b = find_matching_delim(text, i, b'{', b'}').unwrap_or(body_end);
            header_tokens.clear();
            header_start = None;
            pending_anns.clear();
            i = (end_b + 1).min(body_end);
            continue;
        }

        // Read identifier or type token including `<...>`
        if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'<' | b'>' | b'.' | b':' | b'[' | b']' | b'?') {
            if header_start.is_none() {
                header_start = Some(i);
            }
            let tok_start = i;
            if c == b'<' {
                if let Some(close_g) = find_matching_delim(text, i, b'<', b'>') {
                    if let Some(last) = header_tokens.last_mut() {
                        last.0.push_str(&text[tok_start..=close_g]);
                        last.2 = close_g + 1;
                    }
                    i = close_g + 1;
                    continue;
                }
            }
            while i < body_end
                && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'.' | b'?' | b'[' | b']'))
            {
                i += 1;
            }
            if i == tok_start {
                i += 1;
            } else {
                header_tokens.push((text[tok_start..i].to_owned(), tok_start, i));
            }
            continue;
        }
        i += 1;
    }
    out
}

fn interpret_callable_header(
    tokens: &[(String, usize, usize)],
    class_name: &str,
    kt_ret: &str,
    is_kotlin: bool,
    text: &str,
    idx: &LineIndex,
) -> Option<(String, usize, usize, String, bool, bool, bool, Option<Range>)> {
    if tokens.is_empty() {
        return None;
    }
    let last = tokens.last()?;
    // Avoid misinterpreting control keywords like `if (...)` if any leaked
    if matches!(last.0.as_str(), "if" | "for" | "while" | "switch" | "catch" | "return") {
        return None;
    }
    let is_priv = tokens.iter().any(|(t, _, _)| t == "private");
    let is_fin = tokens.iter().any(|(t, _, _)| t == "final");
    let mod_range = tokens
        .iter()
        .find(|(t, _, _)| t == "private" || t == "final")
        .map(|(_, s, e)| idx.range_from_bytes(text, *s, *e));
    let is_ctor = last.0 == class_name || (is_kotlin && last.0 == "constructor");

    let rtype = if is_kotlin {
        if kt_ret.is_empty() { "Unit".to_owned() } else { kt_ret.to_owned() }
    } else if is_ctor {
        class_name.to_owned()
    } else {
        // Return type is the token right before the method name (ignoring modifiers)
        let mut r = String::new();
        for (t, _, _) in &tokens[..tokens.len().saturating_sub(1)] {
            if !is_java_modifier(t) && !t.starts_with('<') {
                r = t.clone();
            }
        }
        r
    };
    Some((last.0.clone(), last.1, last.2, rtype, is_ctor, is_priv, is_fin, mod_range))
}

fn interpret_field_header(
    tokens: &[(String, usize, usize)],
    is_kotlin: bool,
    text: &str,
    idx: &LineIndex,
) -> Option<(String, usize, usize, String, Range, bool, bool, bool, Option<Range>)> {
    if tokens.len() < 2 {
        return None;
    }
    let is_priv = tokens.iter().any(|(t, _, _)| t == "private");
    let is_fin = tokens.iter().any(|(t, _, _)| t == "final" || t == "val");
    let is_stat = tokens.iter().any(|(t, _, _)| t == "static" || t == "const");
    let mod_range = tokens
        .iter()
        .find(|(t, _, _)| t == "private" || t == "final")
        .map(|(_, s, e)| idx.range_from_bytes(text, *s, *e));

    if is_kotlin {
        // `private val userRepo: UserRepository` or `lateinit var userRepo: UserRepository`
        if let Some(colon_idx) = tokens.iter().position(|(t, _, _)| t == ":") {
            if colon_idx >= 1 && colon_idx + 1 < tokens.len() {
                let (name, ns, ne) = &tokens[colon_idx - 1];
                let (ty, ts, te) = &tokens[colon_idx + 1];
                return Some((
                    name.clone(),
                    *ns,
                    *ne,
                    ty.clone(),
                    idx.range_from_bytes(text, *ts, *te),
                    is_priv,
                    is_fin,
                    is_stat,
                    mod_range,
                ));
            }
        }
        return None;
    }

    let (fname, ns, ne) = tokens.last()?;
    if is_java_modifier(fname) || matches!(fname.as_str(), "return" | "throw") {
        return None;
    }
    let mut ftype = None;
    for (t, ts, te) in tokens[..tokens.len() - 1].iter().rev() {
        if !is_java_modifier(t) {
            ftype = Some((t.clone(), idx.range_from_bytes(text, *ts, *te)));
            break;
        }
    }
    let (ty, tr) = ftype?;
    Some((
        fname.clone(),
        *ns,
        *ne,
        ty,
        tr,
        is_priv,
        is_fin,
        is_stat,
        mod_range,
    ))
}

fn is_java_modifier(t: &str) -> bool {
    matches!(
        t,
        "public"
            | "private"
            | "protected"
            | "static"
            | "final"
            | "transient"
            | "volatile"
            | "synchronized"
            | "abstract"
            | "default"
            | "native"
            | "strictfp"
            | "suspend"
            | "override"
            | "open"
            | "internal"
            | "fun"
    )
}

fn parse_param_list(
    text: &str,
    start: usize,
    end: usize,
    is_kotlin: bool,
    idx: &LineIndex,
) -> Vec<ParsedParam> {
    let slice = &text[start..end];
    if slice.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut offset = start;
    for chunk in split_top_level_commas(slice) {
        let chunk_start = offset;
        offset += chunk.len() + 1;
        let trimmed = chunk.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lead = chunk.len() - chunk.trim_start().len();
        let abs_start = chunk_start + lead;
        let mut anns = Vec::new();
        let mut cur = abs_start;
        let abs_end = chunk_start + chunk.len();
        let b = text.as_bytes();
        while cur < abs_end {
            cur = skip_ws_and_comments(text, cur);
            if b.get(cur) == Some(&b'@')
                && let Some(ann) = read_annotation(text, cur)
            {
                cur = ann.end_byte;
                anns.push(ann);
                continue;
            }
            break;
        }
        let rest = text[cur..abs_end].trim();
        if is_kotlin {
            if let Some((left, right)) = rest.split_once(':') {
                let pname = left
                    .split_whitespace()
                    .last()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let ptype = right.split('=').next().unwrap_or(right).trim().to_owned();
                if !pname.is_empty() && !ptype.is_empty() {
                    let r = idx.range_from_bytes(text, cur, abs_end);
                    out.push(ParsedParam {
                        annotations: anns,
                        name: pname,
                        type_name: ptype,
                        type_range: r,
                        range: r,
                    });
                }
            }
        } else {
            let mut toks = Vec::new();
            let mut ti = cur;
            while ti < abs_end {
                ti = skip_ws_and_comments(text, ti);
                if ti >= abs_end {
                    break;
                }
                let ts = ti;
                if b[ti] == b'<'
                    && let Some(cg) = find_matching_delim(text, ti, b'<', b'>')
                {
                    if let Some(last) = toks.last_mut() {
                        let prev: &mut (String, usize, usize) = last;
                        prev.0.push_str(&text[ts..=cg]);
                        prev.2 = cg + 1;
                    }
                    ti = cg + 1;
                    continue;
                }
                while ti < abs_end && !b[ti].is_ascii_whitespace() {
                    ti += 1;
                }
                if ti > ts {
                    let w = &text[ts..ti];
                    if w != "final" {
                        toks.push((w.to_owned(), ts, ti));
                    }
                }
            }
            if toks.len() >= 2 {
                let (pname, ns, ne) = toks.last().unwrap().clone();
                let (ptype, ts, te) = toks[toks.len() - 2].clone();
                out.push(ParsedParam {
                    annotations: anns,
                    name: pname,
                    type_name: ptype,
                    type_range: idx.range_from_bytes(text, ts, te),
                    range: idx.range_from_bytes(text, ns, ne),
                });
            }
        }
    }
    out
}

/// Refactoring: Convert all `@Autowired` field injections in a Java class to clean constructor injection.
pub fn convert_field_injection_to_constructor(text: &str) -> Option<Vec<TextEdit>> {
    let idx = LineIndex::new(text);
    let classes = find_top_classes(text, false, &idx);
    let cls = classes.first()?;
    let members = parse_members(text, cls.body_start, cls.body_end, &cls.name, false, &idx);

    let field_injections: Vec<&MemberDecl> = members
        .iter()
        .filter(|m| {
            m.is_field
                && !m.is_static
                && (has_ann(&m.annotations, "Autowired")
                    || has_ann(&m.annotations, "Inject")
                    || has_ann(&m.annotations, "Resource"))
        })
        .collect();
    if field_injections.is_empty() {
        return None;
    }

    let mut edits = Vec::new();
    let mut ctor_params = Vec::new();
    let mut ctor_assigns = Vec::new();
    let mut last_field_end = cls.body_start;

    for m in &field_injections {
        last_field_end = last_field_end.max(m.decl_end);
        // Remove `@Autowired` / `@Inject` / `@Resource` annotation on the field
        for ann in &m.annotations {
            if matches!(ann.name.as_str(), "Autowired" | "Inject" | "Resource") {
                // Remove line whitespace after annotation if it's on its own line
                let mut remove_end = ann.end_byte;
                while text.as_bytes().get(remove_end).is_some_and(|b| *b == b' ' || *b == b'\t') {
                    remove_end += 1;
                }
                if text.as_bytes().get(remove_end) == Some(&b'\r') {
                    remove_end += 1;
                }
                if text.as_bytes().get(remove_end) == Some(&b'\n') {
                    remove_end += 1;
                    while text.as_bytes().get(remove_end).is_some_and(|b| *b == b' ' || *b == b'\t') {
                        remove_end += 1;
                    }
                }
                edits.push(TextEdit {
                    range: idx.range_from_bytes(text, ann.start_byte, remove_end),
                    text: String::new(),
                });
            }
        }
        // Add `final` keyword if not already final
        if !m.is_final {
            let type_byte = idx.pos_to_byte(text, m.type_range.start);
            edits.push(TextEdit {
                range: idx.range_from_bytes(text, type_byte, type_byte),
                text: "final ".to_owned(),
            });
        }
        let qual_prefix = find_ann(&m.annotations, "Qualifier")
            .and_then(|q| extract_string_attr(q, &["value"]))
            .map(|q| format!("@Qualifier(\"{q}\") "))
            .unwrap_or_default();
        ctor_params.push(format!("{qual_prefix}{} {}", m.return_or_field_type, m.name));
        ctor_assigns.push(format!("        this.{} = {};", m.name, m.name));
    }

    // Insert constructor right after the last converted field
    let ctor_text = format!(
        "\n\n    public {}({}) {{\n{}\n    }}",
        cls.name,
        ctor_params.join(", "),
        ctor_assigns.join("\n")
    );
    edits.push(TextEdit {
        range: idx.range_from_bytes(text, last_field_end, last_field_end),
        text: ctor_text,
    });

    // Sort edits cleanly and ensure non-overlapping
    edits.sort_by(|a, b| a.range.start.cmp(&b.range.start).then(a.range.end.cmp(&b.range.end)));
    Some(edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spring_service_controller_and_repository() {
        let java = r#"
package com.example.demo;

import org.springframework.stereotype.Service;
import org.springframework.beans.factory.annotation.Autowired;

@Service
@Profile("prod")
public class OrderService {
    @Autowired
    @Qualifier("primaryRepo")
    private OrderRepository orderRepository;

    @Value("${app.tax-rate:0.08}")
    private double taxRate;

    @Transactional
    private void helperTx() {}

    @Scheduled(cron = "0 */5 * * * *")
    public void sync() {}
}
"#;
        let facts = parse_jvm_source("/p/OrderService.java", "java", java);
        assert_eq!(facts.beans.len(), 1);
        assert_eq!(facts.beans[0].stereotype, BeanStereotype::Service);
        assert_eq!(facts.beans[0].profiles, ["prod"]);
        assert_eq!(facts.injections.len(), 1);
        assert_eq!(facts.injections[0].target_type, "OrderRepository");
        assert_eq!(facts.injections[0].qualifier.as_ref().unwrap().0, "primaryRepo");
        assert_eq!(facts.value_sites.len(), 1);
        assert_eq!(facts.value_sites[0].prop_key.as_deref(), Some("app.tax-rate"));
        assert_eq!(facts.scheduled_sites.len(), 1);
        assert_eq!(facts.scheduled_sites[0].cron.as_deref(), Some("0 */5 * * * *"));
        assert_eq!(facts.proxy_issues.len(), 1);
        assert_eq!(facts.proxy_issues[0].kind, ProxyIssueKind::PrivateMethod);
    }

    #[test]
    fn converts_field_injection_to_constructor_injection() {
        let src = "@Service\npublic class UserService {\n    @Autowired\n    private UserRepo userRepo;\n}\n";
        let edits = convert_field_injection_to_constructor(src).expect("edits");
        // Verify edits contain removing @Autowired, adding final, and inserting constructor
        assert!(edits.iter().any(|e| e.text.is_empty()));
        assert!(edits.iter().any(|e| e.text == "final "));
        assert!(edits.iter().any(|e| e.text.contains("public UserService(UserRepo userRepo)")));
    }
}
