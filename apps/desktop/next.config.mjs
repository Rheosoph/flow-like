import { PHASE_DEVELOPMENT_SERVER } from "next/constants.js";

/** @type {import('next').NextConfig} */
const nextConfig = {
	output: "export",
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
		// PostCSS and other Node-backed Turbopack plugins otherwise run in child
		// processes connected over sockets. Worker threads avoid the process-pool
		// stall seen while compiling dependency CSS and keep the work parallel.
		turbopackPluginRuntimeStrategy: "workerThreads",
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
		// Keep /use/ distinct from an Event-only /use link during development.
		skipTrailingSlashRedirect: true,
		// Production serves these paths through the static host or Tauri assets.
		rewrites: async () => [{ source: "/use/:path*", destination: "/use" }],
	};
};
