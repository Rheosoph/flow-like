import {
	BanIcon,
	CheckIcon,
	ChevronDownIcon,
	ChevronUpIcon,
	CircleXIcon,
	ClockIcon,
	EllipsisVerticalIcon,
	GripIcon,
	InfoIcon,
	ListIcon,
	type LucideIcon,
	ScrollTextIcon,
	SquareCheckIcon,
	TriangleAlertIcon,
	VariableIcon,
	WorkflowIcon,
} from "lucide-react";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

export interface IconRaster {
	width: number;
	height: number;
	rgba: Uint8Array;
	/** Monochrome icons stroke with currentColor; the painter tints their white mask. */
	tint: boolean;
}

/**
 * Node chrome glyphs, keyed as the painter expects. Stroke widths are the production
 * components' values in viewBox units, so the raster size never changes their weight.
 */
export const NODE_GLYPHS: Readonly<
	Record<string, { icon: LucideIcon; strokeWidth: number }>
> = {
	"glyph:workflow": { icon: WorkflowIcon, strokeWidth: 2 },
	"glyph:variable": { icon: VariableIcon, strokeWidth: 2 },
	"glyph:check": { icon: CheckIcon, strokeWidth: 2 },
	"glyph:chevron-down": { icon: ChevronDownIcon, strokeWidth: 2 },
	"glyph:chevron-up": { icon: ChevronUpIcon, strokeWidth: 3 },
	"glyph:array": { icon: GripIcon, strokeWidth: 3 },
	"glyph:set": { icon: EllipsisVerticalIcon, strokeWidth: 3 },
	"glyph:map": { icon: ListIcon, strokeWidth: 3 },
	"glyph:scroll": { icon: ScrollTextIcon, strokeWidth: 2 },
	"glyph:square-check": { icon: SquareCheckIcon, strokeWidth: 2 },
	"glyph:clock": { icon: ClockIcon, strokeWidth: 2 },
	"glyph:triangle-alert": { icon: TriangleAlertIcon, strokeWidth: 2 },
	"glyph:circle-x": { icon: CircleXIcon, strokeWidth: 2 },
	"glyph:ban": { icon: BanIcon, strokeWidth: 2 },
	"glyph:info": { icon: InfoIcon, strokeWidth: 2 },
};

const rasters = new Map<string, Promise<IconRaster>>();

function cached(key: string, load: () => Promise<IconRaster>) {
	let raster = rasters.get(key);
	if (!raster) {
		raster = load().catch((error: unknown) => {
			rasters.delete(key);
			throw error;
		});
		rasters.set(key, raster);
	}
	return raster;
}

export function rasterizeIcon(url: string, size: number): Promise<IconRaster> {
	return cached(`${size}:${url}`, async () => {
		const response = await fetch(url);
		if (!response.ok)
			throw new Error(`Cannot load node icon ${url}: HTTP ${response.status}`);
		const svg = await response.text();
		return rasterizeSvg(
			svg.replace(/currentColor/gi, "#fff"),
			size,
			/currentColor/i.test(svg),
			url,
		);
	});
}

export function rasterizeGlyph(key: string, size: number): Promise<IconRaster> {
	const glyph = NODE_GLYPHS[key];
	if (!glyph) return Promise.reject(new Error(`Unknown node glyph ${key}`));
	return cached(`${size}:${key}`, () =>
		rasterizeSvg(
			renderToStaticMarkup(
				createElement(glyph.icon, {
					size,
					color: "#fff",
					strokeWidth: glyph.strokeWidth,
				}),
			),
			size,
			true,
			key,
		),
	);
}

async function rasterizeSvg(
	svg: string,
	size: number,
	tint: boolean,
	name: string,
): Promise<IconRaster> {
	const source = URL.createObjectURL(
		new Blob([svg], { type: "image/svg+xml" }),
	);
	try {
		const image = new Image();
		image.src = source;
		await image.decode();
		const canvas = document.createElement("canvas");
		canvas.width = size;
		canvas.height = size;
		const context = canvas.getContext("2d");
		if (!context)
			throw new Error(`Cannot rasterize ${name}: no 2D canvas context`);
		context.drawImage(image, 0, 0, size, size);
		return {
			width: size,
			height: size,
			rgba: new Uint8Array(context.getImageData(0, 0, size, size).data.buffer),
			tint,
		};
	} finally {
		URL.revokeObjectURL(source);
	}
}
