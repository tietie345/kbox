import { useEffect, useLayoutEffect, useRef, useState } from "react";

export interface ContextMenuItem {
  id: string;
  label: string;
  icon?: React.ReactNode;
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
  separator?: false;
}

export interface ContextMenuSeparator {
  id: string;
  separator: true;
}

export type ContextMenuEntry = ContextMenuItem | ContextMenuSeparator;

export interface ContextMenuState {
  x: number;
  y: number;
  items: ContextMenuEntry[];
  // 可选的上下文对象 id（用于对象右键菜单）
  __objectId?: string;
  // 可选的上下文分组名
  __groupName?: string;
  // 可选的 Collection 上下文
  __collectionId?: string;
  __collectionPath?: string;
}

interface ContextMenuProps {
  state: ContextMenuState | null;
  onClose: () => void;
  onSelect: (item: ContextMenuItem) => void;
}

export function ContextMenu({ state, onClose, onSelect }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number }>({ x: 0, y: 0 });

  useLayoutEffect(() => {
    if (!state) return;
    const el = ref.current;
    const menuW = el?.offsetWidth ?? 180;
    const menuH = el?.offsetHeight ?? 200;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let x = state.x;
    let y = state.y;
    if (x + menuW > vw - 4) x = Math.max(4, vw - menuW - 4);
    if (y + menuH > vh - 4) y = Math.max(4, vh - menuH - 4);
    setPos({ x, y });
  }, [state]);

  useEffect(() => {
    if (!state) return;
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", handleClick);
    window.addEventListener("keydown", handleKey);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", handleClick);
      window.removeEventListener("keydown", handleKey);
      window.removeEventListener("resize", onClose);
    };
  }, [state, onClose]);

  if (!state) return null;

  return (
    <div
      ref={ref}
      className="context-menu"
      style={{ left: pos.x, top: pos.y }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {state.items.map((entry) => {
        if (entry.separator) {
          return <div key={entry.id} className="context-menu-sep" />;
        }
        return (
          <div
            key={entry.id}
            className={`context-menu-item${entry.danger ? " danger" : ""}${
              entry.disabled ? " disabled" : ""
            }`}
            onClick={() => {
              if (entry.disabled) return;
              onSelect(entry);
              onClose();
            }}
          >
            {entry.icon && <span className="cmd-item-icon">{entry.icon}</span>}
            <span>{entry.label}</span>
            {entry.shortcut && (
              <span className="context-menu-shortcut">{entry.shortcut}</span>
            )}
          </div>
        );
      })}
    </div>
  );
}
