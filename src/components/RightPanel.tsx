import type { ObjectMetadata, Relation } from "../mock/data";
import {
  IconChevronRight,
  IconClock,
  IconClose,
  IconFile,
  IconHash,
  IconLink,
  IconRobot,
  IconTag,
  IconUser,
  objTypeMeta,
} from "./Icons";

interface RightPanelProps {
  collapsed: boolean;
  metadata: ObjectMetadata | null;
  relations: Relation[];
  onToggleCollapse: () => void;
  onRemoveRelation?: (targetId: string, relationType: string) => void;
}

function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(
      d.getDate()
    )} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  } catch {
    return iso;
  }
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

const statusLabel: Record<string, string> = {
  curated: "已整理",
  draft: "草稿",
  processing: "处理中",
};

const statusColor: Record<string, string> = {
  curated: "var(--green)",
  draft: "var(--text-muted)",
  processing: "var(--orange)",
};

const relationLabel: Record<string, string> = {
  related_to: "相关",
  depends_on: "依赖",
  extends: "扩展",
  part_of: "属于",
};

export function RightPanel({
  collapsed,
  metadata,
  relations,
  onToggleCollapse,
  onRemoveRelation,
}: RightPanelProps) {
  if (collapsed) {
    return (
      <div
        style={{
          width: 32,
          background: "var(--bg-secondary)",
          borderLeft: "1px solid var(--border)",
          display: "flex",
          alignItems: "flex-start",
          justifyContent: "center",
          paddingTop: 8,
          flexShrink: 0,
        }}
      >
        <button
          className="icon-btn"
          title="展开右侧面板"
          onClick={onToggleCollapse}
        >
          <IconChevronRight size={16} style={{ transform: "rotate(180deg)" }} />
        </button>
      </div>
    );
  }

  if (!metadata) {
    return (
      <div className="right-panel">
        <div className="right-panel-toolbar">
          <span>元数据</span>
          <button className="icon-btn" title="收起右侧面板" onClick={onToggleCollapse}>
            <IconChevronRight size={15} />
          </button>
        </div>
        <div className="right-section">
          <div
            className="right-section-body"
            style={{ color: "var(--text-muted)", fontSize: 12 }}
          >
            选择一个对象以查看元数据
          </div>
        </div>
      </div>
    );
  }

  const typeMeta = objTypeMeta[metadata.type];
  const TypeIcon = typeMeta?.icon ?? IconFile;

  return (
    <div className="right-panel">
      <div className="right-panel-toolbar">
        <span>元数据</span>
        <button className="icon-btn" title="收起右侧面板" onClick={onToggleCollapse}>
          <IconChevronRight size={15} />
        </button>
      </div>
      {/* 元数据 */}
      <div className="right-section">
        <div className="right-section-body">
          <div className="meta-row">
            <span className="meta-label">类型</span>
            <span
              className="meta-type-badge"
              style={{
                background: `var(--accent-soft)`,
                color: typeMeta?.color ?? "var(--accent)",
              }}
            >
              <TypeIcon size={11} />
              {typeMeta?.label ?? metadata.type}
            </span>
          </div>
          <div className="meta-row">
            <span className="meta-label">格式</span>
            <span className="meta-value mono">{metadata.format}</span>
          </div>
          <div className="meta-row">
            <span className="meta-label">分组</span>
            <span className="meta-value">{metadata.group}</span>
          </div>
          <div className="meta-row">
            <span className="meta-label">状态</span>
            <span
              className="meta-value"
              style={{ color: statusColor[metadata.status] }}
            >
              {statusLabel[metadata.status] ?? metadata.status}
            </span>
          </div>
          <div className="meta-row">
            <span className="meta-label">创建者</span>
            <span
              className="meta-value"
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 4,
              }}
            >
              {metadata.createdBy === "agent" ? (
                <IconRobot size={11} />
              ) : (
                <IconUser size={11} />
              )}
              {metadata.createdBy === "agent" ? "Agent" : "手动"}
            </span>
          </div>
          <div className="meta-row">
            <span className="meta-label">来源</span>
            <span className="meta-value">{metadata.source ?? "—"}</span>
          </div>
        </div>
      </div>

      {/* 标签 */}
      <div className="right-section">
        <div className="right-section-header">
          <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
            <IconTag size={10} /> 标签
          </span>
        </div>
        <div className="right-section-body">
          {metadata.tags.length === 0 ? (
            <span style={{ color: "var(--text-muted)", fontSize: 12 }}>
              无标签
            </span>
          ) : (
            <div>
              {metadata.tags.map((t) => (
                <span key={t} className="meta-tag">
                  #{t}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* 关系 */}
      <div className="right-section" style={{ flex: 1, overflowY: "auto" }}>
        <div className="right-section-header">
          <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
            <IconLink size={10} /> 关系 ({relations.length})
          </span>
        </div>
        <div className="right-section-body" style={{ paddingBottom: 12 }}>
          {relations.length === 0 ? (
            <span style={{ color: "var(--text-muted)", fontSize: 12 }}>
              暂无关系
            </span>
          ) : (
            relations.map((r, idx) => (
              <div key={`${r.targetId}-${idx}`} className="relation-item">
                <IconLink size={12} style={{ color: "var(--text-muted)" }} />
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {r.targetTitle}
                </span>
                <span className="relation-type">
                  {relationLabel[r.relationType] ?? r.relationType}
                </span>
                {onRemoveRelation && (
                  <button
                    className="tab-close"
                    title="删除关系"
                    onClick={() => onRemoveRelation(r.targetId, r.relationType)}
                    style={{ width: 16, height: 16, flexShrink: 0 }}
                  >
                    <IconClose size={11} />
                  </button>
                )}
              </div>
            ))
          )}
        </div>
      </div>

      {/* 原文件信息 */}
      <div className="right-section">
        <div className="right-section-header">
          <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
            <IconFile size={10} /> 原文件
          </span>
        </div>
        <div className="right-section-body">
          <div className="meta-row">
            <span className="meta-label">
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 4,
                }}
              >
                <IconClock size={10} /> 创建
              </span>
            </span>
            <span className="meta-value mono" style={{ fontSize: 11 }}>
              {formatTime(metadata.createdAt)}
            </span>
          </div>
          <div className="meta-row">
            <span className="meta-label">
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 4,
                }}
              >
                <IconClock size={10} /> 更新
              </span>
            </span>
            <span className="meta-value mono" style={{ fontSize: 11 }}>
              {formatTime(metadata.updatedAt)}
            </span>
          </div>
          <div className="meta-row">
            <span className="meta-label">
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 4,
                }}
              >
                <IconHash size={10} /> 大小
              </span>
            </span>
            <span className="meta-value mono">{formatSize(metadata.fileSize)}</span>
          </div>
          <div className="meta-row">
            <span className="meta-label">SHA256</span>
            <span
              className="meta-value mono"
              style={{ fontSize: 10, maxWidth: 180 }}
            >
              {metadata.fileHash}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
