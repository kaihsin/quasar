# Detail Date Defaults + Editable GitHub Status Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Pre-fill the detail modal's Start/Target date inputs from the board, and add an editable GitHub Projects v2 "Status" (single-select) control — both driven by enriching the detail response with the item's project fields.

**Architecture:** The GitHub detail path gains one repo-scoped `gh api graphql` enrichment call that fills the item's dates and the current Status + available Status options. The write endpoint `PATCH /api/work-item-dates` is generalized to `PATCH /api/work-item-field` handling `start`/`target`/`status`; the adapter gains `set_project_status` beside `set_project_date`. The modal seeds dates from the enriched detail and adds a Status `<select>`.

**Tech Stack:** Rust (axum, serde, `gh` via `CommandRunner`), React 17 + TypeScript, Jest.

**BRANCH:** Continue on `feat/editable-dates` (this extends that unmerged work).

**Conventions (already in the codebase):**
- Adapters return `AdapterResult<T>` and route process calls through `CommandRunner::run(program, args)`. Tests use a `SeqRunner`/routing runner that branches on the `-f query=...` arg and records calls.
- `gh api graphql`: query as `-f query=<single-line>`, String/ID/Date vars via `-f`, Int vars via `-F` (helpers `gh_graphql` and `gh_graphql_with_int` already exist in github.rs). Queries MUST be single-line.
- Frontend tests: mock `react-markdown` (ESM), assert with `.not.toBeNull()` / `.toBeNull()` (no jest-dom matchers). Jest has no `clearMocks`, so a test reusing a shared `api` spy across calls should `mockClear()` where counts matter.
- Existing adapter fns on this branch: `set_project_date`, `resolve_project_field`, `resolve_issue_item`, `add_issue_to_project`, `gh_graphql`, `gh_graphql_with_int`, `DateField`. The org→user resolve loop tolerates a command error on the first owner kind.

---

## Task 1: Config — `status_field` on `GitHubProject`

**Files:**
- Modify: `crates/quasar/src/config.rs`

**Step 1: Write the failing test**

Find the config tests module. There is a `default_start_date_field` / `default_target_date_field` pattern (functions returning "Start date" / "Target date") used via `#[serde(default = "...")]`. Add a test asserting the default status field:

