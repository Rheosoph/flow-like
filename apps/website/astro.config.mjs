import { fileURLToPath } from "node:url";
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
	// Astro 7 defaults to 'jsx', which drops the space between adjacent inline
	// elements. Keep HTML-aware whitespace so prose spacing stays unchanged.
	compressHTML: true,
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
		// The store hubs are on-demand routes and already reach the sitemap through
		// the route table (with a trailing slash). Listing them as customPages added
		// a second, slash-less copy of every URL. Per-item store URLs live in
		// /sitemap-store.xml, which robots.txt announces separately.
		sitemap(),
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
			alias: {
				// flow-like-ui leaf components import Next App Router hooks; this
				// site has no Next runtime, so alias them to inert no-ops so those
				// components (FlowPilot bubble, interactive a2ui) run in Astro islands.
				"next/navigation": fileURLToPath(
					new URL("./src/shims/next-navigation.ts", import.meta.url),
				),
			},
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
