import { IThemes } from "../../../types";

export function themeSelection(theme?: { id?: string } | null): string {
	const id = theme?.id;
	return !id
		? IThemes.FLOW_LIKE
		: Object.values(IThemes).includes(id as IThemes)
			? id
			: "CUSTOM";
}

export function parseProfileTheme(input: string, name: string) {
	const parseBlock = (selector: string) => {
		const block =
			input.match(new RegExp(`${selector}\\s*\\{([\\s\\S]*?)\\}`, "m"))?.[1] ??
			"";
		return Object.fromEntries(
			[...block.matchAll(/--([a-z0-9-]+)\s*:\s*([^;]+);/gi)].map((match) => [
				match[1].replace(/-([a-z0-9])/gi, (_, letter) => letter.toUpperCase()),
				match[2].trim(),
			]),
		);
	};
	const light = parseBlock(":root");
	const dark = parseBlock("\\.dark");
	if (!light.background || !dark.background)
		throw new Error(
			"Include a background variable in both :root and .dark blocks.",
		);
	const id = name.trim();
	if (!id || id === "CUSTOM" || Object.values(IThemes).includes(id as IThemes))
		throw new Error(
			"Give your custom theme a name different from a built-in theme.",
		);
	return { id, light, dark };
}

export function profileThemeCss(
	theme?: {
		light?: Record<string, string>;
		dark?: Record<string, string>;
	} | null,
) {
	const block = (selector: string, values: Record<string, string> = {}) =>
		`${selector} {\n${Object.entries(values)
			.map(
				([key, value]) =>
					`  --${key.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}: ${value};`,
			)
			.join("\n")}\n}`;
	return theme
		? `${block(":root", theme.light)}\n\n${block(".dark", theme.dark)}`
		: "";
}
