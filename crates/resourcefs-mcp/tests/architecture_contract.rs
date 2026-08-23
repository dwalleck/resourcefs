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
    for forbidden in ["serde", "serde_json", "schemars"] {
        assert!(
            !sources.contains(forbidden),
            "forbidden edge resourcefs-sources -> {forbidden}: operator profile syntax and schema belong to resourcefs-mcp"
        );
    }

    let mcp = dependency_names(packages["resourcefs-mcp"]);
    assert!(mcp.contains("resourcefs-core"));
    assert!(mcp.contains("resourcefs-sources"));
    assert!(mcp.contains("rmcp"));
    assert!(mcp.contains("clap"));
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
