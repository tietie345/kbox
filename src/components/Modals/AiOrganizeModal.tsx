import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  cancelAiTask,
  clearAiTasks,
  deleteAiTask,
  getAiConfig,
  getAiTaskActions,
  getAiTasks,
  startAiOrganize,
  type AiConversationTurn,
  type AiOrganizeResult,
  type AiTask,
  type AiTaskEvent,
} from "../../api";
import { markdownToHtml } from "../../utils/markdown";

interface AiOrganizeModalProps {
  onClose: () => void;
  onOpenSettings: () => void;
  collectionId?: string;
  collectionPath?: string;
}

interface ConversationMessage {
  role: "user" | "assistant";
  text: string;
  pending?: boolean;
  error?: boolean;
}

function updatePendingAssistant(
  messages: ConversationMessage[],
  text: string,
  mode: "append" | "replace",
  error = false,
): ConversationMessage[] {
  const next = [...messages];
  for (let index = next.length - 1; index >= 0; index -= 1) {
    const message = next[index];
    if (message.role === "assistant" && message.pending) {
      next[index] = {
        role: "assistant",
        text: mode === "append" ? `${message.text}${text}`.slice(-24_000) : text,
        pending: mode === "append",
        error,
      };
      return next;
    }
  }
  return [...next, { role: "assistant", text, pending: mode === "append", error }];
}

/**
 * 终态事件（completed/failed/cancelled）使用：仅当还存在 pending 的 assistant
 * 消息时才写入最终结果。快速完成的任务可能已被 run() 中的任务轮询写入最终
 * summary（pending 已置为 false），此时不再追加，否则会产生重复气泡。
 */
function finalizePendingAssistant(
  messages: ConversationMessage[],
  text: string,
  error = false,
): ConversationMessage[] {
  const next = [...messages];
  for (let index = next.length - 1; index >= 0; index -= 1) {
    const message = next[index];
    if (message.role === "assistant" && message.pending) {
      next[index] = { role: "assistant", text, pending: false, error };
      return next;
    }
  }
  return next;
}

const statusLabel: Record<AiTask["status"], string> = {
  running: "执行中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已停止",
  interrupted: "已中断",
};

