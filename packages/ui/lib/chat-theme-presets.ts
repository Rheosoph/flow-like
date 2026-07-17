export interface ChatThemePreset {
	value: string;
	label: string;
	description: string;
	badge: string;
	preview: {
		background: string;
		accent: string;
	};
	css: string;
}

export const CUSTOM_CHAT_THEME_VALUE = "custom" as const;

export const CHAT_THEME_PRESETS = [
	{
		value: "flow-like",
		label: "Flow Like",
		description: "Clean defaults that inherit the app palette.",
		badge: "App-aware",
		preview: {
			background:
				"linear-gradient(135deg, #f8fafc 0 48%, #e2e8f0 48% 52%, #0f172a 52%)",
			accent: "#f0442b",
		},
		css: `
/* Flow Like — clean, app-aware defaults */
:root {
  --fl-chat-content-width: 64rem;
  --fl-chat-message-radius: 0.75rem;
  --fl-chat-surface-background: transparent;
  --fl-chat-composer-background: var(--background);
  --fl-chat-user-message-background: color-mix(in oklch, var(--muted) 30%, transparent);
  --fl-chat-user-message-foreground: var(--foreground);
  --fl-chat-ai-message-background: transparent;
  --fl-chat-ai-message-foreground: var(--foreground);
  --fl-chat-disclosure-background: color-mix(in oklch, var(--primary) 10%, var(--background));
  --fl-chat-disclosure-foreground: var(--foreground);
  --fl-chat-background-overlay: color-mix(in oklch, var(--background) 38%, transparent);
  --fl-chat-background-overlay-strong: color-mix(in oklch, var(--background) 56%, transparent);
  --fl-chat-background-size: cover;
  --fl-chat-background-position: center;
  --fl-chat-pad-bottom: 0.75rem;
}

:root[data-fl-chat-has-background="true"] {
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 76%, transparent);
  --fl-chat-user-message-background: color-mix(in oklch, var(--muted) 82%, transparent);
  --fl-chat-ai-message-background: color-mix(in oklch, var(--background) 76%, transparent);
  --fl-chat-disclosure-background: color-mix(in oklch, var(--background) 84%, transparent);
}
		`.trim(),
	},
	{
		value: "executive",
		label: "Executive",
		description: "Restrained spacing, crisp cards, and confident blue.",
		badge: "Professional",
		preview: {
			background: "linear-gradient(135deg, #f8fafc, #dbeafe)",
			accent: "#2563eb",
		},
		css: `
/* Executive — restrained and professional */
:root {
  --primary: oklch(0.56 0.17 255);
  --primary-foreground: white;
  --ring: color-mix(in oklch, var(--primary) 75%, white);
  --radius: 0.75rem;
  --fl-chat-content-width: 56rem;
  --fl-chat-message-radius: 1rem;
  --fl-chat-surface-background: var(--background);
  --fl-chat-composer-background: var(--card);
  --fl-chat-user-message-background: var(--primary);
  --fl-chat-user-message-foreground: var(--primary-foreground);
  --fl-chat-ai-message-background: var(--card);
  --fl-chat-ai-message-foreground: var(--card-foreground);
  --fl-chat-disclosure-background: var(--muted);
  --fl-chat-disclosure-foreground: var(--muted-foreground);
}

[data-fl-chat-message="assistant"] {
  border: 1px solid var(--border);
  box-shadow: 0 8px 24px color-mix(in oklch, var(--foreground) 6%, transparent);
}

[data-fl-chat-message="user"] {
  box-shadow: 0 6px 18px color-mix(in oklch, var(--primary) 18%, transparent);
}

[data-fl-chat-composer] {
  border-radius: 1rem;
  box-shadow: 0 10px 30px color-mix(in oklch, var(--foreground) 8%, transparent);
}
		`.trim(),
	},
	{
		value: "editorial",
		label: "Editorial",
		description: "Quiet luxury with serif typography and fine rules.",
		badge: "Professional",
		preview: {
			background: "linear-gradient(135deg, #fffdf7, #e8dfcf)",
			accent: "#7c5c3e",
		},
		css: `
/* Editorial — quiet, typographic luxury */
:root {
  --fl-chat-editorial-font: Iowan Old Style, Baskerville, Times New Roman, ui-serif, serif;
  --font-sans: var(--fl-chat-editorial-font);
  --font-serif: var(--fl-chat-editorial-font);
  --primary: color-mix(in oklch, var(--foreground) 70%, oklch(0.67 0.09 55));
  --primary-foreground: var(--background);
  --border: color-mix(in oklch, var(--foreground) 18%, var(--background));
  --fl-chat-content-width: 50rem;
  --fl-chat-message-radius: 0.125rem;
  --fl-chat-surface-background: var(--background);
  --fl-chat-composer-background: var(--background);
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 10%, var(--background));
  --fl-chat-ai-message-background: transparent;
  --fl-chat-disclosure-background: transparent;
  font-family: var(--fl-chat-editorial-font);
}

[data-fl-chat-welcome] h1 {
  font-family: var(--fl-chat-editorial-font);
  font-weight: 500;
  letter-spacing: -0.025em;
}

[data-fl-chat-message] {
  line-height: 1.75;
}

[data-fl-chat-message="assistant"] {
  border-top: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
}

[data-fl-chat-message="user"] {
  border-left: 3px solid var(--primary);
}

[data-fl-chat-composer] {
  border-bottom: 2px solid var(--primary);
  border-radius: 0;
}

[data-fl-chat-ai-disclosure] {
  font-style: italic;
}
		`.trim(),
	},
	{
		value: "typewriter",
		label: "Typewriter",
		description: "Monospace paper texture with stamped message cards.",
		badge: "Character",
		preview: {
			background:
				"repeating-linear-gradient(0deg, #f4ecd8 0 6px, #ded2b7 6px 7px)",
			accent: "#743c2d",
		},
		css: `
/* Typewriter — adaptive paper and ink */
:root {
  --fl-chat-typewriter-font: "Courier New", Courier, ui-monospace, monospace;
  --font-sans: var(--fl-chat-typewriter-font);
  --font-mono: var(--fl-chat-typewriter-font);
  --primary: color-mix(in oklch, var(--foreground) 72%, oklch(0.62 0.08 70));
  --primary-foreground: var(--background);
  --border: color-mix(in oklch, var(--foreground) 24%, var(--background));
  --ring: var(--primary);
  --radius: 0;
  --fl-chat-content-width: 52rem;
  --fl-chat-message-radius: 0;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 96%, var(--primary));
  --fl-chat-composer-background: var(--background);
  --fl-chat-user-message-background: color-mix(in oklch, var(--foreground) 7%, var(--background));
  --fl-chat-ai-message-background: transparent;
  --fl-chat-disclosure-background: transparent;
  font-family: var(--fl-chat-typewriter-font);
  letter-spacing: 0.015em;
}

[data-fl-chat-surface] {
  background-image: repeating-linear-gradient(
    0deg,
    transparent 0 1.75rem,
    color-mix(in oklch, var(--foreground) 7%, transparent) 1.75rem calc(1.75rem + 1px)
  );
}

[data-fl-chat-message="assistant"] {
  border-left: 2px solid var(--primary);
  padding-left: 1rem;
}

[data-fl-chat-message="user"] {
  border: 1px solid var(--border);
  box-shadow: 3px 3px 0 color-mix(in oklch, var(--foreground) 12%, transparent);
}

[data-fl-chat-composer] {
  border-radius: 0;
  box-shadow: 4px 4px 0 color-mix(in oklch, var(--foreground) 16%, transparent);
}

[data-fl-chat-composer] textarea {
  font-family: inherit;
}

[data-fl-chat-ai-disclosure] {
  border: 1px dashed var(--border);
  border-radius: 0;
}
		`.trim(),
	},
	{
		value: "neon-grid",
		label: "Neon Grid",
		description: "Sci-fi grid, holographic glow, and a moving border.",
		badge: "Animated",
		preview: {
			background: "linear-gradient(135deg, #07111f, #12233e 55%, #291348)",
			accent: "#30f2e2",
		},
		css: `
/* Neon Grid — sci-fi glow with an animated composer border */
:root {
  --fl-chat-neon-font: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  --font-sans: var(--fl-chat-neon-font);
  --font-mono: var(--fl-chat-neon-font);
  --primary: oklch(0.82 0.17 190);
  --primary-foreground: oklch(0.16 0.03 230);
  --accent: oklch(0.72 0.2 315);
  --accent-foreground: white;
  --ring: var(--primary);
  --border: color-mix(in oklch, var(--primary) 32%, var(--background));
  --fl-chat-content-width: 72rem;
  --fl-chat-message-radius: 0.25rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 94%, transparent);
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 88%, transparent);
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 18%, var(--background));
  --fl-chat-ai-message-background: color-mix(in oklch, var(--foreground) 4%, transparent);
  --fl-chat-disclosure-background: color-mix(in oklch, var(--primary) 12%, var(--background));
  font-family: var(--fl-chat-neon-font);
}

[data-fl-chat-surface] {
  background-image:
    linear-gradient(color-mix(in oklch, var(--primary) 9%, transparent) 1px, transparent 1px),
    linear-gradient(90deg, color-mix(in oklch, var(--primary) 9%, transparent) 1px, transparent 1px),
    radial-gradient(circle at 50% -20%, color-mix(in oklch, var(--primary) 22%, transparent), transparent 55%);
  background-size: 32px 32px, 32px 32px, 100% 100%;
  animation: fl-chat-neon-grid 24s linear infinite;
}

[data-fl-chat-message] {
  border: 1px solid color-mix(in oklch, var(--primary) 22%, transparent);
  box-shadow: inset 0 1px 0 color-mix(in oklch, var(--primary) 10%, transparent);
}

[data-fl-chat-composer] {
  position: relative;
  border-radius: 0.875rem;
  box-shadow: 0 0 24px color-mix(in oklch, var(--primary) 12%, transparent);
}

[data-fl-chat-composer]::before {
  content: "";
  position: absolute;
  inset: -2px;
  border-radius: inherit;
  padding: 1px;
  pointer-events: none;
  background: linear-gradient(90deg, transparent, var(--primary), var(--accent), transparent);
  background-size: 240% 100%;
  animation: fl-chat-neon-border 3s linear infinite;
  -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
  -webkit-mask-composite: xor;
  mask-composite: exclude;
}

@keyframes fl-chat-neon-grid {
  from { background-position: 0 0, 0 0, 50% 0; }
  to { background-position: 32px 32px, 32px 32px, 50% 100%; }
}

@keyframes fl-chat-neon-border {
  to { background-position: -240% 0; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface],
  [data-fl-chat-composer]::before {
    animation: none;
  }
}
		`.trim(),
	},
	{
		value: "aurora-glass",
		label: "Aurora Glass",
		description: "A fluid gradient shader behind soft glass surfaces.",
		badge: "Animated",
		preview: {
			background: "linear-gradient(135deg, #312e81, #a855f7 48%, #2dd4bf 100%)",
			accent: "#ddd6fe",
		},
		css: `
/* Aurora Glass — animated CSS shader and translucent surfaces */
:root {
  --primary: oklch(0.68 0.2 300);
  --primary-foreground: white;
  --accent: oklch(0.75 0.16 190);
  --ring: var(--primary);
  --fl-chat-content-width: 68rem;
  --fl-chat-message-radius: 1.25rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 78%, transparent);
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 72%, transparent);
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 24%, var(--background));
  --fl-chat-ai-message-background: color-mix(in oklch, var(--card) 74%, transparent);
  --fl-chat-disclosure-background: color-mix(in oklch, var(--accent) 14%, var(--background));
}

[data-fl-chat-surface] {
  background-image:
    radial-gradient(circle at 15% 20%, color-mix(in oklch, var(--primary) 28%, transparent), transparent 38%),
    radial-gradient(circle at 85% 25%, color-mix(in oklch, var(--accent) 24%, transparent), transparent 35%),
    radial-gradient(circle at 50% 90%, color-mix(in oklch, var(--primary) 16%, transparent), transparent 42%);
  background-size: 160% 160%;
  animation: fl-chat-aurora 18s ease-in-out infinite alternate;
}

[data-fl-chat-message] {
  border: 1px solid color-mix(in oklch, var(--foreground) 10%, transparent);
  box-shadow: 0 12px 32px color-mix(in oklch, var(--foreground) 8%, transparent);
}

[data-fl-chat-composer] {
  border-radius: 1rem;
  box-shadow: 0 18px 50px color-mix(in oklch, var(--primary) 14%, transparent);
}

@supports (backdrop-filter: blur(1px)) {
  [data-fl-chat-message],
  [data-fl-chat-composer] {
    backdrop-filter: blur(14px) saturate(1.15);
    -webkit-backdrop-filter: blur(14px) saturate(1.15);
  }
}

@keyframes fl-chat-aurora {
  0% { background-position: 0% 15%, 100% 20%, 50% 100%; }
  100% { background-position: 70% 80%, 20% 65%, 60% 20%; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface] {
    animation: none;
  }
}
		`.trim(),
	},
	{
		value: "green-screen",
		label: "Green Screen",
		description: "Retro terminal scanlines, monospace type, and phosphor glow.",
		badge: "Animated",
		preview: {
			background: "linear-gradient(135deg, #03160b, #0b2b18)",
			accent: "#4ade80",
		},
		css: `
/* Green Screen — retro terminal without forcing a color mode */
:root {
  --fl-chat-terminal-font: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  --font-sans: var(--fl-chat-terminal-font);
  --font-mono: var(--fl-chat-terminal-font);
  --primary: oklch(0.76 0.2 145);
  --primary-foreground: oklch(0.14 0.03 145);
  --accent: oklch(0.82 0.16 105);
  --ring: var(--primary);
  --border: color-mix(in oklch, var(--primary) 34%, var(--background));
  --radius: 0.125rem;
  --fl-chat-content-width: 66rem;
  --fl-chat-message-radius: 0.125rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 95%, var(--primary));
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 92%, var(--primary));
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 18%, var(--background));
  --fl-chat-ai-message-background: transparent;
  --fl-chat-disclosure-background: color-mix(in oklch, var(--primary) 8%, var(--background));
  font-family: var(--fl-chat-terminal-font);
}

[data-fl-chat-surface] {
  background-image: repeating-linear-gradient(
    0deg,
    transparent 0 3px,
    color-mix(in oklch, var(--primary) 5%, transparent) 3px 4px
  );
  animation: fl-chat-terminal-scan 8s linear infinite;
}

[data-fl-chat-message="assistant"] {
  border-left: 1px solid var(--primary);
  text-shadow: 0 0 10px color-mix(in oklch, var(--primary) 25%, transparent);
}

[data-fl-chat-message="user"] {
  border: 1px solid var(--border);
}

[data-fl-chat-composer] {
  outline: 1px solid color-mix(in oklch, var(--primary) 42%, transparent);
  outline-offset: 2px;
}

[data-fl-chat-composer] textarea {
  font-family: inherit;
  caret-color: var(--primary);
}

@keyframes fl-chat-terminal-scan {
  to { background-position: 0 4px; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface] {
    animation: none;
  }
}
		`.trim(),
	},
	{
		value: "blueprint",
		label: "Blueprint",
		description: "Measured grid, drafting blue, and precise technical framing.",
		badge: "Professional",
		preview: {
			background: "linear-gradient(135deg, #eff6ff, #c7dcf8)",
			accent: "#2563eb",
		},
		css: `
/* Blueprint — technical precision */
:root {
  --fl-chat-blueprint-font: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  --font-sans: var(--fl-chat-blueprint-font);
  --primary: oklch(0.56 0.17 252);
  --primary-foreground: white;
  --border: color-mix(in oklch, var(--primary) 34%, var(--background));
  --ring: var(--primary);
  --radius: 0.25rem;
  --fl-chat-content-width: 62rem;
  --fl-chat-message-radius: 0.25rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 97%, var(--primary));
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 94%, var(--primary));
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 14%, var(--background));
  --fl-chat-ai-message-background: color-mix(in oklch, var(--background) 90%, transparent);
  font-family: var(--fl-chat-blueprint-font);
}

[data-fl-chat-surface] {
  background-image:
    linear-gradient(color-mix(in oklch, var(--primary) 9%, transparent) 1px, transparent 1px),
    linear-gradient(90deg, color-mix(in oklch, var(--primary) 9%, transparent) 1px, transparent 1px);
  background-size: 24px 24px;
}

[data-fl-chat-welcome-panel] {
  border: 1px solid var(--border);
  padding: clamp(1rem, 3vw, 2rem);
}

[data-fl-chat-welcome] h1 {
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

[data-fl-chat-message="assistant"] {
  border-block: 1px solid var(--border);
  border-inline-start: 3px solid var(--primary);
}

[data-fl-chat-message="user"] {
  border: 1px solid var(--primary);
  box-shadow: 3px 3px 0 color-mix(in oklch, var(--primary) 20%, transparent);
}

[data-fl-chat-composer] {
  border: 1px solid var(--primary);
  border-radius: 0.25rem;
  box-shadow: inset 0 0 0 3px color-mix(in oklch, var(--primary) 7%, transparent);
}

[data-fl-chat-suggestion] {
  border-bottom: 1px solid var(--border);
  border-radius: 0;
}
		`.trim(),
	},
	{
		value: "swiss",
		label: "Swiss",
		description:
			"International typographic discipline with bold red structure.",
		badge: "Professional",
		preview: {
			background: "linear-gradient(135deg, #faf9f5, #dedbd3)",
			accent: "#e11d2e",
		},
		css: `
/* Swiss — typographic order and decisive geometry */
:root {
  --fl-chat-swiss-font: "Helvetica Neue", Helvetica, Arial, ui-sans-serif, sans-serif;
  --font-sans: var(--fl-chat-swiss-font);
  --primary: oklch(0.57 0.23 27);
  --primary-foreground: white;
  --border: color-mix(in oklch, var(--foreground) 22%, var(--background));
  --ring: var(--primary);
  --radius: 0;
  --fl-chat-content-width: 54rem;
  --fl-chat-message-radius: 0;
  --fl-chat-surface-background: var(--background);
  --fl-chat-composer-background: var(--background);
  --fl-chat-user-message-background: var(--primary);
  --fl-chat-user-message-foreground: var(--primary-foreground);
  --fl-chat-ai-message-background: transparent;
  font-family: var(--fl-chat-swiss-font);
}

[data-fl-chat-surface] {
  background-image: linear-gradient(90deg, var(--primary) 0 6px, transparent 6px);
}

[data-fl-chat-welcome-panel] {
  border-inline-start: 8px solid var(--primary);
  padding-inline-start: clamp(1rem, 4vw, 3rem);
}

[data-fl-chat-welcome] h1 {
  font-weight: 800;
  letter-spacing: -0.045em;
  text-align: left;
  text-transform: uppercase;
}

[data-fl-chat-welcome] h1 + p {
  text-align: left;
}

[data-fl-chat-messages] {
  gap: 2.5rem;
}

[data-fl-chat-message="assistant"] {
  border-top: 1px solid var(--foreground);
  padding-inline: 0;
}

[data-fl-chat-message="user"] {
  border-inline-end: 6px solid color-mix(in oklch, var(--foreground) 35%, transparent);
}

[data-fl-chat-composer] {
  border-top: 4px solid var(--primary);
  border-radius: 0;
  box-shadow: none;
}

[data-fl-chat-suggestion] {
  border-bottom: 1px solid var(--border);
  border-radius: 0;
  font-weight: 600;
}

[data-fl-chat-ai-disclosure] {
  letter-spacing: 0.08em;
  text-transform: uppercase;
}
		`.trim(),
	},
	{
		value: "nordic",
		label: "Nordic",
		description: "Muted spruce, soft planes, and generous breathing room.",
		badge: "Professional",
		preview: {
			background: "linear-gradient(135deg, #f4f8f6, #d6e6df)",
			accent: "#2f7774",
		},
		css: `
/* Nordic — calm, tactile restraint */
:root {
  --fl-chat-nordic-font: Inter, Avenir, "Segoe UI", ui-sans-serif, sans-serif;
  --font-sans: var(--fl-chat-nordic-font);
  --primary: oklch(0.55 0.09 190);
  --primary-foreground: white;
  --border: color-mix(in oklch, var(--foreground) 12%, var(--background));
  --ring: color-mix(in oklch, var(--primary) 70%, var(--background));
  --radius: 1.25rem;
  --fl-chat-content-width: 58rem;
  --fl-chat-message-radius: 1.25rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 96%, var(--primary));
  --fl-chat-composer-background: color-mix(in oklch, var(--card) 92%, var(--background));
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 18%, var(--background));
  --fl-chat-ai-message-background: color-mix(in oklch, var(--card) 78%, transparent);
  --fl-chat-disclosure-background: color-mix(in oklch, var(--primary) 7%, var(--background));
  font-family: var(--fl-chat-nordic-font);
}

[data-fl-chat-surface] {
  background-image: radial-gradient(circle at 12% 8%, color-mix(in oklch, var(--primary) 9%, transparent), transparent 34%);
}

[data-fl-chat-welcome-panel] {
  padding: clamp(0.5rem, 2vw, 1.5rem);
}

[data-fl-chat-welcome] h1 {
  font-weight: 600;
  letter-spacing: -0.035em;
}

[data-fl-chat-message] {
  border: 1px solid var(--border);
  box-shadow: 0 8px 24px color-mix(in oklch, var(--foreground) 5%, transparent);
}

[data-fl-chat-message="assistant"] {
  border-inline-start: 4px solid color-mix(in oklch, var(--primary) 55%, var(--background));
}

[data-fl-chat-composer] {
  border-radius: 1.5rem;
  box-shadow: 0 14px 38px color-mix(in oklch, var(--foreground) 8%, transparent);
}

[data-fl-chat-suggestion] {
  border: 1px solid var(--border);
  border-radius: 999px;
  padding-inline: 1rem;
}

[data-fl-chat-ai-disclosure] {
  border-radius: 999px;
}
		`.trim(),
	},
	{
		value: "brutalist",
		label: "Brutalist",
		description: "Hard rules, offset ink shadows, and uncompromising geometry.",
		badge: "Studio",
		preview: {
			background:
				"repeating-linear-gradient(135deg, #fff 0 18px, #e5e5e5 18px 20px)",
			accent: "#facc15",
		},
		css: `
/* Brutalist — loud structure, zero decoration debt */
:root {
  --fl-chat-brutalist-font: Arial Black, Impact, ui-sans-serif, sans-serif;
  --font-sans: var(--fl-chat-brutalist-font);
  --primary: oklch(0.84 0.17 95);
  --primary-foreground: oklch(0.18 0.02 95);
  --border: var(--foreground);
  --ring: var(--foreground);
  --radius: 0;
  --fl-chat-content-width: 60rem;
  --fl-chat-message-radius: 0;
  --fl-chat-surface-background: var(--background);
  --fl-chat-composer-background: var(--background);
  --fl-chat-user-message-background: var(--foreground);
  --fl-chat-user-message-foreground: var(--background);
  --fl-chat-ai-message-background: var(--background);
  font-family: var(--fl-chat-brutalist-font);
}

[data-fl-chat-surface] {
  background-image: repeating-linear-gradient(135deg, transparent 0 28px, color-mix(in oklch, var(--foreground) 5%, transparent) 28px 30px);
}

[data-fl-chat-welcome-panel] {
  border: 3px solid var(--foreground);
  padding: clamp(1rem, 3vw, 2rem);
  box-shadow: 8px 8px 0 var(--foreground);
}

[data-fl-chat-welcome] h1 {
  font-size: clamp(2rem, 7vw, 4.5rem);
  letter-spacing: -0.07em;
  line-height: 0.88;
  text-transform: uppercase;
}

[data-fl-chat-message] {
  border: 2px solid var(--foreground);
}

[data-fl-chat-message="user"] {
  box-shadow: 5px 5px 0 var(--primary);
}

[data-fl-chat-message="assistant"] {
  box-shadow: 5px 5px 0 color-mix(in oklch, var(--foreground) 18%, transparent);
}

[data-fl-chat-composer] {
  border: 3px solid var(--foreground);
  border-radius: 0;
  box-shadow: 6px 6px 0 var(--foreground);
}

[data-fl-chat-suggestion] {
  border: 2px solid var(--foreground);
  border-radius: 0;
  font-weight: 800;
  text-transform: uppercase;
}

[data-fl-chat-suggestion]:hover {
  box-shadow: 3px 3px 0 var(--primary);
  transform: translate(-2px, -2px);
}

[data-fl-chat-ai-disclosure] {
  border-top: 2px solid var(--foreground);
  border-radius: 0;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}
		`.trim(),
	},
	{
		value: "art-deco",
		label: "Art Deco",
		description:
			"Brass geometry, elegant symmetry, and crisp architectural lines.",
		badge: "Elegant",
		preview: {
			background:
				"repeating-linear-gradient(135deg, #171717 0 12px, #292524 12px 14px)",
			accent: "#d4a853",
		},
		css: `
/* Art Deco — architectural brass and geometric rhythm */
:root {
  --fl-chat-deco-font: Optima, Candara, "Trebuchet MS", ui-sans-serif, sans-serif;
  --font-sans: var(--fl-chat-deco-font);
  --primary: oklch(0.72 0.11 78);
  --primary-foreground: oklch(0.16 0.02 70);
  --ring: var(--primary);
  --border: color-mix(in oklch, var(--foreground) 24%, var(--background));
  --radius: 0;
  --fl-chat-content-width: 58rem;
  --fl-chat-message-radius: 0;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 97%, var(--primary));
  --fl-chat-composer-background: var(--background);
  --fl-chat-user-message-background: var(--primary);
  --fl-chat-user-message-foreground: var(--primary-foreground);
  --fl-chat-ai-message-background: color-mix(in oklch, var(--foreground) 4%, var(--background));
  --fl-chat-disclosure-background: transparent;
  font-family: var(--fl-chat-deco-font);
}

[data-fl-chat-welcome-panel] {
  border-block: 1px solid var(--primary);
  padding-block: 1.75rem;
}

[data-fl-chat-welcome-panel]::before {
  content: "";
  display: block;
  width: min(12rem, 45%);
  height: 3px;
  margin: 0 auto 1.5rem;
  background: linear-gradient(90deg, transparent, var(--primary) 20% 80%, transparent);
}

[data-fl-chat-message] {
  border: 1px solid var(--border);
  letter-spacing: 0.012em;
}

[data-fl-chat-message="user"] {
  box-shadow: 4px 4px 0 color-mix(in oklch, var(--foreground) 16%, transparent);
}

[data-fl-chat-composer] {
  border: 1px solid var(--primary);
  border-radius: 0;
  outline: 1px solid var(--border);
  outline-offset: 3px;
}
		`.trim(),
	},
	{
		value: "solarpunk",
		label: "Solarpunk",
		description:
			"Lush canopy light, warm sun flare, and optimistic organic forms.",
		badge: "Animated",
		preview: {
			background: "linear-gradient(135deg, #123c2b, #4d8b45 55%, #f4c95d)",
			accent: "#76d275",
		},
		css: `
/* Solarpunk — living color and a slowly moving canopy */
:root {
  --primary: oklch(0.67 0.15 145);
  --primary-foreground: oklch(0.16 0.04 145);
  --accent: oklch(0.82 0.14 88);
  --ring: var(--primary);
  --border: color-mix(in oklch, var(--primary) 26%, var(--background));
  --fl-chat-content-width: 62rem;
  --fl-chat-message-radius: 1.5rem 1.5rem 0.4rem 1.5rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 94%, var(--primary));
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 94%, var(--accent));
  --fl-chat-user-message-background: var(--primary);
  --fl-chat-user-message-foreground: var(--primary-foreground);
  --fl-chat-ai-message-background: color-mix(in oklch, var(--primary) 8%, var(--background));
  --fl-chat-disclosure-background: color-mix(in oklch, var(--accent) 10%, var(--background));
}

[data-fl-chat-surface] {
  background-image:
    radial-gradient(circle at 12% 5%, color-mix(in oklch, var(--accent) 28%, transparent), transparent 30%),
    radial-gradient(circle at 88% 20%, color-mix(in oklch, var(--primary) 22%, transparent), transparent 36%),
    radial-gradient(ellipse at 45% 105%, color-mix(in oklch, var(--primary) 14%, transparent), transparent 46%);
  background-size: 130% 130%;
  animation: fl-chat-solarpunk-bloom 18s ease-in-out infinite alternate;
}

[data-fl-chat-message] {
  border: 1px solid color-mix(in oklch, var(--primary) 24%, transparent);
  box-shadow: 0 10px 26px color-mix(in oklch, var(--foreground) 7%, transparent);
}

[data-fl-chat-message="assistant"] {
  border-left: 3px solid var(--primary);
}

[data-fl-chat-composer] {
  border-radius: 1.4rem;
  box-shadow: 0 14px 38px color-mix(in oklch, var(--primary) 14%, transparent);
}

@keyframes fl-chat-solarpunk-bloom {
  from { background-position: 0% 0%, 100% 20%, 50% 100%; }
  to { background-position: 24% 18%, 72% 0%, 42% 72%; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface] { animation: none; }
}
		`.trim(),
	},
	{
		value: "noir-cinema",
		label: "Noir Cinema",
		description:
			"High-contrast monochrome, crimson focus, and subtle film grain.",
		badge: "Cinematic",
		preview: {
			background: "radial-gradient(circle at 25% 10%, #666, #111 58%)",
			accent: "#b91c35",
		},
		css: `
/* Noir Cinema — app-aware monochrome with moving film grain */
:root {
  --fl-chat-noir-font: Didot, "Bodoni MT", Georgia, ui-serif, serif;
  --font-sans: var(--fl-chat-noir-font);
  --primary: oklch(0.52 0.19 20);
  --primary-foreground: white;
  --ring: var(--primary);
  --border: color-mix(in oklch, var(--foreground) 34%, var(--background));
  --radius: 0.125rem;
  --fl-chat-content-width: 54rem;
  --fl-chat-message-radius: 0.125rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 95%, var(--foreground));
  --fl-chat-composer-background: var(--background);
  --fl-chat-user-message-background: var(--foreground);
  --fl-chat-user-message-foreground: var(--background);
  --fl-chat-ai-message-background: color-mix(in oklch, var(--foreground) 4%, var(--background));
  --fl-chat-disclosure-background: transparent;
  font-family: var(--fl-chat-noir-font);
}

[data-fl-chat-surface] {
  position: relative;
  isolation: isolate;
  background-image: radial-gradient(ellipse at 18% -10%, color-mix(in oklch, var(--foreground) 18%, transparent), transparent 52%);
}

[data-fl-chat-surface]::before {
  content: "";
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  opacity: 0.14;
  background-image: radial-gradient(circle, var(--foreground) 0 0.6px, transparent 0.8px);
  background-size: 7px 7px;
  animation: fl-chat-noir-grain 0.8s steps(2, end) infinite;
}

[data-fl-chat-surface] > * {
  position: relative;
  z-index: 1;
}

[data-fl-chat-message] {
  border-block: 1px solid var(--border);
  box-shadow: 8px 8px 0 color-mix(in oklch, var(--foreground) 8%, transparent);
}

[data-fl-chat-composer] {
  border-left: 4px solid var(--primary);
  border-radius: 0;
}

@keyframes fl-chat-noir-grain {
  0% { background-position: 0 0; }
  50% { background-position: 3px -2px; }
  100% { background-position: -2px 3px; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface]::before { animation: none; }
}
		`.trim(),
	},
	{
		value: "pixel-arcade",
		label: "Pixel Arcade",
		description:
			"Chunky pixel borders, electric cyan, and a scrolling game grid.",
		badge: "Animated",
		preview: {
			background:
				"repeating-linear-gradient(90deg, #17102f 0 8px, #241749 8px 16px)",
			accent: "#22d3ee",
		},
		css: `
/* Pixel Arcade — bright 8-bit structure without forcing dark mode */
:root {
  --fl-chat-pixel-font: "Courier New", ui-monospace, monospace;
  --font-sans: var(--fl-chat-pixel-font);
  --font-mono: var(--fl-chat-pixel-font);
  --primary: oklch(0.79 0.15 205);
  --primary-foreground: oklch(0.15 0.04 240);
  --accent: oklch(0.68 0.24 335);
  --ring: var(--accent);
  --border: color-mix(in oklch, var(--foreground) 32%, var(--background));
  --radius: 0;
  --fl-chat-content-width: 60rem;
  --fl-chat-message-radius: 0;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 96%, var(--primary));
  --fl-chat-composer-background: var(--background);
  --fl-chat-user-message-background: var(--primary);
  --fl-chat-user-message-foreground: var(--primary-foreground);
  --fl-chat-ai-message-background: color-mix(in oklch, var(--accent) 7%, var(--background));
  --fl-chat-disclosure-background: color-mix(in oklch, var(--primary) 8%, var(--background));
  font-family: var(--fl-chat-pixel-font);
}

[data-fl-chat-surface] {
  background-image:
    linear-gradient(color-mix(in oklch, var(--foreground) 7%, transparent) 1px, transparent 1px),
    linear-gradient(90deg, color-mix(in oklch, var(--foreground) 7%, transparent) 1px, transparent 1px);
  background-size: 16px 16px;
  animation: fl-chat-pixel-scroll 12s steps(16, end) infinite;
}

[data-fl-chat-message] {
  border: 2px solid var(--border);
  box-shadow: 4px 4px 0 color-mix(in oklch, var(--foreground) 22%, transparent);
}

[data-fl-chat-welcome] h1 {
  letter-spacing: 0.08em;
  text-shadow: 3px 3px 0 color-mix(in oklch, var(--accent) 52%, transparent);
}

[data-fl-chat-composer] {
  border: 2px solid var(--primary);
  border-radius: 0;
  box-shadow: 5px 5px 0 color-mix(in oklch, var(--accent) 38%, transparent);
}

[data-fl-chat-suggestion] {
  border-radius: 0;
}

@keyframes fl-chat-pixel-scroll {
  to { background-position: 16px 16px, 16px 16px; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface] { animation: none; }
}
		`.trim(),
	},
	{
		value: "cyberpunk",
		label: "Cyberpunk",
		description:
			"Glitch scanlines, electric cyan, and a chromatic chase border.",
		badge: "Animated",
		preview: {
			background: "linear-gradient(135deg, #080b18, #1b1233 52%, #3b082a)",
			accent: "#22f4ea",
		},
		css: `
/* Cyberpunk — chromatic scanlines and electric edges */
:root {
  --fl-chat-cyber-font: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  --font-sans: var(--fl-chat-cyber-font);
  --font-mono: var(--fl-chat-cyber-font);
  --primary: oklch(0.84 0.17 190);
  --primary-foreground: oklch(0.16 0.03 230);
  --accent: oklch(0.68 0.27 345);
  --ring: var(--primary);
  --fl-chat-message-radius: 0.25rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 94%, var(--accent));
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 88%, transparent);
  --fl-chat-user-message-background: color-mix(in oklch, var(--accent) 18%, var(--background));
  --fl-chat-ai-message-background: color-mix(in oklch, var(--primary) 5%, transparent);
  font-family: var(--fl-chat-cyber-font);
}

[data-fl-chat-surface] {
  background-image:
    repeating-linear-gradient(0deg, transparent 0 5px, color-mix(in oklch, var(--primary) 5%, transparent) 5px 6px),
    linear-gradient(115deg, transparent 35%, color-mix(in oklch, var(--accent) 10%, transparent) 48%, transparent 62%),
    radial-gradient(circle at 80% 10%, color-mix(in oklch, var(--primary) 18%, transparent), transparent 42%);
  background-size: 100% 6px, 220% 100%, 100% 100%;
  animation: fl-chat-preset-cyberpunk-field 10s linear infinite;
}

[data-fl-chat-message] {
  border: 1px solid color-mix(in oklch, var(--primary) 25%, transparent);
}

[data-fl-chat-message="assistant"] {
  border-left: 3px solid var(--primary);
}

[data-fl-chat-message="user"] {
  border-right: 3px solid var(--accent);
}

[data-fl-chat-composer] {
  position: relative;
}

[data-fl-chat-composer]::before {
  content: "";
  position: absolute;
  inset: -2px;
  border-radius: 0.875rem;
  padding: 1px;
  pointer-events: none;
  background: linear-gradient(90deg, var(--primary), var(--accent), oklch(0.86 0.18 95), var(--primary));
  background-size: 240% 100%;
  animation: fl-chat-preset-cyberpunk-border 2.4s linear infinite;
  -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
  -webkit-mask-composite: xor;
  mask-composite: exclude;
}

@keyframes fl-chat-preset-cyberpunk-field {
  to { background-position: 0 6px, -220% 0, 0 0; }
}

@keyframes fl-chat-preset-cyberpunk-border {
  to { background-position: -240% 0; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface],
  [data-fl-chat-composer]::before {
    animation: none;
  }
}
		`.trim(),
	},
	{
		value: "synthwave",
		label: "Synthwave Sunset",
		description: "A radiant horizon with a cruising retro-future grid.",
		badge: "Animated",
		preview: {
			background: "linear-gradient(180deg, #251153, #e3348c 55%, #10072c)",
			accent: "#ff69d4",
		},
		css: `
/* Synthwave Sunset — radiant horizon and moving grid */
:root {
  --primary: oklch(0.7 0.25 340);
  --primary-foreground: white;
  --accent: oklch(0.82 0.17 90);
  --ring: var(--primary);
  --fl-chat-content-width: 68rem;
  --fl-chat-message-radius: 0.75rem 0.25rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 92%, var(--primary));
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 84%, transparent);
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 20%, var(--background));
  --fl-chat-ai-message-background: color-mix(in oklch, var(--accent) 4%, transparent);
}

[data-fl-chat-surface] {
  background-image:
    linear-gradient(180deg, transparent 0 49%, color-mix(in oklch, var(--primary) 45%, transparent) 49% 50%, transparent 50%),
    linear-gradient(color-mix(in oklch, var(--primary) 10%, transparent) 1px, transparent 1px),
    linear-gradient(90deg, color-mix(in oklch, var(--primary) 10%, transparent) 1px, transparent 1px),
    radial-gradient(circle at 50% 38%, color-mix(in oklch, var(--accent) 36%, transparent) 0 8%, color-mix(in oklch, var(--primary) 18%, transparent) 9% 19%, transparent 20%),
    radial-gradient(ellipse at 50% 100%, color-mix(in oklch, var(--primary) 20%, transparent), transparent 58%);
  background-size: 100% 100%, 100% 28px, 52px 100%, 100% 100%, 100% 100%;
  animation: fl-chat-preset-synthwave-grid 12s linear infinite;
}

[data-fl-chat-message] {
  border-bottom: 1px solid color-mix(in oklch, var(--primary) 35%, transparent);
  box-shadow: 0 8px 28px color-mix(in oklch, var(--primary) 9%, transparent);
}

[data-fl-chat-composer] {
  border-radius: 1rem;
  outline: 1px solid color-mix(in oklch, var(--primary) 45%, transparent);
  box-shadow: 0 0 28px color-mix(in oklch, var(--primary) 16%, transparent);
  animation: fl-chat-preset-synthwave-glow 4s ease-in-out infinite alternate;
}

@keyframes fl-chat-preset-synthwave-grid {
  to { background-position: 0 0, 0 28px, 52px 0, 0 0, 0 0; }
}

@keyframes fl-chat-preset-synthwave-glow {
  to { box-shadow: 0 0 42px color-mix(in oklch, var(--accent) 20%, transparent); }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface],
  [data-fl-chat-composer] {
    animation: none;
  }
}
		`.trim(),
	},
	{
		value: "oceanic",
		label: "Oceanic",
		description: "Bioluminescent currents and slowly rising bubbles.",
		badge: "Animated",
		preview: {
			background: "linear-gradient(145deg, #052d3b, #087b83 56%, #13b8a6)",
			accent: "#5eead4",
		},
		css: `
/* Oceanic — bioluminescent currents and rising bubbles */
:root {
  --primary: oklch(0.76 0.14 185);
  --primary-foreground: oklch(0.18 0.04 205);
  --accent: oklch(0.72 0.15 225);
  --ring: var(--primary);
  --fl-chat-message-radius: 1.4rem 1.4rem 0.35rem 1.4rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 91%, var(--accent));
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 80%, transparent);
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 19%, var(--background));
  --fl-chat-ai-message-background: color-mix(in oklch, var(--accent) 7%, transparent);
  --fl-chat-disclosure-background: color-mix(in oklch, var(--primary) 10%, var(--background));
}

[data-fl-chat-surface] {
  background-image:
    radial-gradient(circle at 20% 30%, color-mix(in oklch, var(--primary) 24%, transparent) 0 2px, transparent 3px),
    radial-gradient(circle at 75% 70%, color-mix(in oklch, var(--accent) 18%, transparent) 0 3px, transparent 4px),
    radial-gradient(ellipse at 30% 110%, color-mix(in oklch, var(--primary) 24%, transparent), transparent 52%),
    radial-gradient(ellipse at 90% -10%, color-mix(in oklch, var(--accent) 18%, transparent), transparent 48%);
  background-size: 90px 130px, 150px 210px, 100% 100%, 100% 100%;
  animation: fl-chat-preset-oceanic-current 18s linear infinite;
}

[data-fl-chat-message] {
  border: 1px solid color-mix(in oklch, var(--primary) 18%, transparent);
  box-shadow: inset 0 1px 0 color-mix(in oklch, var(--primary) 13%, transparent);
}

[data-fl-chat-composer] {
  border-radius: 1.25rem;
  box-shadow: 0 14px 42px color-mix(in oklch, var(--accent) 14%, transparent);
  animation: fl-chat-preset-oceanic-breathe 5s ease-in-out infinite alternate;
}

@keyframes fl-chat-preset-oceanic-current {
  to { background-position: 0 -130px, 30px -150px, 3% 0, -3% 0; }
}

@keyframes fl-chat-preset-oceanic-breathe {
  to { box-shadow: 0 18px 52px color-mix(in oklch, var(--primary) 20%, transparent); }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface],
  [data-fl-chat-composer] {
    animation: none;
  }
}
		`.trim(),
	},
	{
		value: "candy-pop",
		label: "Candy Pop",
		description:
			"Playful sherbet stripes, pillowy cards, and a rainbow border.",
		badge: "Playful",
		preview: {
			background: "linear-gradient(135deg, #ff8fc7, #ffe784 50%, #75e6d4)",
			accent: "#f43f9a",
		},
		css: `
/* Candy Pop — animated sherbet stripes and pillowy cards */
:root {
  --primary: oklch(0.67 0.22 350);
  --primary-foreground: white;
  --accent: oklch(0.84 0.14 175);
  --ring: var(--primary);
  --fl-chat-message-radius: 1.5rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 92%, var(--primary));
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 88%, transparent);
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 18%, var(--background));
  --fl-chat-ai-message-background: color-mix(in oklch, var(--accent) 10%, var(--background));
}

[data-fl-chat-surface] {
  background-image:
    repeating-linear-gradient(
      135deg,
      color-mix(in oklch, var(--primary) 8%, transparent) 0 18px,
      color-mix(in oklch, var(--accent) 8%, transparent) 18px 36px,
      color-mix(in oklch, oklch(0.88 0.16 95) 10%, transparent) 36px 54px
    ),
    radial-gradient(circle at 15% 15%, color-mix(in oklch, var(--primary) 16%, transparent), transparent 30%);
  background-size: 152px 152px, 100% 100%;
  animation: fl-chat-preset-candy-stripes 16s linear infinite;
}

[data-fl-chat-message] {
  border: 2px solid color-mix(in oklch, var(--foreground) 8%, transparent);
  box-shadow: 0 8px 0 color-mix(in oklch, var(--primary) 9%, transparent);
}

[data-fl-chat-composer] {
  position: relative;
  border-radius: 1.5rem;
}

[data-fl-chat-composer]::before {
  content: "";
  position: absolute;
  inset: -3px;
  border-radius: inherit;
  padding: 2px;
  pointer-events: none;
  background: linear-gradient(90deg, var(--primary), oklch(0.88 0.16 95), var(--accent), var(--primary));
  background-size: 240% 100%;
  animation: fl-chat-preset-candy-border 6s linear infinite;
  -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
  -webkit-mask-composite: xor;
  mask-composite: exclude;
}

@keyframes fl-chat-preset-candy-stripes {
  to { background-position: 152px 152px, 0 0; }
}

@keyframes fl-chat-preset-candy-border {
  to { background-position: -240% 0; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface],
  [data-fl-chat-composer]::before {
    animation: none;
  }
}
		`.trim(),
	},
	{
		value: "zen-garden",
		label: "Zen Garden",
		description:
			"Quiet ink-wash gradients, raked sand rings, and gentle motion.",
		badge: "Calm",
		preview: {
			background: "linear-gradient(135deg, #ece8dc, #b8c5ad 58%, #6f8067)",
			accent: "#71866a",
		},
		css: `
/* Zen Garden — ink wash, raked rings, and restrained motion */
:root {
  --fl-chat-zen-font: ui-serif, Georgia, Cambria, "Times New Roman", serif;
  --font-sans: var(--fl-chat-zen-font);
  --primary: oklch(0.53 0.08 145);
  --primary-foreground: white;
  --ring: color-mix(in oklch, var(--primary) 65%, var(--background));
  --fl-chat-content-width: 54rem;
  --fl-chat-message-radius: 0.25rem 1.25rem;
  --fl-chat-surface-background: color-mix(in oklch, var(--background) 96%, var(--primary));
  --fl-chat-composer-background: color-mix(in oklch, var(--background) 94%, transparent);
  --fl-chat-user-message-background: color-mix(in oklch, var(--primary) 11%, var(--background));
  --fl-chat-ai-message-background: transparent;
  --fl-chat-disclosure-background: color-mix(in oklch, var(--primary) 6%, var(--background));
  font-family: var(--fl-chat-zen-font);
}

[data-fl-chat-surface] {
  background-image:
    repeating-radial-gradient(circle at 78% 82%, transparent 0 22px, color-mix(in oklch, var(--foreground) 5%, transparent) 23px 24px, transparent 25px 46px),
    radial-gradient(ellipse at 15% 15%, color-mix(in oklch, var(--primary) 13%, transparent), transparent 46%),
    linear-gradient(115deg, transparent 35%, color-mix(in oklch, var(--foreground) 3%, transparent), transparent 65%);
  background-size: 120% 120%, 110% 110%, 180% 100%;
  animation: fl-chat-preset-zen-drift 24s ease-in-out infinite alternate;
}

[data-fl-chat-message="assistant"] {
  border-left: 1px solid color-mix(in oklch, var(--primary) 42%, transparent);
  line-height: 1.75;
}

[data-fl-chat-message="user"] {
  border-bottom: 2px solid color-mix(in oklch, var(--primary) 34%, transparent);
}

[data-fl-chat-composer] {
  border-radius: 0.25rem 1.25rem;
  border-bottom: 2px solid color-mix(in oklch, var(--primary) 42%, transparent);
  box-shadow: 0 12px 32px color-mix(in oklch, var(--foreground) 6%, transparent);
}

@keyframes fl-chat-preset-zen-drift {
  from { background-position: 0 0, 0 0, 0 0; }
  to { background-position: 3rem 2rem, -2rem 1rem, -80% 0; }
}

@media (prefers-reduced-motion: reduce) {
  [data-fl-chat-surface] { animation: none; }
}
		`.trim(),
	},
] as const satisfies ReadonlyArray<ChatThemePreset>;

export type ChatThemePresetId = (typeof CHAT_THEME_PRESETS)[number]["value"];
export type ChatThemeSelection =
	| ChatThemePresetId
	| typeof CUSTOM_CHAT_THEME_VALUE;

export const DEFAULT_CHAT_THEME_CSS = CHAT_THEME_PRESETS[0].css;

export function resolveChatThemePreset(customCss: unknown): ChatThemeSelection {
	if (typeof customCss !== "string") return CUSTOM_CHAT_THEME_VALUE;
	return (
		CHAT_THEME_PRESETS.find((preset) => preset.css === customCss)?.value ??
		CUSTOM_CHAT_THEME_VALUE
	);
}
