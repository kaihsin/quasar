use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Deserialize;

use crate::clients::command_runner::CommandRunner;
use crate::config::JiraConfig;
use crate::domain::{Comment, WorkItem, WorkItemDetail, WorkSource};

type AdapterResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Planning-date custom fields on this Jira site's board:
/// `customfield_10022` = "Target start", `customfield_10023` = "Target end".
/// Keep these in sync with the `#[serde(rename = ...)]` on `JiraViewFields`.
const JIRA_TARGET_START_FIELD: &str = "customfield_10022";
const JIRA_TARGET_END_FIELD: &str = "customfield_10023";

/// Max concurrent `acli workitem view` processes when enriching planning dates.
const ENRICH_CONCURRENCY: usize = 8;

// `acli jira workitem search --json` returns Jira's native nested shape:
// `{ "key": "SSW-1", "fields": { "summary": ..., "status": { "name": ... }, ... } }`.
#[derive(Debug, Deserialize)]
struct JiraIssue {
    key: String,
    fields: JiraFields,
}

#[derive(Debug, Deserialize)]
struct JiraFields {
    summary: String,
    status: JiraStatus,
    assignee: Option<JiraPerson>,
    #[serde(default)]
    labels: Vec<String>,
    priority: Option<JiraPriority>,
    reporter: Option<JiraPerson>,
    // created/updated are NOT returned by `acli jira workitem search` (only by a
    // per-issue `workitem view --fields '*all'`). Keep optional; empty when absent.
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    updated: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JiraStatus {
    name: String,
}

#[derive(Debug, Deserialize)]
struct JiraPerson {
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(default, rename = "accountId")]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JiraPriority {
    name: String,
}

pub fn load_fixture_work_items(path: &Path, base_url: &str) -> AdapterResult<Vec<WorkItem>> {
    let raw = fs::read_to_string(path)?;
    normalize_work_items(&raw, base_url)
}

pub fn load_work_items_with_runner(
    runner: &dyn CommandRunner,
    jql: &str,
    base_url: &str,
) -> AdapterResult<Vec<WorkItem>> {
    let raw = runner
        .run(
            "acli",
            &[
                "jira",
                "workitem",
                "search",
                "--jql",
                jql,
                // Fetch every matching issue (no cap); the JQL is expected to
                // scope this (e.g. excluding done work) to keep the set small.
                "--paginate",
                "--json",
                "--fields",
                "key,summary,status,assignee,priority,reporter,labels",
            ],
        )
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    let mut items = normalize_work_items(&raw, base_url)?;
    enrich_planning_dates(runner, &mut items);
    Ok(items)
}

// `acli jira workitem view KEY --json` returns a single nested issue object;
// we only request the two planning-date fields.
#[derive(Debug, Deserialize)]
struct JiraViewIssue {
    fields: JiraViewFields,
}

#[derive(Debug, Deserialize)]
struct JiraViewFields {
    #[serde(default, rename = "customfield_10022")]
    target_start: Option<String>,
    #[serde(default, rename = "customfield_10023")]
    target_end: Option<String>,
}

// Full per-issue detail view. `description` and comment bodies may be a plain
// string or ADF (nested JSON), so deserialize as Value and extract text.
#[derive(Debug, Deserialize)]
struct JiraDetailIssue {
    key: String,
    fields: JiraDetailFields,
}

#[derive(Debug, Deserialize)]
struct JiraDetailFields {
    summary: String,
    status: JiraStatus,
    assignee: Option<JiraPerson>,
    #[serde(default)]
    labels: Vec<String>,
    priority: Option<JiraPriority>,
    reporter: Option<JiraPerson>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    description: Option<serde_json::Value>,
    #[serde(default, rename = "customfield_10022")]
    target_start: Option<String>,
    #[serde(default, rename = "customfield_10023")]
    target_end: Option<String>,
    #[serde(default)]
    comment: Option<JiraCommentContainer>,
}

#[derive(Debug, Deserialize)]
struct JiraCommentContainer {
    #[serde(default)]
    comments: Vec<JiraComment>,
}

#[derive(Debug, Deserialize)]
struct JiraComment {
    #[serde(default)]
    author: Option<JiraPerson>,
    #[serde(default)]
    created: String,
    #[serde(default)]
    body: serde_json::Value,
}

// Best-effort text: plain string as-is; ADF flattened by returning `text` leaves
// and recursing only into `content` (never `type`/`version`/`attrs`, which would
// leak structural noise); everything else (null, numbers, bools) -> empty string.
fn extract_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) => {
            items.iter().map(extract_text).collect::<Vec<_>>().join("")
        }
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(text)) = map.get("text") {
                return text.clone();
            }
            match map.get("content") {
                Some(content) => extract_text(content),
                None => String::new(),
            }
        }
        _ => String::new(),
    }
}

