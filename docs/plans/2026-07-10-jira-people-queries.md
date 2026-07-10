# Jira People Queries Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Surface every Jira ticket assigned to or created by configured people, across all projects, driven by a new `[jira_people]` config table.

**Architecture:** `config.rs` composes one extra JQL query (`assignee in (...) OR reporter in (...)`) appended to the existing per-project fan-out, so it streams as its own chunk via `acli`. A new top-level `jira_base_url` de-hardcodes the browse-link domain and is threaded through `AppState` into the Jira adapter. Overlapping tickets (in both a board project and the person set) are de-duplicated by `jira:KEY` id in both the streaming (frontend) and batch (backend) paths.

**Tech Stack:** Rust, axum, serde, `acli`; React + TypeScript, Jest.

**Design doc:** `docs/plans/2026-07-10-jira-people-queries-design.md`

**Conventions:** Backend tests `cargo test -p quasar`; frontend `cd apps/frontend && npm test` (unpiped / redirect to a log, never through a pager) and `npx tsc --noEmit`. Commit after each task. No `Co-authored-by` trailers.

---

## Task 1: Config — `jira_base_url`, `[jira_people]`, person query composition

**Files:**
- Modify: `crates/quasar/src/config.rs`

**Step 1: Write failing tests.** Add to the `tests` module in `config.rs`:

```rust
#[test]
fn compose_jira_queries_appends_person_query_for_users() {
    let queries = super::compose_jira_queries(
        &["SSW".to_string()],
        &["a@x".to_string(), "b@x".to_string()],
        None,
    );
    assert_eq!(
        queries,
        vec![
            "project = SSW ORDER BY updated DESC".to_string(),
            "(assignee in (\"a@x\",\"b@x\") OR reporter in (\"a@x\",\"b@x\")) ORDER BY updated DESC"
                .to_string(),
        ]
    );
}

#[test]
fn compose_jira_queries_person_query_ands_raw_jql_and_honors_order() {
    let queries = super::compose_jira_queries(
        &[],
        &["a@x".to_string()],
        Some("statusCategory != Done ORDER BY created DESC"),
    );
    assert_eq!(
        queries,
        vec![
            "((assignee in (\"a@x\") OR reporter in (\"a@x\"))) AND (statusCategory != Done) ORDER BY created DESC"
                .to_string(),
        ]
    );
}

#[test]
fn compose_jira_queries_no_projects_no_users_is_unchanged() {
    assert_eq!(
        super::compose_jira_queries(&[], &[], Some("project = X order by created desc")),
        vec!["project = X order by created desc".to_string()]
    );
    assert_eq!(
        super::compose_jira_queries(&[], &[], None),
        vec!["ORDER BY updated DESC".to_string()]
    );
}

#[test]
fn loads_jira_people_and_base_url_from_toml() {
    let home_dir = TestDir::new();
    let config_path = write_config(
        home_dir.path(),
        r#"
jira_base_url = "https://acme.atlassian.net"

[jira_board]
projects = ["SSW"]

[jira_people]
users = ["a@x", "b@x"]
"#,
    );
    let config =
        load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
    assert_eq!(config.jira_base_url, "https://acme.atlassian.net");
    assert_eq!(
        config.jira_queries,
        vec![
            "project = SSW ORDER BY updated DESC".to_string(),
            "(assignee in (\"a@x\",\"b@x\") OR reporter in (\"a@x\",\"b@x\")) ORDER BY updated DESC"
                .to_string(),
        ]
    );
}

#[test]
fn jira_people_rejects_user_with_whitespace() {
    let home_dir = TestDir::new();
    let config_path = write_config(
        home_dir.path(),
        "[jira_people]\nusers = [\"a b@x\"]\n",
    );
    let error = load_runtime_config(&config_path, EnvOverrides::default())
        .expect_err("whitespace user should be rejected");
    assert!(matches!(error, ConfigError::InvalidJiraUser(_)));
}

#[test]
fn jira_base_url_defaults_when_absent() {
    let home_dir = TestDir::new();
    let config_path = write_config(home_dir.path(), "github_repos = []\n");
    let config =
        load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
    assert_eq!(config.jira_base_url, "https://quera.atlassian.net");
}
```

Run: `cargo test -p quasar --lib config` — expect compile failure / failures (new field + fn arity).

