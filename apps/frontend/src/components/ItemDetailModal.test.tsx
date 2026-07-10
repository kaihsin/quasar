import { render, screen, waitFor, fireEvent } from "@testing-library/react";

jest.mock("react-markdown", () => ({
  __esModule: true,
  default: ({ children }: { children: string }) => children,
}));

import ItemDetailModal from "./ItemDetailModal";
import * as api from "../api";
import type { WorkItemDetail } from "../types";

const detail: WorkItemDetail = {
  item: {
    source: "github",
    id: "github:openai/quasar#123",
    external_id: "123",
    title: "Investigate sync gap",
    url: "https://example.com/issues/123",
    status: "open",
    assignees: ["kai"],
    labels: ["bug"],
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
  body: "## Summary\nThe sync job drops events.",
  comments: [{ author: "kai", created_at: "2026-07-06T12:00:00Z", body: "I can repro." }],
  project_status: "Todo",
  status_options: ["Todo", "In Progress", "Done"],
  assignee_options: [],
  assignee_selected: [],
};

test("fetches and renders body, comments, and sidebar; calls onClose", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);
  const onClose = jest.fn();

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={onClose} />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  expect(screen.getByText(/The sync job drops events/)).not.toBeNull();
  expect(screen.getByText("I can repro.")).not.toBeNull();
  expect(screen.getByText("open")).not.toBeNull();
  expect(screen.getByText("octocat")).not.toBeNull();

  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).toHaveBeenCalled();
});

test("focuses the close button on open", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);

  const closeButton = screen.getByLabelText("Close");
  await waitFor(() => expect(document.activeElement).toBe(closeButton));
});

test("clicking the backdrop closes, clicking the panel does not", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);
  const onClose = jest.fn();

  const { container } = render(
    <ItemDetailModal itemId="github:openai/quasar#123" onClose={onClose} />
  );

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());

  const backdrop = container.firstChild as HTMLElement;
  const panel = backdrop.querySelector(".modal-panel") as HTMLElement;

  fireEvent.click(panel);
  expect(onClose).not.toHaveBeenCalled();

  fireEvent.click(backdrop);
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("shows an error state when the fetch fails", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockRejectedValue(new Error("boom"));

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);

  await waitFor(() => expect(screen.getByText(/boom/)).not.toBeNull());
});

test("editing a github date calls updateWorkItemField and notifies parent", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(detail);
  const updateSpy = jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);
  const onItemUpdated = jest.fn();

  render(
    <ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} onItemUpdated={onItemUpdated} />,
  );

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());

  const startInput = screen.getByLabelText("Start date") as HTMLInputElement;
  fireEvent.change(startInput, { target: { value: "2026-08-01" } });
  fireEvent.blur(startInput);

  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith(
      "github:openai/quasar#123",
      "start",
      "2026-08-01",
      expect.anything(),
    ),
  );
  await waitFor(() => expect(onItemUpdated).toHaveBeenCalled());
});

test("re-saving the original value works after a prior save (baseline advances)", async () => {
  const datedDetail = {
    ...detail,
    item: { ...detail.item, start_date: "2026-07-01" },
  };
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(datedDetail);
  const updateSpy = jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);
  // No clearMocks in jest config, so the shared spy retains calls from prior
  // tests; reset it to count only this test's saves.
  updateSpy.mockClear();

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());

  const startInput = screen.getByLabelText("Start date") as HTMLInputElement;

  fireEvent.change(startInput, { target: { value: "2026-08-01" } });
  fireEvent.blur(startInput);

  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith(
      "github:openai/quasar#123",
      "start",
      "2026-08-01",
      expect.anything(),
    ),
  );
  await waitFor(() => expect(screen.getByText("Saved")).not.toBeNull());

  // Revert to the original value: must still trigger a save now that the
  // committed baseline advanced to "2026-08-01".
  fireEvent.change(startInput, { target: { value: "2026-07-01" } });
  fireEvent.blur(startInput);

  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith(
      "github:openai/quasar#123",
      "start",
      "2026-07-01",
      expect.anything(),
    ),
  );
  expect(updateSpy).toHaveBeenCalledTimes(2);
});

test("editing a jira date calls updateWorkItemField with the jira id", async () => {
  const jiraDetail = {
    ...detail,
    item: {
      ...detail.item,
      source: "jira",
      id: "jira:ABC-42",
      repo: null,
      container: "ABC",
      start_date: "2026-06-01",
    },
  };
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(jiraDetail as any);
  const updateSpy = jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);

  render(<ItemDetailModal itemId="jira:ABC-42" onClose={() => {}} />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  const startInput = screen.getByLabelText("Start date") as HTMLInputElement;
  expect(startInput.value).toBe("2026-06-01");
  fireEvent.blur(startInput, { target: { value: "2026-07-10" } });
  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith("jira:ABC-42", "start", "2026-07-10", expect.anything()),
  );
});

