import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL(".", import.meta.url));
const repo = fileURLToPath(new URL("../..", import.meta.url));
export default defineConfig({
	root,
	define: { "process.env": "{}" },
	esbuild: { jsx: "automatic" },
	resolve: {
		alias: [
			{ find: "next/navigation", replacement: `${root}next-navigation.ts` },
		],
		dedupe: ["react", "react-dom", "@tanstack/react-query"],
	},
	server: {
		host: "127.0.0.1",
		strictPort: true,
		hmr: false,
		watch: { ignored: ["**/target/**", "**/.next/**", "**/dist/**"] },
		fs: { allow: [repo] },
	},
});
