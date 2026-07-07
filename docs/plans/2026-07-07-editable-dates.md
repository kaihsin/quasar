# Editable Start/Target Dates Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let users edit a GitHub work item's Start and Target dates from the detail overlay (Jira stays read-only), writing via GitHub Projects v2 `gh api graphql` mutations.

**Architecture:** A new `PATCH /api/work-item-dates` endpoint parses the work-item id and, for GitHub, runs a `gh api graphql` sequence: resolve the project + date-field node ids by owner/number, resolve the issue's project-item id (adding the issue to the board if absent), then `updateProjectV2ItemFieldValue` (or `clearProjectV2ItemFieldValue` for empty), then invalidate the work-items cache. The detail modal turns GitHub date values into auto-saving `<input type="date">`s and refetches the board on success. Jira ids are rejected (acli can't write custom fields).

**Tech Stack:** Rust (axum, serde, `gh` via `CommandRunner`), React 17 + TypeScript, Jest.

**BASE BRANCH:** This feature depends on `ItemDetailModal` and the detail endpoint from branch `feat/item-detail-overlay` (not yet merged to main). Implement on a branch based on `feat/item-detail-overlay` (e.g. `feat/editable-dates`), NOT on `main`. Confirm the base before starting.

**Conventions (already in the codebase):**
- Adapters return `AdapterResult<T>` = `Result<T, Box<dyn std::error::Error + Send + Sync>>` and route every process call through `CommandRunner::run(program, args)`. Tests use a `RoutingRunner` that branches on `args` and records calls.
- `gh api graphql` passes the query as `-f query=<single-line string>` and variables as `-f name=value` (String/ID) or `-F name=value` (Int). Queries MUST be single-line (backslash-newline continuations merge adjacent tokens — see the existing `fetch_project_dates` comment in github.rs).
- axum handlers: `State` first, body extractor (`Json`) LAST. Error convention: a local error struct → `(StatusCode, String)`.
- Frontend tests: mock `react-markdown` (ESM) and assert with `.not.toBeNull()` (jest-dom matchers are NOT registered at runtime). Detail is fetched via `api.ts` helpers.

---

## Task 1: GitHub adapter — `set_project_date` (resolve → update)

Implement the happy path first: the issue is already on the board, set a non-empty date.

**Files:**
- Modify: `crates/quasar/src/adapters/github.rs`

**Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` module in `crates/quasar/src/adapters/github.rs`. The module already has `use crate::config::GitHubProject;`, a `test_project()` helper (owner "QuEraComputing", number 18, fields "Start date"/"Target date"), and `CommandResult`/`CommandRunner`/`CommandRunnerError`. Add a routing runner that returns canned JSON per graphql operation and records the ordered calls:

```rust
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
                // resolve project + fields (organization branch)
                Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_START","name":"Start date"},
                    {"id":"FLD_TARGET","name":"Target date"}
                ]}}}}}"#
                    .to_string())
            } else if query.contains("issue(number") {
                // resolve issue node id + project items
                Ok(r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                    {"id":"ITEM_1","project":{"number":18}}
                ]}}}}}"#
                    .to_string())
            } else if query.contains("updateProjectV2ItemFieldValue") {
                Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#
                    .to_string())
            } else {
                Err(CommandRunnerError::new(format!("unexpected query: {query}")))
            }
        }
    }

    let runner = SeqRunner { calls: Mutex::new(Vec::new()) };
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
    // Order: resolve project/fields, resolve item, update.
    assert_eq!(calls.len(), 3);
    assert!(calls[0].contains("projectV2(number"));
    assert!(calls[1].contains("issue(number"));
    assert!(calls[2].contains("updateProjectV2ItemFieldValue"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p quasar adapters::github::tests::set_project_date_resolves_ids_then_updates -- --nocapture`
Expected: FAIL — `cannot find function set_project_date` / `DateField`.

**Step 3: Write minimal implementation**

Add to `crates/quasar/src/adapters/github.rs`. Extend the config import to `use crate::config::GitHubProject;` (already present). Add:

```rust
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

// --- deserialization for the resolve/item/mutation calls ---

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

fn gh_graphql(runner: &dyn CommandRunner, query: &str, vars: &[(&str, &str)]) -> AdapterResult<String> {
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

/// Set (or clear, when `date` is None/empty) a Projects v2 date field for an
/// issue that belongs to the configured project. Resolves the project, field,
/// and project-item node ids, adds the issue to the board if it isn't a member,
/// then runs the update/clear mutation.
pub fn set_project_date(
    runner: &dyn CommandRunner,
    repo: &str,
    number: &str,
    project: &GitHubProject,
    field: DateField,
    date: Option<&str>,
) -> AdapterResult<()> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
            format!("malformed repo slug: {repo}").into()
        })?;

    // 1. Resolve project id + field ids by owner/number (org, then user).
    let (project_id, field_id) = resolve_project_field(runner, project, field)?;

    // 2. Resolve the issue's project item id (and issue node id for add).
    let (issue_node_id, item_id) = resolve_issue_item(runner, owner, name, number, project.number)?;

    // 3. Add to board if not a member.
    let item_id = match item_id {
        Some(id) => id,
        None => add_issue_to_project(runner, &project_id, &issue_node_id)?,
    };

    // 4. Update or clear.
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
                &[("project", &project_id), ("item", &item_id), ("field", &field_id)],
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
    let number = project.number.to_string();

    for owner_kind in ["organization", "user"] {
        let query = format!(
            "query($login:String!,$num:Int!){{{owner_kind}(login:$login){{\
             projectV2(number:$num){{id fields(first:50){{nodes{{\
             ...on ProjectV2FieldCommon{{id name}}}}}}}}}}}}"
        );
        // NOTE: num is Int! — pass with -F. gh_graphql uses -f for all; for the
        // number we still send a string, which gh coerces for Int variables via -f
        // only if quoted. To be safe pass num via a dedicated typed arg:
        let raw = gh_graphql_with_int(runner, &query, &[("login", &project.owner)], ("num", project.number))?;
        let parsed: ProjectResolveResponse = match serde_json::from_str(&raw) {
            Ok(p) => p,
            Err(_) => continue,
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
    Err(format!("project number {} not found for owner {}", project.number, project.owner).into())
}

// gh needs `-F name=value` for Int variables. Small helper mirroring gh_graphql.
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

fn resolve_issue_item(
    runner: &dyn CommandRunner,
    owner: &str,
    name: &str,
    number: &str,
    project_number: u64,
) -> AdapterResult<(String, Option<String>)> {
    let issue_number: u64 = number
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
    let issue = parsed
        .data
        .repository
        .and_then(|r| r.issue)
        .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
            format!("issue {owner}/{name}#{number} not found").into()
        })?;
    let item_id = issue
        .project_items
        .nodes
        .into_iter()
        .find(|item| item.project.as_ref().and_then(|p| p.number) == Some(project_number))
        .map(|item| item.id);
    Ok((issue.id, item_id))
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

fn add_issue_to_project(
    runner: &dyn CommandRunner,
    project_id: &str,
    content_id: &str,
) -> AdapterResult<String> {
    let query = "mutation($project:ID!,$content:ID!){\
        addProjectV2ItemById(input:{projectId:$project,contentId:$content}){item{id}}}";
    let raw = gh_graphql(runner, query, &[("project", project_id), ("content", content_id)])?;
    let parsed: AddItemResponse = serde_json::from_str(&raw)?;
    Ok(parsed.data.add.item.id)
}
```

> Implementer note: the resolve step uses `gh_graphql_with_int` (Int variable via `-F`); the update/clear/add use `gh_graphql` (all-string via `-f`). Keep both. The test's `SeqRunner` only inspects the `query=` arg, so the `-f`/`-F` distinction isn't asserted there — but it matters for real `gh`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p quasar adapters::github::tests::set_project_date_resolves_ids_then_updates -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/quasar/src/adapters/github.rs
git commit -m "feat: add GitHub Projects v2 date write (resolve + update)"
```
No Co-authored-by trailer.

---

## Task 2: `set_project_date` — auto-add and clear paths

**Files:**
- Modify: `crates/quasar/src/adapters/github.rs` (tests only; implementation already covers these — this task locks them with tests, adjusting impl only if a test fails)

**Step 1: Write the failing tests**

Add two tests to the same module:

```rust
#[test]
fn set_project_date_adds_issue_to_board_when_missing_then_updates() {
    use std::sync::Mutex;
    struct SeqRunner { calls: Mutex<Vec<String>> }
    impl CommandRunner for SeqRunner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            let q = args.iter().find_map(|a| a.strip_prefix("query=")).unwrap_or("").to_string();
            self.calls.lock().unwrap().push(q.clone());
            if q.contains("projectV2(number") {
                Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_START","name":"Start date"},{"id":"FLD_TARGET","name":"Target date"}]}}}}}"#.to_string())
            } else if q.contains("issue(number") {
                // No project items -> not on board.
                Ok(r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[]}}}}}"#.to_string())
            } else if q.contains("addProjectV2ItemById") {
                Ok(r#"{"data":{"addProjectV2ItemById":{"item":{"id":"ITEM_NEW"}}}}"#.to_string())
            } else if q.contains("updateProjectV2ItemFieldValue") {
                Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_NEW"}}}}"#.to_string())
            } else { Err(CommandRunnerError::new("unexpected")) }
        }
    }
    let runner = SeqRunner { calls: Mutex::new(Vec::new()) };
    super::set_project_date(&runner, "QuEraComputing/quasar", "123", &test_project(),
        super::DateField::Target, Some("2026-07-20")).expect("ok");
    let calls = runner.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(calls[2].contains("addProjectV2ItemById"));
    assert!(calls[3].contains("updateProjectV2ItemFieldValue"));
}

#[test]
fn set_project_date_clears_when_date_empty() {
    use std::sync::Mutex;
    struct SeqRunner { calls: Mutex<Vec<String>> }
    impl CommandRunner for SeqRunner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            let q = args.iter().find_map(|a| a.strip_prefix("query=")).unwrap_or("").to_string();
            self.calls.lock().unwrap().push(q.clone());
            if q.contains("projectV2(number") {
                Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_START","name":"Start date"},{"id":"FLD_TARGET","name":"Target date"}]}}}}}"#.to_string())
            } else if q.contains("issue(number") {
                Ok(r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                    {"id":"ITEM_1","project":{"number":18}}]}}}}}"#.to_string())
            } else if q.contains("clearProjectV2ItemFieldValue") {
                Ok(r#"{"data":{"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#.to_string())
            } else { Err(CommandRunnerError::new("unexpected")) }
        }
    }
    let runner = SeqRunner { calls: Mutex::new(Vec::new()) };
    super::set_project_date(&runner, "QuEraComputing/quasar", "123", &test_project(),
        super::DateField::Start, None).expect("ok");
    let calls = runner.calls.lock().unwrap();
    assert!(calls.last().unwrap().contains("clearProjectV2ItemFieldValue"));
}
```

**Step 2: Run tests to verify** (they should pass if Task 1 impl is correct; if they fail, fix impl)

Run: `cargo test -p quasar adapters::github::tests::set_project_date -- --nocapture`
Expected: PASS (all three set_project_date tests)

**Step 3: Commit**

```bash
git add crates/quasar/src/adapters/github.rs
git commit -m "test: cover GitHub date auto-add and clear paths"
```

---

## Task 3: API endpoint — `PATCH /api/work-item-dates`

**Files:**
- Modify: `crates/quasar/src/api.rs`

**Step 1: Write the failing tests**

Add to the `tests` module in `crates/quasar/src/api.rs`. It already has `router`, `app_state(github, jira)` (fixture mode), `fixture_path`, `Body`, `Request`, `StatusCode`, `BodyExt`, `serde_json::Value`, `ServiceExt`. Add:

```rust
#[tokio::test]
async fn update_dates_rejects_fixture_mode() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));
    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/work-item-dates")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"id":"github:openai/quasar#123","field":"start","date":"2026-07-01"}"#,
                ))
                .expect("request should build"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn update_dates_rejects_jira_id() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));
    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/work-item-dates")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"id":"jira:ABC-42","field":"start","date":"2026-07-01"}"#))
                .expect("request should build"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn update_dates_rejects_bad_field() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));
    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/work-item-dates")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"id":"github:o/r#1","field":"middle","date":"2026-07-01"}"#))
                .expect("request should build"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

Also add a CLI-mode test that a successful write invalidates the cache. Use a runner that answers the graphql sequence (reuse a small inline routing runner returning the resolve/item/update JSON from Task 1) and a CLI `AppState`:

```rust
#[test]
fn update_dates_cli_invalidates_work_items_cache() {
    use std::sync::Duration;  // if not already imported, use std::time::Duration
    // Build a CLI AppState whose runner answers both the work-items list AND the
    // date-write graphql sequence, so we can prime the cache then write.
    // (See MockCommandRunner in this module for the list; extend it to also
    // return graphql payloads when args contain "graphql".)
    // 1. Call fetch_work_items(&state) to populate the "work-items" cache (miss).
    // 2. PATCH a github date via fetch_work_item_dates(&state, ...).
    // 3. Assert the cache no longer has "work-items" (next fetch is a miss).
    // Implementer: assert via a second fetch_work_items returning cache_status "miss".
}
```

> Implementer note: if wiring a combined runner is awkward, it is acceptable to assert cache invalidation at the unit level by calling the internal `fetch_work_item_dates` helper directly with a routing runner and then checking `state.cache.get("work-items", Instant::now())` is a miss. Keep the three HTTP-level guard tests above regardless.

**Step 2: Run tests to verify they fail**

Run: `cargo test -p quasar api::tests::update_dates -- --nocapture`
Expected: FAIL — 404 (route missing) / compile error.

**Step 3: Write minimal implementation**

In `crates/quasar/src/api.rs`:

- Extend axum routing import: `use axum::{extract::{Query, State}, http::StatusCode, routing::{get, patch}, Json, Router};`
- Register route in `router()`: `.route("/api/work-item-dates", patch(update_work_item_dates))`
- Add request struct + handler + logic. Reuse the `DetailError { status, message }` pattern.

```rust
#[derive(Deserialize)]
struct UpdateDatesRequest {
    id: String,
    field: String,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Serialize)]
