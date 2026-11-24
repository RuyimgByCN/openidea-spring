use openidea_plugin::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronExplanation {
    pub valid: bool,
    pub summary_en: String,
    pub summary_zh: String,
    pub fields: Vec<(String, String)>,
    pub error: Option<String>,
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

pub fn parse_and_explain_cron(raw: &str) -> CronExplanation {
    let expr = raw.trim();
    if expr.is_empty() || expr == "-" {
        return CronExplanation {
            valid: expr == "-",
            summary_en: if expr == "-" { "Disabled cron trigger (`-`)".into() } else { String::new() },
            summary_zh: if expr == "-" { "已禁用定时任务 (`-`)".into() } else { String::new() },
            fields: Vec::new(),
            error: if expr.is_empty() { Some("Empty cron expression".into()) } else { None },
        };
    }

    if let Some((en, zh)) = match expr {
        "@yearly" | "@annually" => Some(("Once a year (Jan 1 at 00:00:00)", "每年一次（1月1日 00:00:00）")),
        "@monthly" => Some(("Once a month (1st at 00:00:00)", "每月一次（每月1日 00:00:00）")),
        "@weekly" => Some(("Once a week (Sunday at 00:00:00)", "每周一次（周日 00:00:00）")),
        "@daily" | "@midnight" => Some(("Once a day (00:00:00)", "每天一次（00:00:00）")),
        "@hourly" => Some(("Once an hour (minute 0, second 0)", "每小时一次（整点 0 分 0 秒）")),
        _ => None,
    } {
        return CronExplanation {
            valid: true,
            summary_en: en.to_owned(),
            summary_zh: zh.to_owned(),
            fields: vec![("Macro".into(), expr.to_owned())],
            error: None,
        };
    }

    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 6 {
        return CronExplanation {
            valid: false,
            summary_en: String::new(),
            summary_zh: String::new(),
            fields: Vec::new(),
            error: Some(format!(
                "Spring `@Scheduled(cron = ...)` expects 6 fields (sec min hour dom month dow) or a macro, found {}",
                parts.len()
            )),
        };
    }

    let specs = [
        ("Second", parts[0], 0, 59, false, false),
        ("Minute", parts[1], 0, 59, false, false),
        ("Hour", parts[2], 0, 23, false, false),
        ("Day of Month", parts[3], 1, 31, true, false),
        ("Month", parts[4], 1, 12, false, true),
        ("Day of Week", parts[5], 0, 7, true, false),
    ];

    let mut fields = Vec::with_capacity(6);
    for (idx, &(label, val, min, max, allow_q, is_month)) in specs.iter().enumerate() {
        if let Err(err) = validate_cron_field(val, min, max, allow_q, is_month, idx == 5, idx == 3) {
            return CronExplanation {
                valid: false,
                summary_en: String::new(),
                summary_zh: String::new(),
                fields,
                error: Some(format!("Invalid {label} `{val}`: {err}")),
            };
        }
        fields.push((label.to_owned(), val.to_owned()));
    }

    let (en, zh) = summarize_6_fields(&parts);
    CronExplanation {
        valid: true,
        summary_en: en,
        summary_zh: zh,
        fields,
        error: None,
    }
}

fn validate_cron_field(
    val: &str,
    min: u32,
    max: u32,
    allow_question: bool,
    is_month: bool,
    is_dow: bool,
    is_dom: bool,
) -> Result<(), String> {
    if val == "?" {
        if allow_question {
            return Ok(());
        }
        return Err("`?` is only allowed in Day-of-Month or Day-of-Week".into());
    }
    if val == "*" {
        return Ok(());
    }
    for segment in val.split(',') {
        if segment.is_empty() {
            return Err("empty list item".into());
        }
        if is_dom && (segment == "L" || segment == "LW" || segment.ends_with('W')) {
            continue;
        }
        if is_dow && (segment.ends_with('L') || segment.contains('#')) {
            if let Some((day, nth)) = segment.split_once('#') {
                parse_cron_atom(day, min, max, is_month, is_dow)?;
                let n = nth.parse::<u32>().map_err(|_| format!("invalid nth `{nth}`"))?;
                if !(1..=5).contains(&n) {
                    return Err("nth weekday must be 1..=5".into());
                }
                continue;
            }
        }
        let (base, step) = match segment.split_once('/') {
            Some((b, st)) => {
                let step = st.parse::<u32>().map_err(|_| format!("invalid step `{st}`"))?;
                if step == 0 {
                    return Err("step must be > 0".into());
                }
                (b, Some(step))
            }
            None => (segment, None),
        };
        if base == "*" {
            continue;
        }
        if let Some((a, b)) = base.split_once('-') {
            let va = parse_cron_atom(a, min, max, is_month, is_dow)?;
            let vb = parse_cron_atom(b, min, max, is_month, is_dow)?;
            if !is_dow && va > vb {
                return Err(format!("range start `{a}` > end `{b}`"));
            }
        } else {
            parse_cron_atom(base, min, max, is_month, is_dow)?;
            let _ = step;
        }
    }
    Ok(())
}

fn parse_cron_atom(atom: &str, min: u32, max: u32, is_month: bool, is_dow: bool) -> Result<u32, String> {
    let upper = atom.trim().to_ascii_uppercase();
    if is_month
        && let Some(pos) = MONTHS.iter().position(|m| *m == upper)
    {
        return Ok(pos as u32 + 1);
    }
    if is_dow
        && let Some(pos) = DAYS.iter().position(|d| *d == upper)
    {
        return Ok(pos as u32);
    }
    let clean = upper.trim_end_matches('L').trim_end_matches('W');
    let num = clean
        .parse::<u32>()
        .map_err(|_| format!("unexpected token `{atom}`"))?;
    if num < min || num > max {
        return Err(format!("value {num} outside {min}..={max}"));
    }
    Ok(num)
}

fn summarize_6_fields(p: &[&str]) -> (String, String) {
    let (sec, min, hour, dom, mon, dow) = (p[0], p[1], p[2], p[3], p[4], p[5]);
    if sec.starts_with("*/") && min == "*" && hour == "*" {
        let n = &sec[2..];
        return (format!("Every {n} seconds"), format!("每 {n} 秒执行一次"));
    }
    if sec == "0" && min.starts_with("*/") && hour == "*" {
        let n = &min[2..];
        return (format!("Every {n} minutes"), format!("每 {n} 分钟执行一次"));
    }
    if sec == "0" && min == "0" && hour.starts_with("*/") {
        let n = &hour[2..];
        return (format!("Every {n} hours"), format!("每 {n} 小时执行一次"));
    }
    if sec == "*" && min == "*" && hour == "*" {
        return ("Every second".into(), "每秒执行一次".into());
    }
    if sec == "0" && min == "*" && hour == "*" {
        return ("Every minute".into(), "每分钟执行一次".into());
    }
    let time_en = if hour != "*" && min != "*" && sec != "*"
        && hour.chars().all(|c| c.is_ascii_digit())
        && min.chars().all(|c| c.is_ascii_digit())
        && sec.chars().all(|c| c.is_ascii_digit())
    {
        format!(
            "at {:02}:{:02}:{:02}",
            hour.parse::<u32>().unwrap_or(0),
            min.parse::<u32>().unwrap_or(0),
            sec.parse::<u32>().unwrap_or(0)
        )
    } else {
        format!("sec={sec}, min={min}, hour={hour}")
    };
    let date_en = match (dom, mon, dow) {
        ("*" | "?", "*", "*" | "?") => "every day".to_owned(),
        ("*" | "?", "*", d) => format!("on weekday {d}"),
        (d, "*", "*" | "?") => format!("on day {d} of month"),
        _ => format!("dom={dom}, month={mon}, dow={dow}"),
    };
    (format!("Runs {date_en} {time_en}"), format!("{date_en} · {time_en} 执行"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpelValidation {
    pub valid: bool,
    pub summary: String,
    pub error: Option<String>,
    pub bean_refs: Vec<(String, Range)>,
    pub prop_refs: Vec<(String, Range)>,
}

pub fn validate_spel(expr: &str) -> SpelValidation {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return SpelValidation {
            valid: false,
            summary: String::new(),
            error: Some("Empty SpEL expression".into()),
            bean_refs: Vec::new(),
            prop_refs: Vec::new(),
        };
    }

    let bytes = trimmed.as_bytes();
    let mut stack: Vec<char> = Vec::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_single {
            if c == '\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' => in_single = true,
            '"' => in_double = true,
            '(' | '[' | '{' => stack.push(c),
            ')' => {
                if stack.pop() != Some('(') {
                    return spel_err("Unmatched `)` in SpEL expression");
                }
            }
            ']' => {
                if stack.pop() != Some('[') {
                    return spel_err("Unmatched `]` in SpEL expression");
                }
            }
            '}' => {
                if stack.pop() != Some('{') {
                    return spel_err("Unmatched `}` in SpEL expression");
                }
            }
            _ => {}
        }
        i += 1;
    }
    if in_single || in_double {
        return spel_err("Unterminated string literal in SpEL expression");
    }
    if !stack.is_empty() {
        return spel_err("Unclosed bracket/parenthesis in SpEL expression");
    }

    let mut features = Vec::new();
    if trimmed.contains('@') {
        features.push("bean reference (`@bean`)");
    }
    if trimmed.contains("T(") {
        features.push("type expression (`T(...)`)");
    }
    if trimmed.contains("#root") || trimmed.contains("#this") {
        features.push("context variable (`#root`/`#this`)");
    }
    if trimmed.contains("${") {
        features.push("property placeholder (`${...}`)");
    }
    if trimmed.contains("?:") {
        features.push("Elvis fallback (`?:`)");
    }
    if trimmed.contains("?.") {
        features.push("safe navigation (`?.`)");
    }
    if trimmed.contains("hasRole") || trimmed.contains("hasAuthority") || trimmed.contains("isAuthenticated") {
        features.push("Spring Security authorization check");
    }
    let summary = if features.is_empty() {
        "Valid SpEL expression".to_owned()
    } else {
        format!("SpEL expression using {}", features.join(", "))
    };

    SpelValidation {
        valid: true,
        summary,
        error: None,
        bean_refs: Vec::new(),
        prop_refs: Vec::new(),
    }
}

fn spel_err(msg: &str) -> SpelValidation {
    SpelValidation {
        valid: false,
        summary: String::new(),
        error: Some(msg.to_owned()),
        bean_refs: Vec::new(),
        prop_refs: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_explains_6_field_spring_cron() {
        let c1 = parse_and_explain_cron("0 */5 * * * *");
        assert!(c1.valid);
        assert_eq!(c1.summary_en, "Every 5 minutes");
        assert_eq!(c1.summary_zh, "每 5 分钟执行一次");

        let c2 = parse_and_explain_cron("0 30 9 * * MON-FRI");
        assert!(c2.valid);
        assert!(c2.summary_en.contains("09:30:00"));

        let bad = parse_and_explain_cron("0 0 * * *"); // 5 fields instead of 6
        assert!(!bad.valid);
        assert!(bad.error.unwrap().contains("expects 6 fields"));

        let macro_daily = parse_and_explain_cron("@daily");
        assert!(macro_daily.valid);
    }

    #[test]
    fn validates_spel_expressions() {
        assert!(validate_spel("#{@myBean.enabled && hasRole('ADMIN')}").valid);
        assert!(!validate_spel("hasRole('ADMIN'").valid);
        assert!(!validate_spel("T(java.lang.Math.PI").valid);
    }
}
