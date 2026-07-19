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

	console.log("[presignPageAssets] Found assets:", assets);

	if (assets.length === 0) {
		return components;
	}

	// Get unique prefixes
	const uniquePrefixes = [...new Set(assets.map((a) => a.prefix))];

	console.log(
		"[presignPageAssets] Requesting presigned URLs for:",
		uniquePrefixes,
	);

	// Presign all prefixes in a single call
	const signedUrls = await storageState.downloadStorageItems(
		appId,
		uniquePrefixes,
	);

	console.log("[presignPageAssets] Received signed URLs:", signedUrls);

	// Create a map of prefix -> signed URL
	const urlMap = new Map<string, string>();
	for (const result of signedUrls) {
		if (result.url && !result.error) {
			urlMap.set(result.prefix, result.url);
		}
	}

	// Clone components and update URLs
	const updatedComponents = components.map((component) => {
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

	return updatedComponents;
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
