//! A gateway dialect is data, never code.
//!
//! Every proxy vendor sells the same thing and spells it differently: its own
//! separators, its own parameter names, its own reaction to a mistake. That
//! difference is the whole of what a provider is from this crate's point of
//! view, so it is one TOML file rather than one Rust module.
//!
//! The reason is not tidiness. A module invites a single `if provider == ...`
//! somewhere nobody reviews, and from then on two gateways are no longer going
//! through the same code path. A data file cannot branch.
//!
//! [`load`] reads a definition compiled into this crate. [`load_file`] reads one
//! from disk, which is how the crate talks to a gateway we ship no definition
//! for - your own, or one you wrote yourself - without this crate having to make
//! any claim about it.
//!
//! The shipped definitions are `include_str!`d rather than read from disk. The
//! Python SDK ships the same files as package data and needs a CI job that
//! installs the wheel to prove they arrived in it; here the linker proves it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::error::{Error, Result};

/// The definition used when a caller names none.
pub const DEFAULT_PROVIDER: &str = "nodemaven";

/// The six characters treated as whitespace in a parameter value.
///
/// Spelled out rather than delegated to `char::is_whitespace` or
/// `u8::is_ascii_whitespace`, because neither means the same thing in four
/// languages: `is_whitespace` is Unicode-wide, and Rust's
/// `is_ascii_whitespace` excludes the vertical tab while Python's
/// `str.isspace` includes it. A value carrying any of these is refused, so the
/// set is part of the contract and has to be a list somebody can copy.
pub const ASCII_WHITESPACE: [char; 6] = [' ', '\t', '\n', '\r', '\u{0b}', '\u{0c}'];

const SHIPPED: &[(&str, &str)] = &[("nodemaven", include_str!("data/providers/nodemaven.toml"))];

/// One gateway's username dialect.
///
/// [`Provider::status`] is load-bearing and not documentation. `measured` means
/// traffic has actually gone through this gateway and the dialect was read off
/// the wire. `documented` means it was transcribed from a vendor's own
/// documentation on [`Provider::source_read`] and has never been exercised. The
/// distinction matters because a wrong username is invisible: at least one
/// gateway answers an unrecognised parameter name with 200 and the setting
/// silently dropped, so the connection succeeds on settings that were never
/// applied and nothing the gateway replies can tell you.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    id: String,
    label: String,
    known_params: BTreeSet<String>,
    status: String,
    prefix: String,
    separator: String,
    pair_separator: String,
    session_param: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    aliases: BTreeMap<String, String>,
    values: BTreeMap<String, Vec<String>>,
    normalize: BTreeSet<String>,
    connect_reactions: BTreeMap<String, String>,
    exit_ip_header: Option<String>,
    source: String,
    source_read: String,
    notes: String,
}

impl Provider {
    /// Start describing a gateway this crate ships no definition for.
    ///
    /// An empty `known_params` is not a stub. It says nobody has established
    /// what this gateway recognises, so every parameter is refused rather than
    /// sent to be silently dropped.
    pub fn builder(id: impl Into<String>, label: impl Into<String>) -> ProviderBuilder {
        ProviderBuilder(Provider {
            id: id.into(),
            label: label.into(),
            known_params: BTreeSet::new(),
            status: "documented".to_string(),
            prefix: "{login}".to_string(),
            separator: "-".to_string(),
            pair_separator: "-".to_string(),
            session_param: None,
            host: None,
            port: None,
            aliases: BTreeMap::new(),
            values: BTreeMap::new(),
            normalize: BTreeSet::new(),
            connect_reactions: BTreeMap::new(),
            exit_ip_header: None,
            source: String::new(),
            source_read: String::new(),
            notes: String::new(),
        })
    }

    /// The id credentials are looked up under, upper-cased, in the environment.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The gateway's name, as it appears in every error message.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The parameter names this gateway is confirmed to recognise.
    pub fn known_params(&self) -> &BTreeSet<String> {
        &self.known_params
    }

