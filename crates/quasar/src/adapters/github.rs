use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::clients::command_runner::CommandRunner;
use crate::config::GitHubProject;
use crate::domain::{Comment, WorkItem, WorkItemDetail, WorkSource};

type AdapterResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Deserialize)]
struct GitHubIssue {
    number: u64,
    title: String,
    url: String,
    state: String,
    assignees: Vec<GitHubUser>,
    labels: Vec<GitHubLabel>,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    author: Option<GitHubUser>,
}

// `gh issue view --json ...` returns a single object with the same base fields
// as `issue list` plus `body` and `comments`.
#[derive(Debug, Deserialize)]
struct GitHubIssueDetail {
    #[serde(flatten)]
    base: GitHubIssue,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    comments: Vec<GitHubComment>,
}

#[derive(Debug, Deserialize)]
struct GitHubComment {
    author: Option<GitHubUser>,
    #[serde(rename = "createdAt")]
    created_at: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
    name: String,
}

pub fn load_fixture_work_items(path: &Path) -> AdapterResult<Vec<WorkItem>> {
    let raw = fs::read_to_string(path)?;
    normalize_work_items(&raw, None)
}

pub fn load_work_items_with_runner(
    runner: &dyn CommandRunner,
    repo: &str,
    project: Option<&GitHubProject>,
) -> AdapterResult<Vec<WorkItem>> {
    let args = vec![
        "issue",
        "list",
        "--json",
        "number,title,url,state,assignees,labels,createdAt,updatedAt,author",
        // `gh` defaults to 30 open issues and has no "all" flag; use a limit high
        // enough to be effectively uncapped. Closed issues are excluded by gh's
        // default `--state open`.
        "--limit",
        "100000",
        "-R",
        repo,
    ];

    let raw = runner
        .run("gh", &args)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    let mut items = normalize_work_items(&raw, Some(repo))?;
    if let Some(project) = project {
        enrich_planning_dates(runner, &mut items, repo, project);
    }
    Ok(items)
}

pub fn load_fixture_issue_detail(path: &Path, repo: &str) -> AdapterResult<WorkItemDetail> {
    let raw = fs::read_to_string(path)?;
    normalize_issue_detail(&raw, repo)
}

pub fn fetch_issue_detail(
    runner: &dyn CommandRunner,
    repo: &str,
    number: &str,
) -> AdapterResult<WorkItemDetail> {
    let raw = runner
        .run(
            "gh",
            &[
                "issue",
                "view",
                number,
                "--json",
                "number,title,url,state,body,assignees,labels,createdAt,updatedAt,author,comments",
                "-R",
                repo,
            ],
        )
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    normalize_issue_detail(&raw, repo)
}

fn normalize_issue_detail(raw: &str, repo: &str) -> AdapterResult<WorkItemDetail> {
    let detail: GitHubIssueDetail = serde_json::from_str(raw)?;
    let comments = detail
        .comments
        .into_iter()
        .map(|comment| Comment {
            author: comment.author.map(|user| user.login),
            created_at: comment.created_at,
            body: comment.body,
        })
        .collect();
    let body = detail.body;
    let item = normalize_issue(detail.base, Some(repo));
    Ok(WorkItemDetail {
        item,
        body,
        comments,
        project_status: None,
        status_options: Vec::new(),
    })
}

// Shape of the repo-scoped GraphQL query below: each open issue carries its
// project memberships (`projectItems`) and their date field values. Field values
// are heterogeneous; only date values carry `date` + `field.name`, so those are
// optional and non-date nodes deserialize to empty structs.
#[derive(Debug, Deserialize)]
struct GraphQlResponse {
    data: GraphQlData,
}

#[derive(Debug, Deserialize)]
struct GraphQlData {
    repository: Option<GraphQlRepository>,
}

#[derive(Debug, Deserialize)]
struct GraphQlRepository {
    issues: GraphQlIssues,
}

#[derive(Debug, Deserialize)]
struct GraphQlIssues {
    nodes: Vec<GraphQlIssueNode>,
}

