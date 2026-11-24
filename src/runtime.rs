use crate::model::{BeanStereotype, SpringProjectModel};
use crate::parser_config::resolve_effective_properties;
use crate::text_util::{mask_secret_value, redact_url_credentials};
use openidea_plugin::details::{Pane, Tab};
use openidea_plugin::views::{Body, Button, TreeItem, View};
use openidea_plugin::{HttpRequest, PluginDataSource, Project, host};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAppInstance {
    pub id: String,
    pub source: String, // "agent" | "actuator"
    pub base_url: String,
    pub app_name: String,
    pub pid: Option<i64>,
    pub server_port: Option<u16>,
    pub context_path: String,
    pub active_profiles: Vec<String>,
    pub ready: bool,
    pub health_status: String,
    pub project_root: Option<String>,
    pub uptime_ms: Option<u64>,
    pub live_beans_json: Option<String>,
    pub mappings_json: Option<String>,
    pub env_json: Option<String>,
    pub conditions_json: Option<String>,
    pub metrics_json: Option<String>,
    pub loggers_json: Option<String>,
    pub health_json: Option<String>,
}

pub fn parse_port_range(raw: Option<&str>) -> (u16, u16) {
    let s = raw.unwrap_or("47391-47398").trim();
    if let Some((a, b)) = s.split_once('-')
        && let (Ok(pa), Ok(pb)) = (a.trim().parse::<u16>(), b.trim().parse::<u16>())
        && pa >= 1024
        && pa <= pb
    {
        return (pa, pb.min(pa.saturating_add(16)));
    }
    (47391, 47398)
}

pub fn parse_actuator_urls(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("http://localhost:8080/actuator")
        .split(',')
        .map(str::trim)
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .map(|s| s.trim_end_matches('/').to_owned())
        .collect()
}

