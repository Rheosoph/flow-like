// Mirrors packages/wasm/src/widget_bundle.rs (serde camelCase) exactly.

export const BUNDLE_FORMAT_VERSION = 1;
export const BUNDLE_MANIFEST_PATH = "bundle.json";
export const WIDGET_BUNDLE_EXTENSION = "flwb";
export const WIDGET_BUNDLE_MEDIA_TYPE =
	"application/vnd.flow-like.widget-bundle";

export interface BundleSharedEntry {
	path: string;
	/** `sha256:<hex>` of the entry bytes */
	hash: string;
}

export interface BundleSizeHint {
	raw: number;
	gzip?: number;
}

export interface BundleWidgetEntry {
	id: string;
	name: string;
	description: string;
	/** Archive path of the widget document, e.g. `widgets/{id}/index.html` */
	entry: string;
	/** Archive path of the widget contract, e.g. `widgets/{id}/contract.json` */
	contract: string;
	/** `sha256:<hex>` of the entry document bytes */
	entryHash: string;
	/** Shared chunk paths this widget references */
	assets: string[];
	framework?: string;
	sizeHint?: BundleSizeHint;
}

export interface WidgetBundleManifest {
	formatVersion: number;
	packageId: string;
	packageVersion: string;
	/** Host<->widget postMessage protocol version, e.g. `flw/1` */
	protocol: string;
	createdAt?: string;
	shared: BundleSharedEntry[];
	widgets: BundleWidgetEntry[];
}

export function isSafeEntryPath(path: string): boolean {
	return (
		path.length > 0 &&
		!path.startsWith("/") &&
		!path.includes("\\") &&
		path.split("/").every((seg) => seg !== "" && seg !== "." && seg !== "..")
	);
}

/** Rebuild with serde field order and skip-serializing semantics. */
export function canonicalizeManifest(
	manifest: WidgetBundleManifest,
): WidgetBundleManifest {
	return {
		formatVersion: manifest.formatVersion,
		packageId: manifest.packageId,
		packageVersion: manifest.packageVersion,
		protocol: manifest.protocol,
		...(manifest.createdAt !== undefined && { createdAt: manifest.createdAt }),
		shared: manifest.shared.map((s) => ({ path: s.path, hash: s.hash })),
		widgets: manifest.widgets.map((w) => ({
			id: w.id,
			name: w.name,
			description: w.description,
			entry: w.entry,
			contract: w.contract,
			entryHash: w.entryHash,
			assets: [...w.assets],
			...(w.framework !== undefined && { framework: w.framework }),
			...(w.sizeHint !== undefined && {
				sizeHint: {
					raw: w.sizeHint.raw,
					...(w.sizeHint.gzip !== undefined && { gzip: w.sizeHint.gzip }),
				},
			}),
		})),
	};
}

export function manifestToJson(manifest: WidgetBundleManifest): string {
	return JSON.stringify(canonicalizeManifest(manifest), null, 2);
}
