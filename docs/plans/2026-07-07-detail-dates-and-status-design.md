# Detail Date Defaults + Editable GitHub Status — Design

Date: 2026-07-07

## Goals

1. When a work item is opened, the Start/Target date inputs should show the
   existing board dates as their default values (today they open blank).
2. Allow editing a GitHub work item's Projects v2 **Status** (the single-select
   board field, e.g. Todo / In Progress / Done) — distinct from the issue
   open/closed state.

## Root causes / mechanics

- **#1:** The detail modal's `item` comes from the `gh issue view` detail fetch,
  where `normalize_issue` hard-codes `start_date`/`target_date` to `""`, and the
  detail path never runs the `enrich_planning_dates` step the board list uses. So
  `EditableDate` always seeds blank.
- **#2:** Projects v2 "Status" is a `ProjectV2SingleSelectField`. Setting it uses
  `updateProjectV2ItemFieldValue` with `value: { singleSelectOptionId }` (resolve
  the field id + the option id for the chosen name). Reading the current Status
  and available options is not implemented today.

## Backend — detail enrichment (fixes #1, enables #2)

In CLI mode the GitHub detail path runs ONE extra repo-scoped `gh api graphql`
query for the opened issue, filtered to the configured project number, returning
in a single call:

- the item's date field values → fill `item.start_date` / `item.target_date`;
- the item's current Status single-select value → `project_status`;
- the project's Status options (via `projectItems.project.field(name:<status>)
  { ...on ProjectV2SingleSelectField { options { id name } } }`) →
  `status_options` (names).

Best-effort: any failure leaves dates `""`, status `None`, options `[]`. Fixture
mode skips enrichment (no gh).

Limitation: if an issue is not yet on the board, `projectItems` is empty, so its
Status options are empty until it is added (a first date/status write auto-adds
it, as today).

## Domain / config

- `WorkItemDetail` gains `project_status: Option<String>` and
  `status_options: Vec<String>`. `item.start_date`/`target_date` are now
  populated on the detail item.
- `GitHubProject` gains `status_field: String` with default `"Status"`, matching
  the existing `start_date_field` / `target_date_field` pattern.

## Write — generalized field endpoint

Generalize `PATCH /api/work-item-dates` → `PATCH /api/work-item-field` with body:

```json
{ "id": "github:owner/repo#123", "field": "start" | "target" | "status", "value": "<...>" | null }
```

- `start` / `target`: `value` is `YYYY-MM-DD` or null (clear). Validated as today.
- `status`: `value` is a Status option **name** or null (clear).

Adapter: keep `set_project_date`; add `set_project_status(runner, repo, number,
project, option_name: Option<&str>)` sharing the resolve/add/mutate machinery.
Status resolution finds the configured status field (single-select) and the
option id whose name matches `value`; the mutation uses
`value: { singleSelectOptionId: $optionId }`, or `clearProjectV2ItemFieldValue`
when `value` is null. Same guards (Jira 409, fixture 409, missing-project 409,
bad input 400, adapter error 502) and `work-items` cache invalidation on success.

Frontend client `updateWorkItemDate` becomes `updateWorkItemField(id, field,
value)`.

## Frontend

- Dates seed from the enriched `item.start_date` / `item.target_date` — #1 fixed
  with no change beyond the client rename.
- New `EditableStatus` in the modal sidebar (GitHub only): a `<select>` of
  `status_options` with `project_status` selected, plus a blank `(none)` entry to
  clear. Auto-saves on change via `updateWorkItemField(id, "status", value)`,
  with the same saving/saved/error indicator and board refetch (`onItemUpdated`).
  Jira shows nothing extra.

## Testing

- Backend: enrichment query parsing (dates + current status + options, filtered
  by project number; not-on-board → empties); `set_project_status`
  resolve→(add-if-missing)→update-with-singleSelectOptionId, and clear path;
  endpoint field routing for start/target/status; guards; cache invalidation.
- Frontend: date inputs pre-fill from enriched detail; status dropdown renders
  the options with the current one selected; changing it calls
  `updateWorkItemField(id, "status", name)`; selecting blank sends null; Jira
  renders no status control.

## Notes / risks

- Status option name→id is resolved server-side; the client sends names.
- One extra `gh api graphql` call per detail open (accepted).
- `gh` token still needs `project` write scope; live write unverified in CI.
- Generalizing the endpoint churns the just-built date endpoint/client/tests, but
  avoids two near-duplicate endpoints (all on the unmerged branch).
