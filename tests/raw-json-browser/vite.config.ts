import { fileURLToPath } from "node:url";
import tailwind from "@tailwindcss/postcss";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL("./", import.meta.url));
const repo = fileURLToPath(new URL("../../", import.meta.url));

export default defineConfig({
	root,
	publicDir: `${repo}apps/web/public`,
	define: { "process.env": "{}" },
	plugins: [
		react(),
		{
			name: "raw-json-browser-fixtures",
			enforce: "pre",
			transform(code, id) {
				if (
					id.endsWith("/components/builder/DevModePanel.tsx") ||
					id.endsWith("/components/builder/Toolbar.tsx")
				) {
					return code.replace(
						'from "./BuilderContext"',
						`from ${JSON.stringify(`${root}builder-fixture.tsx`)}`,
					);
				}
			},
		},
	],
	resolve: { dedupe: ["react", "react-dom"] },
	css: { postcss: { plugins: [tailwind({ base: root })] } },
	server: {
		host: "127.0.0.1",
		port: 4354,
		strictPort: true,
		watch: { ignored: ["**/target/**", "**/.next/**", "**/dist/**"] },
		fs: { allow: [repo] },
	},
});
