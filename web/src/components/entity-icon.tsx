/** Symbol eines Dokuments/Teams/Projekts als Emoji (`:bar_chart:` → 📊); sonst `fallback`. */
import type { ReactNode } from "react";

import { iconEmoji, useEmojiData } from "@/lib/emoji";
import { cn } from "@/lib/cn";

export function EntityIcon({
  icon,
  className,
  fallback = null,
}: {
  icon: string | null | undefined;
  className?: string;
  fallback?: ReactNode;
}) {
  const data = useEmojiData();
  const emoji = iconEmoji(icon, data);
  if (!emoji) return <>{fallback}</>;
  return (
    <span aria-hidden className={cn("inline-flex shrink-0 justify-center leading-none", className)}>
      {emoji}
    </span>
  );
}
