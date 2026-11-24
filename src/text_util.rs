use openidea_plugin::{Position, Range};

/// Precomputed line byte offsets for fast O(log L) UTF-8 byte <-> UTF-16 Position conversion.
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    pub fn line_count(&self) -> u32 {
        self.starts.len() as u32
    }

    pub fn byte_to_pos(&self, text: &str, byte_offset: usize) -> Position {
        let clamped = clamp_char_boundary(text, byte_offset.min(text.len()));
        let line_idx = match self.starts.binary_search(&clamped) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.starts[line_idx];
        let slice = &text[line_start..clamped];
        let utf16_col: usize = slice.chars().map(char::len_utf16).sum();
        Position::new(line_idx as u32, utf16_col as u32)
    }

    pub fn pos_to_byte(&self, text: &str, pos: Position) -> usize {
        let line_idx = (pos.line as usize).min(self.starts.len().saturating_sub(1));
        let line_start = self.starts[line_idx];
        let next_start = self
            .starts
            .get(line_idx + 1)
            .copied()
            .unwrap_or(text.len());
        let line_end = text[line_start..next_start]
            .strip_suffix('\n')
            .map_or(next_start, |s| line_start + s.len());
        let line_end = text[line_start..line_end]
            .strip_suffix('\r')
            .map_or(line_end, |s| line_start + s.len());

        let mut utf16 = 0u32;
        let mut byte = line_start;
        for ch in text[line_start..line_end].chars() {
            if utf16 >= pos.character {
                break;
            }
            utf16 += ch.len_utf16() as u32;
            byte += ch.len_utf8();
        }
        byte
    }

    pub fn range_from_bytes(&self, text: &str, start: usize, end: usize) -> Range {
        let s = start.min(end);
        let e = start.max(end);
        Range::new(self.byte_to_pos(text, s), self.byte_to_pos(text, e))
    }

    pub fn line_start_byte(&self, line: u32) -> usize {
        self.starts
            .get(line as usize)
            .copied()
            .unwrap_or_else(|| *self.starts.last().unwrap_or(&0))
    }

    pub fn line_end_byte(&self, text: &str, line: u32) -> usize {
        let start = self.line_start_byte(line);
        let next = self
            .starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(text.len());
        let mut end = next;
        if end > start && text.as_bytes().get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        if end > start && text.as_bytes().get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        end
    }
}

pub fn clamp_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

pub fn position_contains(range: Range, pos: Position) -> bool {
    pos >= range.start && pos <= range.end
}

pub fn word_at(text: &str, idx: &LineIndex, pos: Position) -> Option<(String, Range)> {
    let byte = idx.pos_to_byte(text, pos);
    let is_word = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'$');
    let bytes = text.as_bytes();
    let mut s = byte;
    if s > 0 && (s == bytes.len() || !is_word(bytes[s])) && is_word(bytes[s - 1]) {
        s -= 1;
    }
    if s >= bytes.len() || !is_word(bytes[s]) {
        return None;
    }
    while s > 0 && is_word(bytes[s - 1]) {
        s -= 1;
    }
    let mut e = s;
    while e < bytes.len() && is_word(bytes[e]) {
        e += 1;
    }
    if s == e {
        return None;
    }
    Some((text[s..e].to_owned(), idx.range_from_bytes(text, s, e)))
}

pub fn ident_at(text: &str, idx: &LineIndex, pos: Position) -> Option<(String, Range)> {
    let byte = idx.pos_to_byte(text, pos);
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bytes = text.as_bytes();
    let mut s = byte;
    if s > 0 && (s == bytes.len() || !is_ident(bytes[s])) && is_ident(bytes[s - 1]) {
        s -= 1;
    }
    if s >= bytes.len() || !is_ident(bytes[s]) {
        return None;
    }
    while s > 0 && is_ident(bytes[s - 1]) {
        s -= 1;
    }
    let mut e = s;
    while e < bytes.len() && is_ident(bytes[e]) {
        e += 1;
    }
    Some((text[s..e].to_owned(), idx.range_from_bytes(text, s, e)))
}

/// Finds a double-quoted string literal surrounding `pos`, returning `(inner_text, inner_range)`.
pub fn quoted_string_at(text: &str, idx: &LineIndex, pos: Position) -> Option<(String, Range)> {
    let byte = idx.pos_to_byte(text, pos);
    let line_start = idx.line_start_byte(pos.line);
    let line_end = idx.line_end_byte(text, pos.line);
    let line = &text[line_start..line_end];
    let rel = byte.saturating_sub(line_start);

    let mut i = 0;
    let bytes = line.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start_inner = i + 1;
            let mut j = start_inner;
            let mut escaped = false;
            while j < bytes.len() {
                if escaped {
                    escaped = false;
                } else if bytes[j] == b'\\' {
                    escaped = true;
                } else if bytes[j] == b'"' {
                    if rel >= i && rel <= j + 1 {
                        return Some((
                            line[start_inner..j].to_owned(),
                            idx.range_from_bytes(text, line_start + start_inner, line_start + j),
                        ));
                    }
                    i = j + 1;
                    break;
                }
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Normalizes Spring Boot relaxed property keys:
/// `server.servlet.contextPath` / `SERVER_SERVLET_CONTEXT_PATH` -> `server.servlet.contextpath`.
pub fn normalize_relaxed_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut chars = key.trim().chars().peekable();
    let mut in_bracket = false;
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                in_bracket = true;
                out.push('.');
            }
            ']' => {
                in_bracket = false;
            }
            '-' | '_' if !in_bracket => {}
            '_' if in_bracket => {}
            '.' => {
                if !out.ends_with('.') && !out.is_empty() {
                    out.push('.');
                }
            }
            _ => out.extend(c.to_lowercase()),
        }
    }
    while out.ends_with('.') {
        out.pop();
    }
    out
}

