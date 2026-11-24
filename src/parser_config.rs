use crate::model::{ConfigEntry, ConfigFile, ConfigFormat, EffectiveProperty};
use crate::parser_xml::extract_placeholders;
use crate::text_util::{LineIndex, mask_secret_value, normalize_relaxed_key};
use openidea_plugin::Range;
use std::collections::{BTreeMap, BTreeSet};

pub fn is_spring_config_path(path: &str) -> bool {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let lower = file_name.to_ascii_lowercase();
    (lower.starts_with("application") || lower.starts_with("bootstrap"))
        && (lower.ends_with(".yml") || lower.ends_with(".yaml") || lower.ends_with(".properties"))
}

pub fn extract_profile_from_filename(path: &str) -> Option<String> {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = file_name
        .strip_suffix(".yaml")
        .or_else(|| file_name.strip_suffix(".yml"))
        .or_else(|| file_name.strip_suffix(".properties"))?;
    let rest = stem
        .strip_prefix("application-")
        .or_else(|| stem.strip_prefix("bootstrap-"))?;
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_owned())
    }
}

pub fn parse_config_file(path: &str, text: &str) -> ConfigFile {
    let lower = path.to_ascii_lowercase();
    let format = if lower.ends_with(".properties") {
        ConfigFormat::Properties
    } else {
        ConfigFormat::Yaml
    };
    let profile = extract_profile_from_filename(path);
    let idx = LineIndex::new(text);

    match format {
        ConfigFormat::Properties => parse_properties_file(path, text, profile, &idx),
        ConfigFormat::Yaml => parse_yaml_file(path, text, profile, &idx),
    }
}

fn parse_properties_file(path: &str, text: &str, profile: Option<String>, idx: &LineIndex) -> ConfigFile {
    let mut entries = Vec::new();
    let mut seen_per_doc: BTreeSet<String> = BTreeSet::new();
    let mut duplicate_keys = Vec::new();
    let mut imports = Vec::new();
    let mut doc_profile: Option<String> = None;

    let lines: Vec<&str> = text.split('\n').collect();
    let mut line_idx = 0usize;
    while line_idx < lines.len() {
        let start_line = line_idx;
        let line_start_byte = idx.line_start_byte(start_line as u32);
        let raw_line = lines[line_idx].trim_end_matches('\r');
        let trimmed = raw_line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            if trimmed == "#---" {
                seen_per_doc.clear();
                doc_profile = None;
            }
            line_idx += 1;
            continue;
        }
        if trimmed.starts_with("#---") {
            seen_per_doc.clear();
            doc_profile = None;
            line_idx += 1;
            continue;
        }

        // Accumulate backslash line continuations
        let mut combined = raw_line.to_owned();
        while combined.ends_with('\\') && !combined.ends_with("\\\\") && line_idx + 1 < lines.len() {
            combined.pop();
            line_idx += 1;
            combined.push_str(lines[line_idx].trim_end_matches('\r').trim_start());
        }

        let lead_ws = raw_line.len() - raw_line.trim_start().len();
        if let Some((sep_pos, _)) = find_prop_separator(&combined) {
            let key = combined[..sep_pos].trim().to_owned();
            let raw_val = combined[sep_pos + 1..].trim().to_owned();
            let value = unquote_value(&raw_val);

            let key_abs_start = line_start_byte + lead_ws;
            let key_abs_end = (key_abs_start + key.len()).min(text.len());
            let key_range = idx.range_from_bytes(text, key_abs_start, key_abs_end);

            let val_rel = raw_line
                .find(&raw_val)
                .filter(|p| *p >= lead_ws + key.len())
                .unwrap_or_else(|| raw_line.len().saturating_sub(raw_val.len()));
            let val_abs_start = line_start_byte + val_rel;
            let val_abs_end = (val_abs_start + raw_val.len()).min(text.len());
            let value_range = idx.range_from_bytes(text, val_abs_start, val_abs_end);
            let placeholders = extract_placeholders(&value, text, value_range, idx);

            let rk = normalize_relaxed_key(&key);
            if rk == "spring.config.activate.onprofile" || rk == "spring.profiles" {
                doc_profile = Some(value.clone());
            }
            if rk == "spring.config.import" {
                for part in value.split(',') {
                    let t = part.trim();
                    if !t.is_empty() {
                        imports.push((t.to_owned(), value_range));
                    }
                }
            }
            if !seen_per_doc.insert(rk.clone()) {
                duplicate_keys.push((key.clone(), key_range, start_line as u32));
            }
            entries.push(ConfigEntry {
                key,
                relaxed_key: rk,
                value,
                raw_value: raw_val,
                doc_profile: doc_profile.clone(),
                key_range,
                value_range,
                line: start_line as u32,
                placeholders,
            });
        }
        line_idx += 1;
    }

    ConfigFile {
        path: path.to_owned(),
        format: ConfigFormat::Properties,
        profile,
        entries,
        duplicate_keys,
        imports,
    }
}

