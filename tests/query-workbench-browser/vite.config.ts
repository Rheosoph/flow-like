import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import tailwind from "@tailwindcss/postcss";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));
export default defineConfig({
	root,
	define: {
		"process.env": "{}",
		__QUERY_MONACO_PATH__: JSON.stringify(
			`/@fs${repo}node_modules/monaco-editor/min/vs`,
		),
	},
	plugins: [
		react(),
		{
			name: "query-preview-import-boundaries",
			enforce: "pre",
			resolveId(id, importer) {
				if (!importer || !id.startsWith(".")) return;
				const path = resolve(dirname(importer), id);
				if (
					path === `${repo}packages/ui` ||
					path === `${repo}packages/ui/index` ||
					path === `${repo}packages/ui/components/ui` ||
					path === `${repo}packages/ui/components/ui/index`
				)
					return `${root}ui-exports.ts`;
			},
			transform(code, id) {
				if (!id.startsWith(`${repo}packages/ui/`)) return;
				// Keep cn imports from loading the unrelated workflow and app barrels.
				return code.replace(
					/import\s*\{\s*cn\s*\}\s*from\s*(["'])((?:\.\.\/)+lib)\1/g,
					(_match, quote, path) =>
						`import { cn } from ${quote}${path}/utils${quote}`,
				);
			},
		},
	],
	worker: { format: "es" },
	resolve: { dedupe: ["react", "react-dom", "@tanstack/react-query"] },
	css: { postcss: { plugins: [tailwind({ base: root })] } },
	server: {
		host: "127.0.0.1",
		port: 4345,
		strictPort: true,
		watch: { ignored: ["**/target/**", "**/.next/**", "**/dist/**"] },
		fs: { allow: [repo] },
	},
});
