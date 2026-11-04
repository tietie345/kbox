import { IconChevronLeft, IconPlus, IconSearch, IconUpload } from "./Icons";
import { FileTree } from "./FileTree";
import type { ContextMenuState } from "./ContextMenu";
import type { TreeNode } from "../mock/data";

interface SidebarProps {
  tree: TreeNode[];
  activeId: string | null;
  collapsed: boolean;
  onToggleCollapse: () => void;
  onSelectObject: (node: TreeNode) => void;
  onContextMenu: (state: ContextMenuState) => void;
  onOpenCommand: () => void;
  onNewNote: () => void;
  onImport: () => void;
  onNewObjectInFolder: (groupName: string, collectionId?: string) => void;
  onMoveObject?: (objectId: string, target: string, label?: string) => void;
  shadowNoteMap?: Record<string, boolean>;
}

export function Sidebar({
  tree,
  activeId,
  collapsed,
  onToggleCollapse,
  onSelectObject,
  onContextMenu,
  onOpenCommand,
  onNewNote,
  onImport,
  onNewObjectInFolder,
  onMoveObject,
  shadowNoteMap,
}: SidebarProps) {
  if (collapsed) {
    return (
      <div
        style={{
          width: 32,
          background: "var(--bg-secondary)",
          borderRight: "1px solid var(--border)",
          display: "flex",
          alignItems: "flex-start",
          justifyContent: "center",
          paddingTop: 8,
          flexShrink: 0,
        }}
      >
        <button
          className="icon-btn"
          title="展开侧边栏"
          onClick={onToggleCollapse}
        >
          <IconChevronLeft size={16} style={{ transform: "rotate(180deg)" }} />
        </button>
      </div>
    );
  }

  return (
    <div className="sidebar">
      <div className="sidebar-header">
        <div className="kb-title">
          <img className="kb-title-logo" src="/kbox-icon.png" alt="" />
          <span>KBOX</span>
        </div>
        <button
          className="icon-btn"
          title="折叠侧边栏"
          onClick={onToggleCollapse}
        >
          <IconChevronLeft size={16} />
        </button>
      </div>

      <div className="sidebar-search">
        <div className="search-input-wrap">
          <span className="search-icon">
            <IconSearch size={12} />
          </span>
          <input
            className="search-input"
            placeholder="搜索对象..."
            readOnly
            onClick={onOpenCommand}
          />
          <span className="search-kbd">⌘K</span>
        </div>
      </div>

      <div className="sidebar-actions">
        <button className="action-btn" onClick={onNewNote} title="新建笔记 (⌘N)">
          <IconPlus size={13} />
          新建
        </button>
        <button className="action-btn" onClick={onImport} title="导入文件 (⌘I)">
          <IconUpload size={13} />
          导入
        </button>
      </div>

      <FileTree
        tree={tree}
        activeId={activeId}
        onSelectObject={onSelectObject}
        onContextMenu={onContextMenu}
        onMoveObject={onMoveObject}
        onNewObjectInFolder={onNewObjectInFolder}
        shadowNoteMap={shadowNoteMap}
      />
    </div>
  );
}
