import type { ThemeState } from "./protocol";

// Stock Flow-Like token set, copied from packages/ui/global.css so standalone
// widgets render correctly themed with zero setup.
export const DEFAULT_LIGHT_TOKENS: Record<string, string> = {
	"--background": "oklch(0.9841 0.0035 39.4848)",
	"--foreground": "oklch(0.184 0.0081 285.5768)",
	"--card": "oklch(1.0 0 0)",
	"--card-foreground": "oklch(0.184 0.0081 285.5768)",
	"--popover": "oklch(1.0 0 0)",
	"--popover-foreground": "oklch(0.184 0.0081 285.5768)",
	"--primary": "oklch(0.6724 0.208 34.7886)",
	"--primary-foreground": "oklch(1.0 0 0)",
	"--secondary": "oklch(0.9466 0.0084 56.3108)",
	"--secondary-foreground": "oklch(0.2503 0.0113 285.5586)",
	"--muted": "oklch(0.9401 0.0068 53.4438)",
	"--muted-foreground": "oklch(0.5137 0.0156 285.8367)",
	"--accent": "oklch(0.6488 0.2155 31.664)",
	"--accent-foreground": "oklch(1.0 0 0)",
	"--destructive": "oklch(0.6227 0.2289 23.472)",
	"--destructive-foreground": "oklch(1.0 0 0)",
	"--border": "oklch(0.8933 0.0111 54.4825)",
	"--input": "oklch(0.8933 0.0111 54.4825)",
	"--ring": "oklch(0.6724 0.208 34.7886)",
	"--chart-1": "oklch(0.6724 0.208 34.7886)",
	"--chart-2": "oklch(0.7069 0.1871 45.3923)",
	"--chart-3": "oklch(0.6227 0.2289 23.472)",
	"--chart-4": "oklch(0.7814 0.1487 63.8806)",
	"--chart-5": "oklch(0.5975 0.0196 285.7923)",
	"--font-sans": '"Inter", ui-sans-serif, system-ui, sans-serif',
	"--font-serif": '"Playfair Display", "Didot", Georgia, serif',
	"--font-mono": '"JetBrains Mono", ui-monospace, monospace',
	"--radius": "0.375rem",
};

export const DEFAULT_DARK_TOKENS: Record<string, string> = {
	"--background": "oklch(0.164 0.0111 268.0057)",
	"--foreground": "oklch(0.9544 0.0059 59.6503)",
	"--card": "oklch(0.1993 0.0111 260.661)",
	"--card-foreground": "oklch(0.9544 0.0059 59.6503)",
	"--popover": "oklch(0.1993 0.0111 260.661)",
	"--popover-foreground": "oklch(0.9544 0.0059 59.6503)",
	"--primary": "oklch(0.6724 0.208 34.7886)",
	"--primary-foreground": "oklch(1.0 0 0)",
	"--secondary": "oklch(0.2515 0.0122 264.3278)",
	"--secondary-foreground": "oklch(0.9247 0.0077 61.447)",
	"--muted": "oklch(0.264 0.012 264.345)",
	"--muted-foreground": "oklch(0.6984 0.0157 264.467)",
	"--accent": "oklch(0.6488 0.2155 31.664)",
	"--accent-foreground": "oklch(1.0 0 0)",
	"--destructive": "oklch(0.6227 0.2289 23.472)",
	"--destructive-foreground": "oklch(1.0 0 0)",
	"--border": "oklch(0.2921 0.014 261.7008)",
	"--input": "oklch(0.2921 0.014 261.7008)",
	"--ring": "oklch(0.6724 0.208 34.7886)",
	"--chart-1": "oklch(0.6724 0.208 34.7886)",
	"--chart-2": "oklch(0.7069 0.1871 45.3923)",
	"--chart-3": "oklch(0.6227 0.2289 23.472)",
	"--chart-4": "oklch(0.7814 0.1487 63.8806)",
	"--chart-5": "oklch(0.6984 0.0157 264.467)",
	"--font-sans": "Open Sans, sans-serif",
	"--font-serif": "Georgia, serif",
	"--font-mono": "Menlo, monospace",
	"--radius": "0.375rem",
};

export function applyTheme(theme: ThemeState): void {
	if (typeof document === "undefined") return;
	const root = document.documentElement;
	for (const [name, value] of Object.entries(theme.tokens)) {
		if (name.startsWith("--")) root.style.setProperty(name, value);
	}
	root.classList.toggle("dark", theme.mode === "dark");
}
