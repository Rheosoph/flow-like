import { flowLikeWidgets } from "@flow-like/widget-bundler/vite";
import preact from "@preact/preset-vite";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [preact(), flowLikeWidgets()],
});
