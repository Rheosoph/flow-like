/** @type {import('next').NextConfig} */
export default {
	output: "export",
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
