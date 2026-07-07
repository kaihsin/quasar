import type { WorkItem } from "../types";

export default function SummaryCards({ items }: { items: WorkItem[] }) {
  const assignedCount = items.filter((item) => item.assignee).length;
  const sourceCount = new Set(items.map((item) => item.source)).size;

  const cards = [
    { label: "Total Items", value: String(items.length), detail: "Across GitHub and Jira" },
    { label: "Active Sources", value: String(sourceCount), detail: "Connected streams" },
    { label: "Assigned Work", value: `${assignedCount} assigned`, detail: "Items with owners" },
  ];

  return (
    <>
      {cards.map((card) => (
        <article className="summary-card" key={card.label}>
          <p>{card.label}</p>
          <strong>{card.value}</strong>
          <span className="summary-detail">{card.detail}</span>
        </article>
      ))}
    </>
  );
}

