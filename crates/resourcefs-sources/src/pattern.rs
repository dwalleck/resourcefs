//! Private compiled search and glob matchers.
//!
//! All raw PCRE2 ownership and return-code interpretation is confined here.

use std::{
    error::Error,
    fmt,
    ptr::{self, NonNull},
};

use globset::GlobBuilder;
use resourcefs_core::{
    ErrorCategory, MAX_DISCOVERY_PATTERN_BYTES, MAX_PATH_REFERENCE_BYTES, PathReference,
    ResourceError, SearchEngine, SearchRecord, SearchSourceResult,
};

const PCRE2_MATCH_LIMIT: u32 = 100_000;
const PCRE2_DEPTH_LIMIT: u32 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatternErrorKind {
    InvalidPattern,
    InputLimitExceeded,
    MatchLimitExceeded,
    DepthLimitExceeded,
    EngineFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PatternError {
    kind: PatternErrorKind,
    message: String,
}

impl PatternError {
    pub(crate) const fn kind(&self) -> PatternErrorKind {
        self.kind
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: PatternErrorKind::InvalidPattern,
            message: message.into(),
        }
    }

    fn input_limit(message: impl Into<String>) -> Self {
        Self {
            kind: PatternErrorKind::InputLimitExceeded,
            message: message.into(),
        }
    }

    fn engine(message: impl Into<String>) -> Self {
        Self {
            kind: PatternErrorKind::EngineFailure,
            message: message.into(),
        }
    }

    fn match_limit() -> Self {
        Self {
            kind: PatternErrorKind::MatchLimitExceeded,
            message: format!("PCRE2 match work exceeded {PCRE2_MATCH_LIMIT} steps"),
        }
    }

    fn depth_limit() -> Self {
        Self {
            kind: PatternErrorKind::DepthLimitExceeded,
            message: format!("PCRE2 match depth exceeded {PCRE2_DEPTH_LIMIT}"),
        }
    }
}

impl fmt::Display for PatternError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PatternError {}

impl From<PatternError> for ResourceError {
    fn from(error: PatternError) -> Self {
        let category = match error.kind() {
            PatternErrorKind::InvalidPattern => ErrorCategory::InvalidPattern,
            PatternErrorKind::InputLimitExceeded
            | PatternErrorKind::MatchLimitExceeded
            | PatternErrorKind::DepthLimitExceeded => ErrorCategory::LimitExceeded,
            PatternErrorKind::EngineFailure => ErrorCategory::SourceUnavailable,
        };
        ResourceError::new(category, error.message)
    }
}

#[derive(Debug)]
pub(crate) struct SearchMatcher {
    inner: SearchMatcherInner,
}

#[derive(Debug)]
enum SearchMatcherInner {
    Rust(regex::Regex),
    Pcre2(Pcre2Matcher),
}

impl SearchMatcher {
    pub(crate) fn compile(pattern: &str, case_sensitive: bool) -> Result<Self, PatternError> {
        validate_pattern(pattern, MAX_DISCOVERY_PATTERN_BYTES, "search")?;

        let rust_result = regex::RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .build();
        let inner = match rust_result {
            Ok(regex) => SearchMatcherInner::Rust(regex),
            Err(regex::Error::Syntax(_)) => {
                SearchMatcherInner::Pcre2(Pcre2Matcher::compile(pattern, case_sensitive)?)
            }
            Err(regex::Error::CompiledTooBig(limit)) => {
                return Err(PatternError::input_limit(format!(
                    "Rust regex compiled form exceeds its {limit}-byte work ceiling"
                )));
            }
            Err(error) => {
                return Err(PatternError::engine(format!(
                    "Rust regex compilation failed: {error}"
                )));
            }
        };
        Ok(Self { inner })
    }

    pub(crate) const fn engine(&self) -> SearchEngine {
        match self.inner {
            SearchMatcherInner::Rust(_) => SearchEngine::RustRegex,
            SearchMatcherInner::Pcre2(_) => SearchEngine::Pcre2,
        }
    }

    pub(crate) fn is_match(&mut self, subject: &str) -> Result<bool, PatternError> {
        match &mut self.inner {
            SearchMatcherInner::Rust(regex) => Ok(regex.is_match(subject)),
            SearchMatcherInner::Pcre2(regex) => regex.is_match(subject),
        }
    }

