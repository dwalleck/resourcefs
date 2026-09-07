use std::collections::BTreeMap;

use resourcefs_core::{
    AllowedOrigin, ErrorCategory, JiraFieldId, JiraIssueId, JiraIssueKey, ResourceError,
};
use url::Url;

pub(crate) mod collections;

const MAX_JIRA_JSON_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StrictJson {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

struct StrictParser<'a> {
    input: &'a [u8],
    index: usize,
}

impl<'a> StrictParser<'a> {
    fn parse(input: &'a [u8]) -> Result<StrictJson, ResourceError> {
        let mut parser = Self { input, index: 0 };
        parser.skip_whitespace();
        let value = parser.parse_value(0)?;
        parser.skip_whitespace();
        if parser.index != input.len() {
            return Err(malformed_upstream(
                "Jira upstream JSON has trailing content",
            ));
        }
        Ok(value)
    }

    fn parse_value(&mut self, depth: usize) -> Result<StrictJson, ResourceError> {
        if depth > MAX_JIRA_JSON_DEPTH {
            return Err(malformed_upstream(
                "Jira upstream JSON exceeds the nesting-depth ceiling",
            ));
        }
        match self.input.get(self.index).copied() {
            Some(b'n') => {
                self.consume_literal(b"null")?;
                Ok(StrictJson::Null)
            }
            Some(b't') => {
                self.consume_literal(b"true")?;
                Ok(StrictJson::Bool(true))
            }
            Some(b'f') => {
                self.consume_literal(b"false")?;
                Ok(StrictJson::Bool(false))
            }
            Some(b'"') => self.parse_string().map(StrictJson::String),
            Some(b'[') => self.parse_array(depth),
            Some(b'{') => self.parse_object(depth),
            Some(b'-' | b'0'..=b'9') => self.parse_number().map(StrictJson::Number),
            _ => Err(malformed_upstream("Jira upstream JSON is malformed")),
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<StrictJson, ResourceError> {
        self.index += 1;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.consume_if(b']') {
            return Ok(StrictJson::Array(values));
        }
        loop {
            values.push(self.parse_value(depth + 1)?);
            self.skip_whitespace();
            if self.consume_if(b']') {
                break;
            }
            self.consume_required(b',')?;
            self.skip_whitespace();
        }
        Ok(StrictJson::Array(values))
    }

    fn parse_object(&mut self, depth: usize) -> Result<StrictJson, ResourceError> {
        self.index += 1;
        self.skip_whitespace();
        let mut values = BTreeMap::new();
        if self.consume_if(b'}') {
            return Ok(StrictJson::Object(values));
        }
        loop {
            if self.input.get(self.index) != Some(&b'"') {
                return Err(malformed_upstream(
                    "Jira upstream JSON object key must be a string",
                ));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.consume_required(b':')?;
            self.skip_whitespace();
            let value = self.parse_value(depth + 1)?;
            if values.insert(key, value).is_some() {
                return Err(malformed_upstream(
                    "Jira upstream JSON contains a duplicate object member",
                ));
            }
            self.skip_whitespace();
            if self.consume_if(b'}') {
                break;
            }
            self.consume_required(b',')?;
            self.skip_whitespace();
        }
        Ok(StrictJson::Object(values))
    }

    fn parse_string(&mut self) -> Result<String, ResourceError> {
        let start = self.index;
        self.index += 1;
        let mut escaped = false;
        while let Some(byte) = self.input.get(self.index).copied() {
            self.index += 1;
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => {
                    return serde_json::from_slice(&self.input[start..self.index])
                        .map_err(|_| malformed_upstream("Jira upstream JSON string is malformed"));
                }
                _ => {}
            }
        }
        Err(malformed_upstream(
            "Jira upstream JSON string is unterminated",
        ))
    }

    fn parse_number(&mut self) -> Result<String, ResourceError> {
        let start = self.index;
        self.consume_if(b'-');
        match self.input.get(self.index).copied() {
            Some(b'0') => {
                self.index += 1;
                if self.input.get(self.index).is_some_and(u8::is_ascii_digit) {
                    return Err(malformed_upstream(
                        "Jira upstream JSON number has a leading zero",
                    ));
                }
            }
            Some(b'1'..=b'9') => {
                self.index += 1;
                while self.input.get(self.index).is_some_and(u8::is_ascii_digit) {
                    self.index += 1;
                }
            }
            _ => return Err(malformed_upstream("Jira upstream JSON number is malformed")),
        }
        if self.consume_if(b'.') {
            let fraction_start = self.index;
            while self.input.get(self.index).is_some_and(u8::is_ascii_digit) {
                self.index += 1;
            }
            if self.index == fraction_start {
                return Err(malformed_upstream(
                    "Jira upstream JSON number fraction is empty",
                ));
            }
        }
        if self
            .input
            .get(self.index)
            .is_some_and(|byte| matches!(byte, b'e' | b'E'))
        {
            self.index += 1;
            if self
                .input
                .get(self.index)
                .is_some_and(|byte| matches!(byte, b'+' | b'-'))
            {
                self.index += 1;
            }
            let exponent_start = self.index;
            while self.input.get(self.index).is_some_and(u8::is_ascii_digit) {
                self.index += 1;
            }
            if self.index == exponent_start {
                return Err(malformed_upstream(
                    "Jira upstream JSON number exponent is empty",
                ));
            }
        }
        let raw = std::str::from_utf8(&self.input[start..self.index])
            .map_err(|_| malformed_upstream("Jira upstream JSON number is not UTF-8"))?;
        canonicalize_number(raw)
    }

    fn consume_literal(&mut self, literal: &[u8]) -> Result<(), ResourceError> {
        if self.input.get(self.index..self.index + literal.len()) == Some(literal) {
            self.index += literal.len();
            Ok(())
        } else {
            Err(malformed_upstream("Jira upstream JSON is malformed"))
        }
    }

    fn consume_required(&mut self, expected: u8) -> Result<(), ResourceError> {
        if self.consume_if(expected) {
            Ok(())
        } else {
            Err(malformed_upstream("Jira upstream JSON is malformed"))
        }
    }

    fn consume_if(&mut self, expected: u8) -> bool {
        if self.input.get(self.index) == Some(&expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while self
            .input
            .get(self.index)
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.index += 1;
        }
    }
}

fn canonicalize_number(raw: &str) -> Result<String, ResourceError> {
    let bytes = raw.as_bytes();
    let negative = bytes.first() == Some(&b'-');
    let unsigned = if negative { &raw[1..] } else { raw };
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, parse_exponent(exponent)?),
        None => (unsigned, 0),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits = String::with_capacity(integer.len() + fraction.len());
    digits.push_str(integer);
    digits.push_str(fraction);
    let leading = digits.bytes().take_while(|byte| *byte == b'0').count();
    if leading == digits.len() {
        return Ok("0".to_owned());
    }
    let coefficient = digits[leading..].trim_end_matches('0');
    let integer_len = i64::try_from(integer.len())
        .map_err(|_| malformed_upstream("Jira upstream JSON number is too large"))?;
    let leading = i64::try_from(leading)
        .map_err(|_| malformed_upstream("Jira upstream JSON number is too large"))?;
    let decimal_position = integer_len
        .checked_add(exponent)
        .and_then(|position| position.checked_sub(leading))
        .ok_or_else(|| malformed_upstream("Jira upstream JSON number exponent is too large"))?;
    let scientific_exponent = decimal_position
        .checked_sub(1)
        .ok_or_else(|| malformed_upstream("Jira upstream JSON number exponent is too small"))?;

    let mut output = String::with_capacity(raw.len() + 4);
    if negative {
        output.push('-');
    }
    if (-6..21).contains(&scientific_exponent) {
        if decimal_position <= 0 {
            output.push_str("0.");
            for _ in 0..decimal_position.unsigned_abs() {
                output.push('0');
            }
            output.push_str(coefficient);
        } else {
            let decimal_position = usize::try_from(decimal_position)
                .map_err(|_| malformed_upstream("Jira upstream JSON number is too large"))?;
            if decimal_position >= coefficient.len() {
                output.push_str(coefficient);
                for _ in coefficient.len()..decimal_position {
                    output.push('0');
                }
            } else {
                output.push_str(&coefficient[..decimal_position]);
                output.push('.');
                output.push_str(&coefficient[decimal_position..]);
            }
        }
    } else {
        output.push(char::from(coefficient.as_bytes()[0]));
        if coefficient.len() > 1 {
            output.push('.');
            output.push_str(&coefficient[1..]);
        }
        output.push('e');
        if scientific_exponent >= 0 {
            output.push('+');
        }
        output.push_str(&scientific_exponent.to_string());
    }
    Ok(output)
}

fn parse_exponent(raw: &str) -> Result<i64, ResourceError> {
    let (negative, digits) = raw.strip_prefix('-').map_or_else(
        || (false, raw.strip_prefix('+').unwrap_or(raw)),
        |digits| (true, digits),
    );
    let mut exponent = 0_i64;
    for digit in digits.bytes() {
        exponent = exponent
            .checked_mul(10)
            .and_then(|value| value.checked_add(i64::from(digit - b'0')))
            .ok_or_else(|| malformed_upstream("Jira upstream JSON number exponent is too large"))?;
    }
    if negative {
        exponent
            .checked_neg()
            .ok_or_else(|| malformed_upstream("Jira upstream JSON number exponent is too small"))
    } else {
        Ok(exponent)
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
    let root = StrictParser::parse(body)?;
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
    validate_object_self(
        origin,
        value,
        &format!("/rest/api/3/issue/{}", issue_id.as_str()),
    )
}

fn validate_object_self(
    origin: &AllowedOrigin,
    value: &str,
    expected_path: &str,
) -> Result<(), ResourceError> {
    let url =
        Url::parse(value).map_err(|_| malformed_upstream("Jira issue self URL is malformed"))?;
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
