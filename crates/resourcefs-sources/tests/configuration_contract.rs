use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use resourcefs_sources::{
    AgentExportConfig, ChildEnvironment, CommandSpec, ConfigurationDirectory,
    ConfigurationTargetKind, ConverterInput, CredentialHeader, DocumentConverter, DocumentsConfig,
    DownstreamMcpConfig, DownstreamServer, DownstreamTransport, EnvironmentValue, GithubConfig,
    GithubRepository, HttpsConfig, HttpsOrigin, MAX_COMMAND_ARGUMENT_BYTES, MAX_COMMAND_ARGUMENTS,
    MAX_COMMAND_ENVIRONMENT_ENTRIES, MAX_CONFIGURATION_ENTRIES, MAX_CONFIGURATION_ID_BYTES,
    MAX_EXTENSION_BYTES, MemoryConfig, MemoryRoot, MutationGrants, MutationSupport, RulesConfig,
    SchemeClaim, SecretReference, SkillsConfig, SshConfig, SshHost, VaultConfig, VaultRoot,
    validate_configuration_id,
};
use tempfile::TempDir;

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

fn command(executable: &str) -> CommandSpec {
    CommandSpec::new(
        vec![executable.to_owned()],
        ChildEnvironment::new(BTreeMap::new()).expect("empty child environment"),
    )
    .expect("valid command")
}

fn secret() -> SecretReference {
    SecretReference::environment("RESOURCEFS_TEST_TOKEN")
        .expect("valid environment secret reference")
}

#[test]
fn command_and_secret_shapes_are_validated() {
    assert!(CommandSpec::new(vec![], ChildEnvironment::default()).is_err());
    assert!(CommandSpec::new(vec![String::new()], ChildEnvironment::default()).is_err());
    assert!(CommandSpec::new(vec!["program\0".to_owned()], ChildEnvironment::default()).is_err());
    assert!(
        CommandSpec::new(
            vec!["program".to_owned(), String::new()],
            ChildEnvironment::default(),
        )
        .is_ok(),
        "empty non-executable argv elements are literal arguments"
    );
    assert!(
        CommandSpec::new(
            vec!["x".to_owned(); MAX_COMMAND_ARGUMENTS],
            ChildEnvironment::default(),
        )
        .is_ok()
    );
    assert!(
        CommandSpec::new(
            vec!["x".to_owned(); MAX_COMMAND_ARGUMENTS + 1],
            ChildEnvironment::default(),
        )
        .is_err()
    );
    assert!(
        CommandSpec::new(
            vec!["x".repeat(MAX_COMMAND_ARGUMENT_BYTES)],
            ChildEnvironment::default(),
        )
        .is_ok()
    );
    assert!(
        CommandSpec::new(
            vec!["x".repeat(MAX_COMMAND_ARGUMENT_BYTES + 1)],
            ChildEnvironment::default(),
        )
        .is_err()
    );

    assert!(EnvironmentValue::literal("value").is_ok());
    assert!(EnvironmentValue::literal("bad\0value").is_err());
    assert!(EnvironmentValue::inherit("PATH").is_ok());
    assert!(EnvironmentValue::inherit("BAD=NAME").is_err());

    let mut invalid_destination = BTreeMap::new();
    invalid_destination.insert(
        "BAD=NAME".to_owned(),
        EnvironmentValue::literal("value").expect("literal"),
    );
    assert!(ChildEnvironment::new(invalid_destination).is_err());
    let mut case_colliding_destinations = BTreeMap::new();
    case_colliding_destinations.insert(
        "Path".to_owned(),
        EnvironmentValue::literal("one").expect("literal"),
    );
    case_colliding_destinations.insert(
        "PATH".to_owned(),
        EnvironmentValue::literal("two").expect("literal"),
    );
    assert!(ChildEnvironment::new(case_colliding_destinations).is_err());
    let mut unicode_case_colliding_destinations = BTreeMap::new();
    unicode_case_colliding_destinations.insert(
        "Straße".to_owned(),
        EnvironmentValue::literal("one").expect("literal"),
    );
    unicode_case_colliding_destinations.insert(
        "STRASSE".to_owned(),
        EnvironmentValue::literal("two").expect("literal"),
    );
    assert!(ChildEnvironment::new(unicode_case_colliding_destinations).is_err());

    let exact = (0..MAX_COMMAND_ENVIRONMENT_ENTRIES)
        .map(|index| {
            (
                format!("NAME_{index}"),
                EnvironmentValue::inherit("PATH").expect("inherited PATH"),
            )
        })
        .collect();
    assert!(ChildEnvironment::new(exact).is_ok());
    let one_over = (0..=MAX_COMMAND_ENVIRONMENT_ENTRIES)
        .map(|index| {
            (
                format!("NAME_{index}"),
                EnvironmentValue::inherit("PATH").expect("inherited PATH"),
            )
        })
        .collect();
    assert!(ChildEnvironment::new(one_over).is_err());

    let mut recursive_environment = BTreeMap::new();
    recursive_environment.insert("TOKEN".to_owned(), EnvironmentValue::secret(secret()));
    let recursive_helper = CommandSpec::new(
        vec!["helper".to_owned()],
        ChildEnvironment::new(recursive_environment).expect("shape-valid environment"),
    )
    .expect("shape-valid helper command");
    assert!(SecretReference::command(recursive_helper).is_err());

    let mut literal_environment = BTreeMap::new();
    literal_environment.insert(
        "MODE".to_owned(),
        EnvironmentValue::literal("credential").expect("literal"),
    );
    let helper = CommandSpec::new(
        vec!["helper".to_owned()],
        ChildEnvironment::new(literal_environment).expect("literal environment"),
    )
    .expect("helper command");
    assert!(SecretReference::command(helper).is_ok());
}

fn https_config(
    origins: impl IntoIterator<Item = HttpsOrigin>,
    grants: MutationGrants,
) -> Result<HttpsConfig, resourcefs_sources::ConfigurationError> {
    HttpsConfig::new("https-source", false, grants, origins.into_iter().collect())
}

#[test]
fn https_rejects_url_userinfo() {
    assert!(
        https_config(
            [HttpsOrigin::new(
                "https://user@example.test/api",
                false,
                None,
            )],
            MutationGrants::default(),
        )
        .is_err()
    );
}

