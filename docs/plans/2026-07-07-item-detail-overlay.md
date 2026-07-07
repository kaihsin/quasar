# Item Detail Overlay Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a pop-up overlay that lazily fetches and displays a GitHub issue's or Jira ticket's body, comments, and metadata sidebar when a user clicks a work-item card.

**Architecture:** A new backend endpoint `GET /api/work-item-detail?id=<id>` parses the work-item id, dispatches to the GitHub or Jira adapter's new single-item fetch (`gh issue view` / `acli jira workitem view`), and returns a `WorkItemDetail` (the normalized `WorkItem` plus `body` and `comments`). No caching — every open fetches fresh. On the frontend, a new `<ItemDetailModal>` mounts when a card is clicked, fetches detail via `AbortSignal`, and renders a two-column overlay (Markdown body + comments on the left, metadata sidebar on the right).

**Tech Stack:** Rust (axum, serde, `gh`/`acli` via `CommandRunner`), React 17 + TypeScript, `react-markdown@^8` for Markdown rendering, Jest + Testing Library.

**Conventions to follow (already in the codebase):**
- Adapters return `AdapterResult<T>` and route all process calls through `CommandRunner::run`, so tests use a mock/routing runner.
- Backend tests live in `#[cfg(test)] mod tests` at the bottom of each file.
- Work-item ids: GitHub = `github:{owner}/{repo}#{number}`, Jira = `jira:{KEY}`.
- The detail endpoint uses a **query param** (not a path segment) because GitHub ids contain `/` and `#`.

---

## Task 1: Backend domain types — `WorkItemDetail` and `Comment`

**Files:**
- Modify: `crates/quasar/src/domain.rs` (add after `WorkItem`, before `SourceWarning` ~line 47)

**Step 1: Write the failing test**

Add to the `tests` module in `crates/quasar/src/domain.rs`:

```rust
#[test]
fn work_item_detail_serializes_with_body_and_comments() {
    use super::{Comment, WorkItemDetail};

    let item = WorkItem {
        source: WorkSource::GitHub,
        id: "github:openai/quasar#123".to_string(),
        external_id: "123".to_string(),
        repo: Some("openai/quasar".to_string()),
        title: "Investigate sync gap".to_string(),
        url: "https://example.com/issues/123".to_string(),
        status: "open".to_string(),
        assignee: None,
        labels: vec![],
        priority: None,
        created_at: "2026-07-06T10:00:00Z".to_string(),
        updated_at: "2026-07-06T11:00:00Z".to_string(),
        start_date: String::new(),
        target_date: String::new(),
        author: Some("octocat".to_string()),
        container: "openai/quasar".to_string(),
        source_metadata: None,
    };
    let detail = WorkItemDetail {
        item,
        body: Some("## Details\nsome text".to_string()),
        comments: vec![Comment {
            author: Some("octocat".to_string()),
            created_at: "2026-07-06T12:00:00Z".to_string(),
            body: "first comment".to_string(),
        }],
    };

    let serialized = serde_json::to_value(detail).expect("detail should serialize");
    assert_eq!(serialized["item"]["id"], "github:openai/quasar#123");
    assert_eq!(serialized["body"], "## Details\nsome text");
    assert_eq!(serialized["comments"][0]["author"], "octocat");
    assert_eq!(serialized["comments"][0]["body"], "first comment");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p quasar domain::tests::work_item_detail -- --nocapture`
Expected: FAIL — `cannot find type WorkItemDetail`.

**Step 3: Write minimal implementation**

Add to `crates/quasar/src/domain.rs` after the `WorkItem` struct:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub author: Option<String>,
    pub created_at: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkItemDetail {
    pub item: WorkItem,
    pub body: Option<String>,
    pub comments: Vec<Comment>,
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p quasar domain::tests::work_item_detail -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/quasar/src/domain.rs
git commit -m "feat: add WorkItemDetail and Comment domain types"
```

---

## Task 2: GitHub adapter — `fetch_issue_detail` + fixture loader

**Files:**
- Modify: `crates/quasar/src/adapters/github.rs`
- Create: `crates/quasar/tests/fixtures/github/issue-detail.json`

**Step 1: Create the detail fixture**

Create `crates/quasar/tests/fixtures/github/issue-detail.json` (shape of `gh issue view --json`, which returns a single object):

```json
{
  "number": 123,
  "title": "Investigate sync gap",
  "url": "https://github.com/openai/quasar/issues/123",
  "state": "OPEN",
  "body": "## Summary\nThe sync job drops events under load.\n\n- [ ] Reproduce\n- [ ] Fix",
  "assignees": [{ "login": "kai" }],
  "labels": [{ "name": "bug" }],
  "createdAt": "2026-07-06T10:00:00Z",
  "updatedAt": "2026-07-06T11:00:00Z",
  "author": { "login": "octocat" },
  "comments": [
    { "author": { "login": "kai" }, "createdAt": "2026-07-06T12:00:00Z", "body": "I can repro on staging." },
    { "author": { "login": "octocat" }, "createdAt": "2026-07-06T13:00:00Z", "body": "Nice, thanks." }
  ]
}
```

**Step 2: Write the failing tests**

Add to the `tests` module in `crates/quasar/src/adapters/github.rs`:

```rust
use super::{fetch_issue_detail, load_fixture_issue_detail};

fn detail_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("github")
        .join("issue-detail.json")
}

