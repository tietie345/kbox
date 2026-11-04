import { EditorEmpty } from "./EditorEmpty";
import { useEffect, useMemo, useRef, useState } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import { IconClose, IconEditor, objTypeMeta } from "./Icons";
import { htmlToMarkdown, markdownToHtml } from "../utils/markdown";
import { listen } from "@tauri-apps/api/event";
import {
  aiEditText,
  getAssetBase64,
  importImageBytes,
  openAsset,
  regenerateConceptNote,
  runOcr,
  type ConceptNoteEvent,
} from "../api";
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
  onReload: (id: string) => Promise<void>;
  allObjects?: { id: string; title: string; type: string }[];
  conceptNote?: string | null;  // 概念摘要文本（已从 index.md 中提取）
}

const KboxImage = Image.extend({
  addOptions() {
    const parent = this.parent?.();
    return {
      inline: parent?.inline ?? false,
      allowBase64: parent?.allowBase64 ?? true,
      HTMLAttributes: parent?.HTMLAttributes ?? {},
      resize: {
        enabled: true,
        directions: ["bottom-right"] as ("bottom-right")[],
        minWidth: 40,
        minHeight: 40,
        alwaysPreserveAspectRatio: true,
      },
    };
  },
  addAttributes() {
    return {
      ...this.parent?.(),
      kboxId: {
        default: null,
        parseHTML: (element) => element.getAttribute("data-kbox-image"),
        renderHTML: (attributes) => attributes.kboxId
          ? { "data-kbox-image": attributes.kboxId }
          : {},
      },
      width: {
        default: null,
        parseHTML: (element) => {
          const w = element.getAttribute("width");
          const n = w ? Number(w) : NaN;
          return Number.isFinite(n) && n > 0 ? n : null;
        },
      },
    };
  },
  parseHTML() {
    return [
      { tag: "img[src]" },
      { tag: "img[data-kbox-image]" },
    ];
  },
}).configure({ allowBase64: true });

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
  onReload,
  allObjects,
  conceptNote,
}: EditorViewProps) {
  const [mode, setMode] = useState<"edit" | "source" | "preview">("edit");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const previewRef = useRef<HTMLDivElement>(null);
  const richEditorRootRef = useRef<HTMLDivElement>(null);
  const [assetBase64, setAssetBase64] = useState<string | null>(null);
  const [assetLightboxOpen, setAssetLightboxOpen] = useState(false);
  const [pasteState, setPasteState] = useState<"idle" | "loading" | "error">("idle");
  const [pasteError, setPasteError] = useState<string | null>(null);
  const pasteInFlightRef = useRef(false);
  const [aiPromptOpen, setAiPromptOpen] = useState(false);
  const [aiInstruction, setAiInstruction] = useState("");
  const [aiRunning, setAiRunning] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [ocrRunning, setOcrRunning] = useState(false);
  const [ocrError, setOcrError] = useState<string | null>(null);
  const aiSelectionRef = useRef({ start: 0, end: 0 });
  const aiRequestTokenRef = useRef(0);
  const activeTabIdRef = useRef(activeTabId);
  const onContentChangeRef = useRef(onContentChange);
  activeTabIdRef.current = activeTabId;
  onContentChangeRef.current = onContentChange;

  const richEditor = useEditor({
    immediatelyRender: false,
    extensions: [StarterKit, KboxImage],
    content: "",
    editorProps: {
      attributes: {
        class: "kbox-rich-editor-content",
        spellcheck: "true",
      },
    },
    onUpdate: ({ editor }) => {
      const id = activeTabIdRef.current;
      if (id) onContentChangeRef.current(id, htmlToMarkdown(editor.getHTML()));
    },
  }, []);

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

  // 切换对象时关闭图片预览弹窗
  useEffect(() => {
    setAssetLightboxOpen(false);
  }, [activeTabId]);

  // 资产预览弹窗打开时按 Escape 关闭
  useEffect(() => {
    if (!assetLightboxOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setAssetLightboxOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [assetLightboxOpen]);

  const format = metadata?.format;
  const isReadOnly = format === "image" || format === "pdf" || format === "attachment";
  const supportsConcept = isReadOnly;

  // 只读文档在"原文 / 概念摘要"两个视图间切换
  const [docView, setDocView] = useState<"original" | "concept">("original");
  const [conceptBusy, setConceptBusy] = useState(false);
  const [conceptError, setConceptError] = useState<string | null>(null);

  // 已保存的概念摘要是否为 AI"内容不足"拒答脏数据（需要重新生成）
  const isRefusalNote =
    !!conceptNote &&
    conceptNote.trim().length < 40 &&
    (conceptNote.includes("无法生成摘要") ||
      conceptNote.includes("内容不足") ||
      conceptNote.includes("没有可提取"));
  const hasValidConcept = !!conceptNote && !isRefusalNote;

  // 切换对象时回到原文视图并重置状态
  useEffect(() => {
    setDocView("original");
    setConceptBusy(false);
    setConceptError(null);
  }, [activeTabId]);

  // 概念摘要异步生成完成（conceptNote 从无到有）时，若停留在概念页则结束加载态
  useEffect(() => {
    if (hasValidConcept) setConceptBusy(false);
  }, [hasValidConcept]);

  const handleRegenerateConcept = async () => {
    if (!activeTabId || conceptBusy) return;
    setConceptBusy(true);
    setConceptError(null);
    try {
      await regenerateConceptNote(activeTabId);
      // 后台 Worker 会自动 OCR（扫描件）并生成；完成后由事件刷新对象
    } catch (error) {
      setConceptError(String(error));
      setConceptBusy(false);
    }
  };

  // 监听当前对象的概念笔记生成结果：失败时结束加载态并展示原因
  useEffect(() => {
    if (!activeTabId) return;
    let dispose: (() => void) | undefined;
    listen<ConceptNoteEvent>("concept-note-event", (event) => {
      const evt = event.payload;
      if (evt.object_id !== activeTabId) return;
      if (evt.kind === "failed") {
        setConceptBusy(false);
        setConceptError(evt.description || "生成失败，请重试");
      } else if (evt.kind === "completed") {
        setConceptBusy(false);
        setConceptError(null);
      }
    }).then((un) => {
      dispose = un;
    });
    return () => dispose?.();
  }, [activeTabId]);


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
    if (!activeTabId || (format !== "image" && format !== "pdf")) {
      setAssetBase64(null);
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
    return () => {
      cancelled = true;
    };
  }, [activeTabId, format]);

  const handleOpenAsset = async () => {
    if (!activeTabId) return;
    try {
      await openAsset(activeTabId);
    } catch (error) {
      setPasteError(String(error));
      setPasteState("error");
    }
  };

  const handleOcr = async () => {
    if (!activeTabId || ocrRunning) return;
    setOcrRunning(true);
    setOcrError(null);
    try {
      await runOcr(activeTabId);
      await onReload(activeTabId);
    } catch (error) {
      setOcrError(String(error));
    } finally {
      setOcrRunning(false);
    }
  };

  const html = useMemo(() => markdownToHtml(content), [content]);

  useEffect(() => {
    if (!richEditor || !activeTabId || format === "code" || isReadOnly) return;
    const currentMarkdown = htmlToMarkdown(richEditor.getHTML());
    if (currentMarkdown !== content.trimEnd()) {
      richEditor.commands.setContent(html, { emitUpdate: false });
    }
  }, [activeTabId, content, format, html, isReadOnly, richEditor]);

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
    if (mode === "edit" && format !== "code" && richEditor) {
      aiSelectionRef.current = {
        start: richEditor.state.selection.from,
        end: richEditor.state.selection.to,
      };
    } else {
      const textarea = textareaRef.current;
      aiSelectionRef.current = {
        start: textarea?.selectionStart ?? 0,
        end: textarea?.selectionEnd ?? 0,
      };
    }
    setAiError(null);
    setAiPromptOpen(true);
  };

  const runAiEdit = async () => {
    if (!activeTabId || !aiInstruction.trim()) return;
    const requestToken = ++aiRequestTokenRef.current;
    const requestTabId = activeTabId;
    const requestMode = mode;
    const requestContent = content;
    const requestInstruction = aiInstruction.trim();
    const { start, end } = aiSelectionRef.current;
    const hasSelection = end > start;
    const richSelectionEditor = mode === "edit" && format !== "code" ? richEditor : null;
    const isRichSelection = Boolean(richSelectionEditor);
    const target = hasSelection
      ? isRichSelection
        ? richSelectionEditor!.state.doc.textBetween(start, end, "\n")
        : content.slice(start, end)
      : content;
    setAiRunning(true);
    setAiError(null);
    try {
      const result = await aiEditText(
        requestInstruction,
        target,
        hasSelection ? requestContent : undefined
      );
      // AI 请求可能跨越切换标签或本地编辑。只有对象、模式、请求令牌和
      // 基准正文都未变化时才允许把结果写回，避免结果落到另一个对象。
      if (
        aiRequestTokenRef.current !== requestToken
        || activeTabIdRef.current !== requestTabId
        || mode !== requestMode
      ) {
        return;
      }
      if (richSelectionEditor) {
        if (htmlToMarkdown(richSelectionEditor.getHTML()) !== requestContent.trimEnd()) {
          setAiError("文档在 AI 处理期间已变化，结果未应用");
          return;
        }
        if (hasSelection) {
          richSelectionEditor
            .chain()
            .focus()
            .insertContentAt({ from: start, to: end }, markdownToHtml(result))
            .run();
        } else {
          richSelectionEditor.commands.setContent(markdownToHtml(result));
        }
      } else {
        if (content !== requestContent) {
          setAiError("文档在 AI 处理期间已变化，结果未应用");
          return;
        }
        const next = hasSelection
          ? requestContent.slice(0, start) + result + requestContent.slice(end)
          : result;
        onContentChange(requestTabId, next);
      }
      setAiPromptOpen(false);
      setAiInstruction("");
      if (!isRichSelection) {
        const cursor = hasSelection ? start + result.length : result.length;
        requestAnimationFrame(() => {
          textareaRef.current?.focus();
          textareaRef.current?.setSelectionRange(cursor, cursor);
        });
      }
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

  // ===== 粘贴图片（保存为库内图片对象并插入引用） =====
  const insertImageFromPaste = async (file: File) => {
    if (!activeTabId || pasteInFlightRef.current) return;
    if (!file.type.startsWith("image/")) {
      setPasteState("error");
      setPasteError("剪贴板内容不是受支持的图片");
      return;
    }
    if (file.size === 0 || file.size > 20 * 1024 * 1024) {
      setPasteState("error");
      setPasteError(file.size === 0 ? "剪贴板图片为空" : "图片超过 20 MB，无法粘贴");
      return;
    }
    const richPasteEditor = mode === "edit" && format !== "code" ? richEditor : null;
    const richPos = richPasteEditor?.state.selection.from;
    const textarea = textareaRef.current;
    const pos = textarea?.selectionStart ?? content.length;
    const reader = new FileReader();
    pasteInFlightRef.current = true;
    setPasteState("loading");
    setPasteError(null);
    reader.onload = async () => {
      const dataUrl = String(reader.result || "");
      try {
        const res = await importImageBytes(dataUrl, metadata?.group, file.name);
        if (richPasteEditor && richPos !== undefined) {
          richPasteEditor
            .chain()
            .focus()
            .insertContentAt(richPos, {
              type: "image",
              attrs: {
                src: dataUrl,
                alt: res.title,
                kboxId: res.id,
              },
            })
            .run();
        } else {
          const token = `\n\n![${res.title}](kbox-image://${res.id})\n\n`;
          const next = content.slice(0, pos) + token + content.slice(pos);
          onContentChange(activeTabId, next);
          if (textareaRef.current) {
            textareaRef.current.focus();
            const newPos = pos + token.length;
            textareaRef.current.setSelectionRange(newPos, newPos);
          }
        }
        setPasteState("idle");
      } catch (err) {
        setPasteState("error");
        setPasteError(`粘贴图片失败：${String(err)}`);
      } finally {
        pasteInFlightRef.current = false;
      }
    };
    reader.onerror = () => {
      pasteInFlightRef.current = false;
      setPasteState("error");
      setPasteError("无法读取剪贴板图片");
    };
    reader.readAsDataURL(file);
  };

  const handlePaste = (e: React.ClipboardEvent<HTMLElement>) => {
    if (!activeTabId) return;
    const items = e.clipboardData?.items;
    if (!items) return;
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (item.kind === "file" && item.type.startsWith("image/")) {
        e.preventDefault();
        const file = item.getAsFile();
        if (file) insertImageFromPaste(file);
        return;
      }
    }
  };

  // ===== 所见即所得编辑/预览时解析 kbox-image://<id> 为 data URL =====
  useEffect(() => {
    if (isReadOnly || mode === "source" || !activeTabId) return;

    // 预览模式：直接操作 innerHTML 渲染出的 DOM
    if (mode === "preview") {
      const container = previewRef.current;
      if (!container) return;
      const timer = setTimeout(() => {
        container.querySelectorAll<HTMLImageElement>("img[data-kbox-image]").forEach((img) => {
          if (img.getAttribute("src")) return;
          const id = img.getAttribute("data-kbox-image");
          if (!id) return;
          img.classList.add("kbox-loading");
          getAssetBase64(id)
            .then((url) => {
              if (url) img.src = url;
            })
            .finally(() => img.classList.remove("kbox-loading"));
        });
      }, 0);
      return () => clearTimeout(timer);
    }

    // 编辑模式：必须把 data URL 写回 ProseMirror 节点的 src 属性。
    // 否则缩放提交触发 NodeView onUpdate 时会按空 src 清空图片。
    if (!richEditor) return;
    const pending: { pos: number; id: string }[] = [];
    richEditor.state.doc.descendants((node, pos) => {
      if (node.type.name === "image" && node.attrs.kboxId && !node.attrs.src) {
        pending.push({ pos, id: node.attrs.kboxId as string });
      }
      return true;
    });
    if (pending.length === 0) return;
    let cancelled = false;
    Promise.all(pending.map(async (p) => ({ pos: p.pos, url: await getAssetBase64(p.id) }))).then(
      (items) => {
        if (cancelled || !richEditor) return;
        const resolved = items.filter((it): it is { pos: number; url: string } => !!it.url);
        if (resolved.length === 0) return;
        const { state, view } = richEditor;
        const tr = state.tr;
        // 倒序写入，避免位置受影响（setNodeMarkup 不改变文档长度）
        for (const it of resolved.sort((a, b) => b.pos - a.pos)) {
          const node = tr.doc.nodeAt(it.pos);
          if (node && node.type.name === "image") {
            tr.setNodeMarkup(it.pos, undefined, { ...node.attrs, src: it.url });
          }
        }
        if (!tr.steps.length) return;
        view.dispatch(tr.setMeta("addToHistory", false));
      }
    );
    return () => {
      cancelled = true;
    };
  }, [content, mode, isReadOnly, activeTabId, richEditor]);

  if (tabs.length === 0) {
    return <EditorEmpty />;
  }

  // 概念摘要独立视图
  const renderConceptView = () => {
    return (
      <div
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "28px 32px",
          background: "var(--bg-primary, #fff)",
        }}
      >
        <div style={{ maxWidth: 760, margin: "0 auto" }}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              marginBottom: 18,
            }}
          >
            <span
              style={{
                width: 28,
                height: 28,
                display: "grid",
                placeItems: "center",
                borderRadius: 7,
                color: "#fff",
                background: "var(--accent, #7062f2)",
                fontSize: 14,
                fontWeight: 700,
              }}
            >
              ◈
            </span>
            <strong style={{ fontSize: 16 }}>概念摘要</strong>
            <span
              style={{
                marginLeft: "auto",
                fontSize: 12,
                color: "var(--text-muted)",
              }}
            >
              AI 生成
            </span>
          </div>

          {conceptBusy && (
            <div style={{ color: "var(--text-secondary)", fontSize: 13, lineHeight: 2 }}>
              <div className="concept-spinner" />
              正在分析文档内容{format === "pdf" || format === "image" ? "（扫描件将自动 OCR 识别）" : ""}，请稍候…
            </div>
          )}

          {!conceptBusy && hasValidConcept && (
            <>
              <div
                className="editor-preview concept-note-view"
                dangerouslySetInnerHTML={{
                  __html: markdownToHtml(conceptNote as string),
                }}
              />
              <button
                className="btn btn-ghost"
                type="button"
                onClick={handleRegenerateConcept}
                style={{ marginTop: 20, fontSize: 12 }}
              >
                ↻ 重新生成
              </button>
            </>
          )}

          {!conceptBusy && !hasValidConcept && (
            <div
              style={{
                border: "1px solid var(--border)",
                borderRadius: 12,
                padding: "28px 24px",
                textAlign: "center",
                color: "var(--text-secondary)",
                background: "var(--bg-secondary, #f7f8fa)",
              }}
            >
              <div style={{ fontSize: 14, marginBottom: 8 }}>
                {isRefusalNote
                  ? "此文档暂未提取到有效文本（可能是扫描件/图片型 PDF）"
                  : "还没有概念摘要"}
              </div>
              <div
                style={{
                  fontSize: 12,
                  color: "var(--text-muted)",
                  marginBottom: 18,
                  lineHeight: 1.8,
                }}
              >
                {isRefusalNote
                  ? "将自动通过 OCR 识别文档文字，再由 AI 生成结构化摘要。"
                  : "由 AI 阅读全文后提炼核心信息，生成结构化概念摘要。"}
              </div>
              <button
                className="btn btn-primary"
                type="button"
                onClick={handleRegenerateConcept}
              >
                ✦ {isRefusalNote ? "OCR 识别并重新生成" : "生成概念摘要"}
              </button>
            </div>
          )}

          {conceptError && (
            <div className="inline-error" style={{ marginTop: 12 }}>
              {conceptError}
            </div>
          )}
        </div>
      </div>
    );
  };

  // 渲染编辑/预览主体
  const renderBody = () => {
    // 只读文档的概念摘要视图（原文 / 概念摘要切换）
    if (supportsConcept && docView === "concept") {
      return renderConceptView();
    }
    // 图片 / PDF：默认折叠为文件卡片，点击在弹窗中预览
    if (format === "image" || format === "pdf") {
      const isImage = format === "image";
      const fileSize = metadata?.fileSize ?? 0;
      const formatLabel = isImage
        ? (metadata?.formatDetail ? metadata.formatDetail.toUpperCase() : "IMAGE")
        : "PDF";
      const cardIcon = isImage ? "🖼️" : "📄";
      return (
        <div
          className="editor-image-view"
          style={{
            flex: 1,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "flex-start",
            padding: 24,
            gap: 16,
            overflow: "auto",
          }}
        >
          <button
            type="button"
            className="image-folded-card"
            onClick={() => setAssetLightboxOpen(true)}
            title={isImage ? "点击查看原图" : "点击预览 PDF"}
          >
            <span className="image-folded-icon">{cardIcon}</span>
            <span className="image-folded-meta">
              <span className="image-folded-name">{metadata?.title ?? (isImage ? "图片" : "PDF 文档")}</span>
              <span className="image-folded-sub">
                {formatLabel} · {fileSize} 字节 · 点击查看
              </span>
            </span>
            <span className="image-folded-arrow" aria-hidden="true">⤢</span>
          </button>

          {!isImage && (
            <button className="btn" type="button" onClick={handleOpenAsset}>
              打开原文件
            </button>
          )}

          {isImage && (
            <>
              <button className="btn" type="button" onClick={handleOcr} disabled={ocrRunning}>
                {ocrRunning ? "正在提取文字..." : "提取图片文字"}
              </button>
              {ocrError && <div className="inline-error">{ocrError}</div>}
              {content.includes("## OCR 文本") && (
                <div
                  className="editor-preview editor-image-ocr"
                  dangerouslySetInnerHTML={{ __html: html }}
                />
              )}
            </>
          )}

          {assetLightboxOpen && (
            <div
              className="image-lightbox-overlay"
              onClick={(e) => {
                if (e.target === e.currentTarget) setAssetLightboxOpen(false);
              }}
            >
              <div className="image-lightbox-header">
                <span className="image-lightbox-title">{metadata?.title}</span>
                <button
                  className="icon-btn image-lightbox-close"
                  title="关闭 (Esc)"
                  onClick={() => setAssetLightboxOpen(false)}
                >
                  <IconClose size={16} />
                </button>
              </div>
              <div className={`image-lightbox-body${isImage ? "" : " is-pdf"}`}>
                {assetBase64 ? (
                  isImage ? (
                    <img
                      src={assetBase64}
                      alt={metadata?.title ?? ""}
                      className="image-lightbox-img"
                    />
                  ) : (
                    <iframe
                      src={assetBase64}
                      className="pdf-lightbox-frame"
                      title={metadata?.title ?? "PDF 预览"}
                    />
                  )
                ) : (
                  <div style={{ color: "rgba(255,255,255,0.7)" }}>
                    {isImage ? "加载图片中..." : "加载 PDF 中..."}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      );
    }

    // 附件：只读视图。docx/xlsx 已抽取文本时优先展示可读内容，否则给出下载入口。
    if (format === "attachment") {
      const hasExtractedText = (content || "").trim().length > 0;
      const typeLabel = metadata?.formatDetail
        ? metadata.formatDetail.toUpperCase()
        : "ATTACHMENT";
      return (
        <div
          className="editor-attachment-view"
          style={{
            flex: 1,
            display: "flex",
            flexDirection: "column",
            alignItems: hasExtractedText ? "flex-start" : "center",
            justifyContent: hasExtractedText ? "flex-start" : "center",
            padding: 24,
            gap: 16,
            overflow: "auto",
          }}
        >
          <div
            style={{
              width: 48,
              height: 48,
              borderRadius: 12,
              background: "var(--bg-secondary)",
              border: "1px solid var(--border)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--text-secondary)",
              fontSize: 22,
            }}
          >
            {hasExtractedText ? "📄" : "📎"}
          </div>
          <div style={{ fontSize: 15, color: "var(--text-secondary)", fontWeight: 500 }}>
            {metadata?.title}
            <span style={{ color: "var(--text-muted)", fontWeight: 400, marginLeft: 8 }}>
              {typeLabel}
              {hasExtractedText ? " · 已提取文本" : ""}
            </span>
          </div>
          {!hasExtractedText && (
            <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
              附件 · {metadata?.fileSize ?? 0} 字节
            </div>
          )}
          {activeTabId && (
            <button
              className="btn btn-primary"
              style={{ marginTop: hasExtractedText ? 4 : 8 }}
              type="button"
              onClick={handleOpenAsset}
            >
              下载/打开原文件
            </button>
          )}
          {hasExtractedText && (
            <div
              className="editor-preview editor-office-preview"
              dangerouslySetInnerHTML={{ __html: html }}
            />
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
          onPasteCapture={handlePaste}
          spellCheck={false}
        />
      ) : (
        <div
          className="editor-preview"
          ref={previewRef}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      );
    }

    // Markdown：默认所见即所得，同时保留 Markdown 源码和只读预览。
    if (mode === "edit") {
      return (
        <div
          ref={richEditorRootRef}
          className="editor-rich"
          onPaste={handlePaste}
          onKeyDownCapture={(event) => {
            if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "j") {
              event.preventDefault();
              openAiPrompt();
            }
          }}
        >
          <EditorContent editor={richEditor} />
        </div>
      );
    }
    return mode === "source" ? (
      <textarea
        ref={textareaRef}
        className="editor-textarea"
        value={content}
        placeholder="在此输入 Markdown 内容..."
        onChange={(e) => handleTextareaChange(e.target.value)}
        onKeyDown={handleTextareaKeyDown}
        onPaste={handlePaste}
        spellCheck={false}
      />
    ) : (
      <div
        className="editor-preview"
        ref={previewRef}
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
        {pasteState !== "idle" && (
          <span className={`editor-paste-status ${pasteState}`} title={pasteError ?? undefined}>
            {pasteState === "loading" ? "正在保存图片..." : pasteError}
          </span>
        )}
        {supportsConcept && (
          <div className="editor-mode-toggle editor-docview-toggle">
            <button
              className={docView === "original" ? "active" : ""}
              onClick={() => setDocView("original")}
              title="查看原始文件"
            >
              原文
            </button>
            <button
              className={docView === "concept" ? "active" : ""}
              onClick={() => setDocView("concept")}
              title="查看 AI 概念摘要"
            >
              <span style={{ color: hasValidConcept ? "var(--accent, #7062f2)" : undefined }}>
                ◈ 概念摘要
              </span>
            </button>
          </div>
        )}
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
