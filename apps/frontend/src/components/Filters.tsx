type FilterValue = "all" | string;

export default function Filters({
  availableRepos,
  availableSources,
  availableStatuses,
  availableAssignees,
  selectedRepo,
  selectedSource,
  selectedStatus,
  selectedAssignee,
  onRepoChange,
  onSourceChange,
  onStatusChange,
  onAssigneeChange,
}: {
  availableRepos: string[];
  availableSources: string[];
  availableStatuses: string[];
  availableAssignees: string[];
  selectedRepo: FilterValue;
  selectedSource: FilterValue;
  selectedStatus: FilterValue;
  selectedAssignee: FilterValue;
  onRepoChange: (value: FilterValue) => void;
  onSourceChange: (value: FilterValue) => void;
  onStatusChange: (value: FilterValue) => void;
  onAssigneeChange: (value: FilterValue) => void;
}) {
  return (
    <section aria-label="Filters" className="filters-panel">
      <div className="filter-field">
        <label htmlFor="repo-filter">Repository</label>
        <select
          id="repo-filter"
          onChange={(event) => onRepoChange(event.target.value)}
          value={selectedRepo}
        >
          <option value="all">All</option>
          {availableRepos.map((repo) => (
            <option key={repo} value={repo}>
              {repo}
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