    /// `measured` or `documented`; see the type-level note.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// The username template, with `{login}` substituted.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// What goes between one parameter pair and the next.
    pub fn separator(&self) -> &str {
        &self.separator
    }

    /// What goes between a parameter name and its value.
    pub fn pair_separator(&self) -> &str {
        &self.pair_separator
    }

    /// The parameter this gateway pins a sticky session with, if it has one.
    pub fn session_param(&self) -> Option<&str> {
        self.session_param.as_deref()
    }

    /// The gateway host, when the definition carries a default.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// The gateway port, when the definition carries a default.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// The header the exit address arrives on, when the gateway sends one.
    pub fn exit_ip_header(&self) -> Option<&str> {
        self.exit_ip_header.as_deref()
    }

    /// Where the dialect was read from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The date [`Provider::source`] was read.
    pub fn source_read(&self) -> &str {
        &self.source_read
    }

    /// What the gateway does with bad input, in prose.
    pub fn notes(&self) -> &str {
        &self.notes
    }

    /// The name this gateway uses on the wire for a canonical parameter.
    pub fn spell<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).map_or(name, String::as_str)
    }

    /// What a CONNECT status means **on this gateway**, or `None` if unrecorded.
    ///
    /// This is dialect and not HTTP. The shipped gateway answers a bad `filter`
    /// value with 407 Proxy Authentication Required, which is a lie about the
    /// cause in the most expensive direction available: it sends the caller to
    /// check credentials that are correct. A status code alone is therefore not
    /// a diagnosis here, and the translation is per-gateway, so it lives in the
    /// TOML beside the separators rather than in a `match` in this module.
    ///
    /// Takes a `u16` and looks up a `String`, because the keys in the file are
    /// strings for a reason that is not about Rust: TOML has no integer keys and
    /// neither does JSON, and the golden vectors are JSON. So the conversion
    /// happens here rather than in the schema, in all four languages.
    pub fn reaction(&self, status: u16) -> Option<&str> {
        self.connect_reactions
            .get(&status.to_string())
            .map(String::as_str)
    }

    /// The whole reaction table, keyed by status as it is written in the file.
    ///
    /// [`Provider::reaction`] is the lookup and this is the table, and both are
    /// public because [`crate::Connect`] needs the table rather than one entry.
    /// The reason is worth stating where it can be read from either side: the
    /// meaning depends on the status, and the status is not known until the
    /// gateway answers, so a caller who resolved one here and passed it along
    /// would always be passing `None`.
    pub fn connect_reactions(&self) -> &BTreeMap<String, String> {
        &self.connect_reactions
    }

    /// Whether this gateway wants the value of `name` folded to a wire form.
    ///
    /// The parameters that say yes carry a human place name - a region, a city,
    /// an ISP - and the gateway wants `district_of_columbia` where a caller
    /// naturally writes `District of Columbia`. See [`Provider::normalized`] for
    /// what the fold does.
    pub fn normalizes(&self, name: &str) -> bool {
        self.normalize.contains(name)
    }

    /// The wire form of a value, or the value unchanged if `name` is not folded.
    ///
    /// The fold is three steps, in this order, and **it is part of the
    /// cross-language contract** - a golden vector pins the username a set of
    /// parameters produces, so four SDKs have to agree on it character for
    /// character:
    ///
    /// 1. strip leading and trailing [`ASCII_WHITESPACE`]
    /// 2. lower-case ASCII `A-Z` only
    /// 3. replace each space with `_`
    ///
    /// ASCII and not Unicode at every step, deliberately. `to_lowercase` differs
    /// between languages and locales on characters like the Turkish dotted capital I (U+0130), and a vector that
    /// depends on which is a vector that fails in one language for reasons that
    /// have nothing to do with proxies. Everything in the vendor's own location
    /// catalogue is ASCII.
    pub fn normalized(&self, name: &str, value: &str) -> String {
        if !self.normalizes(name) {
            return value.to_string();
        }
        value
            .trim_matches(|character| ASCII_WHITESPACE.contains(&character))
            .to_ascii_lowercase()
            .replace(' ', "_")
    }

    /// The legal values for a parameter, or `None` if they are not known.
    ///
    /// `None` and an empty slice are deliberately different answers. `None`
    /// means nobody has established what this gateway accepts, so the value is
    /// passed through unchecked and the caller is no worse off than before.
    /// Filling this in is a change to a data file and to nothing else, which is
    /// the entire reason the key exists before there is anything to put in it:
    /// four language SDKs read this schema, and adding a key to it later is four
    /// parsers, while adding data to a key they already read is one file.
    pub fn allowed(&self, name: &str) -> Option<&[String]> {
        self.values.get(name).map(Vec::as_slice)
    }

    /// Whether traffic has actually gone through this gateway.
    ///
    /// `false` covers both "transcribed from documentation" and any status
    /// nobody has defined, which is the safe way round: a definition is
    /// unverified until something says otherwise.
    pub fn is_measured(&self) -> bool {
        self.status == "measured"
    }

    pub(crate) fn render_prefix(&self, login: &str) -> String {
        self.prefix.replace("{login}", login)
    }
}

