#![no_main]

//! Structured companion to the `server_profile` byte target.
//!
//! The byte target explores the *encoding*: it proves parsing is deterministic
//! and that `MAX_PROFILE_BYTES` refuses oversized input. What it does not do is
//! get past the decoder. Of the 3,912 corpus entries it had accumulated, 98
//! parsed as JSON, 48 carried a `schemaVersion`, and none at all carried a
//! `sources` array -- so `validate_profile`, `StaticSource::from_profile` and
//! `convert_sources` had never seen a fuzzed input.
//!
//! This target starts from a grammar instead of from bytes. Every generated
//! document is shaped like a profile -- correct tag names, required fields
//! present -- so the fuzzer spends its budget on *semantics*: duplicate source
//! identifiers, grants a source kind refuses, path collisions, cardinality
//! ceilings, credential shapes. The values inside that shape stay adversarial.
//!
//! Both targets are needed. This one cannot express the byte-ceiling refusal,
//! because a value derived from a grammar has no oversized spelling.

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use resourcefs_mcp::{ProfileDocument, ProfileErrorKind};
use serde_json::{Map, Value, json};

/// A token that is usually plausible and occasionally hostile.
///
/// Fully arbitrary strings almost never collide, and collision is what the
/// validation layer exists to catch. Drawing from a small pool makes duplicate
/// identifiers, repeated paths and name clashes common rather than
/// astronomically unlikely, while the `Raw` arm keeps arbitrary bytes reachable.
#[derive(Arbitrary, Debug)]
enum Token {
    A,
    B,
    Upper,
    Dotted,
    Empty,
    Whitespace,
    Slash,
    Traversal,
    Nul,
    Unicode,
    Long,
    Raw(String),
}

