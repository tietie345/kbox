import { useEffect, useMemo, useRef, useState } from "react";
import { IconSearch } from "../Icons";

const RELATION_TYPES: { value: string; label: string }[] = [
  { value: "depends_on", label: "依赖 (depends_on)" },
  { value: "related_to", label: "相关 (related_to)" },
  { value: "part_of", label: "属于 (part_of)" },
  { value: "implements", label: "实现 (implements)" },
];

interface RelationModalProps {
  sourceId: string;
  sourceTitle: string;
  objects: { id: string; title: string; type: string }[];
  onConfirm: (targetId: string, relationType: string) => void;
  onClose: () => void;
}

export function RelationModal({
  sourceId,
  sourceTitle,
  objects,
  onConfirm,
  onClose,
}: RelationModalProps) {
  const [query, setQuery] = useState("");
  const [targetId, setTargetId] = useState<string | null>(null);
  const [relationType, setRelationType] = useState<string>("related_to");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // 排除源对象
  const candidates = useMemo(() => {
    return objects.filter((o) => o.id !== sourceId);
  }, [objects, sourceId]);

  const filtered = useMemo(() => {
    if (!query.trim()) return candidates;
    const q = query.toLowerCase();
    return candidates.filter((o) => o.title.toLowerCase().includes(q));
  }, [candidates, query]);

  const handleKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (targetId) submit();
    }
  };

  const submit = () => {
    if (!targetId) return;
    onConfirm(targetId, relationType);
  };

  return (
    <div
      className="modal-overlay"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal" style={{ minWidth: 480, maxWidth: 560 }}>
        <div className="modal-header">添加关系</div>
        <div className="modal-body">
          <div className="modal-label">源对象</div>
          <div
            style={{
              padding: "6px 10px",
              background: "var(--bg-tertiary)",
              border: "1px solid var(--border-soft)",
              borderRadius: 6,
              fontSize: 12,
              color: "var(--text-primary)",
              marginBottom: 12,
            }}
          >
            {sourceTitle}
          </div>

          <div className="modal-label">选择目标对象</div>
          <div className="relation-search-wrap">
            <span className="search-icon">
              <IconSearch size={12} />
            </span>
            <input
              ref={inputRef}
              className="modal-input"
              style={{ paddingLeft: 28 }}
              placeholder="搜索对象..."
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={handleKey}
            />
          </div>

          <div className="relation-list">
            {filtered.length === 0 && (
              <div
                style={{
                  padding: "16px 10px",
                  textAlign: "center",
                  color: "var(--text-muted)",
                  fontSize: 12,
                }}
              >
                未找到匹配的对象
              </div>
            )}
            {filtered.map((o) => (
              <div
                key={o.id}
                className={`relation-list-item${
                  targetId === o.id ? " selected" : ""
                }`}
                onClick={() => setTargetId(o.id)}
              >
                <span className="relation-list-type">[{o.type}]</span>
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {o.title}
                </span>
              </div>
            ))}
          </div>

          <div className="modal-label" style={{ marginTop: 12 }}>
            关系类型
          </div>
          <div className="relation-type-grid">
            {RELATION_TYPES.map((rt) => (
              <button
                key={rt.value}
                className={`type-option${relationType === rt.value ? " selected" : ""}`}
                style={{ padding: "6px 8px", fontSize: 11 }}
                onClick={() => setRelationType(rt.value)}
              >
                {rt.label}
              </button>
            ))}
          </div>
        </div>
        <div className="modal-footer">
          <button className="btn btn-ghost" onClick={onClose}>
            取消 (Esc)
          </button>
          <button
            className="btn btn-primary"
            onClick={submit}
            disabled={!targetId}
          >
            确认 (Enter)
          </button>
        </div>
      </div>
    </div>
  );
}
