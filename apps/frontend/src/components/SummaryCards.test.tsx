import { render, screen } from "@testing-library/react";

import SummaryCards from "./SummaryCards";
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
    author: "ops",
    container: "Unified Tracking",
    source_metadata: null,
  },
  {
    source: "jira",
    id: "jira:ABC-44",
    external_id: "ABC-44",
    title: "Refine warning banners",
    url: "https://jira.example.com/ABC-44",
    status: "open",
    assignee: null,
    labels: [],
    priority: "Medium",
    created_at: "2026-07-05T16:00:00Z",
    updated_at: "2026-07-06T13:00:00Z",
    author: "ops",
    container: "Unified Tracking",
    source_metadata: null,
  },
];

describe("SummaryCards", () => {
  it("renders total items, active sources, and assigned work counts", () => {
    render(<SummaryCards items={items} />);

    expect(screen.getByText("Total Items")).not.toBeNull();
    expect(screen.getByText("3")).not.toBeNull();
    expect(screen.getByText("Active Sources")).not.toBeNull();
    expect(screen.getByText("2")).not.toBeNull();
    expect(screen.getByText("Assigned Work")).not.toBeNull();
    expect(screen.getByText("2 assigned")).not.toBeNull();
  });
});

