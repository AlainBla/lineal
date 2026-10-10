import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Führt Klassen zusammen (Tailwind-Konflikte werden aufgelöst). */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
