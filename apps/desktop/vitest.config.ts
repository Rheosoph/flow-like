import path from "node:path";
import { defineConfig } from "vitest/config";

export default defineConfig({
	test: {
		include: ["**/__tests__/**/*.test.ts"],
		server: {
			deps: {
				inline: [/.*/],
			},
		},
	},
	resolve: {
		alias: {
			"tauri-plugin-remote-push-api": path.resolve(
				__dirname,
				"../../node_modules/tauri-plugin-remote-push-api/guest-js/index.ts",
			),
		},
	},
});