/// Builds a [`Provider`] by hand, for a gateway with no shipped definition.
///
/// [`ProviderBuilder::build`] runs the same consistency checks a definition read
/// from a file goes through, so a hand-built provider cannot be inconsistent in
/// a way a file could not be.
pub struct ProviderBuilder(Provider);

impl ProviderBuilder {
    /// Declare the parameter names the gateway recognises.
    pub fn known_params<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.0.known_params = names.into_iter().map(Into::into).collect();
        self
    }

    /// `measured` if traffic has gone through this gateway, `documented` if not.
    pub fn status(mut self, status: impl Into<String>) -> Self {
        self.0.status = status.into();
        self
    }

    /// The username template. `{login}` is substituted; the rest is literal.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.0.prefix = prefix.into();
        self
    }

    /// What goes between one parameter pair and the next.
    pub fn separator(mut self, separator: impl Into<String>) -> Self {
        self.0.separator = separator.into();
        self
    }

    /// What goes between a parameter name and its value.
    pub fn pair_separator(mut self, separator: impl Into<String>) -> Self {
        self.0.pair_separator = separator.into();
        self
    }

    /// The parameter a sticky session is pinned with.
    pub fn session_param(mut self, name: impl Into<String>) -> Self {
        self.0.session_param = Some(name.into());
        self
    }

    /// The default gateway host.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.0.host = Some(host.into());
        self
    }

    /// The default gateway port.
    pub fn port(mut self, port: u16) -> Self {
        self.0.port = Some(port);
        self
    }

    /// Spell a canonical parameter name differently on the wire.
    pub fn alias(mut self, canonical: impl Into<String>, on_the_wire: impl Into<String>) -> Self {
        self.0.aliases.insert(canonical.into(), on_the_wire.into());
        self
    }

    /// The parameters folded to their wire form before validation.
    ///
    /// See [`Provider::normalized`] for the fold, which is three fixed steps and
    /// part of the cross-language contract.
    pub fn normalize<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.0.normalize = names.into_iter().map(Into::into).collect();
        self
    }

    /// The closed list of values a parameter accepts.
    ///
    /// Lists only, never a pattern - see the note in the TOML parser below.
    pub fn allowed_values<I, S>(mut self, name: impl Into<String>, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.0
            .values
            .insert(name.into(), values.into_iter().map(Into::into).collect());
        self
    }

    /// What one CONNECT status means on this gateway, in prose.
    ///
    /// One call per status. There is no bulk setter because there is nothing to
    /// bulk-set from: each entry is a sentence somebody wrote after watching the
    /// gateway do it, so they arrive one at a time or not at all.
    pub fn connect_reaction(mut self, status: u16, meaning: impl Into<String>) -> Self {
        self.0
            .connect_reactions
            .insert(status.to_string(), meaning.into());
        self
    }

    /// The header the exit address arrives on.
    pub fn exit_ip_header(mut self, header: impl Into<String>) -> Self {
        self.0.exit_ip_header = Some(header.into());
        self
    }

    /// Where the dialect was read from, and when.
    pub fn source(mut self, source: impl Into<String>, read_on: impl Into<String>) -> Self {
        self.0.source = source.into();
        self.0.source_read = read_on.into();
        self
    }

    /// What the gateway does with bad input, in prose.
    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        self.0.notes = notes.into();
        self
    }

    /// Check the definition is internally consistent and hand it over.
    pub fn build(mut self) -> Result<Provider> {
        fold_values(&mut self.0);
        check(&self.0, "this definition")?;
        Ok(self.0)
    }
}

