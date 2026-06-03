//! Sensitive-data sanitization: replace emails, URLs, phone numbers, IPs,
//! credit-card-like and long digit runs with placeholders. Regex rules
//! mirror opendataloader's `FilterConfig`.

use regex::Regex;
use std::sync::OnceLock;

use crate::model::{AnalyzedDoc, Element};

struct Rule {
    re: Regex,
    repl: &'static str,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            Rule {
                re: Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap(),
                repl: "[EMAIL]",
            },
            Rule {
                re: Regex::new(r"https?://[A-Za-z0-9.\-]+(:\d+)?(/\S*)?").unwrap(),
                repl: "[URL]",
            },
            Rule {
                re: Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap(),
                repl: "[IP]",
            },
            Rule {
                re: Regex::new(r"\b\d{4}-?\d{4}-?\d{4}-?\d{4}\b").unwrap(),
                repl: "[CARD]",
            },
            Rule {
                re: Regex::new(r"[+]\d+(?:-\d+)+").unwrap(),
                repl: "[PHONE]",
            },
            Rule {
                re: Regex::new(r"\b\d{10,18}\b").unwrap(),
                repl: "[NUMBER]",
            },
        ]
    })
}

fn scrub(s: &str) -> String {
    let mut out = s.to_string();
    for rule in rules() {
        out = rule.re.replace_all(&out, rule.repl).into_owned();
    }
    out
}

pub fn sanitize_doc(doc: &mut AnalyzedDoc) {
    for el in &mut doc.elements {
        match el {
            Element::Heading { text, .. } | Element::Paragraph { text, .. } => {
                *text = scrub(text);
            }
            Element::List { items, .. } => {
                for it in items {
                    it.text = scrub(&it.text);
                }
            }
            Element::Table { rows, .. } => {
                for row in rows {
                    for cell in row {
                        cell.text = scrub(&cell.text);
                    }
                }
            }
            Element::Image { alt, .. } => {
                *alt = scrub(alt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn redacts_sensitive() {
        assert_eq!(
            scrub("mail me at a.b@x.com please"),
            "mail me at [EMAIL] please"
        );
        assert_eq!(scrub("see https://example.com/x?y=1 now"), "see [URL] now");
        assert_eq!(scrub("ip 192.168.0.1 here"), "ip [IP] here");
        assert_eq!(scrub("card 4111-1111-1111-1111"), "card [CARD]");
        assert_eq!(scrub("plain text stays"), "plain text stays");
    }
}
