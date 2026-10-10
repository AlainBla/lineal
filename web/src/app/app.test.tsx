import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/cn";
import { routeTree } from "../routeTree.gen";
import { Providers } from "./providers";

describe("scaffold", () => {
  it("renders the home route at /", async () => {
    const router = createRouter({
      routeTree,
      history: createMemoryHistory({ initialEntries: ["/"] }),
    });
    render(
      <Providers>
        <RouterProvider router={router} />
      </Providers>,
    );
    expect(await screen.findByRole("heading", { name: "Lineal" })).toBeInTheDocument();
  });

  it("renders a shadcn button", () => {
    render(<Button variant="secondary">Save</Button>);
    expect(screen.getByRole("button", { name: "Save" })).toHaveAttribute("data-slot", "button");
  });

  it("cn merges conflicting tailwind classes", () => {
    expect(cn("px-2", "px-4", undefined)).toBe("px-4");
  });
});