fn find_prop_separator(line: &str) -> Option<(usize, char)> {
    let mut escaped = false;
    for (i, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '=' || ch == ':' {
            return Some((i, ch));
        }
    }
    None
}

fn parse_yaml_file(path: &str, text: &str, profile: Option<String>, idx: &LineIndex) -> ConfigFile {
    let mut entries: Vec<crate::model::ConfigEntry> = Vec::new();
    let mut duplicate_keys = Vec::new();
    let mut imports = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut seen_per_doc: BTreeSet<String> = BTreeSet::new();
    let mut doc_start_entry_idx = 0usize;
    let mut current_doc_profile: Option<String> = None;

    for (line_num, raw_line_cr) in text.split('\n').enumerate() {
        let raw_line = raw_line_cr.trim_end_matches('\r');
        let line_start_byte = idx.line_start_byte(line_num as u32);
        let stripped_comment = strip_yaml_comment(raw_line);
        let trimmed = stripped_comment.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "---" || trimmed.starts_with("--- ") {
            if let Some(dp) = current_doc_profile.take() {
                for entry in &mut entries[doc_start_entry_idx..] {
                    entry.doc_profile = Some(dp.clone());
                }
            }
            doc_start_entry_idx = entries.len();
            stack.clear();
            seen_per_doc.clear();
            continue;
        }
        if trimmed == "..." {
            continue;
        }

        let indent = raw_line.bytes().take_while(|b| *b == b' ').count();
        // Handle YAML list item `- value` attached to current stack key
        if let Some(item_rest) = trimmed.strip_prefix('-')
            && (item_rest.is_empty() || item_rest.starts_with(' '))
        {
            while stack.last().is_some_and(|(ind, _)| *ind >= indent) {
                stack.pop();
            }
            let item_val = item_rest.trim();
            if !item_val.is_empty() && !stack.is_empty() {
                // Append list item to last entry of same prefix if applicable
                let prefix = stack.iter().map(|(_, k)| k.as_str()).collect::<Vec<_>>().join(".");
                if let Some(last) = entries.iter_mut().rev().find(|e: &&mut ConfigEntry| e.key == prefix) {
                    let clean = unquote_value(item_val);
                    if last.value.is_empty() {
                        last.value = clean;
                    } else {
                        last.value.push(',');
                        last.value.push_str(&clean);
                    }
                    if normalize_relaxed_key(&last.key) == "spring.config.activate.onprofile"
                        || normalize_relaxed_key(&last.key) == "spring.profiles"
                    {
                        current_doc_profile = Some(last.value.clone());
                    }
                }
            }
            continue;
        }

        let Some(colon_pos) = find_yaml_colon(stripped_comment) else {
            continue;
        };

        while stack.last().is_some_and(|(ind, _)| *ind >= indent) {
            stack.pop();
        }

        let raw_key_part = &stripped_comment[indent..colon_pos];
        let key_segment = unquote_value(raw_key_part.trim());
        if key_segment.is_empty() {
            continue;
        }
        let key_abs_start = line_start_byte + indent;
        let key_abs_end = line_start_byte + colon_pos;
        let key_range = idx.range_from_bytes(text, key_abs_start, key_abs_end);

        stack.push((indent, key_segment.clone()));
        let full_key = stack.iter().map(|(_, k)| k.as_str()).collect::<Vec<_>>().join(".");
        let rest_after_colon = &stripped_comment[colon_pos + 1..];
        let val_trimmed = rest_after_colon.trim();

        let lead_in_rest = rest_after_colon.len() - rest_after_colon.trim_start().len();
        let val_abs_start = line_start_byte + colon_pos + 1 + lead_in_rest;
        let val_abs_end = val_abs_start + val_trimmed.len();
        let value_range = idx.range_from_bytes(text, val_abs_start, val_abs_end);

        if !val_trimmed.is_empty() {
            let value = parse_yaml_scalar_or_inline_list(val_trimmed);
            let placeholders = extract_placeholders(&value, text, value_range, idx);
            let rk = normalize_relaxed_key(&full_key);
            if rk == "spring.config.activate.onprofile" || rk == "spring.profiles" {
                current_doc_profile = Some(value.clone());
            }
            if rk == "spring.config.import" {
                for part in value.split(',') {
                    let t = part.trim();
                    if !t.is_empty() {
                        imports.push((t.to_owned(), value_range));
                    }
                }
            }
            if !seen_per_doc.insert(rk.clone()) {
                duplicate_keys.push((full_key.clone(), key_range, line_num as u32));
            }
            entries.push(ConfigEntry {
                key: full_key,
                relaxed_key: rk,
                value,
                raw_value: val_trimmed.to_owned(),
                doc_profile: None,
                key_range,
                value_range,
                line: line_num as u32,
                placeholders,
            });
        } else {
            // Possibly a parent object or multiline list header (`include:` followed by `- health`)
            let rk = normalize_relaxed_key(&full_key);
            if !seen_per_doc.insert(rk.clone()) {
                duplicate_keys.push((full_key.clone(), key_range, line_num as u32));
            }
            // Peek if next non-empty line is `- ` at >= indent
            let mut has_list_children = false;
            for next in text.split('\n').skip(line_num + 1) {
                let nt = strip_yaml_comment(next.trim_end_matches('\r')).trim();
                if nt.is_empty() {
                    continue;
                }
                if nt.starts_with("- ") {
                    has_list_children = true;
                }
                break;
            }
            if has_list_children {
                entries.push(ConfigEntry {
                    key: full_key,
                    relaxed_key: rk,
                    value: String::new(),
                    raw_value: String::new(),
                    doc_profile: None,
                    key_range,
                    value_range,
                    line: line_num as u32,
                    placeholders: Vec::new(),
                });
            }
        }
    }

    if let Some(dp) = current_doc_profile.take() {
        for entry in &mut entries[doc_start_entry_idx..] {
            entry.doc_profile = Some(dp.clone());
        }
    }

    ConfigFile {
        path: path.to_owned(),
        format: ConfigFormat::Yaml,
        profile,
        entries,
        duplicate_keys,
        imports,
    }
}

