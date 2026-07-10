# Config-driven Jira "people" queries — Design

Date: 2026-07-10

## Motivation

Today the Jira read path fetches work items only for the project(s) listed in
`[jira_board]`. We want to also surface every ticket **related to specific
people** — assigned to them (`assignee`) or created by them (`reporter`) —
across *all* projects on the site, driven by config. This catches a person's
work that lives outside the configured boards.

## Decisions (from brainstorming)

- **Identify users by email** (e.g. `khwu@quera.com`). Verified live: email
  works on this site; accountId always works; display name silently returns 0.
- **Same site as `acli`, via `acli`.** `acli` is OAuth-authenticated to exactly
  one site and its `search` has no `--site` flag, so the person query runs
  against that site through the existing `acli` read path. No new HTTP code.
- **All projects on the site**, merged with board results, **de-duplicated by
  issue key**.
- **Users live in a new `[jira_people]` table.**
- **Volume** is bounded by the existing `jira_jql` filter (AND'd into the person
  query), e.g. `jira_jql = "statusCategory != Done"`. No dedicated cap config.

## Config shape

```toml
jira_base_url = "https://quera.atlassian.net"   # the JIRA domain (optional)

[jira_board]
projects = ["SSW", "ENG"]

[jira_people]
users = ["khwu@quera.com", "alice@quera.com"]
```

- `[jira_people].users` — emails. Validated non-empty and whitespace-free
  (new `ConfigError::InvalidJiraUser`). Absent/empty → no person query (current
  behavior unchanged).
- `jira_base_url` — top-level, optional, default `https://quera.atlassian.net`.
  Becomes the source for browse-link construction (de-hardcodes the
  `JIRA_BROWSE_BASE` constant in `adapters/jira.rs`).
- The write path's `[jira].base_url` stays independent (also defaults to quera).
  Documented: point both at the same site. Full unification is out of scope.

## Query composition (`config.rs::compose_jira_queries`)

Signature becomes `compose_jira_queries(projects, users, raw_jql) -> Vec<String>`.

- Board queries: unchanged (one `project = KEY` per project, each AND'd with the
  raw `jira_jql` where-clause, single trailing `ORDER BY`).
- If `users` is non-empty, append **one** person query spanning all projects:
  - user list → comma-joined quoted emails: `"a@x","b@x"`.
  - clause: `(assignee in (LIST) OR reporter in (LIST))`.
  - AND'd with the raw where-clause when present; same `ORDER BY` resolution
    (raw's own `ORDER BY`, else `ORDER BY updated DESC`).
- The person query is just another entry in `jira_queries`, so the API's
  existing per-query fan-out fetches and **streams it as its own chunk** — one
  `acli` search + its per-issue date enrichment, exactly like a project query.

Examples:

```
users = ["a@x"], projects = ["SSW"], no jira_jql
-> "project = SSW ORDER BY updated DESC"
-> "(assignee in (\"a@x\") OR reporter in (\"a@x\")) ORDER BY updated DESC"

users = ["a@x","b@x"], jira_jql = "statusCategory != Done", projects = ["SSW"]
-> "(project = SSW) AND (statusCategory != Done) ORDER BY updated DESC"
-> "((assignee in (\"a@x\",\"b@x\") OR reporter in (\"a@x\",\"b@x\"))) AND (statusCategory != Done) ORDER BY updated DESC"
```

## De-duplication (new)

A person's ticket in a configured board project matches **both** its project
query and the person query — same `jira:KEY` id. Dedupe by id:

- **Frontend** (`App.tsx` chunk merge): skip items whose id is already present,
  preventing duplicate cards and React key collisions as chunks stream in.
- **Backend batch path** (`resolve_work_items`): after the existing sort by id,
  drop adjacent duplicates (`dedup_by` on id). Covers summary/activity/cache.

## Browse links

Thread `jira_base_url` from config → `AppState` → the Jira adapter so
`normalize_issue` / `normalize_issue_detail` build `<domain>/browse/KEY` from
config instead of the hardcoded constant. Fixture loaders take the base too
(default quera keeps fixtures stable).

## Error handling / edge cases

- No `[jira_people]` or empty `users` → no person query; behavior unchanged.
- Person query volume can be large; bounded by `jira_jql`. Documented in README.
- A malformed/whitespace email → config load error (fail fast, like project keys).
- Person query failing (e.g. acli error) surfaces as a Jira warning chunk
  without sinking board results (existing per-query isolation).

## Testing

- **Rust (`config.rs`)**: `compose_jira_queries` with users only, users +
  projects, users + `jira_jql` (AND + ORDER BY handling), multi-email `in (...)`
  quoting; `[jira_people]` parsing; email validation rejects whitespace; loading
  `jira_base_url` (default + override).
- **Rust (`adapters/jira.rs`)**: browse URL built from a passed base_url;
  existing tests updated for the new signature (default quera).
- **Rust (`api.rs`)**: batch path dedupes duplicate ids.
- **Frontend (`App.tsx`)**: streamed chunks with an overlapping id render a
  single card.
- **README**: document `jira_base_url`, `[jira_people]`, the person query, dedup,
  and the `jira_jql` volume-bounding recommendation.

## Out of scope (YAGNI)

- Unifying `[jira].base_url` with `jira_base_url`.
- A dedicated per-person result cap / pagination limit.
- GitHub "involves user" queries (Jira only for now).
- Broader "related" senses (watcher, commented, mentioned) — only assignee +
  reporter.
