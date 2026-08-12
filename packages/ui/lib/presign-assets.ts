import type { BoundValue, SurfaceComponent } from "../components/a2ui/types";
import type { IStorageState } from "../state/backend-state/storage-state";

// Component/property pairs that contain storage-backed asset URLs.
// Keep this aligned with the builder inspector's asset picker rules.
const ASSET_FIELDS_BY_COMPONENT: Record<string, string[]> = {
	image: ["src", "fallback"],
	video: ["src", "poster"],
	filePreview: ["src", "url"],
	avatar: ["src"],
	lottie: ["src"],
	iframe: ["src"],
	boundingBoxOverlay: ["src"],
	imageLabeler: ["src"],
	imageHotspot: ["src"],
	sprite: ["src"],
	model3d: ["src", "hdriUrl"],
	scene3d: ["environmentMap"],
	characterPortrait: ["image"],
	miniMap: ["mapImage"],
};

export function isStoragePrefix(value: string): boolean {
	// Storage prefixes are paths without http(s) scheme
	if (!value) return false;
	if (value.startsWith("http://") || value.startsWith("https://")) return false;
	if (value.startsWith("data:")) return false;
	if (value.startsWith("blob:")) return false;
	if (value.startsWith("asset://")) return false;
	return true;
}

function extractStringFromBoundValue(
	value: BoundValue | string | undefined,
): string | undefined {
	if (!value) return undefined;
	// Handle plain strings (not wrapped in BoundValue)
	if (typeof value === "string") return value;
	if (typeof value === "object" && "literalString" in value)
		return value.literalString;
	return undefined;
}

function updateBoundValueString(
	value: BoundValue | string,
	newString: string,
): BoundValue | string {
	// If it was a plain string, return a plain string
	if (typeof value === "string") return newString;
	if (typeof value === "object" && "literalString" in value) {
		return { literalString: newString };
	}
	return value;
}

export interface AssetPrefixInfo {
	componentId: string;
	property: string;
	prefix: string;
	boundValue: BoundValue | string;
}

/** The canvas fields that can carry a storage path. */
export interface CanvasSettingsLike {
	backgroundImage?: string;
}

/**
 * Extracts all storage prefixes from page components that need presigning
 */
export function extractAssetPrefixes(
	components: SurfaceComponent[],
): AssetPrefixInfo[] {
	const assets: AssetPrefixInfo[] = [];

	for (const component of components) {
		if (!component.component) continue;
		const comp = component.component as unknown as Record<string, unknown>;
		const type = comp.type as string;

		const assetProperties = ASSET_FIELDS_BY_COMPONENT[type];
		if (!assetProperties) continue;

		// Check each asset property
		for (const prop of assetProperties) {
			const value = comp[prop] as BoundValue | string | undefined;
			if (!value) continue;

			const stringValue = extractStringFromBoundValue(value);
			if (stringValue && isStoragePrefix(stringValue)) {
				assets.push({
					componentId: component.id,
					property: prop,
					prefix: stringValue,
					boundValue: value,
				});
			}
		}
	}

	return assets;
}

async function signPrefixes(
	appId: string,
	prefixes: string[],
	storageState: IStorageState,
): Promise<Map<string, string>> {
	const urlMap = new Map<string, string>();
	if (prefixes.length === 0) return urlMap;

	const signedUrls = await storageState.downloadStorageItems(appId, prefixes);
	for (const result of signedUrls) {
		if (result.url && !result.error) {
			urlMap.set(result.prefix, result.url);
		}
	}
	return urlMap;
}

function applySignedUrls(
	components: SurfaceComponent[],
	assets: AssetPrefixInfo[],
	urlMap: Map<string, string>,
): SurfaceComponent[] {
	return components.map((component) => {
		const relevantAssets = assets.filter((a) => a.componentId === component.id);
		if (relevantAssets.length === 0) return component;

		const updatedComponent = { ...component };
		if (updatedComponent.component) {
			const comp = { ...updatedComponent.component } as unknown as Record<
				string,
				unknown
			>;

			for (const asset of relevantAssets) {
				const signedUrl = urlMap.get(asset.prefix);
				if (signedUrl) {
					comp[asset.property] = updateBoundValueString(
						asset.boundValue,
						signedUrl,
					);
				}
			}

			updatedComponent.component =
				comp as unknown as typeof component.component;
		}

		return updatedComponent;
	});
}

/**
 * Presigns all asset URLs in page components
 * Returns a new components array with presigned URLs
 */
