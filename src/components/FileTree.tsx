import { useEffect, useState } from "react";
import type { TreeNode } from "../mock/data";
import {
  IconChevronRight,
  IconFolder,
  IconPlus,
  objTypeMeta,
} from "./Icons";
import type { ContextMenuState } from "./ContextMenu";

interface FileTreeProps {
  tree: TreeNode[];
  activeId: string | null;
  onSelectObject: (node: TreeNode) => void;
  onContextMenu: (state: ContextMenuState) => void;
  onMoveObject?: (objectId: string, target: string, label?: string) => void;
  onNewObjectInFolder?: (groupName: string, collectionId?: string) => void;
  // 影子笔记（概念笔记）
  shadowNoteMap?: Record<string, boolean>;
  onGenerateConceptNote?: (objectId: string) => void;
}

export function FileTree({
  tree,
  activeId,
  onSelectObject,
  onContextMenu,
  onMoveObject,
  onNewObjectInFolder,
  shadowNoteMap,
}: FileTreeProps) {
  // 默认所有分组展开
  const [expanded, setExpanded] = useState<Record<string, boolean>>(() => {
    const init: Record<string, boolean> = {};
    tree.forEach((n) => {
      if (n.type === "group") init[n.id] = true;
    });
    return init;
  });
  // 拖拽目标分组高亮
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  useEffect(() => {
    const collapse = () => setExpanded(
      Object.fromEntries(tree.filter((node) => node.type === "group").map((node) => [node.id, false]))
    );
    window.addEventListener("kbox:collapse-groups", collapse);
    return () => window.removeEventListener("kbox:collapse-groups", collapse);
  }, [tree]);

  const toggle = (id: string) => {
    setExpanded((prev) => ({ ...prev, [id]: !prev[id] }));
  };

  const renderNode = (node: TreeNode, depth = 0) => {
    if (node.type === "group") {
      const isOpen = expanded[node.id] ?? true;
      const childCount = node.children?.length ?? 0;
      const isDropTarget = dragOverId === node.id;
      return (
        <div key={node.id} className="tree-node tree-group">
          <div
            className="tree-row"
            onClick={() => toggle(node.id)}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              onContextMenu({
                x: e.clientX,
                y: e.clientY,
                __groupName: node.collectionPath ?? node.name,
                __collectionId: node.collectionId,
                __collectionPath: node.collectionPath ?? node.name,
                items: [
                  {
                    id: "new-object",
                    label: "在此分组新建对象",
                  },
                  {
                    id: "new-subfolder",
                    label: "新建子文件夹",
                    disabled: !node.collectionId,
                  },
                  {
                    id: "ai-organize-collection",
                    label: "使用 AI 整理该目录",
                    disabled: !node.collectionId,
                  },
                  {
                    id: "rename-group",
                    label: "重命名分组",
                  },
                  {
                    id: "sep1",
                    separator: true,
                  },
                  {
                    id: "collapse-all",
                    label: "折叠所有分组",
                  },
                  {
                    id: "sep2",
                    separator: true,
                  },
                  {
                    id: "delete-group",
                    label: "删除分组",
                    danger: true,
                  },
                ],
              });
            }}
            onDragOver={(e) => {
              if (!onMoveObject) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              if (dragOverId !== node.id) setDragOverId(node.id);
            }}
            onDragLeave={(e) => {
              // 只在离开该元素时清除高亮
              if (e.currentTarget === e.target) setDragOverId(null);
            }}
            onDrop={(e) => {
              if (!onMoveObject) return;
              e.preventDefault();
              const objectId = e.dataTransfer.getData("text/plain");
              setDragOverId(null);
              if (objectId) {
                onMoveObject(
                  objectId,
                  node.collectionId ?? node.name,
                  node.collectionPath ?? node.name
                );
              }
            }}
            style={
              isDropTarget
                ? { background: "var(--accent-soft)" }
                : undefined
            }
          >
            <span className={`tree-chevron${isOpen ? " open" : ""}`}>
              <IconChevronRight size={12} />
            </span>
            <span className="tree-icon" style={{ color: "var(--text-secondary)" }}>
              <IconFolder size={14} />
            </span>
            <span className="tree-name">{node.name}</span>
            <span className="tree-count">{childCount}</span>
            {onNewObjectInFolder && (
              <button
                className="tree-folder-add"
                title={`在「${node.name}」中新建`}
                onClick={(e) => {
                  e.stopPropagation();
                  onNewObjectInFolder(node.collectionPath ?? node.name, node.collectionId);
                }}
              >
                <IconPlus size={12} />
              </button>
            )}
          </div>
          {isOpen && node.children && (
            <div className="tree-children">
              {node.children.map((child) => renderNode(child, depth + 1))}
            </div>
          )}
        </div>
      );
    }

    // object
    const meta = node.metadata;
    const typeIcon = meta ? objTypeMeta[meta.objType] : undefined;
    const Icon = typeIcon?.icon ?? IconFolder;
    const color = typeIcon?.color ?? "var(--text-secondary)";
    const isActive = activeId === node.id;

    return (
      <div key={node.id} className="tree-node">
        <div
            className={`tree-row${isActive ? " active" : ""}`}
            draggable={!!onMoveObject}
            onDragStart={(e) => {
              e.dataTransfer.setData("text/plain", node.id);
              e.dataTransfer.effectAllowed = "move";
            }}
            onDragEnd={() => setDragOverId(null)}
            onClick={() => onSelectObject(node)}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              onContextMenu({
                x: e.clientX,
                y: e.clientY,
                __objectId: node.id,
                items: [
                  {
                    id: "open",
                    label: "打开",
                    shortcut: "Enter",
                  },
                  {
                    id: "open-new-tab",
                    label: "在新标签页打开",
                  },
                  {
                    id: "sep1",
                    separator: true,
                  },
                  {
                    id: "rename",
                    label: "重命名",
                  },
                  {
                    id: "duplicate",
                    label: "创建副本",
                  },
                  {
                    id: "copy-id",
                    label: "复制对象 ID",
                  },
                  {
                    id: "sep2",
                    separator: true,
                  },
                  {
                    id: "generate-concept-note",
                    label: "生成影子笔记",
                    icon: "◈",
                    disabled: shadowNoteMap?.[node.id] === true,
                  },
                  {
                    id: "add-relation",
                    label: "添加关系",
                  },
                  {
                    id: "move-to-group",
                    label: "移动到分组",
                  },
                  {
                    id: "sep3",
                    separator: true,
                  },
                  {
                    id: "delete",
                    label: "删除",
                    danger: true,
                  },
                ],
              });
            }}
          >
          <span className="tree-chevron invisible">
            <IconChevronRight size={12} />
          </span>
          <span className="tree-icon" style={{ color }}>
            <Icon size={14} />
          </span>
          <span className="tree-name">{node.name}</span>
          {meta?.createdBy === "agent" && (
            <span
              className="tree-count"
              title="由 Agent 创建"
              style={{ color: "var(--green)" }}
            >
              ●
            </span>
          )}
          {shadowNoteMap?.[node.id] === true && (
            <span
              className="tree-count shadow-note-mark"
              title="已生成影子笔记"
              style={{ color: "var(--accent)" }}
              aria-hidden="true"
            >
              ◈
            </span>
          )}
        </div>
      </div>
    );
  };

  if (tree.length === 0) {
    return (
      <div
        className="file-tree"
        onContextMenu={(e) => {
          e.preventDefault();
          onContextMenu({
            x: e.clientX,
            y: e.clientY,
            items: [
              { id: "new-note", label: "新建笔记", shortcut: "⌘N" },
              { id: "import", label: "导入文件", shortcut: "⌘I" },
              { id: "import-folder", label: "导入文件夹" },
              { id: "sep1", separator: true },
              { id: "new-group", label: "新建文件夹" },
              { id: "refresh", label: "刷新" },
              { id: "sep2", separator: true },
              { id: "rebuild-relations", label: "重建所有关系", icon: "🔗" },
            ],
          });
        }}
      >
        <div
          style={{
            padding: "32px 12px",
            textAlign: "center",
            color: "var(--text-muted)",
            fontSize: 12,
          }}
        >
          暂无对象，点击新建或导入
        </div>
      </div>
    );
  }

  return (
    <div
      className="file-tree"
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu({
          x: e.clientX,
          y: e.clientY,
          items: [
            { id: "new-note", label: "新建笔记", shortcut: "⌘N" },
            { id: "import", label: "导入文件", shortcut: "⌘I" },
            { id: "import-folder", label: "导入文件夹" },
            { id: "sep1", separator: true },
            { id: "new-group", label: "新建文件夹" },
            { id: "refresh", label: "刷新" },
            { id: "sep2", separator: true },
            { id: "rebuild-relations", label: "重建所有关系", icon: "🔗" },
          ],
        });
      }}
    >
      {tree.map((node) => renderNode(node))}
    </div>
  );
}
