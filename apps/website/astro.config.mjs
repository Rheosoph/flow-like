import cloudflare from "@astrojs/cloudflare";
import react from "@astrojs/react";
import tailwindcss from "@tailwindcss/vite";
import compressor from "astro-compressor";
import { defineConfig } from "astro/config";

import mdx from "@astrojs/mdx";

import sitemap from "@astrojs/sitemap";

// https://astro.build/config
export default defineConfig({
	site: "https://flow-like.com",
	adapter: cloudflare(),
	i18n: {
		defaultLocale: "en",
		locales: ["en", "de", "es", "fr", "zh", "ja", "ko", "pt", "it", "nl", "sv"],
		routing: {
			prefixDefaultLocale: false,
		},
	},
	integrations: [
		// markdoc(),
		// robotsTxt(),
		sitemap({
			customPages: [
				"https://flow-like.com/store",
				"https://flow-like.com/store/packages",
				"https://flow-like.com/store/apps",
			],
		}),
		// playformCompress(),
		react(),
		mdx({
			syntaxHighlight: "shiki",
			shikiConfig: {
				theme: "dracula",
				wrap: true,
			},
			remarkRehype: { footnoteLabel: "Footnotes" },
			gfm: true,
		}),
		(await import("@playform/compress")).default(),
		compressor(),
	],
	vite: {
		// React's jsx-dev-runtime is conditional on NODE_ENV. Keep Vite's client,
		// SSR, and Astro dependency caches physically separate so a production
		// pre-bundle can never be reused by development transforms that call jsxDEV.
		cacheDir:
			process.env.NODE_ENV === "production"
				? "node_modules/.vite-production"
				: "node_modules/.vite-development",
		define: {
			"process.env": {},
		},
		resolve: {
			dedupe: ["react", "react-dom"],
		},
		ssr: {
			noExternal: [
				"katex",
				"rehype-katex",
				"@flow-like/flow-like-ui",
				"lodash-es",
				"@platejs/math",
				"@platejs/markdown",
				"platejs",
				"react-lite-youtube-embed",
				"react-tweet",
			],
		},
		plugins: [tailwindcss()],
	},
	markdown: {
		syntaxHighlight: "shiki",
		shikiConfig: {
			themes: {
				light: "min-light",
				dark: "dracula",
			},
			wrap: true,
		},
	},
});
