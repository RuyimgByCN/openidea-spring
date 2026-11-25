use crate::model::{EntityDef, QuerySite, RepositoryDef, RepositoryMethod};
use crate::text_util::{LineIndex, to_lower_camel_case};
use openidea_plugin::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedPredicate {
    pub field_path: Vec<String>,
    pub operator: &'static str,
    pub param_count: usize,
    pub combinator: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedQueryAst {
    pub action: &'static str,
    pub distinct: bool,
    pub limit: Option<u32>,
    pub predicates: Vec<DerivedPredicate>,
    pub order_by: Vec<(Vec<String>, &'static str)>,
}

impl DerivedQueryAst {
    pub fn expected_params(&self) -> usize {
        self.predicates.iter().map(|p| p.param_count).sum()
    }

    pub fn to_jpql_preview(&self, entity_name: &str) -> String {
        let alias = entity_name
            .chars()
            .next()
            .map(|c| c.to_ascii_lowercase())
            .unwrap_or('e');
        let head = match self.action {
            "count" => format!("SELECT COUNT({alias}) FROM {entity_name} {alias}"),
            "exists" => format!("SELECT CASE WHEN COUNT({alias}) > 0 THEN true ELSE false END FROM {entity_name} {alias}"),
            "delete" | "remove" => format!("DELETE FROM {entity_name} {alias}"),
            _ if self.distinct => format!("SELECT DISTINCT {alias} FROM {entity_name} {alias}"),
            _ => format!("SELECT {alias} FROM {entity_name} {alias}"),
        };
        let mut out = head;
        if !self.predicates.is_empty() {
            out.push_str(" WHERE ");
            let mut param_idx = 1usize;
            for (i, pred) in self.predicates.iter().enumerate() {
                if i > 0
                    && let Some(comb) = pred.combinator
                {
                    out.push(' ');
                    out.push_str(comb);
                    out.push(' ');
                }
                let prop = pred.field_path.join(".");
                match pred.operator {
                    "IsNull" => out.push_str(&format!("{alias}.{prop} IS NULL")),
                    "IsNotNull" => out.push_str(&format!("{alias}.{prop} IS NOT NULL")),
                    "True" => out.push_str(&format!("{alias}.{prop} = true")),
                    "False" => out.push_str(&format!("{alias}.{prop} = false")),
                    "Between" => {
                        out.push_str(&format!("{alias}.{prop} BETWEEN ?{param_idx} AND ?{}", param_idx + 1));
                        param_idx += 2;
                    }
                    "In" => {
                        out.push_str(&format!("{alias}.{prop} IN ?{param_idx}"));
                        param_idx += 1;
                    }
                    "NotIn" => {
                        out.push_str(&format!("{alias}.{prop} NOT IN ?{param_idx}"));
                        param_idx += 1;
                    }
                    "Like" | "StartingWith" | "EndingWith" | "Containing" => {
                        out.push_str(&format!("{alias}.{prop} LIKE ?{param_idx}"));
                        param_idx += 1;
                    }
                    "NotLike" => {
                        out.push_str(&format!("{alias}.{prop} NOT LIKE ?{param_idx}"));
                        param_idx += 1;
                    }
                    "Not" => {
                        out.push_str(&format!("{alias}.{prop} <> ?{param_idx}"));
                        param_idx += 1;
                    }
                    "LessThan" | "Before" => {
                        out.push_str(&format!("{alias}.{prop} < ?{param_idx}"));
                        param_idx += 1;
                    }
                    "LessThanEqual" => {
                        out.push_str(&format!("{alias}.{prop} <= ?{param_idx}"));
                        param_idx += 1;
                    }
                    "GreaterThan" | "After" => {
                        out.push_str(&format!("{alias}.{prop} > ?{param_idx}"));
                        param_idx += 1;
                    }
                    "GreaterThanEqual" => {
                        out.push_str(&format!("{alias}.{prop} >= ?{param_idx}"));
                        param_idx += 1;
                    }
                    _ => {
                        out.push_str(&format!("{alias}.{prop} = ?{param_idx}"));
                        param_idx += 1;
                    }
                }
            }
        }
        if !self.order_by.is_empty() {
            out.push_str(" ORDER BY ");
            let parts: Vec<String> = self
                .order_by
                .iter()
                .map(|(path, dir)| format!("{alias}.{} {dir}", path.join(".")))
                .collect();
            out.push_str(&parts.join(", "));
        }
        out
    }
}

const OPERATORS: &[(&str, &str, usize)] = &[
    ("IsNotNull", "IsNotNull", 0),
    ("NotNull", "IsNotNull", 0),
    ("IsNull", "IsNull", 0),
    ("Null", "IsNull", 0),
    ("IsNotEmpty", "IsNotNull", 0),
    ("NotEmpty", "IsNotNull", 0),
    ("IsEmpty", "IsNull", 0),
    ("Empty", "IsNull", 0),
    ("StartingWith", "StartingWith", 1),
    ("EndingWith", "EndingWith", 1),
    ("Containing", "Containing", 1),
    ("NotContaining", "NotLike", 1),
    ("NotLike", "NotLike", 1),
    ("Like", "Like", 1),
    ("LessThanEqual", "LessThanEqual", 1),
    ("LessThan", "LessThan", 1),
    ("GreaterThanEqual", "GreaterThanEqual", 1),
    ("GreaterThan", "GreaterThan", 1),
    ("Before", "Before", 1),
    ("After", "After", 1),
    ("Between", "Between", 2),
    ("NotIn", "NotIn", 1),
    ("In", "In", 1),
    ("IsNot", "Not", 1),
    ("Not", "Not", 1),
    ("IsTrue", "True", 0),
    ("True", "True", 0),
    ("IsFalse", "False", 0),
    ("False", "False", 0),
    ("IsEqual", "Equals", 1),
    ("Equals", "Equals", 1),
    ("Is", "Equals", 1),
    ("Regex", "Regex", 1),
    ("MatchesRegex", "Regex", 1),
    ("Exists", "IsNotNull", 0),
];

pub fn parse_derived_query(method_name: &str) -> Option<DerivedQueryAst> {
    let prefixes = ["find", "read", "query", "get", "stream", "count", "exists", "delete", "remove"];
    let mut rest = "";
    let mut action = "";
    for p in prefixes {
        if let Some(r) = method_name.strip_prefix(p)
            && (r.is_empty() || r.starts_with(|c: char| c.is_ascii_uppercase()))
        {
            action = p;
            rest = r;
            break;
        }
    }
    if action.is_empty() {
        return None;
    }

    let by_idx = rest.find("By")?;
    let subject = &rest[..by_idx];
    let distinct = subject.contains("Distinct");
    let limit = extract_limit(subject);

    let after_by = &rest[by_idx + 2..];
    if after_by.is_empty() {
        return None;
    }

    let (pred_str, order_str) = match after_by.find("OrderBy") {
        Some(idx) => (&after_by[..idx], Some(&after_by[idx + "OrderBy".len()..])),
        None => (after_by, None),
    };

    let mut predicates = Vec::new();
    let chunks = split_combinators(pred_str);
    for (comb, chunk) in chunks {
        if chunk.is_empty() {
            return None;
        }
        let mut s = chunk;
        // Strip IgnoreCase / AllIgnoreCase suffixes
        for suffix in ["AllIgnoreCase", "IgnoreCase"] {
            if let Some(stripped) = s.strip_suffix(suffix)
                && !stripped.is_empty()
            {
                s = stripped;
                break;
            }
        }
        let mut op_name = "Equals";
        let mut param_count = 1usize;
        let mut field_raw = s;
        for &(suffix, op, count) in OPERATORS {
            if let Some(prefix) = s.strip_suffix(suffix)
                && !prefix.is_empty()
            {
                op_name = op;
                param_count = count;
                field_raw = prefix;
                break;
            }
        }
        let field_path = split_property_path(field_raw);
        if field_path.is_empty() {
            return None;
        }
        predicates.push(DerivedPredicate {
            field_path,
            operator: op_name,
            param_count,
            combinator: comb,
        });
    }

    let mut order_by = Vec::new();
    if let Some(mut ord) = order_str {
        while !ord.is_empty() {
            let asc_pos = ord.find("Asc");
            let desc_pos = ord.find("Desc");
            match (asc_pos, desc_pos) {
                (Some(a), Some(d)) => {
                    if a < d {
                        let p = split_property_path(&ord[..a]);
                        if !p.is_empty() {
                            order_by.push((p, "ASC"));
                        }
                        ord = &ord[a + 3..];
                    } else {
                        let p = split_property_path(&ord[..d]);
                        if !p.is_empty() {
                            order_by.push((p, "DESC"));
                        }
                        ord = &ord[d + 4..];
                    }
                }
                (Some(a), None) => {
                    let p = split_property_path(&ord[..a]);
                    if !p.is_empty() {
                        order_by.push((p, "ASC"));
                    }
                    ord = &ord[a + 3..];
                }
                (None, Some(d)) => {
                    let p = split_property_path(&ord[..d]);
                    if !p.is_empty() {
                        order_by.push((p, "DESC"));
                    }
                    ord = &ord[d + 4..];
                }
                (None, None) => {
                    let p = split_property_path(ord);
                    if !p.is_empty() {
                        order_by.push((p, "ASC"));
                    }
                    break;
                }
            }
        }
    }

    Some(DerivedQueryAst {
        action,
        distinct,
        limit,
        predicates,
        order_by,
    })
}

fn extract_limit(subject: &str) -> Option<u32> {
    for kw in ["First", "Top"] {
        if let Some(idx) = subject.find(kw) {
            let digits: String = subject[idx + kw.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            return Some(digits.parse::<u32>().unwrap_or(1));
        }
    }
    None
}

fn split_combinators(s: &str) -> Vec<(Option<&'static str>, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut next_comb: Option<&'static str> = None;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if i > start
            && bytes[i..].starts_with(b"And")
            && bytes.get(i + 3).is_some_and(|b| b.is_ascii_uppercase())
        {
            out.push((next_comb, &s[start..i]));
            next_comb = Some("AND");
            i += 3;
            start = i;
            continue;
        }
        if i > start
            && bytes[i..].starts_with(b"Or")
            && bytes.get(i + 2).is_some_and(|b| b.is_ascii_uppercase())
        {
            out.push((next_comb, &s[start..i]));
            next_comb = Some("OR");
            i += 2;
            start = i;
            continue;
        }
        i += 1;
    }
    out.push((next_comb, &s[start..]));
    out
}

fn split_property_path(raw: &str) -> Vec<String> {
    if raw.contains('_') {
        raw.split('_')
            .filter(|p| !p.is_empty())
            .map(to_lower_camel_case)
            .collect()
    } else if raw.is_empty() {
        Vec::new()
    } else {
        vec![to_lower_camel_case(raw)]
    }
}

pub fn count_bindable_method_params(method: &RepositoryMethod) -> usize {
    method
        .param_types
        .iter()
        .filter(|ty| {
            let simple = ty.rsplit('.').next().unwrap_or(ty);
            !matches!(simple, "Pageable" | "Sort" | "Limit" | "Score" | "ScrollPosition")
        })
        .count()
}

pub fn validate_repository_method(
    repo: &RepositoryDef,
    method: &RepositoryMethod,
    entities: &[&EntityDef],
) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(q) = &method.query {
        let bindable_count = count_bindable_method_params(method);
        for (pname, _) in &q.named_params {
            let has_explicit = method.explicit_params.iter().any(|(ep, _)| ep == pname);
            let has_param_name = method.param_names.iter().any(|pn| pn == pname);
            if !has_explicit && !has_param_name {
                errors.push(format!(
                    "Query parameter `:{pname}` in `{}.{}` has no matching `@Param(\"{pname}\")` or method parameter",
                    repo.name, method.name
                ));
            }
        }
        if q.positional_max as usize > bindable_count {
            errors.push(format!(
                "Query positional parameter `?{}` exceeds bindable method parameters ({bindable_count}) in `{}.{}`",
                q.positional_max, repo.name, method.name
            ));
        }
        return errors;
    }

    let Some(ast) = parse_derived_query(&method.name) else {
        return errors;
    };
    let expected = ast.expected_params();
    let actual = count_bindable_method_params(method);
    if expected != actual {
        errors.push(format!(
            "Derived query `{}.{}` expects {expected} bind parameter(s) but declares {actual}",
            repo.name, method.name
        ));
    }

    if let Some(entity) = entities.iter().find(|e| e.name == repo.entity_type) {
        for pred in &ast.predicates {
            if !entity_has_path(entity, &pred.field_path, entities) {
                errors.push(format!(
                    "Cannot resolve property `{}` on entity `{}` in `{}.{}`",
                    pred.field_path.join("."),
                    entity.name,
                    repo.name,
                    method.name
                ));
            }
        }
        for (ord_path, _) in &ast.order_by {
            if !entity_has_path(entity, ord_path, entities) {
                errors.push(format!(
                    "Cannot resolve OrderBy property `{}` on entity `{}` in `{}.{}`",
                    ord_path.join("."),
                    entity.name,
                    repo.name,
                    method.name
                ));
            }
        }
    }

    errors
}

fn entity_has_path(entity: &EntityDef, path: &[String], all_entities: &[&EntityDef]) -> bool {
    if path.is_empty() {
        return false;
    }
    let first = &path[0];
    let Some(field) = find_field_with_super(entity, first, all_entities, 0) else {
        return false;
    };
    if path.len() == 1 {
        return true;
    }
    let nested_type = field.type_name.split('<').next().unwrap_or(&field.type_name).trim();
    let simple = nested_type.rsplit('.').next().unwrap_or(nested_type);
    if let Some(target_entity) = all_entities.iter().find(|e| e.name == simple) {
        entity_has_path(target_entity, &path[1..], all_entities)
    } else {
        // Unresolved external embedded/JPA type — do not false-positive
        true
    }
}

fn find_field_with_super<'a>(
    entity: &'a EntityDef,
    field_name: &str,
    all_entities: &[&'a EntityDef],
    depth: usize,
) -> Option<&'a crate::model::EntityField> {
    if depth > 6 {
        return None;
    }
    if let Some(f) = entity.fields.iter().find(|f| f.name == field_name) {
        return Some(f);
    }
    if let Some(sup) = &entity.superclass
        && let Some(sup_entity) = all_entities.iter().find(|e| e.name == *sup)
    {
        return find_field_with_super(sup_entity, field_name, all_entities, depth + 1);
    }
    None
}

