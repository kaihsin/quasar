import { fireEvent, render, screen } from "@testing-library/react";

import Filters from "./Filters";

describe("Filters", () => {
  it("emits source, status, container, and assignee filter changes", async () => {
    const onSourceChange = jest.fn();
    const onStatusChange = jest.fn();
    const onContainerChange = jest.fn();
    const onAssigneeChange = jest.fn();

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
        selectedAssignee="all"
        onContainerChange={onContainerChange}
        onSourceChange={onSourceChange}
        onStatusChange={onStatusChange}
        onAssigneeChange={onAssigneeChange}
      />,
    );

    fireEvent.change(screen.getByLabelText("Repository / Project"), {
      target: { value: "openai/platform" },
    });
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "jira" } });
    fireEvent.change(screen.getByLabelText("Status"), { target: { value: "in progress" } });
    fireEvent.change(screen.getByLabelText("Assignee"), { target: { value: "Roger" } });

    expect(onContainerChange).toHaveBeenCalledWith("openai/platform");
    expect(onSourceChange).toHaveBeenCalledWith("jira");
    expect(onStatusChange).toHaveBeenCalledWith("in progress");
    expect(onAssigneeChange).toHaveBeenCalledWith("Roger");
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
        selectedAssignee="all"
        onContainerChange={jest.fn()}
        onSourceChange={jest.fn()}
        onStatusChange={jest.fn()}
        onAssigneeChange={jest.fn()}
      />,
    );

    const projectFilter = screen.getByLabelText("Project");
    expect(projectFilter).not.toBeNull();
    expect(screen.queryByLabelText("Repository")).toBeNull();
  });
});
