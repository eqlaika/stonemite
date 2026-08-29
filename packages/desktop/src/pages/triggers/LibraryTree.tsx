import { Folder as FolderIcon, Pencil, Trash2 } from "lucide-react";
import { useState } from "react";

import { folderTree, type FolderNode } from "../../triggers/model";
import type { TriggerLibrary } from "../../triggers/types";

export function LibraryTree({
  library,
  selected,
  onSelect,
  onRename,
  onDelete,
}: {
  library: TriggerLibrary;
  selected: string | null;
  onSelect: (id: string | null) => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string) => void;
}) {
  const roots = folderTree(library);
  const triggerCount = (folderId: string | null) =>
    library.triggers.filter((trigger) => trigger.folder === folderId).length;

  return (
    <nav className="tw-folder-tree">
      <ul role="tree" aria-label="Folders">
        <li role="treeitem" aria-selected={selected === null}>
          <button
            type="button"
            className={selected === null ? "tw-folder active" : "tw-folder"}
            onClick={() => onSelect(null)}
          >
            <FolderIcon size={14} aria-hidden="true" />
            <span>All triggers</span>
            <span className="tw-count">{library.triggers.length}</span>
          </button>
        </li>
        {roots.map((node) => (
          <TreeNode
            key={node.folder.id}
            node={node}
            selected={selected}
            triggerCount={triggerCount}
            onSelect={onSelect}
            onRename={onRename}
            onDelete={onDelete}
          />
        ))}
      </ul>
    </nav>
  );
}

function TreeNode({
  node,
  selected,
  triggerCount,
  onSelect,
  onRename,
  onDelete,
}: {
  node: FolderNode;
  selected: string | null;
  triggerCount: (folderId: string | null) => number;
  onSelect: (id: string | null) => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string) => void;
}) {
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(node.folder.name);
  const active = selected === node.folder.id;

  return (
    <li role="treeitem" aria-selected={active}>
      <div
        className={active ? "tw-folder-row active" : "tw-folder-row"}
        style={{ paddingLeft: `${node.depth * 14}px` }}
      >
        {renaming ? (
          <input
            autoFocus
            value={draft}
            aria-label={`Rename folder ${node.folder.name}`}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={() => {
              setRenaming(false);
              if (draft.trim()) onRename(node.folder.id, draft.trim());
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
              if (event.key === "Escape") {
                setDraft(node.folder.name);
                setRenaming(false);
              }
            }}
          />
        ) : (
          <button
            type="button"
            className={active ? "tw-folder active" : "tw-folder"}
            onClick={() => onSelect(node.folder.id)}
          >
            <FolderIcon size={14} aria-hidden="true" />
            <span>{node.folder.name}</span>
            <span className="tw-count">{triggerCount(node.folder.id)}</span>
          </button>
        )}
        <span className="tw-folder-actions">
          <button
            type="button"
            aria-label={`Rename ${node.folder.name}`}
            onClick={() => setRenaming(true)}
          >
            <Pencil size={13} aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label={`Delete ${node.folder.name}`}
            onClick={() => onDelete(node.folder.id)}
          >
            <Trash2 size={13} aria-hidden="true" />
          </button>
        </span>
      </div>
      {node.children.length > 0 ? (
        <ul role="group">
          {node.children.map((child) => (
            <TreeNode
              key={child.folder.id}
              node={child}
              selected={selected}
              triggerCount={triggerCount}
              onSelect={onSelect}
              onRename={onRename}
              onDelete={onDelete}
            />
          ))}
        </ul>
      ) : null}
    </li>
  );
}
