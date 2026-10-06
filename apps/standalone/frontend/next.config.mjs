import { prepareMapLibreAssets } from "../../../packages/ui/scripts/prepare-maplibre.mjs";
/** @type {import('next').NextConfig} */
export default {
	output: "export",
	env: {
		NEXT_PUBLIC_MAPLIBRE_WORKER_URL: prepareMapLibreAssets(
			new URL("./public/", import.meta.url),
			"/ui",
		),
	},
	basePath: "/ui",
	images: { unoptimized: true },
	transpilePackages: ["@flow-like/flow-like-ui", "@flow-like/locales"],
	webpack(config) {
		config.resolve.fallback = {
			...config.resolve.fallback,
			fs: false,
			net: false,
			tls: false,
		};
		return config;
	},
};
