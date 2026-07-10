# People Page Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** A new on-demand "People" tab that, for a selected preconfigured `[jira_people]` person, lists the Jira tickets they created (reporter) or are mentioned in (full-text by accountId).

**Architecture:** Backend retains the configured people + raw `jira_jql` in `AppState`, exposes `GET /api/people` (config list) and a lazy `GET /api/person-work-items?user=<email>` that runs a created-by query, derives the person's accountId from the reporter field, runs a mentioned full-text query, and returns two deduped lists (no per-issue date enrichment). Frontend adds a People tab that fetches the people list on entry and the person's tickets on selection.

**Tech Stack:** Rust, axum, serde, `acli`; React + TypeScript, Jest.

**Design doc:** `docs/plans/2026-07-10-people-page-design.md`

**Conventions:** Backend `cargo test -p quasar`; frontend `cd apps/frontend && npm test` (unpiped / log file, never a pager) + `npx tsc --noEmit`. Commit per task. No `Co-authored-by` trailers.

---

## Task 1: Config — retain people + jql, compose person queries

**Files:**
- Modify: `crates/quasar/src/config.rs`
- Modify: `crates/quasar/src/main.rs` (one test literal only)

**Step 1: Failing tests** in the `config.rs` tests module:
```rust
#[test]
fn compose_person_queries_created_by_only_without_account() {
    let q = super::compose_person_queries("a@x", None, None);
    assert_eq!(q.created_by, "reporter = \"a@x\" ORDER BY updated DESC");
    assert_eq!(q.mentioned, None);
}

#[test]
fn compose_person_queries_includes_mentioned_when_account_present() {
    let q = super::compose_person_queries("a@x", Some("acc:1"), None);
    assert_eq!(q.created_by, "reporter = \"a@x\" ORDER BY updated DESC");
    assert_eq!(
        q.mentioned.as_deref(),
        Some("text ~ \"acc:1\" ORDER BY updated DESC")
    );
}

#[test]
fn compose_person_queries_ands_jira_jql_and_honors_order() {
    let q = super::compose_person_queries(
        "a@x",
        Some("acc:1"),
        Some("statusCategory != Done ORDER BY created DESC"),
    );
    assert_eq!(
        q.created_by,
        "(reporter = \"a@x\") AND (statusCategory != Done) ORDER BY created DESC"
    );
    assert_eq!(
        q.mentioned.as_deref(),
        Some("(text ~ \"acc:1\") AND (statusCategory != Done) ORDER BY created DESC")
    );
}

#[test]
fn runtime_config_retains_jira_people_and_jql() {
    let home_dir = TestDir::new();
    let config_path = write_config(
        home_dir.path(),
        "jira_jql = \"statusCategory != Done\"\n\n[jira_people]\nusers = [\"a@x\"]\n",
    );
    let config =
        load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
    assert_eq!(config.jira_people, vec!["a@x".to_string()]);
    assert_eq!(config.jira_jql.as_deref(), Some("statusCategory != Done"));
}
```
Run `cargo test -p quasar --lib config` — expect failures.

**Step 2: RuntimeConfig fields.** Add to `struct RuntimeConfig`, after `jira_base_url`:
```rust
    /// Emails from `[jira_people]`, retained for the People page endpoints.
    pub jira_people: Vec<String>,
    /// The raw `jira_jql` filter (pre-composition), retained to bound the
    /// on-demand person queries the same way board queries are bounded.
    pub jira_jql: Option<String>,
```

**Step 3: `PersonQueries` + `compose_person_queries`.** Add (pub, near `compose_jira_queries`):
```rust
/// The JQL pair for a People-page fetch: tickets created by the person and,
/// when an accountId is known, tickets mentioning them (full-text proxy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonQueries {
    pub created_by: String,
    pub mentioned: Option<String>,
}

/// Compose the created-by (`reporter = email`) and mentioned
/// (`text ~ accountId`) JQL for a person, each AND'd with the raw `jira_jql`
/// where-clause when present and sharing its ORDER BY (else a default).
pub fn compose_person_queries(
    email: &str,
    account_id: Option<&str>,
    raw_jql: Option<&str>,
) -> PersonQueries {
    const DEFAULT_ORDER: &str = "ORDER BY updated DESC";
    let raw = raw_jql.map(str::trim).filter(|jql| !jql.is_empty());
    let (raw_where, raw_order) = match raw {
        Some(raw) => split_order_by(raw),
        None => ("", None),
    };
    let order = raw_order.unwrap_or(DEFAULT_ORDER);
    let with_filter = |clause: String| {
        if raw_where.is_empty() {
            format!("{clause} {order}")
        } else {
            format!("({clause}) AND ({raw_where}) {order}")
        }
    };
    PersonQueries {
        created_by: with_filter(format!("reporter = \"{email}\"")),
        mentioned: account_id.map(|id| with_filter(format!("text ~ \"{id}\""))),
    }
}
```