/// Best-effort: populate `start_date`/`target_date` on each item by fetching the
/// per-issue `view` (bulk `search` cannot return date fields). Runs the `view`
/// calls with bounded concurrency; any failed lookup simply leaves dates empty.
fn enrich_planning_dates(runner: &dyn CommandRunner, items: &mut [WorkItem]) {
    let keys: Vec<String> = items.iter().map(|item| item.external_id.clone()).collect();
    if keys.is_empty() {
        return;
    }

    let next = AtomicUsize::new(0);
    let collected: Mutex<Vec<(usize, String, String)>> = Mutex::new(Vec::new());
    let workers = keys.len().min(ENRICH_CONCURRENCY).max(1);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(key) = keys.get(index) else {
                    break;
                };
                if let Some((start, target)) = fetch_issue_dates(runner, key) {
                    collected
                        .lock()
                        .expect("enrich collected mutex should not be poisoned")
                        .push((index, start, target));
                }
            });
        }
    });

    for (index, start, target) in collected
        .into_inner()
        .expect("enrich collected mutex should not be poisoned")
    {
        items[index].start_date = start;
        items[index].target_date = target;
    }
}

/// Returns `(start_date, target_date)` for a single issue, or `None` on any
/// command/parse failure so enrichment stays best-effort.
fn fetch_issue_dates(runner: &dyn CommandRunner, key: &str) -> Option<(String, String)> {
    let fields = format!("key,{JIRA_TARGET_START_FIELD},{JIRA_TARGET_END_FIELD}");
    let raw = runner
        .run(
            "acli",
            &[
                "jira", "workitem", "view", key, "--json", "--fields", &fields,
            ],
        )
        .ok()?;
    let issue: JiraViewIssue = serde_json::from_str(&raw).ok()?;
    Some((
        issue.fields.target_start.unwrap_or_default(),
        issue.fields.target_end.unwrap_or_default(),
    ))
}

pub fn load_fixture_issue_detail(path: &Path, base_url: &str) -> AdapterResult<WorkItemDetail> {
    let raw = fs::read_to_string(path)?;
    normalize_issue_detail(&raw, base_url)
}

pub fn fetch_issue_detail(
    runner: &dyn CommandRunner,
    key: &str,
    base_url: &str,
) -> AdapterResult<WorkItemDetail> {
    let raw = runner
        .run(
            "acli",
            &[
                "jira",
                "workitem",
                "view",
                key,
                "--json",
                "--fields",
                "summary,description,comment,status,assignee,priority,reporter,labels,created,updated,customfield_10022,customfield_10023",
            ],
        )
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    normalize_issue_detail(&raw, base_url)
}

