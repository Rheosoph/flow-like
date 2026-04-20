import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { GRAPH_ICONS } from "./icons";

const cache = new Map<string, string>();

export function getIconDataUri(iconKey: string): string {
	if (cache.has(iconKey)) return cache.get(iconKey)!;

	const Icon = GRAPH_ICONS[iconKey] ?? GRAPH_ICONS.database;
	if (!Icon) return "";

	const svg = renderToStaticMarkup(
		createElement(Icon, { size: 20, color: "white", strokeWidth: 2.5 }),
	);

	const uri = `data:image/svg+xml,${encodeURIComponent(svg)}`;
	cache.set(iconKey, uri);
	return uri;
}