    #[cfg(test)]
    pub(crate) fn jit_size(&self) -> Result<usize, PatternError> {
        match &self.inner {
            SearchMatcherInner::Rust(_) => Ok(0),
            SearchMatcherInner::Pcre2(regex) => regex.jit_size(),
        }
    }
}

#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "standalone pattern contract includes this module without Source Adapters"
    )
)]
pub(crate) fn search_document(
    content: &str,
    canonical: &PathReference,
    pattern: &str,
    case_sensitive: bool,
) -> Result<SearchSourceResult, ResourceError> {
    let mut matcher = SearchMatcher::compile(pattern, case_sensitive)?;
    let engine = matcher.engine();
    let mut records = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if matcher.is_match(line)? {
            let line_number = u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| {
                    ResourceError::new(
                        ErrorCategory::LimitExceeded,
                        "search line number is not representable",
                    )
                })?;
            records.push(SearchRecord::new(canonical.clone(), line_number, line)?);
        }
    }
    Ok(SearchSourceResult::new(engine, records, Vec::new()))
}

#[derive(Debug)]
pub(crate) struct GlobMatcher {
    matcher: globset::GlobMatcher,
}

impl GlobMatcher {
    pub(crate) fn compile(pattern: &str, case_sensitive: bool) -> Result<Self, PatternError> {
        validate_pattern(pattern, MAX_PATH_REFERENCE_BYTES, "glob")?;
        let glob = GlobBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .literal_separator(true)
            .backslash_escape(true)
            .build()
            .map_err(|error| PatternError::invalid(format!("invalid glob pattern: {error}")))?;
        Ok(Self {
            matcher: glob.compile_matcher(),
        })
    }

    pub(crate) fn is_match(&self, path: &str) -> bool {
        self.matcher.is_match(path)
    }
}

fn validate_pattern(pattern: &str, maximum: usize, name: &str) -> Result<(), PatternError> {
    if pattern.is_empty() {
        return Err(PatternError::invalid(format!(
            "{name} pattern must not be empty"
        )));
    }
    if pattern.len() > maximum {
        return Err(PatternError::input_limit(format!(
            "{name} pattern exceeds the {maximum}-byte ceiling"
        )));
    }
    Ok(())
}

#[derive(Debug)]
struct Pcre2Matcher {
    code: Pcre2Code,
    context: Pcre2MatchContext,
    data: Pcre2MatchData,
}

impl Pcre2Matcher {
    fn compile(pattern: &str, case_sensitive: bool) -> Result<Self, PatternError> {
        let mut error_code = 0;
        let mut error_offset = 0;
        let mut options = pcre2_sys::PCRE2_UTF | pcre2_sys::PCRE2_UCP;
        if !case_sensitive {
            options |= pcre2_sys::PCRE2_CASELESS;
        }

        // SAFETY: `pattern` is valid for exactly `pattern.len()` bytes, both output pointers
        // are valid, and a null compile context requests PCRE2's default allocator.
        let code = unsafe {
            pcre2_sys::pcre2_compile_8(
                pattern.as_ptr(),
                pattern.len(),
                options,
                &mut error_code,
                &mut error_offset,
                ptr::null_mut(),
            )
        };
        let code = NonNull::new(code).ok_or_else(|| {
            PatternError::invalid(format!(
                "pattern rejected by Rust regex and PCRE2 (code {error_code} at byte {error_offset})"
            ))
        })?;
        let code = Pcre2Code(code);

        // SAFETY: a null general context requests PCRE2's default allocator.
        let context = unsafe { pcre2_sys::pcre2_match_context_create_8(ptr::null_mut()) };
        let context = Pcre2MatchContext(
            NonNull::new(context)
                .ok_or_else(|| PatternError::engine("PCRE2 match-context allocation failed"))?,
        );

        // SAFETY: `context` owns a live PCRE2 match context and both limits are valid u32s.
        let match_limit_status =
            unsafe { pcre2_sys::pcre2_set_match_limit_8(context.0.as_ptr(), PCRE2_MATCH_LIMIT) };
        if match_limit_status != 0 {
            return Err(PatternError::engine(format!(
                "PCRE2 rejected match limit with code {match_limit_status}"
            )));
        }
        // SAFETY: `context` owns a live PCRE2 match context and the depth limit is valid.
        let depth_limit_status =
            unsafe { pcre2_sys::pcre2_set_depth_limit_8(context.0.as_ptr(), PCRE2_DEPTH_LIMIT) };
        if depth_limit_status != 0 {
            return Err(PatternError::engine(format!(
                "PCRE2 rejected depth limit with code {depth_limit_status}"
            )));
        }

        // SAFETY: `code` owns a successfully compiled live pattern and a null general context
        // requests PCRE2's default allocator.
        let data = unsafe {
            pcre2_sys::pcre2_match_data_create_from_pattern_8(code.0.as_ptr(), ptr::null_mut())
        };
        let data = Pcre2MatchData(
            NonNull::new(data)
                .ok_or_else(|| PatternError::engine("PCRE2 match-data allocation failed"))?,
        );

        Ok(Self {
            code,
            context,
            data,
        })
    }

