interface StatusBarProps {
  objectCount: number;
  agentOnline: boolean;
  activeObjectTitle: string | null;
}

export function StatusBar({
  objectCount,
  agentOnline,
  activeObjectTitle,
}: StatusBarProps) {
  return (
    <div className="status-bar">
      <div className="status-item">
        <span>对象总数: {objectCount}</span>
      </div>
      {activeObjectTitle && (
        <div className="status-item">
          <span>当前: {activeObjectTitle}</span>
        </div>
      )}
      <div className="status-spacer" />
      <div className="status-item">
        <span className={`status-dot ${agentOnline ? "online" : "offline"}`} />
        <span>Agent {agentOnline ? "已配置" : "未配置"}</span>
      </div>
      <div className="status-item">
        <span>UTF-8</span>
      </div>
      <div className="status-item">
        <span>Markdown</span>
      </div>
    </div>
  );
}
