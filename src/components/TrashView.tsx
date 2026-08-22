import { useCallback, useEffect, useState } from "react";
import type { ObjectSummary } from "../api";
import { getTrashObjects, permanentDelete, restoreObject } from "../api";
import type { ToastType } from "./Toast";
import { IconFile, IconTrash, objTypeMeta } from "./Icons";

interface TrashViewProps {
  onRefresh?: () => void;
  onNotify?: (type: ToastType, message: string) => void;
}

export function TrashView({ onRefresh, onNotify }: TrashViewProps) {
  const [items, setItems] = useState<ObjectSummary[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const result = await getTrashObjects();
      setItems(result);
      setSelectedIds((previous) => {
        const existing = new Set(result.map((item) => item.id));
        return new Set(Array.from(previous).filter((id) => existing.has(id)));
      });
    } catch (error) {
      setItems([]);
      onNotify?.("error", `加载回收站失败: ${error}`);
    } finally {
      setLoading(false);
    }
  }, [onNotify]);

  useEffect(() => {
    load();
  }, [load]);

  const finishMutation = async () => {
    await load();
    onRefresh?.();
  };

  const handleRestore = async (id: string) => {
    setBusyId(id);
    try {
      await restoreObject(id);
      await finishMutation();
      onNotify?.("success", "文件已恢复");
    } catch (error) {
      onNotify?.("error", `恢复失败: ${error}`);
    } finally {
      setBusyId(null);
    }
  };

  const handlePermanentDelete = async (id: string) => {
    if (!window.confirm("永久删除后将无法恢复，确定继续吗？")) return;
    setBusyId(id);
    try {
      await permanentDelete(id);
      await finishMutation();
      onNotify?.("success", "文件已永久删除");
    } catch (error) {
      onNotify?.("error", `永久删除失败: ${error}`);
    } finally {
      setBusyId(null);
    }
  };

  const runBatch = async (action: "restore" | "delete") => {
    const ids = Array.from(selectedIds);
    if (ids.length === 0) return;
    if (
      action === "delete" &&
      !window.confirm(`将永久删除选中的 ${ids.length} 个文件，此操作无法恢复。确定继续吗？`)
    ) return;

    setBusy(true);
    try {
      const operation = action === "restore" ? restoreObject : permanentDelete;
      let successCount = 0;
      for (const id of ids) {
        try {
          // 恢复和永久删除都会写知识库，顺序执行可避免多个文件争用同一写锁。
          await operation(id);
          successCount += 1;
        } catch {
          // 继续处理其余选择项，结束后统一反馈失败数量。
        }
      }
      const failedCount = ids.length - successCount;
      await finishMutation();
      if (failedCount > 0) {
        onNotify?.(
          "error",
          `${action === "restore" ? "恢复" : "永久删除"}完成 ${successCount} 个，失败 ${failedCount} 个`
        );
      } else {
        onNotify?.(
          "success",
          action === "restore" ? `已恢复 ${successCount} 个文件` : `已永久删除 ${successCount} 个文件`
        );
      }
    } finally {
      setBusy(false);
    }
  };

  const allSelected = items.length > 0 && selectedIds.size === items.length;
  const mutationBusy = busy || busyId !== null;
  const toggleAll = () => {
    setSelectedIds(allSelected ? new Set() : new Set(items.map((item) => item.id)));
  };
  const toggleOne = (id: string) => {
    setSelectedIds((previous) => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  if (loading && items.length === 0) {
    return (
      <div className="editor-empty">
        <div className="editor-empty-icon"><div className="graph-loading-spinner" /></div>
        <div className="editor-empty-title">加载回收站中...</div>
      </div>
    );
  }

  if (items.length === 0) {
    return (
      <div className="editor-empty">
        <div className="editor-empty-icon"><IconTrash size={26} /></div>
        <div className="editor-empty-title">回收站为空</div>
        <div className="editor-empty-hint">从“文件”视图删除的内容将出现在这里</div>
      </div>
    );
  }

  return (
    <div className="file-management-view">
      <div className="file-management-header">
        <div>
          <div className="file-management-title">回收站</div>
          <div className="file-management-subtitle">共 {items.length} 个文件，可恢复或永久删除</div>
        </div>
        <div className="trash-batch-actions">
          <button
            className="btn btn-ghost"
            disabled={selectedIds.size === 0 || mutationBusy}
            onClick={() => runBatch("restore")}
          >
            {busy ? "处理中..." : `批量恢复${selectedIds.size > 0 ? ` (${selectedIds.size})` : ""}`}
          </button>
          <button
            className="btn btn-ghost file-danger-button"
            disabled={selectedIds.size === 0 || mutationBusy}
            onClick={() => runBatch("delete")}
          >
            <IconTrash size={13} />
            永久删除{selectedIds.size > 0 ? ` (${selectedIds.size})` : ""}
          </button>
        </div>
      </div>

      <div className="file-table-wrap">
        <table className="file-table trash-table">
          <thead>
            <tr>
              <th className="file-table-check">
                <input
                  type="checkbox"
                  checked={allSelected}
                  onChange={toggleAll}
                  disabled={mutationBusy}
                  aria-label={allSelected ? "取消全选" : "全选回收站文件"}
                />
              </th>
              <th>名称</th>
              <th>类型</th>
              <th>创建方式</th>
              <th className="trash-actions-column">操作</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => {
              const typeMeta = objTypeMeta[item.type];
              const Icon = typeMeta?.icon ?? IconFile;
              const itemBusy = mutationBusy;
              return (
                <tr key={item.id} className={selectedIds.has(item.id) ? "selected" : ""}>
                  <td className="file-table-check">
                    <input
                      type="checkbox"
                      checked={selectedIds.has(item.id)}
                      onChange={() => toggleOne(item.id)}
                      disabled={mutationBusy}
                      aria-label={`选择 ${item.title || "无标题"}`}
                    />
                  </td>
                  <td>
                    <div className="file-name-button">
                      <span style={{ color: typeMeta?.color ?? "var(--text-secondary)" }}>
                        <Icon size={14} />
                      </span>
                      <span title={item.title || "无标题"}>{item.title || "无标题"}</span>
                    </div>
                  </td>
                  <td>{typeMeta?.label ?? item.type} · {item.format}</td>
                  <td>{item.created_by === "agent" ? "Agent" : "手动"}</td>
                  <td className="trash-row-actions">
                    <button className="btn btn-ghost" disabled={itemBusy} onClick={() => handleRestore(item.id)}>
                      恢复
                    </button>
                    <button
                      className="btn btn-ghost"
                      style={{ color: "var(--red)" }}
                      disabled={itemBusy}
                      onClick={() => handlePermanentDelete(item.id)}
                    >
                      永久删除
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
