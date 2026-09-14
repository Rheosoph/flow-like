import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";

const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));

export default defineConfig({
	root,
	devToolbar: { enabled: false },
	server: { host: "127.0.0.1", port: 4332 },
	vite: {
		plugins: [tailwindcss()],
		server: { fs: { allow: [repo] }, strictPort: true },
	},
});
