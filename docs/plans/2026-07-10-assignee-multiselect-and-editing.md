# Assignee Multi-Select + Editing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let users multi-select assignees in the board/timeline filter, and add/change assignees per work item from the detail overlay (GitHub allows many, Jira exactly one).

**Architecture:** Backend (Rust/axum) normalizes assignees into a list, enriches item detail with fetched assignable-user candidates, and writes assignees via a new `PATCH /api/work-item-assignees` endpoint (GitHub via `gh issue edit` add/remove diff; Jira via REST `PUT` with an accountId). Frontend (React) changes the assignee filter to a checkbox dropdown and adds a source-appropriate assignee editor to the detail modal.

**Tech Stack:** Rust, axum, serde, `gh`/`acli`/`curl` CLIs; React + TypeScript, Jest/RTL.

**Design doc:** `docs/plans/2026-07-10-assignee-multiselect-and-editing-design.md`

**Conventions:**
- Backend tests: `cargo test -p quasar`. Frontend tests: `cd apps/frontend && npm test`.
- Commit after each task. TDD: write the failing test first where a test exists.

---

## Task 1: Backend — `assignee` → `assignees: Vec<String>`

**Files:**
- Modify: `crates/quasar/src/domain.rs` (field + 3 test literals)
- Modify: `crates/quasar/src/adapters/github.rs:343` (normalize_issue)
- Modify: `crates/quasar/src/adapters/jira.rs:302,347` (both normalizers)
- Modify: `crates/quasar/src/adapters/jira.rs:651` (test assertion)

**Step 1: Update the domain field.** In `domain.rs`, replace `pub assignee: Option<String>,` with:

```rust
#[serde(default)]
pub assignees: Vec<String>,
```

**Step 2: Update the three `WorkItem` literals in `domain.rs` tests** (currently `assignee: Some("kai".to_string())`, and two `assignee: None`):
- `assignee: Some("kai".to_string()),` → `assignees: vec!["kai".to_string()],`
- `assignee: None,` → `assignees: Vec::new(),` (both occurrences)

**Step 3: Update GitHub normalizer.** In `github.rs` `normalize_issue`, replace:

```rust
assignee: issue.assignees.into_iter().next().map(|user| user.login),
```
with
```rust
assignees: issue.assignees.into_iter().map(|user| user.login).collect(),
```

**Step 4: Update Jira normalizers.** In `jira.rs`, in both `normalize_issue_detail` and `normalize_issue`, replace:

```rust
assignee: fields.assignee.map(|person| person.display_name),
```
with
```rust
assignees: fields.assignee.map(|person| person.display_name).into_iter().collect(),
```

**Step 5: Update the Jira adapter test.** In `jira.rs` `jira_fixture_normalizes_into_work_items`, replace:

```rust
assert_eq!(items[0].assignee.as_deref(), Some("Kai Hsin Wu"));
```
with
```rust
assert_eq!(items[0].assignees, vec!["Kai Hsin Wu".to_string()]);
```

**Step 6: Compile + test.** Run: `cargo test -p quasar`. Expected: PASS (fix any remaining `assignee` references the compiler flags).

**Step 7: Commit.**
```bash
git add crates/quasar/src
git commit -m "refactor: model work-item assignees as a list"
```

---

## Task 2: Backend — multi-assignee normalization tests

**Files:**
- Modify: `crates/quasar/src/adapters/github.rs` (tests module)

**Step 1: Write a failing test** that a GitHub issue with two assignees keeps both. Add to the `tests` module in `github.rs`:

```rust
#[test]
fn github_keeps_all_assignees() {
    let payload = r#"[{"number":9,"title":"t","url":"https://github.com/o/r/issues/9","state":"OPEN","assignees":[{"login":"alice"},{"login":"bob"}],"labels":[],"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","author":{"login":"a"}}]"#;
    let runner = MockCommandRunner::success(payload);
    let items = load_work_items_with_runner(&runner, "o/r", None).expect("load");
    assert_eq!(items[0].assignees, vec!["alice".to_string(), "bob".to_string()]);
}
```

**Step 2: Run it.** Run: `cargo test -p quasar github_keeps_all_assignees`. Expected: PASS (Task 1 already implemented the behavior; this locks it in).

**Step 3: Commit.**
```bash
git add crates/quasar/src/adapters/github.rs
git commit -m "test: assert github issues retain all assignees"
```

