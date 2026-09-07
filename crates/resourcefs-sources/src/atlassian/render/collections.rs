use std::fmt::Write as _;

use resourcefs_core::{
    AtlassianSiteId, JiraAddress, JiraIssueResource, PathReference, ResourceError,
};

use super::{escape_markdown, quoted};
use crate::atlassian::wire::collections::{JiraIssueSummary, JiraProject, Presence};

pub(crate) fn render_projects(
    site: &AtlassianSiteId,
    rows: &[JiraProject],
) -> Result<String, ResourceError> {
    let reference = PathReference::jira(JiraAddress::Projects { site: site.clone() }, None)?;
    let mut output = format!(
        "# Jira Projects\n\nCanonical Reference: {}\n\n",
        reference.requested()
    );
    if rows.is_empty() {
        output.push_str("No visible projects in this page.\n");
    }
    for project in rows {
        let reference = project_reference(site, project)?;
        writeln!(
            output,
            "## [{}]({reference})\n",
            escape_markdown(&quoted(project.key.as_str()))
        )
        .expect("writing to a String cannot fail");
        render_metadata(&mut output, project);
        writeln!(output, "Reference: {reference}\n").expect("writing to a String cannot fail");
    }
    Ok(output)
}

pub(crate) fn render_project(
    site: &AtlassianSiteId,
    project: &JiraProject,
) -> Result<String, ResourceError> {
    let reference = project_reference(site, project)?;
    let mut output = format!(
        "# Jira Project {}\n\nCanonical Reference: {reference}\n",
        escape_markdown(&quoted(project.key.as_str()))
    );
    render_metadata(&mut output, project);
    let issues = PathReference::jira(
        JiraAddress::ProjectIssues {
            site: site.clone(),
            project: project.id.clone(),
        },
        None,
    )?;
    writeln!(output, "\n[Issues]({})", issues.requested())
        .expect("writing to a String cannot fail");
    Ok(output)
}

fn project_reference(
    site: &AtlassianSiteId,
    project: &JiraProject,
) -> Result<String, ResourceError> {
    PathReference::jira(
        JiraAddress::Project {
            site: site.clone(),
            project_id: project.id.clone(),
        },
        None,
    )
    .map(|reference| reference.requested().to_owned())
}

fn render_metadata(output: &mut String, project: &JiraProject) {
    writeln!(
        output,
        "Project ID: {}\nProject Key: {}\nName: {}\nSelf: {}",
        project.id.as_str(),
        quoted(project.key.as_str()),
        quoted(&project.name),
        quoted(&project.self_url)
    )
    .expect("writing to a String cannot fail");
    render_presence(
        output,
        "Project Type",
        &project.project_type_key,
        |output, value| output.push_str(&quoted(value)),
    );
    render_presence(output, "Archived", &project.archived, render_bool);
    render_presence(output, "Deleted", &project.deleted, render_bool);
}

fn render_bool(output: &mut String, value: &bool) {
    output.push_str(if *value { "true" } else { "false" });
}

fn render_presence<T>(
    output: &mut String,
    label: &str,
    value: &Presence<T>,
    render: impl FnOnce(&mut String, &T),
) {
    match value {
        Presence::Absent => return,
        Presence::Null => {
            write!(output, "{label}: null").expect("writing to a String cannot fail");
        }
        Presence::Value(value) => {
            write!(output, "{label}: ").expect("writing to a String cannot fail");
            render(output, value);
        }
    }
    output.push('\n');
}

pub(crate) fn render_issues(
    address: &JiraAddress,
    rows: &[JiraIssueSummary],
) -> Result<String, ResourceError> {
    let reference = PathReference::jira(address.clone(), None)?;
    let site = address.site();
    let mut output = format!(
        "# Jira Issues\n\nCanonical Reference: {}\n\n",
        reference.requested()
    );
    if rows.is_empty() {
        output.push_str("No visible issues in this page.\n");
    }
    for issue in rows {
        let reference = PathReference::jira(
            JiraAddress::Issue {
                site: site.clone(),
                issue_id: issue.id.clone(),
                resource: JiraIssueResource::Aggregate,
            },
            None,
        )?;
        let project = PathReference::jira(
            JiraAddress::Project {
                site: site.clone(),
                project_id: issue.project_id.clone(),
            },
            None,
        )?;
        writeln!(
            output,
            "## [{}]({})\n\nIssue ID: {}\nIssue Key: {}\nSummary: {}\nSelf: {}\nProject: [{}]({})",
            escape_markdown(&quoted(issue.key.as_str())),
            reference.requested(),
            issue.id.as_str(),
            quoted(issue.key.as_str()),
            quoted(&issue.summary),
            quoted(&issue.self_url),
            issue.project_id.as_str(),
            project.requested()
        )
        .expect("writing to a String cannot fail");
        render_presence(&mut output, "Status", &issue.status, |output, status| {
            output.push('{');
            match &status.name {
                Presence::Absent => {}
                Presence::Null => output.push_str("name: null"),
                Presence::Value(name) => {
                    output.push_str("name: ");
                    output.push_str(&quoted(name));
                }
            }
            output.push('}');
        });
        writeln!(output, "Reference: {}\n", reference.requested())
            .expect("writing to a String cannot fail");
    }
    Ok(output)
}
