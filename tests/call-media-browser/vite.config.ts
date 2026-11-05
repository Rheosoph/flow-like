import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL("../..", import.meta.url));
export default defineConfig({
	root: fileURLToPath(new URL(".", import.meta.url)),
	publicDir: `${root}/node_modules/simple-peer`,
	esbuild: { jsx: "automatic" },
	server: { host: "127.0.0.1", strictPort: true, fs: { allow: [root] } },
});