struct UpdateDatesResponse {
    ok: bool,
}

async fn update_work_item_dates(
    State(state): State<AppState>,
    Json(body): Json<UpdateDatesRequest>,
) -> Result<Json<UpdateDatesResponse>, (StatusCode, String)> {
    fetch_work_item_dates(&state, &body)
        .map(|_| Json(UpdateDatesResponse { ok: true }))
        .map_err(|error| (error.status, error.message))
}

fn fetch_work_item_dates(state: &AppState, body: &UpdateDatesRequest) -> Result<(), DetailError> {
    // Parse the field.
    let field = match body.field.as_str() {
        "start" => adapters::github::DateField::Start,
        "target" => adapters::github::DateField::Target,
        other => {
            return Err(DetailError {
                status: StatusCode::BAD_REQUEST,
                message: format!("unknown date field: {other}"),
            })
        }
    };

    if body.id.starts_with("jira:") {
        return Err(DetailError {
            status: StatusCode::CONFLICT,
            message: "Jira dates are read-only".to_string(),
        });
    }

    let rest = body.id.strip_prefix("github:").ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("unrecognized work-item id: {}", body.id),
    })?;
    let (repo, number) = rest.rsplit_once('#').ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("malformed GitHub id: {}", body.id),
    })?;
    if number.is_empty() {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: "missing issue number".to_string(),
        });
    }

    match &state.github_source {
        GitHubSource::Fixture(_) => {
            return Err(DetailError {
                status: StatusCode::CONFLICT,
                message: "writes unavailable in fixture mode".to_string(),
            })
        }
        GitHubSource::Cli => {}
    }

    let project = state.github_project.as_ref().ok_or_else(|| DetailError {
        status: StatusCode::CONFLICT,
        message: "no github_project configured; cannot edit dates".to_string(),
    })?;

    adapters::github::set_project_date(
        state.runner.as_ref(),
        repo,
        number,
        project,
        field,
        body.date.as_deref(),
    )
    .map_err(|error| DetailError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;

    // Invalidate the work-items cache so board cards reflect the new date.
    state.cache.invalidate("work-items");
    Ok(())
}
```

- If `ResponseCache` has no `invalidate`, add one in `crates/quasar/src/cache.rs`:

```rust
pub fn invalidate(&self, key: &str) {
    self.entries
        .lock()
        .expect("cache mutex should not be poisoned")
        .remove(key);
}
```

(Match the internal field/lock names used by the existing `get`/`insert`. Read cache.rs first and mirror them.)

**Step 4: Run tests to verify they pass**

Run: `cargo test -p quasar api::tests::update_dates -- --nocapture`
Expected: PASS
Then the full backend suite: `cargo test -p quasar -- --nocapture` — all pass.

**Step 5: Commit**

```bash
git add crates/quasar/src/api.rs crates/quasar/src/cache.rs
git commit -m "feat: add PATCH work-item-dates endpoint (github only)"
```

---

## Task 4: Frontend API client + types

**Files:**
- Modify: `apps/frontend/src/api.ts`
- Modify: `apps/frontend/src/types.ts`

**Step 1: Add the type**

Append to `apps/frontend/src/types.ts`:

```ts
export type DateFieldKind = "start" | "target";
```

**Step 2: Add the API client**

Append to `apps/frontend/src/api.ts` (merge the type import as needed):

```ts
export async function updateWorkItemDate(
  id: string,
  field: DateFieldKind,
  date: string | null,
  signal?: AbortSignal,
): Promise<void> {
  const response = await fetch("/api/work-item-dates", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id, field, date: date && date.length ? date : null }),
    signal,
  });

  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }
}
```

Add `DateFieldKind` to the `import type { ... } from "./types";` line in api.ts.

**Step 3: Verify compile**

Run: `cd apps/frontend && npx tsc --noEmit`
Expected: no errors.

**Step 4: Commit**

```bash
git add apps/frontend/src/api.ts apps/frontend/src/types.ts
git commit -m "feat: add updateWorkItemDate API client"
```

---

## Task 5: Modal — editable GitHub dates, read-only Jira

**Files:**
- Modify: `apps/frontend/src/components/ItemDetailModal.tsx`

**Step 1: Write the failing test**

Add tests to `apps/frontend/src/components/ItemDetailModal.test.tsx` (it already mocks react-markdown and uses `.not.toBeNull()`). Add a jira `detail` fixture and github editing test:

```tsx
test("editing a github date calls updateWorkItemDate and updates the value", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail); // github detail from existing test
  const updateSpy = jest.spyOn(api, "updateWorkItemDate").mockResolvedValue(undefined);
  const onItemUpdated = jest.fn();

  render(
    <ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} onItemUpdated={onItemUpdated} />,
  );

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());

  const startInput = screen.getByLabelText("Start date") as HTMLInputElement;
  fireEvent.change(startInput, { target: { value: "2026-08-01" } });
  fireEvent.blur(startInput);

  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith(
      "github:openai/quasar#123",
      "start",
      "2026-08-01",
      expect.anything(),
    ),
  );
  await waitFor(() => expect(onItemUpdated).toHaveBeenCalled());
});