---

## Task 3: Frontend — `assignees` list + multi-avatar display

**Files:**
- Modify: `apps/frontend/src/types.ts:16`
- Create: `apps/frontend/src/components/AssigneeAvatars.tsx`
- Modify: `apps/frontend/src/App.tsx` (search, derivation, card display)
- Modify: `apps/frontend/src/components/Timeline.tsx:131`
- Modify: `apps/frontend/src/components/ItemDetailModal.tsx:152-153` (display only for now)
- Modify: `apps/frontend/src/styles.css` (avatar stack)

**Step 1: Update the type.** In `types.ts`, replace `assignee: string | null;` with `assignees: string[];`.

**Step 2: Create `AssigneeAvatars.tsx`** — renders up to 3 avatars then a `+N` chip, or a single "unassigned" avatar when empty:

```tsx
import Avatar from "./Avatar";

const MAX_SHOWN = 3;

export default function AssigneeAvatars({ names }: { names: string[] }) {
  if (names.length === 0) {
    return <Avatar name={null} />;
  }
  const shown = names.slice(0, MAX_SHOWN);
  const overflow = names.length - shown.length;
  return (
    <span className="avatar-stack" title={names.join(", ")}>
      {shown.map((name) => (
        <Avatar key={name} name={name} />
      ))}
      {overflow > 0 ? (
        <span aria-label={`${overflow} more`} className="avatar avatar-none">
          +{overflow}
        </span>
      ) : null}
    </span>
  );
}
```

**Step 3: Update `App.tsx`:**
- Search haystack (`matchesSearch`): replace `item.assignee ?? "",` with `...item.assignees,`.
- Assignee derivation (~lines 182-190): replace the `assigneeNames` block with:

```tsx
const assigneeNames = Array.from(
  new Set(items.flatMap((item) => item.assignees)),
).sort((left, right) => left.localeCompare(right));
const hasUnassigned = items.some((item) => item.assignees.length === 0);
```
- Card display: replace `<Avatar name={item.assignee} />` with `<AssigneeAvatars names={item.assignees} />` (add the import; keep the `Avatar` import only if still used elsewhere in the file — it is not, so replace it).
- Card meta line: replace
  ```tsx
  {item.assignee ? `Assigned to ${item.assignee}` : "Unassigned"}
  ```
  with
  ```tsx
  {item.assignees.length ? `Assigned to ${item.assignees.join(", ")}` : "Unassigned"}
  ```

**Step 4: Update `Timeline.tsx`:** replace `import Avatar from "./Avatar";` with `import AssigneeAvatars from "./AssigneeAvatars";` and `<Avatar name={item.assignee} />` with `<AssigneeAvatars names={item.assignees} />`.

**Step 5: Update `ItemDetailModal.tsx` display** (editor comes in Task 7): replace `<dd>{item.assignee ?? "Unassigned"}</dd>` with `<dd>{item.assignees.length ? item.assignees.join(", ") : "Unassigned"}</dd>`.

**Step 6: Add avatar-stack CSS** to `styles.css`:

```css
.avatar-stack {
  display: inline-flex;
  align-items: center;
}
.avatar-stack .avatar:not(:first-child) {
  margin-left: -6px;
}
```

**Step 7: Fix test fixtures/mocks.** Search the frontend tests for `assignee:` object literals and update any `WorkItem` mocks to use `assignees: [...]`. Run: `cd apps/frontend && npx tsc --noEmit` to surface all type errors, fix each.

**Step 8: Run tests.** Run: `cd apps/frontend && npm test`. Expected: PASS.

**Step 9: Commit.**
```bash
git add apps/frontend/src
git commit -m "feat: render work items with a list of assignees"
```

---

## Task 4: Frontend — multi-select assignee filter (checkbox dropdown)

**Files:**
- Modify: `apps/frontend/src/components/Filters.tsx` (assignee field → dropdown)
- Modify: `apps/frontend/src/App.tsx` (state, semantics, props, reconcile)
- Modify: `apps/frontend/src/components/Filters.test.tsx`
- Modify: `apps/frontend/src/styles.css` (dropdown)

**Step 1: Write failing filter tests.** In `App.test.tsx` (or a focused new test), assert OR-semantics: selecting Alice and Bob shows items assigned to either; selecting `Unassigned` shows items with no assignees. (Follow the existing test style in `App.test.tsx`; drive the new checkbox dropdown.) Run and confirm they fail.

