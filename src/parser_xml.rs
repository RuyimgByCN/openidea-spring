use crate::model::{
    BeanDef, BeanStereotype, ConfigEntry, ConfigFile, ConfigFormat, FileFacts, InjectionKind,
    InjectionSite, LinkTargetSite, PlaceholderRef,
};
use crate::text_util::{LineIndex, normalize_relaxed_key, simple_type_name};
use openidea_plugin::Range;
use std::collections::BTreeSet;

pub fn parse_spring_xml(path: &str, text: &str) -> FileFacts {
    let mut facts = FileFacts {
        path: path.to_owned(),
        language: "xml".to_owned(),
        ..FileFacts::default()
    };
    if !text.contains("<bean")
        && !text.contains("<property")
        && !text.contains("<constructor-arg")
        && !text.contains("<prop")
        && !text.contains("<import")
        && !text.contains("property-placeholder")
        && !text.contains("${")
    {
        return facts;
    }
    let idx = LineIndex::new(text);
    let mut config_entries = Vec::new();
    let mut i = 0usize;
    let bytes = text.as_bytes();
    let mut current_bean_id = String::new();
    let mut current_bean_class = String::new();

    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") {
            if let Some(end) = text[i + 4..].find("-->") {
                i += 4 + end + 3;
                continue;
            }
            break;
        }
        if bytes[i] == b'<' {
            let tag_start = i;
            if let Some((tag_end, raw_tag)) = read_xml_tag(text, i) {
                let trimmed = raw_tag.trim();
                if trimmed.starts_with("</beans") || trimmed.starts_with("</bean") {
                    if trimmed.starts_with("</bean") && !trimmed.starts_with("</beans") {
                        current_bean_id.clear();
                        current_bean_class.clear();
                    }
                    i = tag_end;
                    continue;
                }
                let self_closing = trimmed.ends_with('/');
                let name = tag_name(trimmed);
                let attrs = parse_attrs(trimmed, tag_start, text, &idx);

                match name {
                    "bean" => {
                        let id = attr_val(&attrs, "id")
                            .or_else(|| attr_val(&attrs, "name"))
                            .unwrap_or_default();
                        let class_fqn = attr_val(&attrs, "class").unwrap_or_default();
                        let scope = attr_val(&attrs, "scope").unwrap_or_else(|| "singleton".into());
                        let is_primary = attr_val(&attrs, "primary").is_some_and(|v| v == "true");
                        let is_lazy = attr_val(&attrs, "lazy-init").is_some_and(|v| v == "true");
                        let simple = simple_type_name(&class_fqn);
                        let effective_name = if id.is_empty() {
                            simple.clone()
                        } else {
                            id.clone()
                        };
                        let name_range = attr_range(&attrs, "id")
                            .or_else(|| attr_range(&attrs, "class"))
                            .unwrap_or_else(|| idx.range_from_bytes(text, tag_start, tag_end));
                        let decl_range = idx.range_from_bytes(text, tag_start, tag_end);
                        let line = decl_range.start.line;

                        if !effective_name.is_empty() || !simple.is_empty() {
                            facts.beans.push(BeanDef {
                                id: format!("{path}#{effective_name}#{line}"),
                                name: effective_name.clone(),
                                class_name: simple.clone(),
                                class_fqn: class_fqn.clone(),
                                package: class_fqn
                                    .rsplit_once('.')
                                    .map(|(p, _)| p.to_owned())
                                    .unwrap_or_default(),
                                stereotype: BeanStereotype::XmlBean,
                                scope,
                                is_primary,
                                is_lazy,
                                proxy_bean_methods: true,
                                qualifiers: if id.is_empty() {
                                    Vec::new()
                                } else {
                                    vec![(id.clone(), name_range)]
                                },
                                profiles: Vec::new(),
                                conditions: Vec::new(),
                                interfaces: Vec::new(),
                                superclass: None,
                                factory_bean: None,
                                file: path.to_owned(),
                                name_range,
                                decl_range,
                                line,
                            });
                        }
                        if !self_closing {
                            current_bean_id = effective_name;
                            current_bean_class = simple;
                        }
                    }
                    "property" | "constructor-arg" => {
                        if let Some((ref_val, ref_range)) = attr_with_range(&attrs, "ref") {
                            let prop_name = attr_val(&attrs, "name").unwrap_or_else(|| ref_val.clone());
                            facts.injections.push(InjectionSite {
                                owner_bean: if current_bean_id.is_empty() {
                                    "xml".to_owned()
                                } else {
                                    current_bean_id.clone()
                                },
                                owner_class: current_bean_class.clone(),
                                kind: InjectionKind::XmlRef,
                                field_or_param_name: prop_name,
                                raw_type: ref_val.clone(),
                                target_type: ref_val.clone(),
                                qualifier: Some((ref_val, ref_range)),
                                required: true,
                                is_collection_or_provider: false,
                                is_final: false,
                                file: path.to_owned(),
                                range: ref_range,
                                type_range: ref_range,
                                line: ref_range.start.line,
                            });
                        }
                        if let Some((val_text, val_range)) = attr_with_range(&attrs, "value") {
                            let placeholders = extract_placeholders(&val_text, text, val_range, &idx);
                            if let Some((key, key_range)) = attr_with_range(&attrs, "name")
                                && (!placeholders.is_empty() || key.contains('.'))
                            {
                                config_entries.push(ConfigEntry {
                                    key: key.clone(),
                                    relaxed_key: normalize_relaxed_key(&key),
                                    value: val_text.clone(),
                                    raw_value: val_text,
                                    doc_profile: None,
                                    key_range,
                                    value_range: val_range,
                                    line: key_range.start.line,
                                    placeholders,
                                });
                            }
                        }
                    }
                    "prop" => {
                        if let Some((key, key_range)) = attr_with_range(&attrs, "key") {
                            let body_start = tag_end;
                            let val_end = text[body_start..]
                                .find("</prop>")
                                .map_or(body_start, |p| body_start + p);
                            let val_text = text[body_start..val_end].trim().to_owned();
                            let val_range = idx.range_from_bytes(text, body_start, val_end);
                            let placeholders = extract_placeholders(&val_text, text, val_range, &idx);
                            config_entries.push(ConfigEntry {
                                key: key.clone(),
                                relaxed_key: normalize_relaxed_key(&key),
                                value: val_text.clone(),
                                raw_value: val_text,
                                doc_profile: None,
                                key_range,
                                value_range: val_range,
                                line: key_range.start.line,
                                placeholders,
                            });
                        }
                    }
                    "import" => {
                        if let Some((res, res_range)) = attr_with_range(&attrs, "resource") {
                            facts.link_targets.push(LinkTargetSite {
                                raw_target: res,
                                range: res_range,
                                file: path.to_owned(),
                            });
                        }
                    }
                    "property-placeholder" | "context:property-placeholder" | "properties" | "util:properties" => {
                        if let Some((loc, loc_range)) = attr_with_range(&attrs, "location") {
                            facts.link_targets.push(LinkTargetSite {
                                raw_target: loc,
                                range: loc_range,
                                file: path.to_owned(),
                            });
                        }
                    }
                    _ => {}
                }

                // Scan any `${...}` inside attribute values
                for (_, val, range) in &attrs {
                    let phs = extract_placeholders(val, text, *range, &idx);
                    for ph in phs {
                        facts.value_sites.push(crate::model::ValueExprSite {
                            raw: format!("${{{}}}", ph.key),
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

                i = tag_end;
                continue;
            }
        }
        i += 1;
    }

    if !config_entries.is_empty() {
        let mut seen = BTreeSet::new();
        let mut dups = Vec::new();
        for e in &config_entries {
            if !seen.insert(e.key.clone()) {
                dups.push((e.key.clone(), e.key_range, e.line));
            }
        }
        facts.config_file = Some(ConfigFile {
            path: path.to_owned(),
            format: ConfigFormat::Properties,
            profile: None,
            entries: config_entries,
            duplicate_keys: dups,
            imports: Vec::new(),
        });
    }

    facts
}

fn read_xml_tag(text: &str, start: usize) -> Option<(usize, &str)> {
    let bytes = text.as_bytes();
    let mut i = start + 1;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some((i + 1, &text[start..=i]));
        }
        i += 1;
    }
    None
}

