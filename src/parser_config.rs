use crate::model::{ConfigEntry, ConfigFile, ConfigFormat, EffectiveProperty};
use crate::parser_xml::extract_placeholders;
use crate::text_util::{LineIndex, mask_secret_value, normalize_relaxed_key};
use openidea_plugin::Range;
use serde::Deserialize;
use serde_yaml_ng::Value as YamlValue;
use std::collections::{BTreeMap, BTreeSet};

fn count_trailing_backslashes(s: &str) -> usize {
    s.bytes().rev().take_while(|b| *b == b'\\').count()
}

pub fn escape_properties_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for ch in key.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\x0c' => out.push_str("\\f"),
            '=' => out.push_str("\\="),
            ':' => out.push_str("\\:"),
            ' ' => out.push_str("\\ "),
            '#' => out.push_str("\\#"),
            '!' => out.push_str("\\!"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

pub fn escape_properties_value(val: &str) -> String {
    let mut out = String::with_capacity(val.len());
    let mut leading = true;
    for ch in val.chars() {
        if ch == ' ' && leading {
            out.push_str("\\ ");
            continue;
        }
        leading = false;
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn unescape_properties_token(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(esc) = chars.next() else {
            break;
        };
        match esc {
            't' => out.push('\t'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            'f' => out.push('\x0c'),
            'u' => {
                let mut hex = String::with_capacity(4);
                for _ in 0..4 {
                    if let Some(h) = chars.next() {
                        hex.push(h);
                    }
                }
                if hex.len() != 4 {
                    return Err(format!("Invalid \\uXXXX escape in .properties: \\u{hex}"));
                }
                let code = u32::from_str_radix(&hex, 16)
                    .map_err(|_| format!("Invalid \\uXXXX escape in .properties: \\u{hex}"))?;
                let c = char::from_u32(code)
                    .ok_or_else(|| format!("Invalid Unicode scalar \\u{hex}"))?;
                out.push(c);
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

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

        // Accumulate backslash line continuations (odd trailing-backslash count only)
        let mut combined = raw_line.to_owned();
        while count_trailing_backslashes(&combined) % 2 == 1 && line_idx + 1 < lines.len() {
            combined.pop();
            line_idx += 1;
            combined.push_str(lines[line_idx].trim_end_matches('\r').trim_start_matches([' ', '\t', '\x0c']));
        }

        let lead_ws = raw_line.len() - raw_line.trim_start().len();
        if let Some((sep_pos, _)) = find_prop_separator(&combined) {
            let raw_key = combined[..sep_pos].trim();
            let key = unescape_properties_token(raw_key).unwrap_or_else(|_| raw_key.to_owned());
            let raw_val = combined[sep_pos + 1..].trim_start_matches([' ', '\t', '\x0c']).to_owned();
            let value = unescape_properties_token(&raw_val).unwrap_or_else(|_| unquote_value(&raw_val));

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

enum PropPathSeg {
    Key(String),
    Index(usize),
}

enum PropNode {
    Scalar(String),
    Map(BTreeMap<String, PropNode>),
    Seq(BTreeMap<usize, PropNode>),
}

fn parse_prop_path(key: &str) -> Result<Vec<PropPathSeg>, String> {
    let mut segs = Vec::new();
    for part in key.split('.') {
        if part.is_empty() {
            return Err(format!("Invalid empty segment in property key '{key}'"));
        }
        if let Some(first_br) = part.find('[') {
            let head = &part[..first_br];
            if head.is_empty() {
                return Err(format!("Missing property key name before '[' in '{key}'"));
            }
            segs.push(PropPathSeg::Key(head.to_owned()));
            let mut rest = &part[first_br..];
            while !rest.is_empty() {
                let Some(inner_and_tail) = rest.strip_prefix('[') else {
                    return Err(format!("Unsupported bracket syntax in property key '{key}'"));
                };
                let Some(close_idx) = inner_and_tail.find(']') else {
                    return Err(format!("Unclosed '[' in property key '{key}'"));
                };
                let idx_str = &inner_and_tail[..close_idx];
                let idx = idx_str
                    .parse::<usize>()
                    .map_err(|_| format!("Non-numeric sequence index '[{idx_str}]' in '{key}'"))?;
                segs.push(PropPathSeg::Index(idx));
                rest = &inner_and_tail[close_idx + 1..];
            }
        } else {
            if part.contains(']') {
                return Err(format!("Unmatched ']' in property key '{key}'"));
            }
            segs.push(PropPathSeg::Key(part.to_owned()));
        }
    }
    Ok(segs)
}

fn insert_prop_node(node: &mut PropNode, path: &[PropPathSeg], value: String, full_key: &str) -> Result<(), String> {
    if path.is_empty() {
        return match node {
            PropNode::Scalar(_) => {
                *node = PropNode::Scalar(value);
                Ok(())
            }
            _ => Err(format!("Property key '{full_key}' conflicts with nested entries")),
        };
    }
    match &path[0] {
        PropPathSeg::Key(k) => {
            let PropNode::Map(map) = node else {
                return Err(format!("Property key '{full_key}' conflicts with a scalar or sequence"));
            };
            if path.len() == 1 {
                if map.contains_key(k) {
                    if let Some(PropNode::Map(_) | PropNode::Seq(_)) = map.get(k) {
                        return Err(format!("Property key '{full_key}' conflicts with nested entries"));
                    }
                }
                map.insert(k.clone(), PropNode::Scalar(value));
                Ok(())
            } else {
                let child = map.entry(k.clone()).or_insert_with(|| match path[1] {
                    PropPathSeg::Key(_) => PropNode::Map(BTreeMap::new()),
                    PropPathSeg::Index(_) => PropNode::Seq(BTreeMap::new()),
                });
                insert_prop_node(child, &path[1..], value, full_key)
            }
        }
        PropPathSeg::Index(idx) => {
            let PropNode::Seq(seq) = node else {
                return Err(format!("Property key '{full_key}' mixes sequence index with mapping or scalar"));
            };
            if path.len() == 1 {
                if let Some(PropNode::Map(_) | PropNode::Seq(_)) = seq.get(idx) {
                    return Err(format!("Property key '{full_key}' conflicts with nested entries"));
                }
                seq.insert(*idx, PropNode::Scalar(value));
                Ok(())
            } else {
                let child = seq.entry(*idx).or_insert_with(|| match path[1] {
                    PropPathSeg::Key(_) => PropNode::Map(BTreeMap::new()),
                    PropPathSeg::Index(_) => PropNode::Seq(BTreeMap::new()),
                });
                insert_prop_node(child, &path[1..], value, full_key)
            }
        }
    }
}

fn format_yaml_key_strict(k: &str) -> String {
    let needs_quote = k.is_empty()
        || k.starts_with(['-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', ' ', '\t'])
        || k.ends_with([' ', '\t'])
        || k.chars().any(|c| matches!(c, ':' | '#' | '\n' | '\r' | '\t' | '\\' | '"') || (c as u32) < 0x20 || (c as u32) == 0x7f)
        || matches!(
            k,
            "true" | "True" | "TRUE" | "false" | "False" | "FALSE" | "null" | "Null" | "NULL" | "~" | "yes" | "no" | "on" | "off"
        )
        || k.parse::<f64>().is_ok();
    if needs_quote {
        serde_json::to_string(k).unwrap_or_else(|_| format!("\"{k}\""))
    } else {
        k.to_owned()
    }
}

fn format_yaml_scalar(v: &str) -> String {
    if v == "true" || v == "false" {
        return v.to_owned();
    }
    // Canonical integers/floats without leading zeros remain unquoted; strings like "00123" stay quoted.
    if !v.is_empty() && is_canonical_yaml_number(v) {
        return v.to_owned();
    }
    let needs_quote = v.is_empty()
        || v.starts_with(['-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', ' ', '\t'])
        || v.ends_with([' ', '\t'])
        || v == "~"
        || matches!(
            v,
            "null" | "Null" | "NULL" | "True" | "TRUE" | "False" | "FALSE" | "yes" | "Yes" | "YES" | "no" | "No" | "NO" | "on" | "On" | "ON" | "off" | "Off" | "OFF"
        )
        || v.contains(": ")
        || v.ends_with(':')
        || v.contains(" #")
        || v.contains("${")
        || v.chars().any(|c| matches!(c, '\\' | '"' | '\n' | '\r' | '\t') || (c as u32) < 0x20 || (c as u32) == 0x7f)
        || v.parse::<f64>().is_ok();
    if needs_quote {
        serde_json::to_string(v).unwrap_or_else(|_| format!("\"{v}\""))
    } else {
        v.to_owned()
    }
}

fn is_canonical_yaml_number(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    if body.is_empty() {
        return false;
    }
    if body == "0" {
        return true;
    }
    if let Some((int_part, frac_exp)) = body.split_once('.') {
        if int_part.is_empty()
            || !int_part.bytes().all(|b| b.is_ascii_digit())
            || (int_part.len() > 1 && int_part.starts_with('0'))
        {
            return false;
        }
        return !frac_exp.is_empty() && s.parse::<f64>().is_ok_and(f64::is_finite);
    }
    !body.starts_with('0') && body.bytes().all(|b| b.is_ascii_digit()) && s.parse::<i64>().is_ok()
}

fn emit_yaml_node(node: &PropNode, indent: usize, out: &mut String) -> Result<(), String> {
    let pad = "  ".repeat(indent);
    match node {
        PropNode::Map(map) => {
            if map.is_empty() {
                out.push_str(&pad);
                out.push_str("{}\n");
                return Ok(());
            }
            for (k, child) in map {
                out.push_str(&pad);
                out.push_str(&format_yaml_key_strict(k));
                match child {
                    PropNode::Scalar(v) => {
                        out.push_str(": ");
                        out.push_str(&format_yaml_scalar(v));
                        out.push('\n');
                    }
                    PropNode::Map(m) if m.is_empty() => out.push_str(": {}\n"),
                    PropNode::Seq(s) if s.is_empty() => out.push_str(": []\n"),
                    _ => {
                        out.push_str(":\n");
                        emit_yaml_node(child, indent + 1, out)?;
                    }
                }
            }
            Ok(())
        }
        PropNode::Seq(seq) => {
            for idx in 0..seq.len() {
                if !seq.contains_key(&idx) {
                    return Err(format!("Non-contiguous sequence index [{idx}] in .properties input"));
                }
            }
            for item in seq.values() {
                match item {
                    PropNode::Scalar(v) => {
                        out.push_str(&pad);
                        out.push_str("- ");
                        out.push_str(&format_yaml_scalar(v));
                        out.push('\n');
                    }
                    PropNode::Map(map) => {
                        if map.is_empty() {
                            out.push_str(&pad);
                            out.push_str("- {}\n");
                            continue;
                        }
                        let inner_pad = "  ".repeat(indent + 1);
                        for (entry_idx, (k, v)) in map.iter().enumerate() {
                            if entry_idx == 0 {
                                out.push_str(&pad);
                                out.push_str("- ");
                            } else {
                                out.push_str(&inner_pad);
                            }
                            out.push_str(&format_yaml_key_strict(k));
                            match v {
                                PropNode::Scalar(sv) => {
                                    out.push_str(": ");
                                    out.push_str(&format_yaml_scalar(sv));
                                    out.push('\n');
                                }
                                PropNode::Map(m) if m.is_empty() => out.push_str(": {}\n"),
                                PropNode::Seq(s) if s.is_empty() => out.push_str(": []\n"),
                                _ => {
                                    out.push_str(":\n");
                                    emit_yaml_node(v, indent + 2, out)?;
                                }
                            }
                        }
                    }
                    PropNode::Seq(nested) => {
                        if nested.is_empty() {
                            out.push_str(&pad);
                            out.push_str("- []\n");
                        } else {
                            out.push_str(&pad);
                            out.push_str("-\n");
                            emit_yaml_node(item, indent + 1, out)?;
                        }
                    }
                }
            }
            Ok(())
        }
        PropNode::Scalar(v) => {
            out.push_str(&pad);
            out.push_str(&format_yaml_scalar(v));
            out.push('\n');
            Ok(())
        }
    }
}

fn parse_properties_documents(text: &str) -> Result<Vec<Vec<(String, String)>>, String> {
    let norm = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = norm.split('\n').collect();
    let mut docs: Vec<Vec<(String, String)>> = vec![Vec::new()];
    let mut i = 0usize;

    while i < lines.len() {
        let raw_line = lines[i];
        let trimmed = raw_line.trim_start_matches([' ', '\t', '\x0c']);
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if trimmed == "#---" || trimmed.starts_with("#---") {
            if !docs.last().is_some_and(Vec::is_empty) {
                docs.push(Vec::new());
            }
            i += 1;
            continue;
        }
        if trimmed.starts_with('#') || trimmed.starts_with('!') {
            i += 1;
            continue;
        }

        let mut logical = trimmed.to_owned();
        while count_trailing_backslashes(&logical) % 2 == 1 && i + 1 < lines.len() {
            logical.pop();
            i += 1;
            logical.push_str(lines[i].trim_start_matches([' ', '\t', '\x0c']));
        }
        i += 1;

        let bytes = logical.as_bytes();
        let mut p = 0usize;
        let mut key_end = bytes.len();
        let mut val_start = bytes.len();
        while p < bytes.len() {
            if bytes[p] == b'\\' {
                p += 2;
                continue;
            }
            if bytes[p] == b'=' || bytes[p] == b':' {
                key_end = p;
                let mut q = p + 1;
                while q < bytes.len() && matches!(bytes[q], b' ' | b'\t' | b'\x0c') {
                    q += 1;
                }
                val_start = q;
                break;
            }
            if matches!(bytes[p], b' ' | b'\t' | b'\x0c') {
                key_end = p;
                let mut q = p + 1;
                while q < bytes.len() && matches!(bytes[q], b' ' | b'\t' | b'\x0c') {
                    q += 1;
                }
                if q < bytes.len() && (bytes[q] == b'=' || bytes[q] == b':') {
                    q += 1;
                    while q < bytes.len() && matches!(bytes[q], b' ' | b'\t' | b'\x0c') {
                        q += 1;
                    }
                }
                val_start = q;
                break;
            }
            p += 1;
        }
        let k = unescape_properties_token(&logical[..key_end])?;
        if k.is_empty() {
            continue;
        }
        let v = unescape_properties_token(&logical[val_start..])?;
        docs.last_mut().unwrap().push((k, v));
    }
    docs.retain(|d| !d.is_empty());
    Ok(docs)
}

/// Converts a `.properties` file into clean nested Spring Boot YAML (preserving indexed sequences & escaping).
pub fn properties_to_yaml(text: &str) -> Result<String, String> {
    let docs = parse_properties_documents(text)?;
    if docs.is_empty() {
        return Err("No .properties entries found to convert".to_owned());
    }
    let mut rendered_docs = Vec::with_capacity(docs.len());
    for doc_entries in docs {
        let mut root = PropNode::Map(BTreeMap::new());
        for (k, v) in doc_entries {
            let segs = parse_prop_path(&k)?;
            if matches!(segs.first(), Some(PropPathSeg::Index(_))) {
                return Err(format!("Unsupported root sequence key '{k}'"));
            }
            insert_prop_node(&mut root, &segs, v, &k)?;
        }
        let mut doc_out = String::new();
        emit_yaml_node(&root, 0, &mut doc_out)?;
        rendered_docs.push(doc_out);
    }
    Ok(rendered_docs.join("---\n"))
}

fn yaml_key_to_string(key: &YamlValue) -> Result<String, String> {
    match key {
        YamlValue::String(s) => Ok(s.clone()),
        YamlValue::Bool(b) => Ok(b.to_string()),
        YamlValue::Number(n) => Ok(n.to_string()),
        other => Err(format!("Unsupported non-scalar YAML mapping key: {other:?}")),
    }
}

fn flatten_yaml_value(prefix: &str, val: &YamlValue, rows: &mut Vec<(String, String)>) -> Result<(), String> {
    match val {
        YamlValue::Mapping(map) => {
            if map.is_empty() {
                if !prefix.is_empty() {
                    rows.push((prefix.to_owned(), String::new()));
                }
                return Ok(());
            }
            for (k_val, v) in map {
                let key = yaml_key_to_string(k_val)?;
                if key == "<<" {
                    return Err("Unsupported unresolved YAML merge key '<<'".to_owned());
                }
                let next = if prefix.is_empty() {
                    key
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_yaml_value(&next, v, rows)?;
            }
            Ok(())
        }
        YamlValue::Sequence(seq) => {
            if seq.is_empty() {
                if !prefix.is_empty() {
                    rows.push((prefix.to_owned(), String::new()));
                }
                return Ok(());
            }
            for (idx, item) in seq.iter().enumerate() {
                let next = format!("{prefix}[{idx}]");
                flatten_yaml_value(&next, item, rows)?;
            }
            Ok(())
        }
        YamlValue::String(s) => {
            rows.push((prefix.to_owned(), s.clone()));
            Ok(())
        }
        YamlValue::Bool(b) => {
            rows.push((prefix.to_owned(), b.to_string()));
            Ok(())
        }
        YamlValue::Number(n) => {
            rows.push((prefix.to_owned(), n.to_string()));
            Ok(())
        }
        YamlValue::Null => {
            rows.push((prefix.to_owned(), String::new()));
            Ok(())
        }
        YamlValue::Tagged(tagged) => Err(format!("Unsupported YAML tag: {}", tagged.tag)),
    }
}

/// Converts a Spring Boot YAML file into structure-preserving, `java.util.Properties`-escaped `.properties` lines.
pub fn yaml_to_properties(text: &str) -> Result<String, String> {
    let mut rendered_docs = Vec::new();
    for doc_de in serde_yaml_ng::Deserializer::from_str(text) {
        let mut doc = YamlValue::deserialize(doc_de).map_err(|e| format!("Invalid YAML: {e}"))?;
        if matches!(doc, YamlValue::Null) {
            continue;
        }
        doc.apply_merge()
            .map_err(|e| format!("Unsupported YAML merge key: {e}"))?;
        if !matches!(doc, YamlValue::Mapping(_)) {
            return Err("Top-level YAML document must be a mapping to convert to .properties".to_owned());
        }
        let mut rows = Vec::new();
        flatten_yaml_value("", &doc, &mut rows)?;
        if rows.is_empty() {
            continue;
        }
        let mut doc_out = String::new();
        for (k, v) in rows {
            doc_out.push_str(&escape_properties_key(&k));
            doc_out.push('=');
            doc_out.push_str(&escape_properties_value(&v));
            doc_out.push('\n');
        }
        rendered_docs.push(doc_out);
    }
    if rendered_docs.is_empty() {
        return Err("No YAML properties found to convert".to_owned());
    }
    Ok(rendered_docs.join("#---\n"))
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
        let yaml = properties_to_yaml(props).unwrap();
        assert!(yaml.contains("server:\n  port: 8080\n"));
        assert!(yaml.contains("spring:\n  application:\n    name: demo\n"));
        let roundtrip = yaml_to_properties(&yaml).unwrap();
        assert!(roundtrip.contains("server.port=8080\n"));
        assert!(roundtrip.contains("spring.application.name=demo\n"));
    }

    #[test]
    fn preserves_mapping_sequences_backslashes_and_rejects_invalid_configs() {
        let yaml_seq = "servers:\n  - host: api.example.test\n    port: 8080\n  - host: backup.example.test\n    port: 8081\n";
        let props = yaml_to_properties(yaml_seq).unwrap();
        assert_eq!(
            props,
            "servers[0].host=api.example.test\nservers[0].port=8080\nservers[1].host=backup.example.test\nservers[1].port=8081\n"
        );
        let back_yaml = properties_to_yaml(&props).unwrap();
        assert_eq!(back_yaml, yaml_seq);

        // Windows trailing backslashes & quoted scalars survive round-trip cleanly
        let win_yaml = "app:\n  mode: safe\n  path: \"C:\\\\temp\\\\\"\n";
        let win_props = yaml_to_properties(win_yaml).unwrap();
        assert_eq!(win_props, "app.mode=safe\napp.path=C:\\\\temp\\\\\n");
        let back_win_yaml = properties_to_yaml(&win_props).unwrap();
        assert_eq!(back_win_yaml, win_yaml);

        // Unsupported tags, syntax errors, and non-mapping roots return Err
        assert!(yaml_to_properties("app: !custom foo\n").is_err());
        assert!(yaml_to_properties("a: [unclosed\n").is_err());
        assert!(yaml_to_properties("- only\n- list\n").is_err());

        // Path conflicts & sparse indices in .properties return Err
        assert!(properties_to_yaml("foo=bar\nfoo.baz=qux\n").is_err());
        assert!(properties_to_yaml("servers[0].host=a\nservers[2].host=b\n").is_err());
    }
}