#[test]
fn https_policy_matrix() {
    let origin = |base_url: &str| HttpsOrigin::new(base_url, false, None);
    for (name, origins, accepted) in [
        ("empty", vec![], false),
        ("single", vec![origin("https://example.test/api")], true),
        (
            "disjoint-component-boundary",
            vec![
                origin("https://example.test/api"),
                origin("https://example.test/apix"),
            ],
            true,
        ),
        (
            "same-path-different-host",
            vec![
                origin("https://one.example.test/api"),
                origin("https://two.example.test/api"),
            ],
            true,
        ),
        (
            "same-path-different-port",
            vec![
                origin("https://example.test/api"),
                origin("https://example.test:8443/api"),
            ],
            true,
        ),
        ("non-https", vec![origin("http://example.test/api")], false),
        (
            "userinfo",
            vec![origin("https://user@example.test/api")],
            false,
        ),
        (
            "password",
            vec![origin("https://user:secret@example.test/api")],
            false,
        ),
        (
            "query",
            vec![origin("https://example.test/api?mode=raw")],
            false,
        ),
        (
            "empty-query",
            vec![origin("https://example.test/api?")],
            false,
        ),
        (
            "fragment",
            vec![origin("https://example.test/api#part")],
            false,
        ),
        (
            "empty-fragment",
            vec![origin("https://example.test/api#")],
            false,
        ),
        (
            "wildcard-host",
            vec![origin("https://*.example.test/api")],
            false,
        ),
        (
            "duplicate-normalized",
            vec![
                origin("https://EXAMPLE.test:443/api/"),
                origin("https://example.test/api"),
            ],
            false,
        ),
        (
            "overlapping-prefix",
            vec![
                origin("https://example.test/api"),
                origin("https://example.test/api/v1"),
            ],
            false,
        ),
        (
            "overlap-hidden-by-byte-sort",
            vec![
                origin("https://example.test/api"),
                origin("https://example.test/api-x"),
                origin("https://example.test/api/v1"),
            ],
            false,
        ),
        (
            "root-overlap",
            vec![
                origin("https://example.test/"),
                origin("https://example.test/api"),
            ],
            false,
        ),
    ] {
        assert_eq!(
            https_config(origins, MutationGrants::default()).is_ok(),
            accepted,
            "HTTPS policy row {name}"
        );
    }

    assert!(
        https_config(
            [origin("https://example.test/")],
            MutationGrants::new(false, true, false)
        )
        .is_err(),
        "HTTPS must remain read-only"
    );

    for name in [
        "host",
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "content-length",
        "proxy-connection",
        "forwarded",
        "via",
        "x-forwarded-host",
        "x-forwarded-port",
        "x-forwarded-proto",
    ] {
        assert!(
            CredentialHeader::new(name, None, secret()).is_err(),
            "credential header {name} must be rejected"
        );
    }
    assert!(CredentialHeader::new("X-Api-Key", None, secret()).is_ok());
    assert!(CredentialHeader::new("Authorization", Some("Bearer".to_owned()), secret()).is_ok());
    assert!(CredentialHeader::new("Authorization", Some(String::new()), secret()).is_err());
    assert!(CredentialHeader::new("bad header", None, secret()).is_err());
}

fn github_repository(name: &str, grants: MutationGrants) -> GithubRepository {
    GithubRepository::new(name, grants).expect("valid GitHub repository identity")
}

fn github_config(
    grants: MutationGrants,
    repositories: impl IntoIterator<Item = GithubRepository>,
) -> Result<GithubConfig, resourcefs_sources::ConfigurationError> {
    GithubConfig::new(
        "github-source",
        false,
        grants,
        None,
        false,
        secret(),
        repositories.into_iter().collect(),
    )
}

#[test]
fn github_policy_matrix() {
    let none = MutationGrants::default();
    let create = MutationGrants::new(true, false, false);
    for invalid in [
        "/repository",
        "owner/",
        "owner/repository/extra",
        "./repository",
        "owner/repo name",
    ] {
        assert!(
            GithubRepository::new(invalid, none).is_err(),
            "invalid GitHub repository {invalid}"
        );
    }

    let canonical = github_repository("Owner/Repository", none);
    assert_eq!(canonical.identity().as_str(), "owner/repository");
    assert_eq!(canonical.grants(), none);
    let canonical_config =
        github_config(none, [canonical.clone()]).expect("canonical GitHub config");
    assert_eq!(canonical_config.id(), "github-source");
    assert!(!canonical_config.required());
    assert_eq!(canonical_config.grants(), none);
    assert_eq!(canonical_config.api_base_url(), "https://api.github.com/");
    assert!(!canonical_config.allow_private_network());
    assert_eq!(canonical_config.credential(), &secret());
    assert_eq!(canonical_config.repositories(), &[canonical.clone()]);
    assert_eq!(
        canonical_config
            .repository(canonical.identity())
            .map(GithubRepository::identity),
        Some(canonical.identity())
    );

    for (name, source_grants, repositories, accepted) in [
        (
            "default-public-api",
            none,
            vec![github_repository("owner/repository", none)],
            true,
        ),
        ("empty", none, vec![], false),
        (
            "duplicate",
            none,
            vec![
                github_repository("owner/repository", none),
                github_repository("owner/repository", none),
            ],
            false,
        ),
        (
            "case-fold-duplicate",
            none,
            vec![
                github_repository("Owner/Repository", none),
                github_repository("owner/repository", none),
            ],
            false,
        ),
        (
            "equal-nested-grants",
            create,
            vec![github_repository("owner/repository", create)],
            true,
        ),
        (
            "broader-nested-grants",
            create,
            vec![github_repository(
                "owner/repository",
                MutationGrants::new(false, true, false),
            )],
            false,
        ),
        (
            "nested-delete",
            MutationGrants::new(true, true, false),
            vec![github_repository(
                "owner/repository",
                MutationGrants::new(false, false, true),
            )],
            false,
        ),
    ] {
        assert_eq!(
            github_config(source_grants, repositories).is_ok(),
            accepted,
            "GitHub policy row {name}"
        );
    }

    assert!(
        github_config(
            MutationGrants::new(false, false, true),
            [github_repository("owner/repository", none)]
        )
        .is_err(),
        "GitHub delete must remain unsupported"
    );
    assert!(
        GithubConfig::new(
            "github-source",
            false,
            none,
            Some("https://github.example.test/api/v3".to_owned()),
            true,
            secret(),
            vec![github_repository("owner/repository", none)],
        )
        .is_ok()
    );
    assert!(
        GithubConfig::new(
            "github-source",
            false,
            none,
            Some("http://github.example.test/api/v3".to_owned()),
            false,
            secret(),
            vec![github_repository("owner/repository", none)],
        )
        .is_err()
    );
}

fn ssh_config(
    hosts: impl IntoIterator<Item = SshHost>,
    grants: MutationGrants,
) -> Result<SshConfig, resourcefs_sources::ConfigurationError> {
    SshConfig::new(
        "ssh-source",
        false,
        grants,
        command("ssh"),
        hosts.into_iter().collect(),
    )
}

#[test]
fn ssh_policy_matrix() {
    for (name, hosts, accepted) in [
        (
            "single",
            vec![SshHost::new("host", vec!["/srv".to_owned()])],
            true,
        ),
        (
            "filesystem-root",
            vec![SshHost::new("host", vec!["/".to_owned()])],
            true,
        ),
        (
            "root-overlap",
            vec![SshHost::new(
                "host",
                vec!["/".to_owned(), "/srv".to_owned()],
            )],
            false,
        ),
        (
            "component-disjoint",
            vec![SshHost::new(
                "host",
                vec!["/srv".to_owned(), "/srv2".to_owned()],
            )],
            true,
        ),
        (
            "same-root-different-host",
            vec![
                SshHost::new("one", vec!["/srv".to_owned()]),
                SshHost::new("two", vec!["/srv".to_owned()]),
            ],
            true,
        ),
        ("empty-hosts", vec![], false),
        (
            "empty-alias",
            vec![SshHost::new("", vec!["/srv".to_owned()])],
            false,
        ),
        (
            "pattern-alias",
            vec![SshHost::new("host*", vec!["/srv".to_owned()])],
            false,
        ),
        (
            "duplicate-alias",
            vec![
                SshHost::new("host", vec!["/srv/one".to_owned()]),
                SshHost::new("host", vec!["/srv/two".to_owned()]),
            ],
            false,
        ),
        ("empty-roots", vec![SshHost::new("host", vec![])], false),
        (
            "relative-root",
            vec![SshHost::new("host", vec!["srv".to_owned()])],
            false,
        ),
        (
            "trailing-slash",
            vec![SshHost::new("host", vec!["/srv/".to_owned()])],
            false,
        ),
        (
            "repeated-slash",
            vec![SshHost::new("host", vec!["/srv//data".to_owned()])],
            false,
        ),
        (
            "dot-component",
            vec![SshHost::new("host", vec!["/srv/./data".to_owned()])],
            false,
        ),
        (
            "parent-component",
            vec![SshHost::new("host", vec!["/srv/../data".to_owned()])],
            false,
        ),
        (
            "backslash",
            vec![SshHost::new("host", vec!["/srv\\data".to_owned()])],
            false,
        ),
        (
            "duplicate-root",
            vec![SshHost::new(
                "host",
                vec!["/srv".to_owned(), "/srv".to_owned()],
            )],
            false,
        ),
        (
            "overlapping-root",
            vec![SshHost::new(
                "host",
                vec!["/srv".to_owned(), "/srv/data".to_owned()],
            )],
            false,
        ),
        (
            "overlap-hidden-by-byte-sort",
            vec![SshHost::new(
                "host",
                vec![
                    "/srv".to_owned(),
                    "/srv-data".to_owned(),
                    "/srv/data".to_owned(),
                ],
            )],
            false,
        ),
    ] {
        assert_eq!(
            ssh_config(hosts, MutationGrants::default()).is_ok(),
            accepted,
            "SSH policy row {name}"
        );
    }
    assert!(
        ssh_config(
            [SshHost::new("host", vec!["/srv".to_owned()])],
            MutationGrants::new(true, false, false)
        )
        .is_err(),
        "SSH must remain read-only"
    );
}

