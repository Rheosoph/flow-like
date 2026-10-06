import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { prepareMapLibreAssets } from "../../packages/ui/scripts/prepare-maplibre.mjs";

// @ts-expect-error process is a nodejs global
const mobile = !!/android|ios/.exec(process.env.TAURI_ENV_PLATFORM);
const host = process.env.TAURI_DEV_HOST;

// @ts-ignore
export default defineConfig(async () => ({
	define: {
		"process.env.NEXT_PUBLIC_MAPLIBRE_WORKER_URL": JSON.stringify(
			prepareMapLibreAssets(new URL("./public/", import.meta.url)),
		),
	},
	plugins: [
		react(),
		// tailwindcss()
	],
	// Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
	//
	// 1. prevent vite from obscuring rust errors
	clearScreen: false,
	// 2. tauri expects a fixed port, fail if that port is not available
	// to access the Tauri environment variables set by the CLI with information about the current target
	envPrefix: [
		"VITE_",
		"TAURI_PLATFORM",
		"TAURI_ARCH",
		"TAURI_FAMILY",
		"TAURI_PLATFORM_VERSION",
		"TAURI_PLATFORM_TYPE",
		"TAURI_DEBUG",
	],
	server: {
		port: 1420,
		strictPort: true,
		host: mobile ? (host ?? false) : false,
		hmr: mobile
			? {
					protocol: "ws",
					host,
					port: 1421,
				}
			: undefined,
	},
}));
