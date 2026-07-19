import FLOW_LIKE_THEME from "./theme/flow-like-theme.json";

export interface IThemeColors {
	background?: string;
	foreground?: string;
	card?: string;
	cardForeground?: string;
	popover?: string;
	popoverForeground?: string;
	primary?: string;
	primaryForeground?: string;
	secondary?: string;
	secondaryForeground?: string;
	tertiary?: string;
	tertiaryForeground?: string;
	muted?: string;
	mutedForeground?: string;
	accent?: string;
	accentForeground?: string;
	destructive?: string;
	destructiveForeground?: string;
	border?: string;
	input?: string;
	ring?: string;
	chart1?: string;
	chart2?: string;
	chart3?: string;
	chart4?: string;
	chart5?: string;
	sidebar?: string;
	sidebarForeground?: string;
	sidebarPrimary?: string;
	sidebarPrimaryForeground?: string;
	sidebarAccent?: string;
	sidebarAccentForeground?: string;
	sidebarBorder?: string;
	sidebarRing?: string;
	fontSans?: string;
	fontSerif?: string;
	fontMono?: string;
	radius?: string;
	shadowX?: string;
	shadowY?: string;
	shadowBlur?: string;
	shadowSpread?: string;
	shadowOpacity?: string;
	shadowColor?: string;
	shadow?: string;
	shadow2xs?: string;
	shadowXs?: string;
	shadowSm?: string;
	shadowMd?: string;
	shadowLg?: string;
	shadowXl?: string;
	shadow2xl?: string;
	trackingNormal?: string;
	spacing?: string;
}

export interface ITheme {
	id?: string;
	light: IThemeColors;
	dark: IThemeColors;
}

export function loadTheme(theme: ITheme) {
	// Check if we're in a browser environment
	if (typeof document === "undefined") {
		console.warn("loadTheme: Document is not available (SSR environment)");
		return;
	}

	const defaultTheme: ITheme = FLOW_LIKE_THEME as ITheme;

	// Ensure default theme has required structure
	if (!defaultTheme?.light || !defaultTheme?.dark) {
		console.error(
			"loadTheme: Default theme is missing light or dark properties",
		);
		return;
	}

	const lightTheme: IThemeColors = {
		...defaultTheme.light,
		...theme.light,
	};
	const darkTheme: IThemeColors = {
		...defaultTheme.dark,
		...theme.dark,
	};

	console.log("Loading theme:", {
		light: lightTheme,
		dark: darkTheme,
	});

	// Remove existing theme style if it exists
	const existingStyle = document.getElementById("dynamic-theme");
	if (existingStyle) {
		existingStyle.remove();
	}

	// Convert camelCase to kebab-case for CSS variables
	const toKebabCase = (str: string) =>
		str.replace(/([A-Z])/g, "-$1").toLowerCase();

	// Generate CSS variables for light theme
	const lightVars = Object.entries(lightTheme)
		.filter(
			([_, value]) => value !== undefined && value !== null && value !== "",
		)
		.map(([key, value]) => `    --${toKebabCase(key)}: ${value};`)
		.join("\n");

	// Generate CSS variables for dark theme
	const darkVars = Object.entries(darkTheme)
		.filter(
			([_, value]) => value !== undefined && value !== null && value !== "",
		)
		.map(([key, value]) => `    --${toKebabCase(key)}: ${value};`)
		.join("\n");

	// Create the CSS (removed @layer for better compatibility)
	const css = `
:root {
${lightVars}
}

.dark,
:root[data-theme="dark"] {
${darkVars}
}

[data-fl-chat-color-scheme="light"] {
${lightVars}
}

[data-fl-chat-color-scheme="dark"] {
${darkVars}
}

/* Apply theme fonts to body and font classes */
body {
    font-family: var(--font-sans, inherit) !important;
}

.font-sans {
    font-family: var(--font-sans, inherit) !important;
}

.font-serif {
    font-family: var(--font-serif, inherit) !important;
}

.font-mono {
    font-family: var(--font-mono, inherit) !important;
}`;

	// Inject the CSS
	const styleElement = document.createElement("style");
	styleElement.id = "dynamic-theme";
	styleElement.textContent = css;
	document.head.appendChild(styleElement);
}
