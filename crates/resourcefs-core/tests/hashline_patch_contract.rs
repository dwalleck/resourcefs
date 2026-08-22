use std::time::{Duration, Instant};

use resourcefs_core::{
    ErrorCategory, HashlinePatch, LineNumber, MAX_HASHLINE_PATCH_BYTES, OriginalLineRange,
    PatchOperationRef, PutTarget, VersionTag,
};

#[test]
fn line_types_reject_zero_and_descending_ranges() {
    let zero = LineNumber::new(0).expect_err("zero line");
    assert_eq!(zero.category(), ErrorCategory::InvalidPatch);
    let one = LineNumber::new(1).expect("line one");
    let two = LineNumber::new(2).expect("line two");
    let descending = OriginalLineRange::new(two, one).expect_err("descending range");
    assert_eq!(descending.category(), ErrorCategory::InvalidPatch);
}

fn document(body: &str) -> String {
    format!(
        "[rfs://workspace/workspace/fixture.txt#{}]\n{body}",
        VersionTag::from_content(b"fixture\n")
    )
}

#[test]
fn accepted_ranges_gaps_and_body_rows_parse_in_original_coordinates() {
    let patch = HashlinePatch::parse(&document(
        "PUT 2.=3:\n+replacement\n+\n+-literal-minus\n++literal-plus\nPUT <1:\n+before\nPUT >4:\n+after\nPUT >$:\n+tail\nCUT 8.=9",
    ))
    .expect("approved patch grammar");

    assert_eq!(
        patch.target().requested(),
        "rfs://workspace/workspace/fixture.txt"
    );
    assert_eq!(patch.operations().len(), 5);
    let PatchOperationRef::Put {
        target: PutTarget::Range(range),
        body,
    } = patch.operations()[0].kind()
    else {
        panic!("first operation must be range PUT");
    };
    assert_eq!((range.start().get(), range.end().get()), (2, 3));
    assert_eq!(body, ["replacement", "", "-literal-minus", "+literal-plus"]);
    assert!(matches!(
        patch.operations()[1].kind(),
        PatchOperationRef::Put {
            target: PutTarget::Before(line),
            ..
        } if line.get() == 1
    ));
    assert!(matches!(
        patch.operations()[2].kind(),
        PatchOperationRef::Put {
            target: PutTarget::After(line),
            ..
        } if line.get() == 4
    ));
    assert!(matches!(
        patch.operations()[3].kind(),
        PatchOperationRef::Put {
            target: PutTarget::Tail,
            ..
        }
    ));
}

#[test]
fn rem_and_mv_must_each_be_the_sole_operation() {
    let rem = HashlinePatch::parse(&document("REM")).expect("sole REM");
    assert!(matches!(
        rem.operations()[0].kind(),
        PatchOperationRef::Remove
    ));

    let mv =
        HashlinePatch::parse(&document("MV rfs://workspace/workspace/moved.txt")).expect("sole MV");
    assert!(matches!(
        mv.operations()[0].kind(),
        PatchOperationRef::Move { .. }
    ));

    for malformed in [
        "REM\nCUT 1.=1",
        "PUT <1:\n+x\nREM",
        "MV moved.txt\nCUT 1.=1",
    ] {
        let error = HashlinePatch::parse(&document(malformed)).expect_err("sole operation rule");
        assert_eq!(error.category(), ErrorCategory::InvalidPatch, "{malformed}");
    }
}

#[test]
fn reserved_block_and_register_forms_teach_the_accepted_grammar() {
    for reserved in [
        "PUT 1*:\n+x",
        "PUT >1*:\n+x",
        "CUT 1*",
        "CUT 1.=1 @body",
        "PUT <1 @body",
    ] {
        let error = HashlinePatch::parse(&document(reserved)).expect_err("reserved form");
        assert_eq!(error.category(), ErrorCategory::InvalidPatch, "{reserved}");
        assert!(error.message().contains("reserved"), "{error}");
        assert!(error.message().contains("PUT N.=M:"), "{error}");
        assert!(error.message().contains("CUT N.=M"), "{error}");
    }
}

#[test]
fn conflicting_original_coordinates_are_rejected() {
    for conflicting in [
        "CUT 2.=4\nPUT 4.=5:\n+x",
        "PUT >2:\n+x\nPUT <3:\n+y",
        "PUT >$:\n+x\nPUT >$:\n+y",
        "[rfs://workspace/workspace/other.txt#sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]\nCUT 1.=1",
    ] {
        let error = HashlinePatch::parse(&document(conflicting)).expect_err("conflicting patch");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidPatch,
            "{conflicting}"
        );
    }
}

#[test]
fn one_over_limit_fails_before_ast_growth() {
    let patch = "x".repeat(MAX_HASHLINE_PATCH_BYTES + 1);
    let error = HashlinePatch::parse(&patch).expect_err("one byte over patch ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
}

#[test]
fn exact_limit_parser_budget() {
    let header = format!(
        "[rfs://workspace/workspace/fixture.txt#{}]\nPUT >$:\n+",
        VersionTag::from_content(b"fixture\n")
    );
    let mut document = String::with_capacity(MAX_HASHLINE_PATCH_BYTES);
    document.push_str(&header);
    document.extend(std::iter::repeat_n(
        'x',
        MAX_HASHLINE_PATCH_BYTES - header.len(),
    ));
    assert_eq!(document.len(), MAX_HASHLINE_PATCH_BYTES);

    let started = Instant::now();
    let patch = HashlinePatch::parse(&document).expect("exact-limit patch");
    let elapsed = started.elapsed();

    assert_eq!(patch.operations().len(), 1);
    assert!(
        elapsed <= Duration::from_secs(5),
        "64 MiB patch parse took {elapsed:?}"
    );
}
#[test]
fn version_prefixes_are_canonical_and_at_least_twelve_hex_characters() {
    let prefix =
        HashlinePatch::parse("[rfs://workspace/workspace/fixture.txt#abcdef012345]\nCUT 1.=1")
            .expect("minimum prefix");
    assert_eq!(prefix.version().as_str(), "abcdef012345");

    for invalid in ["abcdef01234", "ABCDEF012345", "xyzxyzxyzxyz"] {
        let error = HashlinePatch::parse(&format!(
            "[rfs://workspace/workspace/fixture.txt#{invalid}]\nCUT 1.=1"
        ))
        .expect_err("invalid prefix");
        assert_eq!(error.category(), ErrorCategory::InvalidPatch, "{invalid}");
    }
}
