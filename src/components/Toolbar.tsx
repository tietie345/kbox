import { IconEditor, IconFile, IconFolderPlus, IconGraph, IconMoon, IconPlus, IconRadar, IconSearch, IconSettings, IconSun, IconTrash, IconUpload } from "./Icons";

export type ViewMode = "editor" | "files" | "graph" | "search" | "trash";

interface ToolbarProps {
  view: ViewMode;
  theme: "dark" | "light";
  breadcrumb: string[];
  onViewChange: (v: ViewMode) => void;
  onNewNote: () => void;
  onImport: () => void;
  onImportFolder: () => void;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  discoveryCount: number;
  discoveryConfigured: boolean;
  onOpenDiscovery: () => void;
}

export function Toolbar({
  view,
  theme,
  breadcrumb,
  onViewChange,
  onNewNote,
  onImport,
  onImportFolder,
  onToggleTheme,
  onOpenSettings,
  discoveryCount,
  discoveryConfigured,
  onOpenDiscovery,
}: ToolbarProps) {
  return (
    <div className="toolbar">
      <div className="toolbar-left">
        <div className="view-switcher">
          <button
            className={view === "editor" ? "active" : ""}
            onClick={() => onViewChange("editor")}
            title="编辑器视图 (⌘E)"
          >
            <IconEditor size={13} />
            编辑器
          </button>
          <button
            className={view === "files" ? "active" : ""}
            onClick={() => onViewChange("files")}
            title="全部文件"
          >
            <IconFile size={13} />
            文件
          </button>
          <button
            className={view === "graph" ? "active" : ""}
            onClick={() => onViewChange("graph")}
            title="图谱视图 (⌘G)"
          >
            <IconGraph size={13} />
            图谱
          </button>
          <button
            className={view === "search" ? "active" : ""}
            onClick={() => onViewChange("search")}
            title="搜索视图 (⌘⇧F)"
          >
            <IconSearch size={13} />
            搜索
          </button>
          <button
            className={view === "trash" ? "active" : ""}
            onClick={() => onViewChange("trash")}
            title="回收站"
          >
            <IconTrash size={13} />
            回收站
          </button>
        </div>
      </div>

      <div className="toolbar-center">
        {breadcrumb.length > 0 ? (
          <div className="breadcrumb">
            {breadcrumb.map((seg, i) => (
              <span
                key={i}
                style={{ display: "inline-flex", alignItems: "center", gap: 4 }}
              >
                <span
                  className={`breadcrumb-item${
                    i === breadcrumb.length - 1 ? " current" : ""
                  }`}
                >
                  {seg}
                </span>
                {i < breadcrumb.length - 1 && (
                  <span className="breadcrumb-sep">/</span>
                )}
              </span>
            ))}
          </div>
        ) : (
          <div className="breadcrumb">
            <span className="breadcrumb-item" style={{ color: "var(--text-muted)" }}>
              KBOX
            </span>
          </div>
        )}
      </div>

      <div className="toolbar-right">
        <button
          className={`icon-btn discovery-trigger${discoveryCount > 0 ? " has-items" : ""}`}
          title={discoveryConfigured ? `文件发现${discoveryCount > 0 ? `：${discoveryCount} 项待处理` : ""}` : "设置文件发现"}
          onClick={onOpenDiscovery}
        >
          <IconRadar size={15} />
          {discoveryCount > 0 && <span className="discovery-badge">{discoveryCount > 99 ? "99+" : discoveryCount}</span>}
        </button>
        <button className="icon-btn" title="新建笔记 (⌘N)" onClick={onNewNote}>
          <IconPlus size={15} />
        </button>
        <button className="icon-btn" title="导入文件 (⌘I)" onClick={onImport}>
          <IconUpload size={15} />
        </button>
        <button className="icon-btn" title="导入文件夹" onClick={onImportFolder}>
          <IconFolderPlus size={15} />
        </button>
        <button
          className="icon-btn"
          title={theme === "dark" ? "切换到明亮主题" : "切换到暗黑主题"}
          onClick={onToggleTheme}
        >
          {theme === "dark" ? <IconSun size={15} /> : <IconMoon size={15} />}
        </button>
        <button className="icon-btn" title="设置" onClick={onOpenSettings}>
          <IconSettings size={15} />
        </button>
      </div>
    </div>
  );
}
