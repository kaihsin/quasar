import { useEffect, useState } from "react";
import type { CSSProperties } from "react";

import { streamWorkItems } from "./api";
import ActivityPanel from "./components/ActivityPanel";
import ItemDetailModal from "./components/ItemDetailModal";
import Filters from "./components/Filters";
import AssigneeAvatars from "./components/AssigneeAvatars";
import StatusChart from "./components/StatusChart";
import SummaryCards from "./components/SummaryCards";
import Timeline from "./components/Timeline";
import type { WorkItem, WorkItemsResponse } from "./types";

const fallbackItems = [
  { title: "GitHub issue stream", meta: "Will render normalized issues from the Rust API." },
  { title: "Jira ticket stream", meta: "Will render normalized tickets from the Rust API." },
];

// Sentinel filter value for issues with no assignee.
const UNASSIGNED = "Unassigned";

type ColumnKey = "backlog" | "open" | "done";

// Columns rendered on the board. Done work is not fetched from either source,
// so there is no Done column; any stray done-classified item is dropped.
const BOARD_COLUMNS: { key: ColumnKey; label: string }[] = [
  { key: "backlog", label: "Backlog" },
  { key: "open", label: "Open" },
];

// Maps a source's raw status string (e.g. GitHub "open", Jira "Selected for
// Development") into one of the three board columns.
function classifyStatus(status: string): ColumnKey {
  const normalized = status.toLowerCase();
  if (/(done|closed|resolved|complete|merged|shipped|cancell?ed|won'?t)/.test(normalized)) {
    return "done";
  }
  if (/(backlog|selected|to ?do|triage|icebox|proposed|new)/.test(normalized)) {
    return "backlog";
  }
  return "open";
}

// Renders an ISO date/datetime as YYYY-MM-DD, or an em dash when unset.
function formatDate(value: string): string {
  if (!value) {
    return "—";
  }
  return value.slice(0, 10);
}

// Numeric issue number for ordering: GitHub "845" -> 845, Jira "SSW-1131" -> 1131.
function issueNumber(item: WorkItem): number {
  const match = item.external_id.match(/(\d+)\s*$/);
  return match ? Number.parseInt(match[1], 10) : 0;
}

function groupByColumn(items: WorkItem[]): Record<ColumnKey, WorkItem[]> {
  const grouped = { backlog: [], open: [], done: [] } as Record<ColumnKey, WorkItem[]>;
  for (const item of items) {
    grouped[classifyStatus(item.status)].push(item);
  }
  // Order each column by issue number, high to low.
  for (const key of Object.keys(grouped) as ColumnKey[]) {
    grouped[key].sort((left, right) => issueNumber(right) - issueNumber(left));
  }
  return grouped;
}

function isSelectionAvailable(selectedValue: string, availableValues: string[]) {
  return selectedValue === "all" || availableValues.includes(selectedValue);
}

// Every search token must appear somewhere in the item's searchable text.
function matchesSearch(item: WorkItem, tokens: string[]): boolean {
  const haystack = [
    item.title,
    item.external_id,
    item.status,
    item.container,
    item.repo ?? "",
    ...item.assignees,
    item.author ?? "",
    ...item.labels,
  ]
    .join(" ")
    .toLowerCase();
  return tokens.every((token) => haystack.includes(token));
}

export default function App() {
  const [response, setResponse] = useState<WorkItemsResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [selectedContainer, setSelectedContainer] = useState<"all" | string>("all");
  const [selectedSource, setSelectedSource] = useState<"all" | string>("all");
  const [selectedStatuses, setSelectedStatuses] = useState<string[]>([]);
  const [selectedAssignees, setSelectedAssignees] = useState<string[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [view, setView] = useState<"board" | "timeline">("board");
  const [selectedItemId, setSelectedItemId] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();

    void loadWorkItems(controller.signal);

    return () => {
      controller.abort();
    };
  }, []);

  async function loadWorkItems(signal?: AbortSignal) {
    setIsLoading(true);
    setError(null);
    // Reset to an empty set so items appear progressively as each source resolves.
    setResponse({ data: [], warnings: [], fetched_at: "", cache_status: "" });

    try {
      await streamWorkItems(
        {
          onChunk: (data, warnings) => {
            setResponse((prev) => {
              const base = prev ?? { data: [], warnings: [], fetched_at: "", cache_status: "" };
              // A ticket can match multiple queries and arrive in more than one
              // chunk under the same id; drop ids we've already merged.
              const seen = new Set(base.data.map((item) => item.id));
              const fresh = data.filter((item) => !seen.has(item.id));
              return {
                ...base,
                data: [...base.data, ...fresh],
                warnings: [...base.warnings, ...warnings],
              };
            });
          },
          onDone: ({ fetched_at, cache_status }) => {
            setResponse((prev) => {
              const base = prev ?? { data: [], warnings: [], fetched_at: "", cache_status: "" };
              return { ...base, fetched_at, cache_status };
            });
          },
        },
        signal,
      );
    } catch (loadError) {
      if (signal?.aborted) {
        return;
      }
      setError(loadError instanceof Error ? loadError.message : "Unknown API failure");
    } finally {
      if (!signal?.aborted) {
        setIsLoading(false);
      }
    }
  }

  const items = response?.data ?? [];
  // The second filter is source-aware: GitHub repositories, Jira projects, or
  // both when no source is selected. Values match `WorkItem.container`; Jira
  // containers carry a "(Jira)" hint in the combined list since project keys
  // aren't self-evidently boards the way `owner/repo` slugs are.
  const containersCombined = selectedSource === "all";
  const containerScopedItems = containersCombined
    ? items
    : items.filter((item) => item.source === selectedSource);
  const containerSources = new Map<string, string>();
  for (const item of containerScopedItems) {
    if (item.container) {
      containerSources.set(item.container, item.source);
    }
  }
  const availableContainers = Array.from(containerSources.entries())
    .map(([value, source]) => ({
      value,
      label: containersCombined && source === "jira" ? `${value} (Jira)` : value,
    }))
    .sort((left, right) => left.value.localeCompare(right.value));
  const containerLabel =
    selectedSource === "jira"
      ? "Project"
      : selectedSource === "github"
        ? "Repository"
        : "Repository / Project";
  const availableSources = Array.from(new Set(items.map((item) => item.source)));
  const availableStatuses = Array.from(new Set(items.map((item) => item.status)));
  const assigneeNames = Array.from(
    new Set(items.flatMap((item) => item.assignees)),
  ).sort((left, right) => left.localeCompare(right));
  const hasUnassigned = items.some((item) => item.assignees.length === 0);
  const availableAssignees = hasUnassigned ? [UNASSIGNED, ...assigneeNames] : assigneeNames;

  useEffect(() => {
    if (!isSelectionAvailable(selectedContainer, availableContainers.map((c) => c.value))) {
      setSelectedContainer("all");
    }
    if (!isSelectionAvailable(selectedSource, availableSources)) {
      setSelectedSource("all");
    }
    // Prune any selected status/assignee that is no longer available. Returning
    // the same reference when nothing changed keeps this effect (whose
    // `available*` deps are fresh arrays each render) from looping.
    setSelectedStatuses((prev) => {
      const next = prev.filter((value) => availableStatuses.includes(value));
      return next.length === prev.length ? prev : next;
    });
    setSelectedAssignees((prev) => {
      const next = prev.filter((value) => availableAssignees.includes(value));
      return next.length === prev.length ? prev : next;
    });
  }, [
    availableContainers,
    availableSources,
    availableStatuses,
    availableAssignees,
    selectedContainer,
    selectedSource,
  ]);

  const searchTokens = searchQuery.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const filteredItems = items.filter((item) => {
    const containerMatches = selectedContainer === "all" || item.container === selectedContainer;
    const sourceMatches = selectedSource === "all" || item.source === selectedSource;
    const statusMatches =
      selectedStatuses.length === 0 || selectedStatuses.includes(item.status);
    const assigneeMatches =
      selectedAssignees.length === 0 ||
      (selectedAssignees.includes(UNASSIGNED) && item.assignees.length === 0) ||
      item.assignees.some((name) => selectedAssignees.includes(name));
    const searchMatches = searchTokens.length === 0 || matchesSearch(item, searchTokens);
    return containerMatches && sourceMatches && statusMatches && assigneeMatches && searchMatches;
  });
  // Status Pulse excludes done items so the chart reflects in-flight work only.
  const pulseItems = filteredItems.filter((item) => classifyStatus(item.status) !== "done");
  const statusCounts = pulseItems.reduce<Record<string, number>>((counts, item) => {
    counts[item.status] = (counts[item.status] ?? 0) + 1;
    return counts;
  }, {});
  const boardColumns = groupByColumn(filteredItems);

  return (
    <main className="app-shell">
      <section className="hero">
        <div className="hero-copy">
          <p className="eyebrow">Unified Work Dashboard</p>
          <h1>See GitHub issues and Jira tickets in one place.</h1>
          <p className="lede">
            This first shell gives us the dashboard structure: summary cards, a refresh action,
            and a shared work-item surface ready to connect to the Rust API.
          </p>
          {error ? <p className="error-banner">API unavailable: {error}</p> : null}
        </div>
        <button
          className="refresh-button"
          onClick={() => {
            void loadWorkItems();
          }}
          type="button"
        >
          {isLoading ? "Refreshing..." : "Refresh data"}
        </button>
      </section>

      <section aria-label="Summary" className="summary-grid">
        <SummaryCards items={items} />
      </section>

      <section aria-label="Visualizations" className="visual-grid">
        <StatusChart statusCounts={statusCounts} total={pulseItems.length} />
        <ActivityPanel items={filteredItems} />
      </section>

      <section aria-label="Unified work items" className="board-panel">
        <div className="board-header">
          <div>
            <p className="section-kicker">Unified List</p>
            <h2>Work items will land here</h2>
          </div>
          <div className="board-header-actions">
            <div className="view-tabs" role="tablist">
              <button
                aria-selected={view === "board"}
                className="view-tab"
                onClick={() => setView("board")}
                role="tab"
                type="button"
              >
                Board
              </button>
              <button
                aria-selected={view === "timeline"}
                className="view-tab"
                onClick={() => setView("timeline")}
                role="tab"
                type="button"
              >
                Timeline
              </button>
            </div>
            <span className="status-pill">
              {response?.cache_status ? `Cache ${response.cache_status}` : "Read-only v1"}
            </span>
          </div>
        </div>

        <div className="search-bar">
          <input
            aria-label="Search work items"
            className="search-input"
            onChange={(event) => setSearchQuery(event.target.value)}
            placeholder="Search by title, number, label, assignee…"
            type="search"
            value={searchQuery}
          />
          {searchTokens.length ? (
            <span className="search-count">{filteredItems.length} match(es)</span>
          ) : null}
        </div>

        <Filters
          availableContainers={availableContainers}
          availableSources={availableSources}
          availableStatuses={availableStatuses}
          availableAssignees={availableAssignees}
          containerLabel={containerLabel}
          onContainerChange={setSelectedContainer}
          onSourceChange={setSelectedSource}
          onStatusesChange={setSelectedStatuses}
          onAssigneesChange={setSelectedAssignees}
          selectedContainer={selectedContainer}
          selectedSource={selectedSource}
          selectedStatuses={selectedStatuses}
          selectedAssignees={selectedAssignees}
        />

        {response?.warnings.length ? (
          <div className="warning-stack" role="status">
            {response.warnings.map((warning) => (
              <p className="warning-banner" key={`${warning.source}:${warning.message}`}>
                {warning.source} warning: {warning.message}
              </p>
            ))}
          </div>
        ) : null}

        {filteredItems.length && view === "timeline" ? (
          <Timeline items={filteredItems} />
        ) : filteredItems.length ? (
          <div
            className="board-columns"
            style={{ "--cols": BOARD_COLUMNS.length } as CSSProperties}
          >
            {BOARD_COLUMNS.map((column) => {
              const columnItems = boardColumns[column.key];
              return (
                <section aria-label={column.label} className="board-column" key={column.key}>
                  <header className="board-column-header">
                    <h3>{column.label}</h3>
                    <span className="board-column-count">{columnItems.length}</span>
                  </header>
                  <div className="board-column-body">
                    {columnItems.length ? (
                      columnItems.map((item) => (
                        <WorkItemCard
                          item={item}
                          key={item.id}
                          onOpen={() => setSelectedItemId(item.id)}
                        />
                      ))
                    ) : (
                      <p className="board-column-empty">Nothing here</p>
                    )}
                  </div>
                </section>
              );
            })}
          </div>
        ) : items.length ? (
          <div className="placeholder-list">
            <article className="placeholder-item">
              <h3>No work items match the current filters</h3>
              <p>Try clearing the search box or widening the repository, source, or status filters.</p>
            </article>
          </div>
        ) : (
          <div className="placeholder-list">
            {fallbackItems.map((item) => (
              <article className="placeholder-item" key={item.title}>
                <h3>{item.title}</h3>
                <p>{item.meta}</p>
              </article>
            ))}
          </div>
        )}

        {response?.fetched_at ? (
          <p className="footnote">
            Last fetch: {response.fetched_at} • {filteredItems.length} item(s)
          </p>
        ) : null}
      </section>

      {selectedItemId ? (
        <ItemDetailModal
          itemId={selectedItemId}
          onClose={() => setSelectedItemId(null)}
          onItemUpdated={() => {
            void loadWorkItems();
          }}
        />
      ) : null}
    </main>
  );
}

function WorkItemCard({ item, onOpen }: { item: WorkItem; onOpen: () => void }) {
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
