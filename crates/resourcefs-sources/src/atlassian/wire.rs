use std::{collections::BTreeMap, fmt};

use resourcefs_core::{
    AllowedOrigin, ErrorCategory, JiraFieldId, JiraIssueId, JiraIssueKey, ResourceError,
};
use serde::{
    Deserialize, Deserializer,
    de::{self, Error as _, MapAccess, SeqAccess, Visitor},
};
use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StrictJson {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

struct StrictJsonVisitor;

impl<'de> Visitor<'de> for StrictJsonVisitor {
    type Value = StrictJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("one strict JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJson::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJson::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictJson::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictJson::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictJson::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(StrictJson::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictJson::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictJson::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictJson::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some((key, value)) = map.next_entry::<String, StrictJson>()? {
            if values.insert(key, value).is_some() {
                return Err(A::Error::custom("duplicate JSON object member"));
            }
        }
        Ok(StrictJson::Object(values))
    }
}

impl StrictJson {
    pub(crate) fn canonical_json(&self) -> String {
        let mut output = String::new();
        self.write_canonical(&mut output);
        output
    }

    fn write_canonical(&self, output: &mut String) {
        match self {
            Self::Null => output.push_str("null"),
            Self::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            Self::Number(value) => output.push_str(&value.to_string()),
            Self::String(value) => {
                output.push_str(
                    &serde_json::to_string(value)
                        .expect("serializing an in-memory JSON string cannot fail"),
                );
            }
            Self::Array(values) => {
                output.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        output.push(',');
                    }
                    value.write_canonical(output);
                }
                output.push(']');
            }
            Self::Object(values) => {
                output.push('{');
                for (index, (key, value)) in values.iter().enumerate() {
                    if index != 0 {
                        output.push(',');
                    }
                    output.push_str(
                        &serde_json::to_string(key)
                            .expect("serializing an in-memory JSON key cannot fail"),
                    );
                    output.push(':');
                    value.write_canonical(output);
                }
                output.push('}');
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct JiraField {
    pub(crate) id: JiraFieldId,
    pub(crate) name: String,
    pub(crate) native_type: String,
    pub(crate) value: StrictJson,
    pub(crate) canonical_json: String,
}

#[derive(Debug, Clone)]
pub(crate) struct JiraIssue {
    pub(crate) id: JiraIssueId,
    pub(crate) key: JiraIssueKey,
    pub(crate) self_url: String,
    pub(crate) fields: BTreeMap<JiraFieldId, JiraField>,
}

pub(crate) enum JiraLookup<'a> {
    StableId(&'a JiraIssueId),
    IssueKeyAlias,
}

pub(crate) fn decode_issue(
    body: &[u8],
    origin: &AllowedOrigin,
    lookup: JiraLookup<'_>,
) -> Result<JiraIssue, ResourceError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let root = StrictJson::deserialize(&mut deserializer)
        .map_err(|_| malformed_upstream("Jira upstream JSON is malformed"))?;
    deserializer
        .end()
        .map_err(|_| malformed_upstream("Jira upstream JSON has trailing content"))?;
    let StrictJson::Object(mut root) = root else {
        return Err(malformed_upstream(
            "Jira issue authority must be one JSON object",
        ));
    };

    let id = JiraIssueId::new(required_string(&mut root, "id", "issue.id")?)
        .map_err(|_| malformed_upstream("Jira issue ID is malformed"))?;
    let key = JiraIssueKey::new(required_string(&mut root, "key", "issue.key")?)
        .map_err(|_| malformed_upstream("Jira issue key is malformed"))?;
    let self_url = required_string(&mut root, "self", "issue.self")?;
    validate_issue_self(origin, &self_url, &id)?;
    if let JiraLookup::StableId(expected) = lookup
        && expected != &id
    {
        return Err(malformed_upstream(
            "Jira returned an issue ID that does not match the stable-ID request",
        ));
    }

    let fields = required_object(&mut root, "fields", "issue.fields")?;
    let mut names = required_object(&mut root, "names", "issue.names")?;
    let mut schemas = required_object(&mut root, "schema", "issue.schema")?;
    let mut decoded_fields = BTreeMap::new();
    for (field_id, value) in fields {
        let field_id = JiraFieldId::new(field_id)
            .map_err(|_| malformed_upstream("Jira field ID is malformed"))?;
        let name = required_string(&mut names, field_id.as_str(), "issue.names")?;
        let mut schema = required_object(&mut schemas, field_id.as_str(), "issue.schema")?;
        let native_type = required_string(&mut schema, "type", "issue.schema.type")?;
        if native_type.is_empty() {
            return Err(malformed_upstream("Jira field native type is empty"));
        }
        let canonical_json = value.canonical_json();
        let field = JiraField {
            id: field_id.clone(),
            name,
            native_type,
            value,
            canonical_json,
        };
        decoded_fields.insert(field_id, field);
    }
    Ok(JiraIssue {
        id,
        key,
        self_url,
        fields: decoded_fields,
    })
}

fn required_string(
    object: &mut BTreeMap<String, StrictJson>,
    field: &str,
    path: &str,
) -> Result<String, ResourceError> {
    match object.remove(field) {
        Some(StrictJson::String(value)) => Ok(value),
        Some(_) => Err(malformed_upstream(format!(
            "Jira upstream field '{path}' must be a string"
        ))),
        None => Err(malformed_upstream(format!(
            "Jira upstream field '{path}' is required"
        ))),
    }
}

fn required_object(
    object: &mut BTreeMap<String, StrictJson>,
    field: &str,
    path: &str,
) -> Result<BTreeMap<String, StrictJson>, ResourceError> {
    match object.remove(field) {
        Some(StrictJson::Object(value)) => Ok(value),
        Some(_) => Err(malformed_upstream(format!(
            "Jira upstream field '{path}' must be an object"
        ))),
        None => Err(malformed_upstream(format!(
            "Jira upstream field '{path}' is required"
        ))),
    }
}

fn validate_issue_self(
    origin: &AllowedOrigin,
    value: &str,
    issue_id: &JiraIssueId,
) -> Result<(), ResourceError> {
    let url =
        Url::parse(value).map_err(|_| malformed_upstream("Jira issue self URL is malformed"))?;
    let expected_path = format!("/rest/api/3/issue/{}", issue_id.as_str());
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !origin.authorizes(&url)
        || url.path() != expected_path
    {
        return Err(malformed_upstream(
            "Jira issue self URL does not match its Site Mount and stable ID",
        ));
    }
    Ok(())
}

fn malformed_upstream(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::SourceUnavailable, message)
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone)]
pub enum JiraWireLookupForTest {
    StableId(String),
    IssueKey(String),
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraWireFieldObservation {
    pub id: String,
    pub name: String,
    pub native_type: String,
    pub canonical_json: String,
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraWireObservation {
    pub issue_id: String,
    pub issue_key: String,
    pub self_url: String,
    pub fields: Vec<JiraWireFieldObservation>,
}

#[cfg(feature = "test-support")]
pub fn inspect_jira_wire_for_test(
    body: &[u8],
    origin: &str,
    lookup: JiraWireLookupForTest,
) -> Result<JiraWireObservation, ResourceError> {
    let origin = AllowedOrigin::new(origin, false)?;
    let stable_id;
    let lookup = match &lookup {
        JiraWireLookupForTest::StableId(value) => {
            stable_id = JiraIssueId::new(value.clone())?;
            JiraLookup::StableId(&stable_id)
        }
        JiraWireLookupForTest::IssueKey(value) => {
            JiraIssueKey::new(value.clone())?;
            JiraLookup::IssueKeyAlias
        }
    };
    let issue = decode_issue(body, &origin, lookup)?;
    Ok(JiraWireObservation {
        issue_id: issue.id.as_str().to_owned(),
        issue_key: issue.key.as_str().to_owned(),
        self_url: issue.self_url,
        fields: issue
            .fields
            .into_values()
            .map(|field| JiraWireFieldObservation {
                id: field.id.as_str().to_owned(),
                name: field.name,
                native_type: field.native_type,
                canonical_json: field.canonical_json,
            })
            .collect(),
    })
}
