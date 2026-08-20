use resourcefs_core::{
    ErrorCategory, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS, MAX_TEXT_LINES, TextLimits,
};

#[test]
fn limits_presence_matrix_preserves_exact_values() {
    struct LimitsCase {
        bytes: Option<usize>,
        lines: Option<usize>,
        columns: Option<usize>,
        expected_bytes: usize,
        expected_lines: usize,
        expected_columns: usize,
    }

    let cases = [
        LimitsCase {
            bytes: None,
            lines: None,
            columns: None,
            expected_bytes: MAX_TEXT_BYTES,
            expected_lines: MAX_TEXT_LINES,
            expected_columns: MAX_TEXT_COLUMNS,
        },
        LimitsCase {
            bytes: Some(8_192),
            lines: None,
            columns: None,
            expected_bytes: 8_192,
            expected_lines: MAX_TEXT_LINES,
            expected_columns: MAX_TEXT_COLUMNS,
        },
        LimitsCase {
            bytes: None,
            lines: Some(10),
            columns: None,
            expected_bytes: MAX_TEXT_BYTES,
            expected_lines: 10,
            expected_columns: MAX_TEXT_COLUMNS,
        },
        LimitsCase {
            bytes: None,
            lines: None,
            columns: Some(64),
            expected_bytes: MAX_TEXT_BYTES,
            expected_lines: MAX_TEXT_LINES,
            expected_columns: 64,
        },
        LimitsCase {
            bytes: Some(8_192),
            lines: Some(10),
            columns: None,
            expected_bytes: 8_192,
            expected_lines: 10,
            expected_columns: MAX_TEXT_COLUMNS,
        },
        LimitsCase {
            bytes: Some(8_192),
            lines: None,
            columns: Some(64),
            expected_bytes: 8_192,
            expected_lines: MAX_TEXT_LINES,
            expected_columns: 64,
        },
        LimitsCase {
            bytes: None,
            lines: Some(10),
            columns: Some(64),
            expected_bytes: MAX_TEXT_BYTES,
            expected_lines: 10,
            expected_columns: 64,
        },
        LimitsCase {
            bytes: Some(MAX_TEXT_BYTES),
            lines: Some(MAX_TEXT_LINES),
            columns: Some(MAX_TEXT_COLUMNS),
            expected_bytes: MAX_TEXT_BYTES,
            expected_lines: MAX_TEXT_LINES,
            expected_columns: MAX_TEXT_COLUMNS,
        },
    ];
    for LimitsCase {
        bytes,
        lines,
        columns,
        expected_bytes,
        expected_lines,
        expected_columns,
    } in cases
    {
        let limits = TextLimits::new(bytes, lines, columns).expect("valid presence combination");
        assert_eq!(
            limits.bytes(),
            expected_bytes,
            "bytes for {bytes:?}/{lines:?}/{columns:?}"
        );
        assert_eq!(
            limits.lines(),
            expected_lines,
            "lines for {bytes:?}/{lines:?}/{columns:?}"
        );
        assert_eq!(
            limits.columns(),
            expected_columns,
            "columns for {bytes:?}/{lines:?}/{columns:?}"
        );
    }
    assert_eq!(
        TextLimits::default(),
        TextLimits::new(None, None, None).expect("all-absent limits"),
        "the all-absent combination must equal the binary hard preset"
    );
}

#[test]
fn zero_and_one_over_limits_fail_for_every_dimension() {
    let zero_cases = [
        (Some(0), None, None, "bytes"),
        (None, Some(0), None, "lines"),
        (None, None, Some(0), "columns"),
    ];
    for (bytes, lines, columns, dimension) in zero_cases {
        let error =
            TextLimits::new(bytes, lines, columns).expect_err(&format!("zero {dimension} limit"));
        assert_eq!(error.category(), ErrorCategory::LimitExceeded);
        assert!(
            error.message().contains("between 1 and"),
            "zero {dimension} message: {}",
            error.message()
        );
    }

    let one_over_cases = [
        (Some(MAX_TEXT_BYTES + 1), None, None, "bytes"),
        (None, Some(MAX_TEXT_LINES + 1), None, "lines"),
        (None, None, Some(MAX_TEXT_COLUMNS + 1), "columns"),
    ];
    for (bytes, lines, columns, dimension) in one_over_cases {
        let error = TextLimits::new(bytes, lines, columns)
            .expect_err(&format!("one-over {dimension} limit"));
        assert_eq!(error.category(), ErrorCategory::LimitExceeded);
        assert!(
            error.message().contains("between 1 and"),
            "one-over {dimension} message: {}",
            error.message()
        );
    }
}
