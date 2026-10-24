import { fileURLToPath } from "node:url";
import tailwind from "@tailwindcss/postcss";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Preview the real desktop page: bunx vite --config tests/onboarding-browser/vite.config.ts

const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));

export default defineConfig({
	root,
	publicDir: `${repo}apps/desktop/public`,
	define: { "process.env": "{}" },
	plugins: [
		react(),
		{
			name: "onboarding-direct-utilities",
			enforce: "pre",
			transform(code, id) {
				if (id.endsWith("/packages/ui/components/ui/alert.tsx")) {
					return code.replace('from "../../lib"', 'from "../../lib/utils"');
				}
			},
		},
	],
	resolve: {
		alias: [
			{ find: "next/link", replacement: `${root}next-link.tsx` },
			{
				find: /^@flow-like\/flow-like-ui$/,
				replacement: `${root}ui-exports.ts`,
			},
			{
				find: "next/navigation",
				replacement: `${repo}tests/home-browser/next-navigation.ts`,
			},
			{ find: "@tauri-apps/api/core", replacement: `${root}tauri.ts` },
			{ find: "@tauri-apps/plugin-http", replacement: `${root}tauri.ts` },
		],
		dedupe: ["react", "react-dom", "@tanstack/react-query"],
	},
	css: { postcss: { plugins: [tailwind({ base: root })] } },
	server: {
		host: "127.0.0.1",
		port: 4346,
		strictPort: true,
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
