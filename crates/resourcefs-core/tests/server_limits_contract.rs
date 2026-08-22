use resourcefs_core::{
    DiscoveryLimitInput, ErrorCategory, MAX_ARTIFACT_BYTES, MAX_DISCOVERY_RESULTS, MAX_IMAGE_BYTES,
    MAX_SESSION_BYTES, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS, MAX_TEXT_LINES, ServerLimits,
    ServerLimitsInput, StorageLimitInput, TextLimitInput,
};

#[derive(Debug, Clone, Copy)]
enum Field {
    TextBytes,
    TextLines,
    TextColumns,
    SearchMatches,
    GlobEntries,
    ListingEntries,
    ImageBytes,
    ObjectBytes,
    SessionBytes,
}

impl Field {
    const ROWS: [(Self, &str, usize); 9] = [
        (Self::TextBytes, "text.bytes", MAX_TEXT_BYTES),
        (Self::TextLines, "text.lines", MAX_TEXT_LINES),
        (Self::TextColumns, "text.columns", MAX_TEXT_COLUMNS),
        (
            Self::SearchMatches,
            "discovery.searchMatches",
            MAX_DISCOVERY_RESULTS,
        ),
        (
            Self::GlobEntries,
            "discovery.globEntries",
            MAX_DISCOVERY_RESULTS,
        ),
        (
            Self::ListingEntries,
            "discovery.listingEntries",
            MAX_DISCOVERY_RESULTS,
        ),
        (Self::ImageBytes, "imageBytes", MAX_IMAGE_BYTES),
        (Self::ObjectBytes, "storage.objectBytes", MAX_ARTIFACT_BYTES),
        (
            Self::SessionBytes,
            "storage.sessionBytes",
            MAX_SESSION_BYTES,
        ),
    ];

    fn input(self, value: Option<usize>) -> ServerLimitsInput {
        let mut input = ServerLimitsInput::default();
        match self {
            Self::TextBytes => input.text.bytes = value,
            Self::TextLines => input.text.lines = value,
            Self::TextColumns => input.text.columns = value,
            Self::SearchMatches => input.discovery.search_matches = value,
            Self::GlobEntries => input.discovery.glob_entries = value,
            Self::ListingEntries => input.discovery.listing_entries = value,
            Self::ImageBytes => input.image_bytes = value,
            Self::ObjectBytes => input.storage.object_bytes = value,
            Self::SessionBytes => input.storage.session_bytes = value,
        }
        input
    }

    fn value(self, limits: ServerLimits) -> usize {
        match self {
            Self::TextBytes => limits.text_bytes(),
            Self::TextLines => limits.text_lines(),
            Self::TextColumns => limits.text_columns(),
            Self::SearchMatches => limits.search_matches(),
            Self::GlobEntries => limits.glob_entries(),
            Self::ListingEntries => limits.listing_entries(),
            Self::ImageBytes => limits.image_bytes(),
            Self::ObjectBytes => limits.object_bytes(),
            Self::SessionBytes => limits.session_bytes(),
        }
    }
}

#[test]
fn omitted_empty_and_exact_limits_match_literal_binary_ceilings() {
    let omitted = ServerLimits::new(ServerLimitsInput::default()).expect("omitted limits");
    let empty_groups = ServerLimits::new(ServerLimitsInput {
        text: TextLimitInput::default(),
        discovery: DiscoveryLimitInput::default(),
        image_bytes: None,
        storage: StorageLimitInput::default(),
    })
    .expect("empty groups");
    assert_eq!(omitted, ServerLimits::default());
    assert_eq!(empty_groups, omitted);

    for (field, name, maximum) in Field::ROWS {
        assert_eq!(field.value(omitted), maximum, "default {name}");
        let exact = ServerLimits::new(field.input(Some(maximum)))
            .unwrap_or_else(|error| panic!("exact {name} must succeed: {error}"));
        assert_eq!(field.value(exact), maximum, "exact {name}");
    }
}

#[test]
fn zero_and_one_over_each_literal_ceiling_fail_without_clamping() {
    for (field, name, maximum) in Field::ROWS {
        for value in [0, maximum + 1] {
            let error = match ServerLimits::new(field.input(Some(value))) {
                Ok(_) => panic!("{name}={value} must fail"),
                Err(error) => error,
            };
            assert_eq!(error.category(), ErrorCategory::LimitExceeded, "{name}");
            assert!(
                error.message().contains(name),
                "{name} failure must identify its field: {error}"
            );
        }
    }
}

#[test]
fn storage_object_ceiling_must_not_exceed_session_ceiling() {
    for (object_bytes, session_bytes, accepted) in [(1, 2, true), (2, 2, true), (2, 1, false)] {
        let result = ServerLimits::new(ServerLimitsInput {
            storage: StorageLimitInput {
                object_bytes: Some(object_bytes),
                session_bytes: Some(session_bytes),
            },
            ..ServerLimitsInput::default()
        });
        assert_eq!(
            result.is_ok(),
            accepted,
            "objectBytes={object_bytes}, sessionBytes={session_bytes}"
        );
        if !accepted {
            let error = result.expect_err("greater object ceiling must fail");
            assert_eq!(error.category(), ErrorCategory::LimitExceeded);
            assert!(error.message().contains("storage.objectBytes"));
            assert!(error.message().contains("storage.sessionBytes"));
        }
    }
}