pub fn to_kebab_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    let mut prev_lower_or_digit = false;
    for ch in input.chars() {
        if ch == '_' {
            if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
            prev_lower_or_digit = false;
        } else if ch.is_ascii_uppercase() {
            if prev_lower_or_digit && !out.ends_with('-') && !out.ends_with('.') {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower_or_digit = false;
        } else {
            out.push(ch);
            prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    out
}

pub fn to_lower_camel_case(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let second_upper = chars.clone().next().is_some_and(|c| c.is_ascii_uppercase());
    if first.is_ascii_uppercase() && second_upper {
        // Spring Introspector.decapitalize keeps acronyms uppercase (e.g. URLService -> URLService)
        return trimmed.to_owned();
    }
    let mut out = String::with_capacity(trimmed.len());
    out.extend(first.to_lowercase());
    out.push_str(chars.as_str());
    out
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

pub fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "credential",
        "private",
        "authorization",
        "cookie",
        "apikey",
        "api_key",
        "api-key",
        "accesskey",
        "access_key",
        "keystore",
        "truststore",
    ]
    .iter()
    .any(|p| k.contains(p))
        || k.ends_with(".key")
        || k.ends_with("_key")
        || k.ends_with("-key")
}

pub fn redact_url_credentials(text: &str) -> String {
    let Some(scheme_idx) = text.find("://") else {
        return text.to_owned();
    };
    let after = scheme_idx + 3;
    let rest = &text[after..];
    let Some(at_rel) = rest.find('@') else {
        return text.to_owned();
    };
    let slash_rel = rest.find('/');
    if slash_rel.is_some_and(|s| at_rel > s) {
        return text.to_owned();
    }
    let authority = &rest[..at_rel];
    if let Some(colon_rel) = authority.find(':') {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..after + colon_rel + 1]);
        out.push_str("******");
        out.push_str(&text[after + at_rel..]);
        return out;
    }
    text.to_owned()
}

pub fn mask_secret_value(key: &str, value: &str) -> String {
    if is_sensitive_key(key) && !value.trim().is_empty() && !value.trim().starts_with("${") {
        "******".to_owned()
    } else {
        redact_url_credentials(value)
    }
}

pub fn simple_type_name(raw: &str) -> String {
    let no_generics = raw.split('<').next().unwrap_or(raw).trim();
    let no_array = no_generics.trim_end_matches("[]").trim_end_matches("...").trim();
    let base = no_array.rsplit('.').next().unwrap_or(no_array).trim();
    base.trim_end_matches('?').to_owned()
}

pub fn unwrap_container_type(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();
    for wrapper in [
        "List<",
        "Set<",
        "Collection<",
        "Iterable<",
        "Optional<",
        "ObjectProvider<",
        "Provider<",
    ] {
        if let Some(rest) = trimmed.strip_prefix(wrapper)
            && let Some(inner) = rest.strip_suffix('>')
        {
            return (simple_type_name(inner), wrapper == "Optional<" || wrapper == "ObjectProvider<");
        }
    }
    if let Some(rest) = trimmed.strip_prefix("Map<")
        && let Some(inner) = rest.strip_suffix('>')
        && let Some((_, val_t)) = inner.split_once(',')
    {
        return (simple_type_name(val_t), false);
    }
    (simple_type_name(trimmed), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_handles_utf16_surrogates_and_roundtrips() {
        let text = "hello\n😀world\nend";
        let idx = LineIndex::new(text);
        let byte_world = text.find("world").unwrap();
        let pos = idx.byte_to_pos(text, byte_world);
        assert_eq!(pos, Position::new(1, 2)); // 😀 is 2 UTF-16 code units
        assert_eq!(idx.pos_to_byte(text, pos), byte_world);
    }

    #[test]
    fn relaxed_key_normalization_matches_spring_boot() {
        assert_eq!(
            normalize_relaxed_key("server.servlet.contextPath"),
            "server.servlet.contextpath"
        );
        assert_eq!(
            normalize_relaxed_key("server.servlet.context-path"),
            "server.servlet.contextpath"
        );
        assert_eq!(
            normalize_relaxed_key("SERVER_SERVLET_CONTEXT_PATH"),
            "serverservletcontextpath"
        );
        assert_eq!(to_kebab_case("contextPath"), "context-path");
        assert_eq!(to_lower_camel_case("UserService"), "userService");
        assert_eq!(to_lower_camel_case("URLResolver"), "URLResolver");
    }

    #[test]
    fn secret_masking_protects_passwords_and_urls() {
        assert!(is_sensitive_key("spring.datasource.password"));
        assert!(!is_sensitive_key("spring.datasource.url"));
        assert_eq!(
            mask_secret_value("spring.datasource.password", "s3cr3t"),
            "******"
        );
        assert_eq!(
            mask_secret_value("spring.datasource.password", "${DB_PASS:}"),
            "${DB_PASS:}"
        );
        assert_eq!(
            mask_secret_value("spring.datasource.url", "postgresql://user:pass123@db:5432/prod"),
            "postgresql://user:******@db:5432/prod"
        );
    }
}
