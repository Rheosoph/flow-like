import {
	type InstalledPackage,
	MemoryTier,
	TimeoutTier,
} from "../../packages/ui/lib/schema/wasm";
import { packages } from "./data";

let installed: InstalledPackage[] = packages.slice(0, 3).map((pkg) => ({
	id: pkg.id,
	version: pkg.latestVersion,
	source: { type: "remote", registryUrl: "https://fixture.invalid" },
	installedAt: "2026-09-05T10:00:00Z",
	wasmPath: `/fixture/packages/${pkg.id}/package.wasm`,
	metadata: pkg.metadata,
	manifest: {
		manifestVersion: 1,
		id: pkg.id,
		name: pkg.metadata?.name ?? pkg.name,
		version: pkg.latestVersion,
		description: pkg.description,
		authors: [{ name: "Community Studio" }],
		license: "MIT",
		keywords: pkg.keywords,
		primaryCategory: pkg.primaryCategory,
		metadata: {},
		permissions: {
			memory: MemoryTier.Standard,
			timeout: TimeoutTier.Standard,
			network: {
				httpEnabled: false,
				allowedHosts: [],
				websocketEnabled: false,
				tcpEnabled: false,
				udpEnabled: false,
				dnsEnabled: false,
			},
			filesystem: {
				nodeStorage: true,
				userStorage: false,
				uploadDir: false,
				cacheDir: true,
			},
			oauthScopes: [],
			variables: true,
			cache: true,
			streaming: false,
			a2ui: false,
			models: false,
		},
	},
}));

export function desktopNativeResponse(
	command: string,
	args: Record<string, unknown> = {},
	empty = false,
): unknown {
	switch (command) {
		case "registry_init":
			return null;
		case "registry_get_installed_packages":
			return empty ? [] : installed;
		case "registry_check_for_updates":
			return [];
		case "registry_uninstall_package":
			installed = installed.filter((pkg) => pkg.id !== args.packageId);
			return null;
		case "registry_update_package":
			return null;
		default:
			throw new Error(`Unhandled native fixture command: ${command}`);
	}
}