#[test]
fn github_fixture_detail_normalizes_body_and_comments() {
    let detail = load_fixture_issue_detail(&detail_fixture_path(), "openai/quasar")
        .expect("detail fixture should load");

    assert_eq!(detail.item.id, "github:openai/quasar#123");
    assert_eq!(detail.item.status, "open");
    assert!(detail.body.as_deref().unwrap().contains("sync job drops events"));
    assert_eq!(detail.comments.len(), 2);
    assert_eq!(detail.comments[0].author.as_deref(), Some("kai"));
    assert_eq!(detail.comments[0].body, "I can repro on staging.");
}

#[test]
fn github_detail_runner_invokes_expected_cli_arguments() {
    let payload = std::fs::read_to_string(detail_fixture_path()).expect("fixture should read");
    let runner = MockCommandRunner::success(&payload);

    let detail = fetch_issue_detail(&runner, "openai/quasar", "123").expect("detail should load");

    let calls = runner.calls.lock().expect("calls mutex should not be poisoned");
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
    let error =
        fetch_issue_detail(&runner, "openai/quasar", "123").expect_err("runner should fail");
    assert!(error.to_string().contains("gh not found"));
}
```

**Step 3: Run tests to verify they fail**

Run: `cargo test -p quasar adapters::github::tests::github_fixture_detail -- --nocapture`
Expected: FAIL — `cannot find function fetch_issue_detail` / `load_fixture_issue_detail`.

**Step 4: Write minimal implementation**

In `crates/quasar/src/adapters/github.rs`, add imports/types and functions. First extend imports at the top:

```rust
use crate::domain::{Comment, WorkItem, WorkItemDetail, WorkSource};
```

Add the detail deserialization structs near `GitHubIssue` (reuse `GitHubIssue` via `flatten` so normalization stays DRY):

```rust
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
```

Add the loader/fetch functions (place after `load_work_items_with_runner`):

```rust
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
    })
}
```

Note: `normalize_issue` already takes `GitHubIssue` and `Option<&str>` — reusing it keeps id/status/label normalization identical to the list path.

**Step 5: Run tests to verify they pass**

Run: `cargo test -p quasar adapters::github -- --nocapture`
Expected: PASS (all github adapter tests).

**Step 6: Commit**

```bash
git add crates/quasar/src/adapters/github.rs crates/quasar/tests/fixtures/github/issue-detail.json
git commit -m "feat: add GitHub single-issue detail fetch with body and comments"
```

---

## Task 3: Jira adapter — `fetch_issue_detail` + fixture loader

**IMPORTANT — verify the real shape first.** Jira's `description`/`comment` fields can be ADF (nested JSON), not plain strings, and `acli`'s exact JSON for `comment` is not yet confirmed in this repo. Before finalizing, run once against a real ticket if credentials are available:

Run: `acli jira workitem view <SOME-KEY> --json --fields summary,description,comment,status,assignee,priority,reporter,labels,created,updated`
Then adjust the `description`/`comment` deserialization below to match. The plan uses a defensive `serde_json::Value` for `description` and comment bodies and extracts text best-effort, which tolerates either a plain string or ADF.

**Files:**
- Modify: `crates/quasar/src/adapters/jira.rs`
- Create: `crates/quasar/tests/fixtures/jira/issue-detail.json`

**Step 1: Create the detail fixture**

Create `crates/quasar/tests/fixtures/jira/issue-detail.json` (plain-string body/comments — the common `acli` case):

```json
{
  "key": "ABC-42",
  "fields": {
    "summary": "Design overlay",
    "description": "We need a modal that shows full ticket detail.",
    "status": { "name": "In Progress" },
    "assignee": { "displayName": "Kai Hsin Wu" },
    "priority": { "name": "High" },
    "reporter": { "displayName": "Kai Hsin Wu" },
    "labels": ["frontend"],
    "created": "2026-07-01T09:00:00Z",
    "updated": "2026-07-06T09:00:00Z",
    "customfield_10022": "2026-07-01",
    "customfield_10023": "2026-07-20",
    "comment": {
      "comments": [
        { "author": { "displayName": "Kai Hsin Wu" }, "created": "2026-07-02T09:00:00Z", "body": "Started on this." }
      ]
    }
  }
}
```

**Step 2: Write the failing tests**

Add to the `tests` module in `crates/quasar/src/adapters/jira.rs`:

```rust
use super::{fetch_issue_detail, load_fixture_issue_detail};

