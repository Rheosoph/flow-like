import { flowLikeWidgets } from "@flow-like/widget-bundler/vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [svelte(), flowLikeWidgets()],
});