fn downstream_config(
    servers: impl IntoIterator<Item = DownstreamServer>,
    grants: MutationGrants,
) -> Result<DownstreamMcpConfig, resourcefs_sources::ConfigurationError> {
    DownstreamMcpConfig::new(
        "downstream-source",
        false,
        grants,
        servers.into_iter().collect(),
    )
}

fn claim(value: &str) -> SchemeClaim {
    SchemeClaim::new(value.to_owned()).expect("valid test scheme")
}

#[test]
fn downstream_claims_are_case_insensitively_unique() {
    let stdio = || DownstreamTransport::stdio(command("resource-server"));
    assert!(
        downstream_config(
            [
                DownstreamServer::new("one", vec![claim("Docs")], stdio()),
                DownstreamServer::new("two", vec![claim("docs")], stdio()),
            ],
            MutationGrants::default(),
        )
        .is_err()
    );
}

#[test]
fn downstream_mcp_matrix() {
    let stdio = || DownstreamTransport::stdio(command("resource-server"));
    assert!(
        downstream_config(
            [DownstreamServer::new("docs", vec![claim("docs")], stdio())],
            MutationGrants::default(),
        )
        .is_ok()
    );
    assert!(
        downstream_config(
            [DownstreamServer::new(
                "remote",
                vec![claim("remote")],
                DownstreamTransport::http("https://example.test/mcp", false, None),
            )],
            MutationGrants::default(),
        )
        .is_ok()
    );

    for (name, servers, accepted) in [
        ("empty", vec![], false),
        (
            "empty-claims",
            vec![DownstreamServer::new("docs", vec![], stdio())],
            false,
        ),
        (
            "duplicate-server-id",
            vec![
                DownstreamServer::new("docs", vec![claim("docs")], stdio()),
                DownstreamServer::new("docs", vec![claim("other")], stdio()),
            ],
            false,
        ),
        (
            "duplicate-claim",
            vec![
                DownstreamServer::new("one", vec![claim("docs")], stdio()),
                DownstreamServer::new("two", vec![claim("docs")], stdio()),
            ],
            false,
        ),
        (
            "case-fold-claim",
            vec![
                DownstreamServer::new("one", vec![claim("Docs")], stdio()),
                DownstreamServer::new("two", vec![claim("docs")], stdio()),
            ],
            false,
        ),
        (
            "non-https-http",
            vec![DownstreamServer::new(
                "remote",
                vec![claim("remote")],
                DownstreamTransport::http("http://example.test/mcp", false, None),
            )],
            false,
        ),
        (
            "http-userinfo",
            vec![DownstreamServer::new(
                "remote",
                vec![claim("remote")],
                DownstreamTransport::http("https://user@example.test/mcp", false, None),
            )],
            false,
        ),
        (
            "http-query",
            vec![DownstreamServer::new(
                "remote",
                vec![claim("remote")],
                DownstreamTransport::http("https://example.test/mcp?x=1", false, None),
            )],
            false,
        ),
    ] {
        assert_eq!(
            downstream_config(servers, MutationGrants::default()).is_ok(),
            accepted,
            "downstream MCP row {name}"
        );
    }

    for built_in in [
        "rfs", "file", "artifact", "local", "https", "github", "issue", "pr", "ssh", "skill",
        "rule", "memory", "vault", "agent", "history",
    ] {
        assert!(
            downstream_config(
                [DownstreamServer::new(
                    "server",
                    vec![claim(built_in)],
                    stdio(),
                )],
                MutationGrants::default(),
            )
            .is_err(),
            "built-in scheme {built_in} must not be shadowed"
        );
    }

    assert!(
        downstream_config(
            [DownstreamServer::new("docs", vec![claim("docs")], stdio())],
            MutationGrants::new(false, true, false),
        )
        .is_err(),
        "downstream MCP must remain read-only"
    );

    let max = format!("a{}", "b".repeat(63));
    assert_eq!(max.len(), 64);
    assert!(SchemeClaim::new(max).is_ok());
    assert!(SchemeClaim::new(format!("a{}", "b".repeat(64))).is_err());
    for invalid in ["", "1docs", "doc_s", "doc/s", "döcs"] {
        assert!(
            SchemeClaim::new(invalid.to_owned()).is_err(),
            "scheme row {invalid:?}"
        );
    }
}

fn converter(
    extensions: &[&str],
    input: ConverterInput,
) -> Result<DocumentConverter, resourcefs_sources::ConfigurationError> {
    DocumentConverter::new(
        extensions
            .iter()
            .map(|extension| (*extension).to_owned())
            .collect(),
        input,
        command("converter"),
    )
}

fn documents_config(
    converters: Vec<DocumentConverter>,
    grants: MutationGrants,
) -> Result<DocumentsConfig, resourcefs_sources::ConfigurationError> {
    DocumentsConfig::new("documents-source", false, grants, converters)
}

#[test]
fn document_extensions_are_globally_unique() {
    // C11 named-mutation fence: removing the global overlap decision in
    // `DocumentsConfig::new` must flip this row.
    assert!(
        documents_config(
            vec![
                converter(&["md"], ConverterInput::Stdin).expect("valid converter"),
                converter(&["md"], ConverterInput::Path).expect("valid converter"),
            ],
            MutationGrants::default(),
        )
        .is_err()
    );
}

