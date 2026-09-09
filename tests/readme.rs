//! The README read as an artifact, not as prose.
//!
//! `tests/check.rs` and `tests/proxy.rs` pin the places the README quotes
//! program *output*. This file pins the two claims it makes that nothing else
//! can check, and one link rule.
//!
//! # Six cases were deleted on 2026-09-09, and what they were for moved
//!
//! The file used to hold a scan per exported type, checking that every `pub fn`
//! appeared in a `## Reference` section, plus one checking that all eighteen
//! exported names appeared somewhere in the file. They were written on
//! 2026-09-08 against a README that carried a full API reference, and they
//! earned their keep: three entries in the equivalent Python section were wrong
//! within an hour of being written, and eleven of the eighteen names appeared
//! nowhere at all.
//!
//! The Reference section is gone, so those scans have no subject. **That is the
//! README changing purpose, not a check being dropped for convenience.** On
//! crates.io the long description and the API reference are two different
//! surfaces: docs.rs generates the second from the source, so a hand-written
//! copy in the first is a third copy of the contract that can only ever go
//! stale. `missing_docs` is `warn` in the manifest and `cargo doc` is clean, so
//! every exported item is documented where the documentation is derived.
//!
//! One direction of the old check still matters and is kept below, reversed:
//! the README shows calls, and a call it shows must exist. The old form asked
//! whether the code was fully documented, which is rustdoc's job. The new form
//! asks whether the documentation is true, which is nobody else's.

const README: &str = include_str!("../README.md");
const PROXY_RS: &str = include_str!("../src/proxy.rs");

#[test]
fn every_call_the_readme_shows_on_a_proxy_exists() {
    // Derived from the README text rather than from a list here, for the reason
    // in the module docs: a list in this file would be a third copy.
    //
    // The receiver has to be spelled `proxy` for a call to be picked up, which
    // is what keeps `reqwest::Proxy::all` and `Duration::from_secs` out of the
    // scan without an allowlist naming them. The `(` requirement is what keeps
    // `proxy.example.com` out - it appeared in two examples and parsed as a
    // method called `example` before that condition was added.
    let mut shown: Vec<String> = Vec::new();
    let mut rest = README;
    while let Some(at) = rest.find("proxy.") {
        rest = &rest[at + "proxy.".len()..];
        let name: String = rest
            .chars()
            .take_while(|character| character.is_alphanumeric() || *character == '_')
            .collect();
        if rest[name.len()..].starts_with('(') && !name.is_empty() && !shown.contains(&name) {
            shown.push(name);
        }
    }
    assert!(
        shown.len() >= 5,
        "found only {shown:?} calls in the README, so the scan is reporting on itself"
    );

    let missing: Vec<&String> = shown
        .iter()
        .filter(|name| !PROXY_RS.contains(&format!("pub fn {name}")))
        .collect();
    assert!(
        missing.is_empty(),
        "the README shows calls that are not public on Proxy: {missing:?}"
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
