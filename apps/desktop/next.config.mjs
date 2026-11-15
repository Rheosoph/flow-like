import { PHASE_DEVELOPMENT_SERVER } from "next/constants.js";
import { prepareMapLibreAssets } from "../../packages/ui/scripts/prepare-maplibre.mjs";

/** @type {import('next').NextConfig} */
const nextConfig = {
	output: "export",
	env: {
		NEXT_PUBLIC_MAPLIBRE_WORKER_URL: prepareMapLibreAssets(
			new URL("./public/", import.meta.url),
		),
	},
	pageExtensions: ["js", "jsx", "md", "mdx", "ts", "tsx"],
	images: {
		unoptimized: true,
	},
	transpilePackages: [
		"@flow-like/flow-like-ui",
		"@flow-like/locales",
		"@flow-like/dexie-tauri-blob-offload",
		"@flow-like/widget-sdk",
		"tauri-plugin-remote-push-api",
	],
	// Keep yjs out of the server bundle: Turbopack re-evaluates bundled modules
	// on every dev recompile, and yjs's duplicate-import guard is a flag on
	// globalThis that survives those rebuilds ("Yjs was already imported").
	// Required from node_modules it is evaluated once per process instead.
	serverExternalPackages: ["yjs"],
	staticPageGenerationTimeout: 120,
	reactCompiler: true,
	experimental: {
		serverComponentsHmrCache: true,
		// This app's Next.js 16.4.3 build emits socket-based PostCSS helpers.
		// Worker threads do not supply the IPC port those helpers expect.
		turbopackPluginRuntimeStrategy: "childProcesses",
		turbopackRustReactCompiler: true,
		webpackMemoryOptimizations: true,
		preloadEntriesOnStart: false,
		turbopackFileSystemCacheForDev: true,
	},
	devIndicators: {
		appIsrStatus: false,
	},
};

export default (phase) => {
	if (phase !== PHASE_DEVELOPMENT_SERVER) return nextConfig;
	return {
		...nextConfig,
		output: undefined,
		experimental: {
			...nextConfig.experimental,
			// Avoid the child-process pool stall when compiling CSS in development.
			turbopackPluginRuntimeStrategy: "workerThreads",
		},
		// Keep /use/ distinct from an Event-only /use link during development.
		skipTrailingSlashRedirect: true,
		// Production serves these paths through the static host or Tauri assets.
		rewrites: async () => [{ source: "/use/:path*", destination: "/use" }],
	};
};
