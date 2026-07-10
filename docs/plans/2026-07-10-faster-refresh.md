# Faster Refresh Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Cut cold refresh from ~23s to ~4-6s and warm refresh to ~1-2s by caching Jira planning dates (TTL, per issue key) and resolving all source queries concurrently.

**Architecture:** Add a second long-TTL `ResponseCache` (`date_cache`) to `AppState`; the Jira board enrichment path only runs the per-issue `acli workitem view` for date-cache misses and populates it on fetch. The app's own Jira date writes invalidate the affected entry. `resolve_work_items` fans out all GitHub repos + Jira queries on scoped threads, streaming each chunk as it completes.

**Tech Stack:** Rust, axum, serde, `acli`; the existing `ResponseCache` (TTL string cache).

**Design doc:** `docs/plans/2026-07-10-faster-refresh-design.md`

**Conventions:** `cargo test -p quasar`. Commit per task. No `Co-authored-by` trailers. (No frontend changes in this feature.)

---

## Task 1: Config + AppState — plumb the date cache

**Files:**
- Modify: `crates/quasar/src/config.rs`
- Modify: `crates/quasar/src/api.rs`
- Modify: `crates/quasar/src/main.rs`

**Step 1: Failing config tests** in `config.rs` tests module:
```rust
#[test]
fn jira_date_cache_ttl_defaults_to_600() {
    let home_dir = TestDir::new();
    let config_path = write_config(home_dir.path(), "github_repos = []\n");
    let config =
        load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
    assert_eq!(config.jira_date_cache_ttl_secs, 600);
}

#[test]
fn jira_date_cache_ttl_is_overridable() {
    let home_dir = TestDir::new();
    let config_path = write_config(home_dir.path(), "jira_date_cache_ttl_secs = 120\n");
    let config =
        load_runtime_config(&config_path, EnvOverrides::default()).expect("config should load");
    assert_eq!(config.jira_date_cache_ttl_secs, 120);
}
```
Run `cargo test -p quasar --lib config` — expect failure.

**Step 2: config.rs.** Add to `struct RuntimeConfig` (after `jira_jql`):
```rust
    /// TTL (seconds) for the per-issue Jira planning-date cache. Dates change
    /// rarely; a longer TTL avoids re-running `acli workitem view` every refresh.
    pub jira_date_cache_ttl_secs: u64,
```
Add to `struct FileConfig`: `jira_date_cache_ttl_secs: Option<u64>,`. In `load_runtime_config`, add:
```rust
    let jira_date_cache_ttl_secs = file_config.jira_date_cache_ttl_secs.unwrap_or(600);
```
and set `jira_date_cache_ttl_secs,` in the returned `RuntimeConfig { ... }`. Add the field (value `600`) to the two full-literal `RuntimeConfig` tests (`loads_multiple_github_repos_from_toml`, `missing_file_uses_defaults`).

**Step 3: AppState.** In `api.rs`, add `pub date_cache: Arc<ResponseCache>,` to `struct AppState`. Add a `jira_date_cache_ttl_secs: u64` parameter to `AppState::new` (place it right AFTER `cache_ttl_secs: u64,`), and in the constructed `Self { ... }` build:
```rust
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(
                jira_date_cache_ttl_secs,
            ))),
```

**Step 4: main.rs.** In `build_app_state`, pass `config.jira_date_cache_ttl_secs,` to BOTH `AppState::new(...)` calls, right after `config.cache_ttl_secs,`. Add `jira_date_cache_ttl_secs: 600,` to the `RuntimeConfig` literal in the `startup_resolver_reads_user_config_file` test.

**Step 5: api.rs test constructions.** Every `AppState { ... }` struct literal in the tests module (helpers `app_state`, `jira_cli_state`, `jira_write_state`, `jira_cli_state_people`, and inline literals) needs a `date_cache` field. Add:
```rust
            date_cache: Arc::new(ResponseCache::new(std::time::Duration::from_secs(600))),
```
(They already import `ResponseCache`; `Arc` is in scope.)

