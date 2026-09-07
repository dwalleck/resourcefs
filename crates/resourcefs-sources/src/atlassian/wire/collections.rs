use std::collections::{BTreeMap, BTreeSet};

use resourcefs_core::{AllowedOrigin, JiraProjectId, JiraProjectKey, ResourceError, SourceOffset};
use url::Url;

use super::{StrictJson, StrictParser, malformed_upstream, required_string, validate_object_self};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Presence<T> {
    Absent,
    Null,
    Value(T),
}

#[derive(Debug, Clone)]
pub(crate) struct JiraProject {
    pub(crate) id: JiraProjectId,
    pub(crate) key: JiraProjectKey,
    pub(crate) name: String,
    pub(crate) self_url: String,
    pub(crate) project_type_key: Presence<String>,
    pub(crate) archived: Presence<bool>,
    pub(crate) deleted: Presence<bool>,
}

pub(crate) enum ProjectLookup<'a> {
    StableId(&'a JiraProjectId),
    ProjectKeyAlias,
}

pub(crate) struct ProjectPage {
    pub(crate) values: Vec<JiraProject>,
    pub(crate) max_results: u64,
    pub(crate) next_offset: Option<SourceOffset>,
}

pub(crate) fn decode_project(
    body: &[u8],
    origin: &AllowedOrigin,
    lookup: ProjectLookup<'_>,
) -> Result<JiraProject, ResourceError> {
    let project = decode_project_value(StrictParser::parse(body)?, origin)?;
    if let ProjectLookup::StableId(expected) = lookup
        && expected != &project.id
    {
        return Err(malformed_upstream(
            "Jira returned a project ID that does not match the stable-ID request",
        ));
    }
    Ok(project)
}

fn decode_project_value(
    value: StrictJson,
    origin: &AllowedOrigin,
) -> Result<JiraProject, ResourceError> {
    let StrictJson::Object(mut object) = value else {
        return Err(malformed_upstream(
            "Jira project authority must be one JSON object",
        ));
    };
    let id = JiraProjectId::new(required_string(&mut object, "id", "project.id")?)
        .map_err(|_| malformed_upstream("Jira project ID is malformed"))?;
    let key = JiraProjectKey::new(required_string(&mut object, "key", "project.key")?)
        .map_err(|_| malformed_upstream("Jira project key is malformed"))?;
    let name = required_string(&mut object, "name", "project.name")?;
    let self_url = required_string(&mut object, "self", "project.self")?;
    validate_object_self(
        origin,
        &self_url,
        &format!("/rest/api/3/project/{}", id.as_str()),
    )?;
    let project_type_key = optional_value(&mut object, "projectTypeKey", |value| match value {
        StrictJson::String(value) => Some(value),
        _ => None,
    })?;
    let archived = optional_value(&mut object, "archived", json_bool)?;
    let deleted = optional_value(&mut object, "deleted", json_bool)?;
    Ok(JiraProject {
        id,
        key,
        name,
        self_url,
        project_type_key,
        archived,
        deleted,
    })
}

fn json_bool(value: StrictJson) -> Option<bool> {
    match value {
        StrictJson::Bool(value) => Some(value),
        _ => None,
    }
}

fn optional_value<T>(
    object: &mut BTreeMap<String, StrictJson>,
    field: &str,
    decode: impl FnOnce(StrictJson) -> Option<T>,
) -> Result<Presence<T>, ResourceError> {
    match object.remove(field) {
        None => Ok(Presence::Absent),
        Some(StrictJson::Null) => Ok(Presence::Null),
        Some(value) => decode(value).map(Presence::Value).ok_or_else(|| {
            malformed_upstream(format!("Jira project field '{field}' has the wrong type"))
        }),
    }
}

pub(crate) fn decode_project_page(
    body: &[u8],
    origin: &AllowedOrigin,
    expected_request: &Url,
) -> Result<ProjectPage, ResourceError> {
    let expected_query = page_query(expected_request, origin)?;
    let expected_offset = query_number(&expected_query, "startAt", false)?;
    let requested_max = query_number(&expected_query, "maxResults", true)?;
    let StrictJson::Object(mut object) = StrictParser::parse(body)? else {
        return Err(malformed_upstream(
            "Jira project page must be one JSON object",
        ));
    };
    let start_at = required_number(&mut object, "startAt", false)?;
    let max_results = required_number(&mut object, "maxResults", true)?;
    if start_at != expected_offset || max_results > requested_max {
        return Err(malformed_upstream(
            "Jira project page offsets or effective maximum do not match the request",
        ));
    }
    let is_last = match object.remove("isLast") {
        Some(StrictJson::Bool(value)) => value,
        _ => {
            return Err(malformed_upstream(
                "Jira project page requires a boolean isLast",
            ));
        }
    };
    let rows = match object.remove("values") {
        Some(StrictJson::Array(values)) => values,
        _ => {
            return Err(malformed_upstream(
                "Jira project page requires an array of values",
            ));
        }
    };
    if rows.len() as u64 > max_results {
        return Err(malformed_upstream(
            "Jira project page exceeds its effective maximum",
        ));
    }
    let next_offset = match (is_last, object.remove("nextPage")) {
        (true, None) => None,
        (false, Some(StrictJson::String(next))) => {
            let next = Url::parse(&next)
                .map_err(|_| malformed_upstream("Jira project next page URL is malformed"))?;
            let mut next_query = page_query(&next, origin)?;
            let offset = query_number(&next_query, "startAt", true)?;
            let next_max = query_number(&next_query, "maxResults", true)?;
            if offset <= start_at || (next_max != max_results && next_max != requested_max) {
                return Err(malformed_upstream(
                    "Jira project next page does not advance with a valid maximum",
                ));
            }
            let mut fixed_query = expected_query;
            fixed_query.remove("startAt");
            fixed_query.remove("maxResults");
            next_query.remove("startAt");
            next_query.remove("maxResults");
            if fixed_query != next_query {
                return Err(malformed_upstream(
                    "Jira project next page changes the fixed query",
                ));
            }
            Some(
                SourceOffset::new(offset)
                    .map_err(|_| malformed_upstream("Jira project next offset is malformed"))?,
            )
        }
        _ => {
            return Err(malformed_upstream(
                "Jira project terminal state contradicts its next page",
            ));
        }
    };
    let mut seen = BTreeSet::new();
    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let project = decode_project_value(row, origin)?;
        if !seen.insert(project.id.clone()) {
            return Err(malformed_upstream(
                "Jira project page contains duplicate stable IDs",
            ));
        }
        values.push(project);
    }
    Ok(ProjectPage {
        values,
        max_results,
        next_offset,
    })
}

