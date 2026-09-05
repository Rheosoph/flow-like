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

export default nextConfig;
