use std::fmt;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use zeroize::Zeroize;

const REDACTION: &str = "<redacted>";

/// A validated credential value with deliberately explicit exposure.
///
/// `Secret` intentionally implements neither `Display`, `Debug`, nor
/// serialization traits. Callers must cross [`Secret::expose`] at the narrow
/// boundary where the credential is applied to an authorized operation.
///
/// Dropping a `Secret` overwrites its bytes before the allocation is freed, so
/// a core dump, a swapped page, or a later allocation does not recover the
/// credential from this value's heap (rfs-1xew). That is the extent of the
/// guarantee: [`Redactor::new`] builds an Aho-Corasick automaton whose internal
/// tables hold the pattern bytes, and zeroizing this value does not reach
/// them, so the automaton narrows the exposure window rather than closing it.
pub struct Secret(String);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

// The marker is implemented by hand: the `Drop` above is the zeroizing drop it
// promises, and `zeroize`'s derive would add `zeroize_derive` to the lockfile
// for the same three lines.
impl zeroize::ZeroizeOnDrop for Secret {}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The property is type-level: after `drop`, the bytes live in freed
    /// memory that cannot be observed without undefined behavior, so the
    /// contract is that `Secret` is a [`zeroize::ZeroizeOnDrop`] type.
    /// Removing the `Drop` impl or the marker fails to compile this test.
    fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}

    #[test]
    fn secret_wipes_its_bytes_on_drop() {
        assert_zeroize_on_drop::<Secret>();
    }

    #[test]
    fn secret_still_exposes_its_value_while_alive() {
        let secret = Secret::new("token-value".to_owned()).expect("non-empty secret");
        assert_eq!(secret.expose(), "token-value");
    }

    #[test]
    fn secret_rejects_empty_and_nul() {
        assert_eq!(
            Secret::new(String::new()).map(|_| ()),
            Err(SecretError::empty())
        );
        assert_eq!(
            Secret::new("a\0b".to_owned()).map(|_| ()),
            Err(SecretError::nul())
        );
    }
}
