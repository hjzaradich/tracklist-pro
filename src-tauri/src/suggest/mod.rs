//! Suggestions and their reasons (ROADMAP §1, "No black boxes"; §1.1,
//! Suggestions and AI / ML).
//!
//! Every suggestion carries at least one reason, and a reason is an i18n key
//! plus parameters, never English text: the frontend turns it into words from
//! the locale files. A reason that comes from the audio model is marked as
//! such, and can never be a suggestion's only reason, so switching the audio
//! model off always leaves a suggestion with a reason.
//!
//! How reason texts are worded (short fragments, facts not opinions, detail
//! in parentheses): `docs/copy-style.md`, "Suggestion reasons".
//!
//! These rules hold by construction: [`Suggestion`] has private fields, and
//! every way to make one ([`Suggestion::new`], deserializing) checks them.
//! Serializing checks them again, so no reasonless suggestion can reach the
//! UI. The frontend refuses to render one too (`src/suggest`).
//!
//! A suggestion can't be built with a struct literal:
//!
//! ```compile_fail
//! use tracklist_pro_lib::suggest::Suggestion;
//! let s = Suggestion { what: 1, reasons: Vec::new() };
//! ```
//!
//! Only through the checked constructor:
//!
//! ```
//! use tracklist_pro_lib::suggest::{Reason, Suggestion};
//! let s = Suggestion::new(1, vec![Reason::fact("crates:reason.sameKey").unwrap()]);
//! assert!(s.is_ok());
//! ```

use std::collections::BTreeMap;
use std::fmt;

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use specta::Type;

/// Why a suggestion or reason was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum SuggestError {
    /// A suggestion needs at least one reason.
    NoReason,
    /// The audio model's opinion is never a suggestion's only reason.
    OnlyAudioModel,
    /// A reason key must be an i18n key, `namespace:path.to.key`, not text.
    BadKey(String),
    /// A parameter name must be a plain identifier, e.g. `bpm`.
    BadParamName(String),
    /// A number parameter must be finite (JSON has no NaN or infinity).
    NotFinite(String),
}

impl fmt::Display for SuggestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SuggestError::NoReason => write!(f, "a suggestion needs at least one reason"),
            SuggestError::OnlyAudioModel => {
                write!(f, "the audio model can't be a suggestion's only reason")
            }
            SuggestError::BadKey(key) => write!(
                f,
                "reason key {key:?} is not an i18n key like `namespace:path.to.key`"
            ),
            SuggestError::BadParamName(name) => {
                write!(f, "reason parameter name {name:?} is not an identifier")
            }
            SuggestError::NotFinite(name) => {
                write!(f, "reason parameter {name:?} is not a finite number")
            }
        }
    }
}

impl std::error::Error for SuggestError {}

/// Where a reason comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReasonSource {
    /// A fact from the Library, rekordbox or the files: a key, a BPM, a tag.
    Fact,
    /// The audio model's opinion (3.8). Labeled as such in the UI, optional,
    /// and never the only reason.
    AudioModel,
}

/// A value filled into a reason's text, e.g. `bpm` = 2 or `tag` = "Peak Time".
///
/// Text parameters are data (a tag name, a title), never prose: the words
/// around them live in the locale file. Numbers stay numbers so the frontend
/// formats them for the user's language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(untagged)]
pub enum ReasonParam {
    Text(String),
    Number(f64),
}

/// One reason for a suggestion: an i18n key, its parameters and its source.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Reason {
    /// An i18n key with its namespace, e.g. `crates:reason.sameKey`.
    key: String,
    params: BTreeMap<String, ReasonParam>,
    source: ReasonSource,
}

impl Reason {
    /// A reason stating a fact. `key` is an i18n key such as
    /// `crates:reason.sameKey`; English text is refused.
    pub fn fact(key: &str) -> Result<Reason, SuggestError> {
        Reason::new(key, ReasonSource::Fact)
    }

    /// A reason that is the audio model's opinion. It's labeled as such in
    /// the UI, and a suggestion needs at least one other reason besides it.
    pub fn audio_model(key: &str) -> Result<Reason, SuggestError> {
        Reason::new(key, ReasonSource::AudioModel)
    }

    fn new(key: &str, source: ReasonSource) -> Result<Reason, SuggestError> {
        if !is_i18n_key(key) {
            return Err(SuggestError::BadKey(key.to_owned()));
        }
        Ok(Reason {
            key: key.to_owned(),
            params: BTreeMap::new(),
            source,
        })
    }