fn detail_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("jira")
        .join("issue-detail.json")
}

#[test]
fn jira_fixture_detail_normalizes_body_and_comments() {
    let detail =
        load_fixture_issue_detail(&detail_fixture_path()).expect("detail fixture should load");

    assert_eq!(detail.item.id, "jira:ABC-42");
    assert_eq!(detail.item.status, "In Progress");
    assert_eq!(detail.item.start_date, "2026-07-01");
    assert_eq!(detail.item.target_date, "2026-07-20");
    assert_eq!(detail.body.as_deref(), Some("We need a modal that shows full ticket detail."));
    assert_eq!(detail.comments.len(), 1);
    assert_eq!(detail.comments[0].author.as_deref(), Some("Kai Hsin Wu"));
    assert_eq!(detail.comments[0].body, "Started on this.");
}

#[test]
fn jira_detail_runner_invokes_expected_cli_arguments() {
    let payload = std::fs::read_to_string(detail_fixture_path()).expect("fixture should read");
    let runner = MockCommandRunner::success(&payload);

    let detail = fetch_issue_detail(&runner, "ABC-42").expect("detail should load");

    let calls = runner.calls.lock().expect("calls mutex should not be poisoned");
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
```

**Step 3: Run tests to verify they fail**

Run: `cargo test -p quasar adapters::jira::tests::jira_fixture_detail -- --nocapture`
Expected: FAIL — unresolved `fetch_issue_detail` / `load_fixture_issue_detail`.

**Step 4: Write minimal implementation**

Extend the import in `crates/quasar/src/adapters/jira.rs`:

```rust
use crate::domain::{Comment, WorkItem, WorkItemDetail, WorkSource};
```

Add detail deserialization structs and a text extractor near `JiraViewFields`:

```rust
// Full per-issue detail view. `description` and comment bodies may be a plain
// string or ADF (nested JSON), so deserialize as Value and extract text.
#[derive(Debug, Deserialize)]
struct JiraDetailIssue {
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
    author: Option<JiraPerson>,
    #[serde(default)]
    created: String,
    #[serde(default)]
    body: serde_json::Value,
}

// Best-effort text: plain string as-is, ADF flattened to its `text` leaves,
// null/absent -> empty string.
fn extract_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(text)) = map.get("text") {
                return text.clone();
            }
            map.values().map(extract_text).collect::<Vec<_>>().join("")
        }
        serde_json::Value::Array(items) => {
            items.iter().map(extract_text).collect::<Vec<_>>().join("")
        }
        other => other.to_string(),
    }
}
```

Add loader/fetch functions after `fetch_issue_dates`:

```rust
pub fn load_fixture_issue_detail(path: &Path) -> AdapterResult<WorkItemDetail> {
    let raw = fs::read_to_string(path)?;
    normalize_issue_detail(&raw)
}

pub fn fetch_issue_detail(runner: &dyn CommandRunner, key: &str) -> AdapterResult<WorkItemDetail> {
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

    normalize_issue_detail(&raw)
}
```

The fixture/CLI detail path needs the issue key, but `acli view` does not echo it back, so pass it in. Adjust: have `normalize_issue_detail` take the raw JSON and read `key` from the top level. Update `JiraDetailIssue` to include it:

```rust
#[derive(Debug, Deserialize)]
struct JiraDetailIssue {
    key: String,
    fields: JiraDetailFields,
}