test("editing jira status transitions via updateWorkItemField and offers no blank option", async () => {
  const jiraDetail = {
    ...detail,
    item: { ...detail.item, source: "jira", id: "jira:ABC-42", repo: null, status: "Backlog" },
    project_status: "Backlog",
    status_options: ["Backlog", "In Progress", "Done"],
  };
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(jiraDetail as any);
  const updateSpy = jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);

  render(<ItemDetailModal itemId="jira:ABC-42" onClose={() => {}} />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  const select = screen.getByLabelText("Status") as HTMLSelectElement;
  expect(select.value).toBe("Backlog");
  // Jira status is workflow-driven and cannot be cleared: no blank option.
  expect(Array.from(select.options).some((option) => option.value === "")).toBe(false);
  fireEvent.change(select, { target: { value: "Done" } });
  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith("jira:ABC-42", "status", "Done", expect.anything()),
  );
});

test("editing github status calls updateWorkItemField with the option name", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue({
    ...detail, project_status: "Todo", status_options: ["Todo", "In Progress", "Done"],
  });
  const updateSpy = jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);
  const onItemUpdated = jest.fn();
  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} onItemUpdated={onItemUpdated} />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  const select = screen.getByLabelText("Board Status") as HTMLSelectElement;
  expect(select.value).toBe("Todo");
  fireEvent.change(select, { target: { value: "In Progress" } });
  await waitFor(() => expect(updateSpy).toHaveBeenCalledWith("github:openai/quasar#123", "status", "In Progress", expect.anything()));
  await waitFor(() => expect(onItemUpdated).toHaveBeenCalled());
});

test("date inputs seed from the detail item's dates", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue({
    ...detail, item: { ...detail.item, start_date: "2026-06-01", target_date: "2026-06-15" },
  });
  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  expect((screen.getByLabelText("Start date") as HTMLInputElement).value).toBe("2026-06-01");
});

test("selecting the blank board status option clears to null", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue({
    ...detail, project_status: "Todo", status_options: ["Todo", "In Progress", "Done"],
  });
  const updateSpy = jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);
  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  const select = screen.getByLabelText("Board Status") as HTMLSelectElement;
  fireEvent.change(select, { target: { value: "" } });
  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith("github:openai/quasar#123", "status", null, expect.anything()),
  );
});

test("board status baseline advances after a save", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue({
    ...detail, project_status: "Todo", status_options: ["Todo", "In Progress", "Done"],
  });
  const updateSpy = jest.spyOn(api, "updateWorkItemField").mockResolvedValue(undefined);
  updateSpy.mockClear();
  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  const select = screen.getByLabelText("Board Status") as HTMLSelectElement;

  fireEvent.change(select, { target: { value: "In Progress" } });
  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith("github:openai/quasar#123", "status", "In Progress", expect.anything()),
  );
  await waitFor(() => expect(screen.getByText("Saved")).not.toBeNull());

  fireEvent.change(select, { target: { value: "Todo" } });
  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith("github:openai/quasar#123", "status", "Todo", expect.anything()),
  );
  expect(updateSpy).toHaveBeenCalledTimes(2);
});

test("github renders assignee checkboxes and saves toggles", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue({
    ...detail,
    assignee_options: [
      { id: "alice", name: "alice" },
      { id: "bob", name: "bob" },
    ],
    assignee_selected: ["alice"],
  });
  const updateSpy = jest
    .spyOn(api, "updateWorkItemAssignees")
    .mockResolvedValue(undefined);

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());

  const alice = screen.getByLabelText("alice") as HTMLInputElement;
  const bob = screen.getByLabelText("bob") as HTMLInputElement;
  expect(alice.checked).toBe(true);
  expect(bob.checked).toBe(false);

  fireEvent.click(bob);

  await waitFor(() =>
    expect(updateSpy).toHaveBeenCalledWith(
      "github:openai/quasar#123",
      ["alice", "bob"],
      expect.anything(),
    ),
  );
});

test("rapid github toggles compute from optimistic state and don't drop changes", async () => {
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue({
    ...detail,
    assignee_options: [
      { id: "alice", name: "alice" },
      { id: "bob", name: "bob" },
      { id: "carol", name: "carol" },
    ],
    assignee_selected: ["alice"],
  });
  // Keep the first save's promise pending so the second toggle fires before it
  // resolves; this proves `next` is computed from the optimistic selection.
  const updateSpy = jest
    .spyOn(api, "updateWorkItemAssignees")
    .mockImplementation(() => new Promise<void>(() => {}));

  render(<ItemDetailModal itemId="github:openai/quasar#123" onClose={() => {}} />);

  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());

  fireEvent.click(screen.getByLabelText("bob"));
  fireEvent.click(screen.getByLabelText("carol"));

  await waitFor(() =>
    expect(updateSpy).toHaveBeenLastCalledWith(
      "github:openai/quasar#123",
      ["alice", "bob", "carol"],
      expect.anything(),
    ),
  );
  expect((screen.getByLabelText("bob") as HTMLInputElement).checked).toBe(true);
  expect((screen.getByLabelText("carol") as HTMLInputElement).checked).toBe(true);
});

test("jira renders no board status control", async () => {
  const jiraDetail = { ...detail, item: { ...detail.item, source: "jira", id: "jira:ABC-42", repo: null }, project_status: null, status_options: [] };
  jest.spyOn(api, "fetchWorkItemDetail").mockResolvedValue(jiraDetail as any);
  render(<ItemDetailModal itemId="jira:ABC-42" onClose={() => {}} />);
  await waitFor(() => expect(screen.getByText("Investigate sync gap")).not.toBeNull());
  expect(screen.queryByLabelText("Board Status")).toBeNull();
});
