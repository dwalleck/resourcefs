use std::fmt::Write as _;

use resourcefs_core::{AtlassianSiteId, JiraAddress, PathReference, ResourceError};

use super::{escape_markdown, quoted};
use crate::atlassian::wire::collections::{JiraProject, Presence};

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
