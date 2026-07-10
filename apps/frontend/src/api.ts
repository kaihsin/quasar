import type {
  PersonWorkItems,
  SourceWarning,
  WorkItem,
  WorkItemFieldKind,
  WorkItemDetail,
  WorkItemsResponse,
} from "./types";

export async function fetchWorkItems(signal?: AbortSignal): Promise<WorkItemsResponse> {
  const response = await fetch("/api/work-items", { signal });

  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }

  return (await response.json()) as WorkItemsResponse;
}

// One line of the NDJSON stream from GET /api/work-items/stream.
type StreamChunk =
  | { type: "items"; data: WorkItem[]; warnings: SourceWarning[] }
  | { type: "done"; fetched_at: string; cache_status: string };

export interface WorkItemsStreamHandlers {
  // Called once per source as it resolves, with that source's items/warnings.
  onChunk: (data: WorkItem[], warnings: SourceWarning[]) => void;
  // Called once when the stream completes, with the fetch metadata.
  onDone: (meta: { fetched_at: string; cache_status: string }) => void;
}

// Streams work items and invokes `onChunk` as each source resolves so the UI can
// render progressively. Falls back to the caller's error handling on failure.
export async function streamWorkItems(
  handlers: WorkItemsStreamHandlers,
  signal?: AbortSignal,
): Promise<void> {
  const response = await fetch("/api/work-items/stream", { signal });

  if (!response.ok || !response.body) {
    throw new Error(`Request failed with status ${response.status}`);
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  const handleLine = (line: string) => {
    const trimmed = line.trim();
    if (!trimmed) {
      return;
    }
    const chunk = JSON.parse(trimmed) as StreamChunk;
    if (chunk.type === "items") {
      handlers.onChunk(chunk.data, chunk.warnings);
    } else if (chunk.type === "done") {
      handlers.onDone({ fetched_at: chunk.fetched_at, cache_status: chunk.cache_status });
    }
  };

  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    buffer += decoder.decode(value, { stream: true });
    let newlineIndex: number;
    while ((newlineIndex = buffer.indexOf("\n")) >= 0) {
      handleLine(buffer.slice(0, newlineIndex));
      buffer = buffer.slice(newlineIndex + 1);
    }
  }

  // Flush any trailing line not terminated by a newline.
  handleLine(buffer);
}

export async function fetchWorkItemDetail(
  id: string,
  signal?: AbortSignal,
): Promise<WorkItemDetail> {
  const response = await fetch(`/api/work-item-detail?id=${encodeURIComponent(id)}`, { signal });

  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }

  return (await response.json()) as WorkItemDetail;
}

export async function updateWorkItemField(
  id: string,
  field: WorkItemFieldKind,
  value: string | null,
  signal?: AbortSignal,
): Promise<void> {
  const response = await fetch("/api/work-item-field", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id, field, value: value && value.length ? value : null }),
    signal,
  });
  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }
}

export async function fetchPeople(signal?: AbortSignal): Promise<string[]> {
  const response = await fetch("/api/people", { signal });
  if (!response.ok) throw new Error(`Request failed with status ${response.status}`);
  return ((await response.json()) as { users: string[] }).users;
}

export async function fetchPersonWorkItems(
  user: string,
  signal?: AbortSignal,
): Promise<PersonWorkItems> {
  const response = await fetch(`/api/person-work-items?user=${encodeURIComponent(user)}`, { signal });
  if (!response.ok) throw new Error(`Request failed with status ${response.status}`);
  return (await response.json()) as PersonWorkItems;
}

export async function updateWorkItemAssignees(
  id: string,
  assigneeIds: string[],
  signal?: AbortSignal,
): Promise<void> {
  const response = await fetch("/api/work-item-assignees", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id, assignee_ids: assigneeIds }),
    signal,
  });
  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }
}