```rust
#[test]
fn github_project_status_field_defaults_to_status() {
    let toml = r#"
owner = "QuEraComputing"
number = 18
"#;
    let project: GitHubProject = toml::from_str(toml).expect("should parse");
    assert_eq!(project.status_field, "Status");
}
```
(If the existing tests parse `GitHubProject` differently, mirror that style. Check how `start_date_field`'s default is tested, if at all, and follow it.)

**Step 2: Run test to verify it fails**

Run: `cargo test -p quasar config -- --nocapture`
Expected: FAIL — no field `status_field`.

**Step 3: Write minimal implementation**

In `crates/quasar/src/config.rs`, add to the `GitHubProject` struct (next to `start_date_field`/`target_date_field`):

```rust
    #[serde(default = "default_status_field")]
    pub status_field: String,
```

And the default fn near `default_start_date_field`:

```rust
fn default_status_field() -> String {
    "Status".to_string()
}
```

Update any place that constructs `GitHubProject` literally in non-test code (there should be none outside config) — but the github.rs test `test_project()` helper builds it literally and will now need `status_field: "Status".to_string()`. Add that field there too (github.rs test module) so the crate still compiles:

```rust
        status_field: "Status".to_string(),
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p quasar -- --nocapture`
Expected: PASS (config + all existing tests still compile/pass).

**Step 5: Commit**

```bash
git add crates/quasar/src/config.rs crates/quasar/src/adapters/github.rs
git commit -m "feat: add status_field to GitHubProject config (default Status)"
```
No Co-authored-by trailer.

---

## Task 2: Domain — `WorkItemDetail` gains `project_status` + `status_options`

**Files:**
- Modify: `crates/quasar/src/domain.rs`

**Step 1: Write the failing test**

Add to the domain tests module:

```rust
#[test]
fn work_item_detail_carries_project_status_and_options() {
    use super::{WorkItemDetail};
    let detail = WorkItemDetail {
        item: sample_github_work_item(), // if no helper exists, build a WorkItem inline like the existing detail test does
        body: None,
        comments: vec![],
        project_status: Some("In Progress".to_string()),
        status_options: vec!["Todo".to_string(), "In Progress".to_string(), "Done".to_string()],
    };
    let serialized = serde_json::to_value(detail).expect("serialize");
    assert_eq!(serialized["project_status"], "In Progress");
    assert_eq!(serialized["status_options"][1], "In Progress");
}
```
If there's no `sample_github_work_item` helper, construct the `WorkItem` inline exactly as the existing `work_item_detail_serializes_with_body_and_comments` test does, and add the two new fields.

**Step 2: Run test to verify it fails**

Run: `cargo test -p quasar domain -- --nocapture`
Expected: FAIL — missing fields `project_status` / `status_options`.

**Step 3: Write minimal implementation**

In `crates/quasar/src/domain.rs`, add to `WorkItemDetail`:

```rust
    #[serde(default)]
    pub project_status: Option<String>,
    #[serde(default)]
    pub status_options: Vec<String>,
```

Update the existing domain detail test (and any other `WorkItemDetail { ... }` literal — notably in `adapters/github.rs` `normalize_issue_detail` and `adapters/jira.rs` `normalize_issue_detail`) to set the new fields. Use `project_status: None, status_options: Vec::new()` at those construction sites (they don't have project data).

**Step 4: Run test to verify it passes**

Run: `cargo test -p quasar -- --nocapture`
Expected: PASS (all crates compile; github/jira detail builders updated).

**Step 5: Commit**

```bash
git add crates/quasar/src/domain.rs crates/quasar/src/adapters/github.rs crates/quasar/src/adapters/jira.rs
git commit -m "feat: add project_status and status_options to WorkItemDetail"
```

---

## Task 3: Adapter — enrich detail with project fields (dates + status + options)

**Files:**
- Modify: `crates/quasar/src/adapters/github.rs`

**Step 1: Write the failing test**

Add to the github tests module. This tests a new `pub fn enrich_detail_project_fields(runner, repo, number, project) -> DetailProjectFields`:

```rust
#[test]
fn enrich_detail_reads_dates_status_and_options() {
    struct Runner;
    impl CommandRunner for Runner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            let q = args.iter().find_map(|a| a.strip_prefix("query=")).unwrap_or("");
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
    let fields = super::enrich_detail_project_fields(&Runner, "QuEraComputing/quasar", "123", &test_project());
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
            Ok(r#"{"data":{"repository":{"issue":{"projectItems":{"nodes":[]}}}}}"#.to_string())
        }
    }
    let fields = super::enrich_detail_project_fields(&Runner, "QuEraComputing/quasar", "123", &test_project());
    assert_eq!(fields.start_date, "");
    assert_eq!(fields.project_status, None);
    assert!(fields.status_options.is_empty());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p quasar adapters::github::tests::enrich_detail -- --nocapture`
Expected: FAIL — no `enrich_detail_project_fields` / `DetailProjectFields`.

**Step 3: Write minimal implementation**

In `crates/quasar/src/adapters/github.rs`:

```rust
/// Result of enriching a single issue's detail with its configured-project fields.
#[derive(Debug, Default)]
pub struct DetailProjectFields {
    pub start_date: String,
    pub target_date: String,
    pub project_status: Option<String>,
    pub status_options: Vec<String>,
}

// --- deserialization for the detail enrichment query ---
#[derive(Debug, Deserialize)]
struct DetailEnrichResponse { data: DetailEnrichData }
#[derive(Debug, Deserialize)]
struct DetailEnrichData { repository: Option<DetailEnrichRepo> }
#[derive(Debug, Deserialize)]
struct DetailEnrichRepo { issue: Option<DetailEnrichIssue> }
#[derive(Debug, Deserialize)]
struct DetailEnrichIssue {
    #[serde(rename = "projectItems")]
    project_items: DetailEnrichItems,
}
#[derive(Debug, Deserialize)]
struct DetailEnrichItems { nodes: Vec<DetailEnrichItem> }
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
struct DetailEnrichOption { name: String }
#[derive(Debug, Default, Deserialize)]
struct DetailEnrichFieldValues { nodes: Vec<DetailEnrichValue> }
#[derive(Debug, Default, Deserialize)]
struct DetailEnrichValue {
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    name: Option<String>, // single-select value name
    #[serde(default)]
    field: Option<GraphQlFieldName>, // reuse existing struct { name: Option<String> }
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
    let Some((owner, name)) = repo.split_once('/') else { return out; };
    let Ok(issue_number) = number.parse::<u64>() else { return out; };

    let query = "query($owner:String!,$name:String!,$number:Int!,$statusField:String!){\
        repository(owner:$owner,name:$name){issue(number:$number){\
        projectItems(first:20){nodes{ project{ number \
        status: field(name:$statusField){...on ProjectV2SingleSelectField{options{name}}} } \
        fieldValues(first:30){nodes{ \
        ...on ProjectV2ItemFieldDateValue{date field{...on ProjectV2FieldCommon{name}}} \
        ...on ProjectV2ItemFieldSingleSelectValue{name field{...on ProjectV2FieldCommon{name}}} \
        }} }} }}}";

    // owner/name/statusField are String (-f); number is Int (-F).
    let raw = match gh_graphql_with_int(
        runner,
        query,
        &[("owner", owner), ("name", name), ("statusField", &project.status_field)],
        ("number", issue_number),
    ) {
        Ok(raw) => raw,
        Err(_) => return out,
    };
    let parsed: DetailEnrichResponse = match serde_json::from_str(&raw) {
        Ok(p) => p,
        Err(_) => return out,
    };
    let Some(issue) = parsed.data.repository.and_then(|r| r.issue) else { return out; };

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
                (Some(date), _, Some(fname)) if fname == project.start_date_field => out.start_date = date,
                (Some(date), _, Some(fname)) if fname == project.target_date_field => out.target_date = date,
                (None, Some(sel), Some(fname)) if fname == project.status_field => {
                    out.project_status = Some(sel)
                }
                _ => {}
            }
        }
    }
    out
}
```

Note: `GraphQlFieldName { name: Option<String> }` already exists (used by `fetch_project_dates`). Reuse it. If its visibility/shape differs, define a small local struct instead.

**Step 4: Run test to verify it passes**

Run: `cargo test -p quasar adapters::github -- --nocapture`
Expected: PASS

**Step 5: fmt + commit**

Run `cargo fmt -p quasar -- --check` (hand-fix only your github.rs lines; never run plain `cargo fmt`).

```bash
git add crates/quasar/src/adapters/github.rs
git commit -m "feat: enrich github detail with project dates, status, and options"
```

---

## Task 4: Wire enrichment into the detail endpoint

**Files:**
- Modify: `crates/quasar/src/api.rs`

**Step 1: Write the failing test**

Add a test that, in CLI mode with a runner answering the detail (`gh issue view`) and the enrichment (`issue(number` graphql), the detail response carries the enriched dates + status + options. Mirror the existing detail tests but with a CLI `AppState` + routing runner:

```rust
#[test]
fn work_item_detail_cli_includes_enriched_project_fields() {
    use crate::config::GitHubProject;
    struct Runner;
    impl CommandRunner for Runner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            let joined = args.join(" ");
            if args.iter().any(|a| a.strip_prefix("query=").map_or(false, |q| q.contains("issue(number"))) {
                // enrichment
                Ok(r#"{"data":{"repository":{"issue":{"projectItems":{"nodes":[
                    {"project":{"number":18,"status":{"options":[{"name":"Todo"},{"name":"Done"}]}},
                     "fieldValues":{"nodes":[
                        {"date":"2026-06-01","field":{"name":"Start date"}},
                        {"name":"Done","field":{"name":"Status"}}
                     ]}}]}}}}}"#.to_string())
            } else if joined.contains("issue view") {
                // gh issue view detail
                Ok(r#"{"number":123,"title":"t","url":"https://github.com/o/r/issues/123","state":"OPEN","assignees":[],"labels":[],"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","author":{"login":"a"}}"#.to_string())
            } else {
                Err(CommandRunnerError::new("unexpected"))
            }
        }
    }
    let state = AppState {
        github_source: GitHubSource::Cli,
        jira_source: JiraSource::Fixture(fixture_path("jira")),
        cache: Arc::new(ResponseCache::new(Duration::from_secs(30))),
        runner: Arc::new(Runner),
        github_repos: vec!["o/r".to_string()],
        jira_jql: "order by updated desc".to_string(),
        github_project: Some(GitHubProject {
            owner: "QuEraComputing".into(), number: 18,
            start_date_field: "Start date".into(), target_date_field: "Target date".into(),
            status_field: "Status".into(),
        }),
    };
    let detail = super::fetch_work_item_detail(&state, "github:o/r#123").expect("detail");
    assert_eq!(detail.item.start_date, "2026-06-01");
    assert_eq!(detail.project_status.as_deref(), Some("Done"));
    assert_eq!(detail.status_options, vec!["Todo", "Done"]);
}
```
Adjust `AppState { ... }` field list to match the actual struct (copy from a nearby test that builds a CLI AppState).

**Step 2: Run test to verify it fails**

Run: `cargo test -p quasar api::tests::work_item_detail_cli_includes_enriched -- --nocapture`
Expected: FAIL — detail not enriched (dates empty, status None).

**Step 3: Write minimal implementation**

In `crates/quasar/src/api.rs`, in `fetch_work_item_detail`, the GitHub CLI branch currently returns `adapters::github::fetch_issue_detail(...)`. Change it so that after obtaining the detail it enriches when a project is configured:

```rust
        GitHubSource::Cli => {
            let mut detail = adapters::github::fetch_issue_detail(state.runner.as_ref(), repo, number)
                .map_err(|error| DetailError { status: StatusCode::BAD_GATEWAY, message: error.to_string() })?;
            if let Some(project) = state.github_project.as_ref() {
                let fields = adapters::github::enrich_detail_project_fields(
                    state.runner.as_ref(), repo, number, project,
                );
                detail.item.start_date = fields.start_date;
                detail.item.target_date = fields.target_date;
                detail.project_status = fields.project_status;
                detail.status_options = fields.status_options;
            }
            return Ok(detail);
        }
```
(Match the surrounding return/`?` style of the existing function; the exact wrapping of errors should mirror what the function already does. If the function currently maps errors at the end rather than inline, keep it consistent.)

Fixture-mode detail is unchanged (no enrichment).

**Step 4: Run test to verify it passes**

Run: `cargo test -p quasar api -- --nocapture` — all pass. Then full suite `cargo test -p quasar`.

**Step 5: fmt + commit**

```bash
git add crates/quasar/src/api.rs
git commit -m "feat: enrich github detail response with project fields"
```

---

## Task 5: Adapter — `set_project_status`

**Files:**
- Modify: `crates/quasar/src/adapters/github.rs`

**Step 1: Write the failing test**

Add to github tests. The function resolves the project id, the single-select field id, and the option id (by name), then updates. Reuse the SeqRunner pattern:

```rust
#[test]
fn set_project_status_resolves_ids_then_updates() {
    use std::sync::Mutex;
    struct SeqRunner { calls: Mutex<Vec<String>> }
    impl CommandRunner for SeqRunner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            let q = args.iter().find_map(|a| a.strip_prefix("query=")).unwrap_or("").to_string();
            self.calls.lock().unwrap().push(q.clone());
            if q.contains("projectV2(number") {
                Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_STATUS","name":"Status","options":[
                        {"id":"OPT_TODO","name":"Todo"},{"id":"OPT_DONE","name":"Done"}]}
                ]}}}}}"#.to_string())
            } else if q.contains("issue(number") {
                Ok(r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                    {"id":"ITEM_1","project":{"number":18}}]}}}}}"#.to_string())
            } else if q.contains("updateProjectV2ItemFieldValue") {
                assert!(q.contains("singleSelectOptionId"), "status update must use singleSelectOptionId");
                Ok(r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#.to_string())
            } else { Err(CommandRunnerError::new("unexpected")) }
        }
    }
    let runner = SeqRunner { calls: Mutex::new(Vec::new()) };
    super::set_project_status(&runner, "QuEraComputing/quasar", "123", &test_project(), Some("Done")).expect("ok");
    let calls = runner.calls.lock().unwrap();
    assert!(calls[0].contains("projectV2(number"));
    assert!(calls[1].contains("issue(number"));
    assert!(calls.last().unwrap().contains("updateProjectV2ItemFieldValue"));
}

#[test]
fn set_project_status_clears_when_none() {
    struct Runner;
    impl CommandRunner for Runner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            let q = args.iter().find_map(|a| a.strip_prefix("query=")).unwrap_or("");
            if q.contains("projectV2(number") {
                Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_STATUS","name":"Status","options":[{"id":"OPT_TODO","name":"Todo"}]}]}}}}}"#.to_string())
            } else if q.contains("issue(number") {
                Ok(r#"{"data":{"repository":{"issue":{"id":"ISSUE_1","projectItems":{"nodes":[
                    {"id":"ITEM_1","project":{"number":18}}]}}}}}"#.to_string())
            } else if q.contains("clearProjectV2ItemFieldValue") {
                Ok(r#"{"data":{"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":"ITEM_1"}}}}"#.to_string())
            } else { Err(CommandRunnerError::new("unexpected")) }
        }
    }
    super::set_project_status(&Runner, "QuEraComputing/quasar", "123", &test_project(), None).expect("ok");
}

#[test]
fn set_project_status_errors_on_unknown_option() {
    struct Runner;
    impl CommandRunner for Runner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            let q = args.iter().find_map(|a| a.strip_prefix("query=")).unwrap_or("");
            if q.contains("projectV2(number") {
                Ok(r#"{"data":{"organization":{"projectV2":{"id":"PVT_1","fields":{"nodes":[
                    {"id":"FLD_STATUS","name":"Status","options":[{"id":"OPT_TODO","name":"Todo"}]}]}}}}}"#.to_string())
            } else { Ok(r#"{"data":{"repository":{"issue":{"id":"I","projectItems":{"nodes":[{"id":"ITEM_1","project":{"number":18}}]}}}}}"#.to_string()) }
        }
    }
    let err = super::set_project_status(&Runner, "QuEraComputing/quasar", "123", &test_project(), Some("Nope")).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("option"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p quasar adapters::github::tests::set_project_status -- --nocapture`
Expected: FAIL — no `set_project_status`.

**Step 3: Write minimal implementation**

Add to github.rs. First, extend the project-fields resolve so options come back. Add option structs and a single-select resolver reusing the existing org→user loop shape:

```rust
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
struct SingleSelectOption { id: String, name: String }

#[derive(Debug, Deserialize)]
struct SsResolveResponse { data: SsResolveData }
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
struct SsResolveProject { id: String, fields: SsResolveFields }
#[derive(Debug, Deserialize)]
struct SsResolveFields { nodes: Vec<SingleSelectFieldNode> }

/// Resolve (project_id, field_id, option_id) for a single-select field + option
/// name. option is None when clearing (option_id not needed).
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
        let raw = match gh_graphql_with_int(runner, &query, &[("login", &project.owner)], ("num", project.number)) {
            Ok(raw) => raw,
            Err(error) => { last_error = Some(error.to_string()); continue; }
        };
        let parsed: SsResolveResponse = match serde_json::from_str(&raw) {
            Ok(p) => p,
            Err(error) => { last_error = Some(error.to_string()); continue; }
        };
        let owner = match owner_kind { "organization" => parsed.data.organization, _ => parsed.data.user };
        if let Some(proj) = owner.and_then(|o| o.project) {
            let field = proj.fields.nodes.into_iter()
                .find(|f| f.name.as_deref() == Some(project.status_field.as_str()))
                .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("status field '{}' not found on project", project.status_field).into()
                })?;
            let field_id = field.id.ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                "status field has no id".into()
            })?;
            let option_id = match option_name {
                Some(name) => Some(
                    field.options.into_iter().find(|o| o.name == name).map(|o| o.id)
                        .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                            format!("status option '{name}' not found").into()
                        })?,
                ),
                None => None,
            };
            return Ok((proj.id, field_id, option_id));
        }
    }
    Err(format!("project number {} not found for owner {} (last error: {:?})",
        project.number, project.owner, last_error).into())
}

/// Set (or clear when option_name is None) the configured Status single-select
/// field for an issue belonging to the configured project. Adds the issue to the
/// board if absent.
pub fn set_project_status(
    runner: &dyn CommandRunner,
    repo: &str,
    number: &str,
    project: &GitHubProject,
    option_name: Option<&str>,
) -> AdapterResult<()> {
    let (owner, name) = repo.split_once('/').ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("malformed repo slug: {repo}").into()
    })?;
    let (project_id, field_id, option_id) = resolve_project_single_select(runner, project, option_name)?;
    let (issue_node_id, item_id) = resolve_issue_item(runner, owner, name, number, project.number)?;
    let item_id = match item_id { Some(id) => id, None => add_issue_to_project(runner, &project_id, &issue_node_id)? };

    match option_id {
        Some(option) => {
            let query = "mutation($project:ID!,$item:ID!,$field:ID!,$option:String!){\
                updateProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,\
                fieldId:$field,value:{singleSelectOptionId:$option}}){projectV2Item{id}}}";
            gh_graphql(runner, query, &[("project", &project_id), ("item", &item_id), ("field", &field_id), ("option", &option)])?;
        }
        None => {
            let query = "mutation($project:ID!,$item:ID!,$field:ID!){\
                clearProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,\
                fieldId:$field}){projectV2Item{id}}}";
            gh_graphql(runner, query, &[("project", &project_id), ("item", &item_id), ("field", &field_id)])?;
        }
    }
    Ok(())
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p quasar adapters::github -- --nocapture` — all pass.

**Step 5: fmt + commit**

```bash
git add crates/quasar/src/adapters/github.rs
git commit -m "feat: add set_project_status for GitHub Projects v2 single-select"
```

---

## Task 6: Generalize the write endpoint → `PATCH /api/work-item-field`

**Files:**
- Modify: `crates/quasar/src/api.rs`

**Step 1: Write the failing tests**

Update/replace the existing `update_dates_*` tests to hit the new route/body and add a status test. The route is `/api/work-item-field`; body is `{ id, field, value }`. Add:

```rust
#[tokio::test]
async fn update_field_rejects_jira_id() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));
    let response = app.oneshot(Request::builder().method("PATCH").uri("/api/work-item-field")
        .header("content-type","application/json")
        .body(Body::from(r#"{"id":"jira:ABC-42","field":"status","value":"Done"}"#)).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn update_field_rejects_bad_field() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));
    let response = app.oneshot(Request::builder().method("PATCH").uri("/api/work-item-field")
        .header("content-type","application/json")
        .body(Body::from(r#"{"id":"github:o/r#1","field":"middle","value":"x"}"#)).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_field_rejects_fixture_mode_for_status() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));
    let response = app.oneshot(Request::builder().method("PATCH").uri("/api/work-item-field")
        .header("content-type","application/json")
        .body(Body::from(r#"{"id":"github:o/r#1","field":"status","value":"Done"}"#)).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn update_field_rejects_bad_date_for_start() {
    let app = router(app_state(fixture_path("github"), fixture_path("jira")));
    let response = app.oneshot(Request::builder().method("PATCH").uri("/api/work-item-field")
        .header("content-type","application/json")
        .body(Body::from(r#"{"id":"github:o/r#1","field":"start","value":"07/01/2026"}"#)).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```
Keep the existing cache-invalidation success test but point it at the new handler name (`fetch_work_item_field`) and body `{ field:"start", value:"2026-07-01" }`.

**Step 2: Run tests to verify they fail**

Run: `cargo test -p quasar api::tests::update_field -- --nocapture`
Expected: FAIL — route/handler missing.

**Step 3: Write minimal implementation**

In `crates/quasar/src/api.rs`:
- Rename the route: `.route("/api/work-item-field", patch(update_work_item_field))` (remove the `/api/work-item-dates` route).
- Rename request struct + handler + helper. Replace `UpdateDatesRequest`/`update_work_item_dates`/`fetch_work_item_dates` with:

```rust
#[derive(Deserialize)]
struct UpdateFieldRequest {
    id: String,
    field: String,
    #[serde(default)]
    value: Option<String>,
}

async fn update_work_item_field(
    State(state): State<AppState>,
    Json(body): Json<UpdateFieldRequest>,
) -> Result<Json<UpdateDatesResponse>, (StatusCode, String)> {
    fetch_work_item_field(&state, &body)
        .map(|_| Json(UpdateDatesResponse { ok: true }))
        .map_err(|error| (error.status, error.message))
}

fn fetch_work_item_field(state: &AppState, body: &UpdateFieldRequest) -> Result<(), DetailError> {
    // field must be known
    let field = body.field.as_str();
    if !matches!(field, "start" | "target" | "status") {
        return Err(DetailError { status: StatusCode::BAD_REQUEST, message: format!("unknown field: {field}") });
    }
    if body.id.starts_with("jira:") {
        return Err(DetailError { status: StatusCode::CONFLICT, message: "Jira fields are read-only".into() });
    }
    let rest = body.id.strip_prefix("github:").ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST, message: format!("unrecognized work-item id: {}", body.id) })?;
    let (repo, number) = rest.rsplit_once('#').ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST, message: format!("malformed GitHub id: {}", body.id) })?;
    if number.is_empty() { return Err(DetailError { status: StatusCode::BAD_REQUEST, message: "missing issue number".into() }); }
    if !number.chars().all(|c| c.is_ascii_digit()) {
        return Err(DetailError { status: StatusCode::BAD_REQUEST, message: "issue number must be numeric".into() });
    }
    // date fields validate the value format
    if matches!(field, "start" | "target") {
        if let Some(v) = body.value.as_deref() {
            if !v.is_empty() && !is_iso_date(v) {
                return Err(DetailError { status: StatusCode::BAD_REQUEST, message: "date must be YYYY-MM-DD".into() });
            }
        }
    }
    match &state.github_source {
        GitHubSource::Fixture(_) => return Err(DetailError { status: StatusCode::CONFLICT, message: "writes unavailable in fixture mode".into() }),
        GitHubSource::Cli => {}
    }
    let project = state.github_project.as_ref().ok_or_else(|| DetailError {
        status: StatusCode::CONFLICT, message: "no github_project configured; cannot edit fields".into() })?;

    let result = match field {
        "start" => adapters::github::set_project_date(state.runner.as_ref(), repo, number, project, adapters::github::DateField::Start, body.value.as_deref()),
        "target" => adapters::github::set_project_date(state.runner.as_ref(), repo, number, project, adapters::github::DateField::Target, body.value.as_deref()),
        _ /* status */ => adapters::github::set_project_status(state.runner.as_ref(), repo, number, project, body.value.as_deref()),
    };
    result.map_err(|error| DetailError { status: StatusCode::BAD_GATEWAY, message: error.to_string() })?;

    state.cache.invalidate("work-items");
    Ok(())
}
```
Keep the existing `is_iso_date` helper (from the date feature). Keep `UpdateDatesResponse { ok: bool }` (or rename to `UpdateFieldResponse` — if you rename, update references; renaming is optional).

**Step 4: Run tests + full suite**

Run: `cargo test -p quasar -- --nocapture` — all pass.

**Step 5: fmt + commit**

```bash
git add crates/quasar/src/api.rs
git commit -m "feat: generalize write endpoint to /api/work-item-field (dates + status)"
```

---

## Task 7: Frontend client + types

**Files:**
- Modify: `apps/frontend/src/api.ts`
- Modify: `apps/frontend/src/types.ts`

**Step 1: Update types**

In `apps/frontend/src/types.ts`:
- Change `DateFieldKind` to a broader `WorkItemFieldKind`:
```ts
export type WorkItemFieldKind = "start" | "target" | "status";
```
(Keep `DateFieldKind` as an alias if anything still imports it, or update all importers. Prefer updating importers.)
- Add the two new fields to `WorkItemDetail`:
```ts
  project_status: string | null;
  status_options: string[];
```

**Step 2: Update the client**

In `apps/frontend/src/api.ts`, replace `updateWorkItemDate` with:
```ts
export async function updateWorkItemField(
  id: string,
  field: WorkItemFieldKind,
  value: string | null,
  signal?: AbortSignal,
): Promise<void> {
  const response = await fetch("/api/work-item-field", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id, field, value: value && value.length ? value : null }),
    signal,
  });
  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }
}
```
Update the type import accordingly (`WorkItemFieldKind`, `WorkItemDetail`, `WorkItemsResponse`).

**Step 3: Verify compile**

Run: `cd apps/frontend && npx tsc --noEmit`
Expected: errors ONLY where `ItemDetailModal.tsx` still references the old `updateWorkItemDate`/`DateFieldKind` — those are fixed in Task 8. To keep this task green on its own, it is acceptable that tsc reports errors localized to ItemDetailModal until Task 8; note them. (Alternatively do Tasks 7 and 8 back-to-back before running the full suite.)

**Step 4: Commit**

```bash
git add apps/frontend/src/api.ts apps/frontend/src/types.ts
git commit -m "feat: generalize frontend client to updateWorkItemField + detail status types"
```

---

## Task 8: Modal — seed dates from detail + editable Status dropdown

**Files:**
- Modify: `apps/frontend/src/components/ItemDetailModal.tsx`
- Modify: `apps/frontend/src/components/ItemDetailModal.test.tsx`

**Step 1: Write the failing tests**

The existing test file has a github `detail` fixture. Extend it to include `project_status` and `status_options` (add to the fixture object: `project_status: "Todo", status_options: ["Todo", "In Progress", "Done"]`). Update existing EditableDate tests to use `updateWorkItemField` (renamed) — change `jest.spyOn(api, "updateWorkItemDate")` to `jest.spyOn(api, "updateWorkItemField")` and the expected call args to `(id, "start", "2026-08-01", anything)`.

Add:

```tsx
test("editing github status calls updateWorkItemField with the option name", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue({
    ...detail,
    project_status: "Todo",
    status_options: ["Todo", "In Progress", "Done"],
  });
  const updateSpy = jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);
  const onItemUpdated = jest.fn();

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} onItemUpdated={onItemUpdated} />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());

  const select = screen.getByLabelText("Board Status") as HTMLSelectElement;
  expect(select.value).toBe("Todo");
  fireEvent.change(select, { target: { value: "In Progress" } });

  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith("github:openai/quasar#123", "status", "In Progress", expect.anything()));
  await waitFor(() => expect(onItemUpdated).toHaveBeenCalled());
});

test("date inputs seed from the detail item's dates", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue({
    ...detail,
    item: { ...detail.item, start_date: "2026-06-01", target_date: "2026-06-15" },
  });
  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  expect((screen.getByLabelText("Start date") as HTMLInputElement).value).toBe("2026-06-01");
});

test("jira renders no board status control", async () => {
  const jiraDetail = { ...detail, item: { ...detail.item, source: "jira", id: "jira:ABC-42", repo: null } };
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(jiraDetail as any);
  render(<ItemDetailModal itemId="jira:ABC-42" onClose={() => {}} />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  expect(screen.queryByLabelText("Board Status")).toBeNull();
});
```

**Step 2: Run tests to verify they fail**

Run: `cd apps/frontend && npx jest ItemDetailModal`
Expected: FAIL — no "Board Status" control; `updateWorkItemField` not used yet.

**Step 3: Implement**

In `ItemDetailModal.tsx`:
- Update imports: `import { fetchWorkItemDetail, updateWorkItemField } from "../api";` and `import type { WorkItemFieldKind, WorkItemDetail } from "../types";`
- In `EditableDate`, change the save call from `updateWorkItemDate(itemId, field, ...)` to `updateWorkItemField(itemId, field, ...)` (the `field` prop is now `WorkItemFieldKind` but only "start"/"target" are passed).
- Add a `Board Status` row in the sidebar `<dl>`, GitHub only, AFTER the existing rows. The EXISTING `<dt>Status</dt><dd>{item.status}</dd>` row (issue open/closed or Jira workflow status) stays unchanged. Add:
```tsx
{item.source === "github" ? (
  <>
    <dt>Board Status</dt>
    <dd>
      <EditableStatus
        itemId={item.id}
        initial={detail?.project_status ?? ""}
        options={detail?.status_options ?? []}
        onSaved={onItemUpdated}
      />
    </dd>
  </>
) : null}
```
- Add the `EditableStatus` component (mirror EditableDate's lifecycle: lastCommitted ref, AbortController wired to unmount, aria-live status):
```tsx
function EditableStatus({
  itemId,
  initial,
  options,
  onSaved,
}: {
  itemId: string;
  initial: string;
  options: string[];
  onSaved?: () => void;
}) {
  const [value, setValue] = useState(initial);
  const lastCommitted = useRef(initial);
  const [state, setState] = useState<SaveState>("idle");
  const controllerRef = useRef<AbortController | null>(null);

  useEffect(() => () => controllerRef.current?.abort(), []);

  async function save(next: string) {
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    setState("saving");
    try {
      await updateWorkItemField(itemId, "status", next ? next : null, controller.signal);
      if (!controller.signal.aborted) {
        lastCommitted.current = next;
        setState("saved");
        onSaved?.();
      }
    } catch {
      if (!controller.signal.aborted) setState("error");
    }
  }

  return (
    <span className="editable-status">
      <select
        aria-label="Board Status"
        className="status-select"
        onChange={(event) => {
          setValue(event.target.value);
          if (event.target.value !== lastCommitted.current) {
            void save(event.target.value);
          }
        }}
        value={value}
      >
        <option value="">(none)</option>
        {options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
      <span aria-live="polite" className="date-status-live">
        {state === "saving" ? <span className="date-status">Saving…</span> : null}
        {state === "saved" ? <span className="date-status date-status-ok">Saved</span> : null}
        {state === "error" ? <span className="date-status date-status-err">Couldn't save</span> : null}
      </span>
    </span>
  );
}
```
`SaveState` type already exists (from EditableDate). Reuse it. Ensure `useRef`/`useEffect` are imported.

Edge case: if `initial` (project_status) is a value not present in `options`, the `<select>` would have no matching `<option>`. Because we always render `(none)` + the options list, and `project_status` should be one of the options, this is fine; if it ever isn't, the select falls back to the first option visually — acceptable, and a save only fires on user change.

**Step 4: Run tests + tsc + full suite**

Run: `cd apps/frontend && npx jest ItemDetailModal` — all pass.
Run: `cd apps/frontend && npx tsc --noEmit` — clean.
Run: `cd apps/frontend && npm test` — all pass (App.detail test may reference `updateWorkItemDate`; if so, update it to `updateWorkItemField` — see Task 9 note).

**Step 5: Commit**

```bash
git add apps/frontend/src/components/ItemDetailModal.tsx apps/frontend/src/components/ItemDetailModal.test.tsx
git commit -m "feat: editable board status + date defaults in detail modal"
```

---

## Task 9: Fix App test reference + styles

**Files:**
- Modify: `apps/frontend/src/App.detail.test.tsx` (if it references the old client)
- Modify: `apps/frontend/src/styles.css`

**Step 1: Update App.detail.test.tsx**

The refetch-after-save test spies on `api.updateWorkItemDate`. Rename to `api.updateWorkItemField` (the save-a-date test still edits a date, so it calls `updateWorkItemField(id, "start", ...)`). Run `cd apps/frontend && npx jest App.detail` and confirm green.

**Step 2: Add CSS**

Append to `apps/frontend/src/styles.css`, matching the existing `.date-input` / `.date-status*` styling:
```css
.editable-status { display: inline-flex; align-items: center; gap: 0.4rem; flex-wrap: wrap; }
.status-select {
  font: inherit;
  padding: 0.15rem 0.4rem;
  border: 1px solid rgba(20, 33, 61, 0.2);
  border-radius: 6px;
  background: #fff;
  color: inherit;
}
```

**Step 3: Verify**

Run: `cd apps/frontend && npm test` — all pass.
Run: `cd apps/frontend && npm run build` — succeeds.
Run: `cd apps/frontend && npx tsc --noEmit` — clean.

**Step 4: Commit**

```bash
git add apps/frontend/src/App.detail.test.tsx apps/frontend/src/styles.css
git commit -m "style: board status select; update App test to updateWorkItemField"
```

---

## Task 10: Verification + README

**Files:**
- Modify: `README.md`

**Step 1: Backend guard smoke test (fixtures mode)**

Start `QUASAR_MODE=fixtures cargo run -p quasar &`; wait for boot (127.0.0.1:3000). Verify the generalized endpoint:
- status write in fixtures → 409:
  `curl -s -o /dev/null -w "%{http_code}" -X PATCH -H 'content-type: application/json' -d '{"id":"github:openai/quasar#123","field":"status","value":"Done"}' http://127.0.0.1:3000/api/work-item-field`
- jira status → 409; bad field → 400; bad start date → 400 (see Task 6 bodies).
Kill the backend afterward (confirm `pgrep -x quasar` empty). A live GitHub status write is NOT exercised (needs project-scoped token + board). State this.

**Step 2: Full sweep**

- `cargo test -p quasar` — all pass (report counts).
- `cd apps/frontend && npm test` — all pass.
- `cd apps/frontend && npx tsc --noEmit` — clean.
- `cd apps/frontend && npm run build` — succeeds.

**Step 3: README**

Update the "Editing dates" subsection (from the prior feature) to reflect the generalized endpoint and add Status. Replace/extend so it reads roughly:

```markdown
### Editing dates and status

GitHub work-item Start/Target dates and the Projects v2 **Status** (board
single-select, distinct from the issue open/closed state) are editable inline
from the detail overlay. Opening an item enriches the detail with the item's
current dates, Status, and the available Status options via one
`gh api graphql` query. Edits issue `PATCH /api/work-item-field`
(`{ id, field: "start" | "target" | "status", value }`), which resolves the
project/field/(option)/item, adds the issue to the configured board if needed,
runs an `updateProjectV2ItemFieldValue` (or clear) mutation, and invalidates the
work-items cache. Requires a `gh` token with `project` write scope and a
`[github_project]` configured (optional `status_field`, default `"Status"`).

Jira dates/status are read-only in the UI (`acli` 1.3.22 cannot set custom
fields).
```

**Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document editable board status and generalized field endpoint"
```

---

## Final verification checklist

- `cargo test -p quasar` → all pass
- `cd apps/frontend && npm test` → all pass
- `npx tsc --noEmit` → clean; `npm run build` → succeeds
- Fixtures-mode PATCH /api/work-item-field → 409 (github write) / 409 (jira) / 400 (bad field/date)
- Manual (optional, needs project scope): open a board issue → dates pre-filled, Status dropdown shows current + options; change Status → persists on the board.

## Notes / risks for the implementer

- **One extra `gh api graphql` call per detail open** (the enrichment). Accepted.
- **Not-on-board issues**: enrichment returns empty options, so the Status dropdown shows only `(none)`; a first write auto-adds the issue. Live status editing for such issues is limited until it's on the board.
- **Status name→id** resolved server-side; the client sends option names.
- **Single-line GraphQL** only; Int vars via `-F`, others `-f`.
- **Endpoint rename** churns the date feature's endpoint/client/tests — update every reference (`/api/work-item-dates` → `/api/work-item-field`, `updateWorkItemDate` → `updateWorkItemField`, `DateFieldKind` → `WorkItemFieldKind`). Grep to be sure none remain.
- Don't run plain `cargo fmt` (reformats unrelated api.rs/main.rs); hand-fix only new lines.
- gh token `project` scope required for live writes; unverified in CI.
