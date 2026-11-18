//! Every text the desktop app passes to `tr`, `trf` or `trn` has a Brazilian Portuguese
//! translation, in the app's catalog or in the library's, and the app's catalog is sound.

use dreame_i18n::{add_catalog, check_catalog, literal_keys, lookup, Language};
use std::collections::BTreeSet;
use std::path::Path;

const PT_BR: &str = include_str!("../locales/pt-BR.toml");

/// Drops the white space between an opening parenthesis and a string literal, so a call such
/// as `tr(` with its text on the next line is found too.
fn join_calls(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '(' {
            let mut gap = String::new();
            while let Some(&w) = chars.peek().filter(|w| w.is_whitespace()) {
                gap.push(w);
                chars.next();
            }
            if chars.peek() != Some(&'"') {
                out.push_str(&gap);
            }
        }
    }
    out
}

/// The keys in the app's sources.
fn keys() -> BTreeSet<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut keys = BTreeSet::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "rs") {
            keys.extend(literal_keys(&join_calls(&std::fs::read_to_string(p).unwrap())));
        }
    }
    keys
}

fn missing() -> Vec<String> {
    add_catalog(Language::PortugueseBrazil, PT_BR).unwrap();
    keys().into_iter().filter(|k| lookup(Language::PortugueseBrazil, k).is_none()).collect()
}

#[test]
fn desktop_text_has_brazilian_portuguese() {
    let keys = keys();
    assert!(keys.len() > 100, "only {} keys found; is literal_keys still reading the sources?", keys.len());
    let missing = missing();
    assert!(missing.is_empty(), "{} keys lack pt-BR in apps/dreame-desktop/locales/pt-BR.toml:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn desktop_catalog_is_consistent() {
    let problems = check_catalog(PT_BR);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn calls_with_the_text_on_the_next_line_are_found() {
    let src = "let a = tr(\n    \"Hello\",\n); let b = f( x );";
    assert_eq!(join_calls(src), "let a = tr(\"Hello\",\n); let b = f( x );");
    assert_eq!(literal_keys(&join_calls(src)), vec!["Hello"]);
}

/// Prints the missing keys as catalog lines with empty values, for a translator to fill:
/// `cargo test -p dreame-desktop --test i18n -- --ignored print_missing --nocapture`.
#[test]
#[ignore]
fn print_missing() {
    for k in missing() {
        println!("{k:?} = \"\"");
    }
}
