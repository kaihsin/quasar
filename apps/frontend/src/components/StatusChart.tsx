export default function StatusChart({
  statusCounts,
  total,
}: {
  statusCounts: Record<string, number>;
  total: number;
}) {
  const entries = Object.entries(statusCounts).sort((left, right) => right[1] - left[1]);

  return (
    <section aria-label="Status distribution" className="viz-card">
      <div className="viz-header">
        <div>
          <p className="section-kicker">Status Pulse</p>
          <h3>Status distribution</h3>
        </div>
      </div>
      <div className="status-stack">
        {entries.length ? (
          entries.map(([status, count]) => {
            const percent = total ? Math.round((count / total) * 100) : 0;
            return (
              <div className="status-row" key={status}>
                <div className="status-row-copy">
                  <span>{status}</span>
                  <strong>{percent}%</strong>
                </div>
                <div aria-hidden="true" className="status-bar-track">
                  <div className="status-bar-fill" style={{ width: `${percent}%` }} />
                </div>
              </div>
            );
          })
        ) : (
          <p className="empty-state">No status data yet.</p>
        )}
      </div>
    </section>
  );
}

