import { useEffect, useMemo, useState } from "react";
import {
  getObjectVersions,
  restoreObjectVersion,
  type ObjectVersion,
} from "../../api";

interface VersionHistoryModalProps {
  objectId: string;
  title: string;
  onClose: () => void;
  onRestored: () => Promise<void>;
}

export function VersionHistoryModal({
  objectId,
  title,
  onClose,
  onRestored,
}: VersionHistoryModalProps) {
  const [versions, setVersions] = useState<ObjectVersion[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [restoring, setRestoring] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getObjectVersions(objectId)
      .then((items) => {
        setVersions(items);
        setSelectedId(items[0]?.operation_id ?? null);
      })
      .catch((reason) => setError(String(reason)))
      .finally(() => setLoading(false));
  }, [objectId]);

  const selected = useMemo(
    () => versions.find((version) => version.operation_id === selectedId) ?? null,
    [selectedId, versions]
  );

  const restore = async () => {
    if (!selected || restoring) return;
    setRestoring(true);
    setError(null);
    try {
      await restoreObjectVersion(objectId, selected.operation_id);
      await onRestored();
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setRestoring(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={(event) => event.target === event.currentTarget && onClose()}>
      <div className="modal version-history-modal">
        <div className="modal-header">版本历史 · {title}</div>
        <div className="modal-body version-history-body">
          <div className="version-history-list">
            {loading && <div className="version-history-empty">正在加载...</div>}
            {!loading && versions.length === 0 && (
              <div className="version-history-empty">还没有可恢复的正文版本</div>
            )}
            {versions.map((version) => (
              <button
                type="button"
                key={version.operation_id}
                className={selectedId === version.operation_id ? "selected" : ""}
                onClick={() => setSelectedId(version.operation_id)}
              >
                <strong>{version.title || "未命名版本"}</strong>
                <span>{new Date(version.timestamp * 1000).toLocaleString()}</span>
              </button>
            ))}
          </div>
          <pre className="version-history-preview">
            {selected?.content ?? "选择一个版本查看正文"}
          </pre>
        </div>
        {error && <div className="modal-error">{error}</div>}
        <div className="modal-footer">
          <button className="btn btn-ghost" type="button" onClick={onClose}>取消</button>
          <button
            className="btn btn-primary"
            type="button"
            onClick={restore}
            disabled={!selected || restoring}
          >
            {restoring ? "正在恢复..." : "恢复此版本"}
          </button>
        </div>
      </div>
    </div>
  );
}
