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
  selectedAssignee,
  onContainerChange,
  onSourceChange,
  onStatusChange,
  onAssigneeChange,
}: {
  availableContainers: ContainerOption[];
  availableSources: string[];
  availableStatuses: string[];
  availableAssignees: string[];
  containerLabel: string;
  selectedContainer: FilterValue;
  selectedSource: FilterValue;
  selectedStatus: FilterValue;
  selectedAssignee: FilterValue;
  onContainerChange: (value: FilterValue) => void;
  onSourceChange: (value: FilterValue) => void;
  onStatusChange: (value: FilterValue) => void;
  onAssigneeChange: (value: FilterValue) => void;
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
        <select
          id="assignee-filter"
          onChange={(event) => onAssigneeChange(event.target.value)}
          value={selectedAssignee}
        >
          <option value="all">All</option>
          {availableAssignees.map((assignee) => (
            <option key={assignee} value={assignee}>
              {assignee}
            </option>
          ))}
        </select>
      </div>
    </section>
  );
}