impl Token {
    fn as_str(&self) -> &str {
        match self {
            Self::A => "alpha",
            Self::B => "beta",
            Self::Upper => "ALPHA",
            Self::Dotted => "alpha.beta",
            Self::Empty => "",
            Self::Whitespace => "  ",
            Self::Slash => "alpha/beta",
            Self::Traversal => "../alpha",
            Self::Nul => "alpha\0beta",
            Self::Unicode => "\u{03b1}\u{03b2}",
            Self::Long => "alpha-repeated-segment-for-length-checks-0123456789",
            Self::Raw(value) => value.as_str(),
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Grants {
    create: Option<bool>,
    update: Option<bool>,
    delete: Option<bool>,
}

impl Grants {
    fn to_json(&self) -> Value {
        let mut map = Map::new();
        if let Some(create) = self.create {
            map.insert("create".to_owned(), json!(create));
        }
        if let Some(update) = self.update {
            map.insert("update".to_owned(), json!(update));
        }
        if let Some(delete) = self.delete {
            map.insert("delete".to_owned(), json!(delete));
        }
        Value::Object(map)
    }
}

#[derive(Arbitrary, Debug)]
enum SecretRef {
    Environment(Token),
    Command(Token),
}

impl SecretRef {
    fn to_json(&self) -> Value {
        match self {
            Self::Environment(name) => json!({"kind": "environment", "name": name.as_str()}),
            Self::Command(program) => json!({
                "kind": "command",
                "command": {"argv": [program.as_str()], "environment": {}},
            }),
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Origin {
    base_url: Token,
    allow_private_network: bool,
    credential: Option<(Token, SecretRef)>,
}

impl Origin {
    fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("baseUrl".to_owned(), json!(self.base_url.as_str()));
        map.insert(
            "allowPrivateNetwork".to_owned(),
            json!(self.allow_private_network),
        );
        if let Some((header, secret)) = &self.credential {
            map.insert(
                "credential".to_owned(),
                json!({"header": header.as_str(), "secret": secret.to_json()}),
            );
        }
        Value::Object(map)
    }
}

#[derive(Arbitrary, Debug)]
struct NamedPath {
    name: Token,
    path: Token,
}

impl NamedPath {
    fn to_json(&self) -> Value {
        json!({"name": self.name.as_str(), "path": self.path.as_str()})
    }
}

/// One `sources` entry. The tag and the required fields are always correct;
/// only the values vary, so the decoder admits the document and the validation
/// layer is what decides.
#[derive(Arbitrary, Debug)]
enum Source {
    Https(Vec<Origin>),
    Github {
        credential: SecretRef,
        allow_private_network: bool,
        repositories: Vec<Token>,
    },
    Skills(Vec<Token>),
    Rules(Vec<Token>),
    AgentExport(Vec<Token>),
    Memory(Vec<NamedPath>),
    Vault(Vec<NamedPath>),
}

impl Source {
    fn to_json(&self, id: &Token, required: bool, grants: Option<&Grants>) -> Value {
        let mut map = Map::new();
        map.insert("id".to_owned(), json!(id.as_str()));
        map.insert("required".to_owned(), json!(required));
        if let Some(grants) = grants {
            map.insert("grants".to_owned(), grants.to_json());
        }
        let kind = match self {
            Self::Https(origins) => {
                let values: Vec<Value> = origins.iter().map(Origin::to_json).collect();
                map.insert("origins".to_owned(), Value::Array(values));
                "https"
            }
            Self::Github {
                credential,
                allow_private_network,
                repositories,
            } => {
                map.insert("credential".to_owned(), credential.to_json());
                map.insert(
                    "allowPrivateNetwork".to_owned(),
                    json!(allow_private_network),
                );
                let values: Vec<Value> = repositories
                    .iter()
                    .map(|name| json!({"name": name.as_str()}))
                    .collect();
                map.insert("repositories".to_owned(), Value::Array(values));
                "github"
            }
            Self::Skills(roots) => {
                map.insert("roots".to_owned(), token_array(roots));
                "skills"
            }
            Self::Rules(manifests) => {
                map.insert("manifests".to_owned(), token_array(manifests));
                "rules"
            }
            Self::AgentExport(manifests) => {
                map.insert("manifests".to_owned(), token_array(manifests));
                "agentExport"
            }
            Self::Memory(roots) => {
                let values: Vec<Value> = roots.iter().map(NamedPath::to_json).collect();
                map.insert("roots".to_owned(), Value::Array(values));
                "memory"
            }
            Self::Vault(vaults) => {
                let values: Vec<Value> = vaults.iter().map(NamedPath::to_json).collect();
                map.insert("vaults".to_owned(), Value::Array(values));
                "vault"
            }
        };
        map.insert("kind".to_owned(), json!(kind));
        Value::Object(map)
    }
}

fn token_array(tokens: &[Token]) -> Value {
    Value::Array(tokens.iter().map(|t| json!(t.as_str())).collect())
}

#[derive(Arbitrary, Debug)]
struct Profile {
    /// Usually 1, so the version gate admits the document; other values keep
    /// the refusal path reachable.
    schema_version: Option<u32>,
    sources: Vec<(Token, bool, Option<Grants>, Source)>,
}

impl Profile {
    fn to_json(&self) -> Value {
        let sources: Vec<Value> = self
            .sources
            .iter()
            .map(|(id, required, grants, source)| source.to_json(id, *required, grants.as_ref()))
            .collect();
        json!({
            "schemaVersion": self.schema_version.unwrap_or(1),
            "sources": sources,
        })
    }
}

fuzz_target!(|profile: Profile| {
    let document = profile.to_json();
    let encoded = serde_json::to_vec(&document).expect("a generated profile always serializes");

    let first = ProfileDocument::from_slice(&encoded)
        .map(|_| ())
        .map_err(|error| error.kind());
    let second = ProfileDocument::from_slice(&encoded)
        .map(|_| ())
        .map_err(|error| error.kind());
    assert_eq!(first, second, "profile parsing must be deterministic");

    // A generated document is always well-formed JSON of the right shape, so
    // the decoder must never reach for the malformed-input categories. Anything
    // it refuses here is a validation decision, which is the point of the
    // target; `Io` or `LimitExceeded` would mean the grammar drifted from the
    // model it is meant to mirror.
    if let Err(kind) = first {
        assert_ne!(
            kind,
            ProfileErrorKind::Io,
            "a generated profile must not fail as I/O: {document}"
        );
    }
});
