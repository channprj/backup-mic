import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "./App";

test("shows the product identity", () => {
  render(<App />);
  expect(screen.getByText("DJI Mic Backup")).toBeInTheDocument();
});
