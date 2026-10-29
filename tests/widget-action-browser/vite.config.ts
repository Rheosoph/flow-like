import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import tailwind from "@tailwindcss/postcss";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));
const proxy = {
	"/benchmark": {
		target: process.env.WIDGET_BENCHMARK_SERVER || "http://127.0.0.1:4340",
		changeOrigin: false,
		rewrite: (path: string) => path.replace(/^\/benchmark/, ""),
	},
};

export default defineConfig({
	root,
	cacheDir: resolve(tmpdir(), "flow-like-widget-action-vite"),
	define: { "process.env": "{}" },
	plugins: [
		react(),
		{
			name: "widget-benchmark-boundaries",
			enforce: "pre",
			resolveId(id, importer) {
				if (["next/link", "next/image", "next/dynamic"].includes(id))
					return `\0widget-benchmark:${id}`;
				if (!importer || !id.startsWith(".")) return;
				const path = resolve(dirname(importer), id).replace(/\.[jt]sx?$/, "");
				if (path === `${repo}packages/ui/components/a2ui/ComponentRegistry`)
					return `${root}component-registry.tsx`;
				if (path === `${repo}packages/ui/state/backend-state`)
					return `${root}backend.ts`;
				if (path === `${repo}packages/ui/state/execution-service-context`)
					return `${repo}packages/ui/state/execution-service-context-value.ts`;
				if (
					path === `${repo}packages/ui/components/ui/index` ||
					path === `${repo}packages/ui/components/ui`
				)
					return `${root}ui-exports.ts`;
			},
			load(id) {
				if (!id.startsWith("\0widget-benchmark:")) return;
				const name = id.endsWith("/link")
					? "Link"
					: id.endsWith("/image")
						? "Image"
						: "dynamic";
				return `export { ${name} as default } from ${JSON.stringify(`${repo}tests/admin-browser/next-components.tsx`)};`;
			},
		},
	],
	resolve: {
		alias: [
			{
				find: "next/navigation",
				replacement: `${repo}tests/admin-browser/next-navigation.ts`,
			},
		],
		dedupe: ["react", "react-dom", "@tanstack/react-query"],
	},
	css: { postcss: { plugins: [tailwind({ base: root })] } },
	build: {
		outDir: "../../tmp/widget-action-benchmark/site",
		emptyOutDir: true,
	},
	server: {
		host: "127.0.0.1",
		port: 4342,
		strictPort: true,
		proxy,
		fs: { allow: [repo] },
	},
	preview: { host: "127.0.0.1", port: 4342, strictPort: true, proxy },
});