fn find_yaml_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
            continue;
        }
        if b == b'\'' || b == b'"' {
            quote = Some(b);
            continue;
        }
        if b == b':' && (i + 1 == bytes.len() || bytes[i + 1].is_ascii_whitespace()) {
            return Some(i);
        }
    }
    None
}

fn strip_yaml_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if let Some(q) = quote {
            if b == q && (b == b'\'' || i == 0 || bytes[i - 1] != b'\\') {
                quote = None;
            }
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = Some(b);
            continue;
        }
        if b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &line[..i];
        }
    }
    line
}

fn parse_yaml_scalar_or_inline_list(val: &str) -> String {
    let trimmed = val.trim();
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return inner
            .split(',')
            .map(|p| unquote_value(p.trim()))
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(",");
    }
    unquote_value(trimmed)
}

fn unquote_value(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        return t[1..t.len() - 1].to_owned();
    }
    t.to_owned()
}

pub fn doc_profile_matches(doc_profile: Option<&str>, active_profile: Option<&str>) -> bool {
    let Some(expr) = doc_profile else {
        return true;
    };
    let active = active_profile.unwrap_or("default");
    expr.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|part| {
            if let Some(neg) = part.strip_prefix('!') {
                neg != active
            } else {
                part == active
            }
        })
}