pub fn extract_query_params(query_text: &str, full_text: &str, inner_range: Range, idx: &LineIndex) -> (Vec<(String, Range)>, u32) {
    let base_byte = idx.pos_to_byte(full_text, inner_range.start);
    let bytes = query_text.as_bytes();
    let mut named = Vec::new();
    let mut max_pos = 0u32;
    let mut i = 0usize;
    let mut in_single = false;

    while i < bytes.len() {
        if bytes[i] == b'\'' {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if in_single {
            i += 1;
            continue;
        }
        if bytes[i] == b':' && i + 1 < bytes.len() && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_') {
            let s = i + 1;
            let mut e = s;
            while e < bytes.len() && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'_') {
                e += 1;
            }
            // Skip PostgreSQL `::type` casts
            if i == 0 || bytes[i - 1] != b':' {
                let name = query_text[s..e].to_owned();
                let range = idx.range_from_bytes(full_text, base_byte + s, base_byte + e);
                named.push((name, range));
            }
            i = e;
            continue;
        }
        if bytes[i] == b'?' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            let s = i + 1;
            let mut e = s;
            while e < bytes.len() && bytes[e].is_ascii_digit() {
                e += 1;
            }
            if let Ok(num) = query_text[s..e].parse::<u32>() {
                max_pos = max_pos.max(num);
            }
            i = e;
            continue;
        }
        i += 1;
    }

    (named, max_pos)
}