#[test]
fn document_converter_matrix() {
    for (name, extensions, input, accepted) in [
        ("stdin-single", vec!["md"], ConverterInput::Stdin, true),
        ("path-single", vec!["pdf"], ConverterInput::Path, true),
        (
            "multiple",
            vec!["md", "markdown", "mdown"],
            ConverterInput::Stdin,
            true,
        ),
        (
            "uppercase-spelling",
            vec!["MarkDown"],
            ConverterInput::Path,
            false,
        ),
        (
            "operator-punctuation",
            vec!["c++", "x-1_y"],
            ConverterInput::Stdin,
            true,
        ),
        ("empty-list", vec![], ConverterInput::Stdin, false),
        ("empty-extension", vec![""], ConverterInput::Stdin, false),
        ("leading-dot", vec![".md"], ConverterInput::Stdin, false),
        ("internal-dot", vec!["tar.gz"], ConverterInput::Stdin, false),
        ("slash", vec!["a/b"], ConverterInput::Stdin, false),
        ("backslash", vec!["a\\b"], ConverterInput::Stdin, false),
        ("wildcard", vec!["m*"], ConverterInput::Stdin, false),
        ("unicode", vec!["média"], ConverterInput::Stdin, false),
    ] {
        assert_eq!(
            converter(&extensions, input).is_ok(),
            accepted,
            "converter row {name}"
        );
    }

    let exact = "a".repeat(MAX_EXTENSION_BYTES);
    assert!(converter(&[exact.as_str()], ConverterInput::Stdin).is_ok());
    let over = "a".repeat(MAX_EXTENSION_BYTES + 1);
    assert!(converter(&[over.as_str()], ConverterInput::Stdin).is_err());

    let exact_list: Vec<String> = (0..4_096).map(|index| format!("e{index}")).collect();
    assert!(
        DocumentConverter::new(exact_list, ConverterInput::Stdin, command("converter")).is_ok()
    );
    let over_list: Vec<String> = (0..4_097).map(|index| format!("e{index}")).collect();
    assert!(
        DocumentConverter::new(over_list, ConverterInput::Stdin, command("converter")).is_err()
    );

    // No argv placeholder mechanism exists; brace spellings stay literal for
    // other command uses but are rejected on converter commands.
    let placeholder = CommandSpec::new(
        vec!["converter".to_owned(), "{path}".to_owned()],
        ChildEnvironment::default(),
    )
    .expect("literal braces remain valid generic argv");
    assert!(
        DocumentConverter::new(
            vec!["md".to_owned()],
            ConverterInput::Path,
            placeholder.clone(),
        )
        .is_err()
    );
    assert!(
        DocumentConverter::new(vec!["md".to_owned()], ConverterInput::Stdin, placeholder).is_err()
    );

    assert!(
        documents_config(
            vec![
                converter(&["md"], ConverterInput::Stdin).expect("valid converter"),
                converter(&["pdf"], ConverterInput::Path).expect("valid converter"),
                converter(&["epub"], ConverterInput::Stdin).expect("valid converter"),
            ],
            MutationGrants::default(),
        )
        .is_ok()
    );

    for (name, converters, accepted) in [
        ("empty-converters", vec![], false),
        (
            "duplicate-within-converter",
            vec![converter(&["md", "md"], ConverterInput::Stdin).expect("valid converter")],
            false,
        ),
        (
            "duplicate-across-converters",
            vec![
                converter(&["md", "markdown"], ConverterInput::Stdin).expect("valid converter"),
                converter(&["pdf", "md"], ConverterInput::Path).expect("valid converter"),
            ],
            false,
        ),
    ] {
        assert_eq!(
            documents_config(converters, MutationGrants::default()).is_ok(),
            accepted,
            "documents row {name}"
        );
    }

    for (operation, grants) in [
        ("create", MutationGrants::new(true, false, false)),
        ("update", MutationGrants::new(false, true, false)),
        ("delete", MutationGrants::new(false, false, true)),
    ] {
        assert!(
            documents_config(
                vec![converter(&["md"], ConverterInput::Stdin).expect("valid converter")],
                grants,
            )
            .is_err(),
            "documents must reject {operation} grants"
        );
    }

    let exact_converters: Vec<DocumentConverter> = (0..4_096)
        .map(|index| {
            let extension = format!("e{index}");
            converter(&[extension.as_str()], ConverterInput::Stdin).expect("valid converter")
        })
        .collect();
    assert!(documents_config(exact_converters, MutationGrants::default()).is_ok());
    let over_converters: Vec<DocumentConverter> = (0..4_097)
        .map(|index| {
            let extension = format!("e{index}");
            converter(&[extension.as_str()], ConverterInput::Stdin).expect("valid converter")
        })
        .collect();
    assert!(documents_config(over_converters, MutationGrants::default()).is_err());
}

struct MemoryVaultTree {
    _temporary: TempDir,
    directory: ConfigurationDirectory,
    base: PathBuf,
    outside: PathBuf,
}

impl MemoryVaultTree {
    fn absolute(&self, relative: &str) -> String {
        self.base.join(relative).to_string_lossy().into_owned()
    }

    fn outside_file(&self) -> String {
        self.outside
            .join("secret.md")
            .to_string_lossy()
            .into_owned()
    }

    fn outside_directory(&self) -> String {
        self.outside.to_string_lossy().into_owned()
    }
}

fn memory_vault_tree() -> MemoryVaultTree {
    let temporary = TempDir::new().expect("temporary configuration tree");
    let base = temporary.path().join("config");
    fs::create_dir(&base).expect("configuration base");
    fs::create_dir(base.join("memory")).expect("memory directory");
    fs::write(base.join("memory").join("notes.md"), "notes").expect("memory file");
    fs::create_dir(base.join("memory").join("archive")).expect("archive directory");
    fs::create_dir_all(base.join("vaults").join("personal")).expect("personal vault");
    fs::create_dir(base.join("vaults").join("team space")).expect("spaced vault");
    fs::create_dir(base.join("vaults").join("sécrets")).expect("unicode vault");
    fs::create_dir(base.join("exports")).expect("exports directory");
    fs::write(base.join("exports").join("agents.json"), "{}").expect("manifest file");
    fs::write(base.join("exports").join("more agents.json"), "{}").expect("spaced manifest");
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).expect("outside directory");
    fs::write(outside.join("secret.md"), "outside").expect("outside file");
    let directory = ConfigurationDirectory::new(&base).expect("configuration directory");
    MemoryVaultTree {
        _temporary: temporary,
        directory,
        base,
        outside,
    }
}

fn memory_config(
    tree: &MemoryVaultTree,
    roots: Vec<MemoryRoot>,
    grants: MutationGrants,
) -> Result<MemoryConfig, resourcefs_sources::ConfigurationError> {
    MemoryConfig::new("memory-source", false, grants, &tree.directory, roots)
}

fn vault_config(
    tree: &MemoryVaultTree,
    vaults: Vec<VaultRoot>,
    grants: MutationGrants,
) -> Result<VaultConfig, resourcefs_sources::ConfigurationError> {
    VaultConfig::new("vault-source", false, grants, &tree.directory, vaults)
}

fn agent_export_config(
    tree: &MemoryVaultTree,
    manifests: Vec<String>,
    grants: MutationGrants,
) -> Result<AgentExportConfig, resourcefs_sources::ConfigurationError> {
    AgentExportConfig::new(
        "agent-export-source",
        false,
        grants,
        &tree.directory,
        manifests,
    )
}

#[test]
fn memory_rejects_canonical_alias_targets() {
    // C14 named-mutation fence: deduplicating by name only in
    // `MemoryConfig::new` must flip this row.
    let tree = memory_vault_tree();
    assert!(
        memory_config(
            &tree,
            vec![
                MemoryRoot::new("notes", "memory/notes.md"),
                MemoryRoot::new("alias", "memory/./notes.md"),
            ],
            MutationGrants::default(),
        )
        .is_err(),
        "two spellings of one canonical target must fail"
    );
}

#[test]
fn memory_root_matrix() {
    let tree = memory_vault_tree();
    let none = MutationGrants::default();

    for (name, roots, accepted) in [
        (
            "single-file",
            vec![MemoryRoot::new("notes", "memory/notes.md")],
            true,
        ),
        (
            "single-directory",
            vec![MemoryRoot::new("archive", "memory/archive")],
            true,
        ),
        (
            "distinct-roots",
            vec![
                MemoryRoot::new("notes", "memory/notes.md"),
                MemoryRoot::new("archive", "memory/archive"),
            ],
            true,
        ),
        (
            "unicode-and-spaces",
            vec![
                MemoryRoot::new("unicode", "vaults/sécrets"),
                MemoryRoot::new("spaced", "exports/more agents.json"),
            ],
            true,
        ),
        (
            "case-variant-names",
            vec![
                MemoryRoot::new("Notes", "memory/notes.md"),
                MemoryRoot::new("notes", "memory/archive"),
            ],
            true,
        ),
        (
            "absolute-contained",
            vec![MemoryRoot::new("notes", tree.absolute("memory/notes.md"))],
            true,
        ),
        ("empty-roots", vec![], false),
        (
            "duplicate-name",
            vec![
                MemoryRoot::new("notes", "memory/notes.md"),
                MemoryRoot::new("notes", "memory/archive"),
            ],
            false,
        ),
        (
            "duplicate-target",
            vec![
                MemoryRoot::new("notes", "memory/notes.md"),
                MemoryRoot::new("copy", "memory/notes.md"),
            ],
            false,
        ),
        (
            "invalid-name",
            vec![MemoryRoot::new("bad name", "memory/notes.md")],
            false,
        ),
        (
            "empty-name",
            vec![MemoryRoot::new("", "memory/notes.md")],
            false,
        ),
        (
            "missing-target",
            vec![MemoryRoot::new("missing", "memory/missing.md")],
            false,
        ),
        (
            "escaping-relative",
            vec![MemoryRoot::new("escape", "../outside/secret.md")],
            false,
        ),
        (
            "absolute-outside",
            vec![MemoryRoot::new("outside", tree.outside_file())],
            false,
        ),
    ] {
        assert_eq!(
            memory_config(&tree, roots, none).is_ok(),
            accepted,
            "memory row {name}"
        );
    }

    for (operation, grants) in [
        ("create", MutationGrants::new(true, false, false)),
        ("update", MutationGrants::new(false, true, false)),
        ("delete", MutationGrants::new(false, false, true)),
    ] {
        assert!(
            memory_config(
                &tree,
                vec![MemoryRoot::new("notes", "memory/notes.md")],
                grants,
            )
            .is_err(),
            "memory must reject {operation} grants"
        );
    }
}

