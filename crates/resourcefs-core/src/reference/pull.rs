//! Pull-request fact families and their opaque source-cursor selector.
use super::{
    ConversationCommentId, ProjectionSelector, PullRequestAddress, PullRequestResource,
    invalid_reference, parse_pull_request_address, projection_candidate_split,
};

/// Immutable machine-readable pull-request fact family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PullRequestFact {
    /// The pull request itself.
    Pull,
    /// Conversation-comment collection.
    Comments,
    /// One conversation comment.
    Comment(ConversationCommentId),
}

/// Parses a `pr://` reference, splitting the opaque source cursor this family
/// alone accepts.
///
/// Native page markers stay with the generic split; a `:cursor:` continuation
/// belongs to the conversation-comment collection and nowhere else.
pub(super) fn parse_pull_request_reference(
    input: &str,
) -> Result<(PullRequestAddress, Option<ProjectionSelector>), super::ResourceError> {
    let split = input
        .find(":cursor:")
        .map(|index| (&input[..index], &input[index + 1..]))
        .or_else(|| projection_candidate_split(input));
    let (base, projection) = match split {
        Some((base, selector)) => (base, Some(ProjectionSelector::parse(selector)?)),
        None => (input, None),
    };
    let address = parse_pull_request_address(base)?;
    if projection
        .as_ref()
        .is_some_and(|selector| selector.source_cursor().is_some())
        && !matches!(
            address,
            PullRequestAddress::Item {
                resource: PullRequestResource::Facts(PullRequestFact::Comments),
                ..
            }
        )
    {
        return Err(invalid_reference(
            "source cursor requires the GitHub pull request conversation-comment collection",
        ));
    }
    Ok((address, projection))
}
