use crate::model::{EndpointDef, EndpointKind, SpringProjectModel};
use crate::parser_config::resolve_effective_properties;
use openidea_plugin::endpoints::Endpoint as HostEndpoint;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedEndpointItem {
    pub id: String,
    pub methods: Vec<String>,
    pub raw_path: String,
    pub full_path: String,
    pub resolved_url: String,
    pub kind: EndpointKind,
    pub framework: String,
    pub controller: String,
    pub handler: String,
    pub service_name: Option<String>,
    pub query_params: Vec<String>,
    pub has_request_body: bool,
    pub duplicate_conflict: bool,
    pub runtime_matched: bool,
    pub file: String,
    pub line: u32,
}

pub fn resolve_base_url(model: &SpringProjectModel, active_profile: Option<&str>) -> (String, String) {
    let config_files = model.all_config_files();
    let eff = resolve_effective_properties(&config_files, active_profile);
    let port = eff
        .iter()
        .find(|p| p.relaxed_key == "server.port")
        .map(|p| p.value.trim().to_owned())
        .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or_else(|| "8080".to_owned());
    let ctx_path = eff
        .iter()
        .find(|p| {
            matches!(
                p.relaxed_key.as_str(),
                "server.servlet.contextpath" | "spring.webflux.basepath" | "spring.mvc.servlet.path"
            )
        })
        .map(|p| p.value.trim().to_owned())
        .unwrap_or_default();

    let clean_ctx = if ctx_path == "/" || ctx_path.is_empty() {
        String::new()
    } else {
        format!("/{}", ctx_path.trim_matches('/'))
    };
    (format!("http://localhost:{port}"), clean_ctx)
}

pub fn join_context_path(ctx_path: &str, route_path: &str) -> String {
    if ctx_path.is_empty() {
        return route_path.to_owned();
    }
    let trimmed = route_path.trim_start_matches('/');
    if trimmed.is_empty() {
        ctx_path.to_owned()
    } else {
        format!("{ctx_path}/{trimmed}")
    }
}

/// Returns ONLY WebFlux and Spring 6 @HttpExchange routes to the OpenIDEA EndpointsProvider
/// so OpenIDEA's built-in MVC route indexer is never duplicated.
pub fn endpoints_for_host_provider(
    model: &SpringProjectModel,
    active_profile: Option<&str>,
) -> Vec<HostEndpoint> {
    let (_, ctx_path) = resolve_base_url(model, active_profile);
    let mut out = Vec::new();
    for ep in model.all_endpoints() {
        if !matches!(ep.kind, EndpointKind::WebFlux | EndpointKind::HttpExchange) {
            continue;
        }
        let path = join_context_path(&ctx_path, &ep.full_path);
        for method in &ep.methods {
            out.push(HostEndpoint {
                method: Some(method.clone()),
                path: path.clone(),
                framework: ep.kind.framework_label().to_owned(),
                handler: Some(ep.handler.clone()),
                location: SpringProjectModel::to_location(&ep.file, ep.range),
            });
        }
    }
    out
}

pub fn unified_endpoints_for_panel(
    model: &SpringProjectModel,
    active_profile: Option<&str>,
    runtime_paths: &[String],
) -> Vec<UnifiedEndpointItem> {
    let (base_host, ctx_path) = resolve_base_url(model, active_profile);
    let all = model.all_endpoints();
    let mut counts: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    for ep in &all {
        if ep.kind == EndpointKind::FeignClient {
            continue;
        }
        for m in &ep.methods {
            *counts.entry((m.clone(), ep.full_path.clone())).or_insert(0) += 1;
        }
    }

    let mut out = Vec::new();
    for (idx, ep) in all.iter().enumerate() {
        let full_with_ctx = if ep.kind == EndpointKind::FeignClient {
            ep.full_path.clone()
        } else {
            join_context_path(&ctx_path, &ep.full_path)
        };
        let resolved_url = if ep.kind == EndpointKind::FeignClient {
            let svc = ep.service_name.as_deref().unwrap_or("service");
            format!("http://{svc}{full_with_ctx}")
        } else {
            format!("{base_host}{full_with_ctx}")
        };
        let dup = ep.kind != EndpointKind::FeignClient
            && ep
                .methods
                .iter()
                .any(|m| counts.get(&(m.clone(), ep.full_path.clone())).copied().unwrap_or(0) > 1);
        let runtime_matched = runtime_paths
            .iter()
            .any(|rp| rp == &ep.full_path || rp == &full_with_ctx);

        out.push(UnifiedEndpointItem {
            id: format!("ep:{idx}:{}:{}", ep.file, ep.line),
            methods: ep.methods.clone(),
            raw_path: ep.raw_path.clone(),
            full_path: full_with_ctx,
            resolved_url,
            kind: ep.kind,
            framework: ep.kind.framework_label().to_owned(),
            controller: ep.controller.clone(),
            handler: ep.handler.clone(),
            service_name: ep.service_name.clone(),
            query_params: ep.query_params.clone(),
            has_request_body: ep.has_request_body,
            duplicate_conflict: dup,
            runtime_matched,
            file: ep.file.clone(),
            line: ep.line,
        });
    }
    out.sort_by(|a, b| a.full_path.cmp(&b.full_path).then_with(|| a.line.cmp(&b.line)));
    out
}

pub fn format_curl(ep: &EndpointDef, base_host: &str, ctx_path: &str) -> String {
    let method = ep.methods.first().map(String::as_str).unwrap_or("GET");
    let path = join_context_path(ctx_path, &ep.full_path);
    let mut url = format!("{base_host}{path}");
    if !ep.query_params.is_empty() {
        let q: Vec<String> = ep.query_params.iter().map(|p| format!("{p}=")).collect();
        url.push('?');
        url.push_str(&q.join("&"));
    }
    let mut cmd = format!("curl -X {method} \"{url}\"");
    if ep.has_request_body {
        cmd.push_str(" -H \"Content-Type: application/json\" -d '{}'");
    }
    cmd
}
