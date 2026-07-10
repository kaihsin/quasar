# Assignee multi-select filtering + per-item assignee editing — Design

Date: 2026-07-10

## Motivation

Two improvements to the unified GitHub/Jira dashboard:

1. **Multi-select assignee filter** for the timeline and board views. Today the
   assignee filter is a single `<select>`; you can only narrow to one person at
   a time.
2. **Per-item assignee editing** from the detail overlay. GitHub issues allow
   *multiple* assignees; Jira work items allow *exactly one*. Today the assignee
   is read-only text.

## Current state (what we're changing)

- The domain model stores a single `assignee: Option<String>`
  (`crates/quasar/src/domain.rs`). The GitHub adapter drops all but the first
  assignee: `issue.assignees.into_iter().next()`.
- The assignee filter is a single-select in `apps/frontend/src/components/Filters.tsx`;
  both board and timeline render from the same `filteredItems` in `App.tsx`.
- The detail overlay (`ItemDetailModal.tsx`) shows assignee as read-only text.
  Inline editing exists for dates and status via `PATCH /api/work-item-field`
  (`{ id, field, value }`), routed by the `github:` / `jira:` id prefix.

## 1. Data model change

Rename the single assignee to a list across the stack:

- `WorkItem.assignee: Option<String>` → `assignees: Vec<String>` (display
  names for Jira, logins for GitHub).
- `github.rs`: collect *all* `issue.assignees` logins.
- `jira.rs`: `fields.assignee` maps to a 0-or-1 element vec.
- Frontend `types.ts`: `assignee: string | null` → `assignees: string[]`.
- Every consumer updated: work-item card, timeline label, search haystack,
  detail modal, `SummaryCards`/`ActivityPanel` if they touch assignee.
- Fixtures and Rust/TS test assertions updated.

## 2. Feature 1 — multi-select assignee filter

- App state `selectedAssignee: "all" | string` → `selectedAssignees: string[]`
  (empty array means "All").
- **Semantics: OR.** An item matches if it has *any* of the selected
  assignees. The `Unassigned` sentinel matches items with an empty `assignees`
  list.
- New **checkbox-dropdown** control in `Filters.tsx`: a button showing
  "N selected" (or "All") that opens a checklist of available assignees; closes
  on outside-click and Escape. The container/source/status filters stay
  single-select (unchanged).
- Available-assignee derivation stays as the union of all item assignees, with
  the `Unassigned` sentinel prepended when any item is unassigned.
- The existing reconcile `useEffect` prunes any selected value that is no longer
  available.
- Both board and timeline render from `filteredItems`, so this covers both
  views with one change.

## 3. Feature 2 — per-item assignee editing

### Candidate users (fetched lazily with the detail)

Mirrors how status options are enriched today (best-effort, gated on
credentials/mode):

- GitHub: `gh api repos/{owner}/{repo}/assignees --paginate` → logins.
- Jira: `GET /rest/api/3/user/assignable/search?issueKey={key}` via the existing
  `jira_curl` → `{ accountId, displayName }`.

### New `WorkItemDetail` fields

- `assignee_options: Vec<AssigneeOption>` where
  `AssigneeOption { id: String, name: String }`.
  - GitHub: `id == name == login`.
  - Jira: `id = accountId`, `name = displayName`.
- `assignee_selected: Vec<String>` — the currently-assigned **ids** (GitHub
  logins; Jira `[accountId]` or `[]`). Drives the widget's initial state
  uniformly across sources. Parsing `accountId` into the Jira detail person is
  added for this.
- The editor renders only when `assignee_options` is non-empty (same gating as
  the status dropdown).

### Write path — new endpoint `PATCH /api/work-item-assignees`

Body: `{ id: string, assignee_ids: string[] }`. A list value does not fit the
existing single-string `value` field, so a dedicated route is cleaner than
overloading `/api/work-item-field`.

- GitHub: fetch the issue's current assignees, diff against the desired set,
  then `gh issue edit <number> -R <repo> --add-assignee <...> --remove-assignee
  <...>` (only the flags with non-empty lists).
- Jira: `PUT /rest/api/3/issue/{key}` with
  `{"fields":{"assignee":{"accountId": id}}}`, or `{"assignee": null}` to
  unassign. Reject a payload with more than one id (`400`).
- On success, invalidate the `work-items` cache (as other writes do).
- Fixture mode or missing credentials → `409` (consistent with existing
  date/status write behavior).

### Editor UI (detail overlay)

- GitHub: a checkbox list of `assignee_options`, initial checked =
  `assignee_selected`. Saving the full selected id-set on change, with a
  Saving / Saved / Couldn't-save indicator (reusing the existing save-state
  pattern).
- Jira: a single `<select>` with a `(none)` unassign option; saves on change.

## 4. Display of multiple assignees

- Cards and timeline labels render stacked avatars (up to ~3, then a `+N`
  overflow chip).
- The card's meta line joins assignee names ("Assigned to A, B") or shows
  "Unassigned".

## 5. Error handling

- Candidate-user fetches are best-effort: any failure leaves `assignee_options`
  empty, which hides the editor (read-only fallback), matching the status
  pattern.
- Writes surface backend errors as the existing Saving→error state.
- Fixture mode / missing credentials return `409`; malformed ids or a Jira
  payload with >1 id return `400`.

## 6. Testing

- **Rust**
  - Adapter: multi-assignee normalization (GitHub keeps all; Jira 0/1);
    assignable-user fetch parsing for both sources; GitHub add/remove diff and
    Jira set/unassign write paths (mock `CommandRunner`).
  - API: new endpoint — fixture-mode `409`, GitHub add/remove sequencing, Jira
    set, `>1` id rejection for Jira, and `work-items` cache invalidation.
- **Frontend**
  - Filter OR-semantics including the `Unassigned` sentinel; checkbox-dropdown
    open/close/toggle behavior.
  - Editor rendering per source and that changes issue the expected save call.

## Out of scope (YAGNI)

- Making the container/source/status filters multi-select.
- Assignee editing directly from cards (editing is via the detail overlay only).
- Any assignee-based grouping/swimlanes on the board or timeline.
