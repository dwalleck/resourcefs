use std::collections::BTreeMap;

use resourcefs_sources::{
    ChildEnvironment, CommandSpec, CredentialHeader, DownstreamMcpConfig, DownstreamServer,
    DownstreamTransport, EnvironmentValue, GithubConfig, GithubRepository, HttpsConfig,
    HttpsOrigin, MAX_COMMAND_ARGUMENT_BYTES, MAX_COMMAND_ARGUMENTS,
    MAX_COMMAND_ENVIRONMENT_ENTRIES, MAX_CONFIGURATION_ID_BYTES, MutationGrants, MutationSupport,
    SchemeClaim, SecretReference, SshConfig, SshHost, validate_configuration_id,
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
    for (name, source_grants, repositories, accepted) in [
        (
            "default-public-api",
            none,
            vec![GithubRepository::new("owner/repository", none)],
            true,
        ),
        (
            "case-preserving-single",
            none,
            vec![GithubRepository::new("Owner/Repository", none)],
            true,
        ),
        ("empty", none, vec![], false),
        (
            "missing-owner",
            none,
            vec![GithubRepository::new("/repository", none)],
            false,
        ),
        (
            "missing-repository",
            none,
            vec![GithubRepository::new("owner/", none)],
            false,
        ),
        (
            "extra-segment",
            none,
            vec![GithubRepository::new("owner/repository/extra", none)],
            false,
        ),
        (
            "dot-owner",
            none,
            vec![GithubRepository::new("./repository", none)],
            false,
        ),
        (
            "whitespace",
            none,
            vec![GithubRepository::new("owner/repo name", none)],
            false,
        ),
        (
            "duplicate",
            none,
            vec![
                GithubRepository::new("owner/repository", none),
                GithubRepository::new("owner/repository", none),
            ],
            false,
        ),
        (
            "case-fold-duplicate",
            none,
            vec![
                GithubRepository::new("Owner/Repository", none),
                GithubRepository::new("owner/repository", none),
            ],
            false,
        ),
        (
            "equal-nested-grants",
            create,
            vec![GithubRepository::new("owner/repository", create)],
            true,
        ),
        (
            "broader-nested-grants",
            create,
            vec![GithubRepository::new(
                "owner/repository",
                MutationGrants::new(false, true, false),
            )],
            false,
        ),
        (
            "nested-delete",
            MutationGrants::new(true, true, false),
            vec![GithubRepository::new(
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
            [GithubRepository::new("owner/repository", none)]
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
            vec![GithubRepository::new("owner/repository", none)],
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
            vec![GithubRepository::new("owner/repository", none)],
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
