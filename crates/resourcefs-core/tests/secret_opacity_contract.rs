use resourcefs_core::{Redactor, Secret};

#[test]
fn secret_rejects_observable_traits_at_compile_time() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/secret_display.rs");
    cases.compile_fail("tests/ui/secret_debug.rs");
    cases.compile_fail("tests/ui/secret_serialize.rs");
}

fn secret(value: &str) -> Secret {
    match Secret::new(value.to_owned()) {
        Ok(secret) => secret,
        Err(error) => panic!("valid fixture secret was rejected: {error}"),
    }
}

#[test]
fn secret_validation_is_value_free() {
    for (value, expected) in [
        (String::new(), "secret value must not be empty"),
        (
            "private-sentinel\0suffix".to_owned(),
            "secret value must not contain NUL",
        ),
    ] {
        let error = match Secret::new(value) {
            Ok(_) => panic!("invalid secret was accepted"),
            Err(error) => error,
        };
        let rendered = error.to_string();
        assert_eq!(rendered, expected);
        assert!(!rendered.contains("private-sentinel"));
    }
}

#[test]
fn empty_redactor_preserves_text() {
    let redactor = Redactor::new(std::iter::empty())
        .unwrap_or_else(|error| panic!("empty redactor failed: {error}"));
    assert_eq!(redactor.scrub("ordinary text"), "ordinary text");
}

#[test]
fn redactor_deduplicates_and_prefers_longest_prefix_match() {
    let short = secret("abc");
    let long = secret("abcdef");
    let duplicate = secret("abcdef");
    let redactor = Redactor::new([&short, &long, &duplicate])
        .unwrap_or_else(|error| panic!("redactor failed: {error}"));

    assert_eq!(
        redactor.scrub("before abc between abcdef after"),
        "before <redacted> between <redacted> after"
    );
    assert!(!redactor.scrub("abcdef").contains("def"));
}

#[test]
fn redactor_accepts_one_thousand_twenty_four_distinct_secrets() {
    let secrets: Vec<Secret> = (0..1_024)
        .map(|index| secret(&format!("secret-value-{index:04}")))
        .collect();
    let redactor =
        Redactor::new(&secrets).unwrap_or_else(|error| panic!("large redactor failed: {error}"));

    assert_eq!(
        redactor.scrub("secret-value-0000/secret-value-1023"),
        "<redacted>/<redacted>"
    );
}
