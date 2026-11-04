import { IconEditor } from "./Icons";

export function EditorEmpty() {
  return (
    <div className="editor-empty">
      <div className="editor-empty-icon">
        <IconEditor size={26} />
      </div>
      <div className="editor-empty-title">尚未打开任何对象</div>
      <div className="editor-empty-hint">
        从左侧文件树选择一个对象，或使用快捷键创建新笔记
      </div>
      <div className="empty-shortcut">
        <kbd>⌘N</kbd>
        <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
          新建笔记
        </span>
        <kbd>⌘K</kbd>
        <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
          命令面板
        </span>
      </div>
    </div>
  );
}