/// Ids of the definitions compiled into this crate.
pub fn available() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = SHIPPED.iter().map(|(id, _)| *id).collect();
    ids.sort_unstable();
    ids
}

/// A definition shipped with this crate.
pub fn load(provider_id: &str) -> Result<Provider> {
    let Some((_, text)) = SHIPPED.iter().find(|(id, _)| *id == provider_id) else {
        return Err(Error::Provider(format!(
            "no provider definition {provider_id:?} is shipped here, so no username \
             can be built for it. Shipped: {:?}. To use a gateway that is not in \
             that list, write a .toml for it and pass it to load_file().",
            available()
        )));
    };
    parse(
        text,
        provider_id,
        &format!("the shipped {provider_id} definition"),
    )
}

/// A definition read from an arbitrary path.
///
/// This is the seam that lets the crate address a gateway it ships no definition
/// for. Nothing about it is special-cased: a file loaded from disk goes through
/// the same checks and the same builder as a shipped one.
///
/// The id comes from the file stem, because that is what credentials are looked
/// up under in the environment: `my-gateway.toml` reads `MY_GATEWAY_LOGIN`. Use
/// [`load_file_as`] to say the id outright.
pub fn load_file(path: impl AsRef<Path>) -> Result<Provider> {
    let path = path.as_ref();
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            Error::Provider(format!(
                "cannot read a provider id out of the path {}, and the id is what \
                 credentials are looked up under in the environment. Rename the file \
                 or pass the id to load_file_as().",
                path.display()
            ))
        })?
        .to_string();
    load_file_as(path, &id)
}

/// A definition read from an arbitrary path, under an id you choose.
pub fn load_file_as(path: impl AsRef<Path>, provider_id: &str) -> Result<Provider> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|error| {
        Error::Provider(format!(
            "cannot read the provider definition {}: {error}",
            path.display()
        ))
    })?;
    parse(&text, provider_id, &path.display().to_string())
}

/// A definition read from TOML already in memory.
pub fn load_str(text: &str, provider_id: &str) -> Result<Provider> {
    parse(text, provider_id, "the definition passed in")
}

