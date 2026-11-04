import { useState } from "react";

interface ImportUrlModalProps {
  groups: string[];
  onClose: () => void;
  onConfirm: (url: string, group: string) => Promise<void>;
}

export function ImportUrlModal({ groups, onClose, onConfirm }: ImportUrlModalProps) {
  const [url, setUrl] = useState("");
  const [group, setGroup] = useState(groups[0] || "收件箱");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!url.trim() || loading) return;
    setLoading(true);
    setError(null);
    try {
      await onConfirm(url.trim(), group);
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={(event) => event.target === event.currentTarget && onClose()}>
      <div className="modal" style={{ minWidth: 460 }}>
        <div className="modal-header">导入网页</div>
        <div className="modal-body">
          <label className="settings-ai-field">
            <span>网页地址</span>
            <input
              autoFocus
              type="url"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              onKeyDown={(event) => event.key === "Enter" && submit()}
              placeholder="https://example.com/article"
            />
          </label>
          <label className="settings-ai-field">
            <span>保存到</span>
            <select value={group} onChange={(event) => setGroup(event.target.value)}>
              {Array.from(new Set(["收件箱", ...groups])).map((item) => (
                <option key={item} value={item}>{item}</option>
              ))}
            </select>
          </label>
          {error && <div className="modal-error">{error}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-ghost" type="button" onClick={onClose}>取消</button>
          <button className="btn btn-primary" type="button" onClick={submit} disabled={!url.trim() || loading}>
            {loading ? "正在抓取..." : "导入"}
          </button>
        </div>
      </div>
    </div>
  );
}
