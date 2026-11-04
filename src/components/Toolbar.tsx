import { IconEditor, IconFile, IconGraph, IconHistory, IconLink, IconMoon, IconPlus, IconRadar, IconRobot, IconSearch, IconSettings, IconSun, IconTrash, IconUndo, IconUpload } from "./Icons";
import type { ConceptNoteStatus } from "../api";

export type ViewMode = "editor" | "files" | "graph" | "search" | "trash";

interface ToolbarProps {
  view: ViewMode;
  theme: "dark" | "light";
  breadcrumb: string[];
  onViewChange: (v: ViewMode) => void;
  onNewNote: () => void;
  onImport: () => void;
  onImportUrl: () => void;
  onOpenAiAssistant: () => void;
  onUndo: () => void;
  onOpenHistory: () => void;
  historyAvailable: boolean;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  discoveryCount: number;
  discoveryConfigured: boolean;
  onOpenDiscovery: () => void;
  // 影子笔记（概念笔记）
  shadowNoteCount?: number;
  shadowNoteStatus?: ConceptNoteStatus | null;
  onToggleShadowPopover?: () => void;
  shadowPopoverOpen?: boolean;
  onStartConceptNoteBatch?: () => void;
  onSetConceptNoteEnabled?: (enabled: boolean) => void;
  onSetConceptNotePaused?: (paused: boolean) => void;
  onRetryAllConceptNotes?: () => void;
  onRetryConceptNote?: (taskId: number) => void;
  // Agent 接入入口
  onOpenAgentLink?: () => void;
  /** 已安装但尚未接入 KBOX 的 Agent 数量；>0 时图标高亮并显示角标 */
  agentLinkPending?: number;
}