fn parse(text: &str, provider_id: &str, origin: &str) -> Result<Provider> {
    let raw: toml::Table = text
        .parse()
        .map_err(|error| Error::Provider(format!("{origin} is not valid TOML: {error}")))?;

    let missing: Vec<&str> = ["label", "known_params"]
        .into_iter()
        .filter(|key| !raw.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        return Err(Error::Provider(format!(
            "{origin} is missing {missing:?}, so it does not describe a gateway. \
             Without known_params nothing can be validated, and an unknown parameter \
             name is the one mistake a gateway does not report."
        )));
    }

    let mut provider = Provider::builder(provider_id, string(&raw, "label", origin)?).0;
    provider.known_params = string_list(&raw, "known_params", origin)?
        .into_iter()
        .collect();

    for (key, field) in [
        ("status", &mut provider.status),
        ("prefix", &mut provider.prefix),
        ("separator", &mut provider.separator),
        ("pair_separator", &mut provider.pair_separator),
        ("source", &mut provider.source),
        ("source_read", &mut provider.source_read),
        ("notes", &mut provider.notes),
    ] {
        if raw.contains_key(key) {
            *field = string(&raw, key, origin)?;
        }
    }

    for (key, field) in [
        ("session_param", &mut provider.session_param),
        ("host", &mut provider.host),
        ("exit_ip_header", &mut provider.exit_ip_header),
    ] {
        if raw.contains_key(key) {
            let value = string(&raw, key, origin)?;
            *field = (!value.is_empty()).then_some(value);
        }
    }

    // Through `crate::check::port_number`, the same rule `ProxyBuilder::build`
    // and the environment path use, and not through `u16::try_from`.
    //
    // `try_from` refuses a negative and refuses 65536 and accepts **0**, which
    // is the one value that reads as a port and is not one. A definition
    // carrying `port = 0` used to load here and then die much later against the
    // zero-check in `proxy.rs`, whose sentence is written for a caller who
    // passed `port(0)`: it said "The gateway's own port is 0" and told them to
    // pass `port()`, which is the one thing they had not done. An error that
    // names the wrong file is worse than no error, because it sends the reader
    // to edit code that is correct.
    //
    // Found 2026-09-11 in an external review of this crate. The same defect was
    // fixed in the Python SDK on 2026-09-09 by routing the definition's port
    // through `check._port_number`, and the fix never crossed over - which is
    // the review's actual finding and is worth more than either bug.
    if let Some(port) = raw.get("port") {
        let number = port.as_integer().ok_or_else(|| {
            Error::Provider(format!(
                "{origin} gives port as {port:?}, which is not a number."
            ))
        })?;
        let checked = crate::check::port_number(&number.to_string()).ok_or_else(|| {
            Error::Provider(format!(
                "{origin} gives port as {number}. It has to be a whole number from \
                 1 to 65535; 0 means \"any free port\" when binding and is meaningless \
                 when connecting."
            ))
        })?;
        provider.port = Some(checked);
    }

    if let Some(aliases) = raw.get("aliases") {
        let table = aliases.as_table().ok_or_else(|| {
            Error::Provider(format!(
                "{origin} gives aliases as {aliases:?}, which is not a table."
            ))
        })?;
        for (canonical, on_the_wire) in table {
            let spelled = on_the_wire.as_str().ok_or_else(|| {
                Error::Provider(format!(
                    "{origin} aliases {canonical:?} to {on_the_wire:?}, which is not a string."
                ))
            })?;
            provider
                .aliases
                .insert(canonical.clone(), spelled.to_string());
        }
    }

    // Which parameters are folded to their wire form before anything else looks
    // at them. Data and not code, for the same reason known_params is: the
    // public API must never name a gateway's parameters in its own signature,
    // and the next gateway will fold a different set or none at all.
    //
    // One list and one rule, deliberately. The vendor's own client applies two -
    // lower-case for `country` and `type`, lower-case plus space-to-underscore
    // for `region`, `city` and `isp` - and the difference cannot be observed,
    // because a country code and a pool name have no spaces in them to convert.
    // One rule that four languages have to agree on beats two.
    if raw.contains_key("normalize") {
        provider.normalize = string_list(&raw, "normalize", origin)?
            .into_iter()
            .collect();
    }

    // Legal values, when anybody has established them. A closed list and
    // nothing else: no regular expressions. A pattern in this file would have to
    // mean the same thing to Python, Go, Rust and JavaScript, and their engines
    // disagree on enough of the syntax that the contract would be about the
    // regex dialect rather than about the gateway. A list of strings is the same
    // in every language there is.
    if let Some(values) = raw.get("values") {
        let table = values.as_table().ok_or_else(|| {
            Error::Provider(format!(
                "{origin} gives values as {values:?}, which is not a table."
            ))
        })?;
        for (name, allowed) in table {
            let list = allowed
                .as_array()
                .filter(|items| !items.is_empty())
                .ok_or_else(|| {
                    Error::Provider(format!(
                        "{origin} gives the legal values of {name:?} as {allowed:?}. It has \
                         to be a non-empty list of strings. Leave the entry out entirely to \
                         mean 'nobody has established these' - an empty list would mean \
                         'every value is refused', which is not a thing anybody wants to say."
                    ))
                })?;
            let mut legal = Vec::with_capacity(list.len());
            for item in list {
                let text = item.as_str().ok_or_else(|| {
                    Error::Provider(format!(
                        "{origin} lists {item:?} as a legal value of {name:?}, and it is \
                         not a string."
                    ))
                })?;
                legal.push(text.to_string());
            }
            provider.values.insert(name.clone(), legal);
        }
    }

    fold_values(&mut provider);

    // What each CONNECT status means on this gateway. Keys are the status code
    // as a string, because TOML has no integer keys and JSON has none either -
    // and the golden vectors are JSON, so a schema that used integers here would
    // already have to be stringified to be shared.
    //
    // Any status may be described and none has to be: an entry is a sentence a
    // human wrote after watching the gateway do it, so a gateway nobody has
    // probed simply has none and `check()` reports the bare status. There is no
    // validation to do beyond "it is a table of strings" - unlike `values`, an
    // entry here cannot refuse anything, so a wrong one is a misleading sentence
    // and not a blocked request.
    //
    // The key is not parsed into a number on the way in, on purpose. A file
    // describing a status this crate's `u16` could not hold is a file describing
    // something that is not an HTTP status, and refusing it at load would turn a
    // dead entry - which `reaction()` simply never finds - into a definition
    // that will not load at all. The narrow type is enforced where a status
    // comes off the wire, in `check.rs`, and not here.
    if let Some(reactions) = raw.get("connect_reactions") {
        let table = reactions.as_table().ok_or_else(|| {
            Error::Provider(format!(
                "{origin} gives connect_reactions as {reactions:?}, which is not a table."
            ))
        })?;
        for (status, meaning) in table {
            let text = meaning
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    Error::Provider(format!(
                        "{origin} describes CONNECT status {status:?} as {meaning:?}. It has \
                         to be a non-empty string: this text is shown to a caller as the \
                         reason their connection was refused."
                    ))
                })?;
            provider
                .connect_reactions
                .insert(status.clone(), text.to_string());
        }
    }

    check(&provider, origin)?;
    Ok(provider)
}

