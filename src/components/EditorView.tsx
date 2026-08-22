import { useEffect, useMemo, useRef, useState } from "react";
import { IconClose, IconEditor, objTypeMeta } from "./Icons";
import { markdownToHtml } from "../utils/markdown";
import { aiEditText, getAssetBase64, getAssetUrl } from "../api";
import type { ObjectMetadata } from "../mock/data";

export interface EditorTab {
  id: string; // object id
  title: string;
  type: string; // objType
}

interface EditorViewProps {
  tabs: EditorTab[];
  activeTabId: string | null;
  content: string;
  metadata: ObjectMetadata | null;
  dirty?: boolean;
  onActivateTab: (id: string) => void;
  onCloseTab: (id: string) => void;
  onCloseAllTabs: () => void;
  onContentChange: (id: string, content: string) => void;
  onTitleChange: (id: string, title: string) => void;
  onSave: (id: string) => void;
  allObjects?: { id: string; title: string; type: string }[];
}

export function EditorView({
  tabs,
  activeTabId,
  content,
  metadata,
  dirty,
  onActivateTab,
  onCloseTab,
  onCloseAllTabs,
  onContentChange,
  onTitleChange,
  onSave,
  allObjects,
}: EditorViewProps) {
  const [mode, setMode] = useState<"edit" | "preview">("edit");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [assetUrl, setAssetUrl] = useState<string | null>(null);
  const [assetBase64, setAssetBase64] = useState<string | null>(null);
  const [aiPromptOpen, setAiPromptOpen] = useState(false);
  const [aiInstruction, setAiInstruction] = useState("");
  const [aiRunning, setAiRunning] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const aiSelectionRef = useRef({ start: 0, end: 0 });

  // ===== [[xxx]] 自动补全 =====
  const [suggestionVisible, setSuggestionVisible] = useState(false);
  const [suggestionIndex, setSuggestionIndex] = useState(0);
  const [wikiLinkQuery, setWikiLinkQuery] = useState("");
  // 用于记录 [[ 起始位置，便于替换
  const wikiStartRef = useRef<number>(-1);

  // 根据 query 过滤对象列表
  const suggestions = useMemo(() => {
    if (!suggestionVisible || !allObjects) return [];
    const q = wikiLinkQuery.toLowerCase();
    const list = allObjects.filter((o) => {
      // 排除当前对象自身
      if (activeTabId && o.id === activeTabId) return false;
      if (!q) return true;
      return o.title.toLowerCase().includes(q);
    });
    // 限制最多 10 个
    return list.slice(0, 10);
  }, [suggestionVisible, allObjects, wikiLinkQuery, activeTabId]);

  // 当 suggestions 变化时，重置选中索引
  useEffect(() => {
    setSuggestionIndex(0);
  }, [suggestionVisible, wikiLinkQuery]);

  // 切换标签或对象时隐藏建议
  useEffect(() => {
    setSuggestionVisible(false);
  }, [activeTabId]);

  const format = metadata?.format;
  const isReadOnly = format === "image" || format === "pdf" || format === "attachment";

  // 切换对象时根据格式决定默认模式
  useEffect(() => {
    if (isReadOnly) {
      setMode("preview");
    } else {
      setMode("edit");
    }
  }, [activeTabId, isReadOnly]);

  // 加载图片/PDF 资产 base64（用于预览）
  useEffect(() => {
    if (!activeTabId || !isReadOnly) {
      setAssetBase64(null);
      setAssetUrl(null);
      return;
    }
    let cancelled = false;
    // 加载 base64 用于预览
    getAssetBase64(activeTabId)
      .then((url) => {
        if (!cancelled) setAssetBase64(url);
      })
      .catch(() => {
        if (!cancelled) setAssetBase64(null);
      });
    // 加载文件路径用于下载链接
    getAssetUrl(activeTabId)
      .then((url) => {
        if (!cancelled) setAssetUrl(url);
      })
      .catch(() => {
        if (!cancelled) setAssetUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [activeTabId, isReadOnly]);

  const html = useMemo(() => markdownToHtml(content), [content]);

  // ===== [[xxx]] 自动补全处理 =====

  // 检测光标前是否有未闭合的 [[
  const detectWikiLink = (value: string, cursorPos: number) => {
    // 从光标位置向前查找最近的 [[
    const before = value.substring(0, cursorPos);
    // 查找最后一个 [[
    const startIdx = before.lastIndexOf("[[");
    if (startIdx === -1) {
      return { active: false, start: -1, query: "" };
    }
    // 检查 [[ 之后是否已经有 ]]
    const afterStart = before.substring(startIdx + 2);
    if (afterStart.includes("]]")) {
      return { active: false, start: -1, query: "" };
    }
    return {
      active: true,
      start: startIdx,
      query: afterStart,
    };
  };

  // 处理 textarea 内容变化，检测 [[xxx]] 并显示建议
  const handleTextareaChange = (value: string) => {
    if (!activeTabId) return;
    onContentChange(activeTabId, value);

    const textarea = textareaRef.current;
    if (!textarea) return;
    const cursorPos = textarea.selectionStart;
    const detection = detectWikiLink(value, cursorPos);
    if (detection.active) {
      wikiStartRef.current = detection.start;
      setWikiLinkQuery(detection.query);
      setSuggestionVisible(true);
    } else {
      setSuggestionVisible(false);
      wikiStartRef.current = -1;
    }
  };

  // 插入选中的建议
  const insertSuggestion = (suggestion: { id: string; title: string }) => {
    const textarea = textareaRef.current;
    if (!textarea || !activeTabId) return;
    const start = wikiStartRef.current;
    if (start < 0) return;
    const cursorPos = textarea.selectionStart;
    const before = content.substring(0, start);
    const after = content.substring(cursorPos);
    const inserted = `[[${suggestion.title}]]`;
    const newValue = before + inserted + after;
    onContentChange(activeTabId, newValue);
    // 设置光标位置到插入文本之后
    const newCursorPos = start + inserted.length;
    requestAnimationFrame(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.setSelectionRange(newCursorPos, newCursorPos);
      }
    });
    setSuggestionVisible(false);
    wikiStartRef.current = -1;
  };

  const openAiPrompt = () => {
    if (!activeTabId || isReadOnly) return;
    const textarea = textareaRef.current;
    aiSelectionRef.current = {
      start: textarea?.selectionStart ?? 0,
      end: textarea?.selectionEnd ?? 0,
    };
    setAiError(null);
    setAiPromptOpen(true);
  };

  const runAiEdit = async () => {
    if (!activeTabId || !aiInstruction.trim()) return;
    const { start, end } = aiSelectionRef.current;
    const hasSelection = end > start;
    const target = hasSelection ? content.slice(start, end) : content;
    setAiRunning(true);
    setAiError(null);
    try {
      const result = await aiEditText(
        aiInstruction.trim(),
        target,
        hasSelection ? content : undefined
      );
      const next = hasSelection
        ? content.slice(0, start) + result + content.slice(end)
        : result;
      onContentChange(activeTabId, next);
      setAiPromptOpen(false);
      setAiInstruction("");
      const cursor = hasSelection ? start + result.length : result.length;
      requestAnimationFrame(() => {
        textareaRef.current?.focus();
        textareaRef.current?.setSelectionRange(cursor, cursor);
      });
    } catch (e: any) {
      setAiError(String(e));
    } finally {
      setAiRunning(false);
    }
  };

  // 处理键盘事件：上下选择，回车/Tab 确认，Escape 关闭
  const handleTextareaKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "j") {
      e.preventDefault();
      openAiPrompt();
      return;
    }
    if (!suggestionVisible || suggestions.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSuggestionIndex((prev) => (prev + 1) % suggestions.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSuggestionIndex(
        (prev) => (prev - 1 + suggestions.length) % suggestions.length
      );
    } else if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      const selected = suggestions[suggestionIndex];
      if (selected) {
        insertSuggestion(selected);
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      setSuggestionVisible(false);
    }
  };

  if (tabs.length === 0) {
    return (
      <div className="editor-empty">
        <div className="editor-empty-icon">
          <IconEditor size={26} />
        </div>
        <div className="editor-empty-title">尚未打开任何对象</div>
        <div className="editor-empty-hint">
          从左侧文件树选择一个对象，或使用快捷键创建新笔记
        </div>
        <div className="empty-shortcut">
          <kbd>⌘N</kbd>
          <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
            新建笔记
          </span>
          <kbd>⌘K</kbd>
          <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
            命令面板
          </span>
        </div>
      </div>
    );
  }

  // 渲染编辑/预览主体
  const renderBody = () => {
    // 图片：只读视图
    if (format === "image") {
      return (
        <div
          className="editor-image-view"
          style={{
            flex: 1,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            padding: 24,
            gap: 16,
            overflow: "auto",
          }}
        >
          {assetBase64 ? (
            <img
              src={assetBase64}
              alt={metadata?.title ?? ""}
              style={{
                maxWidth: "100%",
                maxHeight: "70vh",
                objectFit: "contain",
                borderRadius: 8,
                boxShadow: "0 4px 16px rgba(0,0,0,0.15)",
              }}
            />
          ) : (
            <div style={{ color: "var(--text-muted)" }}>加载图片中...</div>
          )}
          <div style={{ color: "var(--text-secondary)", fontSize: 13 }}>
            {metadata?.title} ({metadata?.fileSize ?? 0} 字节)
          </div>
        </div>
      );
    }

    // PDF：只读视图
    if (format === "pdf") {
      return (
        <div
          className="editor-pdf-view"
          style={{
            flex: 1,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            padding: 24,
            gap: 16,
          }}
        >
          <div
            style={{
              width: 64,
              height: 64,
              borderRadius: 16,
              background: "var(--bg-secondary)",
              border: "1px solid var(--border)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--red, var(--accent))",
              fontSize: 28,
              fontWeight: 700,
            }}
          >
            PDF
          </div>
          <div style={{ fontSize: 15, color: "var(--text-secondary)", fontWeight: 500 }}>
            {metadata?.title}
          </div>
          <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
            {metadata?.fileSize ?? 0} 字节
          </div>
          {assetBase64 && (
            <iframe
              src={assetBase64}
              style={{
                width: "100%",
                maxWidth: 800,
                height: "60vh",
                border: "1px solid var(--border)",
                borderRadius: 8,
              }}
              title={metadata?.title ?? "PDF 预览"}
            />
          )}
          {assetUrl && (
            <a
              className="btn btn-primary"
              style={{ marginTop: 8, textDecoration: "none" }}
              href={assetUrl}
              target="_blank"
              rel="noreferrer"
            >
              下载/打开原文件
            </a>
          )}
        </div>
      );
    }

    // 附件：只读视图
    if (format === "attachment") {
      return (
        <div
          className="editor-attachment-view"
          style={{
            flex: 1,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            padding: 24,
            gap: 16,
          }}
        >
          <div
            style={{
              width: 64,
              height: 64,
              borderRadius: 16,
              background: "var(--bg-secondary)",
              border: "1px solid var(--border)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--text-secondary)",
              fontSize: 28,
            }}
          >
            📎
          </div>
          <div style={{ fontSize: 15, color: "var(--text-secondary)", fontWeight: 500 }}>
            {metadata?.title}
          </div>
          <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
            附件 · {metadata?.fileSize ?? 0} 字节
          </div>
          {assetUrl && (
            <a
              className="btn btn-primary"
              style={{ marginTop: 8, textDecoration: "none" }}
              href={assetUrl}
              target="_blank"
              rel="noreferrer"
            >
              下载/打开原文件
            </a>
          )}
        </div>
      );
    }

    // 代码：编辑（等宽字体） / 预览（代码块）
    if (format === "code") {
      return mode === "edit" ? (
        <textarea
          ref={textareaRef}
          className="editor-textarea code-editor"
          style={{ fontFamily: "var(--font-mono, monospace)" }}
          value={content}
          placeholder="代码内容..."
          onChange={(e) => handleTextareaChange(e.target.value)}
          onKeyDown={handleTextareaKeyDown}
          spellCheck={false}
        />
      ) : (
        <div
          className="editor-preview"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      );
    }

    // Markdown：编辑 / 预览
    return mode === "edit" ? (
      <textarea
        ref={textareaRef}
        className="editor-textarea"
        value={content}
        placeholder="在此输入 Markdown 内容..."
        onChange={(e) => handleTextareaChange(e.target.value)}
        onKeyDown={handleTextareaKeyDown}
        spellCheck={false}
      />
    ) : (
      <div
        className="editor-preview"
        dangerouslySetInnerHTML={{ __html: html }}
      />
    );
  };

  return (
    <div className="editor-area">
      {/* 标签栏 */}
      <div className="editor-tabs-bar">
        <div className="editor-tabs">
          {tabs.map((tab) => {
          const typeMeta = objTypeMeta[tab.type];
          const Icon = typeMeta?.icon ?? IconEditor;
          const color = typeMeta?.color ?? "var(--text-secondary)";
          return (
            <div
              key={tab.id}
              className={`editor-tab${tab.id === activeTabId ? " active" : ""}`}
              onClick={() => onActivateTab(tab.id)}
            >
              <span className="tree-icon" style={{ color }}>
                <Icon size={13} />
              </span>
              <span>{tab.title}</span>
              <span
                className="tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  onCloseTab(tab.id);
                }}
                title="关闭"
              >
                <IconClose size={11} />
              </span>
            </div>
          );
          })}
        </div>
        <button
          className="editor-close-all"
          onClick={onCloseAllTabs}
          title="关闭所有标签页"
        >
          <IconClose size={12} />
          关闭全部
        </button>
      </div>

      {/* 工具栏 */}
      <div className="editor-toolbar">
        <input
          className="editor-title-input"
          value={metadata?.title ?? ""}
          placeholder="无标题"
          onChange={(e) => {
            if (activeTabId) onTitleChange(activeTabId, e.target.value);
          }}
        />
        <div className="editor-mode-toggle">
          <button
            className={mode === "edit" ? "active" : ""}
            onClick={() => setMode("edit")}
            disabled={isReadOnly}
            style={isReadOnly ? { opacity: 0.5, cursor: "not-allowed" } : undefined}
          >
            编辑
          </button>
          <button
            className={mode === "preview" ? "active" : ""}
            onClick={() => setMode("preview")}
          >
            预览
          </button>
        </div>
        <button
          className="btn btn-ghost editor-ai-button"
          onClick={openAiPrompt}
          disabled={!activeTabId || isReadOnly}
          title="使用 AI（⌘J）"
        >
          ✦ AI
        </button>
        <button
          className={`btn btn-ghost${dirty ? " dirty" : ""}`}
          style={{ height: 26, fontSize: 11, padding: "0 10px" }}
          onClick={() => {
            if (activeTabId) onSave(activeTabId);
          }}
          disabled={!activeTabId || isReadOnly}
          title="保存 (⌘S)"
        >
          {dirty ? "保存*" : "已保存"}
        </button>
      </div>

      {/* 编辑/预览区 */}
      <div className="editor-body" style={{ position: "relative" }}>
        {renderBody()}
        {aiPromptOpen && (
          <div className="editor-ai-popover" role="dialog" aria-label="使用 AI 编辑">
            <div className="editor-ai-popover-title">
              <span>✦ 使用 AI 编辑</span>
              <button
                className="icon-btn"
                onClick={() => !aiRunning && setAiPromptOpen(false)}
                aria-label="关闭"
              >
                ×
              </button>
            </div>
            <div className="editor-ai-scope">
              {aiSelectionRef.current.end > aiSelectionRef.current.start
                ? "作用于当前选区"
                : "作用于整篇文档"}
            </div>
            <textarea
              autoFocus
              value={aiInstruction}
              onChange={(e) => setAiInstruction(e.target.value)}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                  e.preventDefault();
                  runAiEdit();
                } else if (e.key === "Escape" && !aiRunning) {
                  setAiPromptOpen(false);
                }
              }}
              placeholder="例如：润色并补充关键细节"
              disabled={aiRunning}
            />
            {aiError && <div className="editor-ai-error">{aiError}</div>}
            <div className="editor-ai-actions">
              <span>⌘↵ 执行</span>
              <button
                className="btn btn-primary"
                onClick={runAiEdit}
                disabled={aiRunning || !aiInstruction.trim()}
              >
                {aiRunning ? "AI 正在处理..." : "执行"}
              </button>
            </div>
          </div>
        )}
        {/* [[xxx]] 自动补全建议浮层 */}
        {suggestionVisible && suggestions.length > 0 && (
          <div
            style={{
              position: "absolute",
              top: "30%",
              left: "50%",
              transform: "translateX(-50%)",
              minWidth: 240,
              maxWidth: 360,
              background: "var(--bg-elevated, var(--bg-secondary, #fff))",
              border: "1px solid var(--border-color, #e1e4e8)",
              borderRadius: 6,
              boxShadow: "0 4px 16px rgba(0,0,0,0.15)",
              zIndex: 100,
              overflow: "hidden",
            }}
          >
            <div
              style={{
                padding: "4px 10px",
                fontSize: 10,
                color: "var(--text-muted)",
                borderBottom: "1px solid var(--border-color, #eee)",
                background: "var(--bg-secondary, #f6f8fa)",
              }}
            >
              链接到对象（↑↓选择，回车确认，Esc关闭）
            </div>
            <div style={{ maxHeight: 200, overflowY: "auto" }}>
              {suggestions.map((s, idx) => {
                const meta = objTypeMeta[s.type as keyof typeof objTypeMeta];
                const Icon = meta?.icon ?? IconEditor;
                const color = meta?.color ?? "var(--text-secondary)";
                return (
                  <div
                    key={s.id}
                    onClick={() => insertSuggestion(s)}
                    style={{
                      padding: "6px 10px",
                      cursor: "pointer",
                      fontSize: 12,
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                      background:
                        idx === suggestionIndex
                          ? "var(--accent-soft, #f0f6ff)"
                          : undefined,
                    }}
                    onMouseEnter={() => setSuggestionIndex(idx)}
                  >
                    <span className="tree-icon" style={{ color }}>
                      <Icon size={12} />
                    </span>
                    <span>{s.title}</span>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
