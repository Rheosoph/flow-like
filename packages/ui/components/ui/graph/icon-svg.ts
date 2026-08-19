import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { GRAPH_ICONS } from "./icons";

const cache = new Map<string, string>();

/**
 * Intrinsic size of the rasterised glyph, in pixels.
 *
 * Sigma's image program draws an SVG into its texture atlas at the size the
 * markup declares and caps it at 512 — it never re-rasterises per zoom level. At
 * the icon's own display size the atlas entry is then upscaled onto a node that
 * is larger still on a retina panel, which is what made the glyphs look soft.
 * Rendering well above any node diameter costs one atlas cell and nothing per
 * frame; the stroke is expressed in viewBox units, so weight is unchanged.
 */
const ICON_RENDER_SIZE = 128;

export function getIconDataUri(iconKey: string): string {
	if (cache.has(iconKey)) return cache.get(iconKey)!;

	const Icon = GRAPH_ICONS[iconKey] ?? GRAPH_ICONS.database;
	if (!Icon) return "";

	const svg = renderToStaticMarkup(
		createElement(Icon, {
			size: ICON_RENDER_SIZE,
			color: "white",
			strokeWidth: 2.5,
		}),
	);

	const uri = `data:image/svg+xml,${encodeURIComponent(svg)}`;
	cache.set(iconKey, uri);
	return uri;
}