pub fn probe_runtime_instances(
    project_root: Option<&str>,
    port_range: (u16, u16),
    actuator_urls: &[String],
) -> Vec<RuntimeAppInstance> {
    let mut instances = Vec::new();
    let mut seen_app_ports = std::collections::BTreeSet::new();

    // 1. Scan loopback Java Agent probes
    for port in port_range.0..=port_range.1 {
        let base = format!("http://127.0.0.1:{port}");
        let mut req = HttpRequest::get(&format!("{base}/info"));
        req.timeout_ms = Some(350);
        let Ok(resp) = host::fetch(&req) else {
            continue;
        };
        if resp.status != 200 {
            continue;
        }
        let Ok(info) = serde_json::from_str::<Value>(&resp.text()) else {
            continue;
        };
        let inst_root = info
            .get("projectRoot")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let (Some(pr), Some(ir)) = (project_root, inst_root.as_deref())
            && !ir.is_empty()
            && !ir.starts_with(pr)
            && !pr.starts_with(ir)
        {
            continue;
        }
        let server_port = info
            .get("serverPort")
            .and_then(Value::as_u64)
            .map(|p| p as u16);
        if let Some(sp) = server_port {
            seen_app_ports.insert(sp);
        }
        let app_name = info
            .get("appName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| info.get("mainClass").and_then(Value::as_str))
            .unwrap_or("Spring Boot App")
            .to_owned();
        let active_profiles = info
            .get("activeProfiles")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let context_path = info
            .get("contextPath")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();

        let health_json = fetch_json_text(&format!("{base}/health"), 450);
        let health_status = extract_health_status(health_json.as_deref());
        let live_beans_json = fetch_json_text(&format!("{base}/beans"), 600);
        let mappings_json = fetch_json_text(&format!("{base}/mappings"), 600);
        let env_json = fetch_json_text(&format!("{base}/env"), 600);
        let conditions_json = fetch_json_text(&format!("{base}/conditions"), 600);
        let metrics_json = fetch_json_text(&format!("{base}/metrics"), 500);
        let loggers_json = fetch_json_text(&format!("{base}/loggers"), 500);

        instances.push(RuntimeAppInstance {
            id: format!("agent:{port}"),
            source: "agent".into(),
            base_url: base,
            app_name,
            pid: info.get("pid").and_then(Value::as_i64),
            server_port,
            context_path,
            active_profiles,
            ready: info.get("ready").and_then(Value::as_bool).unwrap_or(true),
            health_status,
            project_root: inst_root,
            uptime_ms: info.get("uptimeMs").and_then(Value::as_u64),
            live_beans_json,
            mappings_json,
            env_json,
            conditions_json,
            metrics_json,
            loggers_json,
            health_json,
        });
    }

    // 2. Poll configured Spring Boot Actuator URLs
    for url in actuator_urls {
        let Some(health_json) = fetch_json_text(&format!("{url}/health"), 500) else {
            continue;
        };
        let port = extract_url_port(url);
        if port.is_some_and(|p| seen_app_ports.contains(&p)) {
            continue;
        }
        let info_json = fetch_json_text(&format!("{url}/info"), 450);
        let app_name = info_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| {
                v.pointer("/app/name")
                    .or_else(|| v.pointer("/build/artifact"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("Actuator ({})", url));

        let env_json = fetch_json_text(&format!("{url}/env"), 700).map(|s| sanitize_actuator_env(&s));
        let beans_json = fetch_json_text(&format!("{url}/beans"), 700);
        let mappings_json = fetch_json_text(&format!("{url}/mappings"), 700);
        let conditions_json = fetch_json_text(&format!("{url}/conditions"), 700);
        let metrics_json = fetch_json_text(&format!("{url}/metrics"), 500);
        let loggers_json = fetch_json_text(&format!("{url}/loggers"), 600);

        let profiles = env_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| v.get("activeProfiles").and_then(Value::as_array).cloned())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        instances.push(RuntimeAppInstance {
            id: format!("actuator:{url}"),
            source: "actuator".into(),
            base_url: url.clone(),
            app_name,
            pid: None,
            server_port: port,
            context_path: String::new(),
            active_profiles: profiles,
            ready: true,
            health_status: extract_health_status(Some(&health_json)),
            project_root: project_root.map(str::to_owned),
            uptime_ms: None,
            live_beans_json: beans_json,
            mappings_json,
            env_json,
            conditions_json,
            metrics_json,
            loggers_json,
            health_json: Some(health_json),
        });
    }

    instances
}

pub fn set_runtime_logger_level(instance: &RuntimeAppInstance, logger_name: &str, level: Option<&str>) -> Result<(), String> {
    let url = format!("{}/loggers/{}", instance.base_url.trim_end_matches('/'), logger_name);
    let body = json!({ "configuredLevel": level }).to_string();
    let mut req = HttpRequest::post(&url, body.into_bytes()).header("Content-Type", "application/json");
    if instance.source == "agent" {
        req = req.header("X-OpenIDEA-Spring", "1");
    }
    req.timeout_ms = Some(2000);
    let resp = host::fetch(&req).map_err(|e| e.to_string())?;
    if resp.status >= 200 && resp.status < 300 {
        Ok(())
    } else {
        Err(format!("HTTP {} updating logger `{logger_name}`", resp.status))
    }
}

pub fn extract_runtime_mapping_paths(instances: &[RuntimeAppInstance]) -> Vec<String> {
    let mut paths = Vec::new();
    for inst in instances {
        if let Some(raw) = &inst.mappings_json
            && let Ok(val) = serde_json::from_str::<Value>(raw)
        {
            collect_mapping_patterns(&val, &mut paths);
        }
    }
    paths
}

fn collect_mapping_patterns(val: &Value, out: &mut Vec<String>) {
    match val {
        Value::Object(map) => {
            if let Some(patterns) = map.get("patterns").and_then(Value::as_array) {
                for p in patterns {
                    if let Some(s) = p.as_str() {
                        out.push(s.to_owned());
                    }
                }
            }
            for v in map.values() {
                collect_mapping_patterns(v, out);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_mapping_patterns(v, out);
            }
        }
        _ => {}
    }
}

fn fetch_json_text(url: &str, timeout_ms: u32) -> Option<String> {
    let mut req = HttpRequest::get(url);
    req.timeout_ms = Some(timeout_ms);
    let resp = host::fetch(&req).ok()?;
    if resp.status == 200 {
        Some(resp.text())
    } else {
        None
    }
}

fn extract_health_status(raw: Option<&str>) -> String {
    raw.and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.get("status").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| "UNKNOWN".to_owned())
}

fn extract_url_port(url: &str) -> Option<u16> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host_port = rest.split('/').next()?;
    let (_, port_s) = host_port.rsplit_once(':')?;
    port_s.parse().ok()
}

pub fn sanitize_actuator_env(raw: &str) -> String {
    let Ok(mut val) = serde_json::from_str::<Value>(raw) else {
        return raw.to_owned();
    };
    redact_env_value("", &mut val);
    serde_json::to_string_pretty(&val).unwrap_or_else(|_| raw.to_owned())
}

