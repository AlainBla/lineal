import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import "@/app/test-polyfills";

import { Dialog, DialogContent, DialogDescription, DialogTitle } from "./dialog";

function Harness() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)}>opener</button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogTitle>Dialog</DialogTitle>
          <DialogDescription>Text</DialogDescription>
          <input aria-label="field" />
        </DialogContent>
      </Dialog>
    </>
  );
}

describe("DialogContent Fokus (B4.25)", () => {
  it("gibt den Fokus beim Schließen an das öffnende Element zurück (auch ohne Trigger)", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "opener" });
    opener.focus();
    await user.keyboard("{Enter}");
    expect(await screen.findByLabelText("field")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByLabelText("field")).toBeNull());
    await waitFor(() => expect(opener).toHaveFocus());
  });
});
