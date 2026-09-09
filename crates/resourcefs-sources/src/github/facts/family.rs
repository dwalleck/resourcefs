//! The set of pull-request discussion fact families and their dispatch.
//!
//! The collection engine owns bounded traversal and coverage; each family
//! module owns its native decoding, validation and projection. This module is
//! the only place that knows the whole set, so adding a family is one
//! exhaustive match away from compiling.
use resourcefs_core::{
    ErrorReason, GithubRepositoryIdentity, PullRequestFact, PullRequestNumber, ResourceError,
};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

use super::{Unavailable, comment, failure, identity, inline, pull, review};

/// Native discussion records the collection engine can traverse.
///
/// The variants differ only by how many native fields a family supplies, so
/// the largest is wider than the smallest. Boxing the wide variant would add
/// one heap allocation per admitted record; the engine already bounds a read
/// at 1,000 records, so the layout cost is bounded and cheaper than the
/// allocation.
#[allow(clippy::large_enum_variant)]
pub(super) enum Native {
    Conversation(comment::NativeComment),
    Review(review::NativeReview),
    Inline(inline::NativeInlineComment),
}

/// Validated discussion records: identity and availability are already proven.
///
/// Same layout rationale as [`Native`].
#[allow(clippy::large_enum_variant)]
pub(super) enum Record {
    Conversation(comment::ValidatedRecord),
    Review(review::ValidatedRecord),
    Inline(inline::ValidatedRecord),
}

/// One owned projection, serialized untagged so a collection body is a flat
/// record array regardless of family.
#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum RecordView<'a> {
    Conversation(comment::CommentRecord<'a>),
    Review(review::ReviewRecord<'a>),
    Inline(inline::InlineCommentRecord<'a>),
}

/// A bounded collection family. Constructing this from a non-collection fact
/// is refused, so the engine cannot traverse an item family.
#[derive(Clone, Copy)]
pub(super) enum CollectionFamily {
    Conversation,
    Review,
    Inline,
}

impl CollectionFamily {
    pub(super) fn from_fact(fact: PullRequestFact) -> Result<Self, ResourceError> {
        match fact {
            PullRequestFact::Comments => Ok(Self::Conversation),
            PullRequestFact::Reviews => Ok(Self::Review),
            PullRequestFact::ReviewComments => Ok(Self::Inline),
            PullRequestFact::Pull
            | PullRequestFact::Comment(_)
            | PullRequestFact::Review(_)
            | PullRequestFact::ReviewComment(_) => {
                Err(super::super::unsupported_github_projection())
            }
        }
    }

    pub(super) const fn kind(self) -> &'static str {
        match self {
            Self::Conversation => "github.conversation_comment_collection",
            Self::Review => "github.review_submission_collection",
            Self::Inline => "github.review_comment_collection",
        }
    }

    /// The provider path below `repos/<owner>/<name>/` for this collection.
    pub(super) fn suffix(self, number: PullRequestNumber) -> String {
        match self {
            Self::Conversation => format!("issues/{}/comments", number.get()),
            Self::Review => format!("pulls/{}/reviews", number.get()),
            Self::Inline => format!("pulls/{}/comments", number.get()),
        }
    }

    /// Decode one provider page into native records of this family.
    pub(super) fn decode(self, body: &[u8]) -> Result<Vec<Native>, ResourceError> {
        match self {
            Self::Conversation => {
                decode_page::<comment::NativeComment, Native>(body, Native::Conversation)
            }
            Self::Review => decode_page::<review::NativeReview, Native>(body, Native::Review),
            Self::Inline => {
                decode_page::<inline::NativeInlineComment, Native>(body, Native::Inline)
            }
        }
    }
}

fn decode_page<T: DeserializeOwned, N>(
    body: &[u8],
    wrap: impl Fn(T) -> N,
) -> Result<Vec<N>, ResourceError> {
    let records: Vec<T> =
        serde_json::from_slice(body).map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    Ok(records.into_iter().map(wrap).collect())
}

/// Validate and own one native record. The variant selects the family's
/// validator, so a record can never be validated as the wrong family.
pub(super) fn validate(
    native: Native,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    api: &Url,
    web: &Url,
) -> Result<Record, ResourceError> {
    match native {
        Native::Conversation(comment) => {
            comment::validate_record(comment, repository, number, None, api, web)
                .map(Record::Conversation)
        }
        Native::Review(review) => {
            review::validate_record(review, repository, number, None, api, web).map(Record::Review)
        }
        Native::Inline(comment) => {
            inline::validate_record(comment, repository, number, None, api, web).map(Record::Inline)
        }
    }
}

pub(super) fn id(record: &Record) -> u64 {
    match record {
        Record::Conversation(record) => record.id(),
        Record::Review(record) => record.id(),
        Record::Inline(record) => record.id(),
    }
}

pub(super) fn unavailable(record: &Record) -> &[Unavailable] {
    match record {
        Record::Conversation(record) => record.unavailable(),
        Record::Review(record) => record.unavailable(),
        Record::Inline(record) => record.unavailable(),
    }
}

pub(super) fn project<'a>(
    record: &'a Record,
    pull: &'a pull::NativePull,
    identity: &'a identity::ValidatedIdentity<'a>,
) -> RecordView<'a> {
    match record {
        Record::Conversation(record) => RecordView::Conversation(record.project(pull, identity)),
        Record::Review(record) => RecordView::Review(record.project(pull, identity)),
        Record::Inline(record) => RecordView::Inline(record.project(pull, identity)),
    }
}
