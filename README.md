# GitHub and Jira Unified Dashboard

This repository contains a local dashboard that combines GitHub issues from `gh`
and Jira tickets from `acli` into one unified view. Planning dates and status are
editable inline for both sources — GitHub via `gh api graphql` (Projects v2) and
Jira via the REST API (`curl`), the latter requiring a `[jira]` credentials block
(see [Editing dates and status](#editing-dates-and-status)).

## Workspace Layout

- `crates/quasar`: Rust backend for CLI integration, config loading,
  normalized APIs, and tests
- `apps/frontend`: React frontend for dashboard visualizations and filters
- `docs/plans`: design and implementation planning documents

## Getting Started

A first-time, end-to-end setup. Commands use Homebrew (macOS); on Linux use your
package manager or the upstream installers linked below.

### 1. Install prerequisites

| Tool | Why it's needed | Install (macOS / Homebrew) |
|------|-----------------|----------------------------|
| **Rust toolchain** (`cargo`, `rustc`) | Builds and runs the backend | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` (via [rustup](https://rustup.rs)) |
| **Node.js + npm** | Builds and serves the frontend | `brew install node` |
| **`gh`** (GitHub CLI) | Fetches GitHub issues and edits Projects v2 fields | `brew install gh` |
| **`acli`** (Atlassian CLI) | Fetches Jira work items | `brew install atlassian/acli/acli` |
| **`mise`** (optional) | Task runner for `mise run dev` | `brew install mise` |

Verify:

```bash
cargo --version && node --version && gh --version && acli --version
```

### 2. Authenticate the CLIs

**GitHub** — log in, then ensure the token carries the Projects v2 scope. This is
required for date/Status enrichment and inline editing; without it those queries
fail with `INSUFFICIENT_SCOPES` and dates silently render blank:

```bash
gh auth login                 # if not already logged in
gh auth refresh -s project    # add the Projects v2 (read+write) scope
gh auth status                # confirm scopes include 'project'
```

**Jira** — `acli` handles its own auth (OAuth or API token) for *reading*:

```bash
acli jira auth login          # OAuth or API token
acli jira auth status         # confirm the site + account
```

> Editing Jira dates/status is separate: it uses the Jira REST API with the
> `[jira]` email/token block in your config (see
> [Editing dates and status](#editing-dates-and-status)). Reading works with
> `acli` auth alone.

### 3. Create your config file

The backend reads `~/.config/quasar/config.toml`. Create it and lock it down
(it may hold a Jira API token):

```bash
mkdir -p ~/.config/quasar
$EDITOR ~/.config/quasar/config.toml
chmod 600 ~/.config/quasar/config.toml
```

A minimal starting point:

```toml
github_repos = ["your-org/your-repo"]

[jira_board]
projects = ["ENG"]              # your Jira project key(s)
```

See [Backend Configuration](#backend-configuration) for every option, including
the optional `[github_project]` (date/status enrichment + editing) and `[jira]`
(Jira write credentials) blocks.

### 4. Install frontend dependencies

```bash
cd apps/frontend
npm install
```

### 5. Run it

From the repo root, with `mise`:

```bash
mise run dev            # backend (:3000) + frontend (:5173)
```

…or without `mise`, in two terminals (see [Launch Locally](#launch-locally)).
Then open <http://localhost:5173>. To try it with no live credentials, use
fixture mode: `mise run dev-fixtures`.

## Backend Configuration

The backend reads its user-local config from:

```text
~/.config/quasar/config.toml
```

If the file is missing, the service falls back to built-in defaults.

Example config:

```toml
bind_addr = "127.0.0.1:3000"
cache_ttl_secs = 30
mode = "cli"
github_repos = [
  "openai/quasar",
  "rust-lang/rust",
  "tokio-rs/tokio",
]
# Optional: the Jira site domain used to build work-item browse links.
# Defaults to https://quera.atlassian.net. Set it to match the site `acli`
# is authenticated to (and the write-path [jira].base_url below).
jira_base_url = "https://quera.atlassian.net"

# Which Jira project(s) to pull work items from. Keys compose a
# `project in (...)` clause; ordering defaults to `ORDER BY updated DESC`.
[jira_board]
projects = ["ENG"]

# Optional: pull every ticket assigned to OR created by these people, across
# all projects on the site (not just [jira_board]). Emails must be non-empty
# and contain no whitespace or double-quote characters.
[jira_people]
users = ["alice@example.com", "bob@example.com"]

# Optional: an extra raw JQL filter AND'd with the [jira_board] selection above.
# Its own `ORDER BY`, if present, becomes the query's ordering. Omit [jira_board]
# and set only this to run a fully hand-written query verbatim (escape hatch).
# jira_jql = "statusCategory != Done"

# Optional: GitHub Projects v2 board for date/status enrichment + editing.
[github_project]
owner = "your-org"
number = 18

# Optional: Jira REST credentials. Required only to edit Jira dates/status;
# omit it and Jira fields stay read-only. Token is stored in plaintext, so
# keep the file owner-only (chmod 600).
[jira]
email = "you@example.com"
token = "<atlassian-api-token>"   # id.atlassian.com -> Security -> API tokens
# base_url = "https://your-site.atlassian.net"   # optional, set to your site
```

Config resolution order is:

1. Environment variable overrides
2. `~/.config/quasar/config.toml`
3. Built-in defaults

Environment overrides still work for one-off runs and local debugging:

- `QUASAR_BIND` default: `127.0.0.1:3000`
- `QUASAR_CACHE_TTL_SECS` default: `30`
- `QUASAR_MODE` values: `cli` or `fixtures`
- `QUASAR_GITHUB_REPO` example: `openai/quasar`
  This override forces a single GitHub repo, even if `github_repos` contains
  multiple entries in the config file.
- `QUASAR_JIRA_JQL` default: `ORDER BY updated DESC`. Sets the optional raw JQL
  filter (the one AND'd with `[jira_board]`), overriding any `jira_jql` in the
  file. Project selection via `[jira_board]` has no environment override.

## GitHub Data Fetching

The backend fetches GitHub data by shelling out to the `gh` CLI in two steps:

1. **Issues** — `gh issue list -R <repo>` pulls open issues for each slug in
   `github_repos`.
2. **Planning dates** — if a `[github_project]` table is configured, a
   `gh api graphql` (Projects v2) query enriches each issue with Start and
   Target dates from a project board.

Two separate config pieces identify what gets fetched:

- **`github_repos`** — a list of `owner/repo` slugs. This is what actually
  scopes the query; issues are read per-repo, and the owner/name come from
  splitting each slug.
- **`[github_project]`** — the board is selected purely by its numeric
  `number`. There is **no project name field** — a board like
  "Scientific Software Dev" is identified only by the number in its URL
  (e.g. `github.com/orgs/your-org/projects/18` → `number = 18`).

Example:

```toml
github_repos = ["your-org/quasar"]

[github_project]
owner = "your-org"
number = 18
# optional, defaults shown:
start_date_field = "Start date"
target_date_field = "Target date"
```

Notes:

- The query is **repository-scoped**. It walks each configured repo's issues
  and matches their `projectItems` against `number`. Issues on the board whose
  repo is not listed in `github_repos` are not fetched. There is no org-level
  "list everything on project N" lookup.
- The `owner` field in `[github_project]` is required by the config parser but
  is not currently read at runtime — the effective owner/repo come from the
  `github_repos` slugs.
- The same single `[github_project]` is applied to all configured repos.

## Jira Data Fetching

Jira work items come from `acli jira workitem search --jql <query>`. Fetching is
**per project** (mirroring the per-repo GitHub fan-out): the backend runs one
`acli` query per configured project and streams each project's results as they
resolve, so cards appear progressively and one project failing surfaces a
warning without sinking the others. The query set is composed from two config
pieces:

- **`[jira_board]`** — `projects = ["SSW", "ENG"]` selects which Jira project(s)
  to pull from. Each key becomes its **own** `project = KEY` query (a **union**
  across projects, fetched independently). This is the structured analog of
  `github_repos`.
- **`jira_jql`** (optional) — a raw JQL filter **AND'd** into *each* project's
  query, so it narrows the results (e.g. exclude Done). A single trailing
  `ORDER BY` is applied: the raw clause's own `ORDER BY` if it has one, otherwise
  the default `ORDER BY updated DESC`.
- **`[jira_people]`** (optional) — `users = ["email", ...]` pulls every ticket
  **assigned to** OR **created by** the listed people across **all** projects on
  the site (not just those in `[jira_board]`). Mechanically it appends **one**
  extra `acli` query, `(assignee in (...) OR reporter in (...))`, to the
  per-project fan-out, streamed as its own chunk. Emails are validated
  (non-empty, no whitespace, no double-quote). Its results are merged with the
  board results and **de-duplicated by issue key**, so a ticket that is both in
  a configured project and matches a person appears **once**. `jira_jql` is
  AND'd into the person query too, so set e.g. `jira_jql = "statusCategory != Done"`
  to bound it — "all tickets related to a person" can be large (a prolific
  reporter can have hundreds), and each fetched item still costs a per-issue
  `view` call for planning-date enrichment, so a large person set slows refresh.

The browse link on each Jira card (and the `↗` original link) is built from
**`jira_base_url`** (top-level, optional, default `https://quera.atlassian.net`).
Set it to match the site `acli` is authenticated to; it should also match the
write-path `[jira].base_url`, which remains a separate key with the same default.

Composition examples (each line is a separate `acli` query):

```toml
# one project -> one query
[jira_board]
projects = ["SSW"]
# -> project = SSW ORDER BY updated DESC

# multiple projects -> one query each (streamed independently)
[jira_board]
projects = ["SSW", "ENG"]
# -> project = SSW ORDER BY updated DESC
# -> project = ENG ORDER BY updated DESC

# each project's query AND'd with an extra filter
jira_jql = "statusCategory != Done"
[jira_board]
projects = ["SSW", "ENG"]
# -> (project = SSW) AND (statusCategory != Done) ORDER BY updated DESC
# -> (project = ENG) AND (statusCategory != Done) ORDER BY updated DESC

# escape hatch: no [jira_board], raw JQL is the sole query, verbatim
jira_jql = "project = SSW AND statusCategory != Done ORDER BY updated DESC"

# [jira_people] -> one extra cross-project query, merged + deduped by key
[jira_board]
projects = ["SSW", "ENG"]
[jira_people]
users = ["alice@example.com", "bob@example.com"]
# -> project = SSW ORDER BY updated DESC
# -> project = ENG ORDER BY updated DESC
# -> (assignee in ("alice@example.com","bob@example.com") OR reporter in ("alice@example.com","bob@example.com")) ORDER BY updated DESC

# jira_jql bounds the person query too (AND'd in)
jira_jql = "statusCategory != Done"
[jira_people]
users = ["alice@example.com"]
# -> ((assignee in ("alice@example.com") OR reporter in ("alice@example.com")) AND (statusCategory != Done)) ORDER BY updated DESC
```

To combine boards with a filter, `[jira_board]` selects the project(s) (union)
and `jira_jql` narrows them (AND). For anything the two can't express together,
omit `[jira_board]` and write the whole query in `jira_jql`.

## Item Detail Overlay

Clicking a work-item card's title opens an overlay with the full issue/ticket
body (rendered Markdown), the comment thread, and a metadata sidebar (status,
assignees, author, labels, priority, dates, repo/project, and a link to the
original). A work item carries a **list** of assignees — GitHub issues can have
several, Jira has 0 or 1 — rendered as stacked avatars on cards and in the
timeline. Detail is fetched lazily only when an item is opened, via
`GET /api/work-item-detail?id=<work-item-id>`, and is not cached — each open
fetches fresh from `gh issue view` / `acli jira workitem view`. The `↗` link on
each card still opens the original issue/ticket in a new tab.

### Editing dates and status

GitHub work-item Start/Target dates and the Projects v2 **Status** (the board
single-select field, distinct from the issue open/closed state) are editable
inline from the detail overlay. Opening an item enriches the detail with the
item's current dates, Status, and the available Status options via one
`gh api graphql` query, so the date inputs and Status dropdown open pre-filled.
Edits issue `PATCH /api/work-item-field`
(`{ id, field: "start" | "target" | "status", value }`), which resolves the
project/field/(option)/item, adds the issue to the configured board if needed,
runs an `updateProjectV2ItemFieldValue` (or clear) mutation, and invalidates the
work-items cache. Requires a `gh` token with `project` write scope and a
`[github_project]` configured (optional `status_field`, default `"Status"`).

Jira **Target start**/**Target end** dates and workflow **Status** are also
editable inline when a `[jira]` credentials block is configured. The installed
`acli` (1.3.22) cannot set custom fields, so Jira writes go through the REST API
via `curl`: dates are set with `PUT /rest/api/3/issue/<key>`
(`customfield_10022`/`10023`), and status changes look up the matching workflow
transition (`GET /rest/api/3/issue/<key>/transitions`) and apply it
(`POST .../transitions`). Opening a Jira item enriches its detail with the
reachable transition targets so the Status dropdown is pre-filled; because Jira
status is workflow-driven it offers no blank/clear option. The same
`PATCH /api/work-item-field` endpoint handles both sources, keyed off the
`github:`/`jira:` id prefix.

```toml
[jira]
email = "you@example.com"
token = "<atlassian-api-token>"    # id.atlassian.com → Security → API tokens
# optional, set to your Atlassian site:
base_url = "https://your-site.atlassian.net"
```

The token is stored in plaintext in `config.toml`, so keep the file readable
only by you (`chmod 600`). Without a `[jira]` block, Jira fields stay read-only
and edit attempts return `409`.

Assignees are also editable inline. The list of assignable candidates is fetched
lazily when an item is opened (GitHub `gh api repos/<repo>/assignees`; Jira
`GET /rest/api/3/user/assignable/search?issueKey=<key>`) on a best-effort basis
— if it can't be fetched, the assignee field renders read-only. Editing behaves
per source:

- **GitHub** — a checkbox list of the repo's assignable users; you can assign
  several. Writes go through `gh issue edit --add-assignee`/`--remove-assignee`
  (the backend diffs current vs. desired). This needs a `gh` token with write
  access to the repo's issues; it does **not** require the Projects v2 `project`
  scope or a `[github_project]` config.
- **Jira** — a single-select dropdown (with a `(none)` option to unassign),
  since Jira allows exactly one assignee. Writes go through the Jira REST API
  (`PUT /rest/api/3/issue/<key>` with the assignee `accountId`, or `null` to
  clear) and therefore require the `[jira]` credentials block, same as
  date/status editing.

Both sources use the same endpoint, `PATCH /api/work-item-assignees`
(`{ id, assignee_ids: [...] }`).

## People Page

Alongside the **Board** and **Timeline** views, a third **People** tab tracks a
specific configured person's Jira tickets, fetched **on demand**. Opening the
tab lists the configured `[jira_people]` emails in a single-select dropdown;
**nothing is fetched until a person is selected** (lazy). Selecting a person
fetches, via `GET /api/person-work-items?user=<email>`, two groups:

- **Created by** — tickets where `reporter = <email>`.
- **Mentioned** — a **best-effort full-text proxy**, `text ~ "<accountId>"`. Jira
  has no exact @mention JQL, so this matches content where the person's
  accountId appears (i.e. @mentions); it is **not** an exact match.

Details and caveats:

- The person's accountId is derived from the `reporter` field of one of their own
  created tickets (a single `acli` search with `--limit 1`), so the page needs
  **no `[jira]` REST credentials**. If the person has created nothing, the
  accountId can't be resolved and the Mentioned section shows "mentions
  unavailable" — Created-by still works.
- Results are **deduplicated by issue key**: a ticket that is both created-by and
  mentioned appears only under **Created by**.
- The optional `jira_jql` filter is **AND'd** into both queries, bounding them.
- The list is **not date-enriched** (cards show "—" for planning dates), keeping
  the on-demand fetch fast.
- Only preconfigured people can be queried — the endpoint rejects any `user` not
  in `[jira_people]`.

This reuses the **same** `[jira_people]` list already used for the board-stream
merge (see [Jira Data Fetching](#jira-data-fetching)); that board behavior is
unchanged. A companion endpoint, `GET /api/people`, returns the configured
`[jira_people]` emails and powers the dropdown.

## Backend Commands

Start the local API server:

```bash
cargo run -p quasar
```

Run backend tests:

```bash
cargo test -p quasar -- --nocapture
```

Start the backend with fixture data instead of live `gh` and `acli` calls:

```bash
QUASAR_MODE=fixtures cargo run -p quasar
```

## Frontend Commands

Install dependencies:

```bash
cd apps/frontend
npm install
```

Run frontend tests:

```bash
npm test
```

Build production assets:

```bash
npm run build
```

Start the development server:

```bash
npm run dev
```

In dev mode, the frontend proxies `/api/*` requests to `http://127.0.0.1:3000`.

## Current Behavior

Implemented now:

- backend config loading from `~/.config/quasar/config.toml`
- GitHub fan-out across multiple configured repositories
- Jira per-project fan-out plus an optional `[jira_people]` cross-project query
  (assignee OR reporter across all projects), merged with the board results and
  de-duplicated by issue key
- configurable `jira_base_url` for Jira browse links (default
  `https://quera.atlassian.net`)
- fixture-backed and CLI-backed adapter paths for GitHub and Jira
- short-lived in-memory caching for API responses
- unified work-item rendering with explicit repo metadata in the payload
- work items carrying a list of assignees (multiple on GitHub, 0 or 1 on Jira),
  shown as stacked avatars on cards and the timeline
- summary cards, status chart, recent activity panel, and tests
- source-aware container and source filters plus multi-select status and
  assignee filters in the frontend
- item detail overlay with lazily-fetched body, comments, and metadata
- inline editing of GitHub start/target dates and board Status
- inline editing of assignees (GitHub multi-select, Jira single-select)
- a **People** tab that lazily fetches a configured person's Jira tickets on
  demand (created-by, plus a best-effort full-text "mentioned" proxy), needing
  no `[jira]` credentials

The second filter is **source-aware**: with Source = GitHub it lists
repositories, with Source = Jira it lists projects, and with Source = All it
shows a combined list (Jira entries hinted). Its options are drawn from each
item's `container` (GitHub `owner/repo` or Jira project key), and the dashboard
cards/list update against the active container, source, status, and assignee
selections. The container and source filters are single-select, while the
**status** and **assignee** filters (shared by the board and timeline views) are
**multi-select** — a checkbox dropdown where selecting several values matches
items with **any** of them (OR). The assignee dropdown also carries an
"Unassigned" entry that matches items with no assignee.

## Launch Locally

Use two terminals.

Terminal 1, start the backend from the repo root:

```bash
cargo run -p quasar
```

Terminal 2, start the frontend dev server:

```bash
cd apps/frontend
npm run dev
```

Then open:

```text
http://localhost:5173
```

For predictable sample data, either set `mode = "fixtures"` in your config file
or launch the backend with:

```bash
QUASAR_MODE=fixtures cargo run -p quasar
```

## Launch With Mise

If you use `mise`, you can start services from the repo root:

```bash
mise run dev
```

Or start the fixture-backed version:

```bash
mise run dev-fixtures
```

Available tasks:

- `mise run backend`
- `mise run backend-fixtures`
- `mise run frontend`
- `mise run dev`
- `mise run dev-fixtures`
