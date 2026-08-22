use std::fmt;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

const REDACTION: &str = "<redacted>";

/// A validated credential value with deliberately explicit exposure.
///
/// `Secret` intentionally implements neither `Display`, `Debug`, nor
/// serialization traits. Callers must cross [`Secret::expose`] at the narrow
/// boundary where the credential is applied to an authorized operation.
pub struct Secret(String);

impl Secret {
    /// Validates and owns one non-empty, NUL-free UTF-8 credential.
    pub fn new(value: String) -> Result<Self, SecretError> {
        if value.is_empty() {
            return Err(SecretError::empty());
        }
        if value.contains('\0') {
            return Err(SecretError::nul());
        }
        Ok(Self(value))
    }

    /// Exposes the credential for its authorized point of use.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// A value-free failure while validating a secret or building its redactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretError {
    kind: SecretErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretErrorKind {
    Empty,
    Nul,
    Redactor,
}

impl SecretError {
    const fn empty() -> Self {
        Self {
            kind: SecretErrorKind::Empty,
        }
    }

    const fn nul() -> Self {
        Self {
            kind: SecretErrorKind::Nul,
        }
    }

    const fn redactor() -> Self {
        Self {
            kind: SecretErrorKind::Redactor,
        }
    }
}

impl fmt::Display for SecretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            SecretErrorKind::Empty => "secret value must not be empty",
            SecretErrorKind::Nul => "secret value must not contain NUL",
            SecretErrorKind::Redactor => "secret redactor could not be constructed",
        })
    }
}

impl std::error::Error for SecretError {}

/// Replaces every registered credential occurrence before text reaches a sink.
pub struct Redactor {
    matcher: Option<AhoCorasick>,
}

impl Redactor {
    /// Builds one leftmost-longest matcher over distinct secret values.
    pub fn new<'a>(secrets: impl IntoIterator<Item = &'a Secret>) -> Result<Self, SecretError> {
        let mut patterns: Vec<&str> = secrets.into_iter().map(Secret::expose).collect();
        patterns.sort_unstable();
        patterns.dedup();

        if patterns.is_empty() {
            return Ok(Self { matcher: None });
        }

        let matcher = AhoCorasickBuilder::new()
            .match_kind(MatchKind::LeftmostLongest)
            .build(patterns)
            .map_err(|_| SecretError::redactor())?;
        Ok(Self {
            matcher: Some(matcher),
        })
    }

    /// Returns text with all registered credential occurrences replaced.
    #[must_use]
    pub fn scrub(&self, text: &str) -> String {
        let Some(matcher) = &self.matcher else {
            return text.to_owned();
        };

        let mut output = String::with_capacity(text.len());
        let mut copied_until = 0;
        for matched in matcher.find_iter(text) {
            output.push_str(&text[copied_until..matched.start()]);
            output.push_str(REDACTION);
            copied_until = matched.end();
        }
        output.push_str(&text[copied_until..]);
        output
    }
}
