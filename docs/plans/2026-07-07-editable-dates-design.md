# Editable Start/Target Dates — Design

Date: 2026-07-07

## Goal

Let users update the Start date and Target date of work items from the detail
overlay. GitHub is fully editable; Jira is read-only due to a CLI limitation.

## Scope decisions

- **GitHub: editable.** Dates are GitHub Projects v2 board field values, written
  via `gh api graphql` mutations.
- **Jira: read-only.** The installed `acli` (1.3.22) cannot set custom fields:
  its `edit --from-json` schema strictly rejects any unknown key (`fields`,
  top-level `customfield_*`, `customFields`), and there is no raw REST/API
  passthrough. Confirmed by probing ticket SSW-1132 (all three JSON shapes
  returned `json: unknown field ...`). The modal shows Jira dates as read-only
  text with a short "not editable" hint.
- **Save UX:** inline auto-save per field (each date input saves on change/blur,
  independently), not a batched Save button.
- **Not on the board:** if a GitHub issue has no project item on the configured
  board, auto-add it (`addProjectV2ItemById`) then set the date.
- **Refresh:** update the modal's shown dates immediately, and invalidate the
  backend work-items cache so board cards reflect the change.

## Backend

### Endpoint

`PATCH /api/work-item-dates`, added to `crates/quasar/src/api.rs`.

Request body:

```json
{ "id": "github:owner/repo#123", "field": "start" | "target", "date": "YYYY-MM-DD" | null }
```

Per-field (matches inline auto-save). Localhost-only, consistent with the rest
of the API — no new auth or CSRF token is added; the localhost bind is the only
protection and this is a deliberate, documented assumption. Handler uses
`State` + `Json` (Json extractor last).

Dispatch by id prefix:

- `jira:` → `409 CONFLICT` "Jira dates are read-only" (guard; the UI never sends
  this).
- `github:{repo}#{n}` → the GitHub write sequence below.
- Unrecognized / malformed / empty parts → `400`.
- Fixture mode → `409 CONFLICT` "writes unavailable in fixture mode".

Returns the updated dates (e.g. the refreshed `WorkItem`, or a small
`{ start_date, target_date }`).

### GitHub write sequence

New `set_project_date` in `crates/quasar/src/adapters/github.rs`, all via
`gh api graphql` (fits in argv; `CommandRunner` stays args-only):

1. **Resolve project + field node ids** from config `owner` + `number`. Query
   `organization(login:$owner){ projectV2(number:$num){ id fields(first:50){
   nodes{ ...on ProjectV2FieldCommon{ id name } } } } }`; on failure, retry with
   `user(login:$owner){ ... }` (handles org-vs-user owners). Match the two date
   field ids by the configured `start_date_field` / `target_date_field` names.
2. **Resolve the issue's project item id** via a repo-scoped query:
   `repository(owner,name){ issue(number:$n){ id projectItems(first:20){ nodes{
   id project{ number } } } } }`, picking the item whose `project.number` matches
   config.
3. **If not on the board** (no matching item): `addProjectV2ItemById(input:{
   projectId, contentId: <issue node id from step 2> }){ item{ id } }` → item id.
4. **Set the value**: for a non-empty date,
   `updateProjectV2ItemFieldValue(input:{ projectId, itemId, fieldId,
   value:{ date:$date } })`; for null/empty, `clearProjectV2ItemFieldValue(input:{
   projectId, itemId, fieldId })`.
5. **Invalidate the work-items cache** (`state.cache`) so the next list load is
   fresh.

Any failure (missing `project` token scope, project/field not found, etc.)
surfaces as `502 BAD_GATEWAY` with gh's stderr message.

Config `GitHubProject` (owner, number, start_date_field, target_date_field) is
sufficient — no schema change.

## Frontend

- `apps/frontend/src/api.ts`: `updateWorkItemDate(id, field, date, signal?)`
  issuing the PATCH, following the existing fetch + `!response.ok` throw pattern.
- `apps/frontend/src/components/ItemDetailModal.tsx`: for GitHub items, the
  Start/Target sidebar values become `<input type="date">` that auto-save on
  change/blur per field, each with a small idle/saving/saved/error indicator. On
  success, update the modal's `detail` and call an `onItemUpdated` prop. For Jira
  items, the dates stay read-only text with a hint (e.g. "Jira dates are
  read-only here").
- `apps/frontend/src/App.tsx`: pass `onItemUpdated={() => loadWorkItems()}` to the
  modal so the board refreshes after a successful save (server-side cache already
  invalidated).

## Data flow

Change a GitHub date input → `updateWorkItemDate(id, field, date)` → PATCH →
backend resolves project/field/item ids → (auto-add if needed) → mutate → clear
cache → return updated dates → modal updates + parent refetches list.

## Testing

- Backend (mock/routing runner): assert the exact `gh api graphql` call sequence
  for the happy path (resolve → mutate), the not-on-board path (resolve → add →
  mutate), the clear-on-empty path (`clearProjectV2ItemFieldValue`), Jira id →
  409, fixture mode → 409, malformed id → 400, and cache invalidation after a
  successful write.
- Frontend: editing a GitHub date calls the PATCH with `(id, "start", date)` and
  updates the shown value on success; Jira dates render as read-only text (no
  input); a failed save shows an error state and does not corrupt the displayed
  value.

## Risks / notes

- **Token scope**: `gh` must be authenticated with a token carrying `project`
  write scope; verified at runtime (missing scope → 502 surfaced to the UI).
- **org-vs-user**: project owner type resolved by trying `organization` then
  `user`.
- `gh` in this environment is 2.4.0 (2022); `gh api graphql` mutations are
  supported.
- Jira write support would require a newer `acli` with custom-field support or a
  direct Jira REST integration (with its own auth) — out of scope here.
