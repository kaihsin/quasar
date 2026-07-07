import { fireEvent, render, screen } from "@testing-library/react";

import Filters from "./Filters";

describe("Filters", () => {
  it("emits source, status, repo, and assignee filter changes", async () => {
    const onSourceChange = jest.fn();
    const onStatusChange = jest.fn();
    const onRepoChange = jest.fn();
    const onAssigneeChange = jest.fn();

    render(
      <Filters
        availableRepos={["openai/quasar", "openai/platform"]}
        availableSources={["github", "jira"]}
        availableStatuses={["open", "in progress"]}
        availableAssignees={["Kai", "Roger"]}
        selectedRepo="all"
        selectedSource="all"
        selectedStatus="all"
        selectedAssignee="all"
        onRepoChange={onRepoChange}
        onSourceChange={onSourceChange}
        onStatusChange={onStatusChange}
        onAssigneeChange={onAssigneeChange}
      />,
    );

    fireEvent.change(screen.getByLabelText("Repository"), { target: { value: "openai/platform" } });
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "jira" } });
    fireEvent.change(screen.getByLabelText("Status"), { target: { value: "in progress" } });
    fireEvent.change(screen.getByLabelText("Assignee"), { target: { value: "Roger" } });

    expect(onRepoChange).toHaveBeenCalledWith("openai/platform");
    expect(onSourceChange).toHaveBeenCalledWith("jira");
    expect(onStatusChange).toHaveBeenCalledWith("in progress");
    expect(onAssigneeChange).toHaveBeenCalledWith("Roger");
  });
});
