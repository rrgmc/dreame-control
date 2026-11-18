//! Translations for the text the library and the apps show to people.
//!
//! Messages are keyed by their English text, as in gettext: `tr("Charging")` returns the text
//! in the current language, and the English itself when a catalog has no entry for it. So
//! English needs no catalog, and a missing translation shows English instead of a key.
//!
//! A catalog is a TOML file of `"English" = "Translation"` lines. This crate holds the catalog
//! for the library's own text. An app adds its own with [`add_catalog`] at start-up.
//!
//! Placeholders are written `{name}` and filled by [`trf`]. A translation must use the same
//! placeholders as its English key; the tests check that for every catalog.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{OnceLock, RwLock};

/// A language the apps are translated into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    /// English (United States). The text in the source code; it needs no catalog.
    #[default]
    English,
    /// Portuguese (Brazil).
    PortugueseBrazil,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::PortugueseBrazil];

    /// The BCP 47 tag, such as `pt-BR`.
    pub fn tag(self) -> &'static str {
        match self {
            Language::English => "en-US",
            Language::PortugueseBrazil => "pt-BR",
        }
    }

    /// The language's name in that language, for a language picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::PortugueseBrazil => "Português (Brasil)",
        }
    }

    /// The best supported language for a locale tag such as `pt-BR`, `pt_PT` or `en-GB`.
    /// Any Portuguese gives Brazilian Portuguese; anything else gives English.
    pub fn from_tag(tag: &str) -> Language {
        let primary = tag.split(['-', '_', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
        match primary.as_str() {
            "pt" => Language::PortugueseBrazil,
            _ => Language::English,
        }
    }
}

const LIBRARY_PT_BR: &str = include_str!("../locales/pt-BR.toml");

struct Store {
    language: Language,
    /// Per language: English key to translation. Leaked once per catalog, so lookups hand out
    /// `&'static str` without copying.
    catalogs: HashMap<Language, HashMap<&'static str, &'static str>>,
}

fn store() -> &'static RwLock<Store> {
    static STORE: OnceLock<RwLock<Store>> = OnceLock::new();
    STORE.get_or_init(|| {
        let mut s = Store { language: Language::English, catalogs: HashMap::new() };
        merge(&mut s, Language::PortugueseBrazil, LIBRARY_PT_BR).expect("library pt-BR catalog parses");
        RwLock::new(s)
    })
}

/// Parses a catalog into `(key, translation)` pairs.
pub fn parse_catalog(text: &str) -> Result<Vec<(String, String)>, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    table
        .into_iter()
        .map(|(k, v)| match v {
            toml::Value::String(s) => Ok((k, s)),
            other => Err(format!("{k:?}: expected a string, found {}", other.type_str())),
        })
        .collect()
}

fn merge(s: &mut Store, language: Language, text: &str) -> Result<(), String> {
    let pairs = parse_catalog(text)?;
    let map = s.catalogs.entry(language).or_default();
    for (k, v) in pairs {
        map.insert(Box::leak(k.into_boxed_str()), Box::leak(v.into_boxed_str()));
    }
    Ok(())
}

/// Adds an app's catalog for `language`. Its entries win over earlier ones with the same key.
pub fn add_catalog(language: Language, text: &str) -> Result<(), String> {
    let mut s = store().write().unwrap_or_else(|e| e.into_inner());
    merge(&mut s, language, text)
}

/// Switches the language for all later lookups.
pub fn set_language(language: Language) {
    store().write().unwrap_or_else(|e| e.into_inner()).language = language;
}

pub fn language() -> Language {
    store().read().unwrap_or_else(|e| e.into_inner()).language
}

/// The language the operating system asks for, mapped to a supported one.
pub fn system_language(locale: Option<&str>) -> Language {
    locale.map(Language::from_tag).unwrap_or_default()
}

/// `msg` in the current language, or `msg` itself when there is no translation.
pub fn tr(msg: &str) -> &str {
    let s = store().read().unwrap_or_else(|e| e.into_inner());
    if s.language == Language::English {
        return msg;
    }
    match s.catalogs.get(&s.language).and_then(|c| c.get(msg)) {
        Some(t) => t,
        None => msg,
    }
}

/// The translation of `msg` into `language`, if a catalog has one. For tests and tools that
/// check catalogs; apps use [`tr`].
pub fn lookup(language: Language, msg: &str) -> Option<&'static str> {
    let s = store().read().unwrap_or_else(|e| e.into_inner());
    s.catalogs.get(&language).and_then(|c| c.get(msg)).copied()
}

/// [`tr`], then each `{name}` replaced by its value.
pub fn trf(msg: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(tr(msg), args)
}

/// [`trf`] with `one` when `n` is 1 and `other` otherwise. Both English and Brazilian
/// Portuguese use the singular for exactly 1 in the app's texts (counts of rooms, minutes).
/// `{n}` is filled with `n`.
pub fn trn(one: &str, other: &str, n: i64, args: &[(&str, &dyn Display)]) -> String {
    let msg = if n == 1 { one } else { other };
    let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    fill(tr(msg), &all)
}

fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = template.to_string();
    for (name, value) in args {
        let key = format!("{{{name}}}");
        if out.contains(&key) {
            out = out.replace(&key, &value.to_string());
        }
    }
    out
}

/// The `{name}` placeholders in a message, sorted.
pub fn placeholders(msg: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = msg;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) if after[..end].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && end > 0 => {
                out.push(&after[..end]);
                rest = &after[end + 1..];
            }
            _ => rest = after,
        }
    }
    out.sort_unstable();
    out
}

/// Checks a catalog: it parses, no translation is empty, and each translation has the same
/// placeholders as its key. Returns one line per problem.
pub fn check_catalog(text: &str) -> Vec<String> {
    let pairs = match parse_catalog(text) {
        Ok(p) => p,
        Err(e) => return vec![e],
    };
    let mut problems = Vec::new();
    for (k, v) in &pairs {
        if v.trim().is_empty() {
            problems.push(format!("{k:?}: empty translation"));
        }
        if placeholders(k) != placeholders(v) {
            problems.push(format!("{k:?}: placeholders differ in {v:?}"));
        }
    }
    problems
}

/// Every string literal passed as the first argument of `tr(`, `trf(` or `trn(` (both
/// arguments for `trn`) in Rust source, also when the literal starts on the next line. Tests
/// use it to find text missing from a catalog.
pub fn literal_keys(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for call in ["tr(", "trf(", "trn("] {
        let mut rest = source;
        while let Some(i) = rest.find(call) {
            // Skip identifiers that only end in `tr`, such as `attr(`.
            let before = rest[..i].chars().next_back();
            let after_call = &rest[i + call.len()..];
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                rest = after_call;
                continue;
            }
            let Some(body) = after_call.trim_start().strip_prefix('"') else {
                rest = after_call;
                continue;
            };
            let (lit, tail) = read_literal(body);
            out.push(lit);
            rest = tail;
            if call == "trn(" {
                let t = rest.trim_start_matches([',', ' ', '\n', '\r', '\t']);
                if let Some(t) = t.strip_prefix('"') {
                    let (lit, tail) = read_literal(t);
                    out.push(lit);
                    rest = tail;
                }
            }
        }
    }
    out
}

/// Reads a string literal body up to its closing quote, resolving `\"`, `\\` and `\n`.
fn read_literal(s: &str) -> (String, &str) {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return (out, &s[i + 1..]),
            '\\' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, '\n')) | Some((_, '\r')) => {
                    // Line continuation: skip the following whitespace.
                    while let Some((_, c)) = chars.clone().next() {
                        if c.is_whitespace() {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                Some((_, other)) => out.push(other),
                None => break,
            },
            c => out.push(c),
        }
    }
    (out, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_from_tag() {
        assert_eq!(Language::from_tag("pt-BR"), Language::PortugueseBrazil);
        assert_eq!(Language::from_tag("pt_PT.UTF-8"), Language::PortugueseBrazil);
        assert_eq!(Language::from_tag("en-GB"), Language::English);
        assert_eq!(Language::from_tag("de"), Language::English);
        assert_eq!(Language::from_tag(""), Language::English);
    }

    #[test]
    fn placeholders_found() {
        assert_eq!(placeholders("Charging, {pct}% of {max}"), vec!["max", "pct"]);
        assert!(placeholders("{ not one }").is_empty());
    }

    #[test]
    fn library_catalog_is_consistent() {
        let problems = check_catalog(LIBRARY_PT_BR);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn literal_keys_read_calls() {
        let src = r#"let a = tr("Hello"); let b = trf("Hi {n}", &[]); x.attr("no"); trn("1 room", "{n} rooms", n, &[]); tr("Say \"hi\"")"#;
        assert_eq!(literal_keys(src), vec!["Hello", "Say \"hi\"", "Hi {n}", "1 room", "{n} rooms"]);
        assert_eq!(literal_keys("tr(
    \"Next line\",
)  tr(name)"), vec!["Next line"]);
    }

    #[test]
    fn lookups_switch_language_and_fall_back() {
        // One test owns the global language, so parallel tests do not race on it.
        add_catalog(Language::PortugueseBrazil, "\"Test only {x}\" = \"Só teste {x}\"").unwrap();
        set_language(Language::English);
        assert_eq!(trf("Test only {x}", &[("x", &1)]), "Test only 1");
        set_language(Language::PortugueseBrazil);
        assert_eq!(trf("Test only {x}", &[("x", &1)]), "Só teste 1");
        assert_eq!(tr("No such message"), "No such message");
        assert_eq!(trn("{n} test", "{n} tests", 2, &[]), "2 tests");
        set_language(Language::English);
    }
}
