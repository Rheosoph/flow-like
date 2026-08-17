export const ASSET_EXTENSIONS: Record<string, string[]> = {
	image: ["jpg", "jpeg", "png", "gif", "webp", "svg", "ico", "bmp", "avif"],
	model: ["glb", "gltf", "obj", "fbx", "usdz", "usd", "3ds", "dae"],
	video: ["mp4", "webm", "ogg", "ogv", "mov", "mkv", "avi"],
	audio: [
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
	],
	document: [
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
	],
	animation: ["json", "lottie"],
	environment: ["hdr", "exr"],
};

const TRAVERSAL_SEGMENTS = new Set([".", ".."]);

// Picked paths are persisted into widget definitions and replayed as storage
// prefixes, so dot segments are dropped here as well as on the backend.
function pathSegments(path: string): string[] {
	return path
		.split("/")
		.filter((segment) => segment !== "" && !TRAVERSAL_SEGMENTS.has(segment));
}

export function basename(path: string): string {
	return pathSegments(path).pop() ?? "";
}

export function getExtension(path: string): string {
	const name = basename(path);
	const dot = name.lastIndexOf(".");
	return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function matchesAccept(path: string, accept: string): boolean {
	if (accept === "all") return true;
	const ext = getExtension(path);
	const extensions = ASSET_EXTENSIONS[accept];
	return extensions?.includes(ext) ?? false;
}

export function normalizePrefix(prefix: string): string {
	return pathSegments(prefix).join("/");
}

/**
 * Storage listings report either app-relative locations or raw object-store keys
 * (`apps/{app_id}/upload/media/logo.jpg`), depending on the backend. Both are
 * resolved against the browsed prefix so a listed entry can be handed straight
 * back to the storage API without its base being prepended twice.
 */
export function resolveAssetPath(prefix: string, location: string): string {
	const parent = normalizePrefix(prefix);
	const name = basename(location);
	if (!name) return parent;
	return parent ? `${parent}/${name}` : name;
}

export function parentPrefix(prefix: string): string {
	const parts = pathSegments(prefix);
	parts.pop();
	return parts.join("/");
}
