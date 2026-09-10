use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use cargo_metadata::{MetadataCommand, Package};

fn dependency_names(package: &Package) -> BTreeSet<&str> {
    package
        .dependencies
        .iter()
        .map(|dependency| dependency.name.as_str())
        .collect()
}

#[test]
fn enforces_dependency_direction() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("workspace cargo metadata");
    let packages: BTreeMap<_, _> = metadata
        .workspace_packages()
        .into_iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    assert_eq!(
        packages.keys().copied().collect::<Vec<_>>(),
        vec!["resourcefs-core", "resourcefs-mcp", "resourcefs-sources"]
    );

    let core = dependency_names(packages["resourcefs-core"]);
    assert!(!core.contains("rmcp"));
    assert!(!core.contains("clap"));

    let sources = dependency_names(packages["resourcefs-sources"]);
    assert!(sources.contains("resourcefs-core"));
    assert!(!sources.contains("rmcp"));
    assert!(!sources.contains("clap"));
    for required in ["serde", "serde_json", "serde_path_to_error"] {
        assert!(
            sources.contains(required),
            "source-native wire decoding requires resourcefs-sources -> {required}"
        );
    }
    assert!(
        !sources.contains("schemars"),
        "operator profile schemas belong to resourcefs-mcp, never source-native wire decoding"
    );

    let sources_root = metadata
        .workspace_root
        .as_std_path()
        .join("crates/resourcefs-sources");
    for forbidden in ["ProfileDocument", "GithubSourceProfile", "JsonSchema"] {
        assert!(
            files_containing_token(&sources_root, forbidden).is_empty(),
            "operator profile token '{forbidden}' leaked into resourcefs-sources"
        );
    }
    // serde is admitted into resourcefs-sources for source-native wire
    // decoding only. The invariant the old "no serde" fence protected was that
    // operator profile syntax lives in resourcefs-mcp. Confining serde to
    // provider wire modules holds that line: profile DTOs or serde derives in
    // transport/core-facing modules fail here regardless of their type names.
    let sources_src = sources_root.join("src");
    let wire_modules = [sources_src.join("github"), sources_src.join("atlassian")];
    let serde_users = files_containing_token(&sources_src, "serde");
    assert!(
        !serde_users.is_empty(),
        "provider wire modules are expected to decode with serde"
    );
    for file in &serde_users {
        assert!(
            wire_modules.iter().any(|module| file.starts_with(module)),
            "serde reached {} outside provider wire modules src/github/ and src/atlassian/",
            file.display()
        );
    }
    let core_source = metadata
        .workspace_root
        .as_std_path()
        .join("crates/resourcefs-core/src");
    assert!(
        files_containing_token(&core_source, "serde_json").is_empty(),
        "source-neutral core production code must not decode source-native JSON"
    );

    let mcp = dependency_names(packages["resourcefs-mcp"]);
    assert!(mcp.contains("resourcefs-core"));
    assert!(mcp.contains("resourcefs-sources"));
    assert!(mcp.contains("rmcp"));
    assert!(mcp.contains("clap"));
}

#[test]
fn git_object_dependencies_stay_in_source_adapter() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("workspace cargo metadata");
    let packages: BTreeMap<_, _> = metadata
        .workspace_packages()
        .into_iter()
        .map(|package| (package.name.as_str(), package))
        .collect();
    let core = dependency_names(packages["resourcefs-core"]);
    let sources = dependency_names(packages["resourcefs-sources"]);
    let mcp = dependency_names(packages["resourcefs-mcp"]);

    for dependency in ["gix-object", "gix-hash"] {
        assert!(
            sources.contains(dependency),
            "Git object format dependency {dependency} belongs to resourcefs-sources"
        );
        assert!(
            !core.contains(dependency) && !mcp.contains(dependency),
            "Git object format dependency {dependency} must not enter core or MCP"
        );
    }

    let workspace = metadata.workspace_root.as_std_path();
    let core_source = workspace.join("crates/resourcefs-core/src");
    let mcp_source = workspace.join("crates/resourcefs-mcp/src");
    for token in ["gix_object", "gix_hash"] {
        assert!(
            files_containing_token(&core_source, token).is_empty(),
            "{token} implementation leaked into resourcefs-core"
        );
        assert!(
            files_containing_token(&mcp_source, token).is_empty(),
            "{token} implementation leaked into resourcefs-mcp"
        );
    }
}

#[test]
fn mutation_engine_dependency_direction() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("workspace cargo metadata");
    let core = metadata
        .workspace_packages()
        .into_iter()
        .find(|package| package.name == "resourcefs-core")
        .expect("resourcefs-core package");
    let dependencies = dependency_names(core);
    for forbidden in ["cap-std", "rmcp", "rustix", "windows-sys"] {
        assert!(
            !dependencies.contains(forbidden),
            "forbidden edge resourcefs-core -> {forbidden}: mutation platform mechanics belong to Source Adapters"
        );
    }
}

