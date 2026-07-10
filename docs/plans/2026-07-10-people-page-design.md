# "People" page — created-by + mentioned, on-demand — Design

Date: 2026-07-10

## Motivation

A dedicated page to track, for a **specific** configured person, the Jira tickets
they **created** (reporter) or are **mentioned** in — fetched **only** when the
user opens the page and selects one of the preconfigured `[jira_people]`.

## Decisions (from brainstorming + live probes)

- **Created by** = `reporter = "<email>"` (reliable).
- **Mentioned** has no exact JQL; use the best-effort full-text proxy
  `text ~ "<accountId>"` (@mentions embed the accountId; 77 hits in probe vs 133
  for display-name which also matches prose). Best-effort, documented as fuzzy.
- **accountId is derived from the person's own `reporter` field** in the
  created-by results (one acli call, no `[jira]` REST creds). `acli` has no user
  command; this avoids adding a credential dependency to a read-only page.
- **Single-select**, one person at a time.
- **Additive**: the existing `[jira_people]` board-stream merge (assignee OR
  reporter) is unchanged; this page is separate.
- **Lazy**: nothing fetches until the page is opened AND a person selected.

## Backend

### Retain configured people

`compose_jira_queries` consumes `jira_users` today and only `jira_queries` is
kept. Add `pub jira_people: Vec<String>` (emails) to `RuntimeConfig`, thread it
through `main.rs` into a new `AppState.jira_people: Vec<String>`. Used to power
`/api/people` and to validate the per-person endpoint.

### `GET /api/people`

Returns `{ "users": ["a@x", ...] }` from `state.jira_people`. Cheap, no CLI.

### `GET /api/person-work-items?user=<email>`

Lazy, on-demand. Steps:

1. `400 BAD_REQUEST` unless `user` is in `state.jira_people` (only preconfigured
   people).
2. In fixture mode, return a small fixture or `409` (match existing write-path
   behavior for unsupported modes — reads can serve a fixture). Decision: serve
   an empty result in fixture mode (`created_by: [], mentioned: []`) so the UI
   renders without live Jira. (Simpler than a new fixture file.)
3. **Created-by query** (acli): compose
   `reporter = "<email>" [AND (jira_jql where-clause)] ORDER BY ...` reusing the
   same `jira_jql`/ORDER BY resolution as the board queries. Run via a new
   adapter fn that **searches + normalizes WITHOUT per-issue date enrichment**.
4. **Resolve accountId** from any created-by item's `reporter.accountId`
   (captured during normalization; see below). Also capture display name.
5. **Mentioned query** (acli), only if accountId resolved:
   `text ~ "<accountId>" [AND (jira_jql where-clause)] ORDER BY ...`.
6. Dedupe each list by key; remove from `mentioned` any key already in
   `created_by` (created-by takes priority).
7. Return `PersonWorkItems { user, account_id: Option<String>, created_by:
   Vec<WorkItem>, mentioned: Vec<WorkItem> }`.
8. Cache per user (`person:<email>`, short TTL via the existing `ResponseCache`).

### Adapter changes (`adapters/jira.rs`)

- Refactor: extract `search_work_items(runner, jql, base_url) -> Vec<WorkItem>`
  (search + `normalize_work_items`, **no** `enrich_planning_dates`).
  `load_work_items_with_runner` becomes `search_work_items` + enrichment, so the
  board path is unchanged.
- Add `fn person_account_id(...)`: not a separate call — the created-by search
  already returns `reporter`; expose the reporter accountId. Two options:
  (a) add a helper that parses the raw search JSON for the first
  `reporter.accountId`; or (b) have `search_work_items` optionally return the raw
  parsed reporter. Simplest: a small `fetch_account_id_via_reporter(runner,
  email, base_url) -> Option<(accountId, displayName)>` that runs
  `reporter = "<email>" --fields "key,reporter" --limit 1 --json` and parses
  `reporter.accountId`. This is a tiny dedicated call, independent of the list
  size, and keeps `search_work_items` returning plain `WorkItem`s.
  **Chosen:** the dedicated `--limit 1` resolver (clean, bounded).
- Add `pub fn set_/build person JQL` in `config.rs` or compose inline in the API
  layer (see below).

### JQL composition for the person queries

Add a `config.rs` helper (unit-testable, mirrors `compose_jira_queries`):
`compose_person_queries(email, account_id: Option<&str>, raw_jql) ->
{ created_by: String, mentioned: Option<String> }` producing:
- created_by: `reporter = "<email>" [AND (<where>)] <order>`
- mentioned: `text ~ "<account_id>" [AND (<where>)] <order>` (None if no id).
Emails/accountIds are already validated (config) / site-provided; still, the
email is a configured value (validated no whitespace/quote), and accountId comes
from Jira. Quote both in the JQL.

## Frontend

- Extend `view` to `"board" | "timeline" | "people"` and add a **People** tab
  button in the existing `view-tabs` group.
- New `PeoplePage` component:
  - On first render (page active), `GET /api/people` → single-select dropdown,
    default unselected ("Select a person…").
  - On selection, `GET /api/person-work-items?user=<email>` (AbortController;
    re-select cancels in-flight). Loading / error / empty states.
  - Renders two sections, **Created by (N)** and **Mentioned (N)**, each mapping
    `WorkItem[]` to the existing `WorkItemCard` (clicking opens the detail modal
    via the existing `selectedItemId` flow). A note under Mentioned when
    `account_id` is null: "Couldn't resolve this person's account; mentions
    unavailable."
  - Nothing fetches unless the People tab is active and a person is chosen.
- `api.ts`: `fetchPeople()` and `fetchPersonWorkItems(user, signal)`.
- `types.ts`: `PersonWorkItems { user; account_id: string | null; created_by:
  WorkItem[]; mentioned: WorkItem[] }`.

## Error handling / edge cases

- Unknown/again-unconfigured `user` → `400`.
- acli failure on either query → `502` with the error (page shows the error
  state); a failed mentioned query alone should not sink created-by (best-effort:
  mentioned errors degrade to empty + note).
- Person with no created tickets → accountId unresolved → mentioned skipped with
  the note; created-by empty.
- Volume: created-by can be large; bounded by `jira_jql`. No date enrichment
  keeps the fetch fast. Cards show "—" for dates (acceptable for a list view).

## Testing

- **Rust (`config.rs`)**: `compose_person_queries` — created-by only (no id),
  created-by + mentioned, with/without `jira_jql` (AND + ORDER BY), quoting.
  `jira_people` retained in `RuntimeConfig`.
- **Rust (`adapters/jira.rs`)**: `search_work_items` normalizes without
  enrichment (no `view` calls); `fetch_account_id_via_reporter` parses accountId;
  browse base honored.
- **Rust (`api.rs`)**: `/api/people` returns configured users; person endpoint
  rejects unconfigured user (400); dedup (a key in both lists appears only under
  created_by); mentioned skipped when accountId unresolved; per-user cache.
- **Frontend**: People tab renders; selecting a person calls the endpoint and
  renders the two sections; no fetch before selection; account-null note.
- **README**: document the People page, `/api/people` + `/api/person-work-items`,
  the mentioned proxy caveat, and that it reuses `[jira_people]` + `jira_jql`.

## Out of scope (YAGNI)

- Multi-select people / combined view.
- Exact @mention detection (impossible via JQL here).
- Editing from the People page (detail overlay already covers edits).
- Date enrichment / timeline for the People lists.
- A dedicated fixture file (fixture mode returns empty lists).
