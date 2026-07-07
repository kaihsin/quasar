# Item Detail Overlay — Design

Date: 2026-07-07

## Goal

Add a pop-up overlay (modal) that displays the full content of a GitHub issue or
Jira ticket — body, comments, and a metadata sidebar — styled like a blend of a
JIRA ticket and a GitHub issue. Detailed data is fetched lazily, only when the
user opens an item.

## Decisions

- **Rendering:** bodies and comments are rendered as Markdown (formatted text,
  code blocks, links).
- **Scope:** read-only display for now, but the API and components are structured
  so write actions (add comment, close/transition) can be added later without
  reshaping the read path.
- **Caching:** no backend caching for detail — each open fetches fresh from
  `gh` / `acli`.

## Backend

### New endpoint

`GET /api/work-items/:id`, added to `router()` in `crates/quasar/src/api.rs`.
The `:id` is the existing work-item id and is parsed to dispatch to the right
adapter:

- `github:{repo}#{num}` → GitHub adapter
- `jira:{KEY}` → Jira adapter

Handler takes `State<AppState>` + `Path(id)`, honors the fixture-vs-CLI split,
and returns fresh data (no cache lookup/store).

### New domain types

In `crates/quasar/src/domain.rs`:

```rust
struct WorkItemDetail {
    item: WorkItem,
    body: Option<String>,
    comments: Vec<Comment>,
}

struct Comment {
    author: Option<String>,
    created_at: String,
    body: String,
}
```

A frontend mirror is added to `apps/frontend/src/types.ts`.

### Adapter additions

- **GitHub** (`crates/quasar/src/adapters/github.rs`): new
  `fetch_issue_detail(runner, repo, number)` running
  `gh issue view <num> -R <repo> --json body,comments,title,author,...`.
  Comments come from the `comments` JSON field. Mirrors the shape of
  `load_work_items_with_runner` and is mockable via the existing
  `MockCommandRunner` / `RoutingRunner` test infrastructure.
- **Jira** (`crates/quasar/src/adapters/jira.rs`): extend the already-wired
  `acli jira workitem view <KEY>` call (currently used for date enrichment) to
  request `description,comment` fields, mapping the body to a string. Extend
  `JiraViewFields` deserialization accordingly.

Both adapters get fixture-loading counterparts and mock-runner tests following
the current dual-mode pattern.

## Frontend

- **`fetchWorkItemDetail(id, signal)`** added to `apps/frontend/src/api.ts`,
  following the existing `fetchWorkItems` pattern (fetch + `response.ok` check +
  typed JSON + `AbortSignal`).
- **New `<ItemDetailModal>` component** (greenfield — no modal exists today):
  a two-column overlay.
  - Left column: title, rendered Markdown body, comments thread.
  - Right column (sidebar): status, assignee, author, labels, priority, start /
    target / created / updated dates, repo/container, and an external link to
    the original issue/ticket.
  - Closes on backdrop click and on Escape.
  - Owns its own `useState` for loading/error/data and a `useEffect` keyed on the
    selected id, so detail is fetched only on open (lazy).
- **Trigger:** `WorkItemCard` (`apps/frontend/src/App.tsx`) gets an `onClick`
  that sets `selectedItemId` state in `App`. The existing external-link `<a>`
  is preserved; clicking the card opens the modal.
- **Markdown:** add `react-markdown@^8` (React 17-compatible; renders without
  `dangerouslySetInnerHTML`, so no XSS surface).

## Data flow

Click card → set `selectedItemId` in `App` → modal mounts →
`fetchWorkItemDetail(id)` → backend parses id → dispatches to `gh` / `acli`
`view` → returns body + comments → modal renders the two columns. Loading and
error states are handled inside the modal.

## Extensibility (not built now)

`WorkItemDetail` and the modal's sidebar / comment area are shaped so a future
action bar (add comment, close/transition) can be added without reshaping the
read path.

## Testing

- Backend: unit tests for id parsing/dispatch, GitHub and Jira detail fetch with
  mock runners, and fixture-backed detail loading.
- Frontend: component tests for the modal (loading, error, rendered body +
  comments, sidebar fields) and for the card click opening the modal.
