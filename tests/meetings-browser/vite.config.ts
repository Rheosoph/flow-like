import { fileURLToPath } from "node:url";
import tailwind from "@tailwindcss/postcss";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL(".", import.meta.url));
const repo = fileURLToPath(new URL("../..", import.meta.url));
export default defineConfig({
	root,
	define: { "process.env": "{}" },
	plugins: [
		react(),
		{
			name: "meeting-fixture-room",
			enforce: "pre",
			resolveId(id) {
				if (/\/hooks\/use-meeting-room(?:\.ts)?$/.test(id))
					return `${root}/room.ts`;
			},
		},
	],
	resolve: { dedupe: ["react", "react-dom", "@tanstack/react-query"] },
	css: { postcss: { plugins: [tailwind({ base: root })] } },
	server: {
		host: "127.0.0.1",
		strictPort: true,
		hmr: false,
		fs: { allow: [repo] },
		watch: { ignored: ["**/target/**", "**/.next/**", "**/dist/**"] },
	},
});
