use resourcefs_core::{ErrorCategory, JiraQuery, MAX_PATH_REFERENCE_BYTES, ResourceError};
use serde::Serialize;

use super::{StrictJson, StrictParser, collections::NativeIssueToken, malformed_upstream};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryRequest<'a> {
    jql: &'a str,
    fields: [&'static str; 4],
    max_results: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_page_token: Option<&'a str>,
}

pub(crate) fn encode_query_request(
    query: &JiraQuery,
    token: Option<&NativeIssueToken>,
    requested: usize,
) -> Result<Vec<u8>, ResourceError> {
    let token = token.map(NativeIssueToken::as_str);
    // The canonical query/owner reference is checked by the caller first. Refuse raw
    // lengths before serde can allocate; JSON escaping costs at most six bytes per byte.
    let input_bytes = query.as_str().len().checked_add(token.map_or(0, str::len));
    if requested == 0
        || requested > 100
        || input_bytes.is_none_or(|bytes| bytes > MAX_PATH_REFERENCE_BYTES)
    {
        return Err(invalid_request());
    }
    let request = QueryRequest {
        jql: query.as_str(),
        fields: ["key", "summary", "status", "project"],
        max_results: requested,
        next_page_token: token,
    };
    // Serialize once, refusing an escaped write before it can exceed the derived ceiling.
    // The HTTP constructor independently enforces the same 384 KiB payload ceiling.
    let mut body = QueryBytes(Vec::new());
    serde_json::to_writer(&mut body, &request).map_err(|_| invalid_request())?;
    Ok(body.0)
}

struct QueryBytes(Vec<u8>);

impl std::io::Write for QueryBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        const LIMIT: usize = 6 * MAX_PATH_REFERENCE_BYTES;
        if bytes.len() > LIMIT - self.0.len() {
            return Err(std::io::Error::other(
                "query request exceeds encoded ceiling",
            ));
        }
        let required = self.0.len() + bytes.len();
        if required > self.0.capacity() {
            let capacity = required.max(self.0.capacity().max(128) * 2).min(LIMIT);
            self.0.reserve_exact(capacity - self.0.len());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn invalid_request() -> ResourceError {
    ResourceError::new(
        ErrorCategory::InvalidReference,
        "Jira query request exceeds its representable bounds",
    )
}

/// Validate the published Error Collection shape without retaining diagnostic prose.
pub(crate) fn decode_query_rejection(body: &[u8]) -> Result<ResourceError, ResourceError> {
    let StrictJson::Object(mut object) = StrictParser::parse(body)? else {
        return Err(malformed_upstream(
            "Jira query rejection must be an Error Collection",
        ));
    };
    let global_count = match object.remove("errorMessages") {
        None => 0,
        Some(StrictJson::Array(values))
            if values
                .iter()
                .all(|value| matches!(value, StrictJson::String(_))) =>
        {
            values.len()
        }
        Some(_) => {
            return Err(malformed_upstream(
                "Jira query rejection has malformed global diagnostics",
            ));
        }
    };
    let field_count = match object.remove("errors") {
        None => 0,
        Some(StrictJson::Object(values))
            if values
                .values()
                .all(|value| matches!(value, StrictJson::String(_))) =>
        {
            values.len()
        }
        Some(_) => {
            return Err(malformed_upstream(
                "Jira query rejection has malformed field diagnostics",
            ));
        }
    };
    match object.remove("status") {
        None => {}
        Some(StrictJson::Number(value)) if !value.contains(['.', 'e', 'E']) => {}
        Some(_) => {
            return Err(malformed_upstream(
                "Jira query rejection has malformed status",
            ));
        }
    }
    Ok(ResourceError::new(
        ErrorCategory::InvalidPattern,
        format!(
            "Jira rejected the query (HTTP 400; {global_count} global errors; {field_count} field errors)"
        ),
    ))
}
