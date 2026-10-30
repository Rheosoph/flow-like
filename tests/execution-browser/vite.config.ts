import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import tailwind from "@tailwindcss/postcss";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { executionFixtureServer } from "./server";

const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));

export default defineConfig({
	root,
	cacheDir: resolve(tmpdir(), "flow-like-execution-vite"),
	define: { "process.env": "{}" },
	plugins: [
		react(),
		executionFixtureServer(),
		{
			name: "execution-fixture-boundaries",
			enforce: "pre",
			resolveId(id, importer) {
				if (id === "@flow-like/locales") return `${root}fixture-boundaries.tsx`;
				if (!importer || !id.startsWith(".")) return;
				const path = resolve(dirname(importer), id).replace(/\.[jt]sx?$/, "");
				if (path === `${repo}packages/ui/state/execution-engine-context`)
					return `${root}execution-context.tsx`;
				if (
					[
						`${repo}packages/ui/lib`,
						`${repo}packages/ui/components/ui/text-editor`,
					].includes(path)
				)
					return `${root}fixture-boundaries.tsx`;
			},
		},
	],
	resolve: {
		alias: [
			{ find: "next/navigation", replacement: `${root}next-navigation.ts` },
		],
		dedupe: ["react", "react-dom"],
	},
	css: { postcss: { plugins: [tailwind({ base: root })] } },
	optimizeDeps: {
		include: [
			"react",
			"react-dom",
			"react-dom/client",
			"react/jsx-runtime",
			"react/jsx-dev-runtime",
			"@paralleldrive/cuid2",
			"@radix-ui/react-checkbox",
			"@radix-ui/react-label",
			"@radix-ui/react-popover",
			"@radix-ui/react-radio-group",
			"@radix-ui/react-select",
			"@radix-ui/react-slot",
			"@radix-ui/react-switch",
			"@tauri-apps/api/core",
			"class-variance-authority",
			"clsx",
			"date-fns",
			"dexie",
			"lucide-react",
			"lucide-react/dist/esm/icons/check.js",
			"lucide-react/dist/esm/icons/loader-2.js",
			"nanostores",
			"tailwind-merge",
			"@nats-io/nats-core",
		],
	},
	server: {
		host: "127.0.0.1",
		port: 4348,
		strictPort: true,
		hmr: false,
		fs: { allow: [repo] },
	},
});