    /// Adds a text parameter, e.g. `tag` = "Peak Time".
    pub fn with_text(self, name: &str, value: impl Into<String>) -> Result<Reason, SuggestError> {
        self.with(name, ReasonParam::Text(value.into()))
    }

    /// Adds a number parameter, e.g. `bpm` = 2.
    pub fn with_number(self, name: &str, value: f64) -> Result<Reason, SuggestError> {
        if !value.is_finite() {
            return Err(SuggestError::NotFinite(name.to_owned()));
        }
        self.with(name, ReasonParam::Number(value))
    }

    fn with(mut self, name: &str, value: ReasonParam) -> Result<Reason, SuggestError> {
        if !is_identifier(name) {
            return Err(SuggestError::BadParamName(name.to_owned()));
        }
        self.params.insert(name.to_owned(), value);
        Ok(self)
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn params(&self) -> &BTreeMap<String, ReasonParam> {
        &self.params
    }

    pub fn source(&self) -> ReasonSource {
        self.source
    }

    /// Re-checks a reason that came from outside (deserialized).
    fn check(&self) -> Result<(), SuggestError> {
        if !is_i18n_key(&self.key) {
            return Err(SuggestError::BadKey(self.key.clone()));
        }
        for (name, value) in &self.params {
            if !is_identifier(name) {
                return Err(SuggestError::BadParamName(name.clone()));
            }
            if matches!(value, ReasonParam::Number(n) if !n.is_finite()) {
                return Err(SuggestError::NotFinite(name.clone()));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Reason {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            key: String,
            #[serde(default)]
            params: BTreeMap<String, ReasonParam>,
            source: ReasonSource,
        }
        let raw = Raw::deserialize(deserializer)?;
        let reason = Reason {
            key: raw.key,
            params: raw.params,
            source: raw.source,
        };
        reason.check().map_err(D::Error::custom)?;
        Ok(reason)
    }
}

/// Something the app suggests (`what`), with the reasons it's suggested.
///
/// Always has at least one reason, and at least one that isn't the audio
/// model's. See the module docs.
#[derive(Debug, Clone, PartialEq, Type)]
pub struct Suggestion<T> {
    what: T,
    reasons: Vec<Reason>,
}

impl<T> Suggestion<T> {
    /// A suggestion of `what`, for `reasons`. Refused without a reason, or
    /// when every reason is the audio model's.
    pub fn new(what: T, reasons: Vec<Reason>) -> Result<Suggestion<T>, SuggestError> {
        check_reasons(&reasons)?;
        Ok(Suggestion { what, reasons })
    }

    pub fn what(&self) -> &T {
        &self.what
    }

    pub fn reasons(&self) -> &[Reason] {
        &self.reasons
    }

    /// The same suggestion with the audio model's reasons dropped, for when
    /// the user has switched the audio model off. Always still has a reason.
    pub fn without_audio_model(mut self) -> Suggestion<T> {
        self.reasons
            .retain(|r| r.source != ReasonSource::AudioModel);
        debug_assert!(check_reasons(&self.reasons).is_ok());
        self
    }
}

/// The rules every suggestion's reasons follow. One function, used when a
/// suggestion is made, deserialized and serialized.
pub fn check_reasons(reasons: &[Reason]) -> Result<(), SuggestError> {
    if reasons.is_empty() {
        return Err(SuggestError::NoReason);
    }
    if reasons.iter().all(|r| r.source == ReasonSource::AudioModel) {
        return Err(SuggestError::OnlyAudioModel);
    }
    Ok(())
}

impl<T: Serialize> Serialize for Suggestion<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Out<'a, T> {
            what: &'a T,
            reasons: &'a [Reason],
        }
        // Can't fail for a suggestion made by `new`; checked anyway, so the
        // last step before the UI enforces the rule too.
        check_reasons(&self.reasons).map_err(S::Error::custom)?;
        Out {
            what: &self.what,
            reasons: &self.reasons,
        }
        .serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Suggestion<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw<T> {
            what: T,
            reasons: Vec<Reason>,
        }
        let raw = Raw::<T>::deserialize(deserializer)?;
        Suggestion::new(raw.what, raw.reasons).map_err(D::Error::custom)
    }
}

/// `namespace:path.to.key`: a namespace file name, then dot-separated
/// identifiers. No spaces, so English text can't pass for a key.
fn is_i18n_key(key: &str) -> bool {
    let Some((namespace, path)) = key.split_once(':') else {
        return false;
    };
    let namespace_ok = namespace.starts_with(|c: char| c.is_ascii_alphabetic())
        && namespace
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    namespace_ok && path.split('.').all(is_identifier)
}

/// A letter, then letters, digits or `_`.
fn is_identifier(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests;
