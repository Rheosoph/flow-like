import { flowLikeWidgets } from "@flow-like/widget-bundler/vite";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [flowLikeWidgets()],
});
