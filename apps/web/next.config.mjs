import { PHASE_DEVELOPMENT_SERVER } from "next/constants.js";

/** @type {import('next').NextConfig} */
const nextConfig = {
	reactStrictMode: false,
	output: "export",
	pageExtensions: ["js", "jsx", "md", "mdx", "ts", "tsx"],
	reactCompiler: true,
	images: {
		unoptimized: true,
	},
	transpilePackages: ["@flow-like/flow-like-ui", "@flow-like/locales"],
	experimental: {
		serverComponentsHmrCache: true,
		webpackMemoryOptimizations: true,
		preloadEntriesOnStart: false,
		turbopackFileSystemCacheForDev: true,
	},
	webpack: (config) => {
		config.resolve.fallback = {
			...config.resolve.fallback,
			fs: false,
			net: false,
			tls: false,
		};
		return config;
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
		rewrites: async () => [
			{ source: "/use/:path*", destination: "/use" },
			{ source: "/a/:path*", destination: "/a" },
		],
	};
};
