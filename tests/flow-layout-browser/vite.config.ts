import { fileURLToPath } from "node:url";
import tailwind from "@tailwindcss/postcss";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));

export default defineConfig({
	root,
	define: { "process.env": "{}" },
	plugins: [react()],
	resolve: { dedupe: ["react", "react-dom"] },
	css: { postcss: { plugins: [tailwind()] } },
	server: {
		host: "127.0.0.1",
		port: 4334,
		strictPort: true,
		hmr: false,
		fs: { allow: [repo] },
	},
});
