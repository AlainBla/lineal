import { createRouter, RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { Providers } from "@/app/providers";
import { routeTree } from "./routeTree.gen";
import "./index.css";

// Zurück aus einem Issue (Esc/Browser-Zurück) stellt die Scrollposition der Liste wieder her (#main).
const router = createRouter({
  routeTree,
  scrollRestoration: true,
  scrollToTopSelectors: ["#main"],
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");

createRoot(root).render(
  <StrictMode>
    <Providers>
      <RouterProvider router={router} />
    </Providers>
  </StrictMode>,
);