**Step 2: Change App state.** In `App.tsx`:
- Replace `const [selectedAssignee, setSelectedAssignee] = useState<"all" | string>("all");` with `const [selectedAssignees, setSelectedAssignees] = useState<string[]>([]);`
- Replace the `assigneeMatches` clause in `filteredItems` with:

```tsx
const assigneeMatches =
  selectedAssignees.length === 0 ||
  (selectedAssignees.includes(UNASSIGNED) && item.assignees.length === 0) ||
  item.assignees.some((name) => selectedAssignees.includes(name));
```
- In the reconcile `useEffect`, replace the `selectedAssignee` reset with a prune:

```tsx
setSelectedAssignees((prev) => prev.filter((value) => availableAssignees.includes(value)));
```
  Update the effect's dependency array (`selectedAssignee` → `selectedAssignees`). Note: this setter form does not need `selectedAssignees` in deps, but keep `availableAssignees`.

**Step 3: Change `Filters.tsx` assignee field** to a checkbox dropdown. Replace the assignee `filter-field` block and the `selectedAssignee: FilterValue` / `onAssigneeChange: (value) => void` props with:
- Props: `selectedAssignees: string[]`, `onAssigneesChange: (values: string[]) => void`.
- A local `AssigneeMultiSelect` component in `Filters.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";

function AssigneeMultiSelect({
  options,
  selected,
  onChange,
}: {
  options: string[];
  selected: string[];
  onChange: (values: string[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocClick(event: MouseEvent) {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const label = selected.length === 0 ? "All" : `${selected.length} selected`;
  const toggle = (value: string) =>
    onChange(
      selected.includes(value)
        ? selected.filter((v) => v !== value)
        : [...selected, value],
    );

  return (
    <div className="multiselect" ref={ref}>
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        className="multiselect-toggle"
        onClick={() => setOpen((v) => !v)}
        type="button"
      >
        {label} ▾
      </button>
      {open ? (
        <div className="multiselect-menu" role="listbox">
          {options.length === 0 ? (
            <span className="multiselect-empty">No assignees</span>
          ) : (
            options.map((option) => (
              <label className="multiselect-option" key={option}>
                <input
                  checked={selected.includes(option)}
                  onChange={() => toggle(option)}
                  type="checkbox"
                />
                {option}
              </label>
            ))
          )}
        </div>
      ) : null}
    </div>
  );
}
```
Render it in the assignee `filter-field` (keep the `<label>Assignee</label>`), passing `availableAssignees`, `selectedAssignees`, `onAssigneesChange`.

**Step 4: Update the `<Filters ... />` call in `App.tsx`** to pass `selectedAssignees={selectedAssignees}` and `onAssigneesChange={setSelectedAssignees}` (remove the old `selectedAssignee`/`onAssigneeChange`).

**Step 5: Add dropdown CSS** to `styles.css`:

```css
.multiselect { position: relative; }
.multiselect-toggle {
  width: 100%;
  text-align: left;
  padding: 0.4rem 0.6rem;
}
.multiselect-menu {
  position: absolute;
  z-index: 20;
  margin-top: 4px;
  max-height: 240px;
  overflow-y: auto;
  min-width: 100%;
  background: var(--surface, #fff);
  border: 1px solid rgba(0, 0, 0, 0.15);
  border-radius: 6px;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
  padding: 4px;
}
.multiselect-option {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 6px;
  white-space: nowrap;
}
.multiselect-empty { display: block; padding: 6px; opacity: 0.7; }
```
(Match existing CSS variables/theme; adjust `--surface` to whatever the file uses.)

**Step 6: Update `Filters.test.tsx`** for the new prop names/interaction (open the dropdown, toggle a checkbox, assert `onAssigneesChange` is called with the expected array).

**Step 7: Run tests.** Run: `cd apps/frontend && npm test`. Expected: PASS.

**Step 8: Commit.**
```bash
git add apps/frontend/src
git commit -m "feat: multi-select assignee filter for board and timeline"
```

---

## Task 5: Backend — assignee candidates + `WorkItemDetail` fields

**Files:**
- Modify: `crates/quasar/src/domain.rs` (`AssigneeOption`, detail fields)
- Modify: `crates/quasar/src/adapters/github.rs` (fetch_assignable_users, set assignee_selected in detail)
- Modify: `crates/quasar/src/adapters/jira.rs` (accountId, fetch_assignable_users, set assignee_selected in detail)
- Modify: `crates/quasar/src/api.rs` (enrich detail with options)

