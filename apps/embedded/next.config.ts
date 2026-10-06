"use client";

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
	staticPageGenerationTimeout: 120,
	missingSuspenseWithCSRBailout: false,
	experimental: {
		missingSuspenseWithCSRBailout: false,
	},
	devIndicators: {
		appIsrStatus: false,
	},
};

export default nextConfig;
