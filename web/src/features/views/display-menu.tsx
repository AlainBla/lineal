import { SlidersHorizontal } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";

import { COMPLETED_WINDOWS, GROUPINGS, ORDERINGS, type DisplayOptions } from "./display-options";

const LABEL: Record<string, string> = {
  none: "No grouping",
  status: "Status",
  assignee: "Assignee",
  project: "Project",
  priority: "Priority",
  team: "Team",
  cycle: "Cycle",
  title: "Title",
  updatedAt: "Last updated",
  createdAt: "Created",
  dueDate: "Due date",
  manual: "Manual",
  all: "All issues",
  pastDay: "Past day",
  pastWeek: "Past week",
  pastMonth: "Past month",
};

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex items-center justify-between gap-4 text-sm">
      <span>{label}</span>
      {children}
    </label>
  );
}

const selectCls = "h-8 rounded-md border border-input bg-transparent px-2 text-sm";

/**
 * Anzeigeoptionen (docs/05 §2.1) als Dialog. Jede Änderung ist persönlich und wird sofort über
 * `onChange` gemeldet (der Aufrufer speichert sie per `viewPreferencesUpdate`).
 */
export function DisplayMenu({
  options,
  onChange,
  onReset,
}: {
  options: DisplayOptions;
  onChange: (patch: Partial<DisplayOptions>) => void;
  onReset?: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="ghost" size="sm" onClick={() => setOpen(true)} aria-haspopup="dialog">
        <SlidersHorizontal /> Display
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-sm" aria-describedby={undefined}>
          <DialogTitle>Display options</DialogTitle>
          <DialogDescription className="sr-only">
            Grouping, ordering and visibility of issues.
          </DialogDescription>
          <div className="flex flex-col gap-3">
            <Row label="Grouping">
              <select
                aria-label="Grouping"
                className={selectCls}
                value={options.grouping}
                onChange={(e) =>
                  onChange({ grouping: e.target.value as DisplayOptions["grouping"] })
                }
              >
                {GROUPINGS.map((g) => (
                  <option key={g} value={g}>
                    {LABEL[g]}
                  </option>
                ))}
              </select>
            </Row>
            <Row label="Ordering">
              <select
                aria-label="Ordering"
                className={selectCls}
                value={options.ordering}
                onChange={(e) =>
                  onChange({ ordering: e.target.value as DisplayOptions["ordering"] })
                }
              >
                {ORDERINGS.map((g) => (
                  <option key={g} value={g}>
                    {LABEL[g] ?? "Priority"}
                  </option>
                ))}
              </select>
            </Row>
            <Row label="Direction">
              <select
                aria-label="Direction"
                className={selectCls}
                value={options.orderDirection}
                onChange={(e) =>
                  onChange({ orderDirection: e.target.value as DisplayOptions["orderDirection"] })
                }
              >
                <option value="asc">Ascending</option>
                <option value="desc">Descending</option>
              </select>
            </Row>
            <Row label="Completed issues">
              <select
                aria-label="Completed issues"
                className={selectCls}
                value={options.showCompletedIssues}
                onChange={(e) =>
                  onChange({
                    showCompletedIssues: e.target.value as DisplayOptions["showCompletedIssues"],
                  })
                }
              >
                {COMPLETED_WINDOWS.map((w) => (
                  <option key={w} value={w}>
                    {w === "none" ? "None" : LABEL[w]}
                  </option>
                ))}
              </select>
            </Row>
            <Row label="Show sub-issues">
              <input
                type="checkbox"
                aria-label="Show sub-issues"
                checked={options.showSubIssues}
                onChange={(e) => onChange({ showSubIssues: e.target.checked })}
              />
            </Row>
            {onReset && (
              <Button variant="ghost" size="sm" className="self-end" onClick={onReset}>
                Reset to default
              </Button>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
