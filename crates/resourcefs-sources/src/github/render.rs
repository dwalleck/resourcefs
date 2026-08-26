use std::fmt::Write as _;

use resourcefs_core::GithubRepositoryIdentity;

use super::wire::{
    ConversationComment, DiffFile, Issue, PullRequest, Review, ReviewComment, SimpleUser,
};

fn author(user: &Option<SimpleUser>) -> &str {
    user.as_ref()
        .map_or("[deleted]", |user| user.login.as_str())
}

fn body(value: Option<&str>) -> &str {
    value.unwrap_or("(no body)")
}

pub(super) fn issue_aggregate(
    repository: &GithubRepositoryIdentity,
    issue: &Issue,
    comments: &mut [ConversationComment],
) -> String {
    comments.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let base = format!(
        "issue://{}/{number}",
        repository.as_str(),
        number = issue.number
    );
    let mut rendered = String::new();
    writeln!(&mut rendered, "# Issue #{}: {}", issue.number, issue.title).expect("String write");
    writeln!(&mut rendered).expect("String write");
    writeln!(&mut rendered, "Kind: issue").expect("String write");
    writeln!(&mut rendered, "Number: {}", issue.number).expect("String write");
    writeln!(&mut rendered, "State: {}", issue.state).expect("String write");
    writeln!(&mut rendered, "Author: {}", author(&issue.user)).expect("String write");
    writeln!(&mut rendered, "Created: {}", issue.created_at).expect("String write");
    writeln!(&mut rendered, "Updated: {}", issue.updated_at).expect("String write");
    writeln!(&mut rendered, "URL: {}", issue.html_url).expect("String write");
    writeln!(&mut rendered, "Title: {base}/title").expect("String write");
    writeln!(&mut rendered, "Body: {base}/body").expect("String write");
    writeln!(&mut rendered).expect("String write");
    writeln!(&mut rendered, "## Body").expect("String write");
    writeln!(&mut rendered).expect("String write");
    writeln!(&mut rendered, "{}", body(issue.body.as_deref())).expect("String write");
    writeln!(&mut rendered).expect("String write");
    writeln!(&mut rendered, "## Conversation comments").expect("String write");
    for comment in comments {
        writeln!(&mut rendered).expect("String write");
        writeln!(&mut rendered, "### Comment {}", comment.id).expect("String write");
        writeln!(&mut rendered, "Author: {}", author(&comment.user)).expect("String write");
        writeln!(&mut rendered, "Created: {}", comment.created_at).expect("String write");
        writeln!(&mut rendered, "Updated: {}", comment.updated_at).expect("String write");
        writeln!(&mut rendered, "Reference: {base}/comments/{}", comment.id).expect("String write");
        writeln!(&mut rendered).expect("String write");
        writeln!(&mut rendered, "{}", body(comment.body.as_deref())).expect("String write");
    }
    rendered
}

