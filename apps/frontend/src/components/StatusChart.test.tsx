import { render, screen } from "@testing-library/react";

import StatusChart from "./StatusChart";

describe("StatusChart", () => {
  it("renders provided status totals with proportions", () => {
    render(<StatusChart statusCounts={{ open: 2, "in progress": 1 }} total={3} />);

    expect(screen.getByText("Status distribution")).not.toBeNull();
    expect(screen.getByText("open")).not.toBeNull();
    expect(screen.getByText("67%")).not.toBeNull();
    expect(screen.getByText("in progress")).not.toBeNull();
    expect(screen.getByText("33%")).not.toBeNull();
  });
});

