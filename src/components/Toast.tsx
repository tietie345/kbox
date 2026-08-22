import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { IconAlert, IconCheck, IconInfo } from "./Icons";

export type ToastType = "info" | "warning" | "error" | "success";

export interface ToastItem {
  id: number;
  type: ToastType;
  message: string;
}

interface ToastContextValue {
  toast: (type: ToastType, message: string) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

let toastIdCounter = 0;

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const toast = useCallback((type: ToastType, message: string) => {
    const id = ++toastIdCounter;
    setToasts((prev) => [...prev, { id, type, message }]);
  }, []);

  const remove = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  return (
    <ToastContext.Provider value={{ toast }}>
      {children}
      <div className="toast-container">
        {toasts.map((t) => (
          <ToastView key={t.id} item={t} onClose={() => remove(t.id)} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

function ToastView({ item, onClose }: { item: ToastItem; onClose: () => void }) {
  useEffect(() => {
    const timer = setTimeout(onClose, 3000);
    return () => clearTimeout(timer);
  }, [onClose]);

  const icon = () => {
    switch (item.type) {
      case "success":
        return <IconCheck size={14} />;
      case "error":
      case "warning":
        return <IconAlert size={14} />;
      default:
        return <IconInfo size={14} />;
    }
  };

  return (
    <div className={`toast ${item.type}`} onClick={onClose}>
      <span className="toast-icon">{icon()}</span>
      <span>{item.message}</span>
    </div>
  );
}