export async function presignPageAssets(
	appId: string,
	components: SurfaceComponent[],
	storageState: IStorageState,
): Promise<SurfaceComponent[]> {
	const assets = extractAssetPrefixes(components);
	if (assets.length === 0) return components;

	const urlMap = await signPrefixes(
		appId,
		[...new Set(assets.map((a) => a.prefix))],
		storageState,
	);
	return applySignedUrls(components, assets, urlMap);
}

/**
 * Presigns everything a page renders in one request.
 *
 * The component assets and the canvas background were signed by two awaited calls in sequence,
 * so every page open paid two round trips to render one screen. They address the same storage
 * and the same endpoint takes a list, so one batch answers both.
 */
export async function presignPageContent(
	appId: string,
	components: SurfaceComponent[],
	canvasSettings: CanvasSettingsLike | undefined,
	storageState: IStorageState,
): Promise<{
	components: SurfaceComponent[];
	backgroundImage: string | undefined;
}> {
	const assets = extractAssetPrefixes(components);
	const background = canvasSettings?.backgroundImage;
	const backgroundNeedsSigning = Boolean(
		background && isStoragePrefix(background),
	);

	const prefixes = [
		...new Set([
			...assets.map((a) => a.prefix),
			...(backgroundNeedsSigning && background ? [background] : []),
		]),
	];

	if (prefixes.length === 0) {
		return { components, backgroundImage: background };
	}

	const urlMap = await signPrefixes(appId, prefixes, storageState);

	return {
		components:
			assets.length > 0
				? applySignedUrls(components, assets, urlMap)
				: components,
		backgroundImage:
			backgroundNeedsSigning && background
				? (urlMap.get(background) ?? background)
				: background,
	};
}

/**
 * Checks if a storage path is for an image/asset file
 */
export function isAssetFile(path: string): boolean {
	const ext = path.split(".").pop()?.toLowerCase();
	if (!ext) return false;

	const imageExtensions = [
		"jpg",
		"jpeg",
		"png",
		"gif",
		"webp",
		"svg",
		"ico",
		"bmp",
		"avif",
	];
	const modelExtensions = [
		"glb",
		"gltf",
		"obj",
		"fbx",
		"usdz",
		"usd",
		"3ds",
		"dae",
	];
	const videoExtensions = ["mp4", "webm", "ogg", "ogv", "mov", "mkv", "avi"];
	const audioExtensions = [
		"mp3",
		"wav",
		"ogg",
		"oga",
		"opus",
		"flac",
		"aac",
		"m4a",
		"aif",
		"aiff",
	];
	const documentExtensions = [
		"pdf",
		"txt",
		"csv",
		"md",
		"mdx",
		"log",
		"json",
		"xml",
		"css",
		"js",
		"jsx",
		"ts",
		"tsx",
		"py",
		"rs",
		"html",
		"yml",
		"yaml",
		"toml",
		"sql",
	];
	const animationExtensions = ["json", "lottie"];
	const environmentExtensions = ["hdr", "exr"];

	return (
		imageExtensions.includes(ext) ||
		modelExtensions.includes(ext) ||
		videoExtensions.includes(ext) ||
		audioExtensions.includes(ext) ||
		documentExtensions.includes(ext) ||
		animationExtensions.includes(ext) ||
		environmentExtensions.includes(ext)
	);
}

/**
 * Presigns a single storage path if it needs presigning
 * Returns the presigned URL or the original value if not a storage path
 */
export async function presignSinglePath(
	appId: string,
	path: string | undefined,
	storageState: IStorageState,
): Promise<string | undefined> {
	if (!path || !isStoragePrefix(path)) {
		return path;
	}

	try {
		const results = await storageState.downloadStorageItems(appId, [path]);
		const result = results[0];
		if (result?.url && !result.error) {
			return result.url;
		}
	} catch (err) {
		console.warn("[presignSinglePath] Failed to presign:", path, err);
	}

	return path;
}

export interface PresignableCanvasSettings {
	backgroundColor: string;
	backgroundImage?: string;
	padding: string;
	/** Custom CSS to inject into the canvas (scoped to canvas container) */
	customCss?: string;
}

/**
 * Presigns canvas settings (specifically the background image)
 * Returns new canvas settings with presigned background image URL
 */
export async function presignCanvasSettings(
	appId: string,
	settings: PresignableCanvasSettings,
	storageState: IStorageState,
): Promise<PresignableCanvasSettings> {
	if (!settings.backgroundImage || !isStoragePrefix(settings.backgroundImage)) {
		return settings;
	}

	const presignedUrl = await presignSinglePath(
		appId,
		settings.backgroundImage,
		storageState,
	);

	return {
		...settings,
		backgroundImage: presignedUrl,
	};
}
