#[path = "../src/pattern.rs"]
mod pattern;

use std::time::{Duration, Instant};

use pattern::{GlobMatcher, PatternErrorKind, SearchMatcher};
use resourcefs_core::{MAX_ARTIFACT_BYTES, MAX_DISCOVERY_PATTERN_BYTES, SearchEngine};

const CORPUS: &[&str] = &[
    "a.rs",
    "b.rs",
    "ab.rs",
    "main.rs",
    "a.txt",
    "b.txt",
    "c.txt",
    "ab.txt",
    "x1.txt",
    "x2.txt",
    "xy.txt",
    "literal*.txt",
    "src/main.rs",
    "src/lib.rs",
    "src/a.rs",
    "src/deep/leaf.rs",
    "src/deep/leaf.txt",
    "notes/2024/report.md",
    "notes/2024/01/report.md",
    "notes/todo.md",
    "data/a",
    "data/b",
    "data/ab",
    "data/a/b",
    "data/a/b/c",
    "data/ab/c",
    "README.md",
    "readme.md",
    "Cargo.toml",
    "cargo.toml",
    "img/logo.png",
    "img/logo.PNG",
    "img/logo.jpg",
    "src",
    "src/deep",
    "data",
    "notes",
    "img",
    "a/b",
];

struct GlobCase {
    name: &'static str,
    pattern: &'static str,
    case_sensitive: bool,
    expected: &'static [&'static str],
}

const GLOB_CASES: &[GlobCase] = &[
    GlobCase {
        name: "star_component",
        pattern: "*.rs",
        case_sensitive: true,
        expected: &["a.rs", "ab.rs", "b.rs", "main.rs"],
    },
    GlobCase {
        name: "star_nested_component",
        pattern: "src/*.rs",
        case_sensitive: true,
        expected: &["src/a.rs", "src/lib.rs", "src/main.rs"],
    },
    GlobCase {
        name: "qmark_single_char",
        pattern: "x?.txt",
        case_sensitive: true,
        expected: &["x1.txt", "x2.txt", "xy.txt"],
    },
    GlobCase {
        name: "qmark_component",
        pattern: "data/?",
        case_sensitive: true,
        expected: &["data/a", "data/b"],
    },
    GlobCase {
        name: "qmark_no_separator",
        pattern: "a?b",
        case_sensitive: true,
        expected: &[],
    },
    GlobCase {
        name: "class_single",
        pattern: "[abc].txt",
        case_sensitive: true,
        expected: &["a.txt", "b.txt", "c.txt"],
    },
    GlobCase {
        name: "class_negated",
        pattern: "[^a].txt",
        case_sensitive: true,
        expected: &["b.txt", "c.txt"],
    },
    GlobCase {
        name: "class_range",
        pattern: "x[0-9].txt",
        case_sensitive: true,
        expected: &["x1.txt", "x2.txt"],
    },
    GlobCase {
        name: "recursive_suffix",
        pattern: "src/**",
        case_sensitive: true,
        expected: &[
            "src/a.rs",
            "src/deep",
            "src/deep/leaf.rs",
            "src/deep/leaf.txt",
            "src/lib.rs",
            "src/main.rs",
        ],
    },
    GlobCase {
        name: "recursive_suffix_data",
        pattern: "data/**",
        case_sensitive: true,
        expected: &[
            "data/a",
            "data/a/b",
            "data/a/b/c",
            "data/ab",
            "data/ab/c",
            "data/b",
        ],
    },
    GlobCase {
        name: "recursive_prefix",
        pattern: "**/*.rs",
        case_sensitive: true,
        expected: &[
            "a.rs",
            "ab.rs",
            "b.rs",
            "main.rs",
            "src/a.rs",
            "src/deep/leaf.rs",
            "src/lib.rs",
            "src/main.rs",
        ],
    },
    GlobCase {
        name: "recursive_middle",
        pattern: "src/**/*.rs",
        case_sensitive: true,
        expected: &["src/a.rs", "src/deep/leaf.rs", "src/lib.rs", "src/main.rs"],
    },
    GlobCase {
        name: "recursive_middle_zero",
        pattern: "notes/**/report.md",
        case_sensitive: true,
        expected: &["notes/2024/01/report.md", "notes/2024/report.md"],
    },
    GlobCase {
        name: "alternation",
        pattern: "{a,b}.rs",
        case_sensitive: true,
        expected: &["a.rs", "b.rs"],
    },
    GlobCase {
        name: "alternation_components",
        pattern: "{src,data}/*",
        case_sensitive: true,
        expected: &[
            "data/a",
            "data/ab",
            "data/b",
            "src/a.rs",
            "src/deep",
            "src/lib.rs",
            "src/main.rs",
        ],
    },
    GlobCase {
        name: "separator_non_crossing_star",
        pattern: "data/*/c",
        case_sensitive: true,
        expected: &["data/ab/c"],
    },
    GlobCase {
        name: "separator_non_crossing_qmark",
        pattern: "data/?/b/c",
        case_sensitive: true,
        expected: &["data/a/b/c"],
    },
    GlobCase {
        name: "escaped_star",
        pattern: r"literal\*.txt",
        case_sensitive: true,
        expected: &["literal*.txt"],
    },
    GlobCase {
        name: "case_insensitive_literal",
        pattern: "readme.md",
        case_sensitive: false,
        expected: &["README.md", "readme.md"],
    },
    GlobCase {
        name: "case_insensitive_star",
        pattern: "img/*.png",
        case_sensitive: false,
        expected: &["img/logo.PNG", "img/logo.png"],
    },
];

