"use client";
import process from "node:process";
/** @type {import('next').NextConfig} */
import { withSentryConfig } from "@sentry/nextjs";

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
	missingSuspenseWithCSRBailout: false,
	experimental: {
		serverComponentsHmrCache: true,
		webpackMemoryOptimizations: true,
		preloadEntriesOnStart: false,
		turbopackFileSystemCacheForDev: true,
	},
	devIndicators: {
		appIsrStatus: false,
	},
};

export default withSentryConfig(nextConfig, {
	org: "good-code",
	project: "flow-like-desktop",

	// An auth token is required for uploading source maps.
	authToken: process.env.SENTRY_AUTH_TOKEN,

	silent: false, // Can be used to suppress logs
});
