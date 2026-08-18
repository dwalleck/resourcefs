use std::collections::{BTreeMap, BTreeSet};

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

    let mcp = dependency_names(packages["resourcefs-mcp"]);
    assert!(mcp.contains("resourcefs-core"));
    assert!(mcp.contains("resourcefs-sources"));
    assert!(mcp.contains("rmcp"));
    assert!(mcp.contains("clap"));
}
