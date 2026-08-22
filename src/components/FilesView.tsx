import { useEffect, useMemo, useState } from "react";
import type { GroupNode } from "../api";
import { IconFile, IconTrash, objTypeMeta } from "./Icons";

interface FilesViewProps {
  groups: GroupNode[];
  onOpen: (id: string) => void;
  onDelete: (ids: string[]) => Promise<void>;
}

export function FilesView({ groups, onOpen, onDelete }: FilesViewProps) {
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const items = useMemo(
    () => groups.flatMap((group) => group.objects.map((item) => ({ ...item, group: group.name || "未分组" }))),
    [groups]
  );
  const allSelected = items.length > 0 && selectedIds.size === items.length;

  useEffect(() => {
    const existingIds = new Set(items.map((item) => item.id));
    setSelectedIds((previous) => new Set(Array.from(previous).filter((id) => existingIds.has(id))));
  }, [items]);

  const toggleOne = (id: string) => {
    setSelectedIds((previous) => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleAll = () => {
    setSelectedIds(allSelected ? new Set() : new Set(items.map((item) => item.id)));
  };

  const handleDelete = async () => {
    const ids = Array.from(selectedIds);
    if (ids.length === 0) return;
    if (!window.confirm(`确定将选中的 ${ids.length} 个文件移入回收站吗？`)) return;
    setBusy(true);
    try {
      await onDelete(ids);
      setSelectedIds(new Set());
    } catch {
      // App 层已展示具体失败信息；保留失败项选择，方便用户重试。
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="file-management-view">
      <div className="file-management-header">
        <div>
          <div className="file-management-title">全部文件</div>
          <div className="file-management-subtitle">共 {items.length} 个文件，可多选后移入回收站</div>
        </div>
        <button
          className="btn btn-ghost file-danger-button"
          disabled={selectedIds.size === 0 || busy}
          onClick={handleDelete}
        >
          <IconTrash size={13} />
          {busy ? "正在删除..." : `移入回收站${selectedIds.size > 0 ? ` (${selectedIds.size})` : ""}`}
        </button>
      </div>

      {items.length === 0 ? (
        <div className="file-management-empty">
          <IconFile size={26} />
          <div>暂无文件</div>
        </div>
      ) : (
        <div className="file-table-wrap">
          <table className="file-table">
            <thead>
              <tr>
                <th className="file-table-check">
                  <input
                    type="checkbox"
                    checked={allSelected}
                    onChange={toggleAll}
                    disabled={busy}
                    aria-label={allSelected ? "取消全选" : "全选全部文件"}
                  />
                </th>
                <th>名称</th>
                <th>类型</th>
                <th>所在分组</th>
                <th>创建方式</th>
              </tr>
            </thead>
            <tbody>
              {items.map((item) => {
                const typeMeta = objTypeMeta[item.type];
                const Icon = typeMeta?.icon ?? IconFile;
                return (
                  <tr key={item.id} className={selectedIds.has(item.id) ? "selected" : ""}>
                    <td className="file-table-check">
                      <input
                        type="checkbox"
                        checked={selectedIds.has(item.id)}
                        onChange={() => toggleOne(item.id)}
                        disabled={busy}
                        aria-label={`选择 ${item.title || "无标题"}`}
                      />
                    </td>
                    <td>
                      <button className="file-name-button" disabled={busy} onClick={() => onOpen(item.id)}>
                        <span style={{ color: typeMeta?.color ?? "var(--text-secondary)" }}>
                          <Icon size={14} />
                        </span>
                        <span title={item.title || "无标题"}>{item.title || "无标题"}</span>
                      </button>
                    </td>
                    <td>{typeMeta?.label ?? item.type} · {item.format}</td>
                    <td>{item.group}</td>
                    <td>{item.created_by === "agent" ? "Agent" : "手动"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
