import { useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";

import { fetchWorkItemDetail, updateWorkItemAssignees, updateWorkItemField } from "../api";
import type { AssigneeOption, WorkItemFieldKind, WorkItemDetail } from "../types";

function formatDate(value: string): string {
  return value ? value.slice(0, 10) : "—";
}

export default function ItemDetailModal({
  itemId,
  onClose,
  onItemUpdated,
}: {
  itemId: string;
  onClose: () => void;
  onItemUpdated?: () => void;
}) {
  const [detail, setDetail] = useState<WorkItemDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  // Lazy fetch: only runs when the modal is mounted for a given id.
  useEffect(() => {
    const controller = new AbortController();
    setIsLoading(true);
    setError(null);
    setDetail(null);

    fetchWorkItemDetail(itemId, controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) {
          setDetail(result);
        }
      })
      .catch((loadError: unknown) => {
        if (controller.signal.aborted) {
          return;
        }
        setError(loadError instanceof Error ? loadError.message : "Failed to load item");
      })
      .finally(() => {
        if (!controller.signal.aborted) {
          setIsLoading(false);
        }
      });

    return () => controller.abort();
  }, [itemId]);

  // Close on Escape.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  // Move focus into the dialog on open, restore it to the trigger on close.
  useEffect(() => {
    const previouslyFocused = document.activeElement;
    closeButtonRef.current?.focus();
    return () => {
      if (previouslyFocused instanceof HTMLElement) {
        previouslyFocused.focus();
      }
    };
  }, []);

  const item = detail?.item;

  return (
    <div className="modal-backdrop" onClick={onClose} role="presentation">
      <div
        aria-label="Work item detail"
        aria-labelledby={item ? "modal-title" : undefined}
        aria-modal="true"
        className="modal-panel"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
      >
        <button
          aria-label="Close"
          className="modal-close"
          onClick={onClose}
          ref={closeButtonRef}
          type="button"
        >
          ×
        </button>

        {isLoading ? <p className="modal-loading">Loading…</p> : null}
        {error ? <p className="modal-error">Failed to load: {error}</p> : null}

        {item ? (
          <div className="modal-body-grid">
            <div className="modal-main">
              <div className="modal-title-row">
                <span className={`source-badge source-${item.source}`}>{item.source}</span>
                <span className="work-item-number">{item.external_id}</span>
                <h2 id="modal-title">{item.title}</h2>
              </div>

              <div className="modal-markdown">
                {detail?.body ? (
                  <ReactMarkdown>{detail.body}</ReactMarkdown>
                ) : (
                  <p className="modal-empty">No description provided.</p>
                )}
              </div>

              <h3 className="modal-comments-heading">
                Comments ({detail?.comments.length ?? 0})
              </h3>
              <ul className="modal-comments">
                {detail?.comments.map((comment, index) => (
                  <li className="modal-comment" key={index}>
                    <div className="modal-comment-head">
                      <span className="modal-comment-author">{comment.author ?? "Unknown"}</span>
                      <span className="modal-comment-date">{formatDate(comment.created_at)}</span>
                    </div>
                    <div className="modal-markdown">
                      <ReactMarkdown>{comment.body}</ReactMarkdown>
                    </div>
                  </li>
                ))}
              </ul>
            </div>

            <aside className="modal-sidebar">
              <dl>
                <dt>Status</dt>
                <dd>
                  {item.source === "jira" && (detail?.status_options.length ?? 0) > 0 ? (
                    <EditableStatus
                      allowClear={false}
                      itemId={item.id}
                      initial={detail?.project_status ?? item.status}
                      label="Status"
                      options={detail?.status_options ?? []}
                      onSaved={onItemUpdated}
                    />
                  ) : (
                    item.status
                  )}
                </dd>
                <dt>Assignee</dt>
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
                <dt>Author</dt>
                <dd>{item.author ?? "—"}</dd>
                {item.priority ? (
                  <>
                    <dt>Priority</dt>
                    <dd>{item.priority}</dd>
                  </>
                ) : null}
                <dt>{item.source === "github" ? "Repo" : "Project"}</dt>
                <dd>{item.repo ?? item.container}</dd>
                <dt>Start</dt>
                <dd>
                  <EditableDate
                    field="start"
                    initial={item.start_date}
                    itemId={item.id}
                    onSaved={onItemUpdated}
                  />
                </dd>
                <dt>Target</dt>
                <dd>
                  <EditableDate
                    field="target"
                    initial={item.target_date}
                    itemId={item.id}
                    onSaved={onItemUpdated}
                  />
                </dd>
                <dt>Created</dt>
                <dd>{formatDate(item.created_at)}</dd>
                <dt>Updated</dt>
                <dd>{formatDate(item.updated_at)}</dd>
                {item.source === "github" ? (
                  <>
                    <dt>Board Status</dt>
                    <dd>
                      <EditableStatus
                        itemId={item.id}
                        initial={detail?.project_status ?? ""}
                        options={detail?.status_options ?? []}
                        onSaved={onItemUpdated}
                      />
                    </dd>
                  </>
                ) : null}
              </dl>
              {item.labels.length ? (
                <div className="modal-labels">
                  {item.labels.map((label) => (
                    <span className="label-pill" key={label}>
                      {label}
                    </span>
                  ))}
                </div>
              ) : null}
              <a className="modal-external-link" href={item.url} rel="noreferrer" target="_blank">
                Open original ↗
              </a>
            </aside>
          </div>
        ) : null}
      </div>
    </div>
  );
}

