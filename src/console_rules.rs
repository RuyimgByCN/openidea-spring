use openidea_plugin::ConsoleRule;

/// Returns API 1.12 console highlight rules for Spring Boot Run & Debug output.
/// Every regex is linear-time in JS RegExp (no nested repetitions, lookarounds, or inline flags).
pub fn spring_console_rules() -> Vec<ConsoleRule> {
    vec![
        ConsoleRule::new(r"\bAPPLICATION FAILED TO START\b")
            .foreground("bright-red")
            .style("bold")
            .whole_line(),
        ConsoleRule::new(r"^\s*Caused by:\s")
            .foreground("red")
            .style("bold"),
        ConsoleRule::new(r"\sERROR\s")
            .foreground("red")
            .style("bold"),
        ConsoleRule::new(r"\sWARN\s")
            .foreground("yellow"),
        ConsoleRule::new(r"\sINFO\s")
            .foreground("green"),
        ConsoleRule::new(r"\sDEBUG\s")
            .foreground("cyan"),
        ConsoleRule::new(r"\sTRACE\s")
            .foreground("white")
            .style("dim"),
        ConsoleRule::new(r"Started\s+[A-Za-z0-9_$.]+\s+in\s+[0-9.]+\s+seconds")
            .foreground("bright-green")
            .style("bold"),
        ConsoleRule::new(r"(Tomcat|Netty|Jetty|Undertow)\s+(started|initialized)\s+on\s+port")
            .foreground("bright-cyan")
            .style("bold"),
        ConsoleRule::new(r"::\s*Spring Boot\s*::")
            .foreground("bright-green")
            .style("bold"),
        ConsoleRule::new(r"\bHikariPool-[0-9]+\s*-\s*")
            .foreground("blue"),
        ConsoleRule::new(r"\bHibernate:\s")
            .foreground("magenta")
            .style("bold"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_rules_are_compact_and_valid() {
        let rules = spring_console_rules();
        assert!(!rules.is_empty());
        assert!(rules.len() <= 100);
        for rule in &rules {
            assert!(!rule.pattern.is_empty());
            assert!(rule.pattern.len() <= 512);
            assert!(!rule.pattern.contains("(?"));
        }
    }
}