fn match_limit_subject() -> String {
    "x".to_owned() + &"a".repeat(32) + "b"
}

fn depth_limit_subject() -> String {
    "a".repeat(2_000) + &"b".repeat(2_000)
}

#[test]
fn rust_first_pcre2_fallback_and_limits() {
    let mut rust = SearchMatcher::compile("foo", true).expect("C1 Rust pattern");
    assert_eq!(rust.engine(), SearchEngine::RustRegex, "C1 Rust selection");
    assert!(rust.is_match("before foo after").expect("C1 Rust match"));

    let mut pcre = SearchMatcher::compile(r"(?<=foo)bar", true).expect("C1 PCRE pattern");
    assert_eq!(pcre.engine(), SearchEngine::Pcre2, "C1 PCRE fallback");
    assert!(pcre.is_match("foobar").expect("C1 PCRE lookbehind"));
    assert_eq!(
        pcre.jit_size().expect("C1 JIT query"),
        0,
        "C1 JIT forbidden"
    );

    let mut unicode =
        SearchMatcher::compile(r"(?<=x)CAFÉ", false).expect("C1 Unicode PCRE pattern");
    assert!(unicode.is_match("xcafé").expect("C1 PCRE Unicode match"));
    let mut unicode_property =
        SearchMatcher::compile(r"(?<=x)\w", true).expect("C1 Unicode property pattern");
    assert!(
        unicode_property
            .is_match("xé")
            .expect("C1 PCRE Unicode property match"),
        "C1 PCRE UCP"
    );

    let invalid = SearchMatcher::compile(r"(?<=a+)b", true).expect_err("C1 invalid both");
    assert_eq!(
        invalid.kind(),
        PatternErrorKind::InvalidPattern,
        "C1 invalid kind"
    );

    let mut match_limited =
        SearchMatcher::compile(r"(?<=x)(a+)+$", true).expect("C1 match-limit pattern");
    let error = match_limited
        .is_match(&match_limit_subject())
        .expect_err("C1 match-limit exhaustion");
    assert_eq!(
        error.kind(),
        PatternErrorKind::MatchLimitExceeded,
        "C1 typed -47"
    );

    let mut depth_limited =
        SearchMatcher::compile(r"(a(?1)?b)", true).expect("C1 depth-limit pattern");
    let error = depth_limited
        .is_match(&depth_limit_subject())
        .expect_err("C1 depth-limit exhaustion");
    assert_eq!(
        error.kind(),
        PatternErrorKind::DepthLimitExceeded,
        "C1 typed -53"
    );
}

