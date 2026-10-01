import { unified } from "@astrojs/markdown-remark";
import react from "@astrojs/react";
import starlight from "@astrojs/starlight";
import tailwindcss from "@tailwindcss/vite";

import { defineConfig } from "astro/config";
import { sidebar } from "./src/sidebar.mjs";

// Match Starlight's article column so desktop browsers do not download an
// image sized for the whole window. Keep the source resolution for zooming.
function articleImageSizes() {
	return (tree) => {
		function visit(node) {
			if (node.type === "element" && node.tagName === "img") {
				node.properties ??= {};
				node.properties.sizes ??=
					"(min-width: 72rem) 45rem, (min-width: 50rem) calc(100vw - 22rem), calc(100vw - 2rem)";
			}
			for (const child of node.children ?? []) visit(child);
		}
		visit(tree);
	};
}

// https://astro.build/config
export default defineConfig({
	site: "https://docs.flow-like.com",
	output: "static",
	// Astro 7 defaults to 'jsx', which drops the space between adjacent inline
	// elements. Keep HTML-aware whitespace so prose spacing stays unchanged.
	compressHTML: true,
	build: {
		// Docs navigation is page-to-page: a shared, cached stylesheet beats
		// re-sending the same Starlight CSS inlined into every one of documentation pages.
		inlineStylesheets: "never",
	},
	image: {
		layout: "constrained",
		responsiveStyles: true,
		breakpoints: [480, 800, 1200, 1600],
	},
	markdown: { processor: unified({ rehypePlugins: [articleImageSizes] }) },

	integrations: [
		react(),
		starlight({
			title: "Flow-Like Docs",
			expressiveCode: {
				shiki: { langAlias: { nivo: "json", plotly: "json" } },
			},
			favicon: "/favicon.svg",
			description:
				"Documentation for Flow-Like, the source-available local-first workflow engine. Build type-safe, self-hosted automation with Rust performance.",
			components: {
				MarkdownContent: "./src/components/docs/MarkdownContent.astro",
				Search: "./src/components/docs/Search.astro",
				SiteTitle: "./src/components/docs/SiteTitle.astro",
			},
			head: [
				{
					tag: "meta",
					attrs: {
						name: "robots",
						content:
							"index,follow,max-image-preview:large,max-snippet:-1,max-video-preview:-1",
					},
				},
				{
					tag: "link",
					attrs: {
						rel: "icon",
						type: "image/svg+xml",
						href: "/favicon.svg",
					},
				},
				{
					tag: "link",
					attrs: {
						rel: "icon",
						type: "image/png",
						href: "/favicon-32x32.png",
						sizes: "32x32",
					},
				},
				{
					tag: "link",
					attrs: {
						rel: "icon",
						type: "image/png",
						href: "/favicon-16x16.png",
						sizes: "16x16",
					},
				},
			],
			editLink: {
				baseUrl: "https://github.com/Rheosoph/flow-like/edit/dev/apps/docs/",
			},
			logo: {
				light: "./src/assets/icon.webp",
				dark: "./src/assets/icon.webp",
			},
			customCss: ["./src/styles/global.css"],
			social: [
				{
					icon: "discord",
					label: "Discord",
					href: "https://discord.gg/mdBA9kMjFJ",
				},
				{
					icon: "github",
					label: "GitHub",
					href: "https://github.com/Rheosoph/flow-like",
				},
				{ icon: "x.com", label: "X.com", href: "https://x.com/greatco_de" },
				{
					icon: "linkedin",
					label: "LinkedIn",
					href: "https://linkedin.com/company/greatco-de",
				},
			],
			lastUpdated: true,
			tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 4 },
			sidebar,
		}),
	],
	vite: {
		ssr: {
			noExternal: [
				"katex",
				"rehype-katex",
				"@flow-like/flow-like-ui",
				"lodash-es",
				"@platejs/math",
				"react-lite-youtube-embed",
				"react-tweet",
			],
		},
		define: {
			"process.env": {},
			"process.env.NODE_ENV": JSON.stringify(
				process.env.NODE_ENV || "production",
			),
		},
		plugins: [tailwindcss()],
	},
});
