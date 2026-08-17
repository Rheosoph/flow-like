import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { localesApi } from "./server/locales-plugin";

export default defineConfig({
	plugins: [react(), tailwindcss(), localesApi()],
	server: {
		port: 5177,
		// The locale files live outside this app, so the dev server has to be
		// allowed to read them.
		fs: { allow: [".."] },
	},
});