#[derive(Debug, Deserialize)]
struct GraphQlIssueNode {
    number: Option<u64>,
    #[serde(rename = "projectItems")]
    project_items: GraphQlProjectItems,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectItems {
    nodes: Vec<GraphQlProjectItem>,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectItem {
    project: Option<GraphQlProjectRef>,
    #[serde(rename = "fieldValues")]
    field_values: GraphQlFieldValues,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectRef {
    number: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GraphQlFieldValues {
    nodes: Vec<GraphQlFieldValue>,
}

#[derive(Debug, Default, Deserialize)]
struct GraphQlFieldValue {
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    field: Option<GraphQlFieldName>,
}

#[derive(Debug, Deserialize)]
struct GraphQlFieldName {
    #[serde(default)]
    name: Option<String>,
}

/// Best-effort: fill `start_date`/`target_date` on each item from the configured
/// Projects v2 board. Any failure (missing scope, parse error) leaves dates empty.
fn enrich_planning_dates(
    runner: &dyn CommandRunner,
    items: &mut [WorkItem],
    repo: &str,
    project: &GitHubProject,
) {
    let Some(dates) = fetch_project_dates(runner, repo, project) else {
        return;
    };
    for item in items {
        if let Some((start, target)) = dates.get(&item.external_id) {
            item.start_date = start.clone();
            item.target_date = target.clone();
        }
    }
}

/// Returns a map of `issue-number -> (start_date, target_date)` for the repo's
/// open issues that belong to the configured project, or `None` on any
/// command/parse failure. Queries the repo directly (one page for a typical
/// backlog) rather than walking the whole project board.
fn fetch_project_dates(
    runner: &dyn CommandRunner,
    repo: &str,
    project: &GitHubProject,
) -> Option<HashMap<String, (String, String)>> {
    let (owner, name) = repo.split_once('/')?;
    // Single line: `\`-continuations would strip whitespace and merge adjacent
    // field names (e.g. `number projectItems`) into invalid tokens.
    let query = "query($owner:String!,$name:String!,$endCursor:String){ \
        repository(owner:$owner,name:$name){ \
        issues(first:100,states:OPEN,after:$endCursor){ \
        pageInfo{hasNextPage endCursor} \
        nodes{ number projectItems(first:10){nodes{ project{number} \
        fieldValues(first:30){nodes{ \
        ...on ProjectV2ItemFieldDateValue{date field{...on ProjectV2FieldCommon{name}}} \
        }} }} } } } }";
    let query_arg = format!("query={query}");
    let owner_arg = format!("owner={owner}");
    let name_arg = format!("name={name}");

    let raw = match runner.run(
        "gh",
        &[
            "api",
            "graphql",
            "--paginate",
            "-f",
            &query_arg,
            "-f",
            &owner_arg,
            "-f",
            &name_arg,
        ],
    ) {
        Ok(raw) => raw,
        Err(error) => {
            // Best-effort enrichment: dates stay blank on failure. Surface why,
            // since the common cause (gh token missing the `project` scope) is
            // otherwise invisible — the UI just shows empty dates.
            eprintln!(
                "warning: could not read GitHub project #{} dates for {repo}: {error}. \
                 Dates will be blank. If the token lacks Projects access, run: \
                 gh auth refresh -s project",
                project.number
            );
            return None;
        }
    };

    let mut map = HashMap::new();
    // `gh --paginate` concatenates one JSON document per page; stream them.
    let stream = serde_json::Deserializer::from_str(&raw).into_iter::<GraphQlResponse>();
    for page in stream {
        let Ok(page) = page else { continue };
        let Some(nodes) = page.data.repository.map(|repo| repo.issues.nodes) else {
            continue;
        };
        for issue in nodes {
            let Some(number) = issue.number else { continue };
            let mut start = String::new();
            let mut target = String::new();
            for item in issue.project_items.nodes {
                if item.project.and_then(|p| p.number) != Some(project.number) {
                    continue;
                }
                for value in item.field_values.nodes {
                    let (Some(date), Some(field_name)) =
                        (value.date, value.field.and_then(|field| field.name))
                    else {
                        continue;
                    };
                    if field_name == project.start_date_field {
                        start = date;
                    } else if field_name == project.target_date_field {
                        target = date;
                    }
                }
            }
            if !start.is_empty() || !target.is_empty() {
                map.insert(number.to_string(), (start, target));
            }
        }
    }
    Some(map)
}

fn normalize_work_items(raw: &str, repo: Option<&str>) -> AdapterResult<Vec<WorkItem>> {
    let issues: Vec<GitHubIssue> = serde_json::from_str(raw)?;
    Ok(issues
        .into_iter()
        .map(|issue| normalize_issue(issue, repo))
        .collect())
}

fn normalize_issue(issue: GitHubIssue, repo: Option<&str>) -> WorkItem {
    let external_id = issue.number.to_string();
    let repo = repo
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| repo_name_from_url(&issue.url));
    let container = repo.clone();
    let id = format!("github:{repo}#{external_id}");

    WorkItem {
        source: WorkSource::GitHub,
        id,
        external_id,
        repo: Some(repo),
        title: issue.title,
        url: issue.url,
        status: issue.state.to_lowercase(),
        assignees: issue.assignees.into_iter().map(|user| user.login).collect(),
        labels: issue.labels.into_iter().map(|label| label.name).collect(),
        priority: None,
        created_at: issue.created_at,
        updated_at: issue.updated_at,
        // GitHub issues have no start/target date concept.
        start_date: String::new(),
        target_date: String::new(),
        author: issue.author.map(|user| user.login),
        container,
        source_metadata: None,
    }
}

fn repo_name_from_url(url: &str) -> String {
    let mut segments = url.split('/').filter(|segment| !segment.is_empty());
    let owner = segments.nth(2);
    let repo = segments.next();

    match (owner, repo) {
        (Some(owner), Some(repo)) => format!("{owner}/{repo}"),
        _ => "unknown/unknown".to_string(),
    }
}

/// Which planning date to write.
#[derive(Debug, Clone, Copy)]
pub enum DateField {
    Start,
    Target,
}

impl DateField {
    fn field_name<'a>(&self, project: &'a GitHubProject) -> &'a str {
        match self {
            DateField::Start => &project.start_date_field,
            DateField::Target => &project.target_date_field,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectResolveResponse {
    data: ProjectResolveData,
}
#[derive(Debug, Deserialize)]
struct ProjectResolveData {
    #[serde(default)]
    organization: Option<ProjectOwner>,
    #[serde(default)]
    user: Option<ProjectOwner>,
}
#[derive(Debug, Deserialize)]
struct ProjectOwner {
    #[serde(rename = "projectV2")]
    project: Option<ProjectNode>,
}
#[derive(Debug, Deserialize)]
struct ProjectNode {
    id: String,
    fields: ProjectFields,
}
#[derive(Debug, Deserialize)]
struct ProjectFields {
    nodes: Vec<ProjectFieldNode>,
}
#[derive(Debug, Default, Deserialize)]
struct ProjectFieldNode {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ItemResolveResponse {
    data: ItemResolveData,
}
#[derive(Debug, Deserialize)]
struct ItemResolveData {
    repository: Option<ItemRepository>,
}
#[derive(Debug, Deserialize)]
struct ItemRepository {
    issue: Option<ItemIssue>,
}
#[derive(Debug, Deserialize)]
struct ItemIssue {
    id: String,
    #[serde(rename = "projectItems")]
    project_items: ItemProjectItems,
}
#[derive(Debug, Deserialize)]
struct ItemProjectItems {
    nodes: Vec<ItemProjectItemNode>,
}
#[derive(Debug, Deserialize)]
struct ItemProjectItemNode {
    id: String,
    project: Option<ItemProjectRef>,
}
#[derive(Debug, Deserialize)]
struct ItemProjectRef {
    number: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AddItemResponse {
    data: AddItemData,
}
#[derive(Debug, Deserialize)]
struct AddItemData {
    #[serde(rename = "addProjectV2ItemById")]
    add: AddItemPayload,
}
#[derive(Debug, Deserialize)]
struct AddItemPayload {
    item: AddItemNode,
}
#[derive(Debug, Deserialize)]
struct AddItemNode {
    id: String,
}

// gh api graphql with all-string (-f) variables (ID!/String!/Date!).
fn gh_graphql(
    runner: &dyn CommandRunner,
    query: &str,
    vars: &[(&str, &str)],
) -> AdapterResult<String> {
    let query_arg = format!("query={query}");
    let mut args: Vec<String> = vec!["api".into(), "graphql".into(), "-f".into(), query_arg];
    for (name, value) in vars {
        args.push("-f".into());
        args.push(format!("{name}={value}"));
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    runner
        .run("gh", &arg_refs)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })
}

// gh api graphql where one variable is Int! (must use -F).
fn gh_graphql_with_int(
    runner: &dyn CommandRunner,
    query: &str,
    string_vars: &[(&str, &str)],
    int_var: (&str, u64),
) -> AdapterResult<String> {
    let query_arg = format!("query={query}");
    let mut args: Vec<String> = vec!["api".into(), "graphql".into(), "-f".into(), query_arg];
    for (name, value) in string_vars {
        args.push("-f".into());
        args.push(format!("{name}={value}"));
    }
    args.push("-F".into());
    args.push(format!("{}={}", int_var.0, int_var.1));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    runner
        .run("gh", &arg_refs)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })
}

/// Set (or clear when `date` is None/empty) a Projects v2 date field for an
/// issue belonging to the configured project. Resolves project/field/item node
/// ids, adds the issue to the board if absent, then runs update/clear.
pub fn set_project_date(
    runner: &dyn CommandRunner,
    repo: &str,
    number: &str,
    project: &GitHubProject,
    field: DateField,
    date: Option<&str>,
) -> AdapterResult<()> {
    let (owner, name) =
        repo.split_once('/')
            .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                format!("malformed repo slug: {repo}").into()
            })?;

    let (project_id, field_id) = resolve_project_field(runner, project, field)?;
    let (issue_node_id, item_id) = resolve_issue_item(runner, owner, name, number, project.number)?;
    let item_id = match item_id {
        Some(id) => id,
        None => add_issue_to_project(runner, &project_id, &issue_node_id)?,
    };

    match date {
        Some(value) if !value.is_empty() => {
            let query = "mutation($project:ID!,$item:ID!,$field:ID!,$date:Date!){\
                updateProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,\
                fieldId:$field,value:{date:$date}}){projectV2Item{id}}}";
            gh_graphql(
                runner,
                query,
                &[
                    ("project", &project_id),
                    ("item", &item_id),
                    ("field", &field_id),
                    ("date", value),
                ],
            )?;
        }
        _ => {
            let query = "mutation($project:ID!,$item:ID!,$field:ID!){\
                clearProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,\
                fieldId:$field}){projectV2Item{id}}}";
            gh_graphql(
                runner,
                query,
                &[
                    ("project", &project_id),
                    ("item", &item_id),
                    ("field", &field_id),
                ],
            )?;
        }
    }
    Ok(())
}

fn resolve_project_field(
    runner: &dyn CommandRunner,
    project: &GitHubProject,
    field: DateField,
) -> AdapterResult<(String, String)> {
    let target_field_name = field.field_name(project);

    // A given owner login is either an organization or a user, never both. GitHub
    // returns a non-zero `NOT_FOUND` error for the wrong owner kind, so a command
    // error on one kind must fall through to the next rather than abort; only fail
    // when both kinds are exhausted, surfacing the last underlying error.
    let mut last_error: Option<String> = None;
    for owner_kind in ["organization", "user"] {
        let query = format!(
            "query($login:String!,$num:Int!){{{owner_kind}(login:$login){{\
             projectV2(number:$num){{id fields(first:50){{nodes{{\
             ...on ProjectV2FieldCommon{{id name}}}}}}}}}}}}"
        );
        let raw = match gh_graphql_with_int(
            runner,
            &query,
            &[("login", &project.owner)],
            ("num", project.number),
        ) {
            Ok(raw) => raw,
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        let parsed: ProjectResolveResponse = match serde_json::from_str(&raw) {
            Ok(parsed) => parsed,
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        let owner = match owner_kind {
            "organization" => parsed.data.organization,
            _ => parsed.data.user,
        };
        if let Some(node) = owner.and_then(|o| o.project) {
            let field_id = node
                .fields
                .nodes
                .into_iter()
                .find(|f| f.name.as_deref() == Some(target_field_name))
                .and_then(|f| f.id)
                .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("date field '{target_field_name}' not found on project").into()
                })?;
            return Ok((node.id, field_id));
        }
    }
    Err(format!(
        "project number {} not found for owner {} (last error: {:?})",
        project.number, project.owner, last_error
    )
    .into())
}

fn resolve_issue_item(
    runner: &dyn CommandRunner,
    owner: &str,
    name: &str,
    number: &str,
    project_number: u64,
) -> AdapterResult<(String, Option<String>)> {
    let issue_number: u64 =
        number
            .parse()
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                format!("invalid issue number: {number}").into()
            })?;
    let query = "query($owner:String!,$name:String!,$number:Int!){\
        repository(owner:$owner,name:$name){issue(number:$number){id \
        projectItems(first:20){nodes{id project{number}}}}}}";
    let raw = gh_graphql_with_int(
        runner,
        query,
        &[("owner", owner), ("name", name)],
        ("number", issue_number),
    )?;
    let parsed: ItemResolveResponse = serde_json::from_str(&raw)?;
    let issue = parsed.data.repository.and_then(|r| r.issue).ok_or_else(
        || -> Box<dyn std::error::Error + Send + Sync> {
            format!("issue {owner}/{name}#{number} not found").into()
        },
    )?;
    let item_id = issue
        .project_items
        .nodes
        .into_iter()
        .find(|item| item.project.as_ref().and_then(|p| p.number) == Some(project_number))
        .map(|item| item.id);
    Ok((issue.id, item_id))
}