test("jira dates render read-only (no date input)", async () => {
  const jiraDetail = {
    ...detail,
    item: { ...detail.item, source: "jira", id: "jira:ABC-42", repo: null, container: "ABC" },
  };
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(jiraDetail as any);

  render(<ItemDetailModal itemId="jira:ABC-42" onClose={() => {}} onItemUpdated={() => {}} />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  expect(screen.queryByLabelText("Start date")).toBeNull();
});
```

> Note: existing ItemDetailModal tests call the component WITHOUT `onItemUpdated`. Make the prop optional so those still compile/pass, OR update them to pass a no-op. Prefer optional prop with a safe default.

**Step 2: Run test to verify it fails**

Run: `cd apps/frontend && npx jest ItemDetailModal`
Expected: FAIL — no `Start date` input; `onItemUpdated` unknown.

**Step 3: Implement**

In `apps/frontend/src/components/ItemDetailModal.tsx`:

- Import the client: `import { fetchWorkItemDetail, updateWorkItemDate } from "../api";` and `import type { DateFieldKind, WorkItemDetail } from "../types";`
- Add `onItemUpdated?: () => void` to the props type (optional).
- Add a small child component (or inline) for an editable date field, used only when `item.source === "github"`. Replace the two sidebar date `<dd>`s:

```tsx
{item.source === "github" ? (
  <>
    <dt>Start</dt>
    <dd>
      <EditableDate
        itemId={item.id}
        field="start"
        initial={item.start_date}
        onSaved={onItemUpdated}
      />
    </dd>
    <dt>Target</dt>
    <dd>
      <EditableDate
        itemId={item.id}
        field="target"
        initial={item.target_date}
        onSaved={onItemUpdated}
      />
    </dd>
  </>
) : (
  <>
    <dt>Start</dt>
    <dd>
      {formatDate(item.start_date)}
      <span className="date-readonly-hint"> (read-only)</span>
    </dd>
    <dt>Target</dt>
    <dd>{formatDate(item.target_date)}</dd>
  </>
)}
```

Add the `EditableDate` component in the same file:

```tsx
type SaveState = "idle" | "saving" | "saved" | "error";

function EditableDate({
  itemId,
  field,
  initial,
  onSaved,
}: {
  itemId: string;
  field: DateFieldKind;
  initial: string;
  onSaved?: () => void;
}) {
  // <input type="date"> uses YYYY-MM-DD; backend dates are already that shape.
  const [value, setValue] = useState(initial ? initial.slice(0, 10) : "");
  const [state, setState] = useState<SaveState>("idle");

  async function save(next: string) {
    setState("saving");
    try {
      await updateWorkItemDate(itemId, field, next ? next : null);
      setState("saved");
      onSaved?.();
    } catch {
      setState("error");
    }
  }

  return (
    <span className="editable-date">
      <input
        aria-label={field === "start" ? "Start date" : "Target date"}
        className="date-input"
        onBlur={(event) => {
          if (event.target.value !== (initial ? initial.slice(0, 10) : "")) {
            void save(event.target.value);
          }
        }}
        onChange={(event) => setValue(event.target.value)}
        type="date"
        value={value}
      />
      {state === "saving" ? <span className="date-status">…</span> : null}
      {state === "saved" ? <span className="date-status date-status-ok">✓</span> : null}
      {state === "error" ? <span className="date-status date-status-err">!</span> : null}
    </span>
  );
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/frontend && npx jest ItemDetailModal`
Expected: PASS (all ItemDetailModal tests, including pre-existing ones)
Then: `cd apps/frontend && npx tsc --noEmit` — clean.

**Step 5: Commit**

```bash
git add apps/frontend/src/components/ItemDetailModal.tsx apps/frontend/src/components/ItemDetailModal.test.tsx
git commit -m "feat: editable github dates in detail modal"
```

---

## Task 6: Wire `onItemUpdated` in App

**Files:**
- Modify: `apps/frontend/src/App.tsx`

**Step 1: Write the failing test**

Add to `apps/frontend/src/App.detail.test.tsx`:

```tsx
test("saving a date from the modal refetches the work-items list", async () => {
  const listSpy = jest.spyOn(api, "fetchWorkItems").mockResolvedValue(listResponse);
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);
  jest.spyOn(api, "updateWorkItemDate").mockResolvedValue(undefined);

  render(<App />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  fireEvent.click(screen.getByRole("button", { name: /Investigate sync gap/i }));

  const startInput = await screen.findByLabelText("Start date");
  const callsBefore = listSpy.mock.calls.length;
  fireEvent.change(startInput, { target: { value: "2026-08-01" } });
  fireEvent.blur(startInput);

  await waitFor(() => expect(listSpy.mock.calls.length).toBeGreaterThan(callsBefore));
});
```

> Ensure `detail.item` is a github item (it is, from the existing file). If `fetchWorkItemDetail` mock returns the shared `detail`, the modal will show editable inputs.

**Step 2: Run test to verify it fails**

Run: `cd apps/frontend && npx jest App.detail`
Expected: FAIL — list not refetched (modal has no onItemUpdated wired).

**Step 3: Implement**

In `apps/frontend/src/App.tsx`, pass the callback to the modal:

```tsx
{selectedItemId ? (
  <ItemDetailModal
    itemId={selectedItemId}
    onClose={() => setSelectedItemId(null)}
    onItemUpdated={() => {
      void loadWorkItems();
    }}
  />
) : null}
```

**Step 4: Run test + full suite**

Run: `cd apps/frontend && npx jest App.detail` — passes.
Run: `cd apps/frontend && npm test` — all pass.
Run: `cd apps/frontend && npx tsc --noEmit` — clean.

**Step 5: Commit**

```bash
git add apps/frontend/src/App.tsx apps/frontend/src/App.detail.test.tsx
git commit -m "feat: refetch board after editing dates in modal"
```

---

## Task 7: Styles

**Files:**
- Modify: `apps/frontend/src/styles.css`

**Step 1: Add CSS** (match the existing light theme; read neighbors first)

```css
.editable-date { display: inline-flex; align-items: center; gap: 0.35rem; }
.date-input {
  font: inherit;
  padding: 0.15rem 0.35rem;
  border: 1px solid rgba(20, 33, 61, 0.2);
  border-radius: 6px;
  background: #fff;
}
.date-status { font-size: 0.85rem; }
.date-status-ok { color: #15803d; }
.date-status-err { color: #b91c1c; }
.date-readonly-hint { color: #7a8699; font-size: 0.8rem; }
```

**Step 2: Verify build + tests**

Run: `cd apps/frontend && npm run build` — succeeds.
Run: `cd apps/frontend && npm test` — all pass.

**Step 3: Commit**

```bash
git add apps/frontend/src/styles.css
git commit -m "style: editable date inputs"
```

---

## Task 8: Verification + README

**Files:**
- Modify: `README.md`

**Step 1: Backend guard smoke test (fixtures mode)**

Start fixtures-mode backend, confirm the endpoint rejects writes safely:

Run: `QUASAR_MODE=fixtures cargo run -p quasar &` (wait for boot)
Run: `curl -s -o /dev/null -w "%{http_code}" -X PATCH -H 'content-type: application/json' -d '{"id":"github:openai/quasar#123","field":"start","date":"2026-07-01"}' http://127.0.0.1:3000/api/work-item-dates`
Expected: `409` (writes unavailable in fixture mode).
Run: same with `{"id":"jira:ABC-42",...}` → `409`.
Kill the backend process afterward.

> Live GitHub write is NOT exercised here (requires a `gh` token with `project` scope, a real repo on the configured board, and would mutate a real project). Note this limitation in the report. If the user wants a live check, do it against a designated test issue with their approval.

**Step 2: Full sweep**

Run: `cargo test -p quasar` — all pass.
Run: `cd apps/frontend && npm test` — all pass.
Run: `cd apps/frontend && npx tsc --noEmit` — clean.
Run: `cd apps/frontend && npm run build` — succeeds.

**Step 3: README**

Add to the "Item Detail Overlay" section (or a new "Editing dates" subsection) in `README.md`:

```markdown
### Editing dates

GitHub work-item Start/Target dates are editable inline from the detail overlay
(they are Projects v2 board fields). Editing a date issues
`PATCH /api/work-item-dates` which resolves the project/field/item via
`gh api graphql`, adds the issue to the configured board if needed, and runs an
`updateProjectV2ItemFieldValue` (or clear) mutation, then invalidates the
work-items cache. This requires a `gh` token with `project` write scope.

Jira dates are read-only in the UI: the installed `acli` (1.3.22) cannot set
custom fields.
```

**Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document editable github dates"
```

---

## Final verification checklist

- `cargo test -p quasar` → all pass
- `cd apps/frontend && npm test` → all pass
- `cd apps/frontend && npx tsc --noEmit` → clean
- `cd apps/frontend && npm run build` → succeeds
- Fixtures-mode PATCH → 409 for both github and jira ids
- Manual (optional, needs project scope): edit a date on a real GitHub issue on the configured board and confirm it lands on the board.

## Notes / risks for the implementer

- **`gh` token scope** is the top runtime risk: Projects v2 writes need `project` scope. A missing scope surfaces as a 502 with gh's error — the modal shows the error indicator. Verify with `gh auth status` / a manual mutation before claiming live success.
- **org-vs-user**: `resolve_project_field` tries `organization` then `user`. If the configured owner is a user account, the org query returns `data.organization: null` and the code falls through to the user query.
- **`-f` vs `-F`**: Int GraphQL variables (project number, issue number) MUST use `-F`; ID/String/Date use `-f`. Keep `gh_graphql` (all `-f`) and `gh_graphql_with_int` separate.
- **Single-line GraphQL**: do not reformat the query string literals across lines with backslash continuations that would merge tokens; the existing `fetch_project_dates` has the same constraint.
- **Base branch**: build on `feat/item-detail-overlay`, not `main` (the modal and detail endpoint live there).
- Jira write support is intentionally out of scope (CLI limitation).
