/** Account → Appearance (`/{ws}/settings/account/appearance`): Theme hell, dunkel oder System. */
import { useSession } from "@/app/auth";
import { useTheme, type ThemePreference } from "@/lib/theme";

import { workspaceSettingsNav } from "./nav";
import { Section, SettingsShell } from "./ui";

const OPTIONS: ReadonlyArray<{ value: ThemePreference; label: string; hint: string }> = [
  { value: "light", label: "Light", hint: "Bright background." },
  { value: "dark", label: "Dark", hint: "Dark background (default)." },
  { value: "system", label: "System", hint: "Follow the setting of your device." },
];

export function AccountAppearancePage({ ws }: { ws: string }) {
  const session = useSession();
  const { preference, setTheme } = useTheme();
  return (
    <SettingsShell
      title="Settings"
      nav={workspaceSettingsNav(ws, session, { account: "appearance" })}
    >
      <Section
        title="Appearance"
        description="The theme is saved in this browser and applies right away."
      >
        <div role="radiogroup" aria-label="Theme" className="flex flex-col gap-2">
          {OPTIONS.map((o) => (
            <label key={o.value} className="flex items-start gap-2 text-sm">
              <input
                type="radio"
                name="theme"
                className="mt-1"
                value={o.value}
                checked={preference === o.value}
                onChange={() => setTheme(o.value)}
              />
              <span>
                {o.label}
                <span className="block text-xs text-muted-foreground">{o.hint}</span>
              </span>
            </label>
          ))}
        </div>
      </Section>
    </SettingsShell>
  );
}