**Step 6: Run.** `cargo test -p quasar` — expect PASS (date_cache is unused for now; that's fine).

**Step 7: Commit.**
```bash
git add crates/quasar/src/config.rs crates/quasar/src/api.rs crates/quasar/src/main.rs
git commit -m "feat: add jira date_cache to config and AppState"
```

---

## Task 2: Jira enrichment reads/writes the date cache

**Files:**
- Modify: `crates/quasar/src/adapters/jira.rs`
- Modify: `crates/quasar/src/api.rs` (the board call site)

**Step 1: Failing adapter tests** in `jira.rs` tests module. Add `use crate::cache::ResponseCache;` and `use std::time::{Duration, Instant};` inside the tests module if not present.
```rust
#[test]
fn enrichment_skips_view_on_date_cache_hit() {
    let payload = std::fs::read_to_string(fixture_path()).expect("fixture");
    let runner = MockCommandRunner::success(&payload);
    let cache = ResponseCache::new(Duration::from_secs(600));
    let now = Instant::now();
    // Pre-seed the fixture item's dates (fixture key is ABC-42).
    cache.insert(
        "jira-dates:ABC-42",
        r#"{"start":"2026-06-01","target":"2026-06-15"}"#.to_string(),
        now,
    );
    let items = super::load_work_items_with_runner(
        &runner, "order by updated desc", TEST_BASE, &cache, now,
    )
    .expect("load");
    assert_eq!(items[0].start_date, "2026-06-01");
    assert_eq!(items[0].target_date, "2026-06-15");
    // Only the search call — the per-issue `view` was skipped by the cache hit.
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
}

#[test]
fn enrichment_populates_date_cache_on_miss() {
    // Routing runner: search returns the fixture; `view` returns dates.
    struct RoutingRunner {
        issues: String,
    }
    impl CommandRunner for RoutingRunner {
        fn run(&self, _p: &str, args: &[&str]) -> CommandResult<String> {
            if args.contains(&"view") {
                Ok(r#"{"key":"ABC-42","fields":{"customfield_10022":"2026-07-01","customfield_10023":"2026-07-20"}}"#.to_string())
            } else {
                Ok(self.issues.clone())
            }
        }
    }
    let runner = RoutingRunner {
        issues: std::fs::read_to_string(fixture_path()).expect("fixture"),
    };
    let cache = ResponseCache::new(Duration::from_secs(600));
    let now = Instant::now();
    let items = super::load_work_items_with_runner(
        &runner, "order by updated desc", TEST_BASE, &cache, now,
    )
    .expect("load");
    assert_eq!(items[0].start_date, "2026-07-01");
    // The miss populated the cache.
    assert_eq!(
        cache.get("jira-dates:ABC-42", now),
        crate::cache::CacheOutcome::Hit(
            r#"{"start":"2026-07-01","target":"2026-07-20"}"#.to_string()
        )
    );
}
```
(Confirm the Jira fixture's key is `ABC-42` by checking `tests/fixtures/jira/issues.json`; adjust the key in the tests if different.) Run `cargo test -p quasar --lib adapters::jira` — expect failure (arity).

**Step 2: Add the cached-dates struct** near the top of `jira.rs`:
```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedDates {
    start: String,
    target: String,
}
```

**Step 3: Change signatures.** `load_work_items_with_runner` and `enrich_planning_dates` take `date_cache: &ResponseCache, now: Instant`:
```rust
pub fn load_work_items_with_runner(
    runner: &dyn CommandRunner,
    jql: &str,
    base_url: &str,
    date_cache: &ResponseCache,
    now: Instant,
) -> AdapterResult<Vec<WorkItem>> {
    let mut items = search_work_items(runner, jql, base_url)?;
    enrich_planning_dates(runner, &mut items, date_cache, now);
    Ok(items)
}
```
Add imports at the top of `jira.rs`: `use std::time::Instant;` and extend the cache import: `use crate::cache::{CacheOutcome, ResponseCache};`.

**Step 4: Rewrite `enrich_planning_dates`** to consult the cache and only `view` misses:
```rust
fn enrich_planning_dates(
    runner: &dyn CommandRunner,
    items: &mut [WorkItem],
    date_cache: &ResponseCache,
    now: Instant,
) {
    // Apply cache hits in place; collect the indices that still need a `view`.
    let mut to_fetch: Vec<usize> = Vec::new();
    for (idx, item) in items.iter_mut().enumerate() {
        let cache_key = format!("jira-dates:{}", item.external_id);
        match date_cache.get(&cache_key, now) {
            CacheOutcome::Hit(payload) => match serde_json::from_str::<CachedDates>(&payload) {
                Ok(dates) => {
                    item.start_date = dates.start;
                    item.target_date = dates.target;
                }
                Err(_) => to_fetch.push(idx),
            },
            CacheOutcome::Miss => to_fetch.push(idx),
        }
    }
    if to_fetch.is_empty() {
        return;
    }

    let keys: Vec<(usize, String)> = to_fetch
        .iter()
        .map(|&idx| (idx, items[idx].external_id.clone()))
        .collect();
    let next = AtomicUsize::new(0);
    let collected: Mutex<Vec<(usize, String, String)>> = Mutex::new(Vec::new());
    let workers = keys.len().min(ENRICH_CONCURRENCY).max(1);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some((item_idx, key)) = keys.get(index) else {
                    break;
                };
                if let Some((start, target)) = fetch_issue_dates(runner, key) {
                    collected
                        .lock()
                        .expect("enrich collected mutex should not be poisoned")
                        .push((*item_idx, start, target));
                }
            });
        }
    });

    for (idx, start, target) in collected
        .into_inner()
        .expect("enrich collected mutex should not be poisoned")
    {
        // Cache the result (including the empty/no-date case) so future refreshes
        // skip the `view` for this issue until the TTL elapses.
        if let Ok(payload) = serde_json::to_string(&CachedDates {
            start: start.clone(),
            target: target.clone(),
        }) {
            date_cache.insert(&format!("jira-dates:{}", items[idx].external_id), payload, now);
        }
        items[idx].start_date = start;
        items[idx].target_date = target;
    }
}
```

**Step 5: Bump concurrency.** Change `const ENRICH_CONCURRENCY: usize = 8;` to `= 12;` (update the doc comment if it mentions 8).

**Step 6: Update the api.rs board call site.** In `resolve_work_items`, the `JiraSource::Cli` branch calls `load_work_items_with_runner(state.runner.as_ref(), jql, &state.jira_base_url)`. Add the cache + now: `..., state.date_cache.as_ref(), now)`. (`now` is the `Instant::now()` already bound at the top of `resolve_work_items`.)

**Step 7: Update existing jira.rs adapter tests** that call `load_work_items_with_runner` (e.g. `jira_runner_invokes_expected_cli_arguments`, `jira_enrichment_populates_planning_dates`). Add a fresh cache + now arg: pass `&ResponseCache::new(Duration::from_secs(600)), Instant::now()`. With an empty cache these behave exactly as before (all items are misses → same `view` calls), so their existing assertions (e.g. `calls.len() == 2`) still hold.

**Step 8: Run.** `cargo test -p quasar` — expect PASS.

**Step 9: Commit.**
```bash
git add crates/quasar/src/adapters/jira.rs crates/quasar/src/api.rs
git commit -m "feat: cache Jira planning dates to skip per-issue enrichment"
```

---

## Task 3: Invalidate the date cache on Jira date writes

**Files:**
- Modify: `crates/quasar/src/api.rs`

**Step 1: Failing test** in the `api.rs` tests module (reuse `jira_write_state` + `JiraWriteRunner`, which perform a Jira date write end-to-end):
```rust
#[test]
fn jira_date_write_invalidates_date_cache_entry() {
    let runner = Arc::new(JiraWriteRunner { calls: Mutex::new(Vec::new()) });
    let state = jira_write_state(runner, Some(test_jira_config()));
    let now = Instant::now();
    state
        .date_cache
        .insert("jira-dates:SSW-1", r#"{"start":"x","target":"y"}"#.to_string(), now);

    fetch_work_item_field(
        &state,
        &UpdateFieldRequest {
            id: "jira:SSW-1".to_string(),
            field: "target".to_string(),
            value: Some("2026-07-20".to_string()),
        },
    )
    .expect("date write should succeed");

    assert_eq!(
        state.date_cache.get("jira-dates:SSW-1", now),
        CacheOutcome::Miss,
        "a date write must invalidate the cached dates for that issue"
    );
}
```
Run `cargo test -p quasar jira_date_write_invalidates` — expect FAIL.

**Step 2: Implement.** In `update_jira_field`, in the branch that handles the `start`/`target` date write, after the write succeeds and the existing `state.cache.invalidate("work-items")` runs, also invalidate the per-issue date entry:
```rust
    if matches!(field, "start" | "target") {
        state.date_cache.invalidate(&format!("jira-dates:{key}"));
    }
```
Place this right after the `state.cache.invalidate("work-items");` in `update_jira_field` (so it runs on any successful Jira field write; gating on the date fields avoids needless work for status writes). Ensure `CacheOutcome` is imported in the tests (it is, via the module import).

**Step 3: Run.** `cargo test -p quasar` — expect PASS.

**Step 4: Commit.**
```bash
git add crates/quasar/src/api.rs
git commit -m "feat: invalidate cached Jira dates on date write"
```

---

## Task 4: Parallelize the resolve_work_items fan-out

**Files:**
- Modify: `crates/quasar/src/api.rs`

**Step 1: Reread `resolve_work_items`** in full. It: (1) returns early on a work-items cache hit; (2) builds `data`/`warnings` by looping GitHub repos then Jira queries **sequentially**, emitting a chunk per unit; (3) sorts by id, dedupes, caches, emits `Done`. This task keeps (1) and (3) and parallelizes (2).

**Step 2: Rewrite the middle section.** Replace the sequential `emit_items`/`emit_warning` blocks (from after the cache-hit early return, up to `data.sort_by(...)`) with a unit-based parallel coordinator. Keep the two local helper fns only if still used; otherwise remove them.
```rust
    // Each unit resolves one source (a GitHub repo, the GitHub fixture, a Jira
    // query, or the Jira fixture) and yields its items + warnings. Units run on
    // scoped threads and stream back over a channel as they finish, so a slow
    // source (e.g. a large Jira project's date enrichment) no longer blocks the
    // others. `emit` stays on this thread (needn't be Sync).
    type Unit<'a> = Box<dyn FnOnce() -> (Vec<WorkItem>, Vec<SourceWarning>) + Send + 'a>;
    let mut units: Vec<Unit> = Vec::new();

    match &state.github_source {
        GitHubSource::Fixture(path) => units.push(Box::new(move || {
            match adapters::github::load_fixture_work_items(path) {
                Ok(items) => (items, Vec::new()),
                Err(error) => (
                    Vec::new(),
                    vec![SourceWarning { source: WorkSource::GitHub, message: error.to_string() }],
                ),
            }
        })),
        GitHubSource::Cli => {
            if state.github_repos.is_empty() {
                units.push(Box::new(|| {
                    (
                        Vec::new(),
                        vec![SourceWarning {
                            source: WorkSource::GitHub,
                            message: "No GitHub repos configured for CLI mode".to_string(),
                        }],
                    )
                }));
            } else {
                for repo in &state.github_repos {
                    units.push(Box::new(move || {
                        match adapters::github::load_work_items_with_runner(
                            state.runner.as_ref(),
                            repo,
                            state.github_project.as_ref(),
                        ) {
                            Ok(items) => (items, Vec::new()),
                            Err(error) => (
                                Vec::new(),
                                vec![SourceWarning {
                                    source: WorkSource::GitHub,
                                    message: format!("GitHub repo {repo} failed: {error}"),
                                }],
                            ),
                        }
                    }));
                }
            }
        }
    }

    match &state.jira_source {
        JiraSource::Fixture(path) => units.push(Box::new(move || {
            match adapters::jira::load_fixture_work_items(path, &state.jira_base_url) {
                Ok(items) => (items, Vec::new()),
                Err(error) => (
                    Vec::new(),
                    vec![SourceWarning { source: WorkSource::Jira, message: error.to_string() }],
                ),
            }
        })),
        JiraSource::Cli => {
            for jql in &state.jira_queries {
                units.push(Box::new(move || {
                    match adapters::jira::load_work_items_with_runner(
                        state.runner.as_ref(),
                        jql,
                        &state.jira_base_url,
                        state.date_cache.as_ref(),
                        now,
                    ) {
                        Ok(items) => (items, Vec::new()),
                        Err(error) => (
                            Vec::new(),
                            vec![SourceWarning { source: WorkSource::Jira, message: error.to_string() }],
                        ),
                    }
                }));
            }
        }
    }

    let mut data: Vec<WorkItem> = Vec::new();
    let mut warnings: Vec<SourceWarning> = Vec::new();
    let (tx, rx) = std::sync::mpsc::channel::<(Vec<WorkItem>, Vec<SourceWarning>)>();
    std::thread::scope(|scope| {
        for unit in units {
            let tx = tx.clone();
            scope.spawn(move || {
                let _ = tx.send(unit());
            });
        }
        drop(tx);
        // Receive results as each unit completes and emit its chunk immediately.
        for (items, warns) in &rx {
            emit(StreamChunk::Items { data: &items, warnings: &warns });
            data.extend(items);
            warnings.extend(warns);
        }
    });
```
Leave the existing tail (`data.sort_by(...)`, `data.dedup_by(...)`, response build, `state.cache.insert(...)`, `emit(StreamChunk::Done { ... })`, `response`) exactly as-is.

**Step 3: Clean up.** Remove the now-unused `emit_items`/`emit_warning` inner fns if nothing references them. Confirm `WorkSource`, `SourceWarning`, `WorkItem` are imported (they are).

**Step 4: Run.** `cargo test -p quasar` — expect PASS. The existing streaming/partial-failure/dedup tests must stay green: `resolve_work_items_streams_one_chunk_per_jira_project`, `fetch_work_items_keeps_successful_github_repo_items_when_one_repo_fails`, `fetch_work_items_keeps_successful_jira_project_items_when_one_query_fails`, `resolve_work_items_dedupes_items_with_same_id`, `work_items_stream_emits_a_chunk_per_source_then_done`. These assert per-chunk contents/counts, which are order-independent, so parallel completion order is fine.

**Step 5: Commit.**
```bash
git add crates/quasar/src/api.rs
git commit -m "perf: resolve GitHub and Jira sources concurrently"
```

---

## Task 5: Docs — README

**Files:**
- Modify: `README.md`

**Step 1:** In the Jira Data Fetching / configuration sections, document:
- Planning dates are fetched per issue via `acli workitem view` (bulk `search` can't return them), and are now **cached** with TTL `jira_date_cache_ttl_secs` (default 600s) keyed by issue key, so only the first fetch (or changed/expired issues) pays that cost. Editing a Jira date in the app invalidates that issue's cached dates immediately.
- Sources (GitHub repos + Jira queries) are now fetched **concurrently**, streaming each as it resolves.
- Add `jira_date_cache_ttl_secs` to the example `config.toml` (commented, with the default) and a "Current Behavior" bullet.

**Step 2: Commit.**
```bash
git add README.md
git commit -m "docs: jira date cache TTL and concurrent source fetching"
```

---

## Final verification

- `cargo test -p quasar` — all pass.
- Manual smoke (live creds): `mise run dev`; first load warms the caches; a second **Refresh** after >30s (work-items cache expired, date cache warm) should be visibly faster and issue few/no `acli workitem view` calls. Editing a Jira Start/Target date still reflects immediately. Use `/run`.
