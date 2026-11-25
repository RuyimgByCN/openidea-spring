pub mod boot_metadata;
pub mod console_rules;
pub mod data_jpa;
pub mod diagnostics;
pub mod endpoints;
pub mod hover_inlay;
pub mod index;
pub mod language_features;
pub mod model;
pub mod parser_config;
pub mod parser_java;
pub mod parser_xml;
pub mod providers;
pub mod runtime;
pub mod search;
pub mod spel_cron;
pub mod text_util;

use crate::console_rules::spring_console_rules;
use crate::data_jpa::parse_derived_query;
use crate::endpoints::{
    endpoints_for_host_provider, format_curl, join_context_path, resolve_base_url,
    unified_endpoints_for_panel,
};
use crate::hover_inlay::InlayPrefs;
use crate::index::{
    ProjectCache, build_document_index_edges, compute_bean_graph, detect_language_from_path,
    resolve_injection, scan_project, should_index_path,
};
use crate::model::{EndpointDef, SpringProjectModel};
use crate::parser_config::{
    properties_to_yaml, render_effective_for_diff, resolve_effective_properties, yaml_to_properties,
};
use crate::parser_java::convert_field_injection_to_constructor;
use crate::runtime::{
    RuntimeAppInstance, extract_runtime_mapping_paths, parse_actuator_urls, parse_port_range,
    probe_runtime_instances, render_service_details, render_services_view,
    set_runtime_logger_level, sync_datasource_to_openidea,
};
use crate::search::search_category;
use crate::spel_cron::{parse_and_explain_cron, validate_spel};
use crate::text_util::{LineIndex, position_contains, quoted_string_at};

