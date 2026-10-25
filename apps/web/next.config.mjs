import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import process from "node:process";
import { URL } from "node:url";
import {
	PHASE_DEVELOPMENT_SERVER,
	PHASE_PRODUCTION_BUILD,
} from "next/constants.js";
import { prepareExcalidrawAssets } from "../../packages/ui/scripts/prepare-excalidraw.mjs";
import { prepareMapLibreAssets } from "../../packages/ui/scripts/prepare-maplibre.mjs";
import { prepareMonacoAssets } from "./scripts/prepare-monaco.mjs";

function readAppVersion() {
	try {
		const config = readFileSync(
			new URL("../desktop/src-tauri/tauri.conf.json", import.meta.url),
			"utf8",
		);
		return JSON.parse(config).version ?? "";
	} catch {
		return "";
	}
}

function readCommit() {
	try {
		return execSync("git rev-parse --short=12 HEAD", {
			cwd: new URL(".", import.meta.url),
			encoding: "utf8",
			stdio: ["ignore", "pipe", "ignore"],
		}).trim();
	} catch {
		return "";
	}
}

/** @type {import('next').NextConfig} */
const nextConfig = {
	reactStrictMode: false,
	poweredByHeader: false,
	output: "export",
	env: {
		NEXT_PUBLIC_EXCALIDRAW_ASSET_PATH: prepareExcalidrawAssets(
			new URL("./public/", import.meta.url),
		),
		NEXT_PUBLIC_MAPLIBRE_WORKER_URL: prepareMapLibreAssets(
			new URL("./public/", import.meta.url),
		),
		NEXT_PUBLIC_FLOW_LIKE_VERSION: readAppVersion(),
		NEXT_PUBLIC_FLOW_LIKE_COMMIT:
			process.env.CF_PAGES_COMMIT_SHA ?? process.env.GITHUB_SHA ?? readCommit(),
		NEXT_PUBLIC_FLOW_LIKE_BUILT_AT: new Date().toISOString(),
	},
	pageExtensions: ["js", "jsx", "md", "mdx", "ts", "tsx"],
	reactCompiler: true,
	images: {
		unoptimized: true,
	},
	transpilePackages: ["@flow-like/flow-like-ui", "@flow-like/locales"],
	// Share one server-side Yjs instance across route bundles and dev recompiles.
	serverExternalPackages: ["yjs"],
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
	if (phase === PHASE_DEVELOPMENT_SERVER || phase === PHASE_PRODUCTION_BUILD) {
		prepareMonacoAssets();
	}
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
