export function DropOverlay() {
  return (
    <div className="drop-overlay">
      <div className="drop-overlay-inner">
        <div className="drop-overlay-title">释放以导入文件</div>
        <div className="drop-overlay-hint">
          支持文档 / 代码 / 图片，将自动识别类型并入库
        </div>
      </div>
    </div>
  );
}