#[test]
fn forbids_telemetry_dependencies() {
    let metadata = MetadataCommand::new()
        .exec()
        .expect("workspace cargo metadata");
    let forbidden = [
        "metrics-exporter-prometheus",
        "opentelemetry",
        "opentelemetry-otlp",
        "sentry",
        "tracing-opentelemetry",
    ];
    let offenders = metadata
        .packages
        .iter()
        .filter_map(|package| {
            forbidden
                .contains(&package.name.as_str())
                .then_some(package.name.as_str())
        })
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "telemetry/exporter dependencies are forbidden: {offenders:?}"
    );
}

#[test]
fn profile_capabilities_stay_in_owning_modules() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("workspace cargo metadata");
    let root = metadata.workspace_root.as_std_path();
    let launch = std::fs::read_to_string(root.join("crates/resourcefs-mcp/src/launch.rs"))
        .expect("launch module");
    let server = std::fs::read_to_string(root.join("crates/resourcefs-mcp/src/server.rs"))
        .expect("server module");

    for forbidden in ["tokio::process", "CommandExecutor", "ProfileDocument"] {
        assert!(
            !launch.contains(forbidden),
            "launch.rs must compose checked values without owning `{forbidden}` capabilities"
        );
    }
    for forbidden in ["CommandExecutor", "ProfileDocument"] {
        assert!(
            !server.contains(forbidden),
            "server.rs must consume LaunchPlan without owning `{forbidden}` capabilities"
        );
    }
}
/// Recursively collect every `.rs` file under `root` whose contents contain
/// `token` as a standalone identifier. Build output (`target`) and hidden
/// directories (`.git`) are skipped so the scan never trips over generated or
/// vendored files.
fn files_containing_token(root: &Path, token: &str) -> Vec<PathBuf> {
    fn is_ident_boundary(c: char) -> bool {
        !(c.is_ascii_alphanumeric() || c == '_')
    }

    fn scan(dir: &Path, token: &str, found: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir).unwrap_or_else(|err| {
            panic!(
                "cannot read directory {} while scanning for `{token}`: {err}",
                dir.display()
            )
        });
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| {
                panic!(
                    "cannot read entry in {} while scanning for `{token}`: {err}",
                    dir.display()
                )
            });
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    scan(&path, token, found);
                }
            } else if path.extension().is_some_and(|ext| ext == OsStr::new("rs")) {
                let content = std::fs::read_to_string(&path).unwrap_or_else(|err| {
                    panic!(
                        "cannot read {} while scanning for `{token}`: {err}",
                        path.display()
                    )
                });
                if content.split(is_ident_boundary).any(|part| part == token) {
                    found.push(path);
                }
            }
        }
    }

    let mut found = Vec::new();
    scan(root, token, &mut found);
    found
}

