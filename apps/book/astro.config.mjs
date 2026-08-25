import { unified } from "@astrojs/markdown-remark";
import react from "@astrojs/react";
import sitemap from "@astrojs/sitemap";
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
		sitemap({
			filter: (page) => {
				const pathname = new URL(page).pathname;
				return (
					!/^\/print\/?$/.test(pathname) &&
					!pathname.endsWith(".md") &&
					pathname !== "/llms.txt" &&
					pathname !== "/llms-full.txt"
				);
			},
		}),
		starlight({
			title: "FlowBook",
			description:
				"Learn Flow-Like FlowScript through typed code, visual workflows, and observable execution.",
			credits: false,
			favicon: "/favicon.svg",
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
				Head: "./src/components/BookHead.astro",
				Hero: "./src/components/BookHero.astro",
				PageTitle: "./src/components/BookPageTitle.astro",
			},
			editLink: {
				baseUrl: "https://github.com/Rheosoph/flow-like/edit/main/apps/book/",
			},
			head: [
				{
					tag: "meta",
					attrs: { name: "theme-color", content: "#0c0e13" },
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
							label: "Part I overview",
							slug: "part-1",
						},
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
