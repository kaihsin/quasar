import type { WorkItem } from "../types";
import AssigneeAvatars from "./AssigneeAvatars";

// Renders an ISO date/datetime as YYYY-MM-DD, or an em dash when unset.
function formatDate(value: string): string {
  if (!value) {
    return "—";
  }
  return value.slice(0, 10);
}

export default function WorkItemCard({ item, onOpen }: { item: WorkItem; onOpen: () => void }) {
  const location = item.source === "github" && item.repo ? item.repo : item.container;

  return (
    <article className="work-item">
      <div className="work-item-head">
        <span className="work-item-number">{item.external_id}</span>
        <span className={`source-badge source-${item.source}`}>{item.source}</span>
        <button className="work-item-title work-item-title-button" onClick={onOpen} type="button">
          {item.title}
        </button>
        <a
          aria-label="Open original in new tab"
          className="work-item-external"
          href={item.url}
          rel="noreferrer"
          target="_blank"
        >
          ↗
        </a>
        <span className="work-item-location">{location}</span>
        <AssigneeAvatars names={item.assignees} />
      </div>
      <div className="work-item-dates">
        <span className="date-chip">
          <span className="date-label">Start</span>
          <span className="date-value">{formatDate(item.start_date)}</span>
        </span>
        <span className="date-chip date-chip-target">
          <span className="date-label">Target</span>
          <span className="date-value">{formatDate(item.target_date)}</span>
        </span>
      </div>
      <div className="work-item-sub">
        <span className="status-chip">{item.status}</span>
        <span className="item-meta">
          {item.assignees.length ? `Assigned to ${item.assignees.join(", ")}` : "Unassigned"}
          {item.priority ? ` • Priority ${item.priority}` : ""}
        </span>
        {item.labels.map((label) => (
          <span className="label-pill" key={label}>
            {label}
          </span>
        ))}
      </div>
    </article>
  );
}