**Step 1: Add domain types.** In `domain.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssigneeOption {
    pub id: String,
    pub name: String,
}
```
Add to `WorkItemDetail`:
```rust
#[serde(default)]
pub assignee_options: Vec<AssigneeOption>,
#[serde(default)]
pub assignee_selected: Vec<String>,
```
Update the three `WorkItemDetail { ... }` literals in `domain.rs` tests to include `assignee_options: Vec::new(), assignee_selected: Vec::new(),`.

**Step 2: GitHub — set `assignee_selected` in detail.** In `github.rs` `normalize_issue_detail`, after building `item`, populate the new fields (selected = the item's assignees/logins):

```rust
let assignee_selected = item.assignees.clone();
Ok(WorkItemDetail {
    item,
    body,
    comments,
    project_status: None,
    status_options: Vec::new(),
    assignee_options: Vec::new(),
    assignee_selected,
})
```

**Step 3: GitHub — fetch assignable users.** Add to `github.rs`:

```rust
/// Best-effort: repo assignable users (logins). Empty on any failure.
/// `gh api --paginate` merges the paged JSON arrays into one array.
pub fn fetch_assignable_users(runner: &dyn CommandRunner, repo: &str) -> Vec<String> {
    let path = format!("repos/{repo}/assignees");
    let Ok(raw) = runner.run("gh", &["api", &path, "--paginate"]) else {
        return Vec::new();
    };
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    serde_json::from_str::<Vec<User>>(&raw)
        .map(|users| users.into_iter().map(|u| u.login).collect())
        .unwrap_or_default()
}
```

**Step 4: GitHub write.** Add to `github.rs`:

```rust
/// Set an issue's assignees to exactly `desired` by diffing against the
/// current assignees and issuing one `gh issue edit` with add/remove flags.
pub fn set_assignees(
    runner: &dyn CommandRunner,
    repo: &str,
    number: &str,
    desired: &[String],
) -> AdapterResult<()> {
    let raw = runner
        .run("gh", &["issue", "view", number, "--json", "assignees", "-R", repo])
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
    #[derive(Deserialize)]
    struct View {
        assignees: Vec<GitHubUser>,
    }
    let current: Vec<String> = serde_json::from_str::<View>(&raw)?
        .assignees
        .into_iter()
        .map(|u| u.login)
        .collect();

    let to_add: Vec<&String> = desired.iter().filter(|d| !current.contains(d)).collect();
    let to_remove: Vec<&String> = current.iter().filter(|c| !desired.contains(c)).collect();
    if to_add.is_empty() && to_remove.is_empty() {
        return Ok(());
    }

    let mut args: Vec<String> = vec![
        "issue".into(), "edit".into(), number.into(), "-R".into(), repo.into(),
    ];
    for login in to_add {
        args.push("--add-assignee".into());
        args.push(login.clone());
    }
    for login in to_remove {
        args.push("--remove-assignee".into());
        args.push(login.clone());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    runner
        .run("gh", &refs)
        .map(|_| ())
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
}
```

**Step 5: Write GitHub tests** (tests module in `github.rs`):
- `fetch_assignable_users_parses_logins` — mock returns `[{"login":"alice"},{"login":"bob"}]`, assert `vec!["alice","bob"]`.
- `set_assignees_adds_and_removes_diff` — a `SeqRunner` returning current `[{"login":"alice"}]` for the `view` call; call `set_assignees(.., desired = ["alice","bob"])`; assert the edit call contains `--add-assignee bob` and no `--remove-assignee`.
- `set_assignees_noops_when_equal` — current == desired ⇒ only the `view` call happens (no `issue edit`).

Run: `cargo test -p quasar github`. Expected: PASS.

**Step 6: Jira — accountId + selected.** In `jira.rs`, add to `JiraPerson`:

```rust
#[serde(default, rename = "accountId")]
account_id: Option<String>,
```
In `normalize_issue_detail`, capture the account id before moving `fields.assignee`:

```rust
let assignee_selected: Vec<String> = fields
    .assignee
    .as_ref()
    .and_then(|p| p.account_id.clone())
    .into_iter()
    .collect();
```
and include `assignee_options: Vec::new(), assignee_selected,` in the returned `WorkItemDetail`. (The `normalize_issue` search path also constructs `JiraPerson`; `account_id` defaults to `None` there — fine.)

**Step 7: Jira — fetch assignable users + write.** Add to `jira.rs`:

```rust
/// Best-effort: users assignable to `key` (accountId + displayName).
pub fn fetch_assignable_users(
    runner: &dyn CommandRunner,
    config: &JiraConfig,
    key: &str,
) -> Vec<crate::domain::AssigneeOption> {
    let url = format!(
        "{}/rest/api/3/user/assignable/search?issueKey={}",
        config.base_url.trim_end_matches('/'),
        key
    );
    let Ok(raw) = jira_curl(runner, config, "GET", &url, None) else {
        return Vec::new();
    };
    #[derive(Deserialize)]
    struct User {
        #[serde(rename = "accountId")]
        account_id: String,
        #[serde(rename = "displayName")]
        display_name: String,
    }
    serde_json::from_str::<Vec<User>>(&raw)
        .map(|users| {
            users
                .into_iter()
                .map(|u| crate::domain::AssigneeOption {
                    id: u.account_id,
                    name: u.display_name,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Set (or clear when None) a Jira issue's single assignee via REST PUT.
pub fn set_assignee(
    runner: &dyn CommandRunner,
    config: &JiraConfig,
    key: &str,
    account_id: Option<&str>,
) -> AdapterResult<()> {
    let value = match account_id {
        Some(id) => serde_json::json!({ "accountId": id }),
        None => serde_json::Value::Null,
    };
    let body = serde_json::json!({ "fields": { "assignee": value } }).to_string();
    jira_curl(runner, config, "PUT", &issue_url(config, key), Some(&body))?;
    Ok(())
}
```

**Step 8: Write Jira tests** (tests module in `jira.rs`):
- `fetch_assignable_users_parses_options` — mock returns `[{"accountId":"a1","displayName":"Alice"}]`, assert one option `{id:"a1", name:"Alice"}`.
- `set_assignee_puts_accountid` — assert a `PUT` curl whose body contains the accountId.
- `set_assignee_clears_with_null` — `None` ⇒ body contains `"assignee":null`.

Run: `cargo test -p quasar jira`. Expected: PASS.

**Step 9: Enrich detail in `api.rs`.** In `fetch_work_item_detail`:
- GitHub `Cli` branch (after project enrichment): `detail.assignee_options = adapters::github::fetch_assignable_users(state.runner.as_ref(), repo).into_iter().map(|login| crate::domain::AssigneeOption { id: login.clone(), name: login }).collect();`
- Jira `Cli` branch (inside the `if let Some(jira)` block, alongside status options): `detail.assignee_options = adapters::jira::fetch_assignable_users(state.runner.as_ref(), jira, key);` (`assignee_selected` is already set by the adapter).

**Step 10: Run all backend tests.** Run: `cargo test -p quasar`. Expected: PASS.

**Step 11: Commit.**
```bash
git add crates/quasar/src
git commit -m "feat: fetch assignable users and expose assignee options in detail"
```

---

## Task 6: Backend — `PATCH /api/work-item-assignees` endpoint

**Files:**
- Modify: `crates/quasar/src/api.rs` (route, handler, tests)

**Step 1: Write failing API tests** in the `api.rs` tests module:
- `update_assignees_rejects_fixture_mode` — PATCH `/api/work-item-assignees` with `{"id":"github:o/r#1","assignee_ids":["alice"]}` on a fixture-mode app ⇒ `409`.
- `update_assignees_rejects_unknown_id` — id without a `github:`/`jira:` prefix ⇒ `400`.
- `update_jira_assignee_rejects_multiple` — `{"id":"jira:SSW-1","assignee_ids":["a","b"]}` ⇒ `400`.
- (Optional) a GitHub add/remove path test using a routing runner, asserting cache invalidation like `update_field_success_invalidates_work_items_cache`.

Run and confirm they fail to compile/pass.

**Step 2: Add the route.** In `router()`, add:
```rust
.route("/api/work-item-assignees", patch(update_work_item_assignees))
```

**Step 3: Add request type + handler:**

```rust
#[derive(Deserialize)]
struct UpdateAssigneesRequest {
    id: String,
    #[serde(default)]
    assignee_ids: Vec<String>,
}

async fn update_work_item_assignees(
    State(state): State<AppState>,
    Json(body): Json<UpdateAssigneesRequest>,
) -> Result<Json<UpdateDatesResponse>, (StatusCode, String)> {
    set_work_item_assignees(&state, &body)
        .map(|_| Json(UpdateDatesResponse { ok: true }))
        .map_err(|error| (error.status, error.message))
}

fn set_work_item_assignees(
    state: &AppState,
    body: &UpdateAssigneesRequest,
) -> Result<(), DetailError> {
    if let Some(key) = body.id.strip_prefix("jira:") {
        if key.is_empty() {
            return Err(DetailError {
                status: StatusCode::BAD_REQUEST,
                message: "missing Jira issue key".to_string(),
            });
        }
        if body.assignee_ids.len() > 1 {
            return Err(DetailError {
                status: StatusCode::BAD_REQUEST,
                message: "Jira work items allow at most one assignee".to_string(),
            });
        }
        if matches!(state.jira_source, JiraSource::Fixture(_)) {
            return Err(DetailError {
                status: StatusCode::CONFLICT,
                message: "writes unavailable in fixture mode".to_string(),
            });
        }
        let jira = state.jira_config.as_ref().ok_or_else(|| DetailError {
            status: StatusCode::CONFLICT,
            message: "no [jira] credentials configured; cannot edit Jira fields".to_string(),
        })?;
        adapters::jira::set_assignee(
            state.runner.as_ref(),
            jira,
            key,
            body.assignee_ids.first().map(String::as_str),
        )
        .map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;
        state.cache.invalidate("work-items");
        return Ok(());
    }

    let rest = body.id.strip_prefix("github:").ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("unrecognized work-item id: {}", body.id),
    })?;
    let (repo, number) = rest.rsplit_once('#').ok_or_else(|| DetailError {
        status: StatusCode::BAD_REQUEST,
        message: format!("malformed GitHub id: {}", body.id),
    })?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DetailError {
            status: StatusCode::BAD_REQUEST,
            message: "issue number must be numeric".to_string(),
        });
    }
    match &state.github_source {
        GitHubSource::Fixture(_) => {
            return Err(DetailError {
                status: StatusCode::CONFLICT,
                message: "writes unavailable in fixture mode".to_string(),
            })
        }
        GitHubSource::Cli => {}
    }
    adapters::github::set_assignees(state.runner.as_ref(), repo, number, &body.assignee_ids)
        .map_err(|error| DetailError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;
    state.cache.invalidate("work-items");
    Ok(())
}
```
(GitHub assignee edits do not require a `[github_project]`, unlike dates/status — do not gate on it.)

**Step 4: Run tests.** Run: `cargo test -p quasar`. Expected: PASS.

**Step 5: Commit.**
```bash
git add crates/quasar/src/api.rs
git commit -m "feat: PATCH /api/work-item-assignees for GitHub and Jira"
```

---

## Task 7: Frontend — assignee editor in the detail overlay

**Files:**
- Modify: `apps/frontend/src/types.ts` (detail fields, `AssigneeOption`)
- Modify: `apps/frontend/src/api.ts` (`updateWorkItemAssignees`)
- Modify: `apps/frontend/src/components/ItemDetailModal.tsx` (editor)
- Modify: `apps/frontend/src/components/ItemDetailModal.test.tsx`

**Step 1: Update types.** In `types.ts` add:
```ts
export interface AssigneeOption {
  id: string;
  name: string;
}
```
and to `WorkItemDetail`:
```ts
assignee_options: AssigneeOption[];
assignee_selected: string[];
```

**Step 2: Add the API call** in `api.ts`:
```ts
export async function updateWorkItemAssignees(
  id: string,
  assigneeIds: string[],
  signal?: AbortSignal,
): Promise<void> {
  const response = await fetch("/api/work-item-assignees", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id, assignee_ids: assigneeIds }),
    signal,
  });
  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }
}
```

**Step 3: Write a failing modal test** in `ItemDetailModal.test.tsx`: mock `fetchWorkItemDetail` to return a GitHub detail with `assignee_options: [{id:"alice",name:"alice"},{id:"bob",name:"bob"}]`, `assignee_selected: ["alice"]`; assert the checkboxes render with alice checked; toggling bob calls `updateWorkItemAssignees` with `["alice","bob"]`. (Mock `../api` as the existing tests do.) Run and confirm it fails.

**Step 4: Add the `EditableAssignees` component** to `ItemDetailModal.tsx`:

```tsx
function EditableAssignees({
  itemId,
  source,
  options,
  initialSelected,
  onSaved,
}: {
  itemId: string;
  source: "github" | "jira";
  options: AssigneeOption[];
  initialSelected: string[];
  onSaved?: () => void;
}) {
  const [selected, setSelected] = useState<string[]>(initialSelected);
  const [state, setState] = useState<SaveState>("idle");
  const controllerRef = useRef<AbortController | null>(null);
  useEffect(() => () => controllerRef.current?.abort(), []);

  async function save(next: string[]) {
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    setState("saving");
    try {
      await updateWorkItemAssignees(itemId, next, controller.signal);
      if (!controller.signal.aborted) {
        setSelected(next);
        setState("saved");
        onSaved?.();
      }
    } catch {
      if (!controller.signal.aborted) {
        setState("error");
      }
    }
  }

  const statusLive = (
    <span aria-live="polite" className="date-status-live">
      {state === "saving" ? <span className="date-status">Saving…</span> : null}
      {state === "saved" ? <span className="date-status date-status-ok">Saved</span> : null}
      {state === "error" ? (
        <span className="date-status date-status-err">Couldn't save</span>
      ) : null}
    </span>
  );

  if (source === "jira") {
    const value = selected[0] ?? "";
    return (
      <span className="editable-status">
        <select
          aria-label="Assignee"
          className="status-select"
          onChange={(event) => {
            const next = event.target.value ? [event.target.value] : [];
            void save(next);
          }}
          value={value}
        >
          <option value="">(none)</option>
          {options.map((option) => (
            <option key={option.id} value={option.id}>
              {option.name}
            </option>
          ))}
        </select>
        {statusLive}
      </span>
    );
  }

  const toggle = (id: string) => {
    const next = selected.includes(id)
      ? selected.filter((value) => value !== id)
      : [...selected, id];
    void save(next);
  };
  return (
    <span className="editable-assignees">
      {options.map((option) => (
        <label className="assignee-option" key={option.id}>
          <input
            checked={selected.includes(option.id)}
            onChange={() => toggle(option.id)}
            type="checkbox"
          />
          {option.name}
        </label>
      ))}
      {statusLive}
    </span>
  );
}
```
Add `AssigneeOption` to the `types` import and `updateWorkItemAssignees` to the `api` import.

**Step 5: Wire it into the sidebar.** Replace the assignee `<dd>` (from Task 3 Step 5) with:

```tsx
<dd>
  {(detail?.assignee_options.length ?? 0) > 0 ? (
    <EditableAssignees
      initialSelected={detail?.assignee_selected ?? []}
      itemId={item.id}
      onSaved={onItemUpdated}
      options={detail?.assignee_options ?? []}
      source={item.source}
    />
  ) : item.assignees.length ? (
    item.assignees.join(", ")
  ) : (
    "Unassigned"
  )}
</dd>
```

**Step 6: Add editor CSS** to `styles.css`:
```css
.editable-assignees {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.assignee-option {
  display: flex;
  align-items: center;
  gap: 6px;
}
```

**Step 7: Run tests.** Run: `cd apps/frontend && npm test`. Expected: PASS. Then `npx tsc --noEmit` clean.

**Step 8: Commit.**
```bash
git add apps/frontend/src
git commit -m "feat: edit work-item assignees from the detail overlay"
```

---

## Task 8: Docs — README

**Files:**
- Modify: `README.md` (Item Detail Overlay / editing sections; Current Behavior)

**Step 1:** Document that assignees are now a list; the detail overlay edits assignees (GitHub multiple via `gh issue edit`; Jira single via REST `PUT`, requiring the `[jira]` block); assignable users are fetched lazily; and the assignee filter is multi-select. Note GitHub assignee editing needs `gh` write access to the repo (issues) — not the `project` scope.

**Step 2: Commit.**
```bash
git add README.md
git commit -m "docs: assignee list, multi-select filter, and editing"
```

---

## Final verification

- `cargo test -p quasar` — all pass.
- `cd apps/frontend && npm test && npx tsc --noEmit` — all pass, no type errors.
- Manual smoke (optional, live creds): `mise run dev`, open an item, add/remove a GitHub assignee and change a Jira assignee; multi-select two assignees in the filter and confirm board + timeline both narrow with OR-semantics. Use `/verify` or `/run` to drive the app.