fn redact_env_value(key_hint: &str, val: &mut Value) {
    match val {
        Value::Object(map) => {
            let prop_name = map
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| key_hint.to_owned());
            for (k, v) in map.iter_mut() {
                let effective_hint = if k == "value" { prop_name.as_str() } else { k.as_str() };
                redact_env_value(effective_hint, v);
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_env_value(key_hint, item);
            }
        }
        Value::String(s) => {
            let masked = mask_secret_value(key_hint, s);
            *s = redact_url_credentials(&masked);
        }
        _ => {}
    }
}

// ============================================================================
// Services Tool Window (`location = "services"`) & Details Pane
// ============================================================================

pub fn render_services_view(
    model: Option<&SpringProjectModel>,
    instances: &[RuntimeAppInstance],
) -> View {
    let toolbar = vec![
        Button::new("refresh", "Refresh").icon("refresh"),
        Button::new("connect", "Connect Actuator").icon("add"),
        Button::new("open-panel", "Open Panel").icon("open"),
    ];

    let mut nodes = Vec::new();
    for inst in instances {
        let port_str = inst
            .server_port
            .map(|p| format!(":{p}"))
            .unwrap_or_default();
        let prof_str = if inst.active_profiles.is_empty() {
            String::new()
        } else {
            format!(" [{}]", inst.active_profiles.join(","))
        };
        let detail = format!("{}{port_str}{prof_str} · {}", inst.health_status, inst.source);
        nodes.push(
            TreeItem::new(&format!("app:{}", inst.id), &inst.app_name)
                .icon(if inst.health_status == "UP" { "green-checkmark" } else { "server" })
                .detail(&detail),
        );
    }

    if let Some(m) = model {
        for b in m.all_beans() {
            if b.stereotype == BeanStereotype::SpringBootApplication {
                let already_running = instances.iter().any(|i| {
                    i.app_name.eq_ignore_ascii_case(&b.class_name)
                        || i.project_root.as_deref() == Some(m.root.as_str())
                });
                let suffix = if already_running { "configured" } else { "offline" };
                nodes.push(
                    TreeItem::new(&format!("static:{}", b.id), &b.class_name)
                        .icon("spring-boot")
                        .detail(&format!("@SpringBootApplication · {suffix}")),
                );
            }
        }
    }

    if nodes.is_empty() {
        return View {
            toolbar,
            body: Body::Message(
                "No live Spring Boot instances detected. Run your Spring Boot app (with the optional Spring Agent or Actuator) or click Connect Actuator.".into(),
            ),
            status: Some("Spring Boot Services: idle".into()),
        };
    }

    let status = format!("{} live instance(s)", instances.len());
    View {
        toolbar,
        body: Body::Tree(nodes),
        status: Some(status),
    }
}

pub fn render_service_details(
    key: &str,
    model: Option<&SpringProjectModel>,
    instances: &[RuntimeAppInstance],
) -> Vec<Tab> {
    if let Some(inst_id) = key.strip_prefix("app:")
        && let Some(inst) = instances.iter().find(|i| i.id == inst_id)
    {
        let overview = format!(
            "Application: {}\nSource:      {} ({})\nHealth:      {}\nServer Port: {}\nContextPath: {}\nProfiles:    {}\nPID:         {}\nProject:     {}",
            inst.app_name,
            inst.source,
            inst.base_url,
            inst.health_status,
            inst.server_port.map_or("unknown".into(), |p| p.to_string()),
            if inst.context_path.is_empty() { "/" } else { &inst.context_path },
            if inst.active_profiles.is_empty() { "default".into() } else { inst.active_profiles.join(", ") },
            inst.pid.map_or("-".into(), |p| p.to_string()),
            inst.project_root.as_deref().unwrap_or("-"),
        );
        let mut tabs = vec![
            Tab::new("overview", "Overview", Pane::Text(overview)),
            Tab::new(
                "health",
                "Health",
                Pane::Text(pretty_or_msg(inst.health_json.as_deref(), "Health endpoint not exposed.")),
            ),
            Tab::new(
                "beans",
                "Beans",
                Pane::Text(pretty_or_msg(inst.live_beans_json.as_deref(), "Live Beans snapshot unavailable.")),
            ),
            Tab::new(
                "mappings",
                "Mappings",
                Pane::Text(pretty_or_msg(inst.mappings_json.as_deref(), "Mappings endpoint not exposed.")),
            ),
            Tab::new(
                "env",
                "Environment",
                Pane::Text(pretty_or_msg(inst.env_json.as_deref(), "Env endpoint not exposed.")),
            ),
            Tab::new(
                "loggers",
                "Loggers",
                Pane::Text(pretty_or_msg(inst.loggers_json.as_deref(), "Loggers endpoint not exposed.")),
            ),
        ];
        tabs.truncate(8);
        return tabs;
    }

    if let Some(bean_id) = key.strip_prefix("static:")
        && let Some(m) = model
        && let Some(b) = m.all_beans().into_iter().find(|x| x.id == bean_id)
    {
        let summary = format!(
            "Application Class: {}\nFile: {}\nStereotype: {}\nTotal Indexed Beans: {}\nTotal Endpoints: {}\nStarters: {}",
            b.class_fqn,
            b.file,
            b.stereotype.label(),
            m.all_beans().len(),
            m.all_endpoints().len(),
            m.starters.badges().join(", "),
        );
        return vec![Tab::new("overview", "Overview", Pane::Text(summary))];
    }

    vec![Tab::new(
        "overview",
        "Overview",
        Pane::Message("Select a Spring Boot instance to inspect live Beans, Health, Mappings, and Environment.".into()),
    )]
}

