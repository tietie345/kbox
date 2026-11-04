import { useEffect, useRef, useState } from "react";
import type { ObjType } from "../../mock/data";
import { IconFolder, IconChevronDown, objTypeMeta } from "../Icons";

export type NewItemType = ObjType | "folder";

export interface NewItemLocation {
  /** 唯一标识：收件箱/分组用组名，collection 用 collection id */
  id: string;
  /** 下拉中显示的名称 */
  label: string;
  /** 若为 collection，传入其 id；收件箱/普通分组为 undefined */
  collectionId?: string;
}

export interface NewNoteResult {
  title: string;
  type: NewItemType;
  /** 目标位置显示名（收件箱 / 分组 / 目录路径） */
  targetLabel: string;
  /** 目标为 collection 时的 id */
  collectionId?: string;
}

interface NewNoteModalProps {
  onClose: () => void;
  onConfirm: (result: NewNoteResult) => void;
  targetGroup?: string;
  initialType?: NewItemType;
  /** 创建子文件夹时的父目录名（仅 folder 类型使用） */
  parentName?: string;
  /** 可选择的创建位置（收件箱 + 所有分组/目录） */
  locations?: NewItemLocation[];
  /** 预选位置 id（优先于 targetGroup；为 collection id 或分组名） */
  initialLocationId?: string;
}

const types: NewItemType[] = ["concept", "project", "code", "document", "inbox", "folder"];

export function NewNoteModal({
  onClose,
  onConfirm,
  targetGroup,
  initialType = "document",
  parentName,
  locations,
  initialLocationId,
}: NewNoteModalProps) {
  const [title, setTitle] = useState("");
  const [type, setType] = useState<NewItemType>(initialType);
  // 当前选中的位置 id（收件箱/分组名 或 collection id）
  const [locationId, setLocationId] = useState<string>(() => {
    if (initialLocationId) return initialLocationId;
    const matched = locations?.find((l) => l.label === targetGroup)?.id;
    return matched ?? targetGroup ?? "收件箱";
  });
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
    if (type === "folder") {
      onConfirm({ title: t, type, targetLabel: parentName ?? "收件箱" });
      return;
    }
    const loc = locations?.find((l) => l.id === locationId);
    onConfirm({
      title: t,
      type,
      targetLabel: loc?.label ?? targetGroup ?? "收件箱",
      collectionId: loc?.collectionId,
    });
  };

  const hasLocations = locations && locations.length > 0;

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
          {type === "folder" ? (
            parentName && (
              <div className="new-item-target">
                创建到：「{parentName}」下
              </div>
            )
          ) : hasLocations ? (
            <div className="new-item-target new-item-target-select">
              <span className="new-item-target-label">创建到：</span>
              <div className="new-item-select-wrap">
                <select
                  className="new-item-select"
                  value={locationId}
                  onChange={(e) => setLocationId(e.target.value)}
                >
                  {locations!.map((loc) => (
                    <option key={loc.id} value={loc.id}>
                      {loc.label}
                    </option>
                  ))}
                </select>
                <IconChevronDown size={12} className="new-item-select-arrow" />
              </div>
            </div>
          ) : (
            targetGroup && (
              <div className="new-item-target">创建到：{targetGroup}</div>
            )
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
