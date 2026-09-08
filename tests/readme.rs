//! The Reference section is a contract, so it is compared against the code.
//!
//! `tests/check.rs` and `tests/proxy.rs` already pin the three places the README
//! quotes program *output*. This file pins the place it states an *API*, which
//! is a different failure: a quoted message that goes stale looks wrong to
//! anyone who runs it, and a signature that goes stale reads as authoritative
//! forever.
//!
//! The section was written on 2026-09-08 and three of its entries were wrong
//! within the hour of writing - in the Python README, where the same section
//! landed the same day: `load(id=...)` for `load(provider_id=...)`,
//! `load_file(path, *, provider_id=...)` for a positional argument, and a `Page`
//! accessor left out entirely. A reference written once and never checked is
//! worse than none, because it is believed.
//!
//! # Why this scans source text rather than asking the compiler
//!
//! Python does this with `inspect.signature` and `dir()`, so the list of things
//! to check is derived and a method added tomorrow is caught tomorrow. Rust has
//! no runtime reflection, and the obvious substitute - a hand-written list of
//! method names in this file - has exactly the defect it is meant to catch: it
//! is a third copy of the contract, and a method added to `proxy.rs` would be
//! missing from the README *and* from the list, and nothing would fail.
//!
//! So the list is derived from the source text instead. It is crude and it is
//! honest about being crude: it relies on `cargo fmt`, which puts `impl X {` at
//! column 0, its closing brace at column 0, and `pub fn` at exactly four spaces.
//! `cargo fmt --check` is in CI, so that layout is enforced by something other
//! than hope. If the scan ever finds nothing it fails rather than passing
//! vacuously - a check whose subject has silently become empty reports on
//! itself and not on its subject, which is a mistake this tree has made before
//! and paid for.

const README: &str = include_str!("../README.md");
const PROXY_RS: &str = include_str!("../src/proxy.rs");
const CHECK_RS: &str = include_str!("../src/check.rs");
const PROVIDER_RS: &str = include_str!("../src/provider.rs");

/// Every `pub fn` name inside `impl <name> {`, in source order.
///
/// Only the inherent `impl` is scanned, so a `Display` or `Debug` body cannot
/// contribute a name. The closing brace is the first line that is exactly `}`,
/// which is what rustfmt produces for a top-level block.
fn public_methods(source: &str, type_name: &str) -> Vec<String> {
    let header = format!("impl {type_name} {{");
    let mut names = Vec::new();
    let mut inside = false;
    for line in source.lines() {
        if line == header {
            inside = true;
            continue;
        }
        if inside {
            if line == "}" {
                break;
            }
            if let Some(rest) = line.strip_prefix("    pub fn ") {
                let name: String = rest
                    .chars()
                    .take_while(|character| character.is_alphanumeric() || *character == '_')
                    .collect();
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
    }
    assert!(
        !names.is_empty(),
        "found no public methods on {type_name}: the scan is reporting on itself"
    );
    names
}

/// The README from `heading` to the next heading of the same level or higher.
fn section<'a>(heading: &str, level: &str) -> &'a str {
    let start = README
        .find(heading)
        .unwrap_or_else(|| panic!("the README no longer has a {heading:?} heading"));
    let rest = &README[start + heading.len()..];
    let end = rest
        .match_indices('\n')
        .map(|(index, _)| index + 1)
        .find(|index| rest[*index..].starts_with(level) && !rest[*index..].starts_with("###"))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// Which of `names` the reference does not mention as a call.
fn undocumented(text: &str, names: &[String], skip: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter(|name| !skip.contains(&name.as_str()))
        .filter(|name| !text.contains(&format!("`.{name}(")) && !text.contains(&format!("{name}(")))
        .cloned()
        .collect()
}

#[test]
fn every_public_proxy_call_is_in_the_reference() {
    let text = section("### `Proxy` and `ProxyBuilder`", "##");
    let missing = undocumented(text, &public_methods(PROXY_RS, "Proxy"), &["builder"]);
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "public on Proxy, undocumented"
    );
}

#[test]
fn every_public_builder_call_is_in_the_reference() {
    let text = section("### `Proxy` and `ProxyBuilder`", "##");
    let missing = undocumented(text, &public_methods(PROXY_RS, "ProxyBuilder"), &[]);
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "public on ProxyBuilder, undocumented"
    );
}

#[test]
fn every_field_a_check_carries_is_in_the_reference() {
    let text = section("### `Check` and `Connect`", "##");
    let missing = undocumented(text, &public_methods(CHECK_RS, "Check"), &[]);
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "public on Check, undocumented"
    );
}

#[test]
fn every_public_connect_call_is_in_the_reference() {
    let text = section("### `Check` and `Connect`", "##");
    let missing = undocumented(text, &public_methods(CHECK_RS, "Connect"), &[]);
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "public on Connect, undocumented"
    );
}

