import { Link } from "@tanstack/react-router";
import { X } from "lucide-react";
import { useEffect } from "react";

import { Button } from "@/components/ui/button";

import { dismissToast, useToasts, type Toast } from "./toast-store";

function ToastItem({ toast }: { toast: Toast }) {
  useEffect(() => {
    const timer = setTimeout(() => dismissToast(toast.id), toast.tone === "error" ? 8000 : 5000);
    return () => clearTimeout(timer);
  }, [toast.id, toast.tone]);
  return (
    <div
      role={toast.tone === "error" ? "alert" : "status"}
      className="pointer-events-auto flex items-center gap-3 rounded-lg border border-border-strong bg-popover px-3 py-2 text-sm shadow-popover"
    >
      <span className={toast.tone === "error" ? "text-destructive" : undefined}>
        {toast.message}
      </span>
      {toast.href && (
        <Link to={toast.href as "/"} className="text-primary hover:underline">
          Open
        </Link>
      )}
      {toast.onUndo && (
        <button
          type="button"
          className="text-primary hover:underline"
          onClick={() => {
            toast.onUndo?.();
            dismissToast(toast.id);
          }}
        >
          Undo
        </button>
      )}
      <Button
        variant="ghost"
        size="icon"
        className="size-5"
        aria-label="Dismiss"
        onClick={() => dismissToast(toast.id)}
      >
        <X className="size-3" />
      </Button>
    </div>
  );
}

export function Toaster() {
  const list = useToasts();
  return (
    <div className="pointer-events-none fixed right-4 bottom-4 z-[60] flex flex-col items-end gap-2">
      {list.map((t) => (
        <ToastItem key={t.id} toast={t} />
      ))}
    </div>
  );
}