fn page_query(
    url: &Url,
    origin: &AllowedOrigin,
) -> Result<BTreeMap<String, String>, ResourceError> {
    if !origin.authorizes(url)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path() != "/rest/api/3/project/search"
    {
        return Err(malformed_upstream(
            "Jira project page URL does not match its Site Mount and endpoint",
        ));
    }
    let mut query = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if query.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err(malformed_upstream(
                "Jira project page URL contains duplicate query parameters",
            ));
        }
    }
    Ok(query)
}

fn query_number(
    query: &BTreeMap<String, String>,
    field: &str,
    positive: bool,
) -> Result<u64, ResourceError> {
    let value = query
        .get(field)
        .ok_or_else(|| malformed_upstream("Jira project page URL lacks a required parameter"))?;
    canonical_u64(value, positive)
}

fn required_number(
    object: &mut BTreeMap<String, StrictJson>,
    field: &str,
    positive: bool,
) -> Result<u64, ResourceError> {
    match object.remove(field) {
        Some(StrictJson::Number(value)) => canonical_u64(&value, positive),
        _ => Err(malformed_upstream(format!(
            "Jira project page field '{field}' requires an integer"
        ))),
    }
}

fn canonical_u64(value: &str, positive: bool) -> Result<u64, ResourceError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(malformed_upstream(
            "Jira project pagination integer is malformed",
        ));
    }
    let number = value
        .parse::<u64>()
        .map_err(|_| malformed_upstream("Jira project pagination integer overflows"))?;
    if positive && number == 0 {
        return Err(malformed_upstream(
            "Jira project pagination integer must be positive",
        ));
    }
    Ok(number)
}

#[cfg(feature = "test-support")]
#[derive(Debug, PartialEq, Eq)]
pub struct JiraProjectWireObservation {
    pub id: String,
    pub key: String,
    pub name: String,
    pub project_type_key: Option<Option<String>>,
    pub archived: Option<Option<bool>>,
    pub deleted: Option<Option<bool>>,
    pub rendered: String,
}

#[cfg(feature = "test-support")]
#[derive(Debug)]
pub struct JiraProjectPageObservation {
    pub projects: Vec<JiraProjectWireObservation>,
    pub max_results: Option<u64>,
    pub next_offset: Option<u64>,
}

#[cfg(feature = "test-support")]
pub fn inspect_project_wire_for_test(
    body: &[u8],
    origin: &str,
    expected_request: Option<&str>,
    expected_id: Option<&str>,
) -> Result<JiraProjectPageObservation, ResourceError> {
    let origin = AllowedOrigin::new(origin, false)?;
    let (projects, max_results, next_offset) = if let Some(request) = expected_request {
        let request =
            Url::parse(request).map_err(|_| malformed_upstream("Invalid test request URL"))?;
        let page = decode_project_page(body, &origin, &request)?;
        (
            page.values,
            Some(page.max_results),
            page.next_offset.map(SourceOffset::get),
        )
    } else {
        let id = expected_id.map(JiraProjectId::new).transpose()?;
        let lookup = match &id {
            Some(id) => ProjectLookup::StableId(id),
            None => ProjectLookup::ProjectKeyAlias,
        };
        (vec![decode_project(body, &origin, lookup)?], None, None)
    };
    let site = resourcefs_core::AtlassianSiteId::new("test")?;
    let projects = projects
        .into_iter()
        .map(|project| {
            let rendered = crate::atlassian::render::collections::render_project(&site, &project)?;
            Ok(JiraProjectWireObservation {
                id: project.id.as_str().to_owned(),
                key: project.key.as_str().to_owned(),
                name: project.name,
                project_type_key: observe_presence(project.project_type_key),
                archived: observe_presence(project.archived),
                deleted: observe_presence(project.deleted),
                rendered,
            })
        })
        .collect::<Result<Vec<_>, ResourceError>>()?;
    Ok(JiraProjectPageObservation {
        projects,
        max_results,
        next_offset,
    })
}

#[cfg(feature = "test-support")]
fn observe_presence<T>(value: Presence<T>) -> Option<Option<T>> {
    match value {
        Presence::Absent => None,
        Presence::Null => Some(None),
        Presence::Value(value) => Some(Some(value)),
    }
}
