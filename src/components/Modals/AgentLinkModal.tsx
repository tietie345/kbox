import { useCallback, useEffect, useState } from "react";
import { detectAgents, linkAgent, type AgentStatus } from "../../api";

interface AgentLinkModalProps {
  onClose: () => void;
  /** 列表发生变化时通知父组件（用于工具栏角标 / 状态栏联动） */
  onChange?: (agents: AgentStatus[]) => void;
}

export function AgentLinkModal({ onClose, onChange }: AgentLinkModalProps) {
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [linkingId, setLinkingId] = useState<string | null>(null);
  const [batchRunning, setBatchRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await detectAgents();
      setAgents(list);
      onChange?.(list);
    } catch (reason) {
      setError(`检测失败：${reason}`);
    } finally {
      setLoading(false);
    }
  }, [onChange]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const linkOne = useCallback(
    async (id: string) => {
      setLinkingId(id);
      setError(null);
      try {
        await linkAgent(id);
        await refresh();
      } catch (reason) {
        setError(`接入失败：${reason}`);
      } finally {
        setLinkingId(null);
      }
    },
    [refresh]
  );

  const linkAll = useCallback(async () => {
    // 批量只处理「配置已存在但未接入」的客户端，不给未安装的客户端凭空建配置
    const targets = agents.filter((a) => a.configExists && !a.registered);
    if (targets.length === 0) return;
    setBatchRunning(true);
    setError(null);
    try {
      for (const agent of targets) {
        setLinkingId(agent.id);
        try {
          await linkAgent(agent.id);
        } catch (reason) {
          setError(`「${agent.name}」接入失败：${reason}`);
        }
      }
      await refresh();
    } finally {
      setLinkingId(null);
      setBatchRunning(false);
    }
  }, [agents, refresh]);

  const linkableCount = agents.filter((a) => a.configExists && !a.registered).length;

  return (
    <div
      className="modal-overlay"
      onClick={(event) => event.target === event.currentTarget && onClose()}
    >
      <div className="modal" style={{ minWidth: 580, maxWidth: 640 }}>
        <div className="modal-header">接入 AI Agent</div>
        <div className="modal-body">
          <div className="agent-link-intro">
            KBOX 通过本地 MCP Server（kb-mcp）让外部 Agent 直接读写知识库。
            选择已安装的客户端一键接入，接入后重启对应客户端生效。
          </div>

          <div className="agent-link-list">
            {loading && agents.length === 0 && (
              <div className="agent-link-empty">正在扫描本机 Agent…</div>
            )}
            {!loading && agents.length === 0 && !error && (
              <div className="agent-link-empty">未发现可接入的 Agent</div>
            )}
            {agents.map((agent) => {
              const linking = linkingId === agent.id;
              return (
                <div className="agent-link-item" key={agent.id}>
                  <div className="agent-link-info">
                    <div className="agent-link-name">
                      {agent.name}
                      <span
                        className={`agent-link-badge ${
                          agent.registered
                            ? "registered"
                            : agent.configExists
                              ? "pending"
                              : "missing"
                        }`}
                      >
                        {agent.registered
                          ? "已接入"
                          : agent.configExists
                            ? "待接入"
                            : "未安装"}
                      </span>
                    </div>
                    <div className="agent-link-path" title={agent.configPath}>
                      {agent.configPath}
                    </div>
                  </div>
                  <div className="agent-link-action">
                    {agent.registered ? (
                      <button
                        className="btn btn-ghost"
                        onClick={() => linkOne(agent.id)}
                        disabled={linking || batchRunning}
                      >
                        {linking ? "接入中…" : "重新接入"}
                      </button>
                    ) : (
                      <button
                        className="btn btn-primary"
                        onClick={() => linkOne(agent.id)}
                        disabled={linking || batchRunning}
                        title={agent.configExists ? undefined : "尚未检测到该客户端，将为你创建配置文件"}
                      >
                        {linking ? "接入中…" : "接入"}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>

          {error && <div className="modal-error">{error}</div>}

          <div className="agent-link-tip">
            写入前会自动备份原配置（同目录 <code>.bak.时间戳</code>），且只合并新增
            KBOX 条目，不会影响你已有的其他 MCP 服务。
          </div>
        </div>
        <div className="modal-footer">
          <button
            className="btn btn-ghost"
            type="button"
            onClick={refresh}
            disabled={loading || batchRunning}
          >
            重新检测
          </button>
          <div style={{ flex: 1 }} />
          <button className="btn btn-ghost" type="button" onClick={onClose}>
            关闭
          </button>
          <button
            className="btn btn-primary"
            type="button"
            onClick={linkAll}
            disabled={linkableCount === 0 || batchRunning || loading}
            title={linkableCount === 0 ? "没有待接入的客户端" : undefined}
          >
            {batchRunning
              ? "正在接入…"
              : linkableCount > 0
                ? `一键接入全部（${linkableCount}）`
                : "已全部接入"}
          </button>
        </div>
      </div>
      <style>{`
        .agent-link-intro {
          font-size: 12px;
          color: var(--text-secondary, var(--text-muted));
          line-height: 1.6;
          margin-bottom: 12px;
        }
        .agent-link-list {
          display: flex;
          flex-direction: column;
          gap: 6px;
          max-height: 340px;
          overflow-y: auto;
        }
        .agent-link-empty {
          padding: 24px 0;
          text-align: center;
          color: var(--text-muted);
          font-size: 12px;
        }
        .agent-link-item {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 12px;
          padding: 10px 12px;
          border: 1px solid var(--border-color, var(--border, #eee));
          border-radius: 8px;
          background: var(--bg-secondary, transparent);
        }
        .agent-link-item:hover {
          background: var(--bg-hover, rgba(0,0,0,0.03));
        }
        .agent-link-info { min-width: 0; flex: 1; }
        .agent-link-name {
          display: flex;
          align-items: center;
          gap: 8px;
          font-size: 13px;
          font-weight: 600;
          color: var(--text-primary);
        }
        .agent-link-badge {
          font-size: 10px;
          font-weight: 500;
          padding: 1px 7px;
          border-radius: 10px;
          line-height: 1.6;
        }
        .agent-link-badge.registered {
          color: var(--success, #2ea043);
          background: rgba(46, 160, 67, 0.12);
        }
        .agent-link-badge.pending {
          color: #d29922;
          background: rgba(210, 153, 34, 0.14);
        }
        .agent-link-badge.missing {
          color: var(--text-muted);
          background: var(--bg-active, rgba(0,0,0,0.05));
        }
        .agent-link-path {
          margin-top: 3px;
          font-size: 11px;
          color: var(--text-muted);
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
        }
        .agent-link-action { flex-shrink: 0; }
        .agent-link-action .btn { height: 28px; font-size: 12px; }
        .agent-link-tip {
          margin-top: 12px;
          font-size: 11px;
          line-height: 1.6;
          color: var(--text-muted);
        }
        .agent-link-tip code {
          padding: 0 3px;
          border-radius: 3px;
          background: var(--bg-active, rgba(0,0,0,0.05));
        }
      `}</style>
    </div>
  );
}
