import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";

vi.mock("./features/backup/BackupApp", () => ({ BackupApp: () => <main>backup-root</main> }));
vi.mock("./features/backup/SettingsApp", () => ({ SettingsApp: () => <main>settings-root</main> }));

import App from "./App";

afterEach(() => {
  cleanup();
  window.history.replaceState({}, "", "/");
});

test("routes the popover window to BackupApp", () => {
  render(<App />);
  expect(screen.getByText("backup-root")).toBeInTheDocument();
});

test("routes only the settings query to SettingsApp", () => {
  window.history.replaceState({}, "", "/?window=settings");
  render(<App />);
  expect(screen.getByText("settings-root")).toBeInTheDocument();
});
