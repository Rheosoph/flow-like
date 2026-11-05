import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
	root: fileURLToPath(new URL(".", import.meta.url)),
	esbuild: { jsx: "automatic" },
	server: {
		host: "127.0.0.1",
		port: 4348,
		strictPort: true,
		fs: { allow: [fileURLToPath(new URL("../..", import.meta.url))] },
	},
});
