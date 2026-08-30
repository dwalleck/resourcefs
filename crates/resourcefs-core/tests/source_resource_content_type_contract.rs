use std::io::Cursor;

use resourcefs_core::{PathReference, SourceResource, Utf8ContentType, VersionTag, select_utf8};

const JSON: &str = "application/json; charset=utf-8";
const ADF: &str = "application/vnd.atlassian.adf+json; charset=utf-8";
const MARKDOWN: &str = "text/markdown; charset=utf-8";

#[test]
fn complete_utf8_resource_preserves_media_and_content_tag() {
    for (content_type, content) in [
        (JSON, r#"{"a":1}"#),
        (ADF, r#"{"content":[],"type":"doc","version":1}"#),
        (MARKDOWN, "# Issue\n"),
    ] {
        let reference =
            PathReference::parse("jira://acme/issues/10001/fields/value").expect("canonical Field");
        let resource = SourceResource::utf8(
            reference,
            content.to_owned(),
            Utf8ContentType::new(content_type).expect("UTF-8 media type"),
        )
        .expect("typed UTF-8 Resource");
        assert_eq!(resource.content_type(), content_type);
        assert_eq!(resource.content(), content);
        assert_eq!(
            resource.version_tag(),
            &VersionTag::from_content(content.as_bytes())
        );
    }
}

#[test]
fn selected_utf8_resource_retains_whole_resource_tag_and_media() {
    let content = "# Issue\n\nFirst\nSecond\n";
    let requested =
        PathReference::parse("jira://acme/issues/10001:3-3").expect("selected Aggregate request");
    let selected =
        select_utf8(Cursor::new(content), requested.projection()).expect("selected Markdown");
    let canonical = PathReference::parse("jira://acme/issues/10001").expect("canonical Aggregate");
    let resource = SourceResource::selected_utf8(
        canonical,
        selected,
        Utf8ContentType::new(MARKDOWN).expect("Markdown media type"),
    )
    .expect("selected typed Resource");
    assert_eq!(resource.content(), "First\n");
    assert_eq!(resource.content_type(), MARKDOWN);
    assert_eq!(
        resource.version_tag(),
        &VersionTag::from_content(content.as_bytes())
    );
}

#[test]
fn utf8_content_type_rejects_ambiguous_or_non_utf8_media() {
    for invalid in [
        "",
        "application/json",
        "application/json; charset=UTF-8",
        "application/json; charset=utf-8; profile=extra",
        "application json; charset=utf-8",
        "application/json; charset=utf-8\nX-Leak: yes",
    ] {
        assert!(Utf8ContentType::new(invalid).is_err(), "{invalid:?}");
    }
}
