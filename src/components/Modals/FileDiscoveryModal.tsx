import { useMemo, useState } from "react";
import type { DiscoveryCandidate } from "../../api";
import { IconClose, IconFile, IconFolderPlus, IconRadar } from "../Icons";

interface FileDiscoveryModalProps {
  candidates: DiscoveryCandidate[];
  folders: string[];
  groups: string[];
  scanning: boolean;
  onAddFolders: () => void;
  onRemoveFolder: (folder: string) => void;
  onScan: () => void;
  onImport: (paths: string[], group: string) => Promise<void>;
  onIgnore: (paths: string[]) => Promise<void>;
  onClose: () => void;
}

const statusLabel = {
  new: "新文件",
  updated: "有更新",
  duplicate: "已存在",
};

function formatSize(size: number) {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / 1024 / 1024).toFixed(1)} MB`;
}

export function FileDiscoveryModal({
  candidates,
  folders,
  groups,
  scanning,
  onAddFolders,
  onRemoveFolder,
  onScan,
  onImport,
  onIgnore,
  onClose,
}: FileDiscoveryModalProps) {
  const [selected, setSelected] = useState<string[]>([]);
  const [group, setGroup] = useState("收件箱");
  const [processing, setProcessing] = useState(false);
  const selectedSet = useMemo(() => new Set(selected), [selected]);
  const allSelected = candidates.length > 0 && candidates.every((item) => selectedSet.has(item.path));

  const toggleAll = () => setSelected(allSelected ? [] : candidates.map((item) => item.path));
  const toggle = (path: string) => setSelected((prev) =>
    prev.includes(path) ? prev.filter((item) => item !== path) : [...prev, path]
  );

  const run = async (action: "import" | "ignore") => {
    if (selected.length === 0) return;
    setProcessing(true);
    try {
      if (action === "import") await onImport(selected, group);
      else await onIgnore(selected);
      setSelected([]);
    } finally {
      setProcessing(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={(event) => event.target === event.currentTarget && onClose()}>
      <div className="modal discovery-modal">
        <div className="discovery-modal-header">
          <div>
            <div className="discovery-modal-title"><IconRadar size={17} /> 文件发现</div>
            <div className="settings-desc">选择新文件或源文件更新，确认后再加入知识库</div>
          </div>
          <button className="icon-btn" title="关闭" onClick={onClose}><IconClose size={15} /></button>
        </div>

        {folders.length === 0 ? (
          <div className="discovery-onboarding">
            <div className="discovery-onboarding-icon"><IconFolderPlus size={26} /></div>
            <div className="discovery-onboarding-title">选择要查看的文件夹</div>
            <div className="discovery-onboarding-copy">KBOX 只扫描你明确选择的目录。文件不会自动入库，也不会被移动或删除。</div>
            <button className="btn btn-primary" onClick={onAddFolders}>选择文件夹</button>
          </div>
        ) : (
          <>
            <div className="discovery-scope">
              <div className="discovery-folder-list">
                {folders.map((folder) => (
                  <span className="discovery-folder" key={folder} title={folder}>
                    {folder.split(/[\\/]/).filter(Boolean).pop() || folder}
                    <button title="停止扫描此文件夹" onClick={() => onRemoveFolder(folder)}>×</button>
                  </span>
                ))}
              </div>
              <button className="btn btn-ghost discovery-small-btn" onClick={onAddFolders}>＋ 文件夹</button>
              <button className="btn btn-ghost discovery-small-btn" onClick={onScan} disabled={scanning}>{scanning ? "扫描中…" : "重新扫描"}</button>
            </div>

            <div className="discovery-selectbar">
              <label><input type="checkbox" checked={allSelected} onChange={toggleAll} /> 全选</label>
              <span>{candidates.length} 项待处理</span>
            </div>

            <div className="discovery-list">
              {candidates.length === 0 ? (
                <div className="discovery-empty"><IconRadar size={25} /><span>{scanning ? "正在扫描授权目录…" : "没有待处理的新文件"}</span></div>
              ) : candidates.map((item) => (
                <label className={`discovery-row${selectedSet.has(item.path) ? " selected" : ""}`} key={item.path}>
                  <input type="checkbox" checked={selectedSet.has(item.path)} onChange={() => toggle(item.path)} />
                  <span className={`discovery-file-icon ${item.kind}`}><IconFile size={14} /></span>
                  <span className="discovery-file-main"><span>{item.name}</span><small title={item.path}>{item.path}</small></span>
                  <span className="discovery-size">{formatSize(item.size)}</span>
                  <span className={`discovery-status ${item.status}`}>{statusLabel[item.status]}</span>
                </label>
              ))}
            </div>

            <div className="discovery-note">“有更新”第一版将作为新对象安全导入，不覆盖旧版本；“已存在”只登记来源，不重复复制。</div>
            <div className="modal-footer discovery-footer">
              <span className="discovery-selected">已选择 {selected.length} 项</span>
              <select value={group} onChange={(event) => setGroup(event.target.value)}>
                {Array.from(new Set(["收件箱", ...groups])).map((name) => <option key={name} value={name}>添加到：{name}</option>)}
              </select>
              <button className="btn btn-ghost" disabled={selected.length === 0 || processing} onClick={() => run("ignore")}>忽略</button>
              <button className="btn btn-primary" disabled={selected.length === 0 || processing} onClick={() => run("import")}>{processing ? "处理中…" : "添加"}</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