export function Toolbar({
  view,
  theme,
  breadcrumb,
  onViewChange,
  onNewNote,
  onImport,
  onImportUrl,
  onOpenAiAssistant,
  onUndo,
  onOpenHistory,
  historyAvailable,
  onToggleTheme,
  onOpenSettings,
  discoveryCount,
  discoveryConfigured,
  onOpenDiscovery,
  shadowNoteCount = 0,
  shadowNoteStatus,
  onToggleShadowPopover,
  shadowPopoverOpen = false,
  onStartConceptNoteBatch,
  onSetConceptNoteEnabled,
  onSetConceptNotePaused,
  onRetryAllConceptNotes,
  onRetryConceptNote,
  onOpenAgentLink,
  agentLinkPending = 0,
}: ToolbarProps) {
  // 影子笔记统计
  const pending = shadowNoteStatus?.pending ?? 0;
  const running = shadowNoteStatus?.running ?? 0;
  const done = shadowNoteStatus?.done ?? 0;
  const failed = shadowNoteStatus?.failed ?? 0;
  const total = pending + running + done + failed;
  const tasks = shadowNoteStatus?.tasks ?? [];
  const activeTasks = tasks
    .filter((t) => t.status === "pending" || t.status === "running")
    .slice(0, 5);
  const failedTasks = tasks.filter((t) => t.status === "failed");
  const isPaused = shadowNoteStatus?.paused ?? false;
  const isEnabled = shadowNoteStatus?.enabled ?? false;
  const progressPct = total > 0 ? Math.round(((done + failed) / total) * 100) : 0;
  const hasPendingWork = pending > 0 || running > 0;

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
        <span className="agent-link-entry">
          <button
            className={`icon-btn agent-link-trigger${agentLinkPending > 0 ? " has-pending" : ""}`}
            title={
              agentLinkPending > 0
                ? `${agentLinkPending} 个 AI Agent 待接入 KBOX`
                : "接入 AI Agent（MCP）"
            }
            onClick={onOpenAgentLink}
          >
            <IconRobot size={15} />
          </button>
          {agentLinkPending > 0 && (
            <span className="agent-link-badge-dot">{agentLinkPending}</span>
          )}
        </span>
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
        <button className="icon-btn" title="导入网页" onClick={onImportUrl}>
          <IconLink size={15} />
        </button>
        <button className="icon-btn" title="撤销最近操作" onClick={onUndo}>
          <IconUndo size={15} />
        </button>
        <button
          className="icon-btn"
          title="当前文档版本历史"
          onClick={onOpenHistory}
          disabled={!historyAvailable}
        >
          <IconHistory size={15} />
        </button>
        <button
          className="icon-btn"
          title={theme === "dark" ? "切换到明亮主题" : "切换到暗黑主题"}
          onClick={onToggleTheme}
        >
          {theme === "dark" ? <IconSun size={15} /> : <IconMoon size={15} />}
        </button>
        <button className="icon-btn toolbar-ai-trigger" title="打开 KBOX AI 助手" onClick={onOpenAiAssistant}>
          <span className="toolbar-ai-text" aria-hidden="true">AI</span>
        </button>
        <div className="shadow-btn">
          <button
            className={`icon-btn shadow-trigger${shadowNoteCount > 0 ? " has-items" : ""}${shadowPopoverOpen ? " active" : ""}`}
            title="影子笔记"
            onClick={onToggleShadowPopover}
            style={shadowPopoverOpen ? { color: "var(--accent)", background: "var(--accent-soft)" } : undefined}
          >
            <span aria-hidden="true">◈</span>
            {shadowNoteCount > 0 && (
              <span className="shadow-badge">{shadowNoteCount > 99 ? "99+" : shadowNoteCount}</span>
            )}
          </button>
          {shadowPopoverOpen && (
            <>
              <div
                className="shadow-backdrop"
                onClick={onToggleShadowPopover}
                aria-hidden="true"
              />
              <div className="shadow-popover" role="dialog" aria-label="影子笔记">
                <div className="shadow-popover-header">
                  <span className="shadow-popover-title">影子笔记</span>
                  <button
                    className="shadow-popover-close"
                    title="关闭"
                    onClick={onToggleShadowPopover}
                  >
                    ×
                  </button>
                </div>

                {total > 0 && (
                  <div className="shadow-progress">
                    <div className="shadow-progress-bar">
                      <div
                        className="shadow-progress-fill"
                        style={{ width: `${progressPct}%` }}
                      />
                    </div>
                    <div className="shadow-progress-meta">
                      <span>
                        {running > 0 ? "生成中" : "已完成"} · {done}/{total}
                      </span>
                      <span>
                        {done} 已完成{failed > 0 ? ` · ${failed} 失败` : ""}
                      </span>
                    </div>
                  </div>
                )}

                <div className="shadow-toggle-row">
                  <div className="shadow-toggle-text">
                    <div className="shadow-toggle-title">自动生成</div>
                    <div className="shadow-toggle-desc">
                      为新对象自动生成影子笔记
                    </div>
                  </div>
                  <div
                    className={`toggle${isEnabled ? " on" : ""}`}
                    role="switch"
                    aria-checked={isEnabled}
                    tabIndex={0}
                    onClick={() => onSetConceptNoteEnabled?.(!isEnabled)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        onSetConceptNoteEnabled?.(!isEnabled);
                      }
                    }}
                  />
                </div>

                {pending > 0 && (
                  <div className="shadow-batch">
                    <div className="shadow-batch-info">
                      <span className="shadow-batch-icon">📋</span>
                      <span>{pending} 个文件待生成</span>
                    </div>
                    <button
                      className="shadow-batch-btn"
                      onClick={onStartConceptNoteBatch}
                    >
                      开始生成
                    </button>
                  </div>
                )}

                {activeTasks.length > 0 && (
                  <div className="shadow-task-list">
                    <div className="shadow-task-list-title">当前任务</div>
                    {activeTasks.map((t) => (
                      <div key={t.id} className="shadow-task-item">
                        <span className="shadow-task-icon">
                          {t.object_format === "image" ? "🖼" : "📄"}
                        </span>
                        <span className="shadow-task-name" title={t.object_title}>
                          {t.object_title}
                        </span>
                        <span className={`shadow-task-status ${t.status}`}>
                          {t.status === "running" ? "生成中" : "等待中"}
                        </span>
                      </div>
                    ))}
                  </div>
                )}

                {failedTasks.length > 0 && (
                  <div className="shadow-failed-list">
                    <div className="shadow-failed-header">
                      <span>失败任务（{failedTasks.length}）</span>
                      {onRetryAllConceptNotes && (
                        <button
                          className="shadow-retry-all"
                          onClick={onRetryAllConceptNotes}
                        >
                          全部重试
                        </button>
                      )}
                    </div>
                    {failedTasks.slice(0, 3).map((t) => (
                      <div key={t.id} className="shadow-failed-item">
                        <div className="shadow-failed-info">
                          <span className="shadow-task-icon">⚠</span>
                          <span className="shadow-task-name" title={t.object_title}>
                            {t.object_title}
                          </span>
                          {onRetryConceptNote && (
                            <button
                              className="shadow-retry-btn"
                              onClick={() => onRetryConceptNote(t.id)}
                            >
                              重试
                            </button>
                          )}
                        </div>
                        {t.error_message && (
                          <div className="shadow-failed-error" title={t.error_message}>
                            {t.error_message}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}

                <div className="shadow-footer">
                  <button
                    className="shadow-footer-btn"
                    onClick={() => onSetConceptNotePaused?.(!isPaused)}
                    disabled={!hasPendingWork}
                    title={isPaused ? "继续队列" : "暂停队列"}
                  >
                    {isPaused ? "继续" : "暂停"}
                  </button>
                  <button
                    className="shadow-footer-btn"
                    onClick={onToggleShadowPopover}
                  >
                    关闭
                  </button>
                </div>
              </div>
            </>
          )}
        </div>
        <button className="icon-btn" title="设置" onClick={onOpenSettings}>
          <IconSettings size={15} />
        </button>
      </div>
      <style>{`
        .agent-link-entry {
          position: relative;
          display: inline-flex;
        }
        .agent-link-entry .icon-btn.agent-link-trigger.has-pending {
          color: var(--accent);
        }
        .agent-link-badge-dot {
          position: absolute;
          top: -3px;
          right: -4px;
          min-width: 14px;
          height: 14px;
          padding: 0 4px;
          border-radius: 7px;
          background: var(--accent);
          color: #fff;
          font-size: 9px;
          font-weight: 600;
          display: flex;
          align-items: center;
          justify-content: center;
          line-height: 1;
          pointer-events: none;
        }
        .shadow-btn {
          position: relative;
          display: inline-flex;
        }
        .shadow-btn .icon-btn.shadow-trigger.has-items {
          color: var(--accent);
        }
        .shadow-badge {
          position: absolute;
          top: -3px;
          right: -4px;
          min-width: 14px;
          height: 14px;
          padding: 0 4px;
          border-radius: 7px;
          background: var(--accent);
          color: #fff;
          font-size: 9px;
          font-weight: 600;
          display: flex;
          align-items: center;
          justify-content: center;
          line-height: 1;
          pointer-events: none;
        }
        .shadow-backdrop {
          position: fixed;
          inset: 0;
          z-index: 100;
        }
        .shadow-popover {
          position: absolute;
          top: calc(100% + 6px);
          right: 0;
          width: 340px;
          max-height: 480px;
          overflow-y: auto;
          background: var(--bg-secondary, var(--bg-elevated));
          border: 1px solid var(--border);
          border-radius: 8px;
          box-shadow: 0 8px 24px rgba(0, 0, 0, 0.28);
          z-index: 101;
          padding: 8px 0;
          font-size: 12px;
          color: var(--text-primary);
        }
        .shadow-popover-header {
          display: flex;
          align-items: center;
          justify-content: space-between;
          padding: 4px 12px 8px;
        }
        .shadow-popover-title {
          font-weight: 600;
          color: var(--text-primary);
        }
        .shadow-popover-close {
          background: transparent;
          border: none;
          color: var(--text-muted);
          cursor: pointer;
          font-size: 16px;
          line-height: 1;
          padding: 2px 6px;
          border-radius: 4px;
        }
        .shadow-popover-close:hover {
          background: var(--bg-hover);
          color: var(--text-primary);
        }
        .shadow-progress {
          padding: 4px 12px 8px;
        }
        .shadow-progress-bar {
          height: 6px;
          background: var(--bg-active);
          border-radius: 3px;
          overflow: hidden;
          margin-bottom: 6px;
        }
        .shadow-progress-fill {
          height: 100%;
          background: var(--accent);
          transition: width 0.24s;
        }
        .shadow-progress-meta {
          display: flex;
          justify-content: space-between;
          align-items: center;
          color: var(--text-secondary);
          font-size: 11px;
        }
        .shadow-toggle-row {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 8px;
          padding: 8px 12px;
          border-top: 1px solid var(--border);
        }
        .shadow-toggle-text {
          flex: 1;
          min-width: 0;
        }
        .shadow-toggle-title {
          font-weight: 500;
          color: var(--text-primary);
        }
        .shadow-toggle-desc {
          color: var(--text-muted);
          font-size: 11px;
          margin-top: 2px;
        }
        .shadow-toggle-row .toggle {
          cursor: pointer;
          flex-shrink: 0;
        }
        .shadow-batch {
          display: flex;
          align-items: center;
          justify-content: space-between;
          padding: 8px 12px;
          background: var(--accent-soft);
          margin: 4px 8px;
          border-radius: 6px;
          gap: 8px;
        }
        .shadow-batch-info {
          display: flex;
          align-items: center;
          gap: 6px;
          color: var(--text-primary);
          min-width: 0;
        }
        .shadow-batch-icon {
          font-size: 13px;
        }
        .shadow-batch-btn {
          background: var(--accent);
          color: #fff;
          border: none;
          padding: 4px 10px;
          border-radius: 4px;
          font-size: 11px;
          cursor: pointer;
          white-space: nowrap;
        }
        .shadow-batch-btn:hover {
          opacity: 0.9;
        }
        .shadow-task-list {
          padding: 4px 0;
          border-top: 1px solid var(--border);
        }
        .shadow-task-list-title {
          padding: 4px 12px;
          color: var(--text-muted);
          font-size: 11px;
        }
        .shadow-task-item {
          display: flex;
          align-items: center;
          gap: 8px;
          padding: 4px 12px;
          color: var(--text-primary);
        }
        .shadow-task-item:hover {
          background: var(--bg-hover);
        }
        .shadow-task-icon {
          font-size: 13px;
          flex-shrink: 0;
        }
        .shadow-task-name {
          flex: 1;
          min-width: 0;
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
        }
        .shadow-task-status {
          font-size: 11px;
          color: var(--text-muted);
          flex-shrink: 0;
        }
        .shadow-task-status.running {
          color: var(--accent);
        }
        .shadow-failed-list {
          margin: 4px 8px;
          padding: 8px;
          background: rgba(218, 54, 51, 0.08);
          border: 1px solid rgba(218, 54, 51, 0.2);
          border-radius: 6px;
        }
        .shadow-failed-header {
          display: flex;
          align-items: center;
          justify-content: space-between;
          color: var(--danger, #da3633);
          font-size: 11px;
          font-weight: 500;
          margin-bottom: 4px;
        }
        .shadow-retry-all {
          background: transparent;
          border: 1px solid var(--danger, #da3633);
          color: var(--danger, #da3633);
          border-radius: 3px;
          padding: 2px 6px;
          font-size: 11px;
          cursor: pointer;
        }
        .shadow-retry-all:hover {
          background: rgba(218, 54, 51, 0.1);
        }
        .shadow-failed-item {
          padding: 4px 0;
          display: flex;
          flex-direction: column;
          gap: 2px;
        }
        .shadow-failed-info {
          display: flex;
          align-items: center;
          gap: 6px;
        }
        .shadow-failed-error {
          color: var(--danger, #da3633);
          font-size: 11px;
          margin-left: 18px;
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
        }
        .shadow-retry-btn {
          background: transparent;
          border: 1px solid var(--danger, #da3633);
          color: var(--danger, #da3633);
          border-radius: 3px;
          padding: 1px 6px;
          font-size: 10px;
          cursor: pointer;
          margin-left: auto;
        }
        .shadow-retry-btn:hover {
          background: rgba(218, 54, 51, 0.1);
        }
        .shadow-footer {
          display: flex;
          gap: 8px;
          padding: 8px 12px;
          border-top: 1px solid var(--border);
        }
        .shadow-footer-btn {
          flex: 1;
          background: var(--bg-active);
          color: var(--text-primary);
          border: 1px solid var(--border);
          border-radius: 4px;
          padding: 4px 10px;
          font-size: 11px;
          cursor: pointer;
        }
        .shadow-footer-btn:hover:not(:disabled) {
          background: var(--bg-hover);
        }
        .shadow-footer-btn:disabled {
          opacity: 0.5;
          cursor: not-allowed;
        }
      `}</style>
    </div>
  );
}
