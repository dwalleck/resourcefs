use resourcefs_core::{ProbeState, Redactor};
use resourcefs_sources::ProbeRun;
use serde::Serialize;

use super::check::{CheckedProfile, CheckedSource};

pub(super) fn render_static(profile: &CheckedProfile) -> Result<String, serde_json::Error> {
    let sources = profile
        .sources()
        .iter()
        .map(StaticSourceReport::from)
        .collect();
    serde_json::to_string(&CheckReport {
        ok: true,
        schema_version: profile.schema_version(),
        probe: false,
        sources,
    })
}

pub(super) fn render_probe(
    schema_version: u32,
    run: &ProbeRun,
    redactor: &Redactor,
) -> Result<String, serde_json::Error> {
    let sources = run
        .records()
        .iter()
        .map(|record| ProbeSourceReport {
            id: record.id(),
            kind: record.kind(),
            required: record.required(),
            state: state_name(record.outcome().state()),
            diagnostic: record
                .outcome()
                .diagnostic()
                .map(|diagnostic| redactor.scrub(diagnostic.as_str())),
        })
        .collect();
    serde_json::to_string(&CheckReport {
        ok: run.ok(),
        schema_version,
        probe: true,
        sources,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckReport<S> {
    ok: bool,
    schema_version: u32,
    probe: bool,
    sources: Vec<S>,
}

#[derive(Serialize)]
struct StaticSourceReport<'a> {
    id: &'a str,
    kind: &'static str,
    required: bool,
    state: &'static str,
}

impl<'a> From<&'a CheckedSource> for StaticSourceReport<'a> {
    fn from(source: &'a CheckedSource) -> Self {
        Self {
            id: source.id(),
            kind: source.kind(),
            required: source.required(),
            state: "notProbed",
        }
    }
}

#[derive(Serialize)]
struct ProbeSourceReport<'a> {
    id: &'a str,
    kind: &'a str,
    required: bool,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<String>,
}

const fn state_name(state: ProbeState) -> &'static str {
    match state {
        ProbeState::Available => "available",
        ProbeState::Degraded => "degraded",
        ProbeState::Failed => "failed",
        ProbeState::Unsupported => "unsupported",
    }
}