use openidea_plugin::details::{Details, Tab};
use openidea_plugin::diagnostics::{Diagnostic, DiagnosticsProvider, Document as DiagnosticsDoc};
use openidea_plugin::endpoints::{Endpoint, EndpointsProvider};
use openidea_plugin::file_actions::{FileAction, FileActions};
use openidea_plugin::hover::{Document as HoverDoc, Hover, HoverProvider};
use openidea_plugin::indexing::{Document as IndexDoc, DocumentIndexer, Edges};
use openidea_plugin::inlay_hints::{InlayHint, InlayHints};
use openidea_plugin::language_features::{
    Document as LangDoc, DocumentLink, GutterAction, Injection, Injections, LanguageFeatures,
};
use openidea_plugin::providers::{CodeAction, CompletionItem, Document as ProviderDoc, LineMarker, Providers};
use openidea_plugin::search::{SearchPage, SearchProvider};
use openidea_plugin::views::{View, ViewEvent, Viewport, Views};
use openidea_plugin::{
    ContentDiff, ContentTab, Context, Error, Event, EventKind, Invocation, Location,
    NotificationLevel, PickItem, Plugin, Position, Project, Range, StatusItem, StatusMenuEntry,
    host, register_plugin,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const STORAGE_PROFILES_KEY: &str = "spring.activeProfiles";
const STORAGE_ACTUATOR_URLS_KEY: &str = "spring.customActuatorUrls";
const STORAGE_CONSOLE_HIGHLIGHT_KEY: &str = "spring.consoleHighlightOverride";
const STATUS_BAR_ID: &str = "spring-profile";

#[derive(Default)]
pub struct SpringPlugin {
    cache: ProjectCache,
    active_profiles: BTreeMap<String, String>,
    runtime_instances: Vec<RuntimeAppInstance>,
    custom_actuator_urls: Vec<String>,
    console_highlight_override: Option<bool>,
    pending_panel_action: Option<Value>,
    last_project_root: Option<String>,
}

fn effective_locale() -> String {
    match host::setting_string("language").as_deref() {
        Some("zh-CN") => "zh-CN".to_owned(),
        Some("en") => "en".to_owned(),
        _ => host::locale(),
    }
}

fn is_zh_locale(locale: &str) -> bool {
    locale.to_ascii_lowercase().starts_with("zh")
}

fn is_zh() -> bool {
    is_zh_locale(&effective_locale())
}

impl SpringPlugin {
    fn is_console_highlighting_enabled(&self) -> bool {
        self.console_highlight_override
            .unwrap_or_else(|| host::setting_bool("consoleHighlighting").unwrap_or(true))
    }

    fn sync_console_rules(&self) {
        if self.is_console_highlighting_enabled() {
            let _ = host::set_console_rules(&spring_console_rules());
        } else {
            let _ = host::set_console_rules(&[]);
        }
    }

    fn restore_storage(&mut self) {
        if let Ok(Some(raw)) = host::storage::get(STORAGE_PROFILES_KEY)
            && let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(&raw)
        {
            self.active_profiles = map;
        }
        if let Ok(Some(raw)) = host::storage::get(STORAGE_ACTUATOR_URLS_KEY)
            && let Ok(urls) = serde_json::from_str::<Vec<String>>(&raw)
        {
            self.custom_actuator_urls = urls;
        }
        if let Ok(Some(raw)) = host::storage::get(STORAGE_CONSOLE_HIGHLIGHT_KEY) {
            self.console_highlight_override = match raw.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
        }
    }

    fn persist_profiles(&self) {
        if let Ok(json_str) = serde_json::to_string(&self.active_profiles) {
            let _ = host::storage::set(STORAGE_PROFILES_KEY, &json_str);
        }
    }

    fn persist_custom_urls(&self) {
        if let Ok(json_str) = serde_json::to_string(&self.custom_actuator_urls) {
            let _ = host::storage::set(STORAGE_ACTUATOR_URLS_KEY, &json_str);
        }
    }

    fn persist_console_override(&self) {
        match self.console_highlight_override {
            Some(v) => {
                let _ = host::storage::set(STORAGE_CONSOLE_HIGHLIGHT_KEY, if v { "true" } else { "false" });
            }
            None => {
                let _ = host::storage::delete(STORAGE_CONSOLE_HIGHLIGHT_KEY);
            }
        }
    }

    fn resolve_project(&mut self, ctx: Option<&Invocation<'_>>) -> Option<Project> {
        if let Some(c) = ctx
            && let Some(p) = c.project()
        {
            if let Ok(r) = p.root() {
                self.last_project_root = Some(r);
            }
            return Some(p);
        }
        if let Some(c) = ctx
            && let Some(ed) = c.editor()
            && let Some(p) = host::project_of(&ed.path())
        {
            if let Ok(r) = p.root() {
                self.last_project_root = Some(r);
            }
            return Some(p);
        }
        if let Ok(list) = host::projects()
            && let Some(first) = list.into_iter().next()
        {
            if let Ok(r) = first.root() {
                self.last_project_root = Some(r);
            }
            return Some(first);
        }
        None
    }

    fn current_root(&self) -> Option<String> {
        if let Some(r) = &self.last_project_root {
            return Some(r.clone());
        }
        self.cache.projects.keys().next().cloned()
    }

    fn active_profile_for_root(&self, root: Option<&str>) -> Option<String> {
        if let Some(r) = root
            && let Some(prof) = self.active_profiles.get(r)
            && !prof.trim().is_empty()
        {
            return Some(prof.clone());
        }
        host::setting_string("defaultProfile").filter(|s| !s.trim().is_empty())
    }

    fn ensure_project_scanned(&mut self, project: &Project, force: bool, show_task: bool) -> String {
        let root = project
            .root()
            .unwrap_or_else(|_| self.last_project_root.clone().unwrap_or_default());
        self.last_project_root = Some(root.clone());
        let needs_scan = force
            || self
                .cache
                .projects
                .get(&root)
                .is_none_or(|m| m.files.is_empty());
        if needs_scan {
            let prof = self.active_profile_for_root(Some(&root));
            scan_project(&mut self.cache, project, prof.as_deref(), show_task);
        }
        root
    }

    fn ensure_active_scanned(&mut self, ctx: Option<&Invocation<'_>>, force: bool, show_task: bool) -> Option<String> {
        if let Some(project) = self.resolve_project(ctx) {
            Some(self.ensure_project_scanned(&project, force, show_task))
        } else {
            self.current_root()
        }
    }

    fn update_status_bar(&self, root: Option<&str>) {
        let zh = is_zh();
        let active = self.active_profile_for_root(root);
        let label = active.as_deref().unwrap_or("default");
        let item = StatusItem {
            text: format!("Spring: {label}"),
            tooltip: Some(if zh {
                format!("Spring 当前配置 Profile：{label}（点击切换或打开面板）")
            } else {
                format!("Spring Active Profile: {label} (Click for Spring menu)")
            }),
            command: Some("switch-profile".into()),
        };

        let mut entries = vec![
            StatusMenuEntry::action(
                if zh { "打开 Spring 工具面板" } else { "Open Spring Panel" },
                "open",
            ),
            StatusMenuEntry::action(
                if zh { "切换激活 Profile..." } else { "Switch Active Profile..." },
                "switch-profile",
            )
            .with_description(label),
            StatusMenuEntry::action(
                if zh { "对比两个 Profile (Diff)..." } else { "Compare Profiles (Diff)..." },
                "compare-profiles",
            ),
            StatusMenuEntry::Separator,
        ];

        if let Some(r) = root
            && let Some(model) = self.cache.projects.get(r)
        {
            let profiles = model.all_profiles();
            if !profiles.is_empty() {
                entries.push(StatusMenuEntry::Header(
                    if zh { "快速切换 Profile" } else { "Quick Profiles" }.into(),
                ));
                entries.push(
                    StatusMenuEntry::action("default", "switch-profile")
                        .with_argument(""),
                );
                for p in profiles.into_iter().take(12) {
                    entries.push(
                        StatusMenuEntry::action(&p, "switch-profile")
                            .with_argument(&p),
                    );
                }
                entries.push(StatusMenuEntry::Separator);
            }
        }

        entries.push(StatusMenuEntry::Header(
            if zh { "运行时与工具" } else { "Runtime & Tools" }.into(),
        ));
        entries.push(StatusMenuEntry::action(
            if zh { "同步数据源到 Database" } else { "Sync Datasource to Database" },
            "sync-datasource",
        ));
        entries.push(StatusMenuEntry::action(
            if zh { "刷新运行时实例 (Agent / Actuator)" } else { "Refresh Runtime Instances" },
            "refresh-runtime",
        ));
        entries.push(StatusMenuEntry::action(
            if self.is_console_highlighting_enabled() {
                if zh { "关闭启动控制台高亮" } else { "Disable Console Highlighting" }
            } else if zh {
                "开启启动控制台高亮"
            } else {
                "Enable Console Highlighting"
            },
            "toggle-console-highlight",
        ));

        let _ = host::set_status_item_menu(STATUS_BAR_ID, &item, &entries);
    }

    fn all_actuator_urls(&self) -> Vec<String> {
        let mut urls = parse_actuator_urls(host::setting_string("actuatorUrls").as_deref());
        for custom in &self.custom_actuator_urls {
            let trimmed = custom.trim().trim_end_matches('/').to_owned();
            if !trimmed.is_empty() && !urls.contains(&trimmed) {
                urls.push(trimmed);
            }
        }
        urls
    }

    fn probe_runtime(&mut self, root: Option<&str>) {
        let range = parse_port_range(host::setting_string("agentPortRange").as_deref());
        let urls = self.all_actuator_urls();
        self.runtime_instances = probe_runtime_instances(root, range, &urls);
    }

    fn set_profile(&mut self, root: Option<String>, profile: Option<String>) {
        if let Some(r) = root.clone() {
            match profile.as_deref().map(str::trim) {
                None | Some("") | Some("default") => {
                    self.active_profiles.remove(&r);
                }
                Some(p) => {
                    self.active_profiles.insert(r.clone(), p.to_owned());
                }
            }
            self.persist_profiles();
            let eff_prof = self.active_profile_for_root(Some(&r));
            if let Some(model) = self.cache.projects.get_mut(&r) {
                let (edges, cycles) = compute_bean_graph(model, eff_prof.as_deref());
                model.bean_edges = edges;
                model.cycles = cycles;
            }
            self.update_status_bar(Some(&r));
        }
        host::refresh_view("spring-services");
        self.push_snapshot_to_webview(root.as_deref());
    }

    fn push_snapshot_to_webview(&mut self, root: Option<&str>) {
        let snapshot = self.build_snapshot_json(root);
        let msg = json!({
            "op": "snapshot",
            "snapshot": snapshot,
        });
        let _ = host::post_view_message("spring", &msg.to_string());
    }

    fn open_webview_tab(&mut self, tab: &str, extra: Value, ctx: Option<&Invocation<'_>>) {
        let _ = self.ensure_active_scanned(ctx, false, false);
        let payload = json!({
            "tab": tab,
            "extra": extra,
        });
        self.pending_panel_action = Some(payload.clone());
        let _ = host::reveal_view("spring");
        let _ = host::post_view_message(
            "spring",
            &json!({
                "op": "open-tab",
                "tab": tab,
                "extra": extra,
                "snapshot": self.build_snapshot_json(self.current_root().as_deref()),
            })
            .to_string(),
        );
    }

    fn build_snapshot_json(&self, root: Option<&str>) -> Value {
        let locale = effective_locale();
        let zh = is_zh_locale(&locale);
        let target_root = root
            .map(str::to_owned)
            .or_else(|| self.current_root())
            .unwrap_or_default();
        let active_profile = if target_root.is_empty() {
            self.active_profile_for_root(None)
        } else {
            self.active_profile_for_root(Some(&target_root))
        };

        let Some(model) = self.cache.projects.get(&target_root) else {
            return json!({
                "projectRoot": target_root,
                "activeProfile": active_profile,
                "discoveredProfiles": ["default"],
                "locale": locale,
                "consoleHighlighting": self.is_console_highlighting_enabled(),
                "starters": { "badges": [] },
                "beans": [],
                "beanEdges": [],
                "cycles": [],
                "endpoints": [],
                "configFiles": [],
                "effectiveProperties": [],
                "configPropClasses": [],
                "entities": [],
                "repositories": [],
                "scheduledSites": [],
                "spelSites": [],
                "runtimeInstances": self.runtime_instances,
            });
        };

        let mut discovered_profiles = vec!["default".to_owned()];
        for p in model.all_profiles() {
            if !discovered_profiles.contains(&p) {
                discovered_profiles.push(p);
            }
        }
        if let Some(ap) = &active_profile
            && !discovered_profiles.contains(ap)
        {
            discovered_profiles.push(ap.clone());
        }

        // Map owner -> outgoing injections for quick bean detail cards
        let all_inj = model.all_injections();
        let beans_json: Vec<Value> = model
            .all_beans()
            .into_iter()
            .map(|b| {
                let deps: Vec<Value> = all_inj
                    .iter()
                    .filter(|inj| inj.owner_bean == b.name || inj.owner_class == b.class_name)
                    .map(|inj| {
                        let (status, targets) =
                            resolve_injection(model, inj, active_profile.as_deref());
                        json!({
                            "field": inj.field_or_param_name,
                            "targetType": inj.target_type,
                            "kind": inj.kind,
                            "status": status,
                            "qualifier": inj.qualifier.as_ref().map(|(q, _)| q),
                            "resolvedIds": targets.into_iter().map(|t| t.bean_id).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                json!({
                    "id": b.id,
                    "name": b.name,
                    "className": b.class_name,
                    "classFqn": b.class_fqn,
                    "package": b.package,
                    "stereotype": b.stereotype,
                    "stereotypeLabel": b.stereotype.label(),
                    "scope": b.scope,
                    "isPrimary": b.is_primary,
                    "isLazy": b.is_lazy,
                    "profiles": b.profiles,
                    "qualifiers": b.qualifiers.iter().map(|(q, _)| q).collect::<Vec<_>>(),
                    "file": b.file,
                    "line": b.line,
                    "dependencies": deps,
                })
            })
            .collect();

        let runtime_paths = extract_runtime_mapping_paths(&self.runtime_instances);
        let endpoints = unified_endpoints_for_panel(model, active_profile.as_deref(), &runtime_paths);

        let config_files_json: Vec<Value> = model
            .all_config_files()
            .into_iter()
            .map(|cf| {
                json!({
                    "path": cf.path,
                    "profile": cf.profile,
                    "format": format!("{:?}", cf.format).to_ascii_lowercase(),
                    "entryCount": cf.entries.len(),
                })
            })
            .collect();

        let cf_refs = model.all_config_files();
        let eff_props = resolve_effective_properties(&cf_refs, active_profile.as_deref());
        // Never transmit raw secret property values over the UI bridge; send only `maskedValue`.
        let eff_json: Vec<Value> = eff_props
            .into_iter()
            .map(|p| {
                json!({
                    "key": p.key,
                    "relaxedKey": p.relaxed_key,
                    "maskedValue": p.masked_value,
                    "sourceFile": p.source_file,
                    "sourceProfile": p.source_profile,
                    "line": p.line,
                    "overriddenFrom": p.overridden_from,
                })
            })
            .collect();

        let cp_classes_json: Vec<Value> = model
            .all_config_prop_classes()
            .into_iter()
            .map(|c| {
                json!({
                    "prefix": c.prefix,
                    "className": c.class_name,
                    "file": c.file,
                    "line": c.range.start.line,
                    "fields": c.fields.iter().map(|f| json!({
                        "name": f.name,
                        "fullKey": f.full_key,
                        "typeName": f.type_name,
                        "defaultValue": f.default_value,
                        "line": f.range.start.line,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();

        let entities = model.all_entities();
        let entities_json: Vec<Value> = entities
            .iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "fqn": e.fqn,
                    "tableName": e.table_name,
                    "file": e.file,
                    "line": e.range.start.line,
                    "fields": e.fields.iter().map(|f| json!({
                        "name": f.name,
                        "typeName": f.type_name,
                        "columnName": f.column_name,
                        "isId": f.is_id,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();

        let repos_json: Vec<Value> = model
            .all_repositories()
            .into_iter()
            .map(|r| {
                let methods: Vec<Value> = r
                    .methods
                    .iter()
                    .map(|m| {
                        let errors = crate::data_jpa::validate_repository_method(r, m, &entities);
                        let derived = if m.query.is_none() {
                            parse_derived_query(&m.name).map(|ast| ast.to_jpql_preview(&r.entity_type))
                        } else {
                            None
                        };
                        json!({
                            "name": m.name,
                            "returnType": m.return_type,
                            "line": m.line,
                            "query": m.query.as_ref().map(|q| json!({
                                "text": q.query,
                                "native": q.native,
                            })),
                            "jpqlPreview": derived,
                            "valid": errors.is_empty(),
                            "errors": errors,
                        })
                    })
                    .collect();
                json!({
                    "name": r.name,
                    "baseInterface": r.base_interface,
                    "entityType": r.entity_type,
                    "idType": r.id_type,
                    "file": r.file,
                    "line": r.line,
                    "methods": methods,
                })
            })
            .collect();

        let mut scheduled_json = Vec::new();
        let mut spel_json = Vec::new();
        for f in model.files.values() {
            for s in &f.scheduled_sites {
                let cron_exp = s.cron.as_deref().map(parse_and_explain_cron);
                let summary = cron_exp.as_ref().map(|c| {
                    if zh {
                        c.summary_zh.clone()
                    } else {
                        c.summary_en.clone()
                    }
                });
                let valid = cron_exp.as_ref().is_none_or(|c| c.valid);
                let error = cron_exp.and_then(|c| c.error);
                scheduled_json.push(json!({
                    "methodName": s.method_name,
                    "cron": s.cron,
                    "fixedRate": s.fixed_rate,
                    "fixedDelay": s.fixed_delay,
                    "zone": s.zone,
                    "summary": summary,
                    "valid": valid,
                    "error": error,
                    "file": s.file,
                    "line": s.line,
                }));
            }
            for sp in &f.spel_sites {
                let v = validate_spel(&sp.expr);
                spel_json.push(json!({
                    "expr": sp.expr,
                    "referencedBeans": sp.bean_refs.iter().map(|(b, _)| b).collect::<Vec<_>>(),
                    "valid": v.valid,
                    "summary": v.summary,
                    "error": v.error,
                    "file": sp.file,
                    "line": sp.line,
                }));
            }
        }

        let mut starters_val = serde_json::to_value(&model.starters).unwrap_or(json!({}));
        if let Some(obj) = starters_val.as_object_mut() {
            obj.insert("badges".into(), json!(model.starters.badges()));
        }

        json!({
            "projectRoot": target_root,
            "activeProfile": active_profile,
            "discoveredProfiles": discovered_profiles,
            "locale": locale,
            "consoleHighlighting": self.is_console_highlighting_enabled(),
            "starters": starters_val,
            "beans": beans_json,
            "beanEdges": model.bean_edges,
            "cycles": model.cycles,
            "endpoints": endpoints,
            "configFiles": config_files_json,
            "effectiveProperties": eff_json,
            "configPropClasses": cp_classes_json,
            "entities": entities_json,
            "repositories": repos_json,
            "scheduledSites": scheduled_json,
            "spelSites": spel_json,
            "runtimeInstances": self.runtime_instances,
        })
    }

    fn find_endpoint_for_command(&mut self, ctx: &Invocation<'_>, arg: Option<&str>) -> Option<(EndpointDef, String, String)> {
        let root = self.ensure_active_scanned(Some(ctx), false, false)?;
        let active_profile = self.active_profile_for_root(Some(&root));
        let model = self.cache.projects.get(&root)?;
        let (base_host, ctx_path) = resolve_base_url(model, active_profile.as_deref());

        if let Some(a) = arg
            && let Some((path, line_str)) = a.rsplit_once(':')
            && let Ok(line) = line_str.parse::<u32>()
            && let Some(facts) = model.files.get(path)
            && let Some(ep) = facts.endpoints.iter().find(|e| e.line == line)
        {
            return Some((ep.clone(), base_host, ctx_path));
        }

        if let Some(ed) = ctx.editor()
            && let Ok(doc) = ed.document()
        {
            self.cache.update_file(
                &root,
                &ed.path(),
                &ed.language(),
                &doc.text,
                active_profile.as_deref(),
            );
            let caret_line = ed
                .selections()
                .ok()
                .and_then(|s| s.into_iter().next())
                .map(|r| r.start.line)
                .unwrap_or(0);
            if let Some(model) = self.cache.projects.get(&root)
                && let Some(facts) = model.files.get(&ed.path())
            {
                let ep = facts
                    .endpoints
                    .iter()
                    .find(|e| e.line == caret_line || position_contains(e.range, Position::new(caret_line, 0)))
                    .or_else(|| {
                        facts
                            .endpoints
                            .iter()
                            .min_by_key(|e| e.line.abs_diff(caret_line))
                    })?;
                return Some((ep.clone(), base_host, ctx_path));
            }
        }
        None
    }

    fn prefs_inlay() -> InlayPrefs {
        InlayPrefs {
            enabled: host::setting_bool("inlayHintsEnabled").unwrap_or(true),
            value_hints: host::setting_bool("showValueInlayHints").unwrap_or(true),
            bean_hints: host::setting_bool("showBeanInlayHints").unwrap_or(true),
            endpoint_hints: host::setting_bool("showEndpointInlayHints").unwrap_or(true),
            cron_hints: host::setting_bool("showCronInlayHints").unwrap_or(true),
        }
    }

    fn doc_profile(&mut self, path: &str, language: &str, text: &str) -> Option<String> {
        if let Some(p) = host::project_of(path)
            && let Ok(root) = p.root()
        {
            self.last_project_root = Some(root.clone());
            if self
                .cache
                .projects
                .get(&root)
                .is_none_or(|m| m.files.is_empty())
            {
                let prof = self.active_profile_for_root(Some(&root));
                scan_project(&mut self.cache, &p, prof.as_deref(), false);
            }
            return self.active_profile_for_root(Some(&root));
        }
        let prof = self.active_profile_for_root(self.current_root().as_deref());
        let _ = (path, language, text);
        prof
    }

    fn cmd_switch_profile(&mut self, ctx: &Invocation<'_>, arg: Option<&str>) -> Result<Option<String>, Error> {
        let root = self.ensure_active_scanned(Some(ctx), false, false);
        let zh = is_zh();
        let chosen = if let Some(a) = arg {
            Some(a.trim().to_owned())
        } else {
            let mut items = vec![PickItem::new("", "default (no profile override)")];
            if let Some(r) = &root
                && let Some(model) = self.cache.projects.get(r)
            {
                for p in model.all_profiles() {
                    items.push(
                        PickItem::new(&p, &p)
                            .description(if zh { "激活该 Spring Profile" } else { "Activate Spring profile" }),
                    );
                }
            }
            items.push(
                PickItem::new("__custom__", if zh { "自定义 Profile 名称..." } else { "Enter custom profile name..." }),
            );
            match host::pick(
                if zh { "切换激活的 Spring Profile" } else { "Switch Active Spring Profile" },
                &items,
            )? {
                None => return Ok(None),
                Some(k) if k == "__custom__" => host::input(
                    if zh { "输入 Spring Profile（例如 dev,test，留空恢复默认）" } else { "Enter Spring profile (e.g. dev,test; empty for default)" },
                    self.active_profile_for_root(root.as_deref()).as_deref(),
                )?,
                Some(k) => Some(k),
            }
        };

        let Some(prof_str) = chosen else {
            return Ok(None);
        };
        let next = (!prof_str.trim().is_empty()).then(|| prof_str.trim().to_owned());
        self.set_profile(root, next.clone());
        let display = next.as_deref().unwrap_or("default");
        host::notify(
            NotificationLevel::Info,
            "Spring Boot",
            &if zh {
                format!("已切换激活 Profile 为 `{display}`。")
            } else {
                format!("Active Spring profile set to `{display}`.")
            },
        );
        Ok(Some(display.to_owned()))
    }

    fn cmd_compare_profiles(&mut self, ctx: &Invocation<'_>, arg: Option<&str>) -> Result<Option<String>, Error> {
        let root = self
            .ensure_active_scanned(Some(ctx), false, false)
            .ok_or_else(|| Error::NotFound("No open Spring project".into()))?;
        let zh = is_zh();
        let mut profiles = vec!["default".to_owned()];
        if let Some(model) = self.cache.projects.get(&root) {
            for p in model.all_profiles() {
                if !profiles.contains(&p) {
                    profiles.push(p);
                }
            }
        }

        let (left_p, right_p) = if let Some(pair) = arg
            && let Some((a, b)) = pair.split_once('|')
        {
            (a.trim().to_owned(), b.trim().to_owned())
        } else {
            let items: Vec<PickItem> = profiles.iter().map(|p| PickItem::new(p, p)).collect();
            let Some(l) = host::pick(
                if zh { "选择左侧基准 Profile" } else { "Select Left (Base) Profile" },
                &items,
            )?
            else {
                return Ok(None);
            };
            let Some(r) = host::pick(
                if zh { "选择右侧对比 Profile" } else { "Select Right (Target) Profile" },
                &items,
            )?
            else {
                return Ok(None);
            };
            (l, r)
        };

        let model = self
            .cache
            .projects
            .get(&root)
            .ok_or_else(|| Error::NotFound("Spring model unavailable".into()))?;
        let cf_refs = model.all_config_files();
        let p1 = (left_p != "default" && !left_p.is_empty()).then_some(left_p.as_str());
        let p2 = (right_p != "default" && !right_p.is_empty()).then_some(right_p.as_str());
        let eff1 = resolve_effective_properties(&cf_refs, p1);
        let eff2 = resolve_effective_properties(&cf_refs, p2);
        let diff = ContentDiff {
            left: render_effective_for_diff(&eff1),
            right: render_effective_for_diff(&eff2),
            left_label: Some(format!("Profile: {left_p}")),
            right_label: Some(format!("Profile: {right_p}")),
            title: Some(format!("Spring Effective Config Diff ({left_p} ↔ {right_p})")),
        };
        host::open_diff(&diff, Some("spring-profile-diff.properties"))?;
        Ok(Some(format!("{left_p}|{right_p}")))
    }

    fn cmd_convert_yaml_properties(&mut self, ctx: &Invocation<'_>) -> Result<Option<String>, Error> {
        let ed = ctx
            .editor()
            .ok_or_else(|| Error::NotFound("No focused editor".into()))?;
        let doc = ed.document()?;
        let lang = ed.language();
        let lower_path = ed.path().to_ascii_lowercase();
        let is_props = lang == "properties" || lower_path.ends_with(".properties");
        let (converted, target_ext, target_lang) = if is_props {
            match properties_to_yaml(&doc.text) {
                Ok(out) => (out, "yml", "yaml"),
                Err(msg) => {
                    host::notify(NotificationLevel::Warning, "Spring Config", &msg);
                    return Ok(None);
                }
            }
        } else {
            match yaml_to_properties(&doc.text) {
                Ok(out) => (out, "properties", "properties"),
                Err(msg) => {
                    host::notify(NotificationLevel::Warning, "Spring Config", &msg);
                    return Ok(None);
                }
            }
        };

        let ed_path = ed.path();
        let base_name = ed_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("application")
            .split('.')
            .next()
            .unwrap_or("application");
        let file_name = format!("{base_name}.converted.{target_ext}");
        host::open_content_tab(&ContentTab {
            key: format!("spring-convert:{file_name}"),
            title: file_name.clone(),
            file_name: Some(file_name),
            language: Some(target_lang.into()),
            content: converted,
            read_only: false,
            actions: Vec::new(),
        })?;
        Ok(Some(target_lang.into()))
    }

    fn cmd_convert_field_to_constructor(&mut self, ctx: &Invocation<'_>) -> Result<Option<String>, Error> {
        let zh = is_zh();
        let ed = ctx
            .editor()
            .ok_or_else(|| Error::NotFound("No focused editor".into()))?;
        let lang = ed.language();
        let lower_path = ed.path().to_ascii_lowercase();
        if lang != "java" && !lower_path.ends_with(".java") {
            host::notify(
                NotificationLevel::Info,
                "Spring Refactor",
                if zh {
                    "仅支持在 Java 文件中将字段注入转换为构造器注入。"
                } else {
                    "Field-to-constructor conversion only supports Java files."
                },
            );
            return Ok(None);
        }
        let doc = ed.document()?;
        let Some(edits) = convert_field_injection_to_constructor(&doc.text) else {
            host::notify(
                NotificationLevel::Info,
                "Spring Refactor",
                if zh {
                    "当前类包含已有构造函数或暂不支持安全转换的字段，已跳过修改。"
                } else {
                    "No safely convertible field injections found (existing constructors or unsupported field layouts are left unchanged)."
                },
            );
            return Ok(None);
        };
        ed.apply_edits(doc.version, &edits)?;
        host::notify(
            NotificationLevel::Info,
            "Spring Refactor",
            if zh {
                "已将字段注入转换为推荐的构造器注入。"
            } else {
                "Converted field injection to constructor injection."
            },
        );
        Ok(Some("converted".into()))
    }

    fn cmd_navigate_bean_candidates(&mut self, ctx: &Invocation<'_>) -> Result<Option<String>, Error> {
        let zh = is_zh();
        let ed = ctx
            .editor()
            .ok_or_else(|| Error::NotFound("No focused editor".into()))?;
        let doc = ed.document()?;
        let root = self
            .ensure_active_scanned(Some(ctx), false, false)
            .unwrap_or_else(|| {
                ed.path()
                    .rsplit_once('/')
                    .map(|(d, _)| d.to_owned())
                    .unwrap_or_else(|| "/".into())
            });
        let prof = self.active_profile_for_root(Some(&root));
        self.cache.update_file(
            &root,
            &ed.path(),
            &ed.language(),
            &doc.text,
            prof.as_deref(),
        );
        let at = ed
            .selections()?
            .into_iter()
            .next()
            .map(|r| r.start)
            .unwrap_or_default();

        let model = self
            .cache
            .projects
            .get(&root)
            .ok_or_else(|| Error::NotFound("Spring model unavailable".into()))?;
        let Some(facts) = model.files.get(&ed.path()) else {
            return Ok(None);
        };

        let mut locs: Vec<(String, String, Location)> = Vec::new();
        for inj in &facts.injections {
            if inj.line == at.line || position_contains(inj.range, at) {
                let (_, targets) = resolve_injection(model, inj, prof.as_deref());
                for t in targets {
                    locs.push((
                        t.bean_name.clone(),
                        format!("{} · {}", t.class_name, t.file),
                        SpringProjectModel::to_location(&t.file, t.range),
                    ));
                }
            }
        }
        if locs.is_empty() {
            for b in &facts.beans {
                if b.line == at.line || position_contains(b.name_range, at) {
                    for inj in model.all_injections() {
                        let (_, targets) = resolve_injection(model, inj, prof.as_deref());
                        if targets.iter().any(|t| t.bean_id == b.id) {
                            locs.push((
                                format!("{}.{}", inj.owner_class, inj.field_or_param_name),
                                inj.file.clone(),
                                SpringProjectModel::to_location(&inj.file, inj.range),
                            ));
                        }
                    }
                }
            }
        }
        if locs.is_empty() {
            for ev in &facts.event_sites {
                if ev.line == at.line || position_contains(ev.range, at) {
                    for f in model.files.values() {
                        for other in &f.event_sites {
                            if other.kind != ev.kind && other.event_type == ev.event_type {
                                locs.push((
                                    other.method_name.clone().unwrap_or_else(|| other.event_type.clone()),
                                    other.file.clone(),
                                    SpringProjectModel::to_location(&other.file, other.range),
                                ));
                            }
                        }
                    }
                }
            }
        }

        if locs.is_empty() {
            host::notify(
                NotificationLevel::Info,
                "Spring Navigation",
                if zh {
                    "光标处未找到关联的 Spring Bean 或注入点。"
                } else {
                    "No Spring Bean candidates or injection sites found at caret."
                },
            );
            return Ok(None);
        }
        if locs.len() == 1 {
            let l = &locs[0].2;
            host::open_location(&l.path, l.range.start.line, l.range.start.character)?;
            return Ok(Some(l.path.clone()));
        }

        let items: Vec<PickItem> = locs
            .iter()
            .enumerate()
            .map(|(i, (title, desc, _))| PickItem::new(&i.to_string(), title).description(desc))
            .collect();
        if let Some(chosen) = host::pick(
            if zh { "跳转到 Spring Bean / 注入点" } else { "Navigate to Spring Bean Candidate" },
            &items,
        )? && let Ok(idx) = chosen.parse::<usize>()
            && let Some((_, _, loc)) = locs.get(idx)
        {
            host::open_location(&loc.path, loc.range.start.line, loc.range.start.character)?;
            return Ok(Some(loc.path.clone()));
        }
        Ok(None)
    }

    fn cmd_explain_cron(&mut self, ctx: &Invocation<'_>, arg: Option<&str>) -> Result<Option<String>, Error> {
        let zh = is_zh();
        let mut expr = arg.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        if expr.is_none()
            && let Some(ed) = ctx.editor()
            && let Ok(doc) = ed.document()
        {
            let pos = ed
                .selections()
                .ok()
                .and_then(|s| s.into_iter().next())
                .map(|r| r.start)
                .unwrap_or_default();
            let idx = LineIndex::new(&doc.text);
            if let Some((q, _)) = quoted_string_at(&doc.text, &idx, pos) {
                expr = Some(q);
            } else {
                let root = self.ensure_active_scanned(Some(ctx), false, false).unwrap_or_default();
                let prof = self.active_profile_for_root(Some(&root));
                self.cache.update_file(&root, &ed.path(), &ed.language(), &doc.text, prof.as_deref());
                if let Some(facts) = self.cache.projects.get(&root).and_then(|m| m.files.get(&ed.path())) {
                    expr = facts
                        .scheduled_sites
                        .iter()
                        .find(|s| s.line == pos.line || position_contains(s.range, pos))
                        .and_then(|s| s.cron.clone());
                }
            }
        }

        let raw = match expr {
            Some(v) => v,
            None => match host::input(
                if zh { "输入 6 位 Spring Cron 表达式（秒 分 时 日 月 周）" } else { "Enter 6-field Spring Cron expression (sec min hour dom month dow)" },
                Some("0 */15 * * * *"),
            )? {
                Some(val) => val,
                None => return Ok(None),
            },
        };

        let exp = parse_and_explain_cron(&raw);
        let summary = if let Some(err) = &exp.error {
            format!("Invalid cron `{raw}`: {err}")
        } else if zh {
            format!("`{raw}` → {}", exp.summary_zh)
        } else {
            format!("`{raw}` → {}", exp.summary_en)
        };
        host::notify(
            if exp.valid { NotificationLevel::Info } else { NotificationLevel::Warning },
            "Spring Cron",
            &summary,
        );
        self.open_webview_tab("data", json!({ "cron": raw }), Some(ctx));
        Ok(Some(summary))
    }

    fn cmd_connect_actuator(&mut self, ctx: &Invocation<'_>, arg: Option<&str>) -> Result<Option<String>, Error> {
        let zh = is_zh();
        let url = match arg {
            Some(u) if !u.trim().is_empty() => u.trim().to_owned(),
            _ => match host::input(
                if zh { "输入 Spring Boot Actuator 地址（例如 http://localhost:8080/actuator）" } else { "Spring Boot Actuator base URL (e.g. http://localhost:8080/actuator)" },
                Some("http://localhost:8080/actuator"),
            )? {
                Some(v) => v.trim().to_owned(),
                None => return Ok(None),
            },
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(Error::Invalid("Actuator URL must start with http:// or https://".into()));
        }
        let clean = url.trim_end_matches('/').to_owned();
        if !self.custom_actuator_urls.contains(&clean) {
            self.custom_actuator_urls.push(clean.clone());
            self.persist_custom_urls();
        }
        let root = self.ensure_active_scanned(Some(ctx), false, false);
        self.probe_runtime(root.as_deref());
        host::refresh_view("spring-services");
        self.push_snapshot_to_webview(root.as_deref());
        host::notify(
            NotificationLevel::Info,
            "Spring Actuator",
            &if zh {
                format!("已添加并探测 `{clean}`，当前在线实例数：{}。", self.runtime_instances.len())
            } else {
                format!("Probed `{clean}` ({} live instance(s)).", self.runtime_instances.len())
            },
        );
        Ok(Some(clean))
    }

    fn handle_webview_rpc(&mut self, id: Value, method: &str, params: &Value, ctx: &Invocation<'_>) {
        let result: Result<Value, String> = (|| {
            match method {
                "project.snapshot" => {
                    let root = self.ensure_active_scanned(Some(ctx), false, false);
                    Ok(self.build_snapshot_json(root.as_deref()))
                }
                "project.refresh" => {
                    let root = self.ensure_active_scanned(Some(ctx), true, true);
                    self.update_status_bar(root.as_deref());
                    host::refresh_view("spring-services");
                    Ok(self.build_snapshot_json(root.as_deref()))
                }
                "profile.set" => {
                    let root = self.ensure_active_scanned(Some(ctx), false, false);
                    let prof = params
                        .get("profile")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    self.set_profile(root.clone(), prof);
                    Ok(self.build_snapshot_json(root.as_deref()))
                }
                "profile.diff" => {
                    let l = params.get("leftProfile").and_then(Value::as_str).unwrap_or("default");
                    let r = params.get("rightProfile").and_then(Value::as_str).unwrap_or("default");
                    self.cmd_compare_profiles(ctx, Some(&format!("{l}|{r}")))
                        .map(|_| json!({ "ok": true }))
                        .map_err(|e| e.to_string())
                }
                "config.convert" => {
                    let fmt = params.get("format").and_then(Value::as_str).unwrap_or("yaml");
                    let text = params.get("text").and_then(Value::as_str).unwrap_or("");
                    let converted = if fmt == "yaml" {
                        yaml_to_properties(text)?
                    } else {
                        properties_to_yaml(text)?
                    };
                    Ok(json!({ "converted": converted }))
                }
                "editor.open" => {
                    let path = params
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing path".to_owned())?;
                    let line = params.get("line").and_then(Value::as_u64).unwrap_or(0) as u32;
                    let col = params.get("character").and_then(Value::as_u64).unwrap_or(0) as u32;
                    host::open_location(path, line, col).map_err(|e| e.to_string())?;
                    Ok(json!({ "ok": true }))
                }
                "clipboard.copy" => {
                    let text = params.get("text").and_then(Value::as_str).unwrap_or("");
                    host::copy_to_clipboard(text).map_err(|e| e.to_string())?;
                    Ok(json!({ "ok": true }))
                }
                "endpoint.curl" => {
                    let ep_id = params.get("id").and_then(Value::as_str).unwrap_or("");
                    let root = self.ensure_active_scanned(Some(ctx), false, false);
                    let prof = self.active_profile_for_root(root.as_deref());
                    let model = root
                        .as_deref()
                        .and_then(|r| self.cache.projects.get(r))
                        .ok_or_else(|| "no indexed Spring project".to_owned())?;
                    let (base_host, ctx_path) = resolve_base_url(model, prof.as_deref());
                    let items = unified_endpoints_for_panel(model, prof.as_deref(), &[]);
                    let item = items
                        .iter()
                        .find(|x| x.id == ep_id)
                        .ok_or_else(|| "endpoint not found".to_owned())?;
                    let ep = model
                        .all_endpoints()
                        .into_iter()
                        .find(|e| e.file == item.file && e.line == item.line)
                        .ok_or_else(|| "endpoint source missing".to_owned())?;
                    let curl = format_curl(ep, &base_host, &ctx_path);
                    let _ = host::copy_to_clipboard(&curl);
                    Ok(json!({ "curl": curl }))
                }
                "spel.validate" => {
                    let expr = params.get("expression").and_then(Value::as_str).unwrap_or("");
                    let v = validate_spel(expr);
                    Ok(json!({
                        "valid": v.valid,
                        "summary": v.summary,
                        "error": v.error,
                        "beanRefs": v.bean_refs.into_iter().map(|(b, _)| b).collect::<Vec<_>>(),
                        "propRefs": v.prop_refs.into_iter().map(|(p, _)| p).collect::<Vec<_>>(),
                    }))
                }
                "cron.explain" => {
                    let expr = params.get("expression").and_then(Value::as_str).unwrap_or("");
                    let e = parse_and_explain_cron(expr);
                    Ok(json!({
                        "valid": e.valid,
                        "summaryEn": e.summary_en,
                        "summaryZh": e.summary_zh,
                        "fields": e.fields,
                        "error": e.error,
                    }))
                }
                "runtime.refresh" => {
                    let root = self.ensure_active_scanned(Some(ctx), false, false);
                    self.probe_runtime(root.as_deref());
                    host::refresh_view("spring-services");
                    Ok(json!({ "instances": self.runtime_instances }))
                }
                "runtime.connect" => {
                    let url = params.get("url").and_then(Value::as_str).unwrap_or("");
                    self.cmd_connect_actuator(ctx, Some(url))
                        .map_err(|e| e.to_string())?;
                    Ok(json!({ "instances": self.runtime_instances }))
                }
                "runtime.setLogger" => {
                    let inst_id = params.get("instanceId").and_then(Value::as_str).unwrap_or("");
                    let logger = params.get("logger").and_then(Value::as_str).unwrap_or("ROOT");
                    let level = params.get("level").and_then(Value::as_str);
                    let inst = self
                        .runtime_instances
                        .iter()
                        .find(|i| i.id == inst_id)
                        .ok_or_else(|| "runtime instance not found".to_owned())?
                        .clone();
                    set_runtime_logger_level(&inst, logger, level)?;
                    let root = self.current_root();
                    self.probe_runtime(root.as_deref());
                    host::refresh_view("spring-services");
                    Ok(json!({ "instances": self.runtime_instances }))
                }
                "runtime.openBrowser" => {
                    let inst_id = params.get("instanceId").and_then(Value::as_str).unwrap_or("");
                    let url = self
                        .runtime_instances
                        .iter()
                        .find(|i| i.id == inst_id)
                        .map(|inst| {
                            if let Some(p) = inst.server_port {
                                format!("http://localhost:{p}{}", inst.context_path)
                            } else {
                                inst.base_url.clone()
                            }
                        })
                        .ok_or_else(|| "runtime instance not found".to_owned())?;
                    host::open_url(&url).map_err(|e| e.to_string())?;
                    Ok(json!({ "url": url }))
                }
                "datasource.sync" => {
                    let project = self
                        .resolve_project(Some(ctx))
                        .ok_or_else(|| "no open project".to_owned())?;
                    let root = self.ensure_project_scanned(&project, false, false);
                    let prof = self.active_profile_for_root(Some(&root));
                    let model = self
                        .cache
                        .projects
                        .get(&root)
                        .ok_or_else(|| "Spring model unavailable".to_owned())?;
                    let name = sync_datasource_to_openidea(&project, model, prof.as_deref())?;
                    Ok(json!({ "name": name }))
                }
                "console.toggle" => {
                    let next = !self.is_console_highlighting_enabled();
                    self.console_highlight_override = Some(next);
                    self.persist_console_override();
                    self.sync_console_rules();
                    self.update_status_bar(self.current_root().as_deref());
                    Ok(json!({ "enabled": next }))
                }
                other => Err(format!("Unknown RPC method: {other}")),
            }
        })();

        let reply = match result {
            Ok(data) => json!({ "op": "rpc-result", "id": id, "ok": true, "data": data }),
            Err(err) => json!({ "op": "rpc-result", "id": id, "ok": false, "error": err }),
        };
        let _ = host::post_view_message("spring", &reply.to_string());
    }
}

impl Plugin for SpringPlugin {
    fn activate(&mut self, ctx: &Context) -> Result<(), Error> {
        if ctx.has("storage") {
            self.restore_storage();
        }
        if ctx.has("events") {
            let _ = host::subscribe(EventKind::FileSaved);
            let _ = host::subscribe(EventKind::FilesChanged);
            let _ = host::subscribe(EventKind::SettingsChanged);
            let _ = host::subscribe(EventKind::EditorFocused);
        }
        self.sync_console_rules();
        if ctx.has("project.read")
            && let Ok(projects) = host::projects()
            && let Some(first) = projects.into_iter().next()
            && let Ok(root) = first.root()
        {
            self.last_project_root = Some(root);
        }
        let root = self.current_root();
        self.update_status_bar(root.as_deref());
        Ok(())
    }

    fn handle_events(&mut self, events: &[Event], ctx: &Invocation) {
        let mut model_dirty = false;
        let mut settings_dirty = false;

        for ev in events {
            match ev {
                Event::FileSaved(path) => {
                    if should_index_path(path)
                        && let Some(project) = host::project_of(path).or_else(|| ctx.project())
                        && let Ok(root) = project.root()
                        && let Ok(text) = project.read_file(path)
                    {
                        let prof = self.active_profile_for_root(Some(&root));
                        self.cache.update_file(
                            &root,
                            path,
                            detect_language_from_path(path),
                            &text,
                            prof.as_deref(),
                        );
                        self.last_project_root = Some(root);
                        model_dirty = true;
                    }
                }
                Event::FilesChanged(batch) => {
                    if let Some(project) = self.resolve_project(Some(ctx))
                        && let Ok(root) = project.root()
                    {
                        let prof = self.active_profile_for_root(Some(&root));
                        for path in batch.created.iter().chain(&batch.modified) {
                            if should_index_path(path)
                                && let Ok(text) = project.read_file(path)
                            {
                                self.cache.update_file(
                                    &root,
                                    path,
                                    detect_language_from_path(path),
                                    &text,
                                    prof.as_deref(),
                                );
                                model_dirty = true;
                            }
                        }
                        for removed in &batch.removed {
                            if should_index_path(removed) {
                                self.cache.remove_file(&root, removed, prof.as_deref());
                                model_dirty = true;
                            }
                        }
                    }
                }
                Event::EditorFocused(Some(path)) => {
                    if let Some(p) = host::project_of(path)
                        && let Ok(root) = p.root()
                    {
                        self.last_project_root = Some(root);
                        self.update_status_bar(self.last_project_root.as_deref());
                    }
                }
                Event::Other { kind, .. } if kind == "settings-changed" => {
                    settings_dirty = true;
                }
                _ => {}
            }
        }

        if settings_dirty {
            self.sync_console_rules();
            let root = self.current_root();
            self.update_status_bar(root.as_deref());
            self.push_snapshot_to_webview(root.as_deref());
        } else if model_dirty {
            let root = self.current_root();
            self.update_status_bar(root.as_deref());
            host::refresh_view("spring-services");
            self.push_snapshot_to_webview(root.as_deref());
        }
    }

    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        let zh = is_zh();
        match id {
            "open" => {
                self.open_webview_tab("beans", json!({}), Some(ctx));
                Ok(None)
            }
            "open-beans" => {
                self.open_webview_tab("beans", json!({ "beanId": arg }), Some(ctx));
                Ok(None)
            }
            "open-endpoints" => {
                self.open_webview_tab("endpoints", json!({}), Some(ctx));
                Ok(None)
            }
            "open-config" => {
                self.open_webview_tab("config", json!({}), Some(ctx));
                Ok(None)
            }
            "open-data" => {
                self.open_webview_tab("data", json!({}), Some(ctx));
                Ok(None)
            }
            "open-runtime" => {
                let root = self.ensure_active_scanned(Some(ctx), false, false);
                self.probe_runtime(root.as_deref());
                host::refresh_view("spring-services");
                self.open_webview_tab("runtime", json!({ "instanceId": arg }), Some(ctx));
                Ok(None)
            }
            "refresh-index" => {
                let root = self.ensure_active_scanned(Some(ctx), true, true);
                self.update_status_bar(root.as_deref());
                host::refresh_view("spring-services");
                self.push_snapshot_to_webview(root.as_deref());
                let (beans, endpoints) = root
                    .as_deref()
                    .and_then(|r| self.cache.projects.get(r))
                    .map(|m| (m.all_beans().len(), m.all_endpoints().len()))
                    .unwrap_or((0, 0));
                host::notify(
                    NotificationLevel::Info,
                    "Spring Index",
                    &if zh {
                        format!("索引已刷新：发现 {beans} 个 Bean，{endpoints} 个端点。")
                    } else {
                        format!("Spring project index refreshed: {beans} bean(s), {endpoints} endpoint(s).")
                    },
                );
                Ok(Some(format!("{beans}:{endpoints}")))
            }
            "switch-profile" => self.cmd_switch_profile(ctx, arg),
            "compare-profiles" => self.cmd_compare_profiles(ctx, arg),
            "convert-yaml-properties" => self.cmd_convert_yaml_properties(ctx),
            "convert-field-to-constructor" => self.cmd_convert_field_to_constructor(ctx),
            "inspect-bean-at-caret" => {
                let mut bean_id = arg.map(str::to_owned);
                if bean_id.is_none()
                    && let Some(ed) = ctx.editor()
                    && let Ok(doc) = ed.document()
                {
                    let root = self.ensure_active_scanned(Some(ctx), false, false).unwrap_or_default();
                    let prof = self.active_profile_for_root(Some(&root));
                    self.cache.update_file(
                        &root,
                        &ed.path(),
                        &ed.language(),
                        &doc.text,
                        prof.as_deref(),
                    );
                    let at = ed
                        .selections()
                        .ok()
                        .and_then(|s| s.into_iter().next())
                        .map(|r| r.start)
                        .unwrap_or_default();
                    if let Some(model) = self.cache.projects.get(&root)
                        && let Some(facts) = model.files.get(&ed.path())
                    {
                        bean_id = facts
                            .beans
                            .iter()
                            .find(|b| b.line == at.line || position_contains(b.decl_range, at))
                            .map(|b| b.id.clone())
                            .or_else(|| facts.beans.first().map(|b| b.id.clone()));
                    }
                }
                self.open_webview_tab("beans", json!({ "beanId": bean_id }), Some(ctx));
                Ok(bean_id)
            }
            "navigate-bean-candidates" => self.cmd_navigate_bean_candidates(ctx),
            "copy-endpoint-url" => {
                let Some((ep, base_host, ctx_path)) = self.find_endpoint_for_command(ctx, arg)
                else {
                    return Ok(None);
                };
                let full_path = join_context_path(&ctx_path, &ep.full_path);
                let url = format!("{base_host}{full_path}");
                host::copy_to_clipboard(&url)?;
                host::notify(NotificationLevel::Info, "Spring Web", &format!("Copied: {url}"));
                Ok(Some(url))
            }
            "copy-endpoint-curl" => {
                let Some((ep, base_host, ctx_path)) = self.find_endpoint_for_command(ctx, arg)
                else {
                    return Ok(None);
                };
                let curl = format_curl(&ep, &base_host, &ctx_path);
                host::copy_to_clipboard(&curl)?;
                host::notify(NotificationLevel::Info, "Spring Web", &format!("Copied: {curl}"));
                Ok(Some(curl))
            }
            "explain-cron" => self.cmd_explain_cron(ctx, arg),
            "sync-datasource" => {
                let project = self
                    .resolve_project(Some(ctx))
                    .ok_or_else(|| Error::NotFound("No open project".into()))?;
                let root = self.ensure_project_scanned(&project, false, false);
                let prof = self.active_profile_for_root(Some(&root));
                let model = self
                    .cache
                    .projects
                    .get(&root)
                    .ok_or_else(|| Error::NotFound("Spring model unavailable".into()))?;
                match sync_datasource_to_openidea(&project, model, prof.as_deref()) {
                    Ok(name) => {
                        host::notify(
                            NotificationLevel::Info,
                            "Spring Datasource",
                            &if zh {
                                format!("已同步数据源 `{name}` 到 Database 工具窗口。")
                            } else {
                                format!("Synced datasource `{name}` to Database tool window.")
                            },
                        );
                        Ok(Some(name))
                    }
                    Err(err) => {
                        host::notify(NotificationLevel::Warning, "Spring Datasource", &err);
                        Err(Error::Failed(err))
                    }
                }
            }
            "connect-actuator" => self.cmd_connect_actuator(ctx, arg),
            "refresh-runtime" => {
                let root = self.ensure_active_scanned(Some(ctx), false, false);
                self.probe_runtime(root.as_deref());
                host::refresh_view("spring-services");
                self.push_snapshot_to_webview(root.as_deref());
                Ok(Some(self.runtime_instances.len().to_string()))
            }
            "open-runtime-browser" => {
                if self.runtime_instances.is_empty() {
                    let root = self.ensure_active_scanned(Some(ctx), false, false);
                    self.probe_runtime(root.as_deref());
                }
                let clean_id = arg.map(|a| a.strip_prefix("app:").unwrap_or(a));
                let target = clean_id
                    .and_then(|id| self.runtime_instances.iter().find(|i| i.id == id))
                    .or_else(|| self.runtime_instances.first());
                let url = if let Some(inst) = target {
                    if let Some(port) = inst.server_port {
                        format!("http://localhost:{port}{}", inst.context_path)
                    } else {
                        inst.base_url.clone()
                    }
                } else {
                    let root = self.ensure_active_scanned(Some(ctx), false, false);
                    let prof = self.active_profile_for_root(root.as_deref());
                    if let Some(m) = root.as_deref().and_then(|r| self.cache.projects.get(r)) {
                        let (base, ctx_path) = resolve_base_url(m, prof.as_deref());
                        format!("{base}{ctx_path}")
                    } else {
                        "http://localhost:8080".into()
                    }
                };
                host::open_url(&url)?;
                Ok(Some(url))
            }
            "toggle-console-highlight" => {
                let next = !self.is_console_highlighting_enabled();
                self.console_highlight_override = Some(next);
                self.persist_console_override();
                self.sync_console_rules();
                self.update_status_bar(self.current_root().as_deref());
                self.push_snapshot_to_webview(self.current_root().as_deref());
                host::notify(
                    NotificationLevel::Info,
                    "Spring Console",
                    if next {
                        if zh { "已启用 Spring Boot 控制台日志高亮。" } else { "Spring Boot console highlighting enabled." }
                    } else if zh {
                        "已禁用 Spring Boot 控制台日志高亮。"
                    } else {
                        "Spring Boot console highlighting disabled."
                    },
                );
                Ok(Some(next.to_string()))
            }
            _ => Err(Error::NotFound(format!("Unknown command: {id}"))),
        }
    }
}

impl Views for SpringPlugin {
    fn render(&mut self, view: &str, _viewport: Option<Viewport>, ctx: &Invocation) -> Result<View, Error> {
        match view {
            "spring" => Ok(View::message("Spring & Spring Boot")),
            "spring-services" => {
                let root = self.ensure_active_scanned(Some(ctx), false, false);
                let model = root.as_deref().and_then(|r| self.cache.projects.get(r));
                Ok(render_services_view(model, &self.runtime_instances))
            }
            _ => Err(Error::NotFound(format!("Unknown view: {view}"))),
        }
    }

    fn handle(&mut self, view: &str, event: ViewEvent, ctx: &Invocation) -> Result<(), Error> {
        match view {
            "spring-services" => match event {
                ViewEvent::Button(ref id) if id == "refresh" => {
                    let root = self.ensure_active_scanned(Some(ctx), false, false);
                    self.probe_runtime(root.as_deref());
                    host::refresh_view("spring-services");
                    self.push_snapshot_to_webview(root.as_deref());
                    Ok(())
                }
                ViewEvent::Button(ref id) if id == "connect" => {
                    let _ = self.cmd_connect_actuator(ctx, None)?;
                    Ok(())
                }
                ViewEvent::Button(ref id) if id == "open-panel" => {
                    self.open_webview_tab("runtime", json!({}), Some(ctx));
                    Ok(())
                }
                ViewEvent::Activate(key) => {
                    if let Some(bean_id) = key.strip_prefix("static:") {
                        let root = self.ensure_active_scanned(Some(ctx), false, false);
                        if let Some(m) = root.as_deref().and_then(|r| self.cache.projects.get(r))
                            && let Some(b) = m.all_beans().into_iter().find(|x| x.id == bean_id)
                        {
                            let _ = host::open_location(&b.file, b.name_range.start.line, b.name_range.start.character);
                        }
                    } else if let Some(inst_id) = key.strip_prefix("app:") {
                        self.open_webview_tab("runtime", json!({ "instanceId": inst_id }), Some(ctx));
                    }
                    Ok(())
                }
                _ => Ok(()),
            },
            "spring" => {
                if let ViewEvent::Message(raw) = event
                    && let Ok(msg) = serde_json::from_str::<Value>(&raw)
                {
                    match msg.get("op").and_then(Value::as_str) {
                        Some("ready") => {
                            let root = self.ensure_active_scanned(Some(ctx), false, false);
                            let locale = effective_locale();
                            let pending = self.pending_panel_action.take();
                            let snapshot = self.build_snapshot_json(root.as_deref());
                            let init = json!({
                                "op": "init",
                                "locale": locale,
                                "pending": pending,
                                "snapshot": snapshot,
                            });
                            let _ = host::post_view_message("spring", &init.to_string());
                        }
                        Some("rpc") => {
                            let id = msg.get("id").cloned().unwrap_or(json!(0));
                            let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
                            let params = msg.get("params").cloned().unwrap_or(json!({}));
                            self.handle_webview_rpc(id, method, &params, ctx);
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

impl Details for SpringPlugin {
    fn detail(&mut self, view: &str, key: &str, ctx: &Invocation) -> Result<Vec<Tab>, Error> {
        if view != "spring-services" {
            return Ok(Vec::new());
        }
        let root = self.ensure_active_scanned(Some(ctx), false, false);
        let model = root.as_deref().and_then(|r| self.cache.projects.get(r));
        Ok(render_service_details(key, model, &self.runtime_instances))
    }
}

impl DocumentIndexer for SpringPlugin {
    fn index(&mut self, doc: &IndexDoc) -> Edges {
        let prof = self.doc_profile(&doc.path, detect_language_from_path(&doc.path), &doc.text);
        build_document_index_edges(doc, &mut self.cache, prof.as_deref())
    }
}

impl Providers for SpringPlugin {
    fn completion(
        &mut self,
        doc: &ProviderDoc,
        at: Position,
        trigger: Option<&str>,
    ) -> Vec<CompletionItem> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        providers::completion(&mut self.cache, doc, at, trigger, prof.as_deref())
    }

    fn code_actions(&mut self, doc: &ProviderDoc, selection: Range) -> Vec<CodeAction> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        providers::code_actions(&mut self.cache, doc, selection, prof.as_deref())
    }

    fn resolve_code_action(
        &mut self,
        doc: &ProviderDoc,
        action: CodeAction,
    ) -> Result<CodeAction, Error> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        providers::resolve_code_action(&mut self.cache, doc, action, prof.as_deref())
    }

    fn definition(&mut self, doc: &ProviderDoc, at: Position) -> Vec<Location> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        providers::definition(&mut self.cache, doc, at, prof.as_deref())
    }

    fn line_markers(&mut self, doc: &ProviderDoc) -> Vec<LineMarker> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        providers::line_markers(&mut self.cache, doc, prof.as_deref())
    }
}

impl HoverProvider for SpringPlugin {
    fn hover(&mut self, doc: &HoverDoc, at: Position) -> Option<Hover> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        hover_inlay::hover(&mut self.cache, doc, at, prof.as_deref(), is_zh())
    }
}

impl InlayHints for SpringPlugin {
    fn inlay_hints(
        &mut self,
        doc: &openidea_plugin::inlay_hints::Document,
        range: Range,
    ) -> Vec<InlayHint> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        hover_inlay::inlay_hints(
            &mut self.cache,
            doc,
            range,
            prof.as_deref(),
            Self::prefs_inlay(),
            is_zh(),
        )
    }
}

impl DiagnosticsProvider for SpringPlugin {
    fn diagnostics(&mut self, doc: &DiagnosticsDoc) -> Vec<Diagnostic> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        let enabled = host::setting_bool("diagnosticsEnabled").unwrap_or(true);
        let warn_field = host::setting_bool("warnFieldInjection").unwrap_or(true);
        let warn_unknown = host::setting_bool("warnUnknownProperties").unwrap_or(true);
        diagnostics::compute_diagnostics(
            &mut self.cache,
            doc,
            prof.as_deref(),
            enabled,
            warn_field,
            warn_unknown,
        )
    }
}

impl EndpointsProvider for SpringPlugin {
    fn endpoints(&mut self, ctx: &Invocation) -> Result<Vec<Endpoint>, Error> {
        let root = self
            .ensure_active_scanned(Some(ctx), false, false)
            .ok_or_else(|| Error::NotFound("No open Spring project".into()))?;
        let prof = self.active_profile_for_root(Some(&root));
        let Some(model) = self.cache.projects.get(&root) else {
            return Ok(Vec::new());
        };
        Ok(endpoints_for_host_provider(model, prof.as_deref()))
    }
}

impl SearchProvider for SpringPlugin {
    fn search(
        &mut self,
        category: &str,
        query: &str,
        limit: u32,
        ctx: &Invocation,
    ) -> Result<SearchPage, Error> {
        let root = self.ensure_active_scanned(Some(ctx), false, false);
        let prof = self.active_profile_for_root(root.as_deref());
        let Some(model) = root.as_deref().and_then(|r| self.cache.projects.get(r)) else {
            return Ok(SearchPage::default());
        };
        Ok(search_category(model, prof.as_deref(), category, query, limit))
    }
}

impl LanguageFeatures for SpringPlugin {
    fn implementation(&mut self, doc: &LangDoc, at: Position) -> Vec<Location> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        language_features::implementation(&mut self.cache, doc, at, prof.as_deref())
    }

    fn document_links(&mut self, doc: &LangDoc) -> Vec<DocumentLink> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        language_features::document_links(&mut self.cache, doc, prof.as_deref())
    }

    fn gutter_actions(&mut self, doc: &LangDoc) -> Vec<GutterAction> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        language_features::gutter_actions(&mut self.cache, doc, prof.as_deref())
    }
}

impl Injections for SpringPlugin {
    fn injections(&mut self, doc: &LangDoc) -> Vec<Injection> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        language_features::sql_injections(&mut self.cache, doc, prof.as_deref())
    }
}

impl FileActions for SpringPlugin {
    fn file_actions(&mut self, doc: &ProviderDoc, selection: Range) -> Vec<FileAction> {
        let prof = self.doc_profile(&doc.path, &doc.language, &doc.text);
        language_features::file_actions(&mut self.cache, doc, selection, prof.as_deref())
    }
}

#[allow(dead_code)]
fn _keep_unused_imports(_: BTreeSet<String>) {}

register_plugin!(
    SpringPlugin,
    views,
    details,
    indexing,
    providers,
    hover,
    inlay_hints,
    diagnostics,
    endpoints,
    search,
    language_features,
    injections,
    file_actions
);

#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::mock::MockHost;

    fn sample_service_java() -> &'static str {
        r#"package com.example.demo;

import org.springframework.stereotype.Service;
import org.springframework.beans.factory.annotation.Autowired;

@Service
public class OrderService {
    @Autowired
    private PaymentClient paymentClient;
}
"#
    }

    fn sample_repos_java() -> &'static str {
        r#"package com.example.demo;

import org.springframework.stereotype.Component;
import org.springframework.data.jpa.repository.JpaRepository;
import org.springframework.data.jpa.repository.Query;
import org.springframework.web.bind.annotation.RestController;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.service.annotation.HttpExchange;
import org.springframework.web.service.annotation.GetExchange;
import jakarta.persistence.Entity;
import jakarta.persistence.Id;

@Component("stripeClient")
public class StripeClient implements PaymentClient {}

@Entity
public class Order {
    @Id
    private Long id;
    private String status;
}

public interface OrderRepo extends JpaRepository<Order, Long> {
    @Query("SELECT o FROM Order o WHERE o.status = :status")
    java.util.List<Order> findActive(String status);
}

@RestController
public class OrderMvcController {
    @GetMapping("/api/orders")
    public String list() { return "ok"; }
}

@HttpExchange("/remote")
public interface RemoteClient {
    @GetExchange("/ping")
    String ping();
}
"#
    }

    fn sample_yaml() -> &'static str {
        "server:\n  port: 9091\n  servlet:\n    context-path: /api\nspring:\n  profiles:\n    active: dev\n"
    }

    fn seed_project(mock: &MockHost) {
        mock.add_file("/project/src/main/java/com/example/demo/OrderService.java", sample_service_java());
        mock.add_file("/project/src/main/java/com/example/demo/Components.java", sample_repos_java());
        mock.add_file("/project/src/main/resources/application.yml", sample_yaml());
        mock.add_file("/project/src/main/resources/application-dev.yml", "server:\n  port: 8099\n");
    }

    #[test]
    fn activate_sets_console_rules_subscriptions_and_status_menu() {
        let mock = MockHost::install();
        seed_project(&mock);
        let mut plugin = SpringPlugin::default();
        let ctx = Context {
            plugin_id: "ruyimg.spring".into(),
            plugin_version: "0.1.0".into(),
            api_version: "1.15".into(),
            capabilities: vec![
                "project.read".into(),
                "storage".into(),
                "events".into(),
            ],
        };
        plugin.activate(&ctx).unwrap();
        assert!(!mock.console_rules().is_empty());
        assert!(mock.subscriptions().contains(&EventKind::FileSaved));
        assert!(mock.subscriptions().contains(&EventKind::SettingsChanged));
        assert!(mock.status_items().contains_key(STATUS_BAR_ID));
        assert!(!mock.status_menu(STATUS_BAR_ID).is_empty());
    }

    #[test]
    fn switch_profile_persists_and_updates_status_bar() {
        let mock = MockHost::install();
        seed_project(&mock);
        let mut plugin = SpringPlugin::default();
        let inv = mock.invocation();
        let res = plugin.run_command("switch-profile", &inv, Some("dev")).unwrap();
        assert_eq!(res.as_deref(), Some("dev"));
        assert_eq!(
            mock.status_items().get(STATUS_BAR_ID).map(|s| s.text.as_str()),
            Some("Spring: dev")
        );
        assert!(mock.storage().contains_key(STORAGE_PROFILES_KEY));
    }

    #[test]
    fn commands_convert_yaml_and_field_injection() {
        let mock = MockHost::install();
        let mut plugin = SpringPlugin::default();
        let inv = mock.invocation();

        mock.open_editor(
            "/project/src/main/java/com/example/demo/OrderService.java",
            "java",
            sample_service_java(),
            &[(7, 4, 7, 4)],
        );
        let out = plugin.run_command("convert-field-to-constructor", &inv, None).unwrap();
        assert_eq!(out.as_deref(), Some("converted"));
        assert!(mock.editor_text().contains("public OrderService(PaymentClient paymentClient)"));

        mock.open_editor(
            "/project/src/main/resources/application.yml",
            "yaml",
            "server:\n  port: 8081\n",
            &[(0, 0, 0, 0)],
        );
        let conv = plugin.run_command("convert-yaml-properties", &inv, None).unwrap();
        assert_eq!(conv.as_deref(), Some("properties"));
    }

    #[test]
    fn endpoints_provider_only_emits_webflux_and_http_exchange() {
        let mock = MockHost::install();
        seed_project(&mock);
        let mut plugin = SpringPlugin::default();
        let inv = mock.invocation();
        let eps = plugin.endpoints(&inv).unwrap();
        assert!(eps.iter().any(|e| e.path.contains("/remote/ping")));
        assert!(!eps.iter().any(|e| e.path.ends_with("/orders")));
    }

    #[test]
    fn search_provider_returns_beans_and_config() {
        let mock = MockHost::install();
        seed_project(&mock);
        let mut plugin = SpringPlugin::default();
        let inv = mock.invocation();

        let beans = plugin.search("spring-beans", "OrderService", 20, &inv).unwrap();
        assert!(beans.items.iter().any(|i| i.title.contains("OrderService")));

        let cfg = plugin.search("spring-config", "server.port", 20, &inv).unwrap();
        assert!(cfg.items.iter().any(|i| i.title.contains("server.port")));
    }

    #[test]
    fn indexing_injections_and_file_actions_work() {
        let mock = MockHost::install();
        seed_project(&mock);
        let mut plugin = SpringPlugin::default();

        let idx_doc = IndexDoc {
            path: "/project/src/main/java/com/example/demo/OrderService.java".into(),
            text: sample_service_java().into(),
            symbols: Vec::new(),
        };
        let edges = plugin.index(&idx_doc);
        assert!(!edges.definitions.is_empty());
        assert!(!edges.references.is_empty());

        let repo_doc = LangDoc {
            path: "/project/src/main/java/com/example/demo/Components.java".into(),
            language: "java".into(),
            version: 1,
            text: sample_repos_java().into(),
        };
        let sql = plugin.injections(&repo_doc);
        assert_eq!(sql.len(), 1);
        assert_eq!(sql[0].language, "sql");

        let prof_src = "@Profile(\"staging\")\n@Service\npublic class StagingSvc {}\n";
        let fa_doc = ProviderDoc {
            path: "/project/src/main/java/com/example/demo/StagingSvc.java".into(),
            language: "java".into(),
            version: 1,
            text: prof_src.into(),
        };
        let actions = plugin.file_actions(
            &fa_doc,
            Range::new(Position::new(0, 0), Position::new(0, 5)),
        );
        assert!(actions.iter().any(|a| a.title.contains("application-staging.yml")));
    }

    #[test]
    fn services_view_details_and_webview_rpc_work() {
        let mock = MockHost::install();
        seed_project(&mock);
        let mut plugin = SpringPlugin::default();
        let inv = mock.invocation();

        let view = plugin.render("spring-services", None, &inv).unwrap();
        assert!(matches!(view.body, openidea_plugin::views::Body::Message(_)));

        // Inject a demo Main Boot app to verify tree + details rendering
        let boot_app = "package com.example.demo;\n@SpringBootApplication\npublic class DemoApplication {}\n";
        mock.add_file("/project/src/main/java/com/example/demo/DemoApplication.java", boot_app);
        plugin.run_command("refresh-index", &inv, None).unwrap();
        let view2 = plugin.render("spring-services", None, &inv).unwrap();
        assert!(matches!(view2.body, openidea_plugin::views::Body::Tree(ref t) if !t.is_empty()));

        plugin
            .handle("spring", ViewEvent::Message(r#"{"op":"ready"}"#.into()), &inv)
            .unwrap();
        plugin
            .handle(
                "spring",
                ViewEvent::Message(r#"{"op":"rpc","id":1,"method":"cron.explain","params":{"expression":"0 0/15 * * * *"}}"#.into()),
                &inv,
            )
            .unwrap();
        plugin
            .handle(
                "spring",
                ViewEvent::Message(r##"{"op":"rpc","id":2,"method":"spel.validate","params":{"expression":"#{@stripeClient.charge()}"}}"##.into()),
                &inv,
            )
            .unwrap();
        plugin
            .handle(
                "spring",
                ViewEvent::Message(r#"{"op":"rpc","id":3,"method":"config.convert","params":{"format":"yaml","text":"server:\n  port: 9000\n"}}"#.into()),
                &inv,
            )
            .unwrap();

        let tabs = plugin.detail("spring-services", "unknown", &inv).unwrap();
        assert_eq!(tabs.len(), 1);
    }

    #[test]
    fn refuses_unsafe_ctor_refactor_and_preserves_yaml_sequences_via_sdk_handlers() {
        let mock = MockHost::install();
        let mut plugin = SpringPlugin::default();
        let inv = mock.invocation();

        // 1. Existing constructor -> command returns None, editor text unchanged, CodeAction omitted
        let example_java = concat!(
            "package com.example;\n",
            "import org.springframework.beans.factory.annotation.Autowired;\n",
            "public class Example {\n",
            "    @Autowired private Repo repo;\n",
            "    public Example() { System.out.println(\"startup work\"); }\n",
            "}\n",
        );
        mock.open_editor("/project/src/main/java/com/example/Example.java", "java", example_java, &[(3, 4, 3, 4)]);
        let res = plugin.run_command("convert-field-to-constructor", &inv, None).unwrap();
        assert_eq!(res, None);
        assert_eq!(mock.editor_text(), example_java);

        let java_actions = plugin.code_actions(
            &ProviderDoc {
                path: "/project/src/main/java/com/example/Example.java".into(),
                version: 1,
                language: "java".into(),
                text: example_java.into(),
            },
            openidea_plugin::Range {
                start: openidea_plugin::Position { line: 3, character: 4 },
                end: openidea_plugin::Position { line: 3, character: 4 },
            },
        );
        assert!(
            !java_actions.iter().any(|a| a.title.contains("constructor injection")),
            "must not offer field-to-constructor code action when class already defines a constructor"
        );

        // 2. YAML mapping sequences -> CodeAction emits indexed Spring keys (servers[0].host, servers[1].port)
        let app_yml = "servers:\n  - host: api.example.test\n    port: 8080\n  - host: backup.example.test\n    port: 8081\n";
        let yml_actions = plugin.code_actions(
            &ProviderDoc {
                path: "/project/src/main/resources/application.yml".into(),
                version: 1,
                language: "yaml".into(),
                text: app_yml.into(),
            },
            openidea_plugin::Range {
                start: openidea_plugin::Position { line: 0, character: 0 },
                end: openidea_plugin::Position { line: 0, character: 0 },
            },
        );
        let convert_action = yml_actions
            .iter()
            .find(|a| a.title == "Spring Boot: Convert YAML to .properties format")
            .expect("expected YAML conversion code action");
        let converted_text = &convert_action.edits[0].1[0].text;
        assert_eq!(
            converted_text,
            "servers[0].host=api.example.test\nservers[0].port=8080\nservers[1].host=backup.example.test\nservers[1].port=8081\n"
        );

        // Invalid YAML -> conversion code action must not be offered
        let invalid_yml_actions = plugin.code_actions(
            &ProviderDoc {
                path: "/project/src/main/resources/application.yml".into(),
                version: 1,
                language: "yaml".into(),
                text: "servers: !bad [unclosed\n".into(),
            },
            openidea_plugin::Range {
                start: openidea_plugin::Position { line: 0, character: 0 },
                end: openidea_plugin::Position { line: 0, character: 0 },
            },
        );
        assert!(!invalid_yml_actions
            .iter()
            .any(|a| a.title == "Spring Boot: Convert YAML to .properties format"));
    }
}