**Step 2: Add the `RuntimeConfig` field.** In `struct RuntimeConfig`, add after `jira_queries`:
```rust
    /// Jira site domain (e.g. `https://quera.atlassian.net`), used to build
    /// browse links. Defaults to the QuEra site.
    pub jira_base_url: String,
```

**Step 3: Add the config types.** Add a `JiraPeople` struct near `JiraBoard`:
```rust
/// People whose related tickets (assigned to / created by) are pulled across
/// all projects, configured as a `[jira_people]` table. Users are emails.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JiraPeople {
    #[serde(default)]
    pub users: Vec<String>,
}
```
Add to `struct FileConfig`:
```rust
    jira_base_url: Option<String>,
    jira_people: Option<JiraPeople>,
```

**Step 4: Add the error variant.** Add `InvalidJiraUser(String)` to `enum ConfigError`, and a `Display` arm:
```rust
Self::InvalidJiraUser(user) => {
    write!(f, "invalid Jira user: {user} (must be a non-empty email with no whitespace)")
}
```

**Step 5: Add validation.** Add:
```rust
fn validate_jira_users(users: &[String]) -> Result<(), ConfigError> {
    for user in users {
        if user.is_empty() || user.chars().any(char::is_whitespace) {
            return Err(ConfigError::InvalidJiraUser(user.clone()));
        }
    }
    Ok(())
}
```

**Step 6: Change `compose_jira_queries`** to take users and append the person query. Replace the function with:
```rust
fn compose_jira_queries(
    projects: &[String],
    users: &[String],
    raw_jql: Option<&str>,
) -> Vec<String> {
    const DEFAULT_ORDER: &str = "ORDER BY updated DESC";
    let raw = raw_jql.map(str::trim).filter(|jql| !jql.is_empty());

    // Escape hatch preserved: with no projects AND no users, the raw JQL (or a
    // bare default) is the sole query, verbatim.
    if projects.is_empty() && users.is_empty() {
        return vec![raw.unwrap_or(DEFAULT_ORDER).to_string()];
    }

    let (raw_where, raw_order) = match raw {
        Some(raw) => split_order_by(raw),
        None => ("", None),
    };
    let order = raw_order.unwrap_or(DEFAULT_ORDER);

    let mut queries: Vec<String> = projects
        .iter()
        .map(|key| {
            if raw_where.is_empty() {
                format!("project = {key} {order}")
            } else {
                format!("(project = {key}) AND ({raw_where}) {order}")
            }
        })
        .collect();

    if !users.is_empty() {
        let list = users
            .iter()
            .map(|user| format!("\"{user}\""))
            .collect::<Vec<_>>()
            .join(",");
        let clause = format!("(assignee in ({list}) OR reporter in ({list}))");
        let where_part = if raw_where.is_empty() {
            clause
        } else {
            format!("({clause}) AND ({raw_where})")
        };
        queries.push(format!("{where_part} {order}"));
    }

    queries
}
```

**Step 7: Wire into `load_runtime_config`.** After `validate_jira_projects(&jira_projects)?;` add:
```rust
    let jira_users = file_config
        .jira_people
        .map(|people| people.users)
        .unwrap_or_default();
    validate_jira_users(&jira_users)?;
    let jira_base_url = file_config
        .jira_base_url
        .unwrap_or_else(default_jira_base_url);
```
Change the `jira_queries` line to:
```rust
    let jira_queries = compose_jira_queries(&jira_projects, &jira_users, raw_jira_jql.as_deref());