/// Fold every legal-values list belonging to a parameter that is folded.
///
/// Without this a definition refuses every value it declares legal. The
/// caller's value is folded in [`ProxyBuilder::param`] before anything looks at
/// it, and the list was stored as it was typed, so `values = { country = ["US",
/// "DE"] }` under `normalize = ["country"]` refuses `"US"` and `"us"` alike -
/// and the message blames the caller for a value the definition went to the
/// trouble of declaring legal.
///
/// Run on both paths that produce a `Provider`, and after the whole definition
/// is in hand rather than as each key arrives. The TOML parser reads
/// `normalize` before `values` and would not need the ordering, but
/// [`ProviderBuilder`]'s setters can be called either way round, and a fold at
/// [`ProviderBuilder::allowed_values`] would quietly depend on
/// [`ProviderBuilder::normalize`] having been called first. A rule that holds
/// only for one call order is the kind of thing this crate exists to not ship.
///
/// Folding the stored list rather than folding at the comparison in
/// `proxy.rs` is deliberate: there is then one folded form in the object, so
/// the refusal message quotes the strings the check actually compared against
/// instead of a prettier set nobody tested.
///
/// Found 2026-09-11 in an external review of this crate, latent at the time
/// because the shipped definition has `values = {}` - but `values` is the
/// documented extension path and [`ProviderBuilder::allowed_values`] is public
/// API, so it was reachable by anyone writing their own definition. The Python
/// SDK had the identical defect and fixed it on 2026-09-09; the fix never
/// crossed over, which is the review's real finding.
///
/// The crate's own tests did not catch it and the reason is worth keeping. Both
/// halves were covered and never together: the `legal_values` module builds a
/// definition with `values` and no `normalize`, and the fold module exercises
/// `normalize` against the shipped definition, whose `values` is empty. Each
/// test silently held the other feature at the value where the defect cannot
/// appear. It did not slip past the tests, it slipped between them.
fn fold_values(provider: &mut Provider) {
    let folded: Vec<(String, Vec<String>)> = provider
        .values
        .iter()
        .filter(|(name, _)| provider.normalizes(name))
        .map(|(name, legal)| {
            let legal = legal
                .iter()
                .map(|value| provider.normalized(name, value))
                .collect();
            (name.clone(), legal)
        })
        .collect();
    for (name, legal) in folded {
        provider.values.insert(name, legal);
    }
}

