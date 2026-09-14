import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import tailwind from "@tailwindcss/postcss";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));
const { tiers, conversion, features } = JSON.parse(
	readFileSync(`${repo}flow-like.config.json`, "utf8"),
);
const config = { tiers, conversion, features, name: "Flow-Like" };

export default defineConfig({
	root,
	define: { "process.env": "{}", __PRICING_CONFIG__: JSON.stringify(config) },
	plugins: [
		react(),
		{
			name: "pricing-browser-fixtures",
			configureServer(server) {
				server.middlewares.use("/api/v1", (_request, response) => {
					response.setHeader("Content-Type", "application/json");
					response.end(JSON.stringify(config));
				});
			},
			resolveId(id) {
				if (/\/state\/backend-state(?:\.ts)?$/.test(id))
					return `${root}backend-fixture.ts`;
				if (["next/link", "next/image", "next/dynamic"].includes(id))
					return `\0fixture:${id}`;
			},
			load(id) {
				if (!id.startsWith("\0fixture:")) return;
				const name = id.endsWith("/link")
					? "Link"
					: id.endsWith("/image")
						? "Image"
						: "dynamic";
				return `export { ${name} as default } from ${JSON.stringify(`${repo}tests/profile-browser/next-components.tsx`)};`;
			},
		},
	],
	resolve: {
		alias: [
			{
				find: "next/navigation",
				replacement: `${repo}tests/pricing-browser/next-navigation.fixture.ts`,
			},
		],
		dedupe: ["react", "react-dom", "@tanstack/react-query"],
	},
	css: { postcss: { plugins: [tailwind()] } },
	server: {
		host: "127.0.0.1",
		port: 4330,
		strictPort: true,
		hmr: false,
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