fn pretty_or_msg(raw: Option<&str>, empty_msg: &str) -> String {
    match raw {
        Some(s) if !s.trim().is_empty() => {
            if let Ok(v) = serde_json::from_str::<Value>(s) {
                let mut text = serde_json::to_string_pretty(&v).unwrap_or_else(|_| s.to_owned());
                if text.len() > 900 * 1024 {
                    text.truncate(900 * 1024);
                    text.push_str("\n... (truncated)");
                }
                text
            } else {
                s.to_owned()
            }
        }
        _ => empty_msg.to_owned(),
    }
}

// ============================================================================
// Datasource JDBC Parser & OpenIDEA Database Sync (`database.sources`)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSpringDataSource {
    pub driver: &'static str,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub options: Vec<(String, String)>,
}

pub fn extract_spring_datasource(
    model: &SpringProjectModel,
    active_profile: Option<&str>,
) -> Option<ParsedSpringDataSource> {
    let files = model.all_config_files();
    let eff = resolve_effective_properties(&files, active_profile);
    let get = |key: &str| -> Option<String> {
        eff.iter()
            .find(|p| p.relaxed_key == key)
            .map(|p| p.value.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let jdbc_url = get("spring.datasource.url")?;
    let user = get("spring.datasource.username");
    let password = get("spring.datasource.password").filter(|p| !p.starts_with("${"));
    parse_jdbc_url(&jdbc_url, user, password)
}

pub fn parse_jdbc_url(
    raw_url: &str,
    user: Option<String>,
    password: Option<String>,
) -> Option<ParsedSpringDataSource> {
    let rest = raw_url.trim().strip_prefix("jdbc:")?;
    if let Some(sqlite_rest) = rest.strip_prefix("sqlite:") {
        let db = sqlite_rest.split('?').next().unwrap_or(sqlite_rest).trim().to_owned();
        if db.is_empty() || db == ":memory:" {
            return None;
        }
        return Some(ParsedSpringDataSource {
            driver: "sqlite",
            host: None,
            port: None,
            database: Some(db),
            user: None,
            password: None,
            options: Vec::new(),
        });
    }

    let (driver, after_scheme): (&'static str, &str) = if let Some(r) = rest.strip_prefix("postgresql://") {
        ("postgres", r)
    } else if let Some(r) = rest.strip_prefix("mysql://") {
        ("mysql", r)
    } else if let Some(r) = rest.strip_prefix("mariadb://") {
        ("mysql", r)
    } else if let Some(r) = rest.strip_prefix("clickhouse://") {
        ("clickhouse", r)
    } else if let Some(r) = rest.strip_prefix("sqlserver://") {
        // jdbc:sqlserver://localhost:1433;databaseName=app
        let (host_port, semi_rest) = match r.split_once(';') {
            Some((hp, sr)) => (hp, Some(sr)),
            None => (r, None),
        };
        let (host, port) = split_host_port(host_port, 1433);
        let mut database = None;
        if let Some(sr) = semi_rest {
            for kv in sr.split(';') {
                if let Some((k, v)) = kv.split_once('=')
                    && k.trim().eq_ignore_ascii_case("databaseName")
                {
                    database = Some(v.trim().to_owned());
                }
            }
        }
        return Some(ParsedSpringDataSource {
            driver: "sqlServer",
            host: Some(host),
            port: Some(port),
            database,
            user,
            password,
            options: Vec::new(),
        });
    } else if let Some(r) = rest.strip_prefix("oracle:thin:@//") {
        ("oracle", r)
    } else {
        return None;
    };

    let (path_part, query_part) = match after_scheme.split_once('?') {
        Some((a, b)) => (a, Some(b)),
        None => (after_scheme, None),
    };
    let (host_port, db_part) = match path_part.split_once('/') {
        Some((hp, db)) => (hp, Some(db.trim().to_owned())),
        None => (path_part, None),
    };
    let default_port = match driver {
        "postgres" => 5432,
        "mysql" => 3306,
        "clickhouse" => 8123,
        "oracle" => 1521,
        _ => 5432,
    };
    let (host, port) = split_host_port(host_port, default_port);

    let mut options = Vec::new();
    if let Some(q) = query_part {
        for kv in q.split('&') {
            if let Some((k, v)) = kv.split_once('=') {
                let kl = k.to_ascii_lowercase();
                if ["password", "passwd", "pwd", "token", "secret", "credential"]
                    .iter()
                    .any(|s| kl.contains(s))
                {
                    continue;
                }
                options.push((k.to_owned(), v.to_owned()));
            }
        }
    }

    Some(ParsedSpringDataSource {
        driver,
        host: Some(host),
        port: Some(port),
        database: db_part.filter(|s| !s.is_empty()),
        user,
        password,
        options,
    })
}

fn split_host_port(hp: &str, default_port: u16) -> (String, u16) {
    if let Some((h, p)) = hp.rsplit_once(':')
        && let Ok(port) = p.parse::<u16>()
    {
        return (h.to_owned(), port);
    }
    (if hp.is_empty() { "localhost".into() } else { hp.to_owned() }, default_port)
}

pub fn sync_datasource_to_openidea(
    project: &Project,
    model: &SpringProjectModel,
    active_profile: Option<&str>,
) -> Result<String, String> {
    let Some(parsed) = extract_spring_datasource(model, active_profile) else {
        return Err("No supported `spring.datasource.url` found (supports postgres, mysql, mariadb, sqlite, sqlserver, clickhouse, oracle).".into());
    };
    let prof_label = active_profile.unwrap_or("default");
    let source_id = format!("spring-boot-{prof_label}");
    let display_name = format!(
        "Spring Boot ({prof_label}) — {}",
        parsed.database.as_deref().unwrap_or(parsed.driver)
    );
    let ds = PluginDataSource {
        id: source_id.clone(),
        name: display_name.clone(),
        driver: parsed.driver.to_owned(),
        host: parsed.host,
        port: parsed.port,
        database: parsed.database,
        user: parsed.user,
        url: None,
        options: parsed.options,
    };
    project.upsert_data_source(&ds).map_err(|e| e.to_string())?;
    if let Some(pw) = parsed.password.filter(|s| !s.is_empty()) {
        project
            .set_data_source_credential(&source_id, &pw)
            .map_err(|e| e.to_string())?;
    }
    let _ = project.connect_data_source(&source_id);
    Ok(display_name)
}

#[allow(dead_code)]
fn _keep_btree(_: BTreeMap<String, String>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jdbc_urls_without_leaking_secret_options() {
        let pg = parse_jdbc_url(
            "jdbc:postgresql://db.local:5433/orders?sslmode=require&password=leaked",
            Some("app".into()),
            Some("s3cr3t".into()),
        )
        .unwrap();
        assert_eq!(pg.driver, "postgres");
        assert_eq!(pg.host.as_deref(), Some("db.local"));
        assert_eq!(pg.port, Some(5433));
        assert_eq!(pg.database.as_deref(), Some("orders"));
        assert_eq!(pg.options, [("sslmode".into(), "require".into())]);

        let sqlsrv = parse_jdbc_url(
            "jdbc:sqlserver://localhost:1433;databaseName=billing",
            Some("sa".into()),
            None,
        )
        .unwrap();
        assert_eq!(sqlsrv.driver, "sqlServer");
        assert_eq!(sqlsrv.database.as_deref(), Some("billing"));
    }

    #[test]
    fn sanitizes_actuator_env_passwords() {
        let raw = r#"{"activeProfiles":["dev"],"propertySources":[{"properties":{"spring.datasource.password":{"value":"secret123"},"server.port":{"value":"8080"}}}]}"#;
        let sanitized = sanitize_actuator_env(raw);
        assert!(!sanitized.contains("secret123"));
        assert!(sanitized.contains("******"));
        assert!(sanitized.contains("8080"));
    }
}