fn add_issue_to_project(
    runner: &dyn CommandRunner,
    project_id: &str,
    content_id: &str,
) -> AdapterResult<String> {
    let query = "mutation($project:ID!,$content:ID!){\
        addProjectV2ItemById(input:{projectId:$project,contentId:$content}){item{id}}}";
    let raw = gh_graphql(
        runner,
        query,
        &[("project", project_id), ("content", content_id)],
    )?;
    let parsed: AddItemResponse = serde_json::from_str(&raw)?;
    Ok(parsed.data.add.item.id)
}

#[derive(Debug, Deserialize)]
struct SingleSelectFieldNode {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    options: Vec<SingleSelectOption>,
}
#[derive(Debug, Deserialize)]
struct SingleSelectOption {
    id: String,
    name: String,
}
#[derive(Debug, Deserialize)]
struct SsResolveResponse {
    data: SsResolveData,
}
#[derive(Debug, Deserialize)]
struct SsResolveData {
    #[serde(default)]
    organization: Option<SsResolveOwner>,
    #[serde(default)]
    user: Option<SsResolveOwner>,
}
#[derive(Debug, Deserialize)]
struct SsResolveOwner {
    #[serde(rename = "projectV2")]
    project: Option<SsResolveProject>,
}
#[derive(Debug, Deserialize)]
struct SsResolveProject {
    id: String,
    fields: SsResolveFields,
}
#[derive(Debug, Deserialize)]
struct SsResolveFields {
    nodes: Vec<SingleSelectFieldNode>,
}

