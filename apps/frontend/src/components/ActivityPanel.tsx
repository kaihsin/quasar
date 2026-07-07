import type { WorkItem } from "../types";

function formatDate(value: string): string {
  return value ? value.slice(0, 10) : "—";
}

export default function ActivityPanel({ items }: { items: WorkItem[] }) {
  const recentItems = [...items]
    .sort((left, right) => right.updated_at.localeCompare(left.updated_at))
    .slice(0, 5);

  // Nearest target dates first (soonest/overdue deadlines), items that have one.
  const dueItems = items
    .filter((item) => item.target_date)
    .sort((left, right) => left.target_date.localeCompare(right.target_date))
    .slice(0, 5);

  return (
    <section aria-label="Recent activity" className="viz-card">
      <div className="viz-header">
        <div>
          <p className="section-kicker">Momentum</p>
          <h3>Recent activity</h3>
        </div>
      </div>
      {recentItems.length ? (
        <ol className="activity-list">
          {recentItems.map((item) => (
            <li className="activity-item" key={item.id}>
              <span className={`source-badge source-${item.source}`}>{item.source}</span>
              <div>
                <strong>{item.title}</strong>
                <p>
                  {item.external_id} • {item.updated_at}
                </p>
              </div>
            </li>
          ))}
        </ol>
      ) : (
        <p className="empty-state">No recent activity yet.</p>
      )}

      <div className="viz-subhead">
        <h3>Most recent due</h3>
      </div>
      {dueItems.length ? (
        <ol className="activity-list">
          {dueItems.map((item) => (
            <li className="activity-item" key={item.id}>
              <span className={`source-badge source-${item.source}`}>{item.source}</span>
              <div>
                <strong>{item.title}</strong>
                <p>
                  {item.external_id} • due {formatDate(item.target_date)}
                </p>
              </div>
            </li>
          ))}
        </ol>
      ) : (
        <p className="empty-state">No due dates set.</p>
      )}
    </section>
  );
}

