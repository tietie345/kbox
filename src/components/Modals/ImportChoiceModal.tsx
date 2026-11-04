import { IconClose, IconFile, IconFolder } from "../Icons";

interface ImportChoiceModalProps {
  onClose: () => void;
  onSelectFiles: () => void;
  onSelectFolder: () => void;
}

export function ImportChoiceModal({
  onClose,
  onSelectFiles,
  onSelectFolder,
}: ImportChoiceModalProps) {
  return (
    <div
      className="modal-overlay"
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="modal import-choice-modal" role="dialog" aria-modal="true">
        <div className="modal-header import-choice-header">
          <span>导入到知识库</span>
          <button className="icon-btn" onClick={onClose} aria-label="关闭">
            <IconClose size={15} />
          </button>
        </div>
        <div className="modal-body import-choice-body">
          <button className="import-choice-card" onClick={onSelectFiles}>
            <span className="import-choice-icon"><IconFile size={22} /></span>
            <span>
              <strong>选择文件</strong>
              <small>支持 Markdown、PDF、代码、图片和 Office 文件</small>
            </span>
          </button>
          <button className="import-choice-card" onClick={onSelectFolder}>
            <span className="import-choice-icon"><IconFolder size={22} /></span>
            <span>
              <strong>选择文件夹</strong>
              <small>递归导入文件夹中的可识别文件</small>
            </span>
          </button>
        </div>
      </div>
    </div>
  );
}
