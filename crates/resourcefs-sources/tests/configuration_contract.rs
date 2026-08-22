use resourcefs_sources::{
    MAX_CONFIGURATION_ID_BYTES, MutationGrants, MutationSupport, validate_configuration_id,
};

#[test]
fn nested_grants_are_subsets() {
    let operations = [
        ("create", MutationGrants::new(true, false, false)),
        ("update", MutationGrants::new(false, true, false)),
        ("delete", MutationGrants::new(false, false, true)),
    ];

    for (support_name, support, expected) in [
        (
            "read-only",
            MutationSupport::READ_ONLY,
            [false, false, false],
        ),
        ("github", MutationSupport::GITHUB, [true, true, false]),
        ("mutable", MutationSupport::FULL, [true, true, true]),
    ] {
        assert!(
            support.validate(MutationGrants::default()).is_ok(),
            "{support_name}: absent grants must remain read-only"
        );
        for ((operation, grants), accepted) in operations.iter().zip(expected) {
            assert_eq!(
                support.validate(*grants).is_ok(),
                accepted,
                "{support_name} {operation}"
            );
        }
    }

    let parent = MutationGrants::new(true, true, false);
    for (name, child, accepted) in [
        ("empty", MutationGrants::default(), true),
        ("equal", parent, true),
        (
            "create-subset",
            MutationGrants::new(true, false, false),
            true,
        ),
        (
            "unsupported-delete",
            MutationGrants::new(false, false, true),
            false,
        ),
        (
            "broader-delete",
            MutationGrants::new(true, true, true),
            false,
        ),
    ] {
        assert_eq!(
            MutationSupport::GITHUB
                .validate_nested(parent, child)
                .is_ok(),
            accepted,
            "nested grant row {name}"
        );
    }

    let independent = MutationGrants::new(true, false, true);
    assert!(independent.create());
    assert!(!independent.update());
    assert!(independent.delete());

    for (name, id, accepted) in [
        ("empty", String::new(), false),
        ("one", "a".to_owned(), true),
        ("punctuation", "a.b_c-d9".to_owned(), true),
        ("bad-first", "-a".to_owned(), false),
        ("bad-rest", "a/b".to_owned(), false),
        ("unicode", "sourcé".to_owned(), false),
        (
            "exact-bound",
            format!("a{}", "b".repeat(MAX_CONFIGURATION_ID_BYTES - 1)),
            true,
        ),
        (
            "one-over",
            format!("a{}", "b".repeat(MAX_CONFIGURATION_ID_BYTES)),
            false,
        ),
    ] {
        assert_eq!(
            validate_configuration_id(&id).is_ok(),
            accepted,
            "configuration ID row {name}"
        );
    }
}
