import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Toolbar } from "./components/Toolbar";
import type { ViewMode } from "./components/Toolbar";
import { Sidebar } from "./components/Sidebar";
import { EditorView } from "./components/EditorView";
import type { EditorTab } from "./components/EditorView";
import { RightPanel } from "./components/RightPanel";
import { StatusBar } from "./components/StatusBar";
import { DropOverlay } from "./components/DropOverlay";
import {
  ContextMenu,
  type ContextMenuItem,
  type ContextMenuState,
} from "./components/ContextMenu";
import { NewNoteModal } from "./components/Modals/NewNoteModal";
import type { NewNoteResult } from "./components/Modals/NewNoteModal";
import type { NewItemType } from "./components/Modals/NewNoteModal";
import { SettingsModal } from "./components/Modals/SettingsModal";
import { RelationModal } from "./components/Modals/RelationModal";
import { FileDiscoveryModal } from "./components/Modals/FileDiscoveryModal";
import { AiOrganizeModal } from "./components/Modals/AiOrganizeModal";
import {
  CommandPalette,
  makeDefaultActions,
  type CommandAction,
} from "./components/CommandPalette";
import { ToastProvider, useToast } from "./components/Toast";
import { GraphView } from "./components/GraphView";
import { TrashView } from "./components/TrashView";
import { FilesView } from "./components/FilesView";
import type { TreeNode, ObjectMetadata, Relation } from "./mock/data";
import {
  autoLink,
  checkAgentConfig,
  createNote as apiCreateNote,
  createCollection as apiCreateCollection,
  deleteObject as apiDeleteObject,
  deleteGroup as apiDeleteGroup,
  duplicateObject as apiDuplicateObject,
  getFileTree,
  getCollectionTree,
  getObject,
  getStats,
  importFolder as apiImportFolder,
  importFile as apiImportFile,
  importDiscoveryFiles,
  ignoreDiscoveryFiles,
  linkObjects as apiLinkObjects,
  pickFiles,
  pickFolders,
  pickDiscoveryFolders,
  rebuildRelations,
  renameGroup as apiRenameGroup,
  renameObject as apiRenameObject,
  saveObject as apiSaveObject,
  searchObjects,
  scanDiscoveryFolders,
  unlinkObjects as apiUnlinkObjects,
  updateObjectGroup as apiUpdateObjectGroup,
  moveObjectToCollection as apiMoveObjectToCollection,
  type CollectionNode,
  type GroupNode,
  type ObjectDetail,
  type DiscoveryCandidate,
  type AiTaskEvent,
} from "./api";

type Theme = "dark" | "light";

function getInitialTheme(): Theme {
  const saved = localStorage.getItem("kb-theme");
  if (saved === "dark" || saved === "light") return saved;
  if (
    window.matchMedia &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  ) {
    return "dark";
  }
  return "light";
}

// 把 GroupNode[] 转为前端 TreeNode 格式
function groupNodesToTree(groups: GroupNode[]): TreeNode[] {
  return groups.map((g, i) => ({
    id: `g-${i}-${g.name}`,
    name: g.name || "未分组",
    type: "group" as const,
    children: g.objects.map((obj) => ({
      id: obj.id,
      name: obj.title,
      type: "object" as const,
      metadata: {
        objType: (obj.type || "document") as any,
        objFormat: (obj.format || "markdown") as any,
        createdBy: (obj.created_by || "human") as "human" | "agent",
      },
    })),
  }));
}

function collectionNodesToTree(collections: CollectionNode[]): TreeNode[] {
  return collections.map((collection) => ({
    id: collection.id,
    name: collection.name,
    type: "group" as const,
    collectionId: collection.id,
    collectionPath: collection.path,
    children: [
      ...collection.children.map((child) => collectionNodesToTree([child])[0]),
      ...collection.objects.map((obj) => ({
        id: obj.id,
        name: obj.title,
        type: "object" as const,
        metadata: {
          objType: (obj.type || "document") as any,
          objFormat: (obj.format || "markdown") as any,
          createdBy: (obj.created_by || "human") as "human" | "agent",
        },
      })),
    ],
  }));
}

// 把 ObjectDetail 转为 RightPanel 使用的 ObjectMetadata
function detailToMetadata(
  detail: ObjectDetail,
  overriddenTitle?: string
): ObjectMetadata {
  const o = detail.object;
  const iso = (n: number) => new Date(n * 1000).toISOString();
  return {
    id: o.id,
    type: o.type as any,
    format: o.format as any,
    title: overriddenTitle ?? o.title,
    tags: detail.tags,
    group: o.group_name || "未分组",
    createdBy: (o.created_by as any) ?? "human",
    status: o.status as any,
    createdAt: iso(o.created_at),
    updatedAt: iso(o.updated_at),
    fileSize: o.file_size,
    fileHash: o.file_hash ?? "",
    source: o.created_by === "agent" ? "Agent 自动生成" : "手动创建",
  };
}

function detailToRelations(detail: ObjectDetail): Relation[] {
  return detail.relations.map((r) => ({
    targetId: r.target_id,
    targetTitle: r.target_title,
    relationType: r.relation_type as any,
  }));
}