```
Add `jira_base_url,` to the returned `RuntimeConfig { ... }`.

**Step 8: Fix existing full-struct-equality tests.** Two tests build a whole `RuntimeConfig` literal (`loads_multiple_github_repos_from_toml`, `missing_file_uses_defaults`) — add `jira_base_url: "https://quera.atlassian.net".to_string(),` to each. Every existing `compose_jira_queries(...)` call in tests now needs a `&[]` users arg inserted as the second argument — update them all.

**Step 9: Run tests.** Run: `cargo test -p quasar --lib config` — expect PASS.

**Step 10: Commit.**
```bash
git add crates/quasar/src/config.rs
git commit -m "feat: compose a Jira person query from [jira_people] users"
```

---

## Task 2: Thread `jira_base_url` end-to-end (browse links + AppState + main)

**Files:**
- Modify: `crates/quasar/src/adapters/jira.rs`
- Modify: `crates/quasar/src/api.rs`
- Modify: `crates/quasar/src/main.rs`

**Step 1: Jira adapter — take `base_url` for browse links.** In `adapters/jira.rs`:
- Delete the `JIRA_BROWSE_BASE` constant.
- Add `base_url: &str` as the LAST parameter to these public fns: `load_fixture_work_items`, `load_work_items_with_runner`, `load_fixture_issue_detail`, `fetch_issue_detail`. Thread it into the internal fns `normalize_work_items(raw, base_url)`, `normalize_issue(issue, base_url)`, and `normalize_issue_detail(raw, base_url)`.
- In `normalize_issue` and `normalize_issue_detail`, replace `let url = format!("{JIRA_BROWSE_BASE}/{external_id}");` with:
```rust
let url = format!("{}/browse/{external_id}", base_url.trim_end_matches('/'));
```
- `load_work_items_with_runner` calls `normalize_work_items(&raw, base_url)?` then `enrich_planning_dates(runner, &mut items)` (unchanged). `load_fixture_work_items` calls `normalize_work_items(&raw, base_url)`.

**Step 2: Update Jira adapter tests** for the new signatures. Add a small const at the top of the `tests` module: `const TEST_BASE: &str = "https://quera.atlassian.net";`. Then:
- `load_fixture_work_items(&fixture_path())` → `load_fixture_work_items(&fixture_path(), TEST_BASE)`.
- `load_work_items_with_runner(&runner, "order by updated desc")` → add `, TEST_BASE`.
- `load_fixture_issue_detail(&detail_fixture_path())` → add `, TEST_BASE`.
- `fetch_issue_detail(&runner, "ABC-42")` → add `, TEST_BASE`.
- The existing URL assertions (`.../browse/ABC-42`) remain valid.
- Add a new test proving the base is honored:
```rust
#[test]
fn jira_browse_url_uses_provided_base() {
    let items = load_fixture_work_items(&fixture_path(), "https://acme.atlassian.net/")
        .expect("fixture should load");
    assert_eq!(items[0].url, "https://acme.atlassian.net/browse/ABC-42");
}
```

**Step 3: `AppState` — add the field.** In `api.rs`, add `pub jira_base_url: String,` to `struct AppState`, add a `jira_base_url: String` parameter to `AppState::new` (place it right after `jira_queries`), and set it in the constructed `Self { ... }`.

**Step 4: Update the Jira adapter call sites in `api.rs`.** Pass `&state.jira_base_url` as the new last arg to: `load_fixture_work_items`, `load_work_items_with_runner`, `load_fixture_issue_detail`, `fetch_issue_detail`.

**Step 5: Update `main.rs`.** In `build_app_state`, add `config.jira_base_url.clone(),` to BOTH `AppState::new(...)` calls (right after `config.jira_queries.clone(),`). In the `startup_resolver_reads_user_config_file` test, add `jira_base_url: "https://quera.atlassian.net".to_string(),` to the `RuntimeConfig` literal.

**Step 6: Update all `AppState` literals + helpers in `api.rs` tests.** The compiler will flag every construction. Add `jira_base_url: "https://quera.atlassian.net".to_string(),` to each: the `app_state` helper, `jira_cli_state`, `jira_write_state`, and every inline `AppState { ... }` in the tests module. For any `AppState::new(...)` calls in tests, insert the base-url arg in the right position.

**Step 7: Build + test.** Run: `cargo test -p quasar` — expect PASS (fix any missed call site the compiler names).

**Step 8: Commit.**
```bash
git add crates/quasar/src
git commit -m "feat: configurable jira_base_url for browse links"
```

---

## Task 3: De-duplicate overlapping tickets by id

**Files:**
- Modify: `crates/quasar/src/api.rs` (batch path + test)
- Modify: `apps/frontend/src/App.tsx` (stream merge)
- Modify: `apps/frontend/src/App.test.tsx` (test)

**Step 1: Backend failing test.** In the `api.rs` tests module, add a test that two Jira queries returning the same key yield one item. Model it on `resolve_work_items_streams_one_chunk_per_jira_project` / `fetch_work_items_keeps_successful_jira_project_items_when_one_query_fails` (use `jira_cli_state` + `JiraQueryMock`), with two queries whose payloads share `SSW-1`:
```rust
#[test]
fn resolve_work_items_dedupes_items_with_same_id() {
    let queries = vec![
        "project = SSW ORDER BY updated DESC".to_string(),
        "(assignee in (\"a@x\")) ORDER BY updated DESC".to_string(),
    ];
    let runner = JiraQueryMock {
        by_jql: HashMap::from([
            (queries[0].clone(), Ok(jira_search_payload("SSW-1", "dup"))),
            (queries[1].clone(), Ok(jira_search_payload("SSW-1", "dup"))),
        ]),
    };
    let state = jira_cli_state(runner, queries);
    let response = fetch_work_items(&state);
    let ssw1 = response
        .data
        .iter()
        .filter(|item| item.id == "jira:SSW-1")
        .count();
    assert_eq!(ssw1, 1, "duplicate ids should be collapsed");
}
```
Run: `cargo test -p quasar resolve_work_items_dedupes` — expect FAIL (two copies today).

**Step 2: Backend implementation.** In `resolve_work_items`, right after `data.sort_by(|left, right| left.id.cmp(&right.id));` add:
```rust
    // A ticket can match multiple queries (e.g. a board project and the person
    // query); collapse duplicates by id (sorted above, so dups are adjacent).
    data.dedup_by(|a, b| a.id == b.id);
