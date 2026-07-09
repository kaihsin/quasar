import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, jest } from "@jest/globals";

jest.mock("react-markdown", () => ({
  __esModule: true,
  default: ({ children }: { children: string }) => children,
}));

import App from "./App";
import type { WorkItemsResponse } from "./types";

// Builds a mock fetch Response whose body streams the payload as NDJSON: one
// `items` chunk with all the data, then a terminating `done` chunk. This mirrors
// what GET /api/work-items/stream returns and lets streamWorkItems parse it.
function streamResponse(payload: WorkItemsResponse) {
  const body =
    JSON.stringify({ type: "items", data: payload.data, warnings: payload.warnings }) +
    "\n" +
    JSON.stringify({
      type: "done",
      fetched_at: payload.fetched_at,
      cache_status: payload.cache_status,
    }) +
    "\n";
  const chunks = [new TextEncoder().encode(body)];
  let index = 0;
  return {
    ok: true,
    body: {
      getReader: () => ({
        read: async () =>
          index < chunks.length
            ? { done: false, value: chunks[index++] }
            : { done: true, value: undefined },
      }),
    },
  };
}

afterEach(() => {
  jest.restoreAllMocks();
  delete (global as Partial<typeof globalThis>).fetch;
});

describe("App shell", () => {
  it("renders the dashboard title, refresh button, and unified list region", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse({
        data: [
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
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
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
            start_date: "",
            target_date: "",
            author: "ops",
            container: "Unified Tracking",
            repo: null,
            source_metadata: null,
          },
        ],
        warnings: [],
        fetched_at: "2026-07-06T12:00:00Z",
        cache_status: "miss",
      }),
    ) as unknown) as typeof fetch;

    render(<App />);

    expect(
      screen.getByRole("heading", {
        name: "See GitHub issues and Jira tickets in one place.",
      }),
    ).not.toBeNull();
    expect(await screen.findByRole("button", { name: "Refresh data" })).not.toBeNull();
    expect(screen.getByRole("region", { name: "Unified work items" })).not.toBeNull();
    expect(await screen.findByText("Last fetch: 2026-07-06T12:00:00Z • 2 item(s)")).not.toBeNull();
  });

  it("groups work items into Backlog, Open, and Done columns by status", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse({
        data: [
          {
            source: "github",
            id: "github:101",
            external_id: "101",
            title: "Open GitHub issue",
            url: "https://example.com/101",
            status: "open",
            assignee: null,
            labels: [],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
            source_metadata: null,
          },
          {
            source: "jira",
            id: "jira:SSW-1",
            external_id: "SSW-1",
            title: "Backlog ticket",
            url: "https://quera.atlassian.net/browse/SSW-1",
            status: "Selected for Development",
            assignee: null,
            labels: [],
            priority: null,
            created_at: "",
            updated_at: "",
            start_date: "",
            target_date: "",
            author: null,
            container: "SSW",
            repo: null,
            source_metadata: null,
          },
          {
            source: "jira",
            id: "jira:SSW-2",
            external_id: "SSW-2",
            title: "Finished ticket",
            url: "https://quera.atlassian.net/browse/SSW-2",
            status: "Done",
            assignee: null,
            labels: [],
            priority: null,
            created_at: "",
            updated_at: "",
            start_date: "",
            target_date: "",
            author: null,
            container: "SSW",
            repo: null,
            source_metadata: null,
          },
        ],
        warnings: [],
        fetched_at: "2026-07-06T12:00:00Z",
        cache_status: "miss",
      }),
    ) as unknown) as typeof fetch;

    render(<App />);

    const backlog = await screen.findByRole("region", { name: "Backlog" });
    const open = screen.getByRole("region", { name: "Open" });

    expect(within(backlog).getByRole("button", { name: "Backlog ticket" })).not.toBeNull();
    expect(within(open).getByRole("button", { name: "Open GitHub issue" })).not.toBeNull();

    // There is no Done column, and done-classified items are dropped entirely.
    expect(screen.queryByRole("region", { name: "Done" })).toBeNull();
    expect(screen.queryByText("Show done")).toBeNull();
    expect(screen.queryByRole("button", { name: "Finished ticket" })).toBeNull();

    // Status Pulse omits done items entirely.
    const pulse = screen.getByRole("region", { name: "Status distribution" });
    expect(within(pulse).queryByText("Done")).toBeNull();
  });

  it("switches to the timeline view, placing dated issues and listing undated ones", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse({
        data: [
          {
            source: "github",
            id: "github:101",
            external_id: "101",
            title: "Dated issue",
            url: "https://example.com/101",
            status: "open",
            assignee: null,
            labels: [],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "2026-06-01",
            target_date: "2026-07-01",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
            source_metadata: null,
          },
          {
            source: "github",
            id: "github:102",
            external_id: "102",
            title: "Undated issue",
            url: "https://example.com/102",
            status: "open",
            assignee: null,
            labels: [],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
            source_metadata: null,
          },
        ],
        warnings: [],
        fetched_at: "2026-07-06T12:00:00Z",
        cache_status: "miss",
      }),
    ) as unknown) as typeof fetch;

    render(<App />);

    await screen.findByRole("button", { name: "Refresh data" });
    await screen.findByRole("button", { name: "Dated issue" });
    fireEvent.click(screen.getByRole("tab", { name: "Timeline" }));

    const timeline = screen.getByRole("region", { name: "Timeline" });
    // Dated issue is placed on the timeline; undated issue is listed separately.
    expect(within(timeline).getByRole("link", { name: "Dated issue" })).not.toBeNull();
    expect(within(timeline).getByText("Undated (1)")).not.toBeNull();
    expect(within(timeline).getByRole("link", { name: "Undated issue" })).not.toBeNull();
  });

  it("searches work items by keyword across title and metadata", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse({
        data: [
          {
            source: "github",
            id: "github:101",
            external_id: "101",
            title: "Fix scheduler race",
            url: "https://example.com/101",
            status: "open",
            assignee: "Kai",
            labels: ["bug"],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
            source_metadata: null,
          },
          {
            source: "jira",
            id: "jira:ABC-42",
            external_id: "ABC-42",
            title: "Improve docs coverage",
            url: "https://jira.example.com/ABC-42",
            status: "in progress",
            assignee: "Kai",
            labels: ["documentation"],
            priority: "High",
            created_at: "2026-07-05T14:00:00Z",
            updated_at: "2026-07-06T12:00:00Z",
            start_date: "",
            target_date: "",
            author: "ops",
            container: "Unified Tracking",
            repo: null,
            source_metadata: null,
          },
        ],
        warnings: [],
        fetched_at: "2026-07-06T12:00:00Z",
        cache_status: "miss",
      }),
    ) as unknown) as typeof fetch;

    render(<App />);

    await screen.findByRole("button", { name: "Fix scheduler race" });
    fireEvent.change(screen.getByLabelText("Search work items"), {
      target: { value: "docs" },
    });

    expect(screen.queryByRole("button", { name: "Fix scheduler race" })).toBeNull();
    expect(screen.getByRole("button", { name: "Improve docs coverage" })).not.toBeNull();
    expect(screen.getByText("Last fetch: 2026-07-06T12:00:00Z • 1 item(s)")).not.toBeNull();
  });

  it("filters visible work items by assignee", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse({
        data: [
          {
            source: "github",
            id: "github:101",
            external_id: "101",
            title: "Kai's issue",
            url: "https://example.com/101",
            status: "open",
            assignee: "Kai Wu",
            labels: [],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
            source_metadata: null,
          },
          {
            source: "github",
            id: "github:102",
            external_id: "102",
            title: "Roger's issue",
            url: "https://example.com/102",
            status: "open",
            assignee: "Roger Luo",
            labels: [],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
            source_metadata: null,
          },
        ],
        warnings: [],
        fetched_at: "2026-07-06T12:00:00Z",
        cache_status: "miss",
      }),
    ) as unknown) as typeof fetch;

    render(<App />);

    await screen.findByRole("button", { name: "Kai's issue" });
    fireEvent.change(screen.getByLabelText("Assignee"), { target: { value: "Roger Luo" } });

    expect(screen.queryByRole("button", { name: "Kai's issue" })).toBeNull();
    expect(screen.getByRole("button", { name: "Roger's issue" })).not.toBeNull();
  });

  it("filters visible work items by source and status", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse({
        data: [
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
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
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
            start_date: "",
            target_date: "",
            author: "ops",
            container: "Unified Tracking",
            repo: null,
            source_metadata: null,
          },
        ],
        warnings: [],
        fetched_at: "2026-07-06T12:00:00Z",
        cache_status: "miss",
      }),
    ) as unknown) as typeof fetch;

    render(<App />);

    await screen.findByRole("button", { name: "Audit issue sync" });
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "jira" } });
    fireEvent.change(screen.getByLabelText("Status"), { target: { value: "in progress" } });

    expect(screen.queryByText("Audit issue sync")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Mirror dashboard health" }),
    ).not.toBeNull();
  });

  it("shows repo labels for GitHub items and falls back to container when repo is missing", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse({
        data: [
          {
            source: "github",
            id: "github:openai/quasar#101",
            external_id: "101",
            title: "Repo-aware issue",
            url: "https://example.com/101",
            status: "open",
            assignee: "Kai",
            labels: ["sync"],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
            source_metadata: null,
          },
          {
            source: "github",
            id: "github:202",
            external_id: "202",
            title: "Fallback issue",
            url: "https://example.com/202",
            status: "open",
            assignee: null,
            labels: [],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "legacy/project",
            repo: null,
            source_metadata: null,
          },
        ],
        warnings: [],
        fetched_at: "2026-07-06T12:00:00Z",
        cache_status: "miss",
      }),
    ) as unknown) as typeof fetch;

    render(<App />);

    // Issue number and location render as separate elements within each card.
    const repoAware = (await screen.findByRole("button", { name: "Repo-aware issue" })).closest(
      "article",
    ) as HTMLElement;
    expect(within(repoAware).getByText("101")).not.toBeNull();
    expect(within(repoAware).getByText("openai/quasar")).not.toBeNull();

    const fallback = screen
      .getByRole("button", { name: "Fallback issue" })
      .closest("article") as HTMLElement;
    expect(within(fallback).getByText("202")).not.toBeNull();
    expect(within(fallback).getByText("legacy/project")).not.toBeNull();
  });

  it("populates the repo filter from fetched repos and combines it with source and status filters", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse({
        data: [
          {
            source: "github",
            id: "github:openai/quasar#101",
            external_id: "101",
            title: "Quasar issue",
            url: "https://example.com/101",
            status: "open",
            assignee: "Kai",
            labels: ["sync"],
            priority: null,
            created_at: "2026-07-05T10:00:00Z",
            updated_at: "2026-07-06T09:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/quasar",
            repo: "openai/quasar",
            source_metadata: null,
          },
          {
            source: "github",
            id: "github:openai/platform#202",
            external_id: "202",
            title: "Platform issue",
            url: "https://example.com/202",
            status: "closed",
            assignee: "Kai",
            labels: ["platform"],
            priority: null,
            created_at: "2026-07-05T11:00:00Z",
            updated_at: "2026-07-06T10:00:00Z",
            start_date: "",
            target_date: "",
            author: "octocat",
            container: "openai/platform",
            repo: "openai/platform",
            source_metadata: null,
          },
          {
            source: "jira",
            id: "jira:ABC-42",
            external_id: "ABC-42",
            title: "Mirror dashboard health",
            url: "https://jira.example.com/ABC-42",
            status: "open",
            assignee: "Kai",
            labels: ["dashboard"],
            priority: "High",
            created_at: "2026-07-05T14:00:00Z",
            updated_at: "2026-07-06T12:00:00Z",
            start_date: "",
            target_date: "",
            author: "ops",
            container: "Unified Tracking",
            repo: null,
            source_metadata: null,
          },
        ],
        warnings: [],
        fetched_at: "2026-07-06T12:00:00Z",
        cache_status: "miss",
      }),
    ) as unknown) as typeof fetch;

    render(<App />);

    // Source = All: the combined list carries repos and the Jira project (hinted).
    expect(await screen.findByRole("option", { name: "openai/quasar" })).not.toBeNull();
    expect(screen.getByRole("option", { name: "openai/platform" })).not.toBeNull();
    expect(screen.getByRole("option", { name: "Unified Tracking (Jira)" })).not.toBeNull();

    fireEvent.change(screen.getByLabelText("Repository / Project"), {
      target: { value: "openai/quasar" },
    });
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "github" } });
    fireEvent.change(screen.getByLabelText("Status"), { target: { value: "open" } });

    expect(screen.getByRole("button", { name: "Quasar issue" })).not.toBeNull();
    expect(screen.queryByText("Platform issue")).toBeNull();
    expect(screen.queryByText("Mirror dashboard health")).toBeNull();
    expect(screen.getByText("Last fetch: 2026-07-06T12:00:00Z • 1 item(s)")).not.toBeNull();
  });

  it("resets stale filter selections after refresh when the latest dataset no longer contains them", async () => {
    global.fetch = (jest
      .fn()
      .mockResolvedValueOnce(
        streamResponse({
          data: [
            {
              source: "github",
              id: "github:openai/quasar#101",
              external_id: "101",
              title: "Quasar issue",
              url: "https://example.com/101",
              status: "open",
              assignee: "Kai",
              labels: ["sync"],
              priority: null,
              created_at: "2026-07-05T10:00:00Z",
              updated_at: "2026-07-06T09:00:00Z",
              start_date: "",
              target_date: "",
              author: "octocat",
              container: "openai/quasar",
              repo: "openai/quasar",
              source_metadata: null,
            },
            {
              source: "jira",
              id: "jira:ABC-42",
              external_id: "ABC-42",
              title: "Jira issue",
              url: "https://jira.example.com/ABC-42",
              status: "In Review",
              assignee: "Kai",
              labels: ["dashboard"],
              priority: "High",
              created_at: "2026-07-05T14:00:00Z",
              updated_at: "2026-07-06T12:00:00Z",
              start_date: "",
              target_date: "",
              author: "ops",
              container: "Unified Tracking",
              repo: null,
              source_metadata: null,
            },
          ],
          warnings: [],
          fetched_at: "2026-07-06T12:00:00Z",
          cache_status: "miss",
        }),
      )
      .mockResolvedValueOnce(
        streamResponse({
          data: [
            {
              source: "github",
              id: "github:openai/platform#202",
              external_id: "202",
              title: "Platform issue",
              url: "https://example.com/202",
              status: "In Progress",
              assignee: "Kai",
              labels: ["platform"],
              priority: null,
              created_at: "2026-07-05T11:00:00Z",
              updated_at: "2026-07-06T10:00:00Z",
              start_date: "",
              target_date: "",
              author: "octocat",
              container: "openai/platform",
              repo: "openai/platform",
              source_metadata: null,
            },
          ],
          warnings: [],
          fetched_at: "2026-07-06T13:00:00Z",
          cache_status: "hit",
        }),
      ) as unknown) as typeof fetch;

    render(<App />);

    expect(await screen.findByRole("button", { name: "Quasar issue" })).not.toBeNull();

    // Both selections are individually valid (the Jira item makes "jira" a real
    // source; the GitHub item makes "open" a real status) but jointly match
    // nothing — the Jira item is "In Review", not "open".
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "jira" } });
    fireEvent.change(screen.getByLabelText("Status"), { target: { value: "open" } });

    expect(screen.getByRole("heading", { name: "No work items match the current filters" })).not.toBeNull();
    expect(
      screen.getByText(
        "Try clearing the search box or widening the repository, source, or status filters.",
      ),
    ).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Refresh data" }));

    // The refreshed dataset has neither the "jira" source nor the "open" status,
    // so both stale selections reset to "all" and the new item is shown.
    expect(await screen.findByRole("button", { name: "Platform issue" })).not.toBeNull();
    expect(screen.queryByRole("heading", { name: "No work items match the current filters" })).toBeNull();
    expect((screen.getByLabelText("Repository / Project") as HTMLSelectElement).value).toBe("all");
    expect((screen.getByLabelText("Source") as HTMLSelectElement).value).toBe("all");
    expect((screen.getByLabelText("Status") as HTMLSelectElement).value).toBe("all");
    expect(await screen.findByText("Last fetch: 2026-07-06T13:00:00Z • 1 item(s)")).not.toBeNull();
  });

  // Two items: a GitHub repo container and a Jira project container, used by the
  // source-aware container filter tests below.
  function mixedSourceResponse(): WorkItemsResponse {
    return {
      data: [
        {
          source: "github",
          id: "github:openai/quasar#101",
          external_id: "101",
          title: "GH issue",
          url: "https://example.com/101",
          status: "open",
          assignee: null,
          labels: [],
          priority: null,
          created_at: "2026-07-05T10:00:00Z",
          updated_at: "2026-07-06T09:00:00Z",
          start_date: "",
          target_date: "",
          author: "octocat",
          container: "openai/quasar",
          repo: "openai/quasar",
          source_metadata: null,
        },
        {
          source: "jira",
          id: "jira:SSW-1",
          external_id: "SSW-1",
          title: "Jira ticket",
          url: "https://quera.atlassian.net/browse/SSW-1",
          status: "Selected for Development",
          assignee: null,
          labels: [],
          priority: null,
          created_at: "",
          updated_at: "",
          start_date: "",
          target_date: "",
          author: null,
          container: "SSW",
          repo: null,
          source_metadata: null,
        },
      ],
      warnings: [],
      fetched_at: "2026-07-06T12:00:00Z",
      cache_status: "miss",
    };
  }

  it("offers a combined repo+project list when Source is All and filters by container", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse(mixedSourceResponse()),
    ) as unknown) as typeof fetch;

    render(<App />);
    await screen.findByRole("button", { name: "GH issue" });

    const containerFilter = screen.getByLabelText("Repository / Project") as HTMLSelectElement;
    expect(within(containerFilter).getByRole("option", { name: "openai/quasar" })).not.toBeNull();
    // Jira containers carry a source hint in the combined (All) list.
    expect(within(containerFilter).getByRole("option", { name: "SSW (Jira)" })).not.toBeNull();

    // Selecting the Jira project hides the GitHub item.
    fireEvent.change(containerFilter, { target: { value: "SSW" } });
    expect(screen.queryByRole("button", { name: "GH issue" })).toBeNull();
    expect(screen.getByRole("button", { name: "Jira ticket" })).not.toBeNull();
  });

  it("relabels and scopes the container filter to the selected source", async () => {
    global.fetch = (jest.fn().mockResolvedValue(
      streamResponse(mixedSourceResponse()),
    ) as unknown) as typeof fetch;

    render(<App />);
    await screen.findByRole("button", { name: "GH issue" });

    // GitHub -> "Repository", only repos, no Jira project.
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "github" } });
    const repoFilter = screen.getByLabelText("Repository") as HTMLSelectElement;
    expect(within(repoFilter).getByRole("option", { name: "openai/quasar" })).not.toBeNull();
    expect(within(repoFilter).queryByRole("option", { name: /SSW/ })).toBeNull();

    // Jira -> "Project", only project keys (no source hint in single-source mode).
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "jira" } });
    const projectFilter = screen.getByLabelText("Project") as HTMLSelectElement;
    expect(within(projectFilter).getByRole("option", { name: "SSW" })).not.toBeNull();
    expect(within(projectFilter).queryByRole("option", { name: "openai/quasar" })).toBeNull();
  });
});
