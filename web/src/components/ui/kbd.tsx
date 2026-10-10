import * as React from "react";

import { formatKeys } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";

function Kbd({ className, ...props }: React.ComponentProps<"kbd">) {
  return (
    <kbd
      data-slot="kbd"
      className={cn(
        "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded border border-border-strong bg-surface px-1 font-sans text-[10px] leading-none font-medium text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

/** Zeigt eine Tastenfolge (`mod+k`, `g i`) als Kbd-Gruppe. */
function Keys({ keys, className }: { keys: string; className?: string }) {
  const seq = formatKeys(keys);
  return (
    <span
      className={cn("inline-flex items-center gap-1", className)}
      aria-label={seq.map((c) => c.join(" ")).join(" then ")}
    >
      {seq.map((combo, i) => (
        <span key={i} className="inline-flex items-center gap-0.5" aria-hidden>
          {combo.map((k, j) => (
            <Kbd key={j}>{k}</Kbd>
          ))}
        </span>
      ))}
    </span>
  );
}

export { Kbd, Keys };
