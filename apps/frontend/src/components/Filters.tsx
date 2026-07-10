import { useEffect, useRef, useState } from "react";

type FilterValue = "all" | string;

// A container is a GitHub repository or a Jira project. `value` is matched
// against `WorkItem.container`; `label` is what the user sees (it may carry a
// source hint when repos and projects are shown together).
export type ContainerOption = { value: string; label: string };

export default function Filters({
  availableContainers,
  availableSources,
  availableStatuses,
  availableAssignees,
  containerLabel,
  selectedContainer,
  selectedSource,
  selectedStatus,
  selectedAssignees,
  onContainerChange,
  onSourceChange,
  onStatusChange,
  onAssigneesChange,
}: {
  availableContainers: ContainerOption[];
  availableSources: string[];
  availableStatuses: string[];
  availableAssignees: string[];
  containerLabel: string;
  selectedContainer: FilterValue;
  selectedSource: FilterValue;
  selectedStatus: FilterValue;
  selectedAssignees: string[];
  onContainerChange: (value: FilterValue) => void;
  onSourceChange: (value: FilterValue) => void;
  onStatusChange: (value: FilterValue) => void;
  onAssigneesChange: (values: string[]) => void;
}) {
  return (
    <section aria-label="Filters" className="filters-panel">
      <div className="filter-field">
        <label htmlFor="container-filter">{containerLabel}</label>
        <select
          id="container-filter"
          onChange={(event) => onContainerChange(event.target.value)}
          value={selectedContainer}
        >
          <option value="all">All</option>
          {availableContainers.map((container) => (
            <option key={container.value} value={container.value}>
              {container.label}
            </option>
          ))}
        </select>
      </div>

      <div className="filter-field">
        <label htmlFor="source-filter">Source</label>
        <select
          id="source-filter"
          onChange={(event) => onSourceChange(event.target.value)}
          value={selectedSource}
        >
          <option value="all">All</option>
          {availableSources.map((source) => (
            <option key={source} value={source}>
              {source}
            </option>
          ))}
        </select>
      </div>

      <div className="filter-field">
        <label htmlFor="status-filter">Status</label>
        <select
          id="status-filter"
          onChange={(event) => onStatusChange(event.target.value)}
          value={selectedStatus}
        >
          <option value="all">All</option>
          {availableStatuses.map((status) => (
            <option key={status} value={status}>
              {status}
            </option>
          ))}
        </select>
      </div>

      <div className="filter-field">
        <label htmlFor="assignee-filter">Assignee</label>
        <AssigneeMultiSelect
          id="assignee-filter"
          options={availableAssignees}
          selected={selectedAssignees}
          onChange={onAssigneesChange}
        />
      </div>
    </section>
  );
}

function AssigneeMultiSelect({
  id,
  options,
  selected,
  onChange,
}: {
  id: string;
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
        aria-haspopup="true"
        className="multiselect-toggle"
        id={id}
        onClick={() => setOpen((v) => !v)}
        type="button"
      >
        {label} ▾
      </button>
      {open ? (
        <div aria-label="Assignee" className="multiselect-menu" role="group">
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
