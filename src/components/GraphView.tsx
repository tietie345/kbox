import { useEffect, useMemo, useRef, useState, useCallback } from "react";
import type { GroupNode, FullRelationData, RelationData } from "../api";
import { getAllRelations, getRelationsRecursive } from "../api";

interface GraphNode {
  id: string;
  title: string;
  type: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
}

interface GraphEdge {
  source: string;
  target: string;
  relationType: string;
}

interface GraphViewProps {
  groups: GroupNode[];
  relationVersion?: number;
  onSelectObject: (id: string) => void;
}

const SIMULATION_STEPS = 300;
const REPULSION = 5000;
const ATTRACTION = 0.05;
const DAMPING = 0.85;

const TYPE_COLOR: Record<string, string> = {
  concept: "var(--blue)",
  code: "var(--green)",
  document: "var(--orange)",
  image: "var(--pink)",
  project: "var(--accent)",
  inbox: "var(--yellow)",
};

function colorByType(type: string): string {
  return TYPE_COLOR[type] ?? "var(--text-secondary)";
}

function edgeStyle(relationType: string): { dash: string; width: number } {
  switch (relationType) {
    case "related_to":
      return { dash: "4 4", width: 1 };
    case "implements":
      return { dash: "2 2", width: 1 };
    case "part_of":
      return { dash: "none", width: 2 };
    case "depends_on":
    default:
      return { dash: "none", width: 1 };
  }
}

function simulate(
  nodes: GraphNode[],
  edges: GraphEdge[],
  width: number,
  height: number
) {
  for (let step = 0; step < SIMULATION_STEPS; step++) {
    // 计算排斥力
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const dx = nodes[i].x - nodes[j].x;
        const dy = nodes[i].y - nodes[j].y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 1;
        const force = REPULSION / (dist * dist);
        nodes[i].vx += (dx / dist) * force;
        nodes[i].vy += (dy / dist) * force;
        nodes[j].vx -= (dx / dist) * force;
        nodes[j].vy -= (dy / dist) * force;
      }
    }
    // 计算吸引力（边）
    for (const edge of edges) {
      const source = nodes.find((n) => n.id === edge.source);
      const target = nodes.find((n) => n.id === edge.target);
      if (!source || !target) continue;
      const dx = target.x - source.x;
      const dy = target.y - source.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const force = ATTRACTION * dist;
      source.vx += (dx / dist) * force;
      source.vy += (dy / dist) * force;
      target.vx -= (dx / dist) * force;
      target.vy -= (dy / dist) * force;
    }
    // 更新位置
    for (const node of nodes) {
      node.vx *= DAMPING;
      node.vy *= DAMPING;
      node.x += node.vx;
      node.y += node.vy;
      node.x = Math.max(50, Math.min(width - 50, node.x));
      node.y = Math.max(50, Math.min(height - 50, node.y));
    }
  }
}

