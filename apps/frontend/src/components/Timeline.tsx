import type { WorkItem } from "../types";
import AssigneeAvatars from "./AssigneeAvatars";

function toTime(value: string): number | null {
  if (!value) {
    return null;
  }
  const time = Date.parse(value);
  return Number.isNaN(time) ? null : time;
}

function formatDate(value: string): string {
  return value ? value.slice(0, 10) : "—";
}

interface DatedRow {
  item: WorkItem;
  start: number;
  end: number;
}

// First-of-month timestamps spanning [min, max], used for axis ticks.
function monthTicks(min: number, max: number): { time: number; label: string }[] {
  const ticks: { time: number; label: string }[] = [];
  const cursor = new Date(min);
  cursor.setDate(1);
  cursor.setHours(0, 0, 0, 0);
  while (cursor.getTime() <= max) {
    ticks.push({
      time: cursor.getTime(),
      label: cursor.toLocaleDateString(undefined, { month: "short", year: "2-digit" }),
    });
    cursor.setMonth(cursor.getMonth() + 1);
  }
  return ticks;
}

function UndatedList({ items }: { items: WorkItem[] }) {
  if (!items.length) {
    return null;
  }
  return (
    <div className="timeline-undated">
      <h3>Undated ({items.length})</h3>
      <div className="timeline-undated-list">
        {items.map((item) => (
          <span className="timeline-undated-item" key={item.id}>
            <span className="work-item-number">{item.external_id}</span>
            <a href={item.url} rel="noreferrer" target="_blank">
              {item.title}
            </a>
          </span>
        ))}
      </div>
    </div>
  );
}

export default function Timeline({ items }: { items: WorkItem[] }) {
  const rows = items.map((item) => {
    const start = toTime(item.start_date);
    const end = toTime(item.target_date);
    // A single date renders as a point; use it for both ends.
    return { item, start: start ?? end, end: end ?? start };
  });

  const dated = rows.filter(
    (row): row is DatedRow => row.start !== null && row.end !== null,
  );
  const undated = rows.filter((row) => row.start === null).map((row) => row.item);

  if (!dated.length) {
    return (
      <section aria-label="Timeline" className="timeline">
        <p className="empty-state">No issues have a start or target date yet.</p>
        <UndatedList items={undated} />
      </section>
    );
  }

  // Layout constants (must match the grid columns in styles.css).
  const LABEL_COL = 240;
  const GAP = 12;
  const trackOffset = LABEL_COL + GAP;

  // Extend the domain to include "today" so the marker is always visible.
  const today = Date.now();
  const min = Math.min(...dated.map((row) => row.start), today);
  const max = Math.max(...dated.map((row) => row.end), today);
  const span = Math.max(max - min, 1);
  const percent = (time: number) => ((time - min) / span) * 100;

  dated.sort((left, right) => left.start - right.start || left.end - right.end);
  const ticks = monthTicks(min, max);

  // Give each month room so the timeline scrolls horizontally when it's long.
  const trackWidth = Math.max(ticks.length * 120, 560);
  const contentWidth = trackOffset + trackWidth;
  const todayLeft = trackOffset + (percent(today) / 100) * trackWidth;

  return (
    <section aria-label="Timeline" className="timeline">
      <div className="timeline-scroll">
        <div className="timeline-content" style={{ minWidth: `${contentWidth}px` }}>
          <div className="timeline-today" style={{ left: `${todayLeft}px` }} aria-hidden="true">
            <span className="timeline-today-label">Today</span>
          </div>

          <div className="timeline-axis-row">
            <div className="timeline-label-spacer" />
            <div className="timeline-axis">
              {ticks.map((tick) => (
                <span
                  className="timeline-tick"
                  key={tick.time}
                  style={{ left: `${percent(tick.time)}%` }}
                >
                  {tick.label}
                </span>
              ))}
            </div>
          </div>

          <div className="timeline-rows">
            {dated.map(({ item, start, end }) => {
              const left = percent(start);
              const width = Math.max(percent(end) - left, 1.2);
              return (
                <div className="timeline-row" key={item.id}>
                  <div className="timeline-label">
                    <AssigneeAvatars names={item.assignees} />
                    <span className="work-item-number">{item.external_id}</span>
                    <a
                      className="timeline-title"
                      href={item.url}
                      rel="noreferrer"
                      target="_blank"
                    >
                      {item.title}
                    </a>
                  </div>
                  <div className="timeline-track">
                    <span
                      className={`timeline-bar source-${item.source}`}
                      style={{ left: `${left}%`, width: `${width}%` }}
                      title={`${formatDate(item.start_date)} → ${formatDate(item.target_date)}`}
                    >
                      <span className="timeline-bar-dates">
                        {formatDate(item.start_date)} – {formatDate(item.target_date)}
                      </span>
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      <UndatedList items={undated} />
    </section>
  );
}
