import { flowLikeWidgets } from "@flow-like/widget-bundler/vite";
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
	plugins: [solid(), flowLikeWidgets()],
});