#[test]
fn every_public_provider_call_is_in_the_reference() {
    let text = section("### `Provider` and the module functions", "##");
    let missing = undocumented(text, &public_methods(PROVIDER_RS, "Provider"), &[]);
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "public on Provider, undocumented"
    );
}

#[test]
fn every_exported_name_appears_somewhere_in_the_readme() {
    // The gap that produced this test, measured on this file on 2026-09-08:
    // eleven of the eighteen exported names appeared nowhere in 450 lines,
    // including `load`, which is the ordinary way to get the shipped definition,
    // and `Connect`, whose constructor an example already called.
    //
    // Derived from `lib.rs` rather than listed here, for the reason in the
    // module docs above: a hand-written list would be a third copy.
    let source = include_str!("../src/lib.rs");
    let mut exported: Vec<String> = Vec::new();
    let mut inside = false;
    for line in source.lines() {
        if line.starts_with("pub use ") {
            inside = true;
        }
        if inside {
            for token in line
                .trim_start_matches("pub use ")
                .split(|character: char| !character.is_alphanumeric() && character != '_')
            {
                let known =
                    token.chars().next().is_some_and(|first| {
                        first.is_ascii_uppercase() || first.is_ascii_lowercase()
                    }) && !["pub", "use", "crate", "check", "error", "provider", "proxy"]
                        .contains(&token);
                if known && !exported.contains(&token.to_string()) {
                    exported.push(token.to_string());
                }
            }
            if line.contains(';') {
                inside = false;
            }
        }
    }
    assert!(
        exported.len() >= 15,
        "the export scan found only {exported:?}, so it is reporting on itself"
    );
    let missing: Vec<&String> = exported
        .iter()
        .filter(|name| !README.contains(name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "exported and absent from the README: {missing:?}"
    );
}

#[test]
fn the_ttl_values_the_readme_names_are_the_ones_the_definition_names() {
    // `ttl` is the only parameter with measured accepted values, measured
    // refused values, and no `values` entry to enforce either - a list of four
    // would refuse a fifth the gateway takes. So these two prose artifacts are
    // the only carriers of the unit rule and nothing else makes them agree.
    //
    // Derived from the README row rather than listed here, for the reason in the
    // module docs: a list in this file would be a third copy of the contract.
    let row = README
        .lines()
        .find(|line| line.starts_with("| `ttl` |"))
        .expect("the README no longer has a `ttl` row in the parameter table");
    let examples = row
        .rsplit_once('|')
        .and_then(|(head, _)| head.rsplit_once('|'))
        .expect("three columns")
        .1;
    let named: Vec<&str> = examples.split('`').skip(1).step_by(2).collect();
    assert!(
        named.len() >= 2,
        "found only {named:?} in the ttl row, so the scan is reporting on itself"
    );

    let notes = nodemaven::load("nodemaven")
        .expect("the shipped definition")
        .notes()
        .to_string();
    let absent: Vec<&&str> = named
        .iter()
        .filter(|value| !notes.contains(**value))
        .collect();
    assert!(
        absent.is_empty(),
        "the README names ttl values the definition's notes do not: {absent:?}"
    );
}

#[test]
fn every_internal_anchor_points_at_a_heading_that_exists() {
    // CEO rule 1 for GitHub, relayed 2026-08-25: a production link that goes
    // nowhere gets fixed, not annotated. This file is the crates.io long
    // description, so a dead anchor here is on the package page.
    //
    // Ten of them were written by hand on 2026-09-08 in one sitting, which is
    // the condition under which this check earns its keep.
    let headings: Vec<String> = README
        .lines()
        .filter_map(|line| {
            line.strip_prefix("## ")
                .or_else(|| line.strip_prefix("### "))
        })
        .map(slug)
        .collect();
    assert!(
        headings.len() >= 8,
        "found only {headings:?} headings, so the scan is reporting on itself"
    );

    let mut dead: Vec<String> = Vec::new();
    let mut rest = README;
    while let Some(at) = rest.find("](#") {
        rest = &rest[at + 3..];
        let anchor: String = rest
            .chars()
            .take_while(|character| *character != ')')
            .collect();
        if !headings.contains(&anchor) {
            dead.push(anchor);
        }
    }
    assert!(dead.is_empty(), "anchors with no heading: {dead:?}");
}

/// GitHub's heading slug: lower-cased, punctuation dropped, spaces to `-`.
///
/// Backticks and periods are dropped rather than turned into `-`, which is what
/// GitHub does and is the case that matters here - `### \`Check\` and \`Connect\``
/// is `check-and-connect` and not `-check--and--connect-`.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|character| match character {
            'a'..='z' | '0'..='9' | '_' => Some(character),
            ' ' | '-' => Some('-'),
            _ => None,
        })
        .collect()
}