fn normalize_issue_detail(raw: &str, base_url: &str) -> AdapterResult<WorkItemDetail> {
    let issue: JiraDetailIssue = serde_json::from_str(raw)?;
    let fields = issue.fields;
    let external_id = issue.key;
    let url = format!("{}/browse/{external_id}", base_url.trim_end_matches('/'));
    let container = external_id
        .split_once('-')
        .map(|(project_key, _)| project_key.to_string())
        .unwrap_or_default();

    let body = fields
        .description
        .as_ref()
        .map(extract_text)
        .filter(|text| !text.is_empty());
    let comments = fields
        .comment
        .map(|container| container.comments)
        .unwrap_or_default()
        .into_iter()
        .map(|comment| Comment {
            author: comment.author.map(|person| person.display_name),
            created_at: comment.created,
            body: extract_text(&comment.body),
        })
        .collect();

    // Capture the current assignee's accountId (the write id) before `assignee`
    // is consumed below to build the display-name `assignees` list.
    let assignee_selected: Vec<String> = fields
        .assignee
        .as_ref()
        .and_then(|p| p.account_id.clone())
        .into_iter()
        .collect();

    let item = WorkItem {
        source: WorkSource::Jira,
        id: format!("jira:{external_id}"),
        external_id,
        repo: None,
        title: fields.summary,
        url,
        status: fields.status.name,
        assignees: fields.assignee.map(|person| person.display_name).into_iter().collect(),
        labels: fields.labels,
        priority: fields.priority.map(|priority| priority.name),
        created_at: fields.created.unwrap_or_default(),
        updated_at: fields.updated.unwrap_or_default(),
        start_date: fields.target_start.unwrap_or_default(),
        target_date: fields.target_end.unwrap_or_default(),
        author: fields.reporter.map(|person| person.display_name),
        container,
        source_metadata: None,
    };

    Ok(WorkItemDetail {
        item,
        body,
        comments,
        project_status: None,
        status_options: Vec::new(),
        assignee_options: Vec::new(),
        assignee_selected,
    })
}

fn normalize_work_items(raw: &str, base_url: &str) -> AdapterResult<Vec<WorkItem>> {
    let issues: Vec<JiraIssue> = serde_json::from_str(raw)?;
    Ok(issues
        .into_iter()
        .map(|issue| normalize_issue(issue, base_url))
        .collect())
}

fn normalize_issue(issue: JiraIssue, base_url: &str) -> WorkItem {
    let external_id = issue.key;
    let url = format!("{}/browse/{external_id}", base_url.trim_end_matches('/'));
    // Project key is the prefix of the issue key (e.g. "SSW" from "SSW-1131");
    // `search` does not return the project object.
    let container = external_id
        .split_once('-')
        .map(|(project_key, _)| project_key.to_string())
        .unwrap_or_default();
    let fields = issue.fields;

    WorkItem {
        source: WorkSource::Jira,
        id: format!("jira:{external_id}"),
        external_id,
        repo: None,
        title: fields.summary,
        url,
        status: fields.status.name,
        assignees: fields.assignee.map(|person| person.display_name).into_iter().collect(),
        labels: fields.labels,
        priority: fields.priority.map(|priority| priority.name),
        created_at: fields.created.unwrap_or_default(),
        updated_at: fields.updated.unwrap_or_default(),
        // Filled in later by `enrich_planning_dates` via per-issue `view` calls;
        // `search` cannot return date fields.
        start_date: String::new(),
        target_date: String::new(),
        author: fields.reporter.map(|person| person.display_name),
        container,
        source_metadata: None,
    }
}

/// Which planning date to write. Maps to the Advanced Roadmaps baseline custom
/// fields on this site (`customfield_10022`/`10023`).
#[derive(Debug, Clone, Copy)]
pub enum DateField {
    Start,
    Target,
}

impl DateField {
    fn field_id(self) -> &'static str {
        match self {
            DateField::Start => JIRA_TARGET_START_FIELD,
            DateField::Target => JIRA_TARGET_END_FIELD,
        }
    }
}

// A transition offered by `GET /issue/{key}/transitions`; `to.name` is the
// workflow status the transition moves the issue into.
#[derive(Debug, Deserialize)]
struct TransitionsResponse {
    #[serde(default)]
    transitions: Vec<Transition>,
}

#[derive(Debug, Deserialize)]
struct Transition {
    id: String,
    to: TransitionTarget,
}

#[derive(Debug, Deserialize)]
struct TransitionTarget {
    name: String,
}

