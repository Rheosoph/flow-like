import { unified } from "@astrojs/markdown-remark";
import react from "@astrojs/react";
import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";
import rehypeFlowbookTables from "./src/lib/rehype-flowbook-tables.mjs";

export default defineConfig({
	site: "https://book.flow-like.com",
	output: "static",
	compressHTML: true,
	build: {
		inlineStylesheets: "never",
	},
	integrations: [
		react(),
		starlight({
			title: "FlowBook",
			description:
				"The FlowScript book: build reliable software in code and as a visible workflow.",
			credits: false,
			favicon: "https://flow-like.com/favicon.svg",
			customCss: ["./src/styles/book.css"],
			expressiveCode: {
				defaultProps: {
					wrap: true,
					preserveIndent: true,
					hangingIndent: 2,
					overridesByLang: {
						text: { wrap: false },
					},
				},
			},
			components: {
				ContentPanel: "./src/components/BookContentPanel.astro",
				Footer: "./src/components/BookFooter.astro",
				Hero: "./src/components/BookHero.astro",
			},
			editLink: {
				baseUrl: "https://github.com/Rheosoph/flow-like/edit/main/apps/book/",
			},
			head: [
				{
					tag: "meta",
					attrs: { name: "theme-color", content: "#0c0e13" },
				},
				{
					tag: "meta",
					attrs: {
						property: "og:image",
						content: "https://book.flow-like.com/og.png",
					},
				},
				{
					tag: "meta",
					attrs: { property: "og:image:width", content: "1200" },
				},
				{
					tag: "meta",
					attrs: { property: "og:image:height", content: "630" },
				},
				{
					tag: "meta",
					attrs: {
						property: "og:image:alt",
						content: "FlowBook — source code becoming a visible workflow",
					},
				},
				{
					tag: "meta",
					attrs: { name: "twitter:card", content: "summary_large_image" },
				},
				{
					tag: "meta",
					attrs: {
						name: "twitter:image",
						content: "https://book.flow-like.com/og.png",
					},
				},
			],
			social: [
				{
					icon: "github",
					label: "Flow-Like on GitHub",
					href: "https://github.com/Rheosoph/flow-like",
				},
			],
			tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
			sidebar: [
				{
					label: "Begin",
					items: [
						{ label: "Welcome", slug: "" },
						{ label: "Introduction", slug: "introduction" },
						{ label: "Complete contents", slug: "contents" },
					],
				},
				{
					label: "Part I — Software That Explains Itself",
					items: [
						{
							label: "1. The 3 A.M. Call",
							slug: "part-1/01-the-3-am-call",
						},
						{
							label: "2. The Manifesto: Constrained Freedom",
							slug: "part-1/02-manifesto-constrained-freedom",
						},
						{
							label: "3. One Platform, One Flow Model",
							slug: "part-1/03-one-platform-one-flow-model",
						},
						{
							label: "4. First Flow: Incident Triage",
							slug: "part-1/04-first-flow-incident-triage",
						},
					],
				},
				{
					label: "Part II — Thinking and Writing in Flows",
					items: [
						{
							autogenerate: {
								directory: "part-2",
							},
						},
					],
				},
				{
					label: "Part III — The Two-Way Contract",
					items: [
						{
							autogenerate: {
								directory: "part-3",
							},
						},
					],
				},
			],
		}),
	],
	markdown: {
		processor: unified({ rehypePlugins: [rehypeFlowbookTables] }),
		syntaxHighlight: "shiki",
		shikiConfig: {
			langAlias: {
				flow: "typescript",
			},
			themes: {
				light: "min-light",
				dark: "dracula",
			},
			wrap: true,
		},
	},
});