#[test]
fn glob_language_table() {
    let started = Instant::now();
    for case in GLOB_CASES {
        let matcher = GlobMatcher::compile(case.pattern, case.case_sensitive)
            .unwrap_or_else(|error| panic!("C1 {} compilation: {error}", case.name));
        let mut actual: Vec<_> = CORPUS
            .iter()
            .copied()
            .filter(|path| matcher.is_match(path))
            .collect();
        actual.sort_unstable();
        assert_eq!(actual, case.expected, "C1 glob case {}", case.name);
    }
    assert!(
        started.elapsed() <= Duration::from_millis(100),
        "C1 glob table exceeded 100 ms: {:?}",
        started.elapsed()
    );
}

#[test]
fn matcher_selection_unicode_and_work_limits() {
    let mut rust_unicode = SearchMatcher::compile("CAFÉ", false).expect("C3 Rust Unicode");
    assert_eq!(
        rust_unicode.engine(),
        SearchEngine::RustRegex,
        "C3 Rust first"
    );
    assert!(rust_unicode.is_match("café").expect("C3 Unicode case fold"));
    let compiled_too_big = SearchMatcher::compile(r"(?:a{1000}){1000}", true)
        .expect_err("C3 oversized Rust automaton");
    assert_eq!(
        compiled_too_big.kind(),
        PatternErrorKind::InputLimitExceeded,
        "C3 non-syntax Rust failure must not fall back"
    );

    let compile_started = Instant::now();
    let maximum_pattern = "a".repeat(MAX_DISCOVERY_PATTERN_BYTES);
    let maximum = SearchMatcher::compile(&maximum_pattern, true).expect("C3 maximum pattern");
    assert_eq!(
        maximum.engine(),
        SearchEngine::RustRegex,
        "C3 maximum engine"
    );
    assert!(
        compile_started.elapsed() <= Duration::from_secs(2),
        "C3 maximum compile exceeded two seconds: {:?}",
        compile_started.elapsed()
    );

    let mut pcre = SearchMatcher::compile(r"(?<=foo)bar", true).expect("C3 lookbehind");
    assert_eq!(pcre.engine(), SearchEngine::Pcre2, "C3 fallback engine");
    assert!(pcre.is_match("foobar").expect("C3 fallback match"));
    assert_eq!(pcre.jit_size().expect("C3 JIT query"), 0, "C3 no JIT");

    let mut match_limited =
        SearchMatcher::compile(r"(?<=x)(a+)+$", true).expect("C3 match-limit pattern");
    assert_eq!(
        match_limited
            .is_match(&match_limit_subject())
            .expect_err("C3 match-limit exhaustion")
            .kind(),
        PatternErrorKind::MatchLimitExceeded,
        "C3 match limit kind"
    );

    let mut depth_limited =
        SearchMatcher::compile(r"(a(?1)?b)", true).expect("C3 depth-limit pattern");
    assert_eq!(
        depth_limited
            .is_match(&depth_limit_subject())
            .expect_err("C3 depth-limit exhaustion")
            .kind(),
        PatternErrorKind::DepthLimitExceeded,
        "C3 depth limit kind"
    );

    let mut haystack = "x".repeat(MAX_ARTIFACT_BYTES);
    haystack.push_str("needle");
    let mut scan = SearchMatcher::compile("needle$", true).expect("C3 scan pattern");
    let scan_started = Instant::now();
    assert!(scan.is_match(&haystack).expect("C3 maximum scan"));
    assert!(
        scan_started.elapsed() <= Duration::from_secs(1),
        "C3 maximum scan exceeded one second: {:?}",
        scan_started.elapsed()
    );
}