type SaveState = "idle" | "saving" | "saved" | "error";

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
    // Optimistic update: reflect the change immediately so the control stays
    // responsive and rapid successive toggles compute from fresh state.
    const previous = selected;
    setSelected(next);
    setState("saving");
    try {
      await updateWorkItemAssignees(itemId, next, controller.signal);
      if (!controller.signal.aborted) {
        setState("saved");
        onSaved?.();
      }
    } catch {
      if (!controller.signal.aborted) {
        setSelected(previous);
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

function EditableDate({
  itemId,
  field,
  initial,
  onSaved,
}: {
  itemId: string;
  field: WorkItemFieldKind;
  initial: string;
  onSaved?: () => void;
}) {
  // <input type="date"> uses YYYY-MM-DD; backend dates are already that shape.
  const initialValue = initial ? initial.slice(0, 10) : "";
  const [value, setValue] = useState(initialValue);
  const [state, setState] = useState<SaveState>("idle");
  // Baseline for change detection; advances to the saved value on success.
  const lastCommitted = useRef(initialValue);
  const controllerRef = useRef<AbortController | null>(null);

  // Abort any in-flight save when the component unmounts.
  useEffect(() => () => controllerRef.current?.abort(), []);

  async function save(next: string) {
    // Cancel any in-flight save before starting a new one.
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    const { signal } = controller;

    setState("saving");
    try {
      await updateWorkItemField(itemId, field, next ? next : null, signal);
      if (!signal.aborted) {
        lastCommitted.current = next;
        setState("saved");
        onSaved?.();
      }
    } catch {
      if (!signal.aborted) {
        setState("error");
      }
    }
  }

  return (
    <span className="editable-date">
      <input
        aria-label={field === "start" ? "Start date" : "Target date"}
        className="date-input"
        onBlur={(event) => {
          if (event.target.value !== lastCommitted.current) {
            void save(event.target.value);
          }
        }}
        onChange={(event) => setValue(event.target.value)}
        type="date"
        value={value}
      />
      <span aria-live="polite" className="date-status-live">
        {state === "saving" ? <span className="date-status">Saving…</span> : null}
        {state === "saved" ? <span className="date-status date-status-ok">Saved</span> : null}
        {state === "error" ? (
          <span className="date-status date-status-err">Couldn't save</span>
        ) : null}
      </span>
    </span>
  );
}

function EditableStatus({
  itemId,
  initial,
  options,
  onSaved,
  label = "Board Status",
  allowClear = true,
}: {
  itemId: string;
  initial: string;
  options: string[];
  onSaved?: () => void;
  label?: string;
  allowClear?: boolean;
}) {
  const [value, setValue] = useState(initial);
  const lastCommitted = useRef(initial);
  const [state, setState] = useState<SaveState>("idle");
  const controllerRef = useRef<AbortController | null>(null);

  useEffect(() => () => controllerRef.current?.abort(), []);

  async function save(next: string) {
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    setState("saving");
    try {
      await updateWorkItemField(itemId, "status", next ? next : null, controller.signal);
      if (!controller.signal.aborted) {
        lastCommitted.current = next;
        setState("saved");
        onSaved?.();
      }
    } catch {
      if (!controller.signal.aborted) {
        // A <select> won't re-fire onChange when re-picking the same visible
        // option, so reset the shown value to the last committed one to let
        // the user retry the failed selection.
        setValue(lastCommitted.current);
        setState("error");
      }
    }
  }

  return (
    <span className="editable-status">
      <select
        aria-label={label}
        className="status-select"
        onChange={(event) => {
          setValue(event.target.value);
          if (event.target.value !== lastCommitted.current) {
            void save(event.target.value);
          }
        }}
        value={value}
      >
        {allowClear ? <option value="">(none)</option> : null}
        {options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
      <span aria-live="polite" className="date-status-live">
        {state === "saving" ? <span className="date-status">Saving…</span> : null}
        {state === "saved" ? <span className="date-status date-status-ok">Saved</span> : null}
        {state === "error" ? <span className="date-status date-status-err">Couldn't save</span> : null}
      </span>
    </span>
  );
}