/// Resolves effective Spring Boot properties across base (`application.*`) and active profile (`application-<profile>.*`) files.
pub fn resolve_effective_properties(files: &[&ConfigFile], active_profile: Option<&str>) -> Vec<EffectiveProperty> {
    let mut sorted: Vec<(&ConfigFile, u8)> = Vec::new();
    for cf in files {
        let is_bootstrap = cf
            .path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .starts_with("bootstrap");
        let rank = match (&cf.profile, active_profile) {
            (None, _) => {
                if is_bootstrap {
                    0
                } else {
                    1
                }
            }
            (Some(p), Some(act)) if p == act => {
                if is_bootstrap {
                    2
                } else {
                    3
                }
            }
            _ => continue,
        };
        sorted.push((*cf, rank));
    }
    sorted.sort_by(|(a, ra), (b, rb)| ra.cmp(rb).then_with(|| a.path.cmp(&b.path)));

    let mut map: BTreeMap<String, EffectiveProperty> = BTreeMap::new();
    for (cf, _) in sorted {
        for entry in &cf.entries {
            if entry.doc_profile.is_some() && !doc_profile_matches(entry.doc_profile.as_deref(), active_profile) {
                continue;
            }
            let source_profile = entry
                .doc_profile
                .clone()
                .or_else(|| cf.profile.clone())
                .unwrap_or_else(|| "default".to_owned());
            let masked = mask_secret_value(&entry.key, &entry.value);
            if let Some(prev) = map.get_mut(&entry.relaxed_key) {
                let mut history = std::mem::take(&mut prev.overridden_from);
                history.push(format!("{} = {} ({})", prev.key, prev.masked_value, prev.source_profile));
                *prev = EffectiveProperty {
                    key: entry.key.clone(),
                    relaxed_key: entry.relaxed_key.clone(),
                    value: entry.value.clone(),
                    masked_value: masked,
                    source_file: cf.path.clone(),
                    source_profile,
                    line: entry.line,
                    key_range: entry.key_range,
                    overridden_from: history,
                };
            } else {
                map.insert(
                    entry.relaxed_key.clone(),
                    EffectiveProperty {
                        key: entry.key.clone(),
                        relaxed_key: entry.relaxed_key.clone(),
                        value: entry.value.clone(),
                        masked_value: masked,
                        source_file: cf.path.clone(),
                        source_profile,
                        line: entry.line,
                        key_range: entry.key_range,
                        overridden_from: Vec::new(),
                    },
                );
            }
        }
    }
    map.into_values().collect()
}

pub fn render_effective_for_diff(props: &[EffectiveProperty]) -> String {
    let mut lines = Vec::with_capacity(props.len());
    for p in props {
        lines.push(format!("{}={}  # {}", p.key, p.masked_value, p.source_profile));
    }
    lines.join("\n")
}

/// Converts a `.properties` file into clean nested Spring Boot YAML.
pub fn properties_to_yaml(text: &str) -> String {
    let cf = parse_config_file("application.properties", text);
    #[derive(Default)]
    struct Node {
        value: Option<String>,
        children: BTreeMap<String, Node>,
    }
    let mut root = Node::default();
    for entry in &cf.entries {
        let mut cur = &mut root;
        for seg in entry.key.split('.') {
            cur = cur.children.entry(seg.to_owned()).or_default();
        }
        cur.value = Some(entry.value.clone());
    }

    fn emit(node: &Node, indent: usize, out: &mut String) {
        for (k, child) in &node.children {
            for _ in 0..indent {
                out.push_str("  ");
            }
            out.push_str(k);
            if child.children.is_empty() {
                let v = child.value.as_deref().unwrap_or("");
                out.push_str(": ");
                out.push_str(&format_yaml_scalar(v));
                out.push('\n');
            } else {
                out.push_str(":\n");
                emit(child, indent + 1, out);
            }
        }
    }

    let mut out = String::new();
    emit(&root, 0, &mut out);
    out
}