#[cfg(unix)]
#[test]
fn memory_symlink_targets_stay_contained() {
    let tree = memory_vault_tree();
    let notes = tree.base.join("memory").join("notes.md");
    std::os::unix::fs::symlink(&notes, tree.base.join("alias.md")).expect("contained alias");
    std::os::unix::fs::symlink(tree.outside.join("secret.md"), tree.base.join("escape.md"))
        .expect("escaping alias");

    assert!(
        memory_config(
            &tree,
            vec![MemoryRoot::new("linked", "alias.md")],
            MutationGrants::default(),
        )
        .is_ok(),
        "a contained symlink target must be admitted"
    );
    assert!(
        memory_config(
            &tree,
            vec![
                MemoryRoot::new("notes", "memory/notes.md"),
                MemoryRoot::new("linked", "alias.md"),
            ],
            MutationGrants::default(),
        )
        .is_err(),
        "symlink aliases of one canonical target must fail"
    );
    assert!(
        memory_config(
            &tree,
            vec![MemoryRoot::new("escape", "escape.md")],
            MutationGrants::default(),
        )
        .is_err(),
        "symlink targets outside the configuration directory must fail"
    );
}

#[test]
fn memory_roots_enforce_entry_ceiling() {
    let tree = memory_vault_tree();
    let none = MutationGrants::default();
    let many = tree.base.join("many");
    fs::create_dir(&many).expect("many directory");
    let exact: Vec<MemoryRoot> = (0..MAX_CONFIGURATION_ENTRIES)
        .map(|index| {
            let file = format!("f{index}");
            fs::write(many.join(&file), "").expect("exact-ceiling file");
            MemoryRoot::new(file.clone(), format!("many/{file}"))
        })
        .collect();
    assert!(
        memory_config(&tree, exact, none).is_ok(),
        "exact-ceiling memory roots must be accepted"
    );

    let over: Vec<MemoryRoot> = (0..=MAX_CONFIGURATION_ENTRIES)
        .map(|index| MemoryRoot::new(format!("f{index}"), "missing"))
        .collect();
    assert!(
        memory_config(&tree, over, none).is_err(),
        "one-over memory roots must be rejected"
    );
}

#[test]
fn vault_rejects_grants_broader_than_source() {
    // C15 named-mutation fence: dropping the per-vault subset decision in
    // `VaultConfig::new` must flip this row.
    let tree = memory_vault_tree();
    assert!(
        vault_config(
            &tree,
            vec![VaultRoot::new(
                "personal",
                "vaults/personal",
                MutationGrants::new(false, true, false),
            )],
            MutationGrants::new(true, false, false),
        )
        .is_err(),
        "per-vault grants broader than source grants must fail"
    );
}

#[test]
fn vault_matrix() {
    let tree = memory_vault_tree();
    let none = MutationGrants::default();
    let create = MutationGrants::new(true, false, false);
    let update = MutationGrants::new(false, true, false);
    let full = MutationGrants::new(true, true, true);

    for (name, vaults, source_grants, accepted) in [
        (
            "single",
            vec![VaultRoot::new("personal", "vaults/personal", none)],
            none,
            true,
        ),
        (
            "distinct-vaults",
            vec![
                VaultRoot::new("personal", "vaults/personal", none),
                VaultRoot::new("team", "vaults/team space", none),
            ],
            none,
            true,
        ),
        (
            "unicode",
            vec![VaultRoot::new("secrets", "vaults/sécrets", none)],
            none,
            true,
        ),
        (
            "case-variant-names",
            vec![
                VaultRoot::new("Personal", "vaults/personal", none),
                VaultRoot::new("personal", "vaults/team space", none),
            ],
            none,
            true,
        ),
        (
            "absolute-contained",
            vec![VaultRoot::new(
                "personal",
                tree.absolute("vaults/personal"),
                none,
            )],
            none,
            true,
        ),
        (
            "full-source-and-vault-grants",
            vec![VaultRoot::new("personal", "vaults/personal", full)],
            full,
            true,
        ),
        (
            "subset-vault-grants",
            vec![VaultRoot::new("personal", "vaults/personal", create)],
            full,
            true,
        ),
        (
            "equal-vault-grants",
            vec![VaultRoot::new("personal", "vaults/personal", create)],
            create,
            true,
        ),
        (
            "independent-per-vault-grants",
            vec![
                VaultRoot::new("personal", "vaults/personal", create),
                VaultRoot::new("team", "vaults/team space", update),
                VaultRoot::new("secrets", "vaults/sécrets", none),
            ],
            full,
            true,
        ),
        ("empty-vaults", vec![], none, false),
        (
            "duplicate-name",
            vec![
                VaultRoot::new("personal", "vaults/personal", none),
                VaultRoot::new("personal", "vaults/team space", none),
            ],
            none,
            false,
        ),
        (
            "duplicate-directory",
            vec![
                VaultRoot::new("personal", "vaults/personal", none),
                VaultRoot::new("copy", "vaults/personal", none),
            ],
            none,
            false,
        ),
        (
            "dot-spelling-alias",
            vec![
                VaultRoot::new("personal", "vaults/personal", none),
                VaultRoot::new("copy", "vaults/./personal", none),
            ],
            none,
            false,
        ),
        (
            "invalid-name",
            vec![VaultRoot::new("bad name", "vaults/personal", none)],
            none,
            false,
        ),
        (
            "file-target",
            vec![VaultRoot::new("file", "memory/notes.md", none)],
            none,
            false,
        ),
        (
            "missing-target",
            vec![VaultRoot::new("missing", "vaults/missing", none)],
            none,
            false,
        ),
        (
            "escaping-relative",
            vec![VaultRoot::new("escape", "../outside", none)],
            none,
            false,
        ),
        (
            "absolute-outside",
            vec![VaultRoot::new("outside", tree.outside_directory(), none)],
            none,
            false,
        ),
        (
            "superset-vault-grants",
            vec![VaultRoot::new("personal", "vaults/personal", full)],
            create,
            false,
        ),
        (
            "ungranted-vault-operation",
            vec![VaultRoot::new("personal", "vaults/personal", create)],
            none,
            false,
        ),
    ] {
        assert_eq!(
            vault_config(&tree, vaults, source_grants).is_ok(),
            accepted,
            "vault row {name}"
        );
    }
}