export function AiOrganizeModal({
  onClose,
  onOpenSettings,
  collectionId,
  collectionPath,
}: AiOrganizeModalProps) {
  const [instruction, setInstruction] = useState("");
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const activeTaskIdRef = useRef<string | null>(null);
  const [running, setRunning] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [actions, setActions] = useState<string[]>([]);
  const [result, setResult] = useState<AiOrganizeResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [history, setHistory] = useState<AiTask[]>([]);
  const [showHistory, setShowHistory] = useState(false);
  const [conversation, setConversation] = useState<ConversationMessage[]>([]);
  const conversationRef = useRef<HTMLDivElement | null>(null);
  const [apiConfigured, setApiConfigured] = useState<boolean | null>(null);
  const [checkingConfig, setCheckingConfig] = useState(false);

  const refreshAiConfig = useCallback(async () => {
    setCheckingConfig(true);
    try {
      const config = await getAiConfig();
      setApiConfigured(Boolean(
        (config.auth_mode === "none" || config.api_key_configured) &&
        config.base_url.trim() &&
        config.model.trim(),
      ));
    } catch {
      setApiConfigured(false);
    } finally {
      setCheckingConfig(false);
    }
  }, []);

  const loadHistory = useCallback(async () => {
    setHistory(await getAiTasks(30));
  }, []);

  // 删除单条历史（运行中的任务不可删）
  const handleDeleteTask = useCallback(
    async (task: AiTask, event: React.MouseEvent) => {
      event.stopPropagation();
      if (task.status === "running") return;
      const ok = await confirm(`确定删除这条记录？\n「${task.instruction}」`, {
        title: "删除历史记录",
        kind: "warning",
      });
      if (!ok) return;
      try {
        await deleteAiTask(task.id);
        await loadHistory();
      } catch (e) {
        setError(String(e));
      }
    },
    [loadHistory],
  );

  // 清空全部已结束历史
  const handleClearAll = useCallback(async () => {
    const finished = history.filter((t) => t.status !== "running").length;
    if (finished === 0) return;
    const ok = await confirm(
      `确定清空全部 ${finished} 条已结束的历史记录？此操作不可恢复。`,
      { title: "清空历史记录", kind: "warning" },
    );
    if (!ok) return;
    try {
      await clearAiTasks();
      await loadHistory();
    } catch (e) {
      setError(String(e));
    }
  }, [history, loadHistory]);

  const openTask = useCallback(async (task: AiTask) => {
    activeTaskIdRef.current = task.id;
    setActiveTaskId(task.id);
    setRunning(task.status === "running");
    setStopping(false);
    setResult(null);
    setError(task.status === "failed" ? task.summary : null);
    setConversation([
      { role: "user", text: task.instruction },
      {
        role: "assistant",
        text: task.status === "running" ? "" : task.summary,
        pending: task.status === "running",
        error: task.status === "failed",
      },
    ]);
    const taskActions = await getAiTaskActions(task.id);
    setActions(taskActions.map((action) => action.description));
  }, []);

  useEffect(() => {
    loadHistory().catch((e) => setError(String(e)));
    refreshAiConfig().catch(() => setApiConfigured(false));
  }, [loadHistory, refreshAiConfig]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    listen<AiTaskEvent>("ai-task-event", (event) => {
      const payload = event.payload;
      if (payload.task_id === activeTaskIdRef.current) {
        if (payload.kind === "stream") {
          setConversation((current) =>
            updatePendingAssistant(current, payload.description, "append")
          );
        } else if (payload.kind === "action") {
          setActions((current) => current.includes(payload.description)
            ? current
            : [...current, payload.description]);
        } else if (payload.kind === "completed") {
          setRunning(false);
          setStopping(false);
          setResult(payload.result);
          setConversation((current) =>
            finalizePendingAssistant(current, payload.description || "任务已完成。")
          );
        } else if (payload.kind === "failed") {
          setRunning(false);
          setStopping(false);
          setError(payload.description);
          setConversation((current) =>
            finalizePendingAssistant(current, `执行失败：${payload.description}`, true)
          );
        } else if (payload.kind === "cancelled") {
          setRunning(false);
          setStopping(false);
          setError(null);
          setActions((current) => [...current, "任务已停止"]);
          setConversation((current) =>
            finalizePendingAssistant(current, "任务已停止，已完成的动作保持不变。")
          );
        }
      }
      if (["completed", "failed", "cancelled"].includes(payload.kind)) {
        loadHistory().catch(() => undefined);
      }
    }).then((unlisten) => {
      dispose = unlisten;
    });
    return () => dispose?.();
  }, [loadHistory]);

  useEffect(() => {
    const node = conversationRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [conversation, actions, result, running]);

  const run = async () => {
    if (!instruction.trim() || running || apiConfigured !== true) return;
    const prompt = instruction.trim();
    const conversationContext: AiConversationTurn[] = conversation
      .filter((message) => !message.pending && message.text.trim())
      .slice(-12)
      .map((message) => ({ role: message.role, content: message.text }));
    setRunning(true);
    setStopping(false);
    setActions([]);
    setResult(null);
    setConversation((current) => [
      ...current,
      { role: "user", text: prompt },
      { role: "assistant", text: "", pending: true },
    ]);
    setInstruction("");
    setError(null);
    try {
      const taskId = await startAiOrganize(prompt, collectionId, conversationContext);
      activeTaskIdRef.current = taskId;
      setActiveTaskId(taskId);
      const [tasks, taskActions] = await Promise.all([
        getAiTasks(30),
        getAiTaskActions(taskId),
      ]);
      setHistory(tasks);
      setActions(taskActions.map((action) => action.description));
      const task = tasks.find((item) => item.id === taskId);
      if (task && task.status !== "running") {
        setRunning(false);
        if (task.status === "failed") setError(task.summary);
        if (task.summary) {
          setConversation((current) =>
            updatePendingAssistant(current, task.summary, "replace", task.status === "failed")
          );
        }
      }
    } catch (e: any) {
      const message = String(e);
      setError(message);
      setRunning(false);
      setConversation((current) =>
        updatePendingAssistant(current, `执行失败：${message}`, "replace", true)
      );
    }
  };

  const stop = async () => {
    if (!activeTaskId || !running || stopping) return;
    setStopping(true);
    try {
      const accepted = await cancelAiTask(activeTaskId);
      if (!accepted) {
        setRunning(false);
        setStopping(false);
        await loadHistory();
      }
    } catch (e: any) {
      const message = String(e);
      setStopping(false);
      setError(message);
      setConversation((current) => [
        ...current,
        { role: "assistant", text: `停止任务失败：${message}`, error: true },
      ]);
    }
  };

  return (
    <div className="ai-assistant-backdrop" onClick={(event) => {
      if (event.target === event.currentTarget) onClose();
    }}>
      <aside className="ai-assistant-panel" role="dialog" aria-modal="true" aria-label="KBOX AI 助手">
        <div className="ai-assistant-header ai-organize-header">
          <div>
            <div className="ai-organize-title">✦ KBOX AI 助手</div>
            <div className="settings-desc">
              {collectionPath
                ? `当前范围：${collectionPath} 及其子目录`
                : "通过对话检索、整理和维护知识库；只有发送后才会调用模型"}
            </div>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">×</button>
        </div>
        <div className="ai-assistant-body ai-organize-body">
          <div className="ai-organize-tabs">
            <button className={!showHistory ? "active" : ""} onClick={() => setShowHistory(false)}>当前任务</button>
            <button className={showHistory ? "active" : ""} onClick={() => setShowHistory(true)}>历史记录</button>
          </div>

          {showHistory ? (
            <div className="ai-task-history">
              {history.length > 0 && (
                <div className="ai-task-history-toolbar">
                  <span className="ai-task-history-count">{history.length} 条记录</span>
                  <button
                    className="ai-task-clear-all"
                    onClick={handleClearAll}
                    disabled={!history.some((t) => t.status !== "running")}
                  >
                    清空全部
                  </button>
                </div>
              )}
              {history.length === 0 ? (
                <div className="ai-task-empty">还没有 AI 整理记录</div>
              ) : history.map((task) => (
                <div
                  className="ai-task-history-item"
                  key={task.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    openTask(task).then(() => setShowHistory(false)).catch((e) => setError(String(e)));
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      openTask(task).then(() => setShowHistory(false)).catch((err) => setError(String(err)));
                    }
                  }}
                >
                  <span className={`ai-task-status ${task.status}`}>{statusLabel[task.status]}</span>
                  <span className="ai-task-history-main">
                    <strong>{task.instruction}</strong>
                    <small>{task.scope.startsWith("collection:") ? "指定目录" : "整个知识库"} · {task.model}</small>
                  </span>
                  <time>{new Date(task.started_at * 1000).toLocaleString()}</time>
                  {task.status !== "running" && (
                    <button
                      className="ai-task-delete"
                      title="删除此记录"
                      aria-label="删除此记录"
                      onClick={(e) => handleDeleteTask(task, e)}
                    >
                      ×
                    </button>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <>
              {apiConfigured !== true && (
                <div className="ai-config-warning" role="status">
                  <div className="ai-config-warning-icon">!</div>
                  <div className="ai-config-warning-content">
                    <strong>{checkingConfig ? "正在检查 AI 配置…" : "需要先配置 AI"}</strong>
                    <span>请先在设置中选择模型服务并配置接口地址、模型和认证信息，配置完成后才能使用整理、检索和维护功能。</span>
                    <div className="ai-config-warning-actions">
                      <button className="btn btn-primary btn-sm" onClick={onOpenSettings}>打开设置</button>
                      <button className="btn btn-ghost btn-sm" onClick={refreshAiConfig} disabled={checkingConfig}>重新检查</button>
                    </div>
                  </div>
                </div>
              )}
              <div className="ai-conversation" aria-live="polite" ref={conversationRef}>
                {conversation.length === 0 ? (
                  <div className="ai-conversation-empty">
                    <div className="ai-conversation-empty-icon">✦</div>
                    <strong>有什么可以帮你？</strong>
                    <span>可以检索、分析并使用工具整理知识库，也可以基于上一轮结果继续追问。</span>
                  </div>
                ) : conversation.map((message, index) => (
                  <div className={`ai-message-row ${message.role}`} key={`${index}-${message.text.slice(0, 12)}`}>
                    {message.role === "assistant" && <div className="ai-message-avatar">✦</div>}
                    <div className={`ai-message ${message.role}${message.error ? " error" : ""}`}>
                      {message.pending && !message.text ? (
                        <span className="ai-thinking"><i /><i /><i /></span>
                      ) : message.role === "assistant" ? (
                        <div
                          className="ai-message-markdown"
                          dangerouslySetInnerHTML={{ __html: markdownToHtml(message.text) }}
                        />
                      ) : message.text}
                    </div>
                  </div>
                ))}
                {(running || actions.length > 0) && (
                  <details className="ai-agent-activity" open={running}>
                    <summary>
                      <span className={`ai-agent-status-dot${running ? " running" : ""}`} />
                      <strong>{running ? (stopping ? "正在停止 Agent" : "Agent 正在执行") : "Agent 执行记录"}</strong>
                      <span>{actions.length > 0 ? `${actions.length} 个动作` : "准备工具"}</span>
                    </summary>
                    <div className="ai-agent-actions">
                      {actions.length === 0 ? (
                        <div className="ai-task-empty">正在分析知识库并准备工具调用…</div>
                      ) : actions.map((action, index) => (
                        <div className="ai-task-action" key={`${index}-${action}`}>
                          <span>✓</span><span>{action}</span>
                        </div>
                      ))}
                    </div>
                  </details>
                )}
                {error && !conversation.some((message) => message.error) && (
                  <div className="editor-ai-error">{error}</div>
                )}
              </div>
              <div className="ai-chat-composer">
                <textarea
                  id="ai-organize-instruction"
                  rows={2}
                  value={instruction}
                  onChange={(event) => setInstruction(event.target.value)}
                  onKeyDown={(event) => {
                    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                      event.preventDefault();
                      void run();
                    }
                  }}
                  disabled={running || apiConfigured === false}
                  placeholder={apiConfigured === false
                    ? "完成模型服务配置后即可对话"
                    : running ? "Agent 正在执行任务…" : "继续对话，或告诉 Agent 下一步做什么…"}
                />
                <div className="ai-chat-composer-footer">
                  <span>{collectionPath ? `范围：${collectionPath}` : "范围：整个知识库"} · ⌘↵ 发送</span>
                  {running ? (
                    <button className="ai-chat-stop" onClick={stop} disabled={stopping}>
                      {stopping ? "正在停止…" : "停止"}
                    </button>
                  ) : (
                    <button
                      className="ai-chat-send"
                      onClick={run}
                      disabled={apiConfigured !== true || !instruction.trim()}
                      aria-label="发送"
                    >↑</button>
                  )}
                </div>
              </div>
            </>
          )}
        </div>
      </aside>
    </div>
  );
}