#[test]
fn discovery_dependencies_stay_in_owning_modules() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("workspace cargo metadata");
    let packages: BTreeMap<_, _> = metadata
        .workspace_packages()
        .into_iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    // The source-neutral core contract must stay free of every search engine,
    // capability sandbox, and FFI binding: discovery machinery belongs to
    // resourcefs-sources alone.
    let core = dependency_names(packages["resourcefs-core"]);
    for forbidden in [
        "rmcp",
        "cap-std",
        "regex",
        "pcre2-sys",
        "globset",
        "ignore",
        "html5ever",
    ] {
        assert!(
            !core.contains(forbidden),
            "forbidden edge resourcefs-core -> {forbidden}: discovery machinery must stay in resourcefs-sources"
        );
    }

    // The MCP server owns rmcp/clap but must not reach into compiled-pattern
    // engines or capability sandboxes.
    let mcp = dependency_names(packages["resourcefs-mcp"]);
    for forbidden in [
        "cap-std",
        "regex",
        "pcre2-sys",
        "globset",
        "ignore",
        "html5ever",
    ] {
        assert!(
            !mcp.contains(forbidden),
            "forbidden edge resourcefs-mcp -> {forbidden}: discovery machinery must stay in resourcefs-sources"
        );
    }

    // Raw pcre2 FFI is a single-file implementation detail of the sources
    // crate. Prove it by scanning the workspace sources (build output and
    // hidden dirs excluded) for the identifier instead of relying on the
    // crate's private implementation ordering. The identifier is assembled at
    // runtime so this fence's own source text does not match the scan.
    let token = format!("pcre2_{}", "sys");
    let workspace_root = metadata.workspace_root.as_std_path();
    let mut offenders: Vec<String> = files_containing_token(workspace_root, &token)
        .into_iter()
        .map(|path| {
            path.strip_prefix(workspace_root)
                .unwrap_or_else(|_| {
                    panic!("scanned path {} escaped the workspace root", path.display())
                })
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    offenders.sort();
    assert_eq!(
        offenders,
        vec!["crates/resourcefs-sources/src/pattern.rs"],
        "raw FFI token must appear only in crates/resourcefs-sources/src/pattern.rs; \
         found in: {}",
        offenders.join(", ")
    );
}

/// Collects the workspace-relative paths naming `token`, sorted.
fn offending_paths(workspace_root: &Path, token: &str) -> Vec<String> {
    let mut offenders: Vec<String> = files_containing_token(workspace_root, token)
        .into_iter()
        .map(|path| {
            path.strip_prefix(workspace_root)
                .unwrap_or_else(|_| {
                    panic!("scanned path {} escaped the workspace root", path.display())
                })
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    offenders.sort();
    offenders
}

/// rfs-g2z9 C15 — the bounded substrate is the workspace's single network
/// egress point.
///
/// Two independent assertions, because they fail for different reasons: the
/// crate graph cannot see *which module* uses a dependency, and a filesystem
/// scan cannot see a dependency edge. Both tokens are assembled at runtime so
/// this fence's own source text does not match its own scan.
#[test]
fn single_http_client_module() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("workspace cargo metadata");
    let packages: BTreeMap<_, _> = metadata
        .workspace_packages()
        .into_iter()
        .map(|package| (package.name.as_str(), package))
        .collect();
    let workspace_root = metadata.workspace_root.as_std_path();

    // The client belongs to the sources crate alone: neither the source-neutral
    // core contract nor the protocol adapter may reach the network.
    let client = format!("req{}", "west");
    for crate_name in ["resourcefs-core", "resourcefs-mcp"] {
        assert!(
            !dependency_names(packages[crate_name]).contains(client.as_str()),
            "forbidden edge {crate_name} -> {client}: network egress belongs to resourcefs-sources"
        );
    }

    // Exactly one module may construct a client. A second one would be able to
    // issue requests that never pass the policy resolver.
    let offenders = offending_paths(workspace_root, &client);
    assert_eq!(
        offenders,
        vec!["crates/resourcefs-sources/src/http/mod.rs"],
        "the HTTP client may be named only in the bounded substrate; found in: {}",
        offenders.join(", ")
    );

    // Raw TCP egress is confined to the probe, which authorizes each resolved
    // address through the same policy the substrate applies.
    let socket = format!("TcpS{}", "tream");
    let offenders = offending_paths(workspace_root, &socket);
    assert_eq!(
        offenders,
        vec!["crates/resourcefs-sources/src/probe.rs"],
        "raw TCP egress must stay in the policed probe; found in: {}",
        offenders.join(", ")
    );
}

/// rfs-cek3 C3 — fixture trust is callable only from the integration-test
/// harness; no production CLI, profile, or environment input can select it.
#[test]
fn profile_https_fixture_trust_is_test_harness_only() {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("workspace cargo metadata");
    let root = metadata.workspace_root.as_std_path();
    let mcp_root = root.join("crates/resourcefs-mcp");
    let source_root = mcp_root.join("src");
    let test_support_path = source_root.join("test_support.rs");

    assert!(
        test_support_path.is_file(),
        "fixture trust must live in the gated test-support module"
    );
    let test_support =
        std::fs::read_to_string(&test_support_path).expect("test-support module source");
    assert!(
        test_support.contains("serve_profile_with_https_root"),
        "the test harness needs one explicit profile-launch interface"
    );

    let library = std::fs::read_to_string(source_root.join("lib.rs")).expect("library source");
    assert!(
        library
            .contains("#[cfg(feature = \"test-support\")]\n#[doc(hidden)]\npub mod test_support;"),
        "the profile fixture interface must be absent without test-support"
    );

    let production_input = format!("RESOURCEFS_TEST_HTTPS_{}", "ROOT");
    assert!(
        files_containing_token(&source_root, &production_input).is_empty(),
        "the production source tree must not read a fixture-root environment input"
    );
    for entrypoint in ["main.rs", "cli.rs"] {
        let source =
            std::fs::read_to_string(source_root.join(entrypoint)).expect("production entrypoint");
        assert!(
            !source.contains("test_support"),
            "{entrypoint} must not call the fixture launch adapter"
        );
    }

    let package = metadata
        .workspace_packages()
        .into_iter()
        .find(|package| package.name.as_str() == "resourcefs-mcp")
        .expect("resourcefs-mcp package");
    assert!(
        !package.features.contains_key("default"),
        "the shipped default feature set must not enable test-support"
    );
}