function AppInner() {
  const { toast } = useToast();

  // ===== 主题 =====
  const [theme, setTheme] = useState<Theme>(getInitialTheme);
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("kb-theme", theme);
  }, [theme]);

  // ===== 布局状态 =====
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [rightCollapsed, setRightCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(260);
  const [rightWidth, setRightWidth] = useState(300);

  // ===== 视图 =====
  const [view, setView] = useState<ViewMode>("editor");

  // ===== 文件树 =====
  const [tree, setTree] = useState<TreeNode[]>([]);
  // 保留原始 GroupNode 数据，用于图谱视图
  const [groupNodes, setGroupNodes] = useState<GroupNode[]>([]);
  // 关系版本号：保存/删除/重建关系后递增，触发图谱重新加载
  const [relationVersion, setRelationVersion] = useState(0);

  // ===== 标签页 =====
  const [tabs, setTabs] = useState<EditorTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);

  // ===== 内容/标题缓存 =====
  const [contents, setContents] = useState<Record<string, string>>({});
  const [titles, setTitles] = useState<Record<string, string>>({});
  // 对象元数据 + 关系缓存（按 id）
  const [metaCache, setMetaCache] = useState<Record<string, ObjectMetadata>>({});
  const [relCache, setRelCache] = useState<Record<string, Relation[]>>({});
  // 标记对象是否已加载过内容
  const [loadedIds, setLoadedIds] = useState<Record<string, boolean>>({});
  // 标记未保存
  const [dirty, setDirty] = useState<Record<string, boolean>>({});

  // ===== 弹窗 =====
  const [showNewNote, setShowNewNote] = useState(false);
  const [newItemGroup, setNewItemGroup] = useState("收件箱");
  const [newItemInitialType, setNewItemInitialType] = useState<NewItemType>("concept");
  const [showSettings, setShowSettings] = useState(false);
  const [showCommand, setShowCommand] = useState(false);
  const [showDiscovery, setShowDiscovery] = useState(false);
  const [showAiOrganize, setShowAiOrganize] = useState(false);
  const [aiOrganizeTarget, setAiOrganizeTarget] = useState<{
    collectionId: string;
    collectionPath: string;
  } | null>(null);
  const [discoveryCandidates, setDiscoveryCandidates] = useState<DiscoveryCandidate[]>([]);
  const [discoveryScanning, setDiscoveryScanning] = useState(false);
  const [discoveryFolders, setDiscoveryFolders] = useState<string[]>(() => {
    try {
      const parsed = JSON.parse(localStorage.getItem("kbox-discovery-folders") || "[]");
      return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === "string") : [];
    } catch {
      return [];
    }
  });
  // 关系弹窗
  const [relationModal, setRelationModal] = useState<{
    sourceId: string;
    sourceTitle: string;
  } | null>(null);

  // ===== 设置项 =====
  const [aiEnhanced, setAiEnhanced] = useState(
    () => localStorage.getItem("kb-ai-enhanced") === "true"
  );
  const [agentOnline, setAgentOnline] = useState(false);

  // ===== 右键菜单 =====
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState | null>(null);

  // ===== 拖拽分隔 =====
  const [dragging, setDragging] = useState<null | "left" | "right">(null);

  // ===== 拖拽入库 =====
  const [dragOver, setDragOver] = useState(false);

  // ===== 统计 =====
  const [objectCount, setObjectCount] = useState(0);

  // ===== 刷新文件树 =====
  const refreshTree = useCallback(async () => {
    try {
      const [groups, collections] = await Promise.all([
        getFileTree(),
        getCollectionTree(),
      ]);
      setGroupNodes(groups);
      const collectionTree = collectionNodesToTree(collections);
      const ungrouped = groups.find((group) => group.name === "未分组");
      if (ungrouped?.objects.length) {
        collectionTree.push(...groupNodesToTree([ungrouped]));
      }
      setTree(collectionTree.length ? collectionTree : groupNodesToTree(groups));
    } catch (e: any) {
      toast("error", `加载文件树失败: ${e}`);
    }
  }, [toast]);

  const refreshStats = useCallback(async () => {
    try {
      const stats = await getStats();
      setObjectCount(stats.total_objects);
    } catch (e: any) {
      // 静默失败
    }
  }, []);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    listen<AiTaskEvent>("ai-task-event", (event) => {
      const task = event.payload;
      if (task.kind === "completed") {
        Promise.all([refreshTree(), refreshStats()]).then(() => {
          setRelationVersion((version) => version + 1);
          toast("success", `AI 整理已完成：${task.description}`);
        }).catch(() => undefined);
      } else if (task.kind === "failed") {
        toast("error", `AI 整理失败：${task.description}`);
      } else if (task.kind === "cancelled") {
        refreshTree().catch(() => undefined);
        toast("info", "AI 整理任务已停止");
      }
    }).then((unlisten) => {
      dispose = unlisten;
    });
    return () => dispose?.();
  }, [refreshTree, refreshStats, toast]);

  // ===== 初始化加载 =====
  useEffect(() => {
    refreshTree();
    refreshStats();
    checkAgentConfig().then(setAgentOnline).catch(() => setAgentOnline(false));
  }, [refreshTree, refreshStats]);

  const runDiscoveryScan = useCallback(async (folders = discoveryFolders) => {
    if (folders.length === 0) {
      setDiscoveryCandidates([]);
      return;
    }
    setDiscoveryScanning(true);
    try {
      setDiscoveryCandidates(await scanDiscoveryFolders(folders));
    } catch (error: any) {
      toast("error", `扫描新文件失败: ${error}`);
    } finally {
      setDiscoveryScanning(false);
    }
  }, [discoveryFolders, toast]);

  useEffect(() => {
    if (discoveryFolders.length === 0) return;
    runDiscoveryScan(discoveryFolders);
    const timer = window.setInterval(() => runDiscoveryScan(discoveryFolders), 6 * 60 * 60 * 1000);
    return () => window.clearInterval(timer);
  }, [discoveryFolders, runDiscoveryScan]);

  const addDiscoveryFolders = useCallback(async () => {
    try {
      const picked = await pickDiscoveryFolders();
      if (!picked?.length) return;
      const next = Array.from(new Set([...discoveryFolders, ...picked]));
      setDiscoveryFolders(next);
      localStorage.setItem("kbox-discovery-folders", JSON.stringify(next));
    } catch (error: any) {
      toast("error", `选择扫描文件夹失败: ${error}`);
    }
  }, [discoveryFolders, toast]);

  const removeDiscoveryFolder = useCallback((folder: string) => {
    const next = discoveryFolders.filter((item) => item !== folder);
    setDiscoveryFolders(next);
    localStorage.setItem("kbox-discovery-folders", JSON.stringify(next));
  }, [discoveryFolders]);

  const handleDiscoveryImport = useCallback(async (paths: string[], group: string) => {
    try {
      const count = await importDiscoveryFiles(paths, group);
      toast("success", `已添加 ${count} 个文件${count < paths.length ? `，${paths.length - count} 个内容已存在` : ""}`);
      await Promise.all([refreshTree(), refreshStats(), runDiscoveryScan()]);
    } catch (error: any) {
      toast("error", `添加文件失败: ${error}`);
    }
  }, [refreshTree, refreshStats, runDiscoveryScan, toast]);

  const handleDiscoveryIgnore = useCallback(async (paths: string[]) => {
    try {
      await ignoreDiscoveryFiles(paths);
      toast("info", `已忽略 ${paths.length} 个文件`);
      await runDiscoveryScan();
    } catch (error: any) {
      toast("error", `忽略文件失败: ${error}`);
    }
  }, [runDiscoveryScan, toast]);

  // ===== 当前对象元数据 =====
  const activeMeta: ObjectMetadata | null = useMemo(() => {
    if (!activeTabId) return null;
    const meta = metaCache[activeTabId];
    if (!meta) return null;
    const overriddenTitle = titles[activeTabId];
    return overriddenTitle ? { ...meta, title: overriddenTitle } : meta;
  }, [activeTabId, metaCache, titles]);

  const activeRelations = useMemo(() => {
    if (!activeTabId) return [];
    return relCache[activeTabId] ?? [];
  }, [activeTabId, relCache]);

  // 扁平化所有对象，供关系弹窗使用
  const allObjectsForRelation = useMemo(() => {
    const arr: { id: string; title: string; type: string }[] = [];
    const walk = (nodes: TreeNode[]) => {
      for (const n of nodes) {
        if (n.type === "group") {
          walk(n.children ?? []);
        } else {
          arr.push({
            id: n.id,
            title: n.name,
            type: n.metadata?.objType ?? "document",
          });
        }
      }
    };
    walk(tree);
    return arr;
  }, [tree]);

  const breadcrumb = useMemo(() => {
    if (!activeMeta) return [];
    return [activeMeta.group, activeMeta.title];
  }, [activeMeta]);

  // ===== 加载对象详情（内容 + 元数据） =====
  const loadObject = useCallback(
    async (id: string) => {
      try {
        const detail = await getObject(id);
        setMetaCache((prev) => ({
          ...prev,
          [id]: detailToMetadata(detail),
        }));
        setRelCache((prev) => ({
          ...prev,
          [id]: detailToRelations(detail),
        }));
        setContents((prev) => ({ ...prev, [id]: detail.content }));
        setTitles((prev) => ({ ...prev, [id]: detail.object.title }));
        setLoadedIds((prev) => ({ ...prev, [id]: true }));
        setDirty((prev) => ({ ...prev, [id]: false }));
      } catch (e: any) {
        toast("error", `加载对象失败: ${e}`);
      }
    },
    [toast]
  );

  // ===== 打开对象 =====
  const openObject = useCallback(
    (node: TreeNode) => {
      if (node.type !== "object") return;
      const id = node.id;
      setTabs((prev) => {
        if (prev.some((t) => t.id === id)) return prev;
        return [
          ...prev,
          {
            id,
            title: node.name,
            type: node.metadata?.objType ?? "document",
          },
        ];
      });
      setActiveTabId(id);
      setView("editor");
      // 异步加载详情
      if (!loadedIds[id]) {
        loadObject(id);
      }
    },
    [loadedIds, loadObject]
  );

  const openObjectById = useCallback(
    (id: string) => {
      const flat = (nodes: TreeNode[]): TreeNode | undefined => {
        for (const n of nodes) {
          if (n.id === id) return n;
          if (n.children) {
            const f = flat(n.children);
            if (f) return f;
          }
        }
        return undefined;
      };
      const node = flat(tree);
      if (node) {
        openObject(node);
      } else {
        // 不在树中，仍可打开（通过搜索结果）
        setTabs((prev) => {
          if (prev.some((t) => t.id === id)) return prev;
          return [
            ...prev,
            { id, title: "加载中...", type: "document" },
          ];
        });
        setActiveTabId(id);
        setView("editor");
        if (!loadedIds[id]) loadObject(id);
      }
    },
    [tree, openObject, loadedIds, loadObject]
  );

  // ===== 关闭标签 =====
  const closeTab = useCallback(
    (id: string) => {
      setTabs((prev) => {
        const idx = prev.findIndex((t) => t.id === id);
        if (idx === -1) return prev;
        const next = prev.filter((t) => t.id !== id);
        if (activeTabId === id) {
          const newActive = next[idx] ?? next[idx - 1] ?? null;
          setActiveTabId(newActive ? newActive.id : null);
        }
        return next;
      });
    },
    [activeTabId]
  );

  const closeAllTabs = useCallback(() => {
    setTabs([]);
    setActiveTabId(null);
  }, []);

  const handleNewFolder = useCallback(() => {
    setNewItemGroup("收件箱");
    setNewItemInitialType("folder");
    setShowNewNote(true);
  }, []);

  // ===== 内容/标题修改 =====
  const handleContentChange = useCallback((id: string, content: string) => {
    setContents((prev) => ({ ...prev, [id]: content }));
    setDirty((prev) => ({ ...prev, [id]: true }));
  }, []);

  const handleTitleChange = useCallback(
    (id: string, title: string) => {
      setTitles((prev) => ({ ...prev, [id]: title }));
      setTabs((prev) => prev.map((t) => (t.id === id ? { ...t, title } : t)));
      setDirty((prev) => ({ ...prev, [id]: true }));
    },
    []
  );

  // ===== 保存当前对象 =====
  const saveActiveObject = useCallback(
    async (id: string) => {
      const title = titles[id] ?? "";
      const content = contents[id] ?? "";
      try {
        await apiSaveObject(id, title, content);
        setDirty((prev) => ({ ...prev, [id]: false }));
        // 更新树显示的标题
        setTree((prev) => {
          const mapTitle = (nodes: TreeNode[]): TreeNode[] =>
            nodes.map((n) => {
              if (n.type === "group" && n.children) {
                return { ...n, children: mapTitle(n.children) };
              }
              if (n.id === id) return { ...n, name: title };
              return n;
            });
          return mapTitle(prev);
        });
        toast("success", "已保存");
        // 自动检测关系
        try {
          const count = await autoLink(id);
          if (count > 0) {
            toast("info", `检测到 ${count} 个新关系`);
            // 刷新当前对象的关系缓存
            if (loadedIds[id]) {
              await loadObject(id);
            }
          }
          // 无论是否检测到新关系，都刷新图谱
          setRelationVersion((v) => v + 1);
        } catch {
          // 关系检测失败不影响保存
        }
      } catch (e: any) {
        toast("error", `保存失败: ${e}`);
      }
    },
    [titles, contents, toast, loadedIds, loadObject]
  );

  // ===== 新建笔记 =====
  const handleCreateNote = useCallback(
    async (result: NewNoteResult) => {
      try {
        if (result.type === "folder") {
          const id = await apiCreateCollection(result.title);
          await refreshTree();
          setShowNewNote(false);
          toast("success", `文件夹「${result.title}」已就绪（${id}）`);
          return;
        }
        const obj = await apiCreateNote(result.title, result.type, newItemGroup);
        // 刷新树和统计
        await refreshTree();
        await refreshStats();
        setShowNewNote(false);
        toast("success", `已创建: ${result.title}`);
        // 打开新标签
        setTabs((prev) => [
          ...prev,
          {
            id: obj.id,
            title: obj.title,
            type: obj.type,
          },
        ]);
        setActiveTabId(obj.id);
        setView("editor");
        // 加载初始内容
        loadObject(obj.id);
      } catch (e: any) {
        toast("error", `创建失败: ${e}`);
      }
    },
    [refreshTree, refreshStats, toast, loadObject, newItemGroup]
  );

  const openNewItem = useCallback((groupName = "收件箱") => {
    setNewItemGroup(groupName);
    setNewItemInitialType("concept");
    setShowNewNote(true);
  }, []);

  // ===== 导入文件 =====
  const handleImport = useCallback(async () => {
    let paths: string[] | null;
    try {
      paths = await pickFiles();
    } catch (e: any) {
      toast("error", `打开文件选择对话框失败: ${e}`);
      return;
    }
    if (!paths || paths.length === 0) return;

    let success = 0;
    let failed = 0;
    for (const p of paths) {
      try {
        await apiImportFile(p, undefined);
        success++;
      } catch (e: any) {
        failed++;
        toast("error", `导入失败 ${p}: ${e}`);
      }
    }
    if (success > 0) {
      toast("success", `已导入 ${success} 个文件${failed > 0 ? `（${failed} 个失败）` : ""}`);
      await refreshTree();
      await refreshStats();
    }
  }, [toast, refreshTree, refreshStats]);

  const handleImportFolder = useCallback(async () => {
    let folders: string[] | null;
    try {
      folders = await pickFolders();
    } catch (e: any) {
      toast("error", `打开文件夹选择对话框失败: ${e}`);
      return;
    }
    if (!folders || folders.length === 0) return;

    let imported = 0;
    let skipped = 0;
    let failed = 0;
    for (const folder of folders) {
      try {
        const result = await apiImportFolder(folder, undefined);
        imported += result.imported;
        skipped += result.skipped;
        failed += result.failed;
      } catch (e: any) {
        failed++;
        toast("error", `导入文件夹失败 ${folder}: ${e}`);
      }
    }

    if (imported > 0) {
      toast(
        "success",
        `已导入 ${imported} 个文件${skipped > 0 ? `，跳过 ${skipped} 个重复文件` : ""}${failed > 0 ? `，${failed} 个失败` : ""}`
      );
      await refreshTree();
      await refreshStats();
    } else if (skipped > 0 || failed > 0) {
      toast(
        failed > 0 ? "error" : "info",
        `未导入新文件${skipped > 0 ? `，跳过 ${skipped} 个重复文件` : ""}${failed > 0 ? `，${failed} 个失败` : ""}`
      );
    }
  }, [toast, refreshTree, refreshStats]);

  // ===== 拖拽移动对象到分组 =====
  const handleMoveObject = useCallback(
    async (objectId: string, target: string, label?: string) => {
      try {
        if (target.startsWith("col_")) {
          await apiMoveObjectToCollection(objectId, target);
        } else {
          await apiUpdateObjectGroup(objectId, target);
        }
        await refreshTree();
        await refreshStats();
        toast("success", `已移动到: ${label ?? target}`);
      } catch (e: any) {
        toast("error", `移动失败: ${e}`);
      }
    },
    [refreshTree, refreshStats, toast]
  );

  // ===== 删除关系 =====
  const handleRemoveRelation = useCallback(
    async (targetId: string, relationType: string) => {
      if (!activeTabId) return;
      try {
        await apiUnlinkObjects(activeTabId, targetId, relationType);
        // 重新加载当前对象的关系
        await loadObject(activeTabId);
        toast("success", "已删除关系");
      } catch (e: any) {
        toast("error", `删除关系失败: ${e}`);
      }
    },
    [activeTabId, loadObject, toast]
  );

  // ===== 添加关系确认 =====
  const handleConfirmRelation = useCallback(
    async (targetId: string, relationType: string) => {
      if (!relationModal) return;
      try {
        await apiLinkObjects(relationModal.sourceId, targetId, relationType);
        // 刷新源对象的关系缓存
        if (loadedIds[relationModal.sourceId]) {
          await loadObject(relationModal.sourceId);
        }
        toast("success", "已添加关系");
        setRelationModal(null);
      } catch (e: any) {
        toast("error", `添加关系失败: ${e}`);
      }
    },
    [relationModal, loadedIds, loadObject, toast]
  );

  // ===== 命令面板默认动作 =====
  const commandActions: CommandAction[] = useMemo(
    () =>
      makeDefaultActions({
        onNewNote: () => openNewItem(),
        onImport: handleImport,
        onImportFolder: handleImportFolder,
        onOpenSettings: () => setShowSettings(true),
        onSwitchView: (v) => setView(v),
        onOpenCommand: () => setShowCommand(true),
        onRebuildRelations: async () => {
          try {
            const count = await rebuildRelations();
            setRelationVersion((v) => v + 1);
            toast("success", `已重建 ${count} 个关系`);
          } catch (e: any) {
            toast("error", `重建关系失败: ${e}`);
          }
        },
        onToggleGraphFilter: () => {
          setView("graph");
          setTimeout(() => window.dispatchEvent(new Event("kbox:toggle-graph-filter")), 0);
        },
      }),
    [handleImport, handleImportFolder, toast, openNewItem]
  );

  // ===== 右键菜单动作 =====
  const handleCtxSelect = useCallback(
    async (item: ContextMenuItem) => {
      const id = item.id;
      if (id === "new-note" || id === "new-object") {
        openNewItem(id === "new-object" ? (ctxMenu?.__groupName ?? "收件箱") : "收件箱");
      } else if (id === "import") {
        handleImport();
      } else if (id === "import-folder") {
        handleImportFolder();
      } else if (id === "new-group") {
        await handleNewFolder();
      } else if (id === "ai-organize-collection") {
        const collectionId = ctxMenu?.__collectionId;
        const collectionPath = ctxMenu?.__collectionPath;
        if (collectionId && collectionPath) {
          setAiOrganizeTarget({ collectionId, collectionPath });
          setShowAiOrganize(true);
        }
      } else if (id === "open" || id === "open-new-tab") {
        const targetId = ctxMenu?.__objectId;
        if (targetId) openObjectById(targetId);
      } else if (id === "copy-id") {
        if (ctxMenu?.__objectId) {
          try {
            await navigator.clipboard.writeText(ctxMenu.__objectId);
            toast("success", "已复制对象 ID");
          } catch {
            toast("info", `对象 ID: ${ctxMenu.__objectId}`);
          }
        }
      } else if (id === "delete") {
        const targetId = ctxMenu?.__objectId;
        if (targetId) {
          try {
            await apiDeleteObject(targetId);
            // 关闭相关标签
            setTabs((prev) => prev.filter((t) => t.id !== targetId));
            if (activeTabId === targetId) setActiveTabId(null);
            await refreshTree();
            await refreshStats();
            toast("success", "已删除（软删除）");
          } catch (e: any) {
            toast("error", `删除失败: ${e}`);
          }
        }
      } else if (id === "move-to-group") {
        const targetId = ctxMenu?.__objectId;
        if (targetId) {
          const name = window.prompt("移动到分组（输入分组名）");
          if (name) {
            try {
              await apiUpdateObjectGroup(targetId, name);
              await refreshTree();
              await refreshStats();
              toast("success", `已移动到: ${name}`);
            } catch (e: any) {
              toast("error", `移动失败: ${e}`);
            }
          }
        }
      } else if (id === "rename") {
        const targetId = ctxMenu?.__objectId;
        if (targetId) {
          const currentTitle = titles[targetId] ?? metaCache[targetId]?.title ?? "";
          const name = window.prompt("新标题", currentTitle);
          if (name) {
            try {
              await apiRenameObject(targetId, name);
              setTitles((prev) => ({ ...prev, [targetId]: name }));
              setTabs((prev) =>
                prev.map((t) => (t.id === targetId ? { ...t, title: name } : t))
              );
              await refreshTree();
              toast("success", "已重命名");
            } catch (e: any) {
              toast("error", `重命名失败: ${e}`);
            }
          }
        }
      } else if (id === "duplicate") {
        const targetId = ctxMenu?.__objectId;
        if (targetId) {
          try {
            const copy = await apiDuplicateObject(targetId);
            await refreshTree();
            await refreshStats();
            toast("success", `已创建副本: ${copy.title}`);
          } catch (e: any) {
            toast("error", `创建副本失败: ${e}`);
          }
        }
      } else if (id === "add-relation") {
        const targetId = ctxMenu?.__objectId;
        if (targetId) {
          const title = titles[targetId]
            ?? metaCache[targetId]?.title
            ?? "对象";
          setRelationModal({ sourceId: targetId, sourceTitle: title });
        }
      } else if (id === "rename-group") {
        const oldGroup = ctxMenu?.__groupName;
        if (oldGroup) {
          const newGroup = window.prompt("新分组名", oldGroup)?.trim();
          if (newGroup && newGroup !== oldGroup) {
            try {
              const count = await apiRenameGroup(oldGroup, newGroup);
              await refreshTree();
              toast("success", `已移动 ${count} 个对象到「${newGroup}」`);
            } catch (e: any) {
              toast("error", `重命名分组失败: ${e}`);
            }
          }
        }
      } else if (id === "delete-group") {
        const group = ctxMenu?.__groupName;
        if (group) {
          try {
            const count = await apiDeleteGroup(group);
            await refreshTree();
            toast("success", `已删除分组，${count} 个对象归入未分组`);
          } catch (e: any) {
            toast("error", `删除分组失败: ${e}`);
          }
        }
      } else if (id === "collapse-all") {
        window.dispatchEvent(new Event("kbox:collapse-groups"));
      } else if (id === "refresh") {
        await refreshTree();
        await refreshStats();
        toast("info", "已刷新");
      } else if (id === "rebuild-relations") {
        try {
          const count = await rebuildRelations();
          toast("success", `已重建 ${count} 个关系`);
          // 刷新当前对象的关系缓存
          if (activeTabId && loadedIds[activeTabId]) {
            await loadObject(activeTabId);
          }
          // 触发图谱重新加载
          setRelationVersion((v) => v + 1);
        } catch (e: any) {
          toast("error", `重建失败: ${e}`);
        }
      }
    },
    [handleImport, handleImportFolder, toast, ctxMenu, activeTabId, contents, titles, metaCache, refreshTree, refreshStats, loadedIds, loadObject]
  );

  // ===== 快捷键 =====
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const meta = e.metaKey || e.ctrlKey;
      if (meta && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setShowCommand((v) => !v);
      } else if (meta && e.key.toLowerCase() === "n") {
        e.preventDefault();
        openNewItem();
      } else if (meta && e.key.toLowerCase() === "i") {
        e.preventDefault();
        handleImport();
      } else if (meta && e.key.toLowerCase() === "g" && !e.shiftKey) {
        e.preventDefault();
        setView("graph");
      } else if (meta && e.key.toLowerCase() === "e") {
        e.preventDefault();
        setView("editor");
      } else if (meta && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (activeTabId) saveActiveObject(activeTabId);
      } else if (meta && e.shiftKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setView("search");
      } else if (e.key === "Escape") {
        if (showCommand) setShowCommand(false);
        else if (showNewNote) setShowNewNote(false);
        else if (showSettings) setShowSettings(false);
        else if (relationModal) setRelationModal(null);
        else if (ctxMenu) setCtxMenu(null);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [handleImport, showCommand, showNewNote, showSettings, relationModal, ctxMenu, activeTabId, saveActiveObject, openNewItem]);

  // ===== 拖拽分隔条 =====
  useEffect(() => {
    if (!dragging) return;
    const onMove = (e: MouseEvent) => {
      if (dragging === "left") {
        const w = Math.max(180, Math.min(500, e.clientX));
        setSidebarWidth(w);
      } else if (dragging === "right") {
        const w = Math.max(
          180,
          Math.min(500, window.innerWidth - e.clientX)
        );
        setRightWidth(w);
      }
    };
    const onUp = () => setDragging(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, [dragging]);

  // ===== 拖拽入库 =====
  useEffect(() => {
    const onDragOver = (e: DragEvent) => {
      if (e.dataTransfer?.types.includes("Files")) {
        e.preventDefault();
        setDragOver(true);
      }
    };
    const onDragLeave = (e: DragEvent) => {
      if (e.relatedTarget === null) setDragOver(false);
    };
    const onDrop = async (e: DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const files = e.dataTransfer?.files;
      if (!files || files.length === 0) return;

      // Tauri webview 中拖入的 File 对象可访问 path 属性（仅桌面端）
      let success = 0;
      let failed = 0;
      for (const f of Array.from(files)) {
        // @ts-ignore - Tauri 在 File 对象上注入了 path 字段
        const filePath: string | undefined = f.path;
        if (!filePath) {
          failed++;
          toast("error", `无法获取文件路径: ${f.name}（请用导入按钮选择文件）`);
          continue;
        }
        try {
          await apiImportFile(filePath, undefined);
          success++;
        } catch (err: any) {
          failed++;
          toast("error", `导入失败 ${f.name}: ${err}`);
        }
      }
      if (success > 0) {
        toast(
          "success",
          `已导入 ${success} 个文件${failed > 0 ? `（${failed} 个失败）` : ""}`
        );
        await refreshTree();
        await refreshStats();
      }
    };
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("dragleave", onDragLeave);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("dragleave", onDragLeave);
      window.removeEventListener("drop", onDrop);
    };
  }, [toast, refreshTree, refreshStats]);

  // ===== 当前内容 =====
  const activeContent = activeTabId ? contents[activeTabId] ?? "" : "";
  const activeDirty = activeTabId ? dirty[activeTabId] ?? false : false;

  const handleBatchDelete = useCallback(async (ids: string[]) => {
    const deletedIds: string[] = [];
    for (const id of ids) {
      try {
        // 后端会为每个文件获取同一把知识库写锁，顺序执行可避免批量并发争锁。
        await apiDeleteObject(id);
        deletedIds.push(id);
      } catch {
        // 继续处理其余文件，结束后统一反馈失败数量。
      }
    }
    const failedCount = ids.length - deletedIds.length;

    if (deletedIds.length > 0) {
      const deleted = new Set(deletedIds);
      setTabs((previous) => previous.filter((tab) => !deleted.has(tab.id)));
      if (activeTabId && deleted.has(activeTabId)) setActiveTabId(null);
      await Promise.all([refreshTree(), refreshStats()]);
      setRelationVersion((version) => version + 1);
    }

    if (failedCount > 0) {
      toast("error", `已将 ${deletedIds.length} 个文件移入回收站，${failedCount} 个删除失败`);
      throw new Error("部分文件删除失败");
    }

    toast("success", `已将 ${deletedIds.length} 个文件移入回收站`);
    setView("trash");
  }, [activeTabId, refreshStats, refreshTree, toast]);

  // ===== 渲染主视图 =====
  const renderMainView = () => {
    if (view === "files") {
      return (
        <FilesView
          groups={groupNodes}
          onOpen={(id) => {
            setView("editor");
            openObjectById(id);
          }}
          onDelete={handleBatchDelete}
        />
      );
    }
    if (view === "graph") {
      return (
        <GraphView
          groups={groupNodes}
          relationVersion={relationVersion}
          onSelectObject={(id) => {
            setView("editor");
            openObjectById(id);
          }}
        />
      );
    }
    if (view === "search") {
      return (
        <div
          style={{
            flex: 1,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            color: "var(--text-muted)",
            flexDirection: "column",
            gap: 8,
          }}
        >
          <div style={{ fontSize: 15, color: "var(--text-secondary)", fontWeight: 500 }}>
            搜索视图
          </div>
          <div style={{ fontSize: 12 }}>按 ⌘K 打开命令面板进行搜索</div>
          <button
            className="btn btn-primary"
            style={{ marginTop: 8 }}
            onClick={() => setShowCommand(true)}
          >
            打开命令面板 (⌘K)
          </button>
        </div>
      );
    }
    if (view === "trash") {
      return (
        <TrashView
          onRefresh={() => { refreshTree(); refreshStats(); }}
          onNotify={toast}
        />
      );
    }
    return (
      <EditorView
        tabs={tabs}
        activeTabId={activeTabId}
        content={activeContent}
        metadata={activeMeta}
        dirty={activeDirty}
        onActivateTab={setActiveTabId}
        onCloseTab={closeTab}
        onCloseAllTabs={closeAllTabs}
        onContentChange={handleContentChange}
        onTitleChange={handleTitleChange}
        onSave={saveActiveObject}
        allObjects={allObjectsForRelation}
      />
    );
  };

  return (
    <div className="app">
      <Toolbar
        view={view}
        theme={theme}
        breadcrumb={breadcrumb}
        onViewChange={setView}
        onNewNote={() => openNewItem()}
        onImport={handleImport}
        onImportFolder={handleImportFolder}
        onToggleTheme={() =>
          setTheme((t) => (t === "dark" ? "light" : "dark"))
        }
        onOpenSettings={() => setShowSettings(true)}
        discoveryCount={discoveryCandidates.length}
        discoveryConfigured={discoveryFolders.length > 0}
        onOpenDiscovery={() => setShowDiscovery(true)}
      />

      <div className="app-body">
        {!sidebarCollapsed && (
          <div style={{ width: sidebarWidth, flexShrink: 0, display: "flex" }}>
            <Sidebar
              tree={tree}
              activeId={activeTabId}
              collapsed={sidebarCollapsed}
              onToggleCollapse={() => setSidebarCollapsed(true)}
              onSelectObject={openObject}
              onContextMenu={setCtxMenu}
              onOpenCommand={() => setShowCommand(true)}
              onNewNote={() => openNewItem()}
              onImport={handleImport}
              onNewFolder={handleNewFolder}
              onAiOrganize={() => {
                setAiOrganizeTarget(null);
                setShowAiOrganize(true);
              }}
              onNewObjectInFolder={openNewItem}
              onMoveObject={handleMoveObject}
            />
          </div>
        )}
        {sidebarCollapsed && (
          <Sidebar
            tree={tree}
            activeId={activeTabId}
            collapsed={sidebarCollapsed}
            onToggleCollapse={() => setSidebarCollapsed(false)}
            onSelectObject={openObject}
            onContextMenu={setCtxMenu}
            onOpenCommand={() => setShowCommand(true)}
            onNewNote={() => openNewItem()}
            onImport={handleImport}
            onNewFolder={handleNewFolder}
            onAiOrganize={() => {
              setAiOrganizeTarget(null);
              setShowAiOrganize(true);
            }}
            onNewObjectInFolder={openNewItem}
            onMoveObject={handleMoveObject}
          />
        )}

        {!sidebarCollapsed && (
          <div
            className={`resizer${dragging === "left" ? " dragging" : ""}`}
            onMouseDown={() => setDragging("left")}
          />
        )}

        <div className="main-content">{renderMainView()}</div>

        {!rightCollapsed && (
          <div
            className={`resizer${dragging === "right" ? " dragging" : ""}`}
            onMouseDown={() => setDragging("right")}
          />
        )}

        {!rightCollapsed && (
          <div style={{ width: rightWidth, flexShrink: 0, display: "flex" }}>
            <RightPanel
              collapsed={rightCollapsed}
              metadata={activeMeta}
              relations={activeRelations}
              onToggleCollapse={() => setRightCollapsed(true)}
              onRemoveRelation={handleRemoveRelation}
            />
          </div>
        )}
        {rightCollapsed && (
          <RightPanel
            collapsed={rightCollapsed}
            metadata={activeMeta}
            relations={activeRelations}
            onToggleCollapse={() => setRightCollapsed(false)}
            onRemoveRelation={handleRemoveRelation}
          />
        )}
      </div>

      <StatusBar
        objectCount={objectCount}
        agentOnline={agentOnline}
        activeObjectTitle={activeMeta?.title ?? null}
      />

      {/* 右键菜单 */}
      <ContextMenu
        state={ctxMenu}
        onClose={() => setCtxMenu(null)}
        onSelect={handleCtxSelect}
      />

      {/* 弹窗 */}
      {showNewNote && (
        <NewNoteModal
          onClose={() => setShowNewNote(false)}
          onConfirm={handleCreateNote}
          targetGroup={newItemGroup}
          initialType={newItemInitialType}
        />
      )}
      {showSettings && (
        <SettingsModal
          onClose={() => setShowSettings(false)}
          aiEnhanced={aiEnhanced}
          onToggleAI={(v) => {
            setAiEnhanced(v);
            localStorage.setItem("kb-ai-enhanced", String(v));
            toast("info", `AI 增强已${v ? "开启" : "关闭"}`);
          }}
          onRedetectAgent={async () => {
            const connected = await checkAgentConfig().catch(() => false);
            setAgentOnline(connected);
            toast(connected ? "success" : "info", connected ? "已检测到 Agent 配置" : "未检测到 KBOX Agent 配置");
            return connected;
          }}
          onRefreshTree={refreshTree}
        />
      )}
      {showDiscovery && (
        <FileDiscoveryModal
          candidates={discoveryCandidates}
          folders={discoveryFolders}
          groups={groupNodes.map((group) => group.name)}
          scanning={discoveryScanning}
          onAddFolders={addDiscoveryFolders}
          onRemoveFolder={removeDiscoveryFolder}
          onScan={() => runDiscoveryScan()}
          onImport={handleDiscoveryImport}
          onIgnore={handleDiscoveryIgnore}
          onClose={() => setShowDiscovery(false)}
        />
      )}
      {showAiOrganize && (
        <AiOrganizeModal
          onClose={() => setShowAiOrganize(false)}
          collectionId={aiOrganizeTarget?.collectionId}
          collectionPath={aiOrganizeTarget?.collectionPath}
        />
      )}
      {showCommand && (
        <CommandPalette
          tree={tree}
          onClose={() => setShowCommand(false)}
          onAction={(a) => a.run()}
          onSelectObject={openObjectById}
          actions={commandActions}
          onSearch={searchObjects}
        />
      )}

      {/* 添加关系弹窗 */}
      {relationModal && (
        <RelationModal
          sourceId={relationModal.sourceId}
          sourceTitle={relationModal.sourceTitle}
          objects={allObjectsForRelation}
          onConfirm={handleConfirmRelation}
          onClose={() => setRelationModal(null)}
        />
      )}

      {/* 拖拽遮罩 */}
      {dragOver && <DropOverlay />}
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <AppInner />
    </ToastProvider>
  );
}
