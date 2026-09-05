import react from "@vitejs/plugin-react";
import tailwind from "@tailwindcss/postcss";
import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));
export default defineConfig({
	root,
	define: { "process.env": "{}" },
	plugins: [
		react(),
		{
			name: "next-fixture-exports",
			resolveId(id) {
				if (id === "next/link" || id === "next/image" || id === "next/dynamic")
					return `\0fixture:${id}`;
			},
			load(id) {
				if (id.startsWith("\0fixture:")) {
					const name = id.endsWith("/link")
						? "Link"
						: id.endsWith("/image")
							? "Image"
							: "dynamic";
					return `export { ${name} as default } from ${JSON.stringify(`${root}next-components.tsx`)};`;
				}
			},
		},
	],
	resolve: {
		alias: [
			{ find: "next/navigation", replacement: `${root}next-navigation.ts` },
		],
		dedupe: ["react", "react-dom", "@tanstack/react-query"],
	},
	css: { postcss: { plugins: [tailwind({ base: root })] } },
	server: {
		host: "127.0.0.1",
		port: 4318,
		strictPort: true,
		hmr: false,
		watch: { ignored: ["**/target/**", "**/.next/**", "**/dist/**"] },
		fs: { allow: [repo] },
	},
	optimizeDeps: {
		include: [
			"react",
			"react-dom/client",
			"@tanstack/react-query",
			"react-oidc-context",
		],
	},
});
