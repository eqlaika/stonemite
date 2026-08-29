import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { TriggersPage } from "./TriggersPage";
import {
  filterTriggers,
  folderSubtree,
  moveTriggers,
  newFolder,
  newTrigger,
  removeFolder,
  setTriggersEnabled,
} from "../triggers/model";
import type { TriggerLibrary } from "../triggers/types";

afterEach(cleanup);

async function renderWorkbench() {
  render(<TriggersPage />);
  await waitFor(() =>
    expect(screen.getByLabelText("Search triggers")).toBeInTheDocument(),
  );
}

describe("TriggersPage", () => {
  it("creates a trigger and edits it in the workbench", async () => {
    await renderWorkbench();

    fireEvent.click(screen.getByRole("button", { name: /Trigger$/ }));
    // A new trigger is focused in the editor.
    const editor = screen.getByRole("region", { name: "Trigger editor" });
    const name = within(editor).getByLabelText("Name");
    fireEvent.change(name, { target: { value: "Complete Heal" } });

    // The list reflects the rename and the result count updates.
    expect(
      screen.getByRole("option", { name: /Complete Heal/ }),
    ).toBeInTheDocument();
    expect(screen.getByText("1 of 1")).toBeInTheDocument();

    // The save button arms once the library is dirty.
    expect(screen.getByRole("button", { name: /Save/ })).toBeEnabled();
  });

  it("filters triggers through search", async () => {
    await renderWorkbench();
    fireEvent.click(screen.getByRole("button", { name: /Trigger$/ }));
    const editor = screen.getByRole("region", { name: "Trigger editor" });
    fireEvent.change(within(editor).getByLabelText("Name"), {
      target: { value: "Mez broken" },
    });

    fireEvent.change(screen.getByLabelText("Search triggers"), {
      target: { value: "no such trigger" },
    });
    expect(
      screen.getByText("No triggers match the current filters."),
    ).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Search triggers"), {
      target: { value: "mez" },
    });
    expect(
      screen.getByRole("option", { name: /Mez broken/ }),
    ).toBeInTheDocument();
  });

  it("supports bulk enable through the selection bar", async () => {
    await renderWorkbench();
    fireEvent.click(screen.getByRole("button", { name: /Trigger$/ }));
    fireEvent.click(screen.getByRole("button", { name: /Trigger$/ }));

    fireEvent.click(
      screen.getByRole("checkbox", { name: "Select all visible triggers" }),
    );
    const bulk = screen.getByRole("toolbar", { name: "Bulk actions" });
    expect(within(bulk).getByText("2 selected")).toBeInTheDocument();
    fireEvent.click(within(bulk).getByRole("button", { name: "Enable" }));

    const toggles = screen.getAllByRole("checkbox", {
      name: /^Enable /,
    });
    for (const toggle of toggles) {
      expect(toggle).toBeChecked();
    }
  });

  it("exposes the test bench behind a toggle", async () => {
    await renderWorkbench();
    fireEvent.click(screen.getByRole("button", { name: /Test bench/ }));
    expect(
      screen.getByRole("region", { name: "Test bench" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Test log lines")).toBeInTheDocument();
  });
});

describe("trigger library model", () => {
  function libraryWith(): TriggerLibrary {
    const folder = newFolder(null, 0);
    const child = newFolder(folder.id, 0);
    const inFolder = newTrigger(folder.id, 0);
    inFolder.name = "In folder";
    const inChild = newTrigger(child.id, 0);
    inChild.name = "In child";
    const atRoot = newTrigger(null, 1);
    atRoot.name = "At root";
    return {
      schemaVersion: 1,
      folders: [folder, child],
      triggers: [inFolder, inChild, atRoot],
      profiles: [],
      textOverlays: [],
      timerOverlays: [],
      assets: [],
    };
  }

  it("folder filters include nested subfolders", () => {
    const library = libraryWith();
    const root = library.folders[0];
    expect(folderSubtree(library, root.id)).toHaveLength(2);
    const filtered = filterTriggers(library, root.id, "", "all");
    expect(filtered.map((trigger) => trigger.name).sort()).toEqual([
      "In child",
      "In folder",
    ]);
  });

  it("status filters and search compose", () => {
    let library = libraryWith();
    library = setTriggersEnabled(library, [library.triggers[0].id], true);
    expect(filterTriggers(library, null, "", "enabled")).toHaveLength(1);
    expect(filterTriggers(library, null, "root", "disabled")).toHaveLength(1);
  });

  it("deleting a folder moves its triggers to the root", () => {
    const library = libraryWith();
    const next = removeFolder(library, library.folders[0].id);
    expect(next.folders).toHaveLength(0);
    expect(next.triggers.every((trigger) => trigger.folder === null)).toBe(
      true,
    );
  });

  it("bulk move retargets the folder", () => {
    const library = libraryWith();
    const target = library.folders[1].id;
    const moved = moveTriggers(
      library,
      library.triggers.map((trigger) => trigger.id),
      target,
    );
    expect(moved.triggers.every((trigger) => trigger.folder === target)).toBe(
      true,
    );
  });
});