/// Resolve (project_id, field_id, option_id) for the configured single-select
/// status field. option is None when clearing.
fn resolve_project_single_select(
    runner: &dyn CommandRunner,
    project: &GitHubProject,
    option_name: Option<&str>,
) -> AdapterResult<(String, String, Option<String>)> {
    let mut last_error: Option<String> = None;
    for owner_kind in ["organization", "user"] {
        let query = format!(
            "query($login:String!,$num:Int!){{{owner_kind}(login:$login){{\
             projectV2(number:$num){{id fields(first:50){{nodes{{\
             ...on ProjectV2SingleSelectField{{id name options{{id name}}}}}}}}}}}}}}"
        );
        let raw = match gh_graphql_with_int(
            runner,
            &query,
            &[("login", &project.owner)],
            ("num", project.number),
        ) {
            Ok(raw) => raw,
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        let parsed: SsResolveResponse = match serde_json::from_str(&raw) {
            Ok(p) => p,
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        let owner = match owner_kind {
            "organization" => parsed.data.organization,
            _ => parsed.data.user,
        };
        if let Some(proj) = owner.and_then(|o| o.project) {
            let field = proj
                .fields
                .nodes
                .into_iter()
                .find(|f| f.name.as_deref() == Some(project.status_field.as_str()))
                .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                    format!(
                        "status field '{}' not found on project",
                        project.status_field
                    )
                    .into()
                })?;
            let field_id =
                field
                    .id
                    .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                        "status field has no id".into()
                    })?;
            let option_id = match option_name {
                Some(wanted) => Some(
                    field
                        .options
                        .into_iter()
                        .find(|o| o.name == wanted)
                        .map(|o| o.id)
                        .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                            format!("status option '{wanted}' not found").into()
                        })?,
                ),
                None => None,
            };
            return Ok((proj.id, field_id, option_id));
        }
    }
    Err(format!(
        "project number {} not found for owner {} (last error: {:?})",
        project.number, project.owner, last_error
    )
    .into())
}

/// Set (or clear when option_name is None) the configured Status single-select
/// for an issue belonging to the configured project. Adds the issue to the board
/// if absent.
pub fn set_project_status(
    runner: &dyn CommandRunner,
    repo: &str,
    number: &str,
    project: &GitHubProject,
    option_name: Option<&str>,
) -> AdapterResult<()> {
    let (owner, name) =
        repo.split_once('/')
            .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                format!("malformed repo slug: {repo}").into()
            })?;
    let (project_id, field_id, option_id) =
        resolve_project_single_select(runner, project, option_name)?;
    let (issue_node_id, item_id) = resolve_issue_item(runner, owner, name, number, project.number)?;
    let item_id = match item_id {
        Some(id) => id,
        None => add_issue_to_project(runner, &project_id, &issue_node_id)?,
    };

    match option_id {
        Some(option) => {
            let query = "mutation($project:ID!,$item:ID!,$field:ID!,$option:String!){\
                updateProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,\
                fieldId:$field,value:{singleSelectOptionId:$option}}){projectV2Item{id}}}";
            gh_graphql(
                runner,
                query,
                &[
                    ("project", &project_id),
                    ("item", &item_id),
                    ("field", &field_id),
                    ("option", &option),
                ],
            )?;
        }
        None => {
            let query = "mutation($project:ID!,$item:ID!,$field:ID!){\
                clearProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,\
                fieldId:$field}){projectV2Item{id}}}";
            gh_graphql(
                runner,
                query,
                &[
                    ("project", &project_id),
                    ("item", &item_id),
                    ("field", &field_id),
                ],
            )?;
        }
    }
    Ok(())
}