#[cfg(unix)]
#[test]
fn vault_symlink_directories_stay_contained() {
    let tree = memory_vault_tree();
    let personal = tree.base.join("vaults").join("personal");
    std::os::unix::fs::symlink(&personal, tree.base.join("alias-vault")).expect("contained alias");
    std::os::unix::fs::symlink(&tree.outside, tree.base.join("escape-vault"))
        .expect("escaping alias");

    assert!(
        vault_config(
            &tree,
            vec![VaultRoot::new(
                "linked",
                "alias-vault",
                MutationGrants::default()
            )],
            MutationGrants::default(),
        )
        .is_ok(),
        "a contained symlink target must be admitted"
    );
    assert!(
        vault_config(
            &tree,
            vec![
                VaultRoot::new("personal", "vaults/personal", MutationGrants::default()),
                VaultRoot::new("linked", "alias-vault", MutationGrants::default()),
            ],
            MutationGrants::default(),
        )
        .is_err(),
        "symlink aliases of one canonical directory must fail"
    );
    assert!(
        vault_config(
            &tree,
            vec![VaultRoot::new(
                "escape",
                "escape-vault",
                MutationGrants::default()
            )],
            MutationGrants::default(),
        )
        .is_err(),
        "symlink targets outside the configuration directory must fail"
    );
}

#[test]
fn vaults_enforce_entry_ceiling() {
    let tree = memory_vault_tree();
    let none = MutationGrants::default();
    let many = tree.base.join("many-vaults");
    fs::create_dir(&many).expect("many-vaults directory");
    let exact: Vec<VaultRoot> = (0..MAX_CONFIGURATION_ENTRIES)
        .map(|index| {
            let directory = format!("v{index}");
            fs::create_dir(many.join(&directory)).expect("exact-ceiling vault");
            VaultRoot::new(directory.clone(), format!("many-vaults/{directory}"), none)
        })
        .collect();
    assert!(
        vault_config(&tree, exact, none).is_ok(),
        "exact-ceiling vaults must be accepted"
    );

    let over: Vec<VaultRoot> = (0..=MAX_CONFIGURATION_ENTRIES)
        .map(|index| VaultRoot::new(format!("v{index}"), "missing", none))
        .collect();
    assert!(
        vault_config(&tree, over, none).is_err(),
        "one-over vaults must be rejected"
    );
}

#[test]
fn agent_export_rejects_true_grants() {
    // C16 named-mutation fence: accepting a true grant in
    // `AgentExportConfig::new` must flip this row.
    let tree = memory_vault_tree();
    let manifests = || vec!["exports/agents.json".to_owned()];
    for (operation, grants) in [
        ("create", MutationGrants::new(true, false, false)),
        ("update", MutationGrants::new(false, true, false)),
        ("delete", MutationGrants::new(false, false, true)),
    ] {
        assert!(
            agent_export_config(&tree, manifests(), grants).is_err(),
            "agent export must reject {operation} grants"
        );
    }
    assert!(
        agent_export_config(&tree, manifests(), MutationGrants::default()).is_ok(),
        "absent grants keep agent export read-only"
    );
}

#[test]
fn agent_export_path_matrix() {
    let tree = memory_vault_tree();
    let none = MutationGrants::default();
    let manifests = |paths: &[&str]| paths.iter().map(|path| (*path).to_owned()).collect();

    for (name, paths, accepted) in [
        ("single", vec!["exports/agents.json"], true),
        (
            "distinct-manifests",
            vec!["exports/agents.json", "exports/more agents.json"],
            true,
        ),
        ("unicode-target", vec!["exports/more agents.json"], true),
        ("empty-manifests", vec![], false),
        (
            "duplicate-manifest",
            vec!["exports/agents.json", "exports/agents.json"],
            false,
        ),
        (
            "dot-spelling-alias",
            vec!["exports/agents.json", "exports/./agents.json"],
            false,
        ),
        ("missing-manifest", vec!["exports/missing.json"], false),
        ("directory-target", vec!["exports"], false),
        ("escaping-relative", vec!["../outside/secret.md"], false),
    ] {
        assert_eq!(
            agent_export_config(&tree, manifests(&paths), none).is_ok(),
            accepted,
            "agent export row {name}"
        );
    }

    assert!(
        agent_export_config(&tree, vec![tree.absolute("exports/agents.json")], none,).is_ok(),
        "absolute contained manifests must be admitted"
    );
    assert!(
        agent_export_config(&tree, vec![tree.outside_file()], none).is_err(),
        "absolute manifests outside the configuration directory must fail"
    );
}

#[cfg(unix)]
#[test]
fn agent_export_symlink_manifests_stay_contained() {
    let tree = memory_vault_tree();
    let manifest = tree.base.join("exports").join("agents.json");
    std::os::unix::fs::symlink(&manifest, tree.base.join("alias.json")).expect("contained alias");
    std::os::unix::fs::symlink(
        tree.outside.join("secret.md"),
        tree.base.join("escape.json"),
    )
    .expect("escaping alias");
    let none = MutationGrants::default();

    assert!(
        agent_export_config(&tree, vec!["alias.json".to_owned()], none).is_ok(),
        "a contained symlink target must be admitted"
    );
    assert!(
        agent_export_config(
            &tree,
            vec!["exports/agents.json".to_owned(), "alias.json".to_owned()],
            none,
        )
        .is_err(),
        "symlink aliases of one canonical manifest must fail"
    );
    assert!(
        agent_export_config(&tree, vec!["escape.json".to_owned()], none).is_err(),
        "symlink targets outside the configuration directory must fail"
    );
}

#[test]
fn agent_export_manifests_enforce_entry_ceiling() {
    let tree = memory_vault_tree();
    let none = MutationGrants::default();
    let many = tree.base.join("many-exports");
    fs::create_dir(&many).expect("many-exports directory");
    let exact: Vec<String> = (0..MAX_CONFIGURATION_ENTRIES)
        .map(|index| {
            let file = format!("m{index}.json");
            fs::write(many.join(&file), "{}").expect("exact-ceiling manifest");
            format!("many-exports/{file}")
        })
        .collect();
    assert!(
        agent_export_config(&tree, exact, none).is_ok(),
        "exact-ceiling manifests must be accepted"
    );

    let over: Vec<String> = (0..=MAX_CONFIGURATION_ENTRIES)
        .map(|index| format!("missing-{index}.json"))
        .collect();
    assert!(
        agent_export_config(&tree, over, none).is_err(),
        "one-over manifests must be rejected"
    );
}

fn configuration_tree() -> (TempDir, PathBuf) {
    let temporary = TempDir::new().expect("temporary directory");
    let base = temporary.path().join("config");
    fs::create_dir(&base).expect("configuration base directory");
    (temporary, base)
}

#[test]
fn configuration_directory_requires_an_existing_directory() {
    let temporary = TempDir::new().expect("temporary directory");
    let file = temporary.path().join("file.txt");
    fs::write(&file, "content").expect("file fixture");

    assert!(ConfigurationDirectory::new(temporary.path()).is_ok());
    assert!(ConfigurationDirectory::new(temporary.path().join("missing")).is_err());
    assert!(ConfigurationDirectory::new(&file).is_err());
}