/// Run a `curl` invocation against the Jira REST API with basic auth. `acli`
/// only reads and cannot set custom fields, so writes go through REST here.
/// `--fail-with-body` makes HTTP >= 400 a non-zero exit (surfaced as an error by
/// the runner) while still returning the response body for context.
fn jira_curl(
    runner: &dyn CommandRunner,
    config: &JiraConfig,
    method: &str,
    url: &str,
    body: Option<&str>,
) -> AdapterResult<String> {
    let userpass = format!("{}:{}", config.email, config.token);
    let mut args: Vec<String> = vec![
        "-sS".into(),
        "--fail-with-body".into(),
        "-X".into(),
        method.into(),
        "-u".into(),
        userpass,
        "-H".into(),
        "Content-Type: application/json".into(),
        "-H".into(),
        "Accept: application/json".into(),
    ];
    if let Some(body) = body {
        args.push("--data".into());
        args.push(body.into());
    }
    args.push(url.into());

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    runner
        .run("curl", &arg_refs)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })
}

fn issue_url(config: &JiraConfig, key: &str) -> String {
    format!(
        "{}/rest/api/3/issue/{}",
        config.base_url.trim_end_matches('/'),
        key
    )
}

/// Set (or clear when `date` is None/empty) a planning-date custom field on a
/// Jira issue via `PUT /issue/{key}` with `{"fields":{"customfield_...": ...}}`.
pub fn set_target_date(
    runner: &dyn CommandRunner,
    config: &JiraConfig,
    key: &str,
    field: DateField,
    date: Option<&str>,
) -> AdapterResult<()> {
    let value = match date {
        Some(value) if !value.is_empty() => serde_json::Value::String(value.to_string()),
        _ => serde_json::Value::Null,
    };
    let mut fields = serde_json::Map::new();
    fields.insert(field.field_id().to_string(), value);
    let body = serde_json::json!({ "fields": fields }).to_string();

    jira_curl(runner, config, "PUT", &issue_url(config, key), Some(&body))?;
    Ok(())
}

/// Move a Jira issue to `status_name` by finding the matching workflow
/// transition (`GET /transitions`) and posting it (`POST /transitions`). Jira
/// status is workflow-driven, so only transitions valid from the current status
/// succeed; an unknown/unreachable target is an error.
pub fn set_status(
    runner: &dyn CommandRunner,
    config: &JiraConfig,
    key: &str,
    status_name: &str,
) -> AdapterResult<()> {
    let url = format!("{}/transitions", issue_url(config, key));
    let raw = jira_curl(runner, config, "GET", &url, None)?;
    let parsed: TransitionsResponse = serde_json::from_str(&raw)?;
    let transition = parsed
        .transitions
        .iter()
        .find(|transition| transition.to.name.eq_ignore_ascii_case(status_name))
        .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
            format!("no transition to status '{status_name}' available for {key}").into()
        })?;
    let body = serde_json::json!({ "transition": { "id": transition.id } }).to_string();
    jira_curl(runner, config, "POST", &url, Some(&body))?;
    Ok(())
}

/// Best-effort: the workflow statuses reachable from the issue's current status
/// (transition targets). Returns empty on any command/parse failure so detail
/// enrichment stays best-effort. Used to populate the status dropdown.
pub fn fetch_status_options(
    runner: &dyn CommandRunner,
    config: &JiraConfig,
    key: &str,
) -> Vec<String> {
    let url = format!("{}/transitions", issue_url(config, key));
    let Ok(raw) = jira_curl(runner, config, "GET", &url, None) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<TransitionsResponse>(&raw) else {
        return Vec::new();
    };
    parsed
        .transitions
        .into_iter()
        .map(|transition| transition.to.name)
        .collect()
}

/// Best-effort: users assignable to `key` (accountId + displayName). Empty on failure.
pub fn fetch_assignable_users(
    runner: &dyn CommandRunner,
    config: &JiraConfig,
    key: &str,
) -> Vec<crate::domain::AssigneeOption> {
    let url = format!(
        "{}/rest/api/3/user/assignable/search?issueKey={}",
        config.base_url.trim_end_matches('/'),
        key
    );
    let Ok(raw) = jira_curl(runner, config, "GET", &url, None) else {
        return Vec::new();
    };
    #[derive(Deserialize)]
    struct User {
        #[serde(rename = "accountId")]
        account_id: String,
        #[serde(rename = "displayName")]
        display_name: String,
    }
    serde_json::from_str::<Vec<User>>(&raw)
        .map(|users| {
            users
                .into_iter()
                .map(|u| crate::domain::AssigneeOption { id: u.account_id, name: u.display_name })
                .collect()
        })
        .unwrap_or_default()
}

