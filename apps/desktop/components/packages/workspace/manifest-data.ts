export interface ManifestData {
	manifest_version: number;
	id: string;
	name: string;
	version: string;
	description: string;
	authors: { name: string; email?: string; url?: string }[];
	license?: string;
	repository?: string;
	homepage?: string;
	keywords: string[];
	// Capability flags are not authored: nodes declare them in code and the
	// registry derives the store listing from the compiled nodes.
	permissions: {
		memory: string;
		timeout: string;
		network?: {
			allowed_hosts: string[];
		};
		oauth_scopes?: {
			provider: string;
			scopes: string[];
			reason: string;
			required: boolean;
		}[];
	};
}

type RawManifest = Partial<Omit<ManifestData, "permissions">> & {
	permissions?: Partial<ManifestData["permissions"]>;
};

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string {
	return typeof value === "string" ? value : "";
}

function authorList(value: unknown): ManifestData["authors"] {
	if (!Array.isArray(value)) return [];
	return value.filter(isRecord).map((author) => ({
		...author,
		name: text(author.name),
	}));
}

function stringList(value: unknown): string[] {
	return Array.isArray(value)
		? value.filter((item): item is string => typeof item === "string")
		: [];
}

/**
 * Fills the fields `flow-like.toml` may omit (the serde defaults on
 * `PackageManifest`), so the editor never dereferences a missing one. Unknown
 * keys such as `[[nodes]]` pass through untouched and are saved back.
 */
export function normalizeManifest(raw: unknown): ManifestData {
	const manifest: RawManifest = isRecord(raw) ? raw : {};
	const permissions = isRecord(manifest.permissions)
		? manifest.permissions
		: {};
	return {
		...manifest,
		manifest_version: manifest.manifest_version ?? 1,
		id: text(manifest.id),
		name: text(manifest.name),
		version: text(manifest.version),
		description: text(manifest.description),
		authors: authorList(manifest.authors),
		keywords: stringList(manifest.keywords),
		permissions: {
			...permissions,
			memory: permissions.memory ?? "standard",
			timeout: permissions.timeout ?? "standard",
		},
	};
}
