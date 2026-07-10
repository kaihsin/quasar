import { render, screen, fireEvent, waitFor } from "@testing-library/react";

import PeoplePage from "./PeoplePage";
import * as api from "../api";
import type { PersonWorkItems, WorkItem } from "../types";

function makeItem(id: string, title: string): WorkItem {
  return {
    source: "jira",
    id,
    external_id: id.replace("jira:", ""),
    title,
    url: "https://example.com",
    status: "open",
    assignees: [],
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
  };
}

const personData: PersonWorkItems = {
  user: "a@x",
  account_id: "acct-1",
  created_by: [makeItem("jira:SSW-1", "Created ticket")],
  mentioned: [makeItem("jira:SSW-2", "Mentioned ticket")],
};

test("does not fetch person work items until a person is selected", async () => {
  jest.spyOn(api, "fetchPeople").mockResolvedValue(["a@x"]);
  const personSpy = jest
    .spyOn(api, "fetchPersonWorkItems")
    .mockResolvedValue(personData);

  render(<PeoplePage onOpenItem={() => {}} />);

  // Wait for the people list to load into the select.
  await screen.findByRole("option", { name: "a@x" });
  expect(personSpy).not.toHaveBeenCalled();
});

test("selecting a person fetches and renders both sections", async () => {
  jest.spyOn(api, "fetchPeople").mockResolvedValue(["a@x"]);
  const personSpy = jest
    .spyOn(api, "fetchPersonWorkItems")
    .mockResolvedValue(personData);

  render(<PeoplePage onOpenItem={() => {}} />);

  await screen.findByRole("option", { name: "a@x" });

  const select = screen.getByLabelText("Person") as HTMLSelectElement;
  fireEvent.change(select, { target: { value: "a@x" } });

  await waitFor(() =>
    expect(personSpy).toHaveBeenCalledWith("a@x", expect.anything()),
  );

  expect(await screen.findByText("Created by (1)")).not.toBeNull();
  expect(await screen.findByText("Mentioned (1)")).not.toBeNull();
  expect(await screen.findByText("Created ticket")).not.toBeNull();
  expect(await screen.findByText("Mentioned ticket")).not.toBeNull();
});

test("shows mentions-unavailable note when account_id is null", async () => {
  jest.spyOn(api, "fetchPeople").mockResolvedValue(["a@x"]);
  jest.spyOn(api, "fetchPersonWorkItems").mockResolvedValue({
    ...personData,
    account_id: null,
    mentioned: [],
  });

  render(<PeoplePage onOpenItem={() => {}} />);

  await screen.findByRole("option", { name: "a@x" });
  fireEvent.change(screen.getByLabelText("Person"), { target: { value: "a@x" } });

  expect(await screen.findByText(/mentions unavailable/i)).not.toBeNull();
});