**Step 4: Retain the values in `load_runtime_config`.** The `jira_users` binding already exists (from the earlier feature). Keep composing `jira_queries` from it, but also stash it and the raw jql into the result. Where `RuntimeConfig { ... }` is built, add:
```rust
        jira_people: jira_users,
        jira_jql: raw_jira_jql,
```
(Confirm `jira_users` isn't moved before this — it's used by `compose_jira_queries(&jira_projects, &jira_users, ...)` which borrows; pass by reference there so `jira_users` remains owned for the struct. It already takes `&jira_users`, so it's fine.) `raw_jira_jql` is the `Option<String>` already bound above; if it's consumed by `compose_jira_queries` via `.as_deref()` (a borrow), it's still available to move here.

**Step 5: Update existing full-literal tests.** Add `jira_people: Vec::new(), jira_jql: None,` to the `RuntimeConfig { ... }` literals in `loads_multiple_github_repos_from_toml` and `missing_file_uses_defaults`. In `crates/quasar/src/main.rs`, add the same two fields to the `RuntimeConfig` literal in `startup_resolver_reads_user_config_file` (this keeps the full suite compiling).

**Step 6: Run.** `cargo test -p quasar` — expect PASS.

**Step 7: Commit.**
```bash
git add crates/quasar/src/config.rs crates/quasar/src/main.rs
git commit -m "feat: retain jira_people/jira_jql and compose person queries"
```

---

## Task 2: Jira adapter — search without enrichment + accountId resolver

**Files:**
- Modify: `crates/quasar/src/adapters/jira.rs`

**Step 1: Failing tests** in the `jira.rs` tests module (reuse `MockCommandRunner`, `fixture_path`, `TEST_BASE`):
```rust
#[test]
fn search_work_items_normalizes_without_enrichment() {
    let payload = std::fs::read_to_string(fixture_path()).expect("fixture");
    let runner = MockCommandRunner::success(&payload);
    let items = super::search_work_items(&runner, "reporter = \"a@x\"", TEST_BASE)
        .expect("search should normalize");
    assert_eq!(items.len(), 1);
    // Only the search call — no per-issue `view` enrichment calls.
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
}

#[test]
fn fetch_account_id_via_reporter_parses_account() {
    let payload =
        r#"[{"key":"X-1","fields":{"reporter":{"accountId":"acc:1","displayName":"Ann"}}}]"#;
    let runner = MockCommandRunner::success(payload);
    let got = super::fetch_account_id_via_reporter(&runner, "a@x");
    assert_eq!(got, Some(("acc:1".to_string(), "Ann".to_string())));
}

#[test]
fn fetch_account_id_via_reporter_none_when_empty() {
    let runner = MockCommandRunner::success("[]");
    assert_eq!(super::fetch_account_id_via_reporter(&runner, "a@x"), None);
}
```
Run `cargo test -p quasar --lib adapters::jira` — expect failure.

**Step 2: Extract `search_work_items`.** Refactor: the body of `load_work_items_with_runner` currently does the `acli ... search ...` run + `normalize_work_items` + `enrich_planning_dates`. Split it:
```rust
/// Search + normalize only (no per-issue date enrichment). Used by the People
/// page, where the result set can be large and dates aren't needed.
pub fn search_work_items(
    runner: &dyn CommandRunner,
    jql: &str,
    base_url: &str,
) -> AdapterResult<Vec<WorkItem>> {
    let raw = runner
        .run(
            "acli",
            &[
                "jira", "workitem", "search", "--jql", jql, "--paginate", "--json",
                "--fields", "key,summary,status,assignee,priority,reporter,labels",
            ],
        )
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    normalize_work_items(&raw, base_url)
}

pub fn load_work_items_with_runner(
    runner: &dyn CommandRunner,
    jql: &str,
    base_url: &str,
) -> AdapterResult<Vec<WorkItem>> {
    let mut items = search_work_items(runner, jql, base_url)?;
    enrich_planning_dates(runner, &mut items);
    Ok(items)
}
```

**Step 3: Add `fetch_account_id_via_reporter`.**
```rust
/// Best-effort: resolve a person's `(accountId, displayName)` from the reporter
/// field of one of their created tickets (a single `--limit 1` search). `None`
/// if they've created nothing or on any failure. Avoids needing `[jira]` creds.
pub fn fetch_account_id_via_reporter(
    runner: &dyn CommandRunner,
    email: &str,
) -> Option<(String, String)> {
    let jql = format!("reporter = \"{email}\"");
    let raw = runner
        .run(
            "acli",
            &[
                "jira", "workitem", "search", "--jql", &jql, "--fields", "key,reporter",
                "--limit", "1", "--json",
            ],
        )
        .ok()?;
    #[derive(Deserialize)]
    struct Issue {
        fields: IssueFields,
    }
    #[derive(Deserialize)]
    struct IssueFields {
        reporter: Option<Reporter>,
    }
    #[derive(Deserialize)]
    struct Reporter {
        #[serde(rename = "accountId")]
        account_id: String,
        #[serde(rename = "displayName")]
        display_name: String,
    }
    let issues: Vec<Issue> = serde_json::from_str(&raw).ok()?;
    let reporter = issues.into_iter().next()?.fields.reporter?;
    Some((reporter.account_id, reporter.display_name))
}
```

**Step 4: Run.** `cargo test -p quasar` — expect PASS (existing board tests still pass since `load_work_items_with_runner` behavior is unchanged).

**Step 5: Commit.**
```bash
git add crates/quasar/src/adapters/jira.rs
git commit -m "feat: jira search_work_items (no enrichment) and accountId resolver"
```

---

## Task 3: API — `/api/people` and `/api/person-work-items`

**Files:**
- Modify: `crates/quasar/src/api.rs`
- Modify: `crates/quasar/src/main.rs`

**Step 1: AppState fields.** Add to `struct AppState`: `pub jira_people: Vec<String>,` and `pub jira_jql: Option<String>,`. Add matching params to `AppState::new` (after `jira_base_url: String`), set both in `Self { ... }`.

**Step 2: main.rs wiring.** In `build_app_state`, pass `config.jira_people.clone(), config.jira_jql.clone(),` (after `config.jira_base_url.clone(),`) in BOTH `AppState::new(...)` arms.

**Step 3: Update api.rs test AppState constructions.** Add `jira_people: Vec::new(), jira_jql: None,` to every `AppState { ... }` literal and helper (`app_state`, `jira_cli_state`, `jira_write_state`, and inline ones); add the two args to any `AppState::new(...)` calls in tests.

**Step 4: Failing API tests** (tests module in `api.rs`). Use a `jira_cli_state`-style helper but set `jira_people`. Add:
```rust
#[tokio::test]
async fn people_endpoint_lists_configured_users() {
    let mut state = app_state(fixture_path("github"), fixture_path("jira"));
    state.jira_people = vec!["a@x".to_string(), "b@x".to_string()];
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/people")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["users"][0], "a@x");
    assert_eq!(payload["users"][1], "b@x");
}

#[test]
fn person_work_items_rejects_unconfigured_user() {
    let mut state = app_state(fixture_path("github"), fixture_path("jira"));
    state.jira_people = vec!["a@x".to_string()];
    let error = super::fetch_person_work_items(&state, "stranger@x")
        .expect_err("unconfigured user should be rejected");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
}

#[test]
fn person_work_items_fixture_mode_returns_empty() {
    let mut state = app_state(fixture_path("github"), fixture_path("jira"));
    state.jira_people = vec!["a@x".to_string()];
    let result = super::fetch_person_work_items(&state, "a@x").expect("ok");
    assert!(result.created_by.is_empty() && result.mentioned.is_empty());
    assert_eq!(result.user, "a@x");
}

#[test]
fn person_work_items_dedupes_mentioned_against_created_by() {
    // Cli mode with a runner that answers the reporter-resolve, created-by, and
    // mentioned searches; created-by and mentioned both return SSW-1, plus
    // mentioned returns SSW-2. Expect created_by=[SSW-1], mentioned=[SSW-2].
    struct Runner;
    impl CommandRunner for Runner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            let jql = args
                .windows(2)
                .find_map(|w| (w[0] == "--jql").then_some(w[1]))
                .unwrap_or("");
            let has_limit = args.iter().any(|a| *a == "--limit");
            if jql.starts_with("reporter =") && has_limit {
                // accountId resolver
                Ok(r#"[{"key":"SSW-1","fields":{"reporter":{"accountId":"acc:1","displayName":"Ann"}}}]"#.to_string())
            } else if jql.starts_with("reporter =") {
                Ok(super::tests::jira_search_payload("SSW-1", "created"))
            } else {
                // text ~ mentioned
                Ok(format!(
                    "[{},{}]",
                    single_issue("SSW-1", "created"),
                    single_issue("SSW-2", "mention")
                ))
            }
        }
    }
    // Provide a `single_issue` helper or inline the JSON objects.
    let mut state = jira_cli_state_people(Runner, vec!["a@x".to_string()]);
    let result = super::fetch_person_work_items(&state, "a@x").expect("ok");
    let created: Vec<&str> = result.created_by.iter().map(|i| i.external_id.as_str()).collect();
    let mentioned: Vec<&str> = result.mentioned.iter().map(|i| i.external_id.as_str()).collect();
    assert_eq!(created, vec!["SSW-1"]);
    assert_eq!(mentioned, vec!["SSW-2"]);
}
```
(Adapt helpers to the file's existing patterns: reuse/extend `jira_search_payload`; add a small `jira_cli_state_people` that sets `jira_source: Cli`, `jira_people`, and a custom runner, plus a `single_issue(key, summary)` string builder. The exact mock wiring is the implementer's call — the assertions above are the contract.) Run and confirm failures.

**Step 5: Implement the endpoints.** Add routes in `router()`:
```rust
.route("/api/people", get(people))
.route("/api/person-work-items", get(person_work_items))
```
Add types + handlers:
```rust
#[derive(Serialize)]
struct PeopleResponse {
    users: Vec<String>,
}

async fn people(State(state): State<AppState>) -> Json<PeopleResponse> {
    Json(PeopleResponse { users: state.jira_people.clone() })
}

#[derive(Deserialize)]
struct PersonQuery {
    user: String,
}

#[derive(Serialize, Deserialize)]
struct PersonWorkItemsResponse {
    user: String,
    account_id: Option<String>,
    created_by: Vec<WorkItem>,
    mentioned: Vec<WorkItem>,
}

async fn person_work_items(
    State(state): State<AppState>,
    Query(query): Query<PersonQuery>,
) -> Result<Json<PersonWorkItemsResponse>, (StatusCode, String)> {
    fetch_person_work_items(&state, &query.user)
        .map(Json)
        .map_err(|error| (error.status, error.message))
}

fn fetch_person_work_items(
    state: &AppState,
    user: &str,
) -> Result<PersonWorkItemsResponse, DetailError> {
    if !state.jira_people.iter().any(|u| u == user) {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: format!("unknown configured person: {user}"),
        });
    }
    if matches!(state.jira_source, JiraSource::Fixture(_)) {
        return Ok(PersonWorkItemsResponse {
            user: user.to_string(),
            account_id: None,
            created_by: Vec::new(),
            mentioned: Vec::new(),
        });
    }

    let cache_key = format!("person:{user}");
    let now = Instant::now();
    if let CacheOutcome::Hit(payload) = state.cache.get(&cache_key, now) {
        if let Ok(cached) = serde_json::from_str::<PersonWorkItemsResponse>(&payload) {
            return Ok(cached);
        }
    }

    let runner = state.runner.as_ref();
    let account_id = adapters::jira::fetch_account_id_via_reporter(runner, user).map(|(id, _)| id);
    let queries =
        crate::config::compose_person_queries(user, account_id.as_deref(), state.jira_jql.as_deref());

    let created_by = adapters::jira::search_work_items(runner, &queries.created_by, &state.jira_base_url)
        .map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;

    // Mentioned is best-effort: a failure there must not sink created-by.
    let mut mentioned = match &queries.mentioned {
        Some(jql) => adapters::jira::search_work_items(runner, jql, &state.jira_base_url)
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let created_ids: std::collections::HashSet<&str> =
        created_by.iter().map(|item| item.id.as_str()).collect();
    mentioned.retain(|item| !created_ids.contains(item.id.as_str()));

    let response = PersonWorkItemsResponse {
        user: user.to_string(),
        account_id,
        created_by,
        mentioned,
    };
    if let Ok(payload) = serde_json::to_string(&response) {
        state.cache.insert(&cache_key, payload, now);
    }
    Ok(response)
}
```
Ensure `get` is imported (it is) and `CacheOutcome` is in scope (import from `crate::cache` if needed — the module already imports `ResponseCache`; add `CacheOutcome`).

**Step 6: Run.** `cargo test -p quasar` — expect PASS.

**Step 7: Commit.**
```bash
git add crates/quasar/src/api.rs crates/quasar/src/main.rs
git commit -m "feat: /api/people and /api/person-work-items endpoints"
```

---

## Task 4: Frontend — People tab + page

**Files:**
- Modify: `apps/frontend/src/types.ts`
- Modify: `apps/frontend/src/api.ts`
- Modify: `apps/frontend/src/App.tsx` (tab, view union, export `WorkItemCard`, render page)
- Create: `apps/frontend/src/components/PeoplePage.tsx`
- Create: `apps/frontend/src/components/PeoplePage.test.tsx`

**Step 1: Types.** In `types.ts` add:
```ts
export interface PersonWorkItems {
  user: string;
  account_id: string | null;
  created_by: WorkItem[];
  mentioned: WorkItem[];
}
```

**Step 2: API.** In `api.ts` add:
```ts
export async function fetchPeople(signal?: AbortSignal): Promise<string[]> {
  const response = await fetch("/api/people", { signal });
  if (!response.ok) throw new Error(`Request failed with status ${response.status}`);
  return ((await response.json()) as { users: string[] }).users;
}

export async function fetchPersonWorkItems(
  user: string,
  signal?: AbortSignal,
): Promise<import("./types").PersonWorkItems> {
  const response = await fetch(`/api/person-work-items?user=${encodeURIComponent(user)}`, { signal });
  if (!response.ok) throw new Error(`Request failed with status ${response.status}`);
  return (await response.json()) as import("./types").PersonWorkItems;
}
```

**Step 3: Export `WorkItemCard`.** In `App.tsx`, change `function WorkItemCard(...)` to `export function WorkItemCard(...)` so the page can reuse it. Add the People tab button in the `view-tabs` group and widen the state type:
- `const [view, setView] = useState<"board" | "timeline" | "people">("board");`
- Add a third `<button ... onClick={() => setView("people")} aria-selected={view === "people"}>People</button>`.
- In the render area, when `view === "people"`, render `<PeoplePage onOpenItem={setSelectedItemId} />` instead of the board/timeline block. (Guard: People view is independent of `filteredItems`; render it directly.)

**Step 4: Create `PeoplePage.tsx`:**
```tsx
import { useEffect, useRef, useState } from "react";

import { fetchPeople, fetchPersonWorkItems } from "../api";
import type { PersonWorkItems } from "../types";
import { WorkItemCard } from "../App";

export default function PeoplePage({ onOpenItem }: { onOpenItem: (id: string) => void }) {
  const [people, setPeople] = useState<string[]>([]);
  const [selected, setSelected] = useState("");
  const [data, setData] = useState<PersonWorkItems | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const controllerRef = useRef<AbortController | null>(null);

  // Fetch the configured people once when the page mounts.
  useEffect(() => {
    const controller = new AbortController();
    fetchPeople(controller.signal)
      .then((users) => {
        if (!controller.signal.aborted) setPeople(users);
      })
      .catch(() => {
        /* people list is best-effort; leave empty */
      });
    return () => controller.abort();
  }, []);

  function selectPerson(user: string) {
    setSelected(user);
    setData(null);
    setError(null);
    controllerRef.current?.abort();
    if (!user) return;
    const controller = new AbortController();
    controllerRef.current = controller;
    setIsLoading(true);
    fetchPersonWorkItems(user, controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) setData(result);
      })
      .catch((err: unknown) => {
        if (!controller.signal.aborted) {
          setError(err instanceof Error ? err.message : "Failed to load");
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setIsLoading(false);
      });
  }

  useEffect(() => () => controllerRef.current?.abort(), []);

  return (
    <section aria-label="People" className="people-page">
      <div className="filter-field">
        <label htmlFor="person-select">Person</label>
        <select
          id="person-select"
          onChange={(event) => selectPerson(event.target.value)}
          value={selected}
        >
          <option value="">Select a person…</option>
          {people.map((user) => (
            <option key={user} value={user}>
              {user}
            </option>
          ))}
        </select>
      </div>

      {isLoading ? <p className="empty-state">Loading…</p> : null}
      {error ? <p className="error-banner">Failed to load: {error}</p> : null}

      {data ? (
        <div className="people-sections">
          <section aria-label="Created by" className="people-section">
            <h3>Created by ({data.created_by.length})</h3>
            {data.created_by.length ? (
              data.created_by.map((item) => (
                <WorkItemCard item={item} key={item.id} onOpen={() => onOpenItem(item.id)} />
              ))
            ) : (
              <p className="board-column-empty">Nothing here</p>
            )}
          </section>
          <section aria-label="Mentioned" className="people-section">
            <h3>Mentioned ({data.mentioned.length})</h3>
            {data.account_id === null ? (
              <p className="board-column-empty">
                Couldn't resolve this person's account; mentions unavailable.
              </p>
            ) : data.mentioned.length ? (
              data.mentioned.map((item) => (
                <WorkItemCard item={item} key={item.id} onOpen={() => onOpenItem(item.id)} />
              ))
            ) : (
              <p className="board-column-empty">Nothing here</p>
            )}
          </section>
        </div>
      ) : null}
    </section>
  );
}
```
Add the `PeoplePage` import to `App.tsx`. (If importing `WorkItemCard` from `../App` into a component that `App.tsx` also imports creates an awkward cycle in tests, instead move `WorkItemCard` into its own `components/WorkItemCard.tsx` and import it from both — the implementer decides; keep the card's markup identical.)

**Step 5: CSS** (`styles.css`) — minimal:
```css
.people-sections {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
  margin-top: 14px;
}
.people-section h3 {
  margin: 0 0 8px;
}
```

**Step 6: Test** (`PeoplePage.test.tsx`). Mock `../api` (`fetchPeople` resolves `["a@x"]`; `fetchPersonWorkItems` resolves a `PersonWorkItems` with one created-by and one mentioned item). Assert:
- No `fetchPersonWorkItems` call before a person is selected.
- After selecting "a@x", the endpoint is called with "a@x" and both section headers show counts and the item titles render.
- When `account_id` is null, the mentions-unavailable note shows.
Follow the existing `ItemDetailModal.test.tsx` mocking style. If reusing `WorkItemCard` via `../App` pulls too much into the test, mock `../App`'s `WorkItemCard` or (preferred) test against the extracted `components/WorkItemCard`.

**Step 7: Run.** `cd apps/frontend && npm test` (PASS) and `npx tsc --noEmit` (clean).

**Step 8: Commit.**
```bash
git add apps/frontend/src
git commit -m "feat: People tab tracking created-by and mentioned tickets"
```

---

## Task 5: Docs — README

**Files:**
- Modify: `README.md`

**Step 1:** Document the People page: a third tab; on entry it lists the configured `[jira_people]`; selecting one lazily fetches (`GET /api/person-work-items?user=<email>`) that person's **created** (`reporter`) and **mentioned** tickets. Note:
- Mentioned is a **best-effort full-text proxy** (`text ~ <accountId>`), not exact @mention detection.
- The accountId is derived from the person's reporter history (no `[jira]` creds needed); a person who created nothing shows "mentions unavailable".
- Bounded by `jira_jql`; no date enrichment (list view). Reuses the same `[jira_people]` list as the board merge.
- Mention `GET /api/people` and `GET /api/person-work-items`.
- Add a "Current Behavior" bullet.

**Step 2: Commit.**
```bash
git add README.md
git commit -m "docs: People page (created-by + mentioned)"
```

---

## Final verification

- `cargo test -p quasar` — all pass.
- `cd apps/frontend && npm test && npx tsc --noEmit` — pass + clean.
- Manual smoke (live creds): `[jira_people] users=["khwu@quera.com"]`, `jira_jql="statusCategory != Done"`, `mise run dev`, open People tab → select the person → confirm Created-by and Mentioned sections populate and no fetch happened before selection.