fn tag_name(raw_tag: &str) -> &str {
    let inner = raw_tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/')
        .trim();
    inner
        .split(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("")
}

fn parse_attrs(raw_tag: &str, tag_start: usize, full_text: &str, idx: &LineIndex) -> Vec<(String, String, Range)> {
    let mut out = Vec::new();
    let bytes = raw_tag.as_bytes();
    let mut i = 1usize;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/' || bytes[i] == b'>') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
            i += 1;
        }
        let name = raw_tag[name_start..i].trim().to_owned();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let q = bytes[i];
            let val_start = i + 1;
            i += 1;
            while i < bytes.len() && bytes[i] != q {
                i += 1;
            }
            let val_end = i;
            if i < bytes.len() {
                i += 1;
            }
            let val = raw_tag[val_start..val_end].to_owned();
            let range = idx.range_from_bytes(full_text, tag_start + val_start, tag_start + val_end);
            out.push((name, val, range));
        }
    }
    out
}

fn attr_val(attrs: &[(String, String, Range)], key: &str) -> Option<String> {
    attrs.iter().find(|(k, _, _)| k == key).map(|(_, v, _)| v.clone())
}

fn attr_range(attrs: &[(String, String, Range)], key: &str) -> Option<Range> {
    attrs.iter().find(|(k, _, _)| k == key).map(|(_, _, r)| *r)
}

