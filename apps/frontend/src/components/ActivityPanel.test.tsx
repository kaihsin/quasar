import { render, screen } from "@testing-library/react";

import ActivityPanel from "./ActivityPanel";
import type { WorkItem } from "../types";

const items: WorkItem[] = [
  {
    source: "github",
    id: "github:101",
    external_id: "101",
    title: "Audit issue sync",
    url: "https://example.com/101",
    status: "open",
    assignee: "Kai",
    labels: ["sync"],
    priority: null,
    created_at: "2026-07-05T10:00:00Z",
    updated_at: "2026-07-06T09:00:00Z",
    start_date: "2026-06-01",
    target_date: "2026-08-20",
    author: "octocat",
    container: "openai/quasar",
    source_metadata: null,
  },
  {
    source: "jira",
    id: "jira:ABC-42",
    external_id: "ABC-42",
    title: "Mirror dashboard health",
    url: "https://jira.example.com/ABC-42",
    status: "in progress",
    assignee: "Kai",
    labels: ["dashboard"],
    priority: "High",
    created_at: "2026-07-05T14:00:00Z",
    updated_at: "2026-07-06T12:00:00Z",
    start_date: "2026-06-10",
    target_date: "2026-07-15",
    author: "ops",
    container: "Unified Tracking",
    source_metadata: null,
  },
];

describe("ActivityPanel", () => {
  it("shows the most recently updated items first", () => {
    render(<ActivityPanel items={items} />);

    expect(screen.getByText("Recent activity")).not.toBeNull();
    expect(screen.getAllByRole("listitem")[0].textContent).toContain("Mirror dashboard health");
    expect(screen.getAllByRole("listitem")[1].textContent).toContain("Audit issue sync");
  });

  it("lists nearest target dates first under Most recent due", () => {
    render(<ActivityPanel items={items} />);

    expect(screen.getByText("Most recent due")).not.toBeNull();
    // Two recent-activity items, then the due list ordered by soonest target date.
    const listItems = screen.getAllByRole("listitem");
    const dueItems = listItems.slice(2);
    expect(dueItems[0].textContent).toContain("Mirror dashboard health");
    expect(dueItems[0].textContent).toContain("due 2026-07-15");
    expect(dueItems[1].textContent).toContain("Audit issue sync");
    expect(dueItems[1].textContent).toContain("due 2026-08-20");
  });
});