/// Result of enriching a single issue's detail with its configured-project fields.
#[derive(Debug, Default)]
pub struct DetailProjectFields {
    pub start_date: String,
    pub target_date: String,
    pub project_status: Option<String>,
    pub status_options: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DetailEnrichResponse {
    data: DetailEnrichData,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichData {
    repository: Option<DetailEnrichRepo>,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichRepo {
    issue: Option<DetailEnrichIssue>,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichIssue {
    #[serde(rename = "projectItems")]
    project_items: DetailEnrichItems,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichItems {
    nodes: Vec<DetailEnrichItem>,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichItem {
    project: Option<DetailEnrichProject>,
    #[serde(rename = "fieldValues")]
    field_values: DetailEnrichFieldValues,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichProject {
    number: Option<u64>,
    #[serde(default)]
    status: Option<DetailEnrichStatusField>,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichStatusField {
    #[serde(default)]
    options: Vec<DetailEnrichOption>,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichOption {
    name: String,
}
#[derive(Debug, Default, Deserialize)]
struct DetailEnrichFieldValues {
    nodes: Vec<DetailEnrichValue>,
}
#[derive(Debug, Default, Deserialize)]
struct DetailEnrichValue {
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    field: Option<GraphQlFieldName>,
}

/// Best-effort: fetch the item's date values, current Status, and the project's
/// Status options for a single issue. Returns defaults (empty) on any failure or
/// when the issue is not on the configured board.
pub fn enrich_detail_project_fields(
    runner: &dyn CommandRunner,
    repo: &str,
    number: &str,
    project: &GitHubProject,
) -> DetailProjectFields {
    let mut out = DetailProjectFields::default();
    let Some((owner, name)) = repo.split_once('/') else {
        return out;
    };
    let Ok(issue_number) = number.parse::<u64>() else {
        return out;
    };

    let query = "query($owner:String!,$name:String!,$number:Int!,$statusField:String!){\
        repository(owner:$owner,name:$name){issue(number:$number){\
        projectItems(first:20){nodes{ project{ number \
        status: field(name:$statusField){...on ProjectV2SingleSelectField{options{name}}} } \
        fieldValues(first:30){nodes{ \
        ...on ProjectV2ItemFieldDateValue{date field{...on ProjectV2FieldCommon{name}}} \
        ...on ProjectV2ItemFieldSingleSelectValue{name field{...on ProjectV2FieldCommon{name}}} \
        }} }} }}}";

    let raw = match gh_graphql_with_int(
        runner,
        query,
        &[
            ("owner", owner),
            ("name", name),
            ("statusField", &project.status_field),
        ],
        ("number", issue_number),
    ) {
        Ok(raw) => raw,
        Err(error) => {
            // Best-effort: leave dates/status blank on failure, but surface the
            // reason (commonly a gh token missing the `project` scope).
            eprintln!(
                "warning: could not read GitHub project #{} fields for {repo}#{number}: \
                 {error}. Dates/status will be blank. If the token lacks Projects access, \
                 run: gh auth refresh -s project",
                project.number
            );
            return out;
        }
    };
    let parsed: DetailEnrichResponse = match serde_json::from_str(&raw) {
        Ok(p) => p,
        Err(_) => return out,
    };
    let Some(issue) = parsed.data.repository.and_then(|r| r.issue) else {
        return out;
    };

    for item in issue.project_items.nodes {
        let Some(proj) = item.project else { continue };
        if proj.number != Some(project.number) {
            continue;
        }
        if let Some(status_field) = proj.status {
            out.status_options = status_field.options.into_iter().map(|o| o.name).collect();
        }
        for value in item.field_values.nodes {
            let field_name = value.field.and_then(|f| f.name);
            match (value.date, value.name, field_name) {
                (Some(date), _, Some(fname)) if fname == project.start_date_field => {
                    out.start_date = date
                }
                (Some(date), _, Some(fname)) if fname == project.target_date_field => {
                    out.target_date = date
                }
                (None, Some(sel), Some(fname)) if fname == project.status_field => {
                    out.project_status = Some(sel)
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Mutex};

    use crate::clients::command_runner::{CommandResult, CommandRunner, CommandRunnerError};
    use crate::config::GitHubProject;

    use super::{load_fixture_work_items, load_work_items_with_runner};

    fn test_project() -> GitHubProject {
        GitHubProject {
            owner: "QuEraComputing".to_string(),
            number: 18,
            start_date_field: "Start date".to_string(),
            target_date_field: "Target date".to_string(),
            status_field: "Status".to_string(),
        }
    }

    struct MockCommandRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        result: CommandResult<String>,
    }

    impl MockCommandRunner {
        fn success(payload: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Ok(payload.to_string()),
            }
        }

        fn failure(message: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Err(CommandRunnerError::new(message)),
            }
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run(&self, program: &str, args: &[&str]) -> CommandResult<String> {
            self.calls
                .lock()
                .expect("calls mutex should not be poisoned")
                .push((
                    program.to_string(),
                    args.iter().map(|arg| (*arg).to_string()).collect(),
                ));
            self.result.clone()
        }
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("github")
            .join("issues.json")
    }

    fn detail_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("github")
            .join("issue-detail.json")
    }

    #[test]
    fn github_fixture_detail_normalizes_body_and_comments() {
        let detail = super::load_fixture_issue_detail(&detail_fixture_path(), "openai/quasar")
            .expect("detail fixture should load");

        assert_eq!(detail.item.id, "github:openai/quasar#123");
        assert_eq!(detail.item.status, "open");
        assert!(detail
            .body
            .as_deref()
            .unwrap()
            .contains("sync job drops events"));
        assert_eq!(detail.comments.len(), 2);
        assert_eq!(detail.comments[0].author.as_deref(), Some("kai"));
        assert_eq!(detail.comments[0].body, "I can repro on staging.");
    }

    #[test]
    fn github_detail_runner_invokes_expected_cli_arguments() {
        let payload = std::fs::read_to_string(detail_fixture_path()).expect("fixture should read");
        let runner = MockCommandRunner::success(&payload);

        let detail =
            super::fetch_issue_detail(&runner, "openai/quasar", "123").expect("detail should load");

        let calls = runner
            .calls
            .lock()
            .expect("calls mutex should not be poisoned");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "gh");
        assert_eq!(
            calls[0].1,
            vec![
                "issue",
                "view",
                "123",
                "--json",
                "number,title,url,state,body,assignees,labels,createdAt,updatedAt,author,comments",
                "-R",
                "openai/quasar",
            ]
        );
        assert_eq!(detail.item.external_id, "123");
    }

    #[test]
    fn github_detail_runner_propagates_cli_failures() {
        let runner = MockCommandRunner::failure("gh not found");
        let error = super::fetch_issue_detail(&runner, "openai/quasar", "123")
            .expect_err("runner should fail");
        assert!(error.to_string().contains("gh not found"));
    }

    #[test]
    fn github_detail_defaults_body_and_comments_when_absent() {
        let payload = r#"{"number":7,"title":"t","url":"https://github.com/o/r/issues/7","state":"OPEN","assignees":[],"labels":[],"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","author":{"login":"a"}}"#;
        let runner = MockCommandRunner::success(payload);

        let detail = super::fetch_issue_detail(&runner, "o/r", "7").expect("detail should load");

        assert_eq!(detail.body, None);
        assert!(detail.comments.is_empty());
    }

    #[test]
    fn github_fixture_normalizes_into_work_items() {
        let items = load_fixture_work_items(&fixture_path()).expect("fixture should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "github:openai/quasar#123");
        assert_eq!(items[0].external_id, "123");
        assert_eq!(items[0].repo.as_deref(), Some("openai/quasar"));
        assert_eq!(items[0].source.to_string(), "github");
        assert_eq!(items[0].container, "openai/quasar");
    }

    #[test]
    fn github_runner_invokes_expected_cli_arguments() {
        let payload = std::fs::read_to_string(fixture_path()).expect("fixture should read");
        let runner = MockCommandRunner::success(&payload);

        let items = load_work_items_with_runner(&runner, "openai/quasar", Some(&test_project()))
            .expect("runner payload should load");

        let calls = runner
            .calls
            .lock()
            .expect("calls mutex should not be poisoned");
        // Issue list, then a Projects v2 GraphQL query to enrich planning dates.
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "gh");
        assert_eq!(
            calls[0].1,
            vec![
                "issue",
                "list",
                "--json",
                "number,title,url,state,assignees,labels,createdAt,updatedAt,author",
                "--limit",
                "100000",
                "-R",
                "openai/quasar",
            ]
        );
        assert_eq!(calls[1].0, "gh");
        assert_eq!(&calls[1].1[0..3], &["api", "graphql", "--paginate"]);
        assert!(calls[1].1.iter().any(|arg| arg == "owner=openai"));
        assert!(calls[1].1.iter().any(|arg| arg == "name=quasar"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].repo.as_deref(), Some("openai/quasar"));
        assert_eq!(items[0].container, "openai/quasar");
    }

    #[test]
    fn github_enrichment_applies_project_dates() {
        struct RoutingRunner {
            issues: String,
            project: String,
        }

        impl CommandRunner for RoutingRunner {
            fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
                if args.contains(&"graphql") {
                    Ok(self.project.clone())
                } else {
                    Ok(self.issues.clone())
                }
            }
        }

        let runner = RoutingRunner {
            issues: std::fs::read_to_string(fixture_path()).expect("fixture should read"),
            project: r#"{"data":{"repository":{"issues":{
                "pageInfo":{"hasNextPage":false,"endCursor":null},
                "nodes":[
                  {"number":123,"projectItems":{"nodes":[
                    {"project":{"number":18},"fieldValues":{"nodes":[
                      {"date":"2026-06-01","field":{"name":"Start date"}},
                      {"date":"2026-06-15","field":{"name":"Target date"}},
                      {}
                    ]}}
                  ]}}
                ]}}}}}"#
                .to_string(),
        };

        let items = load_work_items_with_runner(&runner, "openai/quasar", Some(&test_project()))
            .expect("runner payload should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].start_date, "2026-06-01");
        assert_eq!(items[0].target_date, "2026-06-15");
    }

    #[test]
    fn github_items_use_repo_qualified_ids_across_repos() {
        let payload = std::fs::read_to_string(fixture_path()).expect("fixture should read");
        let first_runner = MockCommandRunner::success(&payload);
        let second_runner = MockCommandRunner::success(&payload);

        let first_items = load_work_items_with_runner(&first_runner, "openai/quasar", None)
            .expect("first runner payload should load");
        let second_items = load_work_items_with_runner(&second_runner, "rust-lang/rust", None)
            .expect("second runner payload should load");

        assert_eq!(first_items[0].external_id, second_items[0].external_id);
        assert_ne!(first_items[0].id, second_items[0].id);
        assert_eq!(first_items[0].id, "github:openai/quasar#123");
        assert_eq!(second_items[0].id, "github:rust-lang/rust#123");
        assert_eq!(second_items[0].container, "rust-lang/rust");
    }

    #[test]
    fn github_runner_propagates_cli_failures() {
        let runner = MockCommandRunner::failure("gh auth expired");

        let error = load_work_items_with_runner(&runner, "openai/quasar", None)
            .expect_err("runner should fail");

        assert!(error.to_string().contains("gh auth expired"));
    }

    #[test]
    fn set_project_date_resolves_ids_then_updates_existing_item() {
        use std::sync::Mutex;

        struct SeqRunner {
            calls: Mutex<Vec<String>>, // the `-f query=...` value for each call
        }
        impl CommandRunner for SeqRunner {
            fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
                let query = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("")
                    .to_string();
                self.calls.lock().unwrap().push(query.clone());
                if query.contains("projectV2(number") {
                    Ok(
                        r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                        {"id":"FLD_START","name":"Start date"},
                        {"id":"FLD_TARGET","name":"Target date"}
                    ]}}}}}"#
                            .to_string(),
                    )
                } else if query.contains("issue(number") {
                    Ok(
                        r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                        {"id":"ITEM_1","project":{"number":18}}
                    ]}}}}}"#
                            .to_string(),
                    )
                } else if query.contains("updateProjectV2ItemFieldValue") {
                    Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#
                        .to_string())
                } else {
                    Err(CommandRunnerError::new(format!(
                        "unexpected query: {query}"
                    )))
                }
            }
        }

        let runner = SeqRunner {
            calls: Mutex::new(Vec::new()),
        };
        let result = super::set_project_date(
            &runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            super::DateField::Start,
            Some("2026-07-01"),
        );
        assert!(result.is_ok(), "expected ok, got {result:?}");

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(calls[0].contains("projectV2(number"));
        assert!(calls[1].contains("issue(number"));
        assert!(calls[2].contains("updateProjectV2ItemFieldValue"));
    }

    #[test]
    fn set_project_date_falls_back_from_organization_to_user_owner() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;

        struct SeqRunner {
            resolve_attempts: AtomicUsize,
            calls: Mutex<Vec<String>>,
        }
        impl CommandRunner for SeqRunner {
            fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
                let query = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("")
                    .to_string();
                self.calls.lock().unwrap().push(query.clone());
                if query.contains("projectV2(number") {
                    // First resolve attempt targets `organization(login:)` and, for a
                    // user-owned project, GitHub replies NOT_FOUND with a non-zero exit.
                    // The second attempt targets `user(login:)` and succeeds.
                    let attempt = self.resolve_attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        Err(CommandRunnerError::new(
                            "GraphQL: Could not resolve to an Organization with the login of 'someuser'. (NOT_FOUND)",
                        ))
                    } else {
                        Ok(
                            r#"{"data":{"user":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                            {"id":"FLD_START","name":"Start date"},
                            {"id":"FLD_TARGET","name":"Target date"}
                        ]}}}}}"#
                                .to_string(),
                        )
                    }
                } else if query.contains("issue(number") {
                    Ok(
                        r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                        {"id":"ITEM_1","project":{"number":18}}
                    ]}}}}}"#
                            .to_string(),
                    )
                } else if query.contains("updateProjectV2ItemFieldValue") {
                    Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#
                        .to_string())
                } else {
                    Err(CommandRunnerError::new(format!(
                        "unexpected query: {query}"
                    )))
                }
            }
        }

        let runner = SeqRunner {
            resolve_attempts: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
        };
        let result = super::set_project_date(
            &runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            super::DateField::Start,
            Some("2026-07-01"),
        );
        assert!(result.is_ok(), "expected ok, got {result:?}");

        let calls = runner.calls.lock().unwrap();
        // Two resolve attempts (org fails, user succeeds), then issue + update.
        let resolve_calls = calls
            .iter()
            .filter(|q| q.contains("projectV2(number"))
            .count();
        assert_eq!(resolve_calls, 2, "should attempt org then user: {calls:?}");
        assert_eq!(runner.resolve_attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn set_project_date_adds_issue_to_board_when_missing_then_updates() {
        use std::sync::Mutex;
        struct SeqRunner {
            calls: Mutex<Vec<String>>,
        }
        impl CommandRunner for SeqRunner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let q = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("")
                    .to_string();
                self.calls.lock().unwrap().push(q.clone());
                if q.contains("projectV2(number") {
                    Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_START","name":"Start date"},{"id":"FLD_TARGET","name":"Target date"}]}}}}}"#.to_string())
                } else if q.contains("issue(number") {
                    Ok(r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[]}}}}}"#.to_string())
                } else if q.contains("addProjectV2ItemById") {
                    Ok(
                        r#"{"data":{"addProjectV2ItemById":{"item":{"id":"ITEM_NEW"}}}}"#
                            .to_string(),
                    )
                } else if q.contains("updateProjectV2ItemFieldValue") {
                    Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_NEW"}}}}"#.to_string())
                } else {
                    Err(CommandRunnerError::new("unexpected"))
                }
            }
        }
        let runner = SeqRunner {
            calls: Mutex::new(Vec::new()),
        };
        super::set_project_date(
            &runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            super::DateField::Target,
            Some("2026-07-20"),
        )
        .expect("ok");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert!(calls[2].contains("addProjectV2ItemById"));
        assert!(calls[3].contains("updateProjectV2ItemFieldValue"));
    }

    #[test]
    fn enrich_detail_reads_dates_status_and_options() {
        struct Runner;
        impl CommandRunner for Runner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let q = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("");
                assert!(q.contains("issue(number"), "unexpected query: {q}");
                Ok(r#"{"data":{"repository":{"issue":{"projectItems":{"nodes":[
                    {"project":{"number":18,"status":{"options":[{"name":"Todo"},{"name":"In Progress"},{"name":"Done"}]}},
                     "fieldValues":{"nodes":[
                        {"date":"2026-06-01","field":{"name":"Start date"}},
                        {"date":"2026-06-15","field":{"name":"Target date"}},
                        {"name":"In Progress","field":{"name":"Status"}},
                        {}
                     ]}}
                ]}}}}}"#.to_string())
            }
        }
        let fields = super::enrich_detail_project_fields(
            &Runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
        );
        assert_eq!(fields.start_date, "2026-06-01");
        assert_eq!(fields.target_date, "2026-06-15");
        assert_eq!(fields.project_status.as_deref(), Some("In Progress"));
        assert_eq!(fields.status_options, vec!["Todo", "In Progress", "Done"]);
    }

    #[test]
    fn enrich_detail_returns_empty_when_issue_not_on_board() {
        struct Runner;
        impl CommandRunner for Runner {
            fn run(&self, _p: &str, _args: &[&str]) -> CommandResult<String> {
                Ok(
                    r#"{"data":{"repository":{"issue":{"projectItems":{"nodes":[]}}}}}"#
                        .to_string(),
                )
            }
        }
        let fields = super::enrich_detail_project_fields(
            &Runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
        );
        assert_eq!(fields.start_date, "");
        assert_eq!(fields.project_status, None);
        assert!(fields.status_options.is_empty());
    }

    #[test]
    fn enrich_detail_returns_empty_on_command_error() {
        struct Runner;
        impl CommandRunner for Runner {
            fn run(&self, _p: &str, _args: &[&str]) -> CommandResult<String> {
                Err(CommandRunnerError::new("gh boom"))
            }
        }
        let fields = super::enrich_detail_project_fields(
            &Runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
        );
        assert_eq!(fields.start_date, "");
        assert_eq!(fields.project_status, None);
    }

    #[test]
    fn set_project_date_clears_when_date_empty() {
        use std::sync::Mutex;
        struct SeqRunner {
            calls: Mutex<Vec<String>>,
        }
        impl CommandRunner for SeqRunner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let q = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("")
                    .to_string();
                self.calls.lock().unwrap().push(q.clone());
                if q.contains("projectV2(number") {
                    Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_START","name":"Start date"},{"id":"FLD_TARGET","name":"Target date"}]}}}}}"#.to_string())
                } else if q.contains("issue(number") {
                    Ok(
                        r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                    {"id":"ITEM_1","project":{"number":18}}]}}}}}"#
                            .to_string(),
                    )
                } else if q.contains("clearProjectV2ItemFieldValue") {
                    Ok(r#"{"data":{"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#.to_string())
                } else {
                    Err(CommandRunnerError::new("unexpected"))
                }
            }
        }
        let runner = SeqRunner {
            calls: Mutex::new(Vec::new()),
        };
        super::set_project_date(
            &runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            super::DateField::Start,
            None,
        )
        .expect("ok");
        let calls = runner.calls.lock().unwrap();
        assert!(calls
            .last()
            .unwrap()
            .contains("clearProjectV2ItemFieldValue"));
    }

    #[test]
    fn set_project_status_resolves_ids_then_updates() {
        use std::sync::Mutex;
        struct SeqRunner {
            calls: Mutex<Vec<String>>,
        }
        impl CommandRunner for SeqRunner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let q = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("")
                    .to_string();
                self.calls.lock().unwrap().push(q.clone());
                if q.contains("projectV2(number") {
                    Ok(
                        r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_STATUS","name":"Status","options":[
                        {"id":"OPT_TODO","name":"Todo"},{"id":"OPT_DONE","name":"Done"}]}
                ]}}}}}"#
                            .to_string(),
                    )
                } else if q.contains("issue(number") {
                    Ok(
                        r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                    {"id":"ITEM_1","project":{"number":18}}]}}}}}"#
                            .to_string(),
                    )
                } else if q.contains("updateProjectV2ItemFieldValue") {
                    assert!(
                        q.contains("singleSelectOptionId"),
                        "status update must use singleSelectOptionId"
                    );
                    Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#.to_string())
                } else {
                    Err(CommandRunnerError::new("unexpected"))
                }
            }
        }
        let runner = SeqRunner {
            calls: Mutex::new(Vec::new()),
        };
        super::set_project_status(
            &runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            Some("Done"),
        )
        .expect("ok");
        let calls = runner.calls.lock().unwrap();
        assert!(calls[0].contains("projectV2(number"));
        assert!(calls[1].contains("issue(number"));
        assert!(calls
            .last()
            .unwrap()
            .contains("updateProjectV2ItemFieldValue"));
    }

    #[test]
    fn set_project_status_adds_to_board_when_missing() {
        use std::sync::Mutex;
        struct SeqRunner {
            calls: Mutex<Vec<String>>,
        }
        impl CommandRunner for SeqRunner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let q = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("")
                    .to_string();
                self.calls.lock().unwrap().push(q.clone());
                if q.contains("projectV2(number") {
                    Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_STATUS","name":"Status","options":[{"id":"OPT_DONE","name":"Done"}]}]}}}}}"#.to_string())
                } else if q.contains("issue(number") {
                    Ok(r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[]}}}}}"#.to_string())
                } else if q.contains("addProjectV2ItemById") {
                    Ok(
                        r#"{"data":{"addProjectV2ItemById":{"item":{"id":"ITEM_NEW"}}}}"#
                            .to_string(),
                    )
                } else if q.contains("updateProjectV2ItemFieldValue") {
                    Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_NEW"}}}}"#.to_string())
                } else {
                    Err(CommandRunnerError::new("unexpected"))
                }
            }
        }
        let runner = SeqRunner {
            calls: Mutex::new(Vec::new()),
        };
        super::set_project_status(
            &runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            Some("Done"),
        )
        .expect("ok");
        let calls = runner.calls.lock().unwrap();
        assert!(calls.iter().any(|q| q.contains("addProjectV2ItemById")));
        assert!(calls
            .last()
            .unwrap()
            .contains("updateProjectV2ItemFieldValue"));
    }

    #[test]
    fn set_project_status_clears_when_none() {
        struct Runner;
        impl CommandRunner for Runner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let q = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("");
                if q.contains("projectV2(number") {
                    Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_STATUS","name":"Status","options":[{"id":"OPT_TODO","name":"Todo"}]}]}}}}}"#.to_string())
                } else if q.contains("issue(number") {
                    Ok(
                        r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                    {"id":"ITEM_1","project":{"number":18}}]}}}}}"#
                            .to_string(),
                    )
                } else if q.contains("clearProjectV2ItemFieldValue") {
                    Ok(r#"{"data":{"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#.to_string())
                } else {
                    Err(CommandRunnerError::new("unexpected"))
                }
            }
        }
        super::set_project_status(
            &Runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            None,
        )
        .expect("ok");
    }

    #[test]
    fn set_project_status_falls_back_from_organization_to_user_owner() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;

        struct SeqRunner {
            resolve_attempts: AtomicUsize,
            calls: Mutex<Vec<String>>,
        }
        impl CommandRunner for SeqRunner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let q = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("")
                    .to_string();
                self.calls.lock().unwrap().push(q.clone());
                if q.contains("projectV2(number") {
                    // First resolve attempt targets `organization(login:)` and, for a
                    // user-owned project, GitHub replies NOT_FOUND with a non-zero exit.
                    // The second attempt targets `user(login:)` and succeeds.
                    let attempt = self.resolve_attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        Err(CommandRunnerError::new(
                            "GraphQL: Could not resolve to an Organization with the login of 'someuser'. (NOT_FOUND)",
                        ))
                    } else {
                        Ok(r#"{"data":{"user":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                        {"id":"FLD_STATUS","name":"Status","options":[{"id":"OPT_DONE","name":"Done"}]}]}}}}}"#.to_string())
                    }
                } else if q.contains("issue(number") {
                    Ok(
                        r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                    {"id":"ITEM_1","project":{"number":18}}]}}}}}"#
                            .to_string(),
                    )
                } else if q.contains("updateProjectV2ItemFieldValue") {
                    Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#.to_string())
                } else {
                    Err(CommandRunnerError::new("unexpected"))
                }
            }
        }

        let runner = SeqRunner {
            resolve_attempts: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
        };
        super::set_project_status(
            &runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            Some("Done"),
        )
        .expect("ok");

        let calls = runner.calls.lock().unwrap();
        let resolve_calls = calls
            .iter()
            .filter(|q| q.contains("projectV2(number"))
            .count();
        assert_eq!(resolve_calls, 2, "should attempt org then user: {calls:?}");
        assert_eq!(runner.resolve_attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn set_project_status_errors_on_unknown_option() {
        struct Runner;
        impl CommandRunner for Runner {
            fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
                let q = args
                    .iter()
                    .find_map(|a| a.strip_prefix("query="))
                    .unwrap_or("");
                if q.contains("projectV2(number") {
                    Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_STATUS","name":"Status","options":[{"id":"OPT_TODO","name":"Todo"}]}]}}}}}"#.to_string())
                } else {
                    Ok(r#"{"data":{"repository":{"issue":{"id":"I","projectItems":{"nodes":[{"id":"ITEM_1","project":{"number":18}}]}}}}}"#.to_string())
                }
            }
        }
        let err = super::set_project_status(
            &Runner,
            "QuEraComputing/quasar",
            "123",
            &test_project(),
            Some("Nope"),
        )
        .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("option"));
    }
}
