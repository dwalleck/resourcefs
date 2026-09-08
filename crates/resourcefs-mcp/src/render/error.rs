use resourcefs_core::{LimitDetail, ResourceError, ResourceErrorDetails, RetryGuidance};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct ErrorOutput {
    category: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<DetailsOutput>,
}

impl From<&ResourceError> for ErrorOutput {
    fn from(error: &ResourceError) -> Self {
        Self {
            category: error.category().as_str(),
            message: error.message().to_owned(),
            details: error.details().map(DetailsOutput::from),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DetailsOutput {
    reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    access_ambiguity: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_guidance: Option<RetryOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limit_reset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<LimitOutput>,
}

impl From<&ResourceErrorDetails> for DetailsOutput {
    fn from(details: &ResourceErrorDetails) -> Self {
        Self {
            reason: details.reason().as_str(),
            http_status: details.http_status().map(|status| status.get()),
            access_ambiguity: details
                .access_ambiguity()
                .map(|ambiguity| ambiguity.as_str()),
            retry_guidance: details.retry_guidance().map(RetryOutput::from),
            rate_limit_reset: details.rate_limit_reset(),
            limit: details.limit().map(LimitOutput::from),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
enum RetryOutput {
    DelaySeconds(u64),
    AtUnixSeconds(u64),
}

impl From<RetryGuidance> for RetryOutput {
    fn from(guidance: RetryGuidance) -> Self {
        match guidance {
            RetryGuidance::DelaySeconds(seconds) => Self::DelaySeconds(seconds),
            RetryGuidance::AtUnixSeconds(seconds) => Self::AtUnixSeconds(seconds),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LimitOutput {
    kind: &'static str,
    bound: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed: Option<u64>,
}

impl From<LimitDetail> for LimitOutput {
    fn from(limit: LimitDetail) -> Self {
        Self {
            kind: limit.kind().as_str(),
            bound: limit.bound(),
            observed: limit.observed(),
        }
    }
}