pub(super) fn pull_request_aggregate(
    repository: &GithubRepositoryIdentity,
    pull: &PullRequest,
    comments: &mut [ConversationComment],
    reviews: &mut [Review],
    review_comments: &mut [ReviewComment],
    files: &[DiffFile],
) -> String {
    comments.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    reviews.sort_by(|left, right| {
        left.submitted_at
            .cmp(&right.submitted_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    review_comments.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let base = format!(
        "pr://{}/{number}",
        repository.as_str(),
        number = pull.number
    );
    let mut rendered = String::new();
    writeln!(
        &mut rendered,
        "# Pull request #{}: {}",
        pull.number, pull.title
    )
    .expect("String write");
    writeln!(&mut rendered).expect("String write");
    writeln!(&mut rendered, "Kind: pull request").expect("String write");
    writeln!(&mut rendered, "Number: {}", pull.number).expect("String write");
    writeln!(&mut rendered, "State: {}", pull.state).expect("String write");
    writeln!(&mut rendered, "Author: {}", author(&pull.user)).expect("String write");
    writeln!(&mut rendered, "Created: {}", pull.created_at).expect("String write");
    writeln!(&mut rendered, "Updated: {}", pull.updated_at).expect("String write");
    writeln!(&mut rendered, "URL: {}", pull.html_url).expect("String write");
    writeln!(&mut rendered, "Draft: {}", pull.draft).expect("String write");
    writeln!(&mut rendered, "Merged: {}", pull.merged).expect("String write");
    writeln!(
        &mut rendered,
        "Merged at: {}",
        pull.merged_at.as_deref().unwrap_or("not merged")
    )
    .expect("String write");
    writeln!(&mut rendered, "Head: {}", pull.head.name).expect("String write");
    writeln!(&mut rendered, "Base: {}", pull.base.name).expect("String write");
    writeln!(&mut rendered, "Title: {base}/title").expect("String write");
    writeln!(&mut rendered, "Body: {base}/body").expect("String write");
    writeln!(&mut rendered).expect("String write");
    writeln!(&mut rendered, "## Body\n").expect("String write");
    writeln!(&mut rendered, "{}", body(pull.body.as_deref())).expect("String write");
    writeln!(&mut rendered, "\n## Conversation comments").expect("String write");
    for comment in comments {
        writeln!(&mut rendered, "\n### Comment {}", comment.id).expect("String write");
        writeln!(&mut rendered, "Author: {}", author(&comment.user)).expect("String write");
        writeln!(&mut rendered, "Created: {}", comment.created_at).expect("String write");
        writeln!(&mut rendered, "Updated: {}", comment.updated_at).expect("String write");
        writeln!(&mut rendered, "Reference: {base}/comments/{}\n", comment.id)
            .expect("String write");
        writeln!(&mut rendered, "{}", body(comment.body.as_deref())).expect("String write");
    }
    writeln!(&mut rendered, "\n## Reviews").expect("String write");
    for review in reviews {
        writeln!(&mut rendered, "\n### Review {}", review.id).expect("String write");
        writeln!(&mut rendered, "State: {}", review.state).expect("String write");
        writeln!(&mut rendered, "Author: {}", author(&review.user)).expect("String write");
        writeln!(
            &mut rendered,
            "Submitted: {}",
            review.submitted_at.as_deref().unwrap_or("pending")
        )
        .expect("String write");
        writeln!(&mut rendered, "Reference: {base}/reviews/{}\n", review.id).expect("String write");
        writeln!(&mut rendered, "{}", review.body).expect("String write");
    }
    writeln!(&mut rendered, "\n## Inline review comments").expect("String write");
    for comment in review_comments {
        writeln!(&mut rendered, "\n### Inline comment {}", comment.id).expect("String write");
        writeln!(&mut rendered, "Author: {}", author(&comment.user)).expect("String write");
        writeln!(&mut rendered, "Path: {}", comment.path).expect("String write");
        writeln!(&mut rendered, "Created: {}", comment.created_at).expect("String write");
        writeln!(&mut rendered, "Updated: {}", comment.updated_at).expect("String write");
        writeln!(
            &mut rendered,
            "Reference: {base}/review-comments/{}\n",
            comment.id
        )
        .expect("String write");
        writeln!(&mut rendered, "{}", comment.body).expect("String write");
    }
    writeln!(&mut rendered, "\n## Diff").expect("String write");
    for (offset, file) in files.iter().enumerate() {
        if file.patch.is_none() {
            writeln!(
                &mut rendered,
                "- {base}/diff/{index} — {} ({}; patch unavailable)",
                file.filename,
                file.status,
                index = offset + 1
            )
            .expect("String write");
        } else {
            writeln!(
                &mut rendered,
                "- {base}/diff/{index} — {} ({})",
                file.filename,
                file.status,
                index = offset + 1
            )
            .expect("String write");
        }
    }
    rendered
}

pub(super) fn comment(comment: &ConversationComment) -> String {
    format!(
        "Author: {}\nCreated: {}\nUpdated: {}\n\n{}",
        author(&comment.user),
        comment.created_at,
        comment.updated_at,
        body(comment.body.as_deref())
    )
}

pub(super) fn review(review: &Review) -> String {
    format!(
        "State: {}\nAuthor: {}\nSubmitted: {}\nCommit: {}\n\n{}",
        review.state,
        author(&review.user),
        review.submitted_at.as_deref().unwrap_or("pending"),
        review.commit_id.as_deref().unwrap_or("unavailable"),
        review.body
    )
}

pub(super) fn review_comment(comment: &ReviewComment) -> String {
    let mut rendered = format!(
        "Author: {}\nPath: {}\nCreated: {}\nUpdated: {}\n",
        author(&comment.user),
        comment.path,
        comment.created_at,
        comment.updated_at
    );
    if let Some(review_id) = comment.pull_request_review_id {
        writeln!(&mut rendered, "Review ID: {review_id}").expect("String write");
    }
    writeln!(
        &mut rendered,
        "Diff hunk: {}\n\n{}",
        comment.diff_hunk, comment.body
    )
    .expect("String write");
    rendered
}

pub(super) fn diff_file(file: &DiffFile) -> String {
    format!(
        "File: {}\nStatus: {}\nPatch: {}",
        file.filename,
        file.status,
        file.patch.as_deref().unwrap_or("unavailable")
    )
}
