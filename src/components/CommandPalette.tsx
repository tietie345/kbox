import { useEffect, useMemo, useRef, useState } from "react";
import {
  IconEditor,
  IconFile,
  IconFolderPlus,
  IconGraph,
  IconHash,
  IconLink,
  IconPlus,
  IconSearch,
  IconSettings,
  IconTrash,
  IconUpload,
  objTypeMeta,
} from "./Icons";
import type { TreeNode } from "../mock/data";

export interface CommandAction {
  id: string;
  label: string;
  hint?: string;
  icon: React.ReactNode;
  run: () => void;
}

interface SearchItem {
  id: string;
  name: string;
  group: string;
  type: string;
  snippet?: string;
}

interface CommandPaletteProps {
  tree: TreeNode[];
  onClose: () => void;
  onAction: (action: CommandAction) => void;
  onSelectObject: (id: string) => void;
  actions: CommandAction[];
  onSearch?: (query: string, limit?: number) => Promise<
    Array<{
      id: string;
      title: string;
      type: string;
      format: string;
      score: number;
      snippet: string;
    }>
  >;
}

export function CommandPalette({
  tree,
  onClose,
  onAction,
  onSelectObject,
  actions,
  onSearch,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const [searching, setSearching] = useState(false);
  const [searchResults, setSearchResults] = useState<SearchItem[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // 当无搜索词时，从树扁平化得到对象列表；有搜索词时优先用搜索结果
  const allObjects = useMemo(() => {
    const flat: SearchItem[] = [];
    const walk = (nodes: TreeNode[], group: string) => {
      for (const n of nodes) {
        if (n.type === "group") {
          walk(n.children ?? [], n.name);
        } else {
          flat.push({
            id: n.id,
            name: n.name,
            group,
            type: n.metadata?.objType ?? "document",
          });
        }
      }
    };
    walk(tree, "");
    return flat;
  }, [tree]);

  // 搜索防抖
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!query.trim() || !onSearch) {
      setSearchResults([]);
      setSearching(false);
      return;
    }
    setSearching(true);
    debounceRef.current = setTimeout(async () => {
      try {
        const results = await onSearch(query, 50);
        setSearchResults(
          results.map((r) => ({
            id: r.id,
            name: r.title,
            group: "",
            type: r.type || "document",
            snippet: r.snippet,
          }))
        );
      } catch {
        setSearchResults([]);
      } finally {
        setSearching(false);
      }
    }, 200);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [query, onSearch]);

  const filteredActions = useMemo(() => {
    if (!query) return actions;
    const q = query.toLowerCase();
    return actions.filter((a) => a.label.toLowerCase().includes(q));
  }, [query, actions]);

  const filteredObjects = useMemo<SearchItem[]>(() => {
    if (!query) return allObjects;
    // 优先使用搜索结果（如果有），否则本地过滤
    if (onSearch && searchResults.length > 0) {
      return searchResults;
    }
    if (onSearch) {
      // 远程搜索无结果，本地不显示
      return [];
    }
    const q = query.toLowerCase();
    return allObjects.filter(
      (o) =>
        o.name.toLowerCase().includes(q) || o.group.toLowerCase().includes(q)
    );
  }, [query, allObjects, searchResults, onSearch]);

  type FlatItem =
    | { kind: "action"; data: CommandAction }
    | { kind: "object"; data: SearchItem };

  const flatItems: FlatItem[] = useMemo(() => {
    const items: FlatItem[] = [];
    filteredActions.forEach((a) => items.push({ kind: "action", data: a }));
    filteredObjects.forEach((o) => items.push({ kind: "object", data: o }));
    return items;
  }, [filteredActions, filteredObjects]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query, searchResults]);

  const handleKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((i) => Math.min(i + 1, flatItems.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = flatItems[activeIndex];
      if (item) {
        if (item.kind === "action") {
          onAction(item.data);
        } else {
          onSelectObject(item.data.id);
        }
        onClose();
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  const objIcon = (type: string) => {
    const meta = objTypeMeta[type];
    const Icon = meta?.icon ?? IconFile;
    const color = meta?.color ?? "var(--text-muted)";
    return <Icon size={14} style={{ color }} />;
  };

  let actionIdx = 0;
  let objStartIdx = filteredActions.length;

  return (
    <div className="modal-overlay" style={{ alignItems: "flex-start" }}>
      <div
        className="cmd-palette"
        onClick={(e) => {
          if (e.target === e.currentTarget) onClose();
        }}
      >
        <div className="cmd-input-wrap">
          <IconSearch size={14} style={{ color: "var(--text-muted)" }} />
          <input
            ref={inputRef}
            className="cmd-input"
            placeholder="搜索对象或执行命令..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKey}
          />
        </div>
        <div className="cmd-results">
          {flatItems.length === 0 && (
            <div className="cmd-empty">没有匹配的结果</div>
          )}
          {filteredActions.length > 0 && (
            <>
              <div className="cmd-section">命令</div>
              {filteredActions.map((a) => {
                const idx = actionIdx++;
                const flatIdx = idx;
                return (
                  <div
                    key={a.id}
                    className={`cmd-item${flatIdx === activeIndex ? " active" : ""}`}
                    onMouseEnter={() => setActiveIndex(flatIdx)}
                    onClick={() => {
                      onAction(a);
                      onClose();
                    }}
                  >
                    <span className="cmd-item-icon">{a.icon}</span>
                    <span>{a.label}</span>
                    {a.hint && <span className="cmd-item-meta">{a.hint}</span>}
                  </div>
                );
              })}
            </>
          )}
          {searching && (
            <div className="cmd-empty">搜索中...</div>
          )}
          {!searching && filteredObjects.length > 0 && (
            <>
              <div className="cmd-section">对象</div>
              {filteredObjects.map((o, i) => {
                const flatIdx = objStartIdx + i;
                return (
                  <div
                    key={o.id}
                    className={`cmd-item${flatIdx === activeIndex ? " active" : ""}`}
                    onMouseEnter={() => setActiveIndex(flatIdx)}
                    onClick={() => {
                      onSelectObject(o.id);
                      onClose();
                    }}
                  >
                    <span className="cmd-item-icon">{objIcon(o.type)}</span>
                    <span style={{ flex: 1, overflow: "hidden" }}>
                      <div style={{ whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{o.name}</div>
                      {o.snippet && (
                        <div
                          className="cmd-item-snippet"
                          style={{
                            fontSize: 11,
                            color: "var(--text-muted)",
                            whiteSpace: "nowrap",
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                          }}
                        >
                          {o.snippet.replace(/<\/?mark>/g, "")}
                        </div>
                      )}
                    </span>
                    {o.group && <span className="cmd-item-meta">{o.group}</span>}
                  </div>
                );
              })}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

// 默认命令动作工厂（在 App 中传入）
export function makeDefaultActions(params: {
  onNewNote: () => void;
  onImport: () => void;
  onImportFolder: () => void;
  onOpenSettings: () => void;
  onSwitchView: (v: "editor" | "graph" | "search" | "trash") => void;
  onOpenCommand: () => void;
  onRebuildRelations?: () => void;
  onToggleGraphFilter?: () => void;
}): CommandAction[] {
  return [
    {
      id: "new-note",
      label: "新建笔记",
      hint: "⌘N",
      icon: <IconPlus size={14} />,
      run: params.onNewNote,
    },
    {
      id: "import",
      label: "导入文件",
      hint: "⌘I",
      icon: <IconUpload size={14} />,
      run: params.onImport,
    },
    {
      id: "import-folder",
      label: "导入文件夹",
      icon: <IconFolderPlus size={14} />,
      run: params.onImportFolder,
    },
    {
      id: "view-editor",
      label: "切换到编辑器视图",
      hint: "⌘E",
      icon: <IconEditor size={14} />,
      run: () => params.onSwitchView("editor"),
    },
    {
      id: "view-graph",
      label: "切换到图谱视图",
      hint: "⌘G",
      icon: <IconGraph size={14} />,
      run: () => params.onSwitchView("graph"),
    },
    {
      id: "view-search",
      label: "切换到搜索视图",
      hint: "⌘⇧F",
      icon: <IconSearch size={14} />,
      run: () => params.onSwitchView("search"),
    },
    {
      id: "view-trash",
      label: "切换到回收站视图",
      icon: <IconTrash size={14} />,
      run: () => params.onSwitchView("trash"),
    },
    {
      id: "rebuild-relations",
      label: "重建所有关系",
      icon: <IconLink size={14} />,
      run: params.onRebuildRelations ?? (() => {}),
    },
    {
      id: "toggle-graph-filter",
      label: "切换图谱过滤器",
      icon: <IconHash size={14} />,
      run: params.onToggleGraphFilter ?? (() => {}),
    },
    {
      id: "settings",
      label: "打开设置",
      icon: <IconSettings size={14} />,
      run: params.onOpenSettings,
    },
  ];
}
