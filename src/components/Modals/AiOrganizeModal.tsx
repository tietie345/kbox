import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  cancelAiTask,
  getAiTaskActions,
  getAiTasks,
  startAiOrganize,
  type AiOrganizeResult,
  type AiTask,
  type AiTaskEvent,
} from "../../api";

interface AiOrganizeModalProps {
  onClose: () => void;
  collectionId?: string;
  collectionPath?: string;
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
  collectionId,
  collectionPath,
}: AiOrganizeModalProps) {
  const [instruction, setInstruction] = useState(
    "根据项目和主题重新建立清晰的结构树，把完成或过期的内容放入归档。"
  );
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const activeTaskIdRef = useRef<string | null>(null);
  const [running, setRunning] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [actions, setActions] = useState<string[]>([]);
  const [result, setResult] = useState<AiOrganizeResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [history, setHistory] = useState<AiTask[]>([]);
  const [selectedTask, setSelectedTask] = useState<AiTask | null>(null);
  const [showHistory, setShowHistory] = useState(false);

  const loadHistory = useCallback(async () => {
    setHistory(await getAiTasks(30));
  }, []);

  const openTask = useCallback(async (task: AiTask) => {
    activeTaskIdRef.current = task.id;
    setActiveTaskId(task.id);
    setSelectedTask(task);
    setRunning(task.status === "running");
    setStopping(false);
    setResult(null);
    setError(task.status === "failed" ? task.summary : null);
    const taskActions = await getAiTaskActions(task.id);
    setActions(taskActions.map((action) => action.description));
  }, []);

  useEffect(() => {
    loadHistory().catch((e) => setError(String(e)));
  }, [loadHistory]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    listen<AiTaskEvent>("ai-task-event", (event) => {
      const payload = event.payload;
      if (payload.task_id === activeTaskIdRef.current) {
        if (payload.kind === "action") {
          setActions((current) => current[current.length - 1] === payload.description
            ? current
            : [...current, payload.description]);
        } else if (payload.kind === "completed") {
          setRunning(false);
          setStopping(false);
          setResult(payload.result);
        } else if (payload.kind === "failed") {
          setRunning(false);
          setStopping(false);
          setError(payload.description);
        } else if (payload.kind === "cancelled") {
          setRunning(false);
          setStopping(false);
          setError(null);
          setActions((current) => [...current, "任务已停止"]);
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

  const run = async () => {
    if (!instruction.trim() || running) return;
    setRunning(true);
    setStopping(false);
    setActions([]);
    setResult(null);
    setSelectedTask(null);
    setError(null);
    try {
      const taskId = await startAiOrganize(instruction.trim(), collectionId);
      activeTaskIdRef.current = taskId;
      setActiveTaskId(taskId);
      const [tasks, taskActions] = await Promise.all([
        getAiTasks(30),
        getAiTaskActions(taskId),
      ]);
      setHistory(tasks);
      setActions(taskActions.map((action) => action.description));
      const task = tasks.find((item) => item.id === taskId);
      setSelectedTask(task ?? null);
      if (task && task.status !== "running") {
        setRunning(false);
        if (task.status === "failed") setError(task.summary);
      }
    } catch (e: any) {
      setError(String(e));
      setRunning(false);
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
      setStopping(false);
      setError(String(e));
    }
  };

  return (
    <div className="modal-overlay" onClick={(event) => {
      if (event.target === event.currentTarget) onClose();
    }}>
      <div className="modal ai-organize-modal" role="dialog" aria-modal="true">
        <div className="modal-header ai-organize-header">
          <div>
            <div className="ai-organize-title">✦ AI 整理{collectionPath ? `：${collectionPath}` : "知识库"}</div>
            <div className="settings-desc">
              {collectionPath
                ? "仅整理该目录及子目录；关闭窗口后任务仍会继续"
                : "只有点击执行后才会调用模型；关闭窗口后任务仍会继续"}
            </div>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">×</button>
        </div>
        <div className="modal-body ai-organize-body">
          <div className="ai-organize-tabs">
            <button className={!showHistory ? "active" : ""} onClick={() => setShowHistory(false)}>当前任务</button>
            <button className={showHistory ? "active" : ""} onClick={() => setShowHistory(true)}>历史记录</button>
          </div>

          {showHistory ? (
            <div className="ai-task-history">
              {history.length === 0 ? (
                <div className="ai-task-empty">还没有 AI 整理记录</div>
              ) : history.map((task) => (
                <button className="ai-task-history-item" key={task.id} onClick={() => {
                  openTask(task).then(() => setShowHistory(false)).catch((e) => setError(String(e)));
                }}>
                  <span className={`ai-task-status ${task.status}`}>{statusLabel[task.status]}</span>
                  <span className="ai-task-history-main">
                    <strong>{task.instruction}</strong>
                    <small>{task.scope.startsWith("collection:") ? "指定目录" : "整个知识库"} · {task.model}</small>
                  </span>
                  <time>{new Date(task.started_at * 1000).toLocaleString()}</time>
                </button>
              ))}
            </div>
          ) : (
            <>
              <label className="modal-label" htmlFor="ai-organize-instruction">整理要求</label>
              <textarea
                id="ai-organize-instruction"
                value={instruction}
                onChange={(event) => setInstruction(event.target.value)}
                disabled={running}
                placeholder="描述希望使用的目录逻辑和归档标准"
              />
              {(running || actions.length > 0) && (
                <div className="ai-task-log">
                  <div className="ai-task-log-title">
                    {running ? (stopping ? "正在停止" : "正在执行") : "执行记录"}
                    {running && <span className="ai-task-running-dot" />}
                  </div>
                  {actions.length === 0 ? (
                    <div className="ai-task-empty">正在等待模型生成结构方案…</div>
                  ) : actions.map((action, index) => (
                    <div className="ai-task-action" key={`${index}-${action}`}>
                      <span>✓</span><span>{action}</span>
                    </div>
                  ))}
                </div>
              )}
              {result && (
                <div className="ai-organize-result">
                  <strong>{result.summary}</strong>
                  <span>目录 {result.created_directories} · 移动 {result.moved_objects} · 归档 {result.archived_objects} · 清理空目录 {result.removed_directories}</span>
                </div>
              )}
              {!result && selectedTask && !running && selectedTask.summary && !error && (
                <div className="ai-organize-result">
                  <strong>{selectedTask.summary}</strong>
                  <span>{statusLabel[selectedTask.status]} · {new Date(selectedTask.started_at * 1000).toLocaleString()}</span>
                </div>
              )}
              {error && <div className="editor-ai-error">{error}</div>}
            </>
          )}
        </div>
        <div className="modal-footer">
          <button className="btn btn-ghost" onClick={onClose}>{running ? "关闭并后台执行" : "关闭"}</button>
          {!showHistory && (running ? (
            <button className="btn btn-danger" onClick={stop} disabled={stopping}>{stopping ? "正在停止…" : "停止任务"}</button>
          ) : (
            <button className="btn btn-primary" onClick={run} disabled={!instruction.trim()}>开始整理</button>
          ))}
        </div>
      </div>
    </div>
  );
}
