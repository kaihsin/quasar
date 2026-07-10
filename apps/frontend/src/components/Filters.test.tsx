import { fireEvent, render, screen } from "@testing-library/react";

import Filters from "./Filters";

describe("Filters", () => {
  it("emits source, status, container, and assignee filter changes", async () => {
    const onSourceChange = jest.fn();
    const onStatusChange = jest.fn();
    const onContainerChange = jest.fn();
    const onAssigneesChange = jest.fn();

    render(
      <Filters
        availableContainers={[
          { value: "openai/quasar", label: "openai/quasar" },
          { value: "openai/platform", label: "openai/platform" },
        ]}
        availableSources={["github", "jira"]}
        availableStatuses={["open", "in progress"]}
        availableAssignees={["Kai", "Roger"]}
        containerLabel="Repository / Project"
        selectedContainer="all"
        selectedSource="all"
        selectedStatus="all"
        selectedAssignees={[]}
        onContainerChange={onContainerChange}
        onSourceChange={onSourceChange}
        onStatusChange={onStatusChange}
        onAssigneesChange={onAssigneesChange}
      />,
    );

    fireEvent.change(screen.getByLabelText("Repository / Project"), {
      target: { value: "openai/platform" },
    });
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "jira" } });
    fireEvent.change(screen.getByLabelText("Status"), { target: { value: "in progress" } });

    // The assignee filter is a checkbox dropdown: open it, then toggle an option.
    fireEvent.click(screen.getByLabelText("Assignee"));
    fireEvent.click(screen.getByRole("checkbox", { name: "Roger" }));

    expect(onContainerChange).toHaveBeenCalledWith("openai/platform");
    expect(onSourceChange).toHaveBeenCalledWith("jira");
    expect(onStatusChange).toHaveBeenCalledWith("in progress");
    expect(onAssigneesChange).toHaveBeenCalledWith(["Roger"]);
  });

  it("adds to an existing assignee selection when another option is toggled", async () => {
    const onAssigneesChange = jest.fn();

    render(
      <Filters
        availableContainers={[{ value: "openai/quasar", label: "openai/quasar" }]}
        availableSources={["github"]}
        availableStatuses={["open"]}
        availableAssignees={["Kai", "Roger"]}
        containerLabel="Repository"
        selectedContainer="all"
        selectedSource="all"
        selectedStatus="all"
        selectedAssignees={["Kai"]}
        onContainerChange={jest.fn()}
        onSourceChange={jest.fn()}
        onStatusChange={jest.fn()}
        onAssigneesChange={onAssigneesChange}
      />,
    );

    // The toggle button (labeled "Assignee") reflects the current selection count.
    const toggle = screen.getByLabelText("Assignee");
    expect(toggle.textContent).toContain("1 selected");
    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole("checkbox", { name: "Roger" }));

    expect(onAssigneesChange).toHaveBeenCalledWith(["Kai", "Roger"]);
  });

  it("renders the container filter under the provided label", async () => {
    render(
      <Filters
        availableContainers={[{ value: "SSW", label: "SSW" }]}
        availableSources={["jira"]}
        availableStatuses={["open"]}
        availableAssignees={["Kai"]}
        containerLabel="Project"
        selectedContainer="all"
        selectedSource="jira"
        selectedStatus="all"
        selectedAssignees={[]}
        onContainerChange={jest.fn()}
        onSourceChange={jest.fn()}
        onStatusChange={jest.fn()}
        onAssigneesChange={jest.fn()}
      />,
    );

    const projectFilter = screen.getByLabelText("Project");
    expect(projectFilter).not.toBeNull();
    expect(screen.queryByLabelText("Repository")).toBeNull();
  });
});
