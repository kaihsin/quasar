import { render, screen, waitFor, fireEvent } from "@testing-library/react";

jest.mock("react-markdown", () => ({
  __esModule: true,
  default: ({ children }: { children: string }) => children,
}));

import App from "./App";
import * as api from "./api";
import type { WorkItemDetail, WorkItemsResponse } from "./types";

const listResponse: WorkItemsResponse = {
  data: [
    {
      source: "github",
      id: "github:openai/quasar#123",
      external_id: "123",
      title: "Investigate sync gap",
      url: "https://example.com/issues/123",
      status: "open",
      assignees: ["kai"],
      labels: [],
      priority: null,
      created_at: "2026-07-06T10:00:00Z",
      updated_at: "2026-07-06T11:00:00Z",
      start_date: "",
      target_date: "",
      author: "octocat",
      container: "openai/quasar",
      repo: "openai/quasar",
      source_metadata: null,
    },
  ],
  warnings: [],
  fetched_at: "0",
  cache_status: "miss",
};

const detail: WorkItemDetail = {
  item: listResponse.data[0],
  body: "Body text here.",
  comments: [],
  project_status: null,
  status_options: [],
};

// Mocks the streaming client so it delivers a whole response as one chunk.
function mockStream(response: WorkItemsResponse) {
  return jest.spyOn(api, "streamWorkItems").mockImplementation(async (handlers) => {
    handlers.onChunk(response.data, response.warnings);
    handlers.onDone({
      fetched_at: response.fetched_at,
      cache_status: response.cache_status,
    });
  });
}

test("clicking a card opens the detail modal and fetches detail lazily", async () => {
  mockStream(listResponse);
  const detailSpy = jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);

  render(<App />);

  await waitFor(() =>
    expect(screen.getAllByText("Investigate sync gap").length).toBeGreaterThan(0),
  );
  // Detail not fetched until the user opens an item.
  expect(detailSpy).not.toHaveBeenCalled();

  // The external "Open original" link still points at the item's url.
  const externalLink = screen.getByRole("link", { name: "Open original in new tab" });
  expect(externalLink.getAttribute("href")).toBe("https://example.com/issues/123");

  fireEvent.click(screen.getByRole("button", { name: /Investigate sync gap/i }));

  await waitFor(() =>
    expect(detailSpy).toHaveBeenCalledWith("github:openai/quasar#123", expect.anything()),
  );
  await waitFor(() => expect(screen.getByText("Body text here.")).not.toBeNull());
});

test("saving a date from the modal refetches the work-items list", async () => {
  const listSpy = mockStream(listResponse);
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);
  jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);

  render(<App />);
  await waitFor(() =>
    expect(screen.getAllByText("Investigate sync gap").length).toBeGreaterThan(0),
  );
  fireEvent.click(screen.getByRole("button", { name: /Investigate sync gap/i }));

  const startInput = await screen.findByLabelText("Start date");
  const callsBefore = listSpy.mock.calls.length;
  fireEvent.change(startInput, { target: { value: "2026-08-01" } });
  fireEvent.blur(startInput);

  await waitFor(() => expect(listSpy.mock.calls.length).toBeGreaterThan(callsBefore));
});
