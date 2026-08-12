import { flowLikeWidgets } from "@flow-like/widget-bundler/vite";
import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [vue(), flowLikeWidgets()],
});