fn normalize_issue_detail(raw: &str) -> AdapterResult<WorkItemDetail> {
    let issue: JiraDetailIssue = serde_json::from_str(raw)?;
    let fields = issue.fields;
    let external_id = issue.key;
    let url = format!("{JIRA_BROWSE_BASE}/{external_id}");
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

    let item = WorkItem {
        source: WorkSource::Jira,
        id: format!("jira:{external_id}"),
        external_id,
        repo: None,
        title: fields.summary,
        url,
        status: fields.status.name,
        assignee: fields.assignee.map(|person| person.display_name),
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

    Ok(WorkItemDetail { item, body, comments })
}
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p quasar adapters::jira -- --nocapture`
Expected: PASS (all jira adapter tests).

**Step 6: Commit**

```bash
git add crates/quasar/src/adapters/jira.rs crates/quasar/tests/fixtures/jira/issue-detail.json
git commit -m "feat: add Jira single-issue detail fetch with body and comments"
```

---

## Task 4: API endpoint — `GET /api/work-item-detail?id=<id>` with id dispatch

**Files:**
- Modify: `crates/quasar/src/api.rs`
- Uses fixtures created in Tasks 2 & 3.

**Design notes:**
- Route uses a query param because GitHub ids contain `/` and `#`.
- Handler returns `Result<Json<WorkItemDetail>, (StatusCode, String)>`: `400` on unparseable id, `502` on adapter failure.
- Fixture mode ignores the id's repo/number and returns the fixture detail (mirrors how list fixtures ignore repo config), so the fixture path can be exercised in tests without live CLIs. It still parses the id to pick GitHub vs Jira.
- No caching.

**Step 1: Write the failing tests**

Add to the `tests` module in `crates/quasar/src/api.rs`. First extend the fixture helper so tests can point at the detail fixtures — add these helpers inside the test module:

```rust
fn github_detail_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests").join("fixtures").join("github").join("issue-detail.json")
}
fn jira_detail_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests").join("fixtures").join("jira").join("issue-detail.json")
}
```

Then the endpoint tests:

```rust
#[tokio::test]
async fn detail_endpoint_returns_github_detail_from_fixture() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/work-item-detail?id=github:openai/quasar%23123")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("response should be produced");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.expect("body").to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["item"]["source"], "github");
    assert!(payload["comments"].as_array().expect("comments array").len() >= 1);
}

#[tokio::test]
async fn detail_endpoint_rejects_unparseable_id() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/work-item-detail?id=nonsense")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("response should be produced");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p quasar api::tests::detail_endpoint -- --nocapture`
Expected: FAIL — 404 (route missing) / compile error.

**Step 3: Write minimal implementation**

In `crates/quasar/src/api.rs`:

Extend imports:

```rust
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::domain::{SourceWarning, SummaryResponse, WorkItemDetail, WorkItemsResponse, WorkSource};
```

Register the route in `router()`:

```rust
        .route("/api/work-item-detail", get(work_item_detail))
```

Add the query struct and handler:

```rust
#[derive(Deserialize)]
struct DetailQuery {
    id: String,
}

async fn work_item_detail(
    State(state): State<AppState>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<WorkItemDetail>, (StatusCode, String)> {
    fetch_work_item_detail(&state, &query.id)
        .map(Json)
        .map_err(|error| (error.status, error.message))
}

struct DetailError {
    status: StatusCode,
    message: String,
}

fn fetch_work_item_detail(state: &AppState, id: &str) -> Result<WorkItemDetail, DetailError> {
    if let Some(rest) = id.strip_prefix("github:") {
        let (repo, number) = rest.rsplit_once('#').ok_or_else(|| DetailError {
            status: StatusCode::BAD_REQUEST,
            message: format!("malformed GitHub id: {id}"),
        })?;
        let result = match &state.github_source {
            GitHubSource::Fixture(_) => {
                // Fixture mode: serve the canned detail fixture regardless of repo/number.
                let path = github_detail_fixture_path();
                adapters::github::load_fixture_issue_detail(&path, repo)
            }
            GitHubSource::Cli => {
                adapters::github::fetch_issue_detail(state.runner.as_ref(), repo, number)
            }
        };
        return result.map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        });
    }

    if let Some(key) = id.strip_prefix("jira:") {
        let result = match &state.jira_source {
            JiraSource::Fixture(_) => {
                let path = jira_detail_fixture_path();
                adapters::jira::load_fixture_issue_detail(&path)
            }
            JiraSource::Cli => adapters::jira::fetch_issue_detail(state.runner.as_ref(), key),
        };
        return result.map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        });
    }

    Err(DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("unrecognized work-item id: {id}"),
    })
}
```

Because fixture mode needs a path for detail (the `GitHubSource::Fixture(path)` holds the *list* fixture path), add two small helpers that derive the detail fixture path from the manifest dir — for non-test builds these are only reached in fixture mode, which is a dev/test convenience:

```rust
fn github_detail_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests").join("fixtures").join("github").join("issue-detail.json")
}
fn jira_detail_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests").join("fixtures").join("jira").join("issue-detail.json")
}
```

> Note: reusing `CARGO_MANIFEST_DIR`-relative fixtures for runtime fixture mode is acceptable here because fixture mode is a local dev aid (`QUASAR_MODE=fixtures`). If you prefer, derive the detail path from the configured list-fixture path's parent instead. Confirm the chosen approach during review.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p quasar api::tests::detail_endpoint -- --nocapture`
Expected: PASS

Then run the full backend suite:

Run: `cargo test -p quasar -- --nocapture`
Expected: PASS (all tests).

**Step 5: Commit**

```bash
git add crates/quasar/src/api.rs
git commit -m "feat: add work-item-detail endpoint with id dispatch"
```

---

## Task 5: Frontend types + API client

**Files:**
- Modify: `apps/frontend/src/types.ts`
- Modify: `apps/frontend/src/api.ts`

**Step 1: Add types**

Append to `apps/frontend/src/types.ts`:

```ts
export interface Comment {
  author: string | null;
  created_at: string;
  body: string;
}

export interface WorkItemDetail {
  item: WorkItem;
  body: string | null;
  comments: Comment[];
}
```

**Step 2: Add the API client function**

Append to `apps/frontend/src/api.ts`:

```ts
import type { WorkItemDetail, WorkItemsResponse } from "./types";

export async function fetchWorkItemDetail(
  id: string,
  signal?: AbortSignal,
): Promise<WorkItemDetail> {
  const response = await fetch(`/api/work-item-detail?id=${encodeURIComponent(id)}`, { signal });

  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }

  return (await response.json()) as WorkItemDetail;
}
```

(Merge the `import type` line with the existing one at the top of `api.ts` rather than duplicating it.)

**Step 3: Verify it compiles**

Run: `cd apps/frontend && npx tsc --noEmit`
Expected: no errors.

**Step 4: Commit**

```bash
git add apps/frontend/src/types.ts apps/frontend/src/api.ts
git commit -m "feat: add work-item detail types and API client"
```

---

## Task 6: Add the Markdown dependency

**Files:**
- Modify: `apps/frontend/package.json`

**Step 1: Install**

Run: `cd apps/frontend && npm install react-markdown@^8`
Expected: adds `react-markdown` to `dependencies` (v8 is React 17-compatible; v9 requires React 18 — do not use v9).

**Step 2: Verify install**

Run: `cd apps/frontend && node -e "require('react-markdown'); console.log('ok')"`
Expected: prints `ok`.

**Step 3: Commit**

```bash
git add apps/frontend/package.json apps/frontend/package-lock.json
git commit -m "chore: add react-markdown for detail rendering"
```

---

## Task 7: `ItemDetailModal` component + tests

**Files:**
- Create: `apps/frontend/src/components/ItemDetailModal.tsx`
- Create: `apps/frontend/src/components/ItemDetailModal.test.tsx`

**Step 1: Write the failing test**

Create `apps/frontend/src/components/ItemDetailModal.test.tsx`:

```tsx
import { render, screen, waitFor, fireEvent } from "@testing-library/react";

import ItemDetailModal from "./ItemDetailModal";
import * as api from "../api";
import type { WorkItemDetail } from "../types";

const detail: WorkItemDetail = {
  item: {
    source: "github",
    id: "github:openai/quasar#123",
    external_id: "123",
    title: "Investigate sync gap",
    url: "https://example.com/issues/123",
    status: "open",
    assignee: "kai",
    labels: ["bug"],
    priority: null,
    created_at: "2026-07-06T10:00:00Z",
    updated_at: "2026-07-06T11:00:00Z",
    start_date: "",
    target_date: "",
    author: "octocat",
    container: "openai/quasar",
    repo: "openai/quasar",
    source_metadata: null,
  },
  body: "## Summary\nThe sync job drops events.",
  comments: [{ author: "kai", created_at: "2026-07-06T12:00:00Z", body: "I can repro." }],
};

test("fetches and renders body, comments, and sidebar; calls onClose", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);
  const onClose = jest.fn();

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={onClose} />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).toBeInTheDocument());
  expect(screen.getByText(/The sync job drops events/)).toBeInTheDocument();
  expect(screen.getByText("I can repro.")).toBeInTheDocument();
  // Sidebar shows status and author.
  expect(screen.getByText("open")).toBeInTheDocument();
  expect(screen.getByText("octocat")).toBeInTheDocument();

  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).toHaveBeenCalled();
});

test("shows an error state when the fetch fails", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockRejectedValue(new Error("boom"));

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);

  await waitFor(() => expect(screen.getByText(/boom/)).toBeInTheDocument());
});
```

**Step 2: Run test to verify it fails**

Run: `cd apps/frontend && npx jest ItemDetailModal`
Expected: FAIL — cannot find `./ItemDetailModal`.

**Step 3: Write minimal implementation**

Create `apps/frontend/src/components/ItemDetailModal.tsx`:

```tsx
import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";

import { fetchWorkItemDetail } from "../api";
import type { WorkItemDetail } from "../types";

function formatDate(value: string): string {
  return value ? value.slice(0, 10) : "—";
}

export default function ItemDetailModal({
  itemId,
  onClose,
}: {
  itemId: string;
  onClose: () => void;
}) {
  const [detail, setDetail] = useState<WorkItemDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  // Lazy fetch: only runs when the modal is mounted for a given id.
  useEffect(() => {
    const controller = new AbortController();
    setIsLoading(true);
    setError(null);
    setDetail(null);

    fetchWorkItemDetail(itemId, controller.signal)
      .then((result) => setDetail(result))
      .catch((loadError: unknown) => {
        if (controller.signal.aborted) {
          return;
        }
        setError(loadError instanceof Error ? loadError.message : "Failed to load item");
      })
      .finally(() => {
        if (!controller.signal.aborted) {
          setIsLoading(false);
        }
      });

    return () => controller.abort();
  }, [itemId]);

  // Close on Escape.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const item = detail?.item;

  return (
    <div className="modal-backdrop" onClick={onClose} role="presentation">
      <div
        aria-modal="true"
        className="modal-panel"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
      >
        <button aria-label="Close" className="modal-close" onClick={onClose} type="button">
          ×
        </button>

        {isLoading ? <p className="modal-loading">Loading…</p> : null}
        {error ? <p className="modal-error">Failed to load: {error}</p> : null}

        {item ? (
          <div className="modal-body-grid">
            <div className="modal-main">
              <div className="modal-title-row">
                <span className={`source-badge source-${item.source}`}>{item.source}</span>
                <span className="work-item-number">{item.external_id}</span>
                <h2>{item.title}</h2>
              </div>

              <div className="modal-markdown">
                {detail?.body ? (
                  <ReactMarkdown>{detail.body}</ReactMarkdown>
                ) : (
                  <p className="modal-empty">No description provided.</p>
                )}
              </div>

              <h3 className="modal-comments-heading">
                Comments ({detail?.comments.length ?? 0})
              </h3>
              <ul className="modal-comments">
                {detail?.comments.map((comment, index) => (
                  <li className="modal-comment" key={index}>
                    <div className="modal-comment-head">
                      <span className="modal-comment-author">{comment.author ?? "Unknown"}</span>
                      <span className="modal-comment-date">{formatDate(comment.created_at)}</span>
                    </div>
                    <div className="modal-markdown">
                      <ReactMarkdown>{comment.body}</ReactMarkdown>
                    </div>
                  </li>
                ))}
              </ul>
            </div>

            <aside className="modal-sidebar">
              <dl>
                <dt>Status</dt>
                <dd>{item.status}</dd>
                <dt>Assignee</dt>
                <dd>{item.assignee ?? "Unassigned"}</dd>
                <dt>Author</dt>
                <dd>{item.author ?? "—"}</dd>
                {item.priority ? (
                  <>
                    <dt>Priority</dt>
                    <dd>{item.priority}</dd>
                  </>
                ) : null}
                <dt>{item.source === "github" ? "Repo" : "Project"}</dt>
                <dd>{item.repo ?? item.container}</dd>
                <dt>Start</dt>
                <dd>{formatDate(item.start_date)}</dd>
                <dt>Target</dt>
                <dd>{formatDate(item.target_date)}</dd>
                <dt>Created</dt>
                <dd>{formatDate(item.created_at)}</dd>
                <dt>Updated</dt>
                <dd>{formatDate(item.updated_at)}</dd>
              </dl>
              {item.labels.length ? (
                <div className="modal-labels">
                  {item.labels.map((label) => (
                    <span className="label-pill" key={label}>
                      {label}
                    </span>
                  ))}
                </div>
              ) : null}
              <a className="modal-external-link" href={item.url} rel="noreferrer" target="_blank">
                Open original ↗
              </a>
            </aside>
          </div>
        ) : null}
      </div>
    </div>
  );
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/frontend && npx jest ItemDetailModal`
Expected: PASS

> If `toBeInTheDocument` is unavailable, check whether the repo already imports `@testing-library/jest-dom` in a Jest setup file; other component tests will show the pattern. If not, assert with `screen.getByText(...)` truthiness instead.

**Step 5: Commit**

```bash
git add apps/frontend/src/components/ItemDetailModal.tsx apps/frontend/src/components/ItemDetailModal.test.tsx
git commit -m "feat: add ItemDetailModal overlay component"
```

---

## Task 8: Wire the card click into `App`

**Files:**
- Modify: `apps/frontend/src/App.tsx`

**Step 1: Write the failing test**

Create `apps/frontend/src/App.detail.test.tsx` (or add to an existing App test file if one exists — check first):

```tsx
import { render, screen, waitFor, fireEvent } from "@testing-library/react";

import App from "./App";
import * as api from "./api";
import type { WorkItemDetail, WorkItemsResponse } from "./types";

const listResponse: WorkItemsResponse = {
  data: [
    {
      source: "github",
      id: "github:openai/quasar#123",
      external_id: "123",
      title: "Investigate sync gap",
      url: "https://example.com/issues/123",
      status: "open",
      assignee: "kai",
      labels: [],
      priority: null,
      created_at: "2026-07-06T10:00:00Z",
      updated_at: "2026-07-06T11:00:00Z",
      start_date: "",
      target_date: "",
      author: "octocat",
      container: "openai/quasar",
      repo: "openai/quasar",
      source_metadata: null,
    },
  ],
  warnings: [],
  fetched_at: "0",
  cache_status: "miss",
};

const detail: WorkItemDetail = {
  item: listResponse.data[0],
  body: "Body text here.",
  comments: [],
};

test("clicking a card opens the detail modal and fetches detail", async () => {
  jest.spyOn(api, "fetchWorkItems").mockResolvedValue(listResponse);
  const detailSpy = jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);

  render(<App />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).toBeInTheDocument());
  // Detail not fetched until the user opens an item.
  expect(detailSpy).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: /Investigate sync gap/i }));

  await waitFor(() =>
    expect(detailSpy).toHaveBeenCalledWith("github:openai/quasar#123", expect.anything()),
  );
  await waitFor(() => expect(screen.getByText("Body text here.")).toBeInTheDocument());
});
```

**Step 2: Run test to verify it fails**

Run: `cd apps/frontend && npx jest App.detail`
Expected: FAIL — card is not a button / no accessible name; modal never opens.

**Step 3: Write minimal implementation**

In `apps/frontend/src/App.tsx`:

Add the import:

```tsx
import ItemDetailModal from "./components/ItemDetailModal";
```

Add state in `App` (next to the other `useState` calls):

```tsx
const [selectedItemId, setSelectedItemId] = useState<string | null>(null);
```

Render the modal near the end of the `<main>` (before its closing tag), conditionally:

```tsx
      {selectedItemId ? (
        <ItemDetailModal itemId={selectedItemId} onClose={() => setSelectedItemId(null)} />
      ) : null}
```

Pass an open handler into `WorkItemCard` at the render site (line ~316):

```tsx
{columnItems.map((item) => (
  <WorkItemCard item={item} key={item.id} onOpen={() => setSelectedItemId(item.id)} />
))}
```

Update the `WorkItemCard` signature and make the title a button that opens the modal (keep the external link separate so "open original" still works). Replace the current title `<a>` with a title button and move the external link to the head:

```tsx
function WorkItemCard({ item, onOpen }: { item: WorkItem; onOpen: () => void }) {
  const location = item.source === "github" && item.repo ? item.repo : item.container;

  return (
    <article className="work-item">
      <div className="work-item-head">
        <span className="work-item-number">{item.external_id}</span>
        <span className={`source-badge source-${item.source}`}>{item.source}</span>
        <button className="work-item-title work-item-title-button" onClick={onOpen} type="button">
          {item.title}
        </button>
        <a
          aria-label="Open original in new tab"
          className="work-item-external"
          href={item.url}
          rel="noreferrer"
          target="_blank"
        >
          ↗
        </a>
        <span className="work-item-location">{location}</span>
        <Avatar name={item.assignee} />
      </div>
      {/* ...unchanged work-item-dates and work-item-sub blocks... */}
    </article>
  );
}
```

Leave the `work-item-dates` and `work-item-sub` blocks exactly as they are.

**Step 4: Run test to verify it passes**

Run: `cd apps/frontend && npx jest App.detail`
Expected: PASS

Then run the whole frontend suite to catch regressions in existing App tests:

Run: `cd apps/frontend && npm test`
Expected: PASS

**Step 5: Commit**

```bash
git add apps/frontend/src/App.tsx apps/frontend/src/App.detail.test.tsx
git commit -m "feat: open detail modal when a work-item card is clicked"
```

---

## Task 9: Modal styles

**Files:**
- Modify: `apps/frontend/src/styles.css` (confirm exact filename; it is referenced from `main.tsx`)

**Step 1: Add CSS**

Append modal styles to the stylesheet. Match the existing visual language (reuse `source-badge`, `label-pill`, `work-item-number`). Minimum needed:

```css
.modal-backdrop {
  position: fixed;
  inset: 0;
  background: rgba(15, 23, 42, 0.55);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem;
  z-index: 50;
}
.modal-panel {
  background: #fff;
  border-radius: 12px;
  max-width: 960px;
  width: 100%;
  max-height: 85vh;
  overflow: auto;
  position: relative;
  padding: 1.5rem 1.75rem;
  box-shadow: 0 20px 60px rgba(15, 23, 42, 0.35);
}
.modal-close {
  position: absolute;
  top: 0.75rem;
  right: 0.9rem;
  border: none;
  background: transparent;
  font-size: 1.5rem;
  cursor: pointer;
  line-height: 1;
}
.modal-body-grid {
  display: grid;
  grid-template-columns: 1fr 260px;
  gap: 1.5rem;
}
.modal-title-row { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
.modal-markdown { line-height: 1.55; overflow-wrap: anywhere; }
.modal-markdown pre { background: #f1f5f9; padding: 0.75rem; border-radius: 8px; overflow: auto; }
.modal-comments { list-style: none; padding: 0; margin: 0; display: grid; gap: 0.9rem; }
.modal-comment { border: 1px solid #e2e8f0; border-radius: 8px; padding: 0.75rem; }
.modal-comment-head { display: flex; justify-content: space-between; font-size: 0.85rem; color: #475569; margin-bottom: 0.4rem; }
.modal-sidebar dl { display: grid; grid-template-columns: auto 1fr; gap: 0.35rem 0.75rem; margin: 0; font-size: 0.9rem; }
.modal-sidebar dt { color: #64748b; }
.modal-sidebar dd { margin: 0; }
.modal-labels { margin-top: 0.75rem; display: flex; flex-wrap: wrap; gap: 0.35rem; }
.modal-external-link { display: inline-block; margin-top: 1rem; }
.work-item-title-button {
  background: none;
  border: none;
  padding: 0;
  font: inherit;
  color: inherit;
  text-align: left;
  cursor: pointer;
  text-decoration: underline;
}
@media (max-width: 720px) {
  .modal-body-grid { grid-template-columns: 1fr; }
}
```

**Step 2: Verify build**

Run: `cd apps/frontend && npm run build`
Expected: build succeeds.

**Step 3: Commit**

```bash
git add apps/frontend/src/styles.css
git commit -m "style: add detail overlay modal styles"
```

---

## Task 10: End-to-end manual verification + README

**Files:**
- Modify: `README.md`

**Step 1: Manual smoke test with fixtures**

Run backend in fixtures mode:

Run: `QUASAR_MODE=fixtures cargo run -p quasar`
In another terminal: `cd apps/frontend && npm run dev`
Open `http://localhost:5173`, click a work-item title. Expected: overlay opens, shows fixture body + comments on the left, metadata sidebar on the right; Escape and backdrop click close it; the ↗ link still opens the original.

Also verify the endpoint directly:

Run: `curl 'http://localhost:3000/api/work-item-detail?id=github:openai/quasar%23123'`
Expected: JSON with `item`, `body`, `comments`.

**Step 2: Update README**

Add a short subsection under "Current Behavior" (or near the GitHub Data Fetching section) in `README.md`:

```markdown
## Item Detail Overlay

Clicking a work-item card opens an overlay with the full issue/ticket body
(rendered Markdown), the comment thread, and a metadata sidebar (status,
assignee, author, labels, priority, dates, repo/project, and a link to the
original). Detail is fetched lazily only when an item is opened, via
`GET /api/work-item-detail?id=<work-item-id>`, and is not cached — each open
fetches fresh from `gh issue view` / `acli jira workitem view`.
```

**Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document the item detail overlay"
```

---

## Final verification checklist

- Run: `cargo test -p quasar -- --nocapture` → all pass
- Run: `cd apps/frontend && npm test` → all pass
- Run: `cd apps/frontend && npx tsc --noEmit` → no errors
- Run: `cd apps/frontend && npm run build` → succeeds
- Manual: overlay opens on click, lazy-fetches once, renders Markdown + comments + sidebar, closes on Escape/backdrop.

## Notes / risks for the implementer

- **Jira JSON shape (Task 3) is the biggest unknown.** `acli`'s `description`/`comment` output may be ADF; the `extract_text` helper is defensive but verify against a real ticket and adjust field renames if needed.
- **Fixture-mode detail path (Task 4)** serves canned fixtures and ignores the requested repo/number — that is intentional for local dev/tests. Live behavior uses the CLI path.
- **react-markdown must stay on v8** for React 17 compatibility.
- Do not add write actions now; the types/components are shaped to allow them later, but that is out of scope.
```
