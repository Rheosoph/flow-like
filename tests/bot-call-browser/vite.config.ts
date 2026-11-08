import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL("../..", import.meta.url));
export default defineConfig({
	root: fileURLToPath(new URL(".", import.meta.url)),
	define: { "process.env": "{}" },
	esbuild: { jsx: "automatic" },
	resolve: { dedupe: ["react", "react-dom", "@tanstack/react-query"] },
	server: {
		host: "127.0.0.1",
		strictPort: true,
		hmr: false,
		watch: { ignored: ["**/target/**", "**/.next/**", "**/dist/**"] },
		fs: { allow: [root] },
	},
});