#[test]
fn configuration_directory_resolves_contained_paths() {
    let (temporary, base_path) = configuration_tree();
    fs::create_dir(base_path.join("inner")).expect("inner directory");
    fs::create_dir_all(base_path.join("nested/deep")).expect("nested directory");
    fs::write(base_path.join("inner/manifest.toml"), "rules").expect("manifest fixture");
    fs::create_dir(base_path.join("spaced root")).expect("spaced directory");
    fs::create_dir(base_path.join("ünïcode")).expect("unicode directory");
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).expect("outside directory");
    fs::write(outside.join("sentinel.txt"), "sentinel").expect("outside sentinel");

    let base = ConfigurationDirectory::new(&base_path).expect("configuration base");
    assert!(base.path().is_absolute());
    assert_eq!(base.path().file_name(), Some("config".as_ref()));

    let absolute = |path: PathBuf| {
        path.into_os_string()
            .into_string()
            .expect("UTF-8 fixture path")
    };
    for (name, input, expected, accepted) in [
        (
            "empty",
            String::new(),
            ConfigurationTargetKind::Directory,
            false,
        ),
        (
            "directory",
            "inner".to_owned(),
            ConfigurationTargetKind::Directory,
            true,
        ),
        (
            "nested-directory",
            "nested/deep".to_owned(),
            ConfigurationTargetKind::Directory,
            true,
        ),
        (
            "dot-spelling",
            "./inner".to_owned(),
            ConfigurationTargetKind::Directory,
            true,
        ),
        (
            "normalizing-spelling",
            "nested/../inner".to_owned(),
            ConfigurationTargetKind::Directory,
            true,
        ),
        (
            "base-directory",
            ".".to_owned(),
            ConfigurationTargetKind::Directory,
            true,
        ),
        (
            "file",
            "inner/manifest.toml".to_owned(),
            ConfigurationTargetKind::File,
            true,
        ),
        (
            "file-or-directory-file",
            "inner/manifest.toml".to_owned(),
            ConfigurationTargetKind::FileOrDirectory,
            true,
        ),
        (
            "file-or-directory-directory",
            "inner".to_owned(),
            ConfigurationTargetKind::FileOrDirectory,
            true,
        ),
        (
            "file-where-directory",
            "inner/manifest.toml".to_owned(),
            ConfigurationTargetKind::Directory,
            false,
        ),
        (
            "directory-where-file",
            "inner".to_owned(),
            ConfigurationTargetKind::File,
            false,
        ),
        (
            "missing",
            "missing".to_owned(),
            ConfigurationTargetKind::FileOrDirectory,
            false,
        ),
        (
            "relative-escape",
            "../outside".to_owned(),
            ConfigurationTargetKind::Directory,
            false,
        ),
        (
            "absolute-contained",
            absolute(base_path.join("inner")),
            ConfigurationTargetKind::Directory,
            true,
        ),
        (
            "absolute-outside",
            absolute(outside.clone()),
            ConfigurationTargetKind::Directory,
            false,
        ),
        (
            "spaces",
            "spaced root".to_owned(),
            ConfigurationTargetKind::Directory,
            true,
        ),
        (
            "unicode",
            "ünïcode".to_owned(),
            ConfigurationTargetKind::Directory,
            true,
        ),
    ] {
        assert_eq!(
            base.resolve(&input, expected).is_ok(),
            accepted,
            "contained path row {name}"
        );
    }

    assert_eq!(
        fs::read(outside.join("sentinel.txt")).expect("sentinel readable"),
        b"sentinel",
        "sibling sentinel must remain untouched"
    );
}

#[cfg(unix)]
#[test]
fn configuration_directory_follows_only_contained_links() {
    use std::os::unix::fs::symlink;

    let temporary = TempDir::new().expect("temporary directory");
    let real_base = temporary.path().join("real-config");
    fs::create_dir(&real_base).expect("real base directory");
    fs::create_dir(real_base.join("inner")).expect("inner directory");
    fs::write(real_base.join("inner/file.txt"), "content").expect("file fixture");
    let linked_base = temporary.path().join("config");
    symlink(&real_base, &linked_base).expect("base symlink");

    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).expect("outside directory");
    fs::write(outside.join("secret.txt"), "secret").expect("outside sentinel");
    symlink(real_base.join("inner"), real_base.join("contained-link")).expect("contained link");
    symlink(&outside, real_base.join("escaping-link")).expect("escaping link");
    symlink(outside.join("secret.txt"), real_base.join("escaping-file"))
        .expect("escaping file link");
    symlink(
        real_base.join("missing-target"),
        real_base.join("dangling-link"),
    )
    .expect("dangling link");

    let base = ConfigurationDirectory::new(&linked_base).expect("symlinked base");
    assert_eq!(
        base.path(),
        std::fs::canonicalize(&real_base)
            .expect("canonical real base")
            .as_path(),
        "the base must canonicalize through its symlink"
    );
    assert!(
        base.resolve("contained-link", ConfigurationTargetKind::Directory)
            .is_ok(),
        "a link whose target stays contained must resolve"
    );
    assert!(
        base.resolve("escaping-link", ConfigurationTargetKind::Directory)
            .is_err(),
        "a link escaping the base must be rejected"
    );
    assert!(
        base.resolve("escaping-file", ConfigurationTargetKind::File)
            .is_err(),
        "a file link escaping the base must be rejected"
    );
    assert!(
        base.resolve("dangling-link", ConfigurationTargetKind::FileOrDirectory)
            .is_err(),
        "a dangling link must be rejected"
    );
}

#[test]
fn configuration_errors_are_bounded_and_hide_paths() {
    let (temporary, base_path) = configuration_tree();
    fs::create_dir(temporary.path().join("outside")).expect("outside directory");
    let canonical = std::fs::canonicalize(&base_path)
        .expect("canonical base")
        .to_string_lossy()
        .into_owned();
    let probe = ConfigurationDirectory::new(base_path.join("missing"))
        .expect_err("missing base must fail")
        .to_string();
    assert!(probe.len() <= 128, "base error must be bounded: {probe}");
    assert!(
        !probe.contains(canonical.as_str()),
        "base error must not disclose the canonical path: {probe}"
    );

    let base = ConfigurationDirectory::new(&base_path).expect("configuration base");
    for (name, result) in [
        (
            "missing",
            base.resolve("missing", ConfigurationTargetKind::FileOrDirectory),
        ),
        (
            "escape",
            base.resolve("../outside", ConfigurationTargetKind::FileOrDirectory),
        ),
        (
            "wrong-kind",
            base.resolve(".", ConfigurationTargetKind::File),
        ),
    ] {
        let message = match result {
            Ok(_) => panic!("{name} row must fail"),
            Err(error) => error.to_string(),
        };
        assert!(
            message.len() <= 128,
            "{name} error must be bounded: {message}"
        );
        assert!(
            !message.contains(canonical.as_str()),
            "{name} error must not disclose the canonical path: {message}"
        );
    }
}

fn skills_config(
    base: &ConfigurationDirectory,
    roots: Vec<String>,
) -> Result<SkillsConfig, resourcefs_sources::ConfigurationError> {
    SkillsConfig::new(
        "skills-source",
        false,
        MutationGrants::default(),
        base,
        roots,
    )
}

#[test]
fn skill_root_matrix() {
    let (temporary, base_path) = configuration_tree();
    fs::create_dir(base_path.join("alpha")).expect("alpha root");
    fs::create_dir_all(base_path.join("nested/beta")).expect("beta root");
    fs::create_dir(base_path.join("spaced root")).expect("spaced root");
    fs::create_dir(base_path.join("ünïcode")).expect("unicode root");
    fs::write(base_path.join("file.txt"), "not a directory").expect("file fixture");
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).expect("outside directory");

    let base = ConfigurationDirectory::new(&base_path).expect("configuration base");
    let absolute = |path: PathBuf| {
        path.into_os_string()
            .into_string()
            .expect("UTF-8 fixture path")
    };

    for (name, roots, accepted) in [
        ("single", vec!["alpha".to_owned()], true),
        (
            "multiple-distinct",
            vec!["alpha".to_owned(), "nested/beta".to_owned()],
            true,
        ),
        ("empty", vec![], false),
        (
            "duplicate",
            vec!["alpha".to_owned(), "alpha".to_owned()],
            false,
        ),
        (
            "canonical-dot-alias",
            vec!["alpha".to_owned(), "./alpha".to_owned()],
            false,
        ),
        (
            "canonical-normalizing-alias",
            vec!["nested/beta".to_owned(), "nested/../nested/beta".to_owned()],
            false,
        ),
        ("missing", vec!["missing".to_owned()], false),
        ("file-not-directory", vec!["file.txt".to_owned()], false),
        (
            "absolute-contained",
            vec![absolute(base_path.join("alpha"))],
            true,
        ),
        ("absolute-outside", vec![absolute(outside.clone())], false),
        ("relative-escape", vec!["../outside".to_owned()], false),
        ("spaces", vec!["spaced root".to_owned()], true),
        ("unicode", vec!["ünïcode".to_owned()], true),
        ("base-directory", vec![".".to_owned()], true),
    ] {
        assert_eq!(
            skills_config(&base, roots).is_ok(),
            accepted,
            "skill root row {name}"
        );
    }

    assert!(
        SkillsConfig::new(
            "skills-source",
            true,
            MutationGrants::new(true, true, true),
            &base,
            vec!["alpha".to_owned()],
        )
        .is_ok(),
        "skills support full source-level grants"
    );
    assert!(
        SkillsConfig::new(
            "bad id",
            false,
            MutationGrants::default(),
            &base,
            vec!["alpha".to_owned()],
        )
        .is_err(),
        "skills ID must use configuration-ID syntax"
    );
}

