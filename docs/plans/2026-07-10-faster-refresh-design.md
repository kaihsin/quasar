# Faster refresh — persistent Jira date cache + parallel fan-out — Design

Date: 2026-07-10

## Diagnosis

A cold refresh is dominated by **Jira planning-date enrichment**. `acli`'s bulk
`search` cannot return the Start/Target custom fields — verified live:
`--fields customfield_10022,customfield_10023` → *"fields … are not allowed"*
(and the `updated` system field is likewise rejected). So
`adapters/jira.rs::enrich_planning_dates` spawns **one `acli workitem view`
subprocess per issue** (~0.55s each, concurrency 8).

Observed open-issue counts on this config: SSW 10, **M4B1 299**, person 30 →
**~339 subprocess spawns** per cold refresh ≈ `339 × 0.55 ÷ 8 ≈ 23 s`. The three
Jira queries and five GitHub repos all resolve **sequentially** on one blocking
thread. GitHub calls are cheap (~0.4s each). M4B1 alone is ~90% of the time.

REST bulk dates are not viable here: classic `/rest/api/3/search` is removed,
the new `/search/jql` returned empty in probes, and the configured `[jira]`
token fails REST basic auth (401). The read path must stay `acli`-only.

## Decisions (brainstorm)

- **Persistent date cache + parallelize** the fan-out. **No cap** on enrichment.
- Forced adjustment: since neither `updated` nor custom fields come back from
  `search`, the date cache is **TTL-based keyed by issue key** (not version),
  with **invalidation on the app's own date writes** for correctness.

## Part 1 — Persistent Jira date cache (steady-state win)

- Add a second `ResponseCache` (the existing TTL string cache) to `AppState` as
  `date_cache: Arc<ResponseCache>`, built with a new config TTL
  `jira_date_cache_ttl_secs` (default **600**). Distinct from the 30s work-items
  cache.
- `enrich_planning_dates` gains `date_cache: &ResponseCache` + `now: Instant`.
  For each item, look up `jira-dates:<KEY>`:
  - **Hit** → parse `{start,target}` JSON, set on the item, **no `view` call**.
  - **Miss** → collect for fetching; run the per-issue `acli view` (concurrency
    bumped 8 → **12**) only for misses; on a successful result (including the
    empty/no-date case) set the item dates **and insert** the JSON into the
    cache. On a `view` failure, leave blank and do **not** cache (so it retries
    next time).
- Effect: first cold fetch still views the misses (parallelized); every refresh
  after the 30s work-items cache expires re-runs the cheap searches but hits the
  date cache for all unchanged issues → ~0 `view` calls.

### Self-edit correctness

When a Jira Start/Target date write succeeds (`api.rs::update_jira_field`, date
branch), invalidate `jira-dates:<KEY>` for that issue (alongside the existing
`work-items` invalidation). Self-edits reflect immediately; only *external* date
changes can be stale, up to the TTL.

Cache scope: the board path (`load_work_items_with_runner`) uses the date cache.
`search_work_items` (People page) stays enrichment-free and untouched.

## Part 2 — Parallel fan-out (cold-fetch win)

`resolve_work_items` today loops GitHub repos then Jira queries sequentially,
emitting a chunk per unit. Rework it to resolve all units **concurrently**:

- Enumerate units: one per GitHub repo (or the single GitHub fixture), one per
  Jira query (or the single Jira fixture).
- Run each unit on a scoped thread; each produces `(Vec<WorkItem>,
  Vec<SourceWarning>)` and sends it over an internal `std::sync::mpsc` channel.
  The coordinating thread receives results **as they complete**, calls `emit`
  (kept single-threaded, so the `StreamChunk` closure needn't be `Sync`), and
  accumulates into `data`/`warnings`. After all units join: sort by id, dedupe
  by id (existing behavior), cache, emit `Done`.
- `&AppState` fields used by units (`runner: Arc<dyn CommandRunner>`, repos,
  `github_project`, `jira_base_url`, `date_cache`) are all `Send + Sync`.
- Streaming already tolerates any chunk order; the cache-hit fast path is
  unchanged.

Concurrency envelope: ≤ (5 repos + 3 queries) unit threads; each Jira unit’s
enrichment fans out ≤ 12. All I/O-bound (subprocess + network); fine to
oversubscribe. With a warm date cache, enrichment issues ~0 subprocesses.

**Net:** cold first fetch ~23s → ~4–6s; subsequent refreshes → ~1–2s.

## Config

New `jira_date_cache_ttl_secs: u64` (default 600) on `RuntimeConfig` +
`FileConfig`, wired through `main.rs` into `AppState`. Documented in README.

## Error handling / edge cases

- `view` failure for an issue → blank dates, not cached (retried next refresh).
- Fixture mode: unchanged (fixtures don't enrich via `view`).
- A unit thread failing surfaces as that source's warning chunk (unchanged
  semantics), isolated from other units.
- Date cache is process-memory only (like the work-items cache); lost on
  restart — acceptable.

## Testing

- **config**: `jira_date_cache_ttl_secs` default (600) + override.
- **adapters/jira**: enrichment **skips the `view` call on a date-cache hit**
  (assert runner call count) and **populates** the cache on a miss; a cached
  empty result still skips future views.
- **api**: a successful Jira date write **invalidates** `jira-dates:<KEY>`;
  `resolve_work_items` still merges + dedupes correctly with the parallel
  coordinator (existing tests must stay green; the per-project streaming test
  still sees one chunk per query, order-independent).
- **README**: document the date cache + TTL and the parallel fan-out.

## Out of scope (YAGNI)

- REST bulk date fetching (not viable here).
- Enrichment cap / pagination limit (chosen: no cap).
- Disk-persistent cache across restarts.
- GitHub-side caching (its per-repo graphql is one call; parallelization covers it).
- Deferred/lazy date loading (a larger streaming/timeline change).