fn attr_with_range(attrs: &[(String, String, Range)], key: &str) -> Option<(String, Range)> {
    attrs
        .iter()
        .find(|(k, _, _)| k == key)
        .map(|(_, v, r)| (v.clone(), *r))
}

pub fn extract_placeholders(value: &str, full_text: &str, value_range: Range, idx: &LineIndex) -> Vec<PlaceholderRef> {
    let mut out = Vec::new();
    let base_byte = idx.pos_to_byte(full_text, value_range.start);
    let bytes = value.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let start = i;
            let key_start = i + 2;
            let mut j = key_start;
            let mut depth = 1usize;
            while j < bytes.len() && depth > 0 {
                if bytes[j] == b'$' && j + 1 < bytes.len() && bytes[j + 1] == b'{' {
                    depth += 1;
                    j += 2;
                    continue;
                }
                if bytes[j] == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if depth == 0 {
                let inner = &value[key_start..j];
                let (key_part, default_val) = match inner.split_once(':') {
                    Some((k, d)) => (k, Some(d.to_owned())),
                    None => (inner, None),
                };
                let key_trimmed = key_part.trim();
                if !key_trimmed.is_empty() {
                    let key_lead = key_part.len() - key_part.trim_start().len();
                    let k_abs_start = base_byte + key_start + key_lead;
                    let k_abs_end = k_abs_start + key_trimmed.len();
                    let f_abs_start = base_byte + start;
                    let f_abs_end = base_byte + j + 1;
                    out.push(PlaceholderRef {
                        key: key_trimmed.to_owned(),
                        default_value: default_val,
                        key_range: idx.range_from_bytes(full_text, k_abs_start, k_abs_end),
                        full_range: idx.range_from_bytes(full_text, f_abs_start, f_abs_end),
                    });
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spring_xml_beans_refs_and_placeholders() {
        let xml = r#"
<beans>
  <bean id="dataSource" class="com.zaxxer.hikari.HikariDataSource" primary="true">
    <property name="jdbcUrl" value="${db.url:jdbc:h2:mem:test}" />
  </bean>
  <bean id="userRepo" class="com.example.JdbcUserRepo">
    <constructor-arg ref="dataSource" />
  </bean>
  <import resource="classpath:extra-beans.xml" />
</beans>
"#;
        let facts = parse_spring_xml("/p/src/main/resources/applicationContext.xml", xml);
        assert_eq!(facts.beans.len(), 2);
        assert_eq!(facts.beans[0].name, "dataSource");
        assert!(facts.beans[0].is_primary);
        assert_eq!(facts.injections.len(), 1);
        assert_eq!(facts.injections[0].target_type, "dataSource");
        assert_eq!(facts.value_sites.len(), 1);
        assert_eq!(facts.value_sites[0].prop_key.as_deref(), Some("db.url"));
        assert_eq!(facts.link_targets.len(), 1);
        assert_eq!(facts.link_targets[0].raw_target, "classpath:extra-beans.xml");
    }

    #[test]
    fn parses_spring_xml_with_utf8_bom_and_non_ascii_text() {
        let xml = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<beans xmlns=\"http://www.springframework.org/schema/beans\"
\txmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:p=\"http://www.springframework.org/schema/p\"
\txmlns:tx=\"http://www.springframework.org/schema/tx\">
  <!-- 数据库配置 -->
  说明文本
  <bean id=\"dataSource\" class=\"com.zaxxer.hikari.HikariDataSource\">
    <property name=\"jdbcUrl\" value=\"${db.url:jdbc:h2:mem:test}\" />
    <prop key=\"app.title\">中文标题</prop>
  </bean>
  <bean id=\"userRepo\" class=\"com.example.JdbcUserRepo\">
    <constructor-arg ref=\"dataSource\" />
  </bean>
</beans>
";
        let facts = parse_spring_xml("/p/src/main/resources/applicationContext.xml", xml);
        assert_eq!(facts.beans.len(), 2);
        assert_eq!(facts.beans[0].name, "dataSource");
        assert_eq!(facts.injections.len(), 1);
        assert_eq!(facts.injections[0].target_type, "dataSource");
        assert_eq!(facts.value_sites.len(), 1);
        assert_eq!(facts.value_sites[0].prop_key.as_deref(), Some("db.url"));
        let cf = facts.config_file.expect("prop config entries");
        assert_eq!(cf.entries.len(), 2);
        assert_eq!(cf.entries[0].key, "jdbcUrl");
        assert_eq!(cf.entries[1].key, "app.title");
        assert_eq!(cf.entries[1].value, "中文标题");
    }
}
