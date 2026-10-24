/** Header icon box in graph units, matching the painter's icon slot. */
export const NODE_ICON_SIZE = 8;

/** One mipmapped raster stays sharp up to the maximum zoom on this display. */
export const iconRasterSize = (maxZoom: number, pixelRatio: number) =>
	Math.ceil(NODE_ICON_SIZE * maxZoom * Math.max(1, pixelRatio));

/** Catalog icons are same-origin Lucide SVGs; other sources keep the browser renderer. */
export const isNativeIcon = (icon: string) =>
	/^\/flow\/icons\/[\w.-]+\.svg$/.test(icon);
