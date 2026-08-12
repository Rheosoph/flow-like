import { flowLikeWidgets } from "@flow-like/widget-bundler/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [react(), flowLikeWidgets()],
});