/// The four ways a definition can be internally inconsistent.
///
/// Each one describes a declaration that reads like a working setting and can
/// never fire, which is the class of mistake this crate exists to make loud.
fn check(provider: &Provider, origin: &str) -> Result<()> {
    let unknown_alias: Vec<&str> = provider
        .aliases
        .keys()
        .filter(|name| !provider.known_params.contains(*name))
        .map(String::as_str)
        .collect();
    if !unknown_alias.is_empty() {
        return Err(Error::Provider(format!(
            "{origin} aliases {unknown_alias:?} which are not in known_params, so those \
             parameters would be refused before the alias was ever used."
        )));
    }

    if let Some(session_param) = provider.session_param() {
        if !provider.known_params.contains(session_param) {
            return Err(Error::Provider(format!(
                "{origin} declares session_param {session_param:?} which is not in \
                 known_params, so every sticky session would be refused."
            )));
        }
    }

    for name in provider.values.keys() {
        if !provider.known_params.contains(name) {
            return Err(Error::Provider(format!(
                "{origin} lists legal values for {name:?}, which is not in known_params, \
                 so that parameter is refused by name before its value is ever looked at."
            )));
        }
    }

    let unknown_normalize: Vec<&str> = provider
        .normalize
        .iter()
        .filter(|name| !provider.known_params.contains(*name))
        .map(String::as_str)
        .collect();
    if !unknown_normalize.is_empty() {
        return Err(Error::Provider(format!(
            "{origin} normalizes {unknown_normalize:?} which are not in known_params, so \
             those parameters are refused by name and the fold can never run."
        )));
    }

    // A normalized value can never contain the separator, because the fold does
    // not introduce one and a value carrying it is refused either way. But a
    // separator of "_" would make the fold *produce* one - `city="New York"`
    // becoming `new_york` and then being cut in half - so the definition that
    // declares both is refused here rather than at the call site, where the
    // caller would be blamed for input that is correct.
    if !provider.normalize.is_empty() {
        for separator in [&provider.separator, &provider.pair_separator] {
            if separator == "_" {
                return Err(Error::Provider(format!(
                    "{origin} separates parameters with {separator:?} and also normalizes \
                     {:?}, and the fold turns a space into {separator:?}. A value with a \
                     space in it would be cut at the separator the fold had just \
                     inserted. Pick one.",
                    provider.normalize
                )));
            }
        }
    }

    Ok(())
}

fn string(raw: &toml::Table, key: &str, origin: &str) -> Result<String> {
    let value = &raw[key];
    value.as_str().map(str::to_string).ok_or_else(|| {
        Error::Provider(format!(
            "{origin} gives {key} as {value:?}, which is not a string."
        ))
    })
}

fn string_list(raw: &toml::Table, key: &str, origin: &str) -> Result<Vec<String>> {
    let value = &raw[key];
    let items = value.as_array().ok_or_else(|| {
        Error::Provider(format!(
            "{origin} gives {key} as {value:?}, which is not a list."
        ))
    })?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let text = item.as_str().ok_or_else(|| {
            Error::Provider(format!(
                "{origin} has {item:?} in {key}, and a parameter name has to be a string."
            ))
        })?;
        out.push(text.to_string());
    }
    Ok(out)
}