#[cfg(unix)]
#[test]
fn skill_roots_reject_canonical_aliases() {
    use std::os::unix::fs::symlink;

    let (_temporary, base_path) = configuration_tree();
    let target = base_path.join("skills-real");
    fs::create_dir(&target).expect("real skill root");
    symlink(&target, base_path.join("skills-alias")).expect("alias symlink");
    let base = ConfigurationDirectory::new(&base_path).expect("configuration base");

    assert!(
        skills_config(&base, vec!["skills-alias".to_owned()]).is_ok(),
        "a contained alias alone resolves to its canonical target"
    );
    assert!(
        skills_config(
            &base,
            vec!["skills-real".to_owned(), "skills-alias".to_owned()]
        )
        .is_err(),
        "C12: two spellings of one canonical root must collide"
    );
}

#[test]
fn skill_roots_enforce_entry_ceiling() {
    let (_temporary, base_path) = configuration_tree();
    let names: Vec<String> = (0..MAX_CONFIGURATION_ENTRIES)
        .map(|index| format!("root-{index}"))
        .collect();
    for name in &names {
        fs::create_dir(base_path.join(name)).expect("ceiling root");
    }
    let base = ConfigurationDirectory::new(&base_path).expect("configuration base");

    assert!(
        skills_config(&base, names.clone()).is_ok(),
        "the exact entry ceiling must succeed"
    );
    let mut over = names;
    over.push("root-0".to_owned());
    assert!(
        skills_config(&base, over).is_err(),
        "one over the entry ceiling must fail even when every entry resolves"
    );
}

fn rules_config(
    base: &ConfigurationDirectory,
    manifests: Vec<String>,
) -> Result<RulesConfig, resourcefs_sources::ConfigurationError> {
    RulesConfig::new(
        "rules-source",
        false,
        MutationGrants::default(),
        base,
        manifests,
    )
}

#[test]
fn rule_path_matrix() {
    let (temporary, base_path) = configuration_tree();
    fs::write(base_path.join("rules-a.toml"), "rules a").expect("first manifest");
    fs::create_dir(base_path.join("nested")).expect("nested directory");
    fs::write(base_path.join("nested/rules-b.toml"), "rules b").expect("second manifest");
    fs::write(base_path.join("spaced manifest.toml"), "spaced").expect("spaced manifest");
    fs::write(base_path.join("ünïcode.toml"), "unicode").expect("unicode manifest");
    fs::create_dir(base_path.join("subdir")).expect("subdirectory fixture");
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).expect("outside directory");
    let sentinel = outside.join("sentinel-rules.toml");
    fs::write(&sentinel, "sibling sentinel").expect("outside sentinel");

    let base = ConfigurationDirectory::new(&base_path).expect("configuration base");
    let absolute = |path: PathBuf| {
        path.into_os_string()
            .into_string()
            .expect("UTF-8 fixture path")
    };

    for (name, manifests, accepted) in [
        ("single", vec!["rules-a.toml".to_owned()], true),
        (
            "multiple-distinct",
            vec!["rules-a.toml".to_owned(), "nested/rules-b.toml".to_owned()],
            true,
        ),
        ("empty", vec![], false),
        (
            "duplicate",
            vec!["rules-a.toml".to_owned(), "rules-a.toml".to_owned()],
            false,
        ),
        (
            "canonical-dot-alias",
            vec!["rules-a.toml".to_owned(), "./rules-a.toml".to_owned()],
            false,
        ),
        ("missing", vec!["missing.toml".to_owned()], false),
        ("directory-not-file", vec!["subdir".to_owned()], false),
        (
            "absolute-contained",
            vec![absolute(base_path.join("rules-a.toml"))],
            true,
        ),
        ("absolute-outside", vec![absolute(sentinel.clone())], false),
        (
            "relative-escape",
            vec!["../outside/sentinel-rules.toml".to_owned()],
            false,
        ),
        ("spaces", vec!["spaced manifest.toml".to_owned()], true),
        ("unicode", vec!["ünïcode.toml".to_owned()], true),
    ] {
        assert_eq!(
            rules_config(&base, manifests).is_ok(),
            accepted,
            "rule path row {name}"
        );
    }

    assert_eq!(
        fs::read(&sentinel).expect("sentinel readable"),
        b"sibling sentinel",
        "sibling sentinel must remain untouched"
    );
    assert!(
        RulesConfig::new(
            "rules-source",
            true,
            MutationGrants::new(true, true, true),
            &base,
            vec!["rules-a.toml".to_owned()],
        )
        .is_ok(),
        "rules support full source-level grants"
    );
    assert!(
        RulesConfig::new(
            "bad id",
            false,
            MutationGrants::default(),
            &base,
            vec!["rules-a.toml".to_owned()],
        )
        .is_err(),
        "rules ID must use configuration-ID syntax"
    );
}

#[cfg(unix)]
#[test]
fn rule_manifests_reject_escaping_links() {
    use std::os::unix::fs::symlink;

    let temporary = TempDir::new().expect("temporary directory");
    let base_path = temporary.path().join("config");
    fs::create_dir(&base_path).expect("configuration base");
    let sibling = temporary.path().join("sibling");
    fs::create_dir(&sibling).expect("sibling directory");
    let sentinel = sibling.join("sentinel-rules.toml");
    fs::write(&sentinel, "sibling sentinel").expect("sibling sentinel");
    symlink(&sentinel, base_path.join("escape.toml")).expect("escaping link");

    let base = ConfigurationDirectory::new(&base_path).expect("configuration base");
    assert!(
        rules_config(&base, vec!["escape.toml".to_owned()]).is_err(),
        "C13: a manifest link escaping the configuration directory must be rejected"
    );
    assert_eq!(
        fs::read(&sentinel).expect("sentinel readable"),
        b"sibling sentinel",
        "sibling sentinel must remain untouched"
    );

    fs::write(base_path.join("contained.toml"), "contained rules").expect("contained manifest");
    assert!(
        rules_config(&base, vec!["contained.toml".to_owned()]).is_ok(),
        "a contained manifest remains accepted"
    );
}

#[test]
fn rule_manifests_enforce_entry_ceiling() {
    let (_temporary, base_path) = configuration_tree();
    let names: Vec<String> = (0..MAX_CONFIGURATION_ENTRIES)
        .map(|index| format!("manifest-{index}.toml"))
        .collect();
    for name in &names {
        fs::write(base_path.join(name), "rules").expect("ceiling manifest");
    }
    let base = ConfigurationDirectory::new(&base_path).expect("configuration base");

    assert!(
        rules_config(&base, names.clone()).is_ok(),
        "the exact entry ceiling must succeed"
    );
    let mut over = names;
    over.push("manifest-0.toml".to_owned());
    assert!(
        rules_config(&base, over).is_err(),
        "one over the entry ceiling must fail even when every entry resolves"
    );
}
