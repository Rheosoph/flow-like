import fs from "fs";
import type { ITheme, IThemeColors } from "@flow-like/flow-like-ui";

const css = fs.readFileSync("./theme-input.css", "utf-8");

const parseCSSVariables = (cssContent: string): ITheme => {
	const lightColors: IThemeColors = {};
	const darkColors: IThemeColors = {};

	const rootMatch = cssContent.match(/:root\s*{([^}]*)}/);
	if (rootMatch) {
		parseVariables(rootMatch[1], lightColors);
	}

	const darkMatch = cssContent.match(/\.dark\s*{([^}]*)}/);
	if (darkMatch) {
		parseVariables(darkMatch[1], darkColors);
	}

	// Add tertiary colors to both themes
	lightColors.tertiary = "oklch(75.285% 0.15368 64.1)";
	lightColors.tertiaryForeground = "oklch(100% 0.00011 271.152)";
	darkColors.tertiary = "oklch(75.285% 0.15368 64.1)";
	darkColors.tertiaryForeground = "oklch(100% 0.00011 271.152)";

	return {
		id: "flow-like",
		light: lightColors,
		dark: darkColors,
	};
};

const parseVariables = (cssVars: string, colors: IThemeColors): void => {
	const variables = cssVars.split(";").filter((line) => line.trim());

	for (const variable of variables) {
		const match = variable.match(/--([a-z0-9-]+):\s*([^;]+)/);
		if (match) {
			const [, cssVar, value] = match;
			const camelCaseKey = toCamelCase(cssVar);

			if (isValidThemeProperty(camelCaseKey)) {
				(colors as any)[camelCaseKey] = value.trim().replace(/\s+/g, " ");
			}
		}
	}
};

const toCamelCase = (str: string): string => {
	return str.replace(/-([a-z0-9])/g, (_, letter) => letter.toUpperCase());
};

const isValidThemeProperty = (key: string): boolean => {
	const validKeys: (keyof IThemeColors)[] = [
		"background",
		"foreground",
		"card",
		"cardForeground",
		"popover",
		"popoverForeground",
		"primary",
		"primaryForeground",
		"secondary",
		"secondaryForeground",
		"tertiary",
		"tertiaryForeground",
		"muted",
		"mutedForeground",
		"accent",
		"accentForeground",
		"destructive",
		"destructiveForeground",
		"border",
		"input",
		"ring",
		"chart1",
		"chart2",
		"chart3",
		"chart4",
		"chart5",
		"sidebar",
		"sidebarForeground",
		"sidebarPrimary",
		"sidebarPrimaryForeground",
		"sidebarAccent",
		"sidebarAccentForeground",
		"sidebarBorder",
		"sidebarRing",
		"fontSans",
		"fontSerif",
		"fontMono",
		"radius",
		"shadowX",
		"shadowY",
		"shadowBlur",
		"shadowSpread",
		"shadowOpacity",
		"shadowColor",
		"shadow",
		"shadow2xs",
		"shadowXs",
		"shadowSm",
		"shadowMd",
		"shadowLg",
		"shadowXl",
		"shadow2xl",
		"trackingNormal",
		"spacing",
	];

	return validKeys.includes(key as keyof IThemeColors);
};

const theme: ITheme = parseCSSVariables(css);

fs.writeFileSync(
	"./generated-theme.json",
	`${JSON.stringify(theme, null, "\t")}\n`,
);

console.log("Theme generated successfully!");
console.log(JSON.stringify(theme, null, "\t"));