/// Set (or clear when None) a Jira issue's single assignee via REST PUT.
pub fn set_assignee(
    runner: &dyn CommandRunner,
    config: &JiraConfig,
    key: &str,
    account_id: Option<&str>,
) -> AdapterResult<()> {
    let value = match account_id {
        Some(id) => serde_json::json!({ "accountId": id }),
        None => serde_json::Value::Null,
    };
    let body = serde_json::json!({ "fields": { "assignee": value } }).to_string();
    jira_curl(runner, config, "PUT", &issue_url(config, key), Some(&body))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Mutex};

    use crate::clients::command_runner::{CommandResult, CommandRunner, CommandRunnerError};

    use super::{load_fixture_work_items, load_work_items_with_runner};

    const TEST_BASE: &str = "https://quera.atlassian.net";

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
            .join("jira")
            .join("issues.json")
    }

    fn detail_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("jira")
            .join("issue-detail.json")
    }

    #[test]
    fn jira_fixture_detail_normalizes_body_and_comments() {
        let detail = super::load_fixture_issue_detail(&detail_fixture_path(), TEST_BASE)
            .expect("detail fixture should load");

        assert_eq!(detail.item.id, "jira:ABC-42");
        assert_eq!(detail.item.status, "In Progress");
        assert_eq!(detail.item.start_date, "2026-07-01");
        assert_eq!(detail.item.target_date, "2026-07-20");
        assert_eq!(
            detail.body.as_deref(),
            Some("We need a modal that shows full ticket detail.")
        );
        assert_eq!(detail.comments.len(), 1);
        assert_eq!(detail.comments[0].author.as_deref(), Some("Kai Hsin Wu"));
        assert_eq!(detail.comments[0].body, "Started on this.");
    }

    #[test]
    fn jira_detail_runner_invokes_expected_cli_arguments() {
        let payload = std::fs::read_to_string(detail_fixture_path()).expect("fixture should read");
        let runner = MockCommandRunner::success(&payload);

        let detail =
            super::fetch_issue_detail(&runner, "ABC-42", TEST_BASE).expect("detail should load");

        let calls = runner
            .calls
            .lock()
            .expect("calls mutex should not be poisoned");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "acli");
        assert_eq!(
            calls[0].1,
            vec![
                "jira",
                "workitem",
                "view",
                "ABC-42",
                "--json",
                "--fields",
                "summary,description,comment,status,assignee,priority,reporter,labels,created,updated,customfield_10022,customfield_10023",
            ]
        );
        assert_eq!(detail.item.external_id, "ABC-42");
    }

    #[test]
    fn extract_text_passes_plain_string_through() {
        let value = serde_json::json!("Hello world");
        assert_eq!(super::extract_text(&value), "Hello world");
    }

    #[test]
    fn extract_text_flattens_adf_without_structural_noise() {
        let value = serde_json::json!({
            "type": "doc",
            "version": 1,
            "content": [
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "Hello world" }]
                }
            ]
        });
        assert_eq!(super::extract_text(&value), "Hello world");
    }

    #[test]
    fn extract_text_null_is_empty() {
        assert_eq!(super::extract_text(&serde_json::Value::Null), "");
    }

    #[test]
    fn jira_fixture_normalizes_into_work_items() {
        let items = load_fixture_work_items(&fixture_path(), TEST_BASE).expect("fixture should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "jira:ABC-42");
        assert_eq!(items[0].external_id, "ABC-42");
        assert_eq!(items[0].source.to_string(), "jira");
        assert_eq!(items[0].container, "ABC");
        assert_eq!(items[0].url, "https://quera.atlassian.net/browse/ABC-42");
        assert_eq!(items[0].status, "In Progress");
        assert_eq!(items[0].assignees, vec!["Kai Hsin Wu".to_string()]);
    }

    #[test]
    fn jira_runner_invokes_expected_cli_arguments() {
        let payload = std::fs::read_to_string(fixture_path()).expect("fixture should read");
        let runner = MockCommandRunner::success(&payload);

        let items = load_work_items_with_runner(&runner, "order by updated desc", TEST_BASE)
            .expect("runner payload should load");

        let calls = runner
            .calls
            .lock()
            .expect("calls mutex should not be poisoned");
        // One search call, then one per-issue `view` enrichment call (1 fixture item).
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "acli");
        assert_eq!(
            calls[0].1,
            vec![
                "jira",
                "workitem",
                "search",
                "--jql",
                "order by updated desc",
                "--paginate",
                "--json",
                "--fields",
                "key,summary,status,assignee,priority,reporter,labels",
            ]
        );
        assert_eq!(
            calls[1].1,
            vec![
                "jira",
                "workitem",
                "view",
                "ABC-42",
                "--json",
                "--fields",
                "key,customfield_10022,customfield_10023",
            ]
        );
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn jira_enrichment_populates_planning_dates() {
        // Returns the search array for the `search` call and a single view object
        // (with dates) for the `view` call, keyed off the args.
        struct RoutingRunner {
            search: String,
            view: String,
        }

        impl CommandRunner for RoutingRunner {
            fn run(&self, _program: &str, args: &[&str]) -> CommandResult<String> {
                if args.contains(&"view") {
                    Ok(self.view.clone())
                } else {
                    Ok(self.search.clone())
                }
            }
        }

        let runner = RoutingRunner {
            search: std::fs::read_to_string(fixture_path()).expect("fixture should read"),
            view: r#"{ "key": "ABC-42", "fields": { "customfield_10022": "2026-07-01", "customfield_10023": "2026-07-20" } }"#
                .to_string(),
        };

        let items = load_work_items_with_runner(&runner, "order by updated desc", TEST_BASE)
            .expect("runner payload should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].start_date, "2026-07-01");
        assert_eq!(items[0].target_date, "2026-07-20");
    }

    #[test]
    fn jira_runner_propagates_cli_failures() {
        let runner = MockCommandRunner::failure("jira unavailable");

        let error = load_work_items_with_runner(&runner, "order by updated desc", TEST_BASE)
            .expect_err("runner should fail");

        assert!(error.to_string().contains("jira unavailable"));
    }

    #[test]
    fn jira_browse_url_uses_provided_base() {
        let items = load_fixture_work_items(&fixture_path(), "https://acme.atlassian.net/")
            .expect("fixture should load");
        assert_eq!(items[0].url, "https://acme.atlassian.net/browse/ABC-42");
    }

    fn test_jira_config() -> crate::config::JiraConfig {
        crate::config::JiraConfig {
            email: "u@example.com".to_string(),
            token: "tok".to_string(),
            base_url: "https://example.atlassian.net".to_string(),
        }
    }

    #[test]
    fn set_target_date_puts_custom_field_via_curl() {
        let runner = MockCommandRunner::success("");
        super::set_target_date(
            &runner,
            &test_jira_config(),
            "SSW-1",
            super::DateField::Start,
            Some("2026-07-10"),
        )
        .expect("date write should succeed");

        let calls = runner.calls.lock().expect("calls mutex");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "curl");
        let args = &calls[0].1;
        assert!(args.contains(&"PUT".to_string()));
        assert!(args.contains(&"u@example.com:tok".to_string()));
        assert!(args
            .iter()
            .any(|arg| arg == "https://example.atlassian.net/rest/api/3/issue/SSW-1"));
        // Body sets the Target-start custom field to the ISO date.
        assert!(args
            .iter()
            .any(|arg| arg.contains("customfield_10022") && arg.contains("2026-07-10")));
    }

    #[test]
    fn set_target_date_clears_with_null_body() {
        let runner = MockCommandRunner::success("");
        super::set_target_date(
            &runner,
            &test_jira_config(),
            "SSW-1",
            super::DateField::Target,
            None,
        )
        .expect("clear should succeed");

        let calls = runner.calls.lock().expect("calls mutex");
        let args = &calls[0].1;
        assert!(args
            .iter()
            .any(|arg| arg.contains("customfield_10023") && arg.contains("null")));
    }

    struct TransitionRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }
    impl CommandRunner for TransitionRunner {
        fn run(&self, program: &str, args: &[&str]) -> CommandResult<String> {
            self.calls.lock().unwrap().push((
                program.to_string(),
                args.iter().map(|a| a.to_string()).collect(),
            ));
            // GET (no --data) lists transitions; POST (--data) performs one.
            if args.contains(&"--data") {
                Ok("{}".to_string())
            } else {
                Ok(r#"{"transitions":[
                    {"id":"11","to":{"name":"Backlog"}},
                    {"id":"41","to":{"name":"Done"}}
                ]}"#
                .to_string())
            }
        }
    }

    #[test]
    fn set_status_finds_transition_then_posts_it() {
        let runner = TransitionRunner {
            calls: Mutex::new(Vec::new()),
        };
        super::set_status(&runner, &test_jira_config(), "SSW-1", "Done").expect("status ok");

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "one GET to list, one POST to apply");
        // First call lists transitions (GET, no body).
        assert!(!calls[0].1.contains(&"--data".to_string()));
        assert!(calls[0]
            .1
            .iter()
            .any(|a| a.ends_with("/rest/api/3/issue/SSW-1/transitions")));
        // Second posts the matched transition id (41 -> Done).
        assert!(calls[1].1.contains(&"POST".to_string()));
        assert!(calls[1].1.iter().any(|a| a.contains("\"id\":\"41\"")));
    }

    #[test]
    fn set_status_errors_on_unreachable_status() {
        let runner = TransitionRunner {
            calls: Mutex::new(Vec::new()),
        };
        let error = super::set_status(&runner, &test_jira_config(), "SSW-1", "Nonexistent")
            .expect_err("unknown status should error");
        assert!(error.to_string().to_lowercase().contains("transition"));
    }

    #[test]
    fn fetch_status_options_returns_transition_targets() {
        let runner = TransitionRunner {
            calls: Mutex::new(Vec::new()),
        };
        let options = super::fetch_status_options(&runner, &test_jira_config(), "SSW-1");
        assert_eq!(options, vec!["Backlog".to_string(), "Done".to_string()]);
    }

    #[test]
    fn fetch_status_options_empty_on_command_failure() {
        let runner = MockCommandRunner::failure("boom");
        let options = super::fetch_status_options(&runner, &test_jira_config(), "SSW-1");
        assert!(options.is_empty());
    }

    #[test]
    fn fetch_assignable_users_parses_options() {
        let runner = MockCommandRunner::success(r#"[{"accountId":"a1","displayName":"Alice"}]"#);
        let options = super::fetch_assignable_users(&runner, &test_jira_config(), "SSW-1");
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].id, "a1");
        assert_eq!(options[0].name, "Alice");
    }

    #[test]
    fn set_assignee_puts_accountid() {
        let runner = MockCommandRunner::success("");
        super::set_assignee(&runner, &test_jira_config(), "SSW-1", Some("a1"))
            .expect("assignee write should succeed");

        let calls = runner.calls.lock().expect("calls mutex");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "curl");
        let args = &calls[0].1;
        assert!(args.contains(&"PUT".to_string()));
        assert!(args
            .iter()
            .any(|arg| arg == "https://example.atlassian.net/rest/api/3/issue/SSW-1"));
        assert!(args
            .iter()
            .any(|arg| arg.contains("accountId") && arg.contains("a1")));
    }

    #[test]
    fn set_assignee_clears_with_null() {
        let runner = MockCommandRunner::success("");
        super::set_assignee(&runner, &test_jira_config(), "SSW-1", None)
            .expect("clear should succeed");

        let calls = runner.calls.lock().expect("calls mutex");
        let args = &calls[0].1;
        assert!(args
            .iter()
            .any(|arg| arg.contains("\"assignee\":null")));
    }
}