pub fn derived_method_completions(entity: &EntityDef) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for field in &entity.fields {
        let cap = capitalize_first(&field.name);
        out.push((
            format!("findBy{cap}"),
            format!("Optional<{}> findBy{cap}({} {})", entity.name, field.type_name, field.name),
        ));
        out.push((
            format!("findAllBy{cap}"),
            format!("List<{}> findAllBy{cap}({} {})", entity.name, field.type_name, field.name),
        ));
        out.push((
            format!("existsBy{cap}"),
            format!("boolean existsBy{cap}({} {})", field.type_name, field.name),
        ));
        out.push((
            format!("countBy{cap}"),
            format!("long countBy{cap}({} {})", field.type_name, field.name),
        ));
        out.push((
            format!("deleteBy{cap}"),
            format!("void deleteBy{cap}({} {})", field.type_name, field.name),
        ));
        if field.type_name.contains("String") {
            out.push((
                format!("findBy{cap}ContainingIgnoreCase"),
                format!(
                    "List<{}> findBy{cap}ContainingIgnoreCase(String {})",
                    entity.name, field.name
                ),
            ));
        }
    }
    out
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

#[allow(dead_code)]
fn _keep_query_site(_: &QuerySite) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EntityField;

    #[test]
    fn parses_derived_query_and_generates_jpql_preview() {
        let ast = parse_derived_query("findDistinctTop5ByEmailIgnoreCaseAndCreatedAtBetweenOrderByCreatedAtDesc")
            .expect("derived query");
        assert_eq!(ast.action, "find");
        assert!(ast.distinct);
        assert_eq!(ast.limit, Some(5));
        assert_eq!(ast.expected_params(), 3);
        let jpql = ast.to_jpql_preview("User");
        assert!(jpql.contains("SELECT DISTINCT u FROM User u WHERE u.email = ?1 AND u.createdAt BETWEEN ?2 AND ?3 ORDER BY u.createdAt DESC"));
    }

    #[test]
    fn validates_derived_query_against_entity_fields() {
        let user = EntityDef {
            name: "User".into(),
            fqn: "com.example.User".into(),
            table_name: "users".into(),
            superclass: None,
            fields: vec![
                EntityField { name: "id".into(), type_name: "Long".into(), column_name: "id".into(), is_id: true, range: Range::default() },
                EntityField { name: "email".into(), type_name: "String".into(), column_name: "email".into(), is_id: false, range: Range::default() },
            ],
            file: "/p/User.java".into(),
            range: Range::default(),
        };
        let repo = RepositoryDef {
            name: "UserRepository".into(),
            base_interface: "JpaRepository".into(),
            entity_type: "User".into(),
            id_type: "Long".into(),
            methods: vec![
                RepositoryMethod {
                    name: "findByMissingField".into(),
                    return_type: "List<User>".into(),
                    param_names: vec!["x".into()],
                    param_types: vec!["String".into()],
                    explicit_params: vec![],
                    query: None,
                    range: Range::default(),
                    name_range: Range::default(),
                    line: 10,
                },
            ],
            file: "/p/UserRepository.java".into(),
            range: Range::default(),
            line: 5,
        };
        let errs = validate_repository_method(&repo, &repo.methods[0], &[&user]);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("missingField"));
    }

    #[test]
    fn handles_non_ascii_derived_query_without_panicking() {
        let ast = parse_derived_query("findBy名称AndStatus").expect("derived query");
        assert_eq!(ast.predicates.len(), 2);
        assert_eq!(ast.predicates[0].field_path, vec!["名称"]);
        assert_eq!(ast.predicates[1].combinator, Some("AND"));
        assert_eq!(ast.predicates[1].field_path, vec!["status"]);
    }
}
