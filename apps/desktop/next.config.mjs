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
		"@flow-like/dexie-tauri-blob-offload",
		"@flow-like/widget-sdk",
		"tauri-plugin-remote-push-api",
	],
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