```
Run: `cargo test -p quasar resolve_work_items_dedupes` — expect PASS.

**Step 3: Frontend failing test.** In `App.test.tsx`, read the existing `streamResponse` helper to see how NDJSON chunks are built. Add a test that a stream with two `items` chunks sharing an id renders a single card. If `streamResponse` emits a single chunk, construct a raw two-line NDJSON body inline (one `{"type":"items",...}` line per chunk, then `{"type":"done",...}`) following that helper's shape. Assert the duplicated title appears once (e.g. `screen.getAllByRole("button", { name: "Dup issue" })` has length 1). Run and confirm it fails.

**Step 4: Frontend implementation.** In `App.tsx`, in the `onChunk` handler's `setResponse` updater, dedupe by id when merging. Replace the `data: [...base.data, ...data]` construction with:
```tsx
              const seen = new Set(base.data.map((item) => item.id));
              const fresh = data.filter((item) => !seen.has(item.id));
              return {
                ...base,
                data: [...base.data, ...fresh],
                warnings: [...base.warnings, ...warnings],
              };
```
Run: `cd apps/frontend && npm test` — expect PASS; `npx tsc --noEmit` clean.

**Step 5: Commit.**
```bash
git add crates/quasar/src/api.rs apps/frontend/src
git commit -m "feat: de-duplicate work items shared across queries by id"
```

---

## Task 4: Docs — README

**Files:**
- Modify: `README.md`

**Step 1:** In the Jira configuration / data-fetching sections, document:
- `jira_base_url` (top-level, optional, default `https://quera.atlassian.net`) — the Jira domain, used for browse links; note it should match the site `acli` is authenticated to and the write path's `[jira].base_url`.
- `[jira_people]` with `users = ["email", ...]` — pulls tickets **assigned to or created by** those people across **all** projects (one extra `acli` query: `assignee in (...) OR reporter in (...)`), merged with board results and de-duplicated by issue key.
- The volume note: the person query can be large; bound it with `jira_jql` (e.g. `statusCategory != Done`), which is AND'd into the person query too. Each fetched Jira item still costs a per-issue `view` call for date enrichment.
- Update the example `config.toml` to show `jira_base_url` and a `[jira_people]` block.

**Step 2: Commit.**
```bash
git add README.md
git commit -m "docs: jira_base_url and [jira_people] people queries"
```

---

## Final verification

- `cargo test -p quasar` — all pass.
- `cd apps/frontend && npm test && npx tsc --noEmit` — pass + clean.
- Manual smoke (live creds): set `[jira_people] users = ["khwu@quera.com"]` and `jira_jql = "statusCategory != Done"`, run `mise run dev`, confirm the person's tickets from non-board projects appear and no ticket is duplicated. (Use `/run` or `/verify`.)