fn format_yaml_scalar(v: &str) -> String {
    if v.is_empty() {
        return "\"\"".to_owned();
    }
    if v == "true" || v == "false" || v.parse::<i64>().is_ok() || v.parse::<f64>().is_ok() {
        return v.to_owned();
    }
    if v.contains(':') || v.contains('#') || v.starts_with('{') || v.starts_with('[') || v.contains("${") {
        let escaped = v.replace('"', "\\\"");
        return format!("\"{escaped}\"");
    }
    v.to_owned()
}

/// Converts a Spring Boot YAML file into flat `.properties` lines (preserving multi-doc profile dividers).
pub fn yaml_to_properties(text: &str) -> String {
    let cf = parse_config_file("application.yml", text);
    let mut out = String::new();
    let mut last_doc_prof: Option<String> = None;
    for (idx, entry) in cf.entries.iter().enumerate() {
        if idx > 0 && entry.doc_profile != last_doc_prof {
            out.push_str("#---\n");
        }
        last_doc_prof = entry.doc_profile.clone();
        out.push_str(&entry.key);
        out.push('=');
        out.push_str(&entry.value);
        out.push('\n');
    }
    out
}

/// Helper to locate YAML/Properties key under cursor `Range`.
pub fn config_entry_at(cf: &ConfigFile, pos: openidea_plugin::Position) -> Option<&ConfigEntry> {
    cf.entries
        .iter()
        .find(|e| (pos >= e.key_range.start && pos <= e.key_range.end) || e.line == pos.line)
}

#[allow(dead_code)]
fn _keep_range(_: Range) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multidoc_yaml_and_resolves_effective_profiles() {
        let base = r#"
server:
  port: 8080
spring:
  application:
    name: orders
  datasource:
    password: base-secret-password
---
spring:
  config:
    activate:
      on-profile: prod
server:
  port: 9090
"#;
        let dev = "server.port=8081\nspring.jpa.show-sql=true\n";
        let cf_base = parse_config_file("/p/src/main/resources/application.yml", base);
        let cf_dev = parse_config_file("/p/src/main/resources/application-dev.properties", dev);
        assert_eq!(cf_dev.profile.as_deref(), Some("dev"));

        let eff_dev = resolve_effective_properties(&[&cf_base, &cf_dev], Some("dev"));
        let port = eff_dev.iter().find(|p| p.key == "server.port").unwrap();
        assert_eq!(port.value, "8081");
        assert_eq!(port.source_profile, "dev");
        assert_eq!(port.overridden_from.len(), 1);
        let pass = eff_dev
            .iter()
            .find(|p| p.key == "spring.datasource.password")
            .unwrap();
        assert_eq!(pass.masked_value, "******");

        let eff_prod = resolve_effective_properties(&[&cf_base, &cf_dev], Some("prod"));
        let port_prod = eff_prod.iter().find(|p| p.key == "server.port").unwrap();
        assert_eq!(port_prod.value, "9090");
    }

    #[test]
    fn converts_properties_and_yaml_bidirectionally() {
        let props = "server.port=8080\nspring.application.name=demo\n";
        let yaml = properties_to_yaml(props);
        assert!(yaml.contains("server:\n  port: 8080\n"));
        assert!(yaml.contains("spring:\n  application:\n    name: demo\n"));
        let roundtrip = yaml_to_properties(&yaml);
        assert!(roundtrip.contains("server.port=8080\n"));
        assert!(roundtrip.contains("spring.application.name=demo\n"));
    }
}