export function GraphView({ groups, relationVersion, onSelectObject }: GraphViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 800, height: 600 });
  const [relations, setRelations] = useState<FullRelationData[]>([]);
  const [loading, setLoading] = useState(true);
  const [scale, setScale] = useState(1);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // 过滤器状态
  const OBJECT_TYPES = ["concept", "code", "document", "image", "project", "inbox"];
  const RELATION_TYPES = ["related_to", "same_group", "same_tag", "title_contains", "depends_on", "part_of", "implements"];
  const [selectedTypes, setSelectedTypes] = useState<Set<string>>(new Set(OBJECT_TYPES));
  const [selectedRelationTypes, setSelectedRelationTypes] = useState<Set<string>>(new Set(RELATION_TYPES));
  const [showFilter, setShowFilter] = useState(true);
  useEffect(() => {
    const toggle = () => setShowFilter((visible) => !visible);
    window.addEventListener("kbox:toggle-graph-filter", toggle);
    return () => window.removeEventListener("kbox:toggle-graph-filter", toggle);
  }, []);
  // 右键菜单状态
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; nodeId: string; nodeTitle: string } | null>(null);
  // 关联关系弹窗状态
  const [relationModal, setRelationModal] = useState<{ nodeId: string; nodeTitle: string; relations: RelationData[] } | null>(null);
  const [relationLoading, setRelationLoading] = useState(false);
  // 节点位置（与 simulate 配合）
  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const dragOffsetRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });

  // 测量容器尺寸
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const update = () => {
      const rect = el.getBoundingClientRect();
      setSize({ width: rect.width, height: rect.height });
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // 加载关系数据
  const loadRelations = useCallback(async () => {
    setLoading(true);
    try {
      const r = await getAllRelations();
      setRelations(r);
    } catch (e) {
      // 静默失败，保持空关系
      setRelations([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadRelations();
  }, [groups, relationVersion, loadRelations]);

  // 所有对象扁平化
  const allObjects = useMemo(() => {
    const arr: { id: string; title: string; type: string }[] = [];
    for (const g of groups) {
      for (const o of g.objects) {
        arr.push({
          id: o.id,
          title: o.title,
          type: o.type || "document",
        });
      }
    }
    return arr;
  }, [groups]);

  const edges = useMemo<GraphEdge[]>(() => {
    return relations.map((r) => ({
      source: r.source_id,
      target: r.target_id,
      relationType: r.relation_type,
    }));
  }, [relations]);

  // 过滤器：按类型筛选节点
  const filteredNodes = useMemo(() => {
    return nodes.filter((n) => selectedTypes.has(n.type));
  }, [nodes, selectedTypes]);

  // 过滤器：按关系类型筛选边，且两端节点都可见
  const filteredEdges = useMemo(() => {
    const visibleIds = new Set(filteredNodes.map((n) => n.id));
    return edges.filter(
      (e) =>
        selectedRelationTypes.has(e.relationType) &&
        visibleIds.has(e.source) &&
        visibleIds.has(e.target)
    );
  }, [edges, selectedRelationTypes, filteredNodes]);

  // 当对象集合或尺寸变化时，重新初始化节点并计算布局
  useEffect(() => {
    if (allObjects.length === 0) {
      setNodes([]);
      return;
    }
    if (size.width === 0 || size.height === 0) return;
    const cx = size.width / 2;
    const cy = size.height / 2;
    const r = Math.min(size.width, size.height) * 0.3;
    const initNodes: GraphNode[] = allObjects.map((o, i) => {
      const angle = (i / allObjects.length) * Math.PI * 2;
      return {
        id: o.id,
        title: o.title,
        type: o.type,
        x: cx + Math.cos(angle) * r,
        y: cy + Math.sin(angle) * r,
        vx: 0,
        vy: 0,
      };
    });
    simulate(initNodes, edges, size.width, size.height);
    setNodes(initNodes);
  }, [allObjects, edges, size]);

  // 鼠标拖拽节点
  const handleMouseDown = (e: React.MouseEvent, node: GraphNode) => {
    e.preventDefault();
    e.stopPropagation();
    setDraggingId(node.id);
    const rect = containerRef.current?.getBoundingClientRect();
    if (rect) {
      // 计算节点在屏幕坐标上的位置（考虑缩放）
      const screenX = rect.left + node.x * scale;
      const screenY = rect.top + node.y * scale;
      dragOffsetRef.current = { x: e.clientX - screenX, y: e.clientY - screenY };
    }
  };

  useEffect(() => {
    if (!draggingId) return;
    const onMove = (e: MouseEvent) => {
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;
      const x = (e.clientX - rect.left - dragOffsetRef.current.x) / scale;
      const y = (e.clientY - rect.top - dragOffsetRef.current.y) / scale;
      setNodes((prev) =>
        prev.map((n) =>
          n.id === draggingId
            ? {
                ...n,
                x: Math.max(20, Math.min(size.width - 20, x)),
                y: Math.max(20, Math.min(size.height - 20, y)),
                vx: 0,
                vy: 0,
              }
            : n
        )
      );
    };
    const onUp = () => setDraggingId(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    document.body.style.cursor = "grabbing";
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
    };
  }, [draggingId, scale, size.width, size.height]);

  // 滚轮缩放
  const handleWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY > 0 ? -0.1 : 0.1;
    setScale((s) => Math.max(0.3, Math.min(3, s + delta)));
  };

  // 点击节点
  const handleNodeClick = (id: string) => {
    setSelectedId(id);
    onSelectObject(id);
  };

  // 空状态
  if (!loading && allObjects.length === 0) {
    return (
      <div className="editor-empty">
        <div className="editor-empty-icon">
          <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="5" r="2" />
            <circle cx="5" cy="19" r="2" />
            <circle cx="19" cy="19" r="2" />
            <line x1="12" y1="7" x2="6" y2="17" />
            <line x1="12" y1="7" x2="18" y2="17" />
            <line x1="7" y1="19" x2="17" y2="19" />
          </svg>
        </div>
        <div className="editor-empty-title">暂无对象或关系</div>
        <div className="editor-empty-hint">
          新建对象或在右键菜单中添加关系后，图谱将自动展示
        </div>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="editor-empty">
        <div className="editor-empty-icon">
          <div className="graph-loading-spinner" />
        </div>
        <div className="editor-empty-title">加载图谱中...</div>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className="graph-view"
      onWheel={handleWheel}
      style={{
        flex: 1,
        position: "relative",
        overflow: "hidden",
        background: "var(--bg-primary)",
        cursor: draggingId ? "grabbing" : "default",
      }}
    >
      <svg
        width="100%"
        height="100%"
        viewBox={`0 0 ${size.width} ${size.height}`}
        style={{ transform: `scale(${scale})`, transformOrigin: "center center" }}
      >
        {/* 边 */}
        {filteredEdges.map((edge, idx) => {
          const source = filteredNodes.find((n) => n.id === edge.source);
          const target = filteredNodes.find((n) => n.id === edge.target);
          if (!source || !target) return null;
          const style = edgeStyle(edge.relationType);
          return (
            <line
              key={`e-${idx}`}
              x1={source.x}
              y1={source.y}
              x2={target.x}
              y2={target.y}
              stroke="var(--border)"
              strokeDasharray={style.dash}
              strokeWidth={style.width}
            />
          );
        })}
        {/* 节点 */}
        {filteredNodes.map((node) => (
          <g
            key={node.id}
            transform={`translate(${node.x},${node.y})`}
            onMouseDown={(e) => handleMouseDown(e, node)}
            onClick={(e) => {
              e.stopPropagation();
              handleNodeClick(node.id);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              setContextMenu({ x: e.clientX, y: e.clientY, nodeId: node.id, nodeTitle: node.title });
            }}
            onMouseEnter={() => setHoveredId(node.id)}
            onMouseLeave={() => setHoveredId(null)}
            style={{ cursor: "grab" }}
          >
            <circle
              r={selectedId === node.id ? 11 : 8}
              fill={colorByType(node.type)}
              stroke={
                selectedId === node.id
                  ? "var(--text-bright)"
                  : hoveredId === node.id
                  ? "var(--text-primary)"
                  : "transparent"
              }
              strokeWidth={2}
            />
            {(hoveredId === node.id || selectedId === node.id) && (
              <text
                x={12}
                y={4}
                fontSize={11}
                fill="var(--text-primary)"
                style={{ pointerEvents: "none", userSelect: "none" }}
              >
                {node.title}
              </text>
            )}
          </g>
        ))}
      </svg>

      {/* 过滤器面板 */}
      {showFilter && <div className="graph-filter">
        <div className="graph-filter-section">
          <div className="graph-filter-title">对象类型</div>
          {OBJECT_TYPES.map((t) => (
            <label key={t} className="graph-filter-label">
              <input
                type="checkbox"
                checked={selectedTypes.has(t)}
                onChange={() => {
                  setSelectedTypes((prev) => {
                    const next = new Set(prev);
                    if (next.has(t)) next.delete(t);
                    else next.add(t);
                    return next;
                  });
                }}
              />
              <span className="legend-dot" style={{ background: colorByType(t) }} />
              <span>{t}</span>
            </label>
          ))}
        </div>
        <div className="graph-filter-section">
          <div className="graph-filter-title">关系类型</div>
          {RELATION_TYPES.map((t) => (
            <label key={t} className="graph-filter-label">
              <input
                type="checkbox"
                checked={selectedRelationTypes.has(t)}
                onChange={() => {
                  setSelectedRelationTypes((prev) => {
                    const next = new Set(prev);
                    if (next.has(t)) next.delete(t);
                    else next.add(t);
                    return next;
                  });
                }}
              />
              <span>{t}</span>
            </label>
          ))}
        </div>
      </div>}

      {/* 右键菜单 */}
      {contextMenu && (
        <>
          <div
            className="context-menu-backdrop"
            onClick={() => setContextMenu(null)}
          />
          <div
            className="context-menu"
            style={{ left: contextMenu.x, top: contextMenu.y }}
          >
            <div
              className="context-menu-item"
              onClick={async () => {
                const id = contextMenu.nodeId;
                const title = contextMenu.nodeTitle;
                setContextMenu(null);
                setRelationLoading(true);
                try {
                  const data = await getRelationsRecursive(id, 3);
                  // 去重（按 target_id 去重）
                  const seen = new Set<string>();
                  const deduped: RelationData[] = [];
                  for (const r of data) {
                    if (!seen.has(r.target_id)) {
                      seen.add(r.target_id);
                      deduped.push(r);
                    }
                  }
                  setRelationModal({ nodeId: id, nodeTitle: title, relations: deduped });
                } catch {
                  setRelationModal({ nodeId: id, nodeTitle: title, relations: [] });
                } finally {
                  setRelationLoading(false);
                }
              }}
            >
              查看关联关系（深度3）
            </div>
            <div
              className="context-menu-item"
              onClick={() => {
                const id = contextMenu.nodeId;
                setContextMenu(null);
                onSelectObject(id);
              }}
            >
              查看详情
            </div>
          </div>
        </>
      )}

      {/* 关联关系弹窗 */}
      {relationModal && (
        <div
          className="modal-overlay"
          onClick={(e) => {
            if (e.target === e.currentTarget) setRelationModal(null);
          }}
        >
          <div className="modal" style={{ minWidth: 420, maxWidth: 560 }}>
            <div className="modal-header">
              关联关系 - {relationModal.nodeTitle}
            </div>
            <div className="modal-body">
              {relationLoading && (
                <div style={{ padding: "20px 0", textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
                  加载中...
                </div>
              )}
              {!relationLoading && relationModal.relations.length === 0 && (
                <div style={{ padding: "20px 0", textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
                  未找到关联关系
                </div>
              )}
              {!relationLoading && relationModal.relations.length > 0 && (
                <div className="relation-list">
                  {relationModal.relations.map((r, idx) => (
                    <div
                      key={`${r.target_id}-${idx}`}
                      className="relation-list-item"
                      style={{ cursor: "pointer" }}
                      onClick={() => {
                        setRelationModal(null);
                        onSelectObject(r.target_id);
                      }}
                    >
                      <span className="relation-list-type">[{r.relation_type}]</span>
                      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {r.target_title}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="modal-footer">
              <button className="btn btn-ghost" onClick={() => setRelationModal(null)}>
                关闭
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 图例 */}
      <div className="graph-legend">
        <div className="graph-legend-section">
          <div className="graph-legend-title">对象类型</div>
          <div className="graph-legend-row">
            <span className="legend-dot" style={{ background: "var(--blue)" }} />
            <span>概念</span>
            <span className="legend-dot" style={{ background: "var(--green)" }} />
            <span>代码</span>
            <span className="legend-dot" style={{ background: "var(--orange)" }} />
            <span>文档</span>
            <span className="legend-dot" style={{ background: "var(--pink)" }} />
            <span>图片</span>
            <span className="legend-dot" style={{ background: "var(--accent)" }} />
            <span>项目</span>
          </div>
        </div>
        <div className="graph-legend-section">
          <div className="graph-legend-title">关系类型</div>
          <div className="graph-legend-row">
            <span className="legend-line solid" /> <span>depends_on</span>
            <span className="legend-line dashed" /> <span>related_to</span>
            <span className="legend-line bold" /> <span>part_of</span>
            <span className="legend-line dotted" /> <span>implements</span>
          </div>
        </div>
        <div className="graph-legend-info">
          节点: {filteredNodes.length} · 边: {filteredEdges.length} · 缩放: {Math.round(scale * 100)}%
        </div>
      </div>
    </div>
  );
}
