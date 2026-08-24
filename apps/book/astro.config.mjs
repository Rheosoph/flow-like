import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";

export default defineConfig({
	site: "https://book.flow-like.com",
	output: "static",
	compressHTML: true,
	build: {
		inlineStylesheets: "never",
	},
	integrations: [
		starlight({
			title: "FlowBook",
			description:
				"The FlowScript book: build reliable software in code and as a visible workflow.",
			credits: false,
			favicon: "https://flow-like.com/favicon.svg",
			customCss: ["./src/styles/book.css"],
			components: {
				ContentPanel: "./src/components/BookContentPanel.astro",
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
							label: "5. Nodes, Pins, Wires, and Execution",
							slug: "part-2/05-nodes-pins-wires-execution",
						},
						{
							label: "6. Anatomy of a FlowScript Document",
							slug: "part-2/06-anatomy-of-a-flowscript-document",
						},
						{
							label: "7. Values, Types, Collections, and Interfaces",
							slug: "part-2/07-values-types-collections-interfaces",
						},
						{
							label: "8. Calling the Node Library",
							slug: "part-2/08-calling-the-node-library",
						},
						{
							label: "9. Expressions, Operators, and Readable Sugar",
							slug: "part-2/09-expressions-operators-readable-sugar",
						},
						{
							label: "10. Branches, Loops, Parallelism, and Return",
							slug: "part-2/10-branches-loops-parallelism-return",
						},
						{
							label: "11. State, Configuration, Runtime Values, and Secrets",
							slug: "part-2/11-state-configuration-runtime-values-secrets",
						},
					],
				},
			],
		}),
	],
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