    fn is_match(&mut self, subject: &str) -> Result<bool, PatternError> {
        // SAFETY: every pointer is owned and live for this call; `subject` is valid for its
        // byte length; start offset zero is a UTF-8 boundary; and the reusable match data and
        // context are accessed exclusively through `&mut self`.
        let status = unsafe {
            pcre2_sys::pcre2_match_8(
                self.code.0.as_ptr(),
                subject.as_ptr(),
                subject.len(),
                0,
                0,
                self.data.0.as_ptr(),
                self.context.0.as_ptr(),
            )
        };
        match status {
            status if status >= 0 => Ok(true),
            pcre2_sys::PCRE2_ERROR_NOMATCH => Ok(false),
            pcre2_sys::PCRE2_ERROR_MATCHLIMIT => Err(PatternError::match_limit()),
            pcre2_sys::PCRE2_ERROR_DEPTHLIMIT => Err(PatternError::depth_limit()),
            other => Err(PatternError::engine(format!(
                "PCRE2 matching failed with code {other}"
            ))),
        }
    }

    #[cfg(test)]
    fn jit_size(&self) -> Result<usize, PatternError> {
        let mut size = 0_usize;
        // SAFETY: `code` owns a live pattern and `size` is writable with the type required by
        // PCRE2_INFO_JITSIZE.
        let status = unsafe {
            pcre2_sys::pcre2_pattern_info_8(
                self.code.0.as_ptr(),
                pcre2_sys::PCRE2_INFO_JITSIZE,
                ptr::from_mut(&mut size).cast::<std::ffi::c_void>(),
            )
        };
        if status == 0 {
            Ok(size)
        } else {
            Err(PatternError::engine(format!(
                "PCRE2 JIT-size query failed with code {status}"
            )))
        }
    }
}

#[derive(Debug)]
struct Pcre2Code(NonNull<pcre2_sys::pcre2_code_8>);

impl Drop for Pcre2Code {
    fn drop(&mut self) {
        // SAFETY: this pointer came from one successful `pcre2_compile_8` call and this RAII
        // owner drops it exactly once.
        unsafe { pcre2_sys::pcre2_code_free_8(self.0.as_ptr()) };
    }
}

#[derive(Debug)]
struct Pcre2MatchContext(NonNull<pcre2_sys::pcre2_match_context_8>);

impl Drop for Pcre2MatchContext {
    fn drop(&mut self) {
        // SAFETY: this pointer came from one successful context allocation and this RAII owner
        // drops it exactly once after all matcher calls have ended.
        unsafe { pcre2_sys::pcre2_match_context_free_8(self.0.as_ptr()) };
    }
}

#[derive(Debug)]
struct Pcre2MatchData(NonNull<pcre2_sys::pcre2_match_data_8>);

impl Drop for Pcre2MatchData {
    fn drop(&mut self) {
        // SAFETY: this pointer came from one successful match-data allocation and this RAII
        // owner drops it exactly once.
        unsafe { pcre2_sys::pcre2_match_data_free_8(self.0.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::SearchMatcher;

    #[test]
    fn pcre2_remains_interpreted() {
        let matcher = SearchMatcher::compile(r"(?<=x)y", true).expect("PCRE2 test matcher");
        assert_eq!(matcher.jit_size().expect("PCRE2 JIT-size query"), 0);
    }
}
