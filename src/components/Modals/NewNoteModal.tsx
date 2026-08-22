import { useEffect, useRef, useState } from "react";
import type { ObjType } from "../../mock/data";
import { IconFolder, objTypeMeta } from "../Icons";

export type NewItemType = ObjType | "folder";

export interface NewNoteResult {
  title: string;
  type: NewItemType;
}

interface NewNoteModalProps {
  onClose: () => void;
  onConfirm: (result: NewNoteResult) => void;
  targetGroup?: string;
  initialType?: NewItemType;
}

const types: NewItemType[] = ["concept", "project", "code", "document", "inbox", "folder"];

export function NewNoteModal({
  onClose,
  onConfirm,
  targetGroup,
  initialType = "concept",
}: NewNoteModalProps) {
  const [title, setTitle] = useState("");
  const [type, setType] = useState<NewItemType>(initialType);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  const submit = () => {
    const t = title.trim();
    if (!t) return;
    onConfirm({ title: t, type });
  };

  return (
    <div
      className="modal-overlay"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal" style={{ minWidth: 460 }}>
        <div className="modal-header">新建</div>
        <div className="modal-body">
          {targetGroup && type !== "folder" && (
            <div className="new-item-target">创建到：{targetGroup}</div>
          )}
          <div className="modal-label">选择类型</div>
          <div className="type-grid">
            {types.map((t) => {
              const meta = t === "folder" ? null : objTypeMeta[t];
              const Icon = t === "folder" ? IconFolder : meta?.icon;
              return (
                <button
                  key={t}
                  className={`type-option${type === t ? " selected" : ""}`}
                  onClick={() => setType(t)}
                >
                  <span className="type-option-icon">
                    {Icon && <Icon size={18} />}
                  </span>
                  <span>{t === "folder" ? "文件夹" : (meta?.label ?? t)}</span>
                </button>
              );
            })}
          </div>
          <div className="modal-label">{type === "folder" ? "文件夹名称" : "标题"}</div>
          <input
            ref={inputRef}
            className="modal-input"
            placeholder={type === "folder" ? "输入文件夹名称..." : "输入笔记标题..."}
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={handleKey}
          />
        </div>
        <div className="modal-footer">
          <button className="btn btn-ghost" onClick={onClose}>
            取消 (Esc)
          </button>
          <button className="btn btn-primary" onClick={submit} disabled={!title.trim()}>
            创建 (Enter)
          </button>
        </div>
      </div>
    </div>
  );
}
