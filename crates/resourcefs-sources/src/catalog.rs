use std::fmt::Write as _;

use resourcefs_core::{
    ErrorCategory, PathReference, ProbeDiagnostic, ResourceError, VersionTag, WorkspaceRootSet,
};

use crate::FilesystemSource;

pub(crate) const MAX_SOURCE_CATALOG_ENTRIES: usize = 256;
const MAX_SOURCE_CATALOG_FIELD_BYTES: usize = 4_096;
const SOURCE_CATALOG_HEADER: &str = concat!(
    "Mounted sources\n",
    "Next discovery step: rfs_read rfs://workspace\n",
    "Selectors: :N | :N-M | :N- | comma-separated ranges | :raw | :page:N\n",
);

pub(crate) trait SourceCatalogMetadata {
    fn catalog_entries(&self) -> Result<Vec<SourceCatalogEntry>, ResourceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceCatalogEntry {
    scheme: String,
    grammar: String,
    example: String,
    degraded_reason: Option<ProbeDiagnostic>,
}

impl SourceCatalogEntry {
    pub(crate) fn new(
        scheme: impl Into<String>,
        grammar: impl Into<String>,
        example: impl Into<String>,
        degraded_reason: Option<ProbeDiagnostic>,
    ) -> Result<Self, ResourceError> {
        let scheme = scheme.into();
        let grammar = grammar.into();
        let example = example.into();
        validate_field("scheme", &scheme)?;
        validate_field("grammar", &grammar)?;
        validate_field("example", &example)?;
        PathReference::parse(example.clone()).map_err(|error| {
            ResourceError::new(
                ErrorCategory::InvalidReference,
                format!(
                    "source catalog example '{example}' is not a valid Path Reference: {error}"
                ),
            )
        })?;
        if let Some(reason) = degraded_reason.as_ref() {
            validate_field("degraded reason", reason.as_str())?;
        }
        Ok(Self {
            scheme,
            grammar,
            example,
            degraded_reason,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogDocument {
    content: String,
    version_tag: VersionTag,
}

impl CatalogDocument {
    fn new(content: String) -> Self {
        let version_tag = VersionTag::from_content(content.as_bytes());
        Self {
            content,
            version_tag,
        }
    }

    pub(crate) fn content(&self) -> &str {
        &self.content
    }

    pub(crate) fn into_parts(self) -> (String, VersionTag) {
        (self.content, self.version_tag)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NamespaceCatalog;

impl NamespaceCatalog {
    pub(crate) fn source_document(
        entries: Vec<SourceCatalogEntry>,
    ) -> Result<CatalogDocument, ResourceError> {
        let entries = normalize_source_entries(entries)?;
        let capacity = entries
            .iter()
            .try_fold(SOURCE_CATALOG_HEADER.len(), |used, entry| {
                let state_bytes = entry
                    .degraded_reason
                    .as_ref()
                    .map_or(0, |reason| " [degraded: ]".len() + reason.as_str().len());
                used.checked_add(entry.scheme.len())
                    .and_then(|value| value.checked_add(" — ".len() * 2))
                    .and_then(|value| value.checked_add(entry.grammar.len()))
                    .and_then(|value| value.checked_add(entry.example.len()))
                    .and_then(|value| value.checked_add(state_bytes + 1))
                    .ok_or_else(catalog_document_too_large)
            })?;
        let mut content = String::with_capacity(capacity);
        content.push_str(SOURCE_CATALOG_HEADER);
        for entry in entries {
            write!(
                content,
                "{} — {} — {}",
                entry.scheme, entry.grammar, entry.example
            )
            .expect("writing source catalog text to String cannot fail");
            if let Some(reason) = entry.degraded_reason {
                write!(content, " [degraded: {}]", reason.as_str())
                    .expect("writing source state to String cannot fail");
            }
            content.push('\n');
        }
        Ok(CatalogDocument::new(content))
    }

    pub(crate) async fn workspace_document(
        filesystem: &FilesystemSource,
    ) -> Result<CatalogDocument, ResourceError> {
        let roots = filesystem.workspace_root_set().await?;
        Ok(Self::workspace_document_for(&roots))
    }

    pub(crate) fn workspace_document_for(roots: &WorkspaceRootSet) -> CatalogDocument {
        if roots.roots().is_empty() {
            return CatalogDocument::new("No Workspace Roots declared.\n".to_owned());
        }
        let mut sorted = roots.roots().iter().collect::<Vec<_>>();
        sorted.sort_unstable_by(|left, right| left.id().cmp(right.id()));
        let mut content = String::with_capacity(sorted.len() * 96 + 36);
        for root in sorted {
            write!(content, "rfs://workspace/{}/", root.id())
                .expect("writing Workspace Root identity to String cannot fail");
            if roots.primary() == Some(root.id()) {
                content.push_str(" (primary)");
            }
            content.push('\n');
        }
        if roots.primary().is_none() {
            content.push_str("No Primary Workspace Root selected.\n");
        }
        CatalogDocument::new(content)
    }
}

fn normalize_source_entries(
    mut entries: Vec<SourceCatalogEntry>,
) -> Result<Vec<SourceCatalogEntry>, ResourceError> {
    if entries.is_empty() {
        return Err(ResourceError::new(
            ErrorCategory::InvalidReference,
            "compiled Source Adapter registry must not be empty",
        ));
    }
    if entries.len() > MAX_SOURCE_CATALOG_ENTRIES {
        return Err(ResourceError::new(
            ErrorCategory::LimitExceeded,
            format!(
                "compiled Source Adapter registry exceeds {MAX_SOURCE_CATALOG_ENTRIES} entries"
            ),
        ));
    }
    entries.sort_unstable_by(|left, right| left.scheme.cmp(&right.scheme));
    if entries
        .windows(2)
        .any(|pair| pair[0].scheme == pair[1].scheme)
    {
        return Err(ResourceError::new(
            ErrorCategory::AmbiguousReference,
            "compiled Source Adapter registry contains a duplicate scheme label",
        ));
    }
    Ok(entries)
}

fn validate_field(name: &str, value: &str) -> Result<(), ResourceError> {
    if value.is_empty() {
        return Err(ResourceError::new(
            ErrorCategory::InvalidReference,
            format!("source catalog {name} must not be empty"),
        ));
    }
    if value.len() > MAX_SOURCE_CATALOG_FIELD_BYTES {
        return Err(ResourceError::new(
            ErrorCategory::LimitExceeded,
            format!("source catalog {name} exceeds {MAX_SOURCE_CATALOG_FIELD_BYTES} UTF-8 bytes"),
        ));
    }
    if value.contains(['\r', '\n']) {
        return Err(ResourceError::new(
            ErrorCategory::InvalidReference,
            format!("source catalog {name} must be one line"),
        ));
    }
    Ok(())
}

fn catalog_document_too_large() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "source catalog document size overflowed",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use resourcefs_core::{
        ErrorCategory, ProbeDiagnostic, WorkspaceRoot, WorkspaceRootId, WorkspaceRootSet,
    };

    use super::{
        MAX_SOURCE_CATALOG_ENTRIES, NamespaceCatalog, SOURCE_CATALOG_HEADER, SourceCatalogEntry,
    };
    use crate::{BackingPathVisibility, FilesystemSource, LaunchRoot, LaunchRootSource};
    use tempfile::TempDir;

    fn active(scheme: &str, grammar: &str, example: &str) -> SourceCatalogEntry {
        SourceCatalogEntry::new(scheme, grammar, example, None).expect("active entry")
    }

    fn degraded(scheme: &str, grammar: &str, example: &str, reason: &str) -> SourceCatalogEntry {
        SourceCatalogEntry::new(
            scheme,
            grammar,
            example,
            Some(ProbeDiagnostic::new(reason.to_owned()).expect("public diagnostic")),
        )
        .expect("degraded entry")
    }

    fn root(id: &str, selector: Option<&str>) -> WorkspaceRoot {
        let path = std::env::temp_dir().join("resourcefs-catalog").join(id);
        WorkspaceRoot::new(
            WorkspaceRootId::new(id).expect("root ID"),
            url::Url::from_directory_path(path)
                .expect("root file URI")
                .to_string(),
            selector.map(str::to_owned),
        )
        .expect("Workspace Root")
    }

    #[test]
    fn catalog_metadata_rejects_duplicate_and_invalid_entries() {
        let empty = NamespaceCatalog::source_document(Vec::new()).expect_err("empty registry");
        assert_eq!(empty.category(), ErrorCategory::InvalidReference);

        let duplicate = NamespaceCatalog::source_document(vec![
            active("artifact://", "first", "fixture.txt"),
            active("artifact://", "second", "other.txt"),
        ])
        .expect_err("duplicate scheme");
        assert_eq!(duplicate.category(), ErrorCategory::AmbiguousReference);

        for (name, value) in [("scheme", ""), ("grammar", "bad\nline")] {
            let result = match name {
                "scheme" => SourceCatalogEntry::new(value, "grammar", "fixture.txt", None),
                "grammar" => SourceCatalogEntry::new("fixture://", value, "fixture.txt", None),
                _ => unreachable!("fixed metadata case"),
            };
            assert_eq!(
                result.expect_err(name).category(),
                ErrorCategory::InvalidReference
            );
        }
        assert_eq!(
            SourceCatalogEntry::new("fixture://", "grammar", "ftp://example.com", None,)
                .expect_err("unsupported example")
                .category(),
            ErrorCategory::InvalidReference
        );
        assert_eq!(
            SourceCatalogEntry::new(
                "s".repeat(super::MAX_SOURCE_CATALOG_FIELD_BYTES + 1),
                "grammar",
                "fixture.txt",
                None,
            )
            .expect_err("oversized scheme")
            .category(),
            ErrorCategory::LimitExceeded
        );
        assert_eq!(
            SourceCatalogEntry::new(
                "fixture://",
                "grammar",
                "fixture.txt",
                Some(ProbeDiagnostic::new("bad\nreason".to_owned()).expect("bounded diagnostic"),),
            )
            .expect_err("multiline reason")
            .category(),
            ErrorCategory::InvalidReference
        );
    }

    #[test]
    fn catalog_metadata_accepts_exact_boundaries() {
        let boundary = SourceCatalogEntry::new(
            "s".repeat(super::MAX_SOURCE_CATALOG_FIELD_BYTES),
            "g".repeat(super::MAX_SOURCE_CATALOG_FIELD_BYTES),
            "x".repeat(super::MAX_SOURCE_CATALOG_FIELD_BYTES),
            None,
        )
        .expect("exact field boundaries");
        NamespaceCatalog::source_document(vec![boundary]).expect("boundary document");

        let entries = (0..MAX_SOURCE_CATALOG_ENTRIES)
            .map(|index| {
                active(
                    &format!("scheme-{index:03}"),
                    "grammar",
                    &format!("fixture-{index:03}.txt"),
                )
            })
            .collect();
        NamespaceCatalog::source_document(entries).expect("256 entries");

        let overflow = (0..=MAX_SOURCE_CATALOG_ENTRIES)
            .map(|index| {
                active(
                    &format!("scheme-{index:03}"),
                    "grammar",
                    &format!("fixture-{index:03}.txt"),
                )
            })
            .collect();
        assert_eq!(
            NamespaceCatalog::source_document(overflow)
                .expect_err("257 entries")
                .category(),
            ErrorCategory::LimitExceeded
        );
    }

    #[test]
    fn source_catalog_marks_degraded_and_omits_unmounted() {
        let active = active(
            "artifact://",
            "artifact://<session>-<id>[:selector]",
            "artifact://00000000000000000000000000000000-1",
        );
        let degraded = degraded(
            "rfs://workspace",
            "rfs://workspace/<root>/<path>[:selector]",
            "rfs://workspace/workspace/src/lib.rs",
            "temporarily unavailable",
        );

        let document = NamespaceCatalog::source_document(vec![degraded, active])
            .expect("source catalog document");
        assert_eq!(
            document.content(),
            concat!(
                "Mounted sources\n",
                "Next discovery step: rfs_read rfs://workspace\n",
                "Selectors: :N | :N-M | :N- | comma-separated ranges | :raw | :page:N\n",
                "artifact:// — artifact://<session>-<id>[:selector] — artifact://00000000000000000000000000000000-1\n",
                "rfs://workspace — rfs://workspace/<root>/<path>[:selector] — rfs://workspace/workspace/src/lib.rs [degraded: temporarily unavailable]\n",
            )
        );
        assert!(!document.content().contains("local://"));
    }

    #[test]
    fn workspace_catalog_covers_empty_primary_and_max_root_sets() {
        let empty = WorkspaceRootSet::new(Vec::new(), None).expect("empty root set");
        assert_eq!(
            NamespaceCatalog::workspace_document_for(&empty).content(),
            "No Workspace Roots declared.\n"
        );

        let single = WorkspaceRootSet::new(vec![root("single", None)], None).expect("single root");
        assert_eq!(
            NamespaceCatalog::workspace_document_for(&single).content(),
            "rfs://workspace/single/ (primary)\n"
        );

        let unselected = WorkspaceRootSet::new(
            vec![root("z-last", Some("z")), root("alpha", Some("a"))],
            None,
        )
        .expect("unselected roots");
        assert_eq!(
            NamespaceCatalog::workspace_document_for(&unselected).content(),
            concat!(
                "rfs://workspace/alpha/\n",
                "rfs://workspace/z-last/\n",
                "No Primary Workspace Root selected.\n",
            )
        );

        let selected = WorkspaceRootSet::new(
            vec![root("z-last", Some("z")), root("alpha", Some("a"))],
            Some("z"),
        )
        .expect("selected roots");
        assert_eq!(
            NamespaceCatalog::workspace_document_for(&selected).content(),
            "rfs://workspace/alpha/\nrfs://workspace/z-last/ (primary)\n"
        );

        let maximum = WorkspaceRootSet::new(
            (0..resourcefs_core::MAX_WORKSPACE_ROOTS)
                .rev()
                .map(|index| root(&format!("r{index:03}"), None))
                .collect(),
            None,
        )
        .expect("maximum roots");
        let maximum_document = NamespaceCatalog::workspace_document_for(&maximum);
        assert_eq!(
            maximum_document
                .content()
                .lines()
                .filter(|line| line.starts_with("rfs://workspace/"))
                .count(),
            resourcefs_core::MAX_WORKSPACE_ROOTS
        );
        assert!(
            maximum_document
                .content()
                .starts_with("rfs://workspace/r000/\n")
        );
    }

    #[tokio::test]
    async fn workspace_document_reads_one_active_authority_snapshot() {
        let temporary = TempDir::new().expect("temporary directory");
        let alpha = temporary.path().join("alpha");
        let z_last = temporary.path().join("z-last");
        fs::create_dir(&alpha).expect("alpha root");
        fs::create_dir(&z_last).expect("z-last root");
        let filesystem = FilesystemSource::new(
            LaunchRootSource::Cli(vec![
                LaunchRoot::read_only(WorkspaceRootId::new("z-last").expect("z-last ID"), z_last),
                LaunchRoot::read_only(WorkspaceRootId::new("alpha").expect("alpha ID"), alpha),
            ]),
            Some("z-last".to_owned()),
            BackingPathVisibility::Hidden,
        )
        .await
        .expect("filesystem source");

        assert_eq!(
            NamespaceCatalog::workspace_document(&filesystem)
                .await
                .expect("active workspace catalog")
                .content(),
            "rfs://workspace/alpha/\nrfs://workspace/z-last/ (primary)\n"
        );

        filesystem.begin_client_root_refresh().await;
        assert_eq!(
            NamespaceCatalog::workspace_document(&filesystem)
                .await
                .expect_err("refreshing authority")
                .category(),
            ErrorCategory::SourceUnavailable
        );
    }

    #[test]
    fn catalog_content_and_tags_are_snapshot_deterministic() {
        let entries = vec![active("artifact://", "grammar", "fixture.txt")];
        let first = NamespaceCatalog::source_document(entries.clone()).expect("first document");
        let second = NamespaceCatalog::source_document(entries).expect("second document");
        assert_eq!(first, second);

        let changed = NamespaceCatalog::source_document(vec![degraded(
            "artifact://",
            "grammar",
            "fixture.txt",
            "degraded",
        )])
        .expect("changed document");
        assert_ne!(first, changed);
        assert_ne!(first.version_tag, changed.version_tag);
    }

    #[test]
    fn source_catalog_header_chains_and_teaches_selectors() {
        let document = NamespaceCatalog::source_document(vec![active(
            "artifact://",
            "grammar",
            "fixture.txt",
        )])
        .expect("source document");
        assert!(document.content().starts_with(SOURCE_CATALOG_HEADER));
        assert_eq!(
            SOURCE_CATALOG_HEADER,
            concat!(
                "Mounted sources\n",
                "Next discovery step: rfs_read rfs://workspace\n",
                "Selectors: :N | :N-M | :N- | comma-separated ranges | :raw | :page:N\n",
            )
        );
    }

    #[test]
    fn catalog_production_budget() {
        let maximum = super::MAX_SOURCE_CATALOG_FIELD_BYTES;
        let entries = (0..MAX_SOURCE_CATALOG_ENTRIES)
            .map(|index| {
                let prefix = format!("s{index:03}");
                SourceCatalogEntry::new(
                    format!("{prefix}{}", "s".repeat(maximum - prefix.len())),
                    "g".repeat(maximum),
                    format!("{index:03}{}", "x".repeat(maximum - 3)),
                    Some(ProbeDiagnostic::new("d".repeat(maximum)).expect("maximum reason")),
                )
                .expect("maximum entry")
            })
            .collect();
        let started = Instant::now();
        let source = NamespaceCatalog::source_document(entries).expect("maximum source document");
        let source_elapsed = started.elapsed();
        assert!(source.content().len() > 4 * 1024 * 1024 - 64 * 1024);

        let roots = WorkspaceRootSet::new(
            (0..resourcefs_core::MAX_WORKSPACE_ROOTS)
                .map(|index| {
                    let prefix = format!("r{index:03}");
                    root(&format!("{prefix}{}", "x".repeat(128 - prefix.len())), None)
                })
                .collect(),
            None,
        )
        .expect("maximum roots");
        let started = Instant::now();
        let workspace = NamespaceCatalog::workspace_document_for(&roots);
        let workspace_elapsed = started.elapsed();
        assert_eq!(
            workspace
                .content()
                .lines()
                .filter(|line| line.starts_with("rfs://workspace/"))
                .count(),
            resourcefs_core::MAX_WORKSPACE_ROOTS
        );

        if !cfg!(debug_assertions) {
            assert!(
                source_elapsed <= Duration::from_millis(250),
                "maximum source catalog took {source_elapsed:?}"
            );
            assert!(
                workspace_elapsed <= Duration::from_millis(25),
                "maximum workspace catalog took {workspace_elapsed:?}"
            );
        }
    }
}
