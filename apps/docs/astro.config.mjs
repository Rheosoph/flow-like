import react from "@astrojs/react";
import starlight from "@astrojs/starlight";
import tailwindcss from "@tailwindcss/vite";

import { defineConfig, passthroughImageService } from "astro/config";
import { generatedNodeSidebar } from "./src/generated/node-sidebar.mjs";
// https://astro.build/config
export default defineConfig({
	site: "https://docs.flow-like.com",
	output: "static",
	// Astro 7 defaults to 'jsx', which drops the space between adjacent inline
	// elements. Keep HTML-aware whitespace so prose spacing stays unchanged.
	compressHTML: true,
	build: {
		// Docs navigation is page-to-page: a shared, cached stylesheet beats
		// re-sending the same Starlight CSS inlined into every one of ~1900 pages.
		inlineStylesheets: "never",
	},
	image: {
		service: passthroughImageService(),
	},

	integrations: [
		react(),
		starlight({
			title: "Flow-Like Docs",
			favicon: "/favicon.svg",
			description:
				"Documentation for Flow-Like, the open source local-first workflow engine. Build type-safe, self-hosted automation with Rust performance.",
			components: {
				Hero: "./src/components/docs/Hero.astro",
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
				baseUrl: "https://github.com/Rheosoph/flow-like/edit/main/apps/docs/",
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
			sidebar: [
				// ===== EVERYONE =====
				{
					label: "Getting Started",
					items: [
						{ label: "Quick Start", slug: "start/getting-started" },
						{ label: "What is Flow-Like?", slug: "start/what-is-flow-like" },
						{ label: "Download & Install", slug: "start/get" },
						{
							label: "Linux Troubleshooting",
							slug: "start/linux-troubleshooting",
						},
						{ label: "First Steps", slug: "start/first-use" },
						{ label: "Developer Mode", slug: "start/developer-mode" },
						{ label: "Login & Accounts", slug: "start/login" },
						{ label: "AI Models", slug: "start/models" },
						{ label: "Profiles", slug: "start/profiles" },
						{ label: "Get Support", slug: "start/support" },
					],
				},
				// ===== APP BUILDERS =====
				{
					label: "Building Apps",
					items: [
						{
							label: "Studio",
							collapsed: false,
							items: [
								{ label: "Overview", slug: "studio/overview" },
								{ label: "FlowPilot AI", slug: "studio/flowpilot" },
								{
									label: "Claude Code & Codex Setup",
									slug: "studio/flowpilot-external-agents",
								},
								{ label: "FlowScript", slug: "studio/flowscript" },
								{ label: "Working with Nodes", slug: "studio/nodes" },
								{ label: "Connecting Pins", slug: "studio/connecting" },
								{ label: "Layers & Organization", slug: "studio/layers" },
								{ label: "Variables", slug: "studio/variables" },
								{
									label: "Local-Only Execution",
									slug: "studio/local-execution",
								},
								{ label: "Logging & Debugging", slug: "studio/logging" },
								{ label: "Version Control", slug: "studio/versioning" },
							],
						},
						{
							label: "Data Studio",
							collapsed: false,
							items: [
								{ label: "Overview", slug: "apps/data-studio" },
								{
									label: "Ontology & Knowledge Graph",
									slug: "topics/ontology/overview",
								},
								{
									label: "Shared & Remote Ontologies",
									slug: "topics/ontology/remote",
								},
							],
						},
						{
							label: "Apps",
							collapsed: true,
							items: [
								{ label: "Overview", slug: "apps/overview" },
								{ label: "Creating Apps", slug: "apps/create" },
								{ label: "Boards & Flows", slug: "apps/boards" },
								{ label: "Runtime Variables", slug: "apps/runtime-variables" },
								{ label: "Pages", slug: "apps/pages" },
								{ label: "Routes", slug: "apps/routes" },
								{ label: "Widgets", slug: "apps/widgets" },
								{ label: "Chat UI", slug: "apps/chat-ui" },
								{ label: "Custom UI (A2UI)", slug: "apps/a2ui" },
								{ label: "Events", slug: "apps/events" },
								{ label: "Templates", slug: "apps/templates" },
								{ label: "Storage", slug: "apps/storage" },
								{ label: "Sharing", slug: "apps/share" },
								{ label: "Offline & Online", slug: "apps/offline-online" },
							],
						},
						{
							label: "Packages & Extensions",
							collapsed: true,
							items: [
								{ label: "Package Store", slug: "start/packages-store" },
								{ label: "Package Library", slug: "start/packages-library" },
							],
						},
						{
							label: "By Topic",
							collapsed: false,
							items: [
								{
									label: "GenAI",
									collapsed: true,
									items: [
										{ label: "Overview", slug: "topics/genai/overview" },
										{ label: "AI Models & Setup", slug: "topics/genai/models" },
										{
											label: "Chat & Conversations",
											slug: "topics/genai/chat",
										},
										{
											label: "RAG & Knowledge Bases",
											slug: "topics/genai/rag",
										},
										{ label: "AI Agents", slug: "topics/genai/agents" },
										{
											label: "Extraction & Structured Output",
											slug: "topics/genai/extraction",
										},
										{
											label: "Prompt Templates",
											slug: "topics/genai/prompt-templates",
										},
									],
								},
								{
									label: "Data Science",
									collapsed: true,
									items: [
										{ label: "Overview", slug: "topics/datascience/overview" },
										{
											label: "Data Loading & Storage",
											slug: "topics/datascience/loading",
										},
										{
											label: "Choosing a Lance Index",
											slug: "topics/datascience/lance-indexes",
										},
										{
											label: "DataFusion & SQL",
											slug: "topics/datascience/datafusion",
										},
										{
											label: "Machine Learning",
											collapsed: true,
											items: [
												{
													label: "Overview & Model Choice",
													slug: "topics/datascience/ml",
												},
												{
													label: "Advanced Configuration",
													slug: "topics/datascience/ml-configuration",
												},
												{
													label: "Auto Training",
													slug: "topics/datascience/ml-auto-training",
												},
											],
										},
										{
											label: "Data Visualization",
											slug: "topics/datascience/visualization",
										},
										{
											label: "AI-Powered Analysis",
											slug: "topics/datascience/ai-analysis",
										},
									],
								},
								{
									label: "Ontology & Knowledge Graph",
									collapsed: true,
									items: [
										{
											label: "Overview",
											slug: "topics/ontology/overview",
										},
										{
											label: "Shared & Remote Ontologies",
											slug: "topics/ontology/remote",
										},
									],
								},
								{
									label: "Internal Tools",
									collapsed: true,
									items: [
										{
											label: "Overview",
											slug: "topics/internal-tools/overview",
										},
									],
								},
								{
									label: "Desktop Automation",
									collapsed: true,
									items: [
										{
											label: "Overview",
											slug: "topics/desktop-automation/overview",
										},
									],
								},
								{
									label: "Document Processing",
									collapsed: true,
									items: [
										{
											label: "Overview",
											slug: "topics/document-processing/overview",
										},
										{
											label: "Summarization Strategies",
											slug: "topics/document-processing/summarization-strategies",
										},
									],
								},
								{
									label: "API Integrations",
									collapsed: true,
									items: [
										{
											label: "Overview",
											slug: "topics/api-integrations/overview",
										},
									],
								},
								{
									label: "Chatbots",
									collapsed: true,
									items: [
										{ label: "Overview", slug: "topics/chatbots/overview" },
									],
								},
								{
									label: "Data Pipelines",
									collapsed: true,
									items: [
										{
											label: "Overview",
											slug: "topics/data-pipelines/overview",
										},
									],
								},
								{
									label: "Business Intelligence",
									collapsed: true,
									items: [
										{
											label: "Overview",
											slug: "topics/business-intelligence/overview",
										},
									],
								},
								{
									label: "Coming From",
									collapsed: true,
									items: [
										{ label: "UiPath", slug: "topics/coming-from/uipath" },
										{
											label: "LangChain",
											slug: "topics/coming-from/langchain",
										},
										{
											label: "Developers",
											slug: "topics/coming-from/developers",
										},
										{ label: "n8n", slug: "topics/coming-from/n8n" },
										{
											label: "Unreal Blueprints",
											slug: "topics/coming-from/unreal",
										},
									],
								},
							],
						},
					],
				},
				// ===== DEVOPS / ADMINS =====
				{
					label: "Self Hosting",
					badge: { text: "DevOps", variant: "caution" },
					items: [
						{ label: "Overview", slug: "self-hosting/overview" },
						{
							label: "Execution Backends",
							slug: "self-hosting/execution-backends",
						},
						{ label: "Desktop Client", slug: "self-hosting/desktop-client" },
						{
							label: "Docker Compose",
							collapsed: true,
							items: [
								{
									autogenerate: {
										directory: "self-hosting/docker-compose",
										collapsed: true,
									},
								},
							],
						},
						{
							label: "Kubernetes",
							collapsed: true,
							items: [
								{
									autogenerate: {
										directory: "self-hosting/kubernetes",
										collapsed: true,
									},
								},
							],
						},
					],
				},
				// ===== EXTENSION DEVELOPERS (WASM) =====
				{
					label: "Extending Flow-Like",
					badge: { text: "Devs", variant: "success" },
					items: [
						{ label: "WASM Nodes Overview", slug: "dev/wasm-nodes/overview" },
						{
							label: "Sandboxing & Permissions",
							slug: "dev/wasm-nodes/sandboxing",
						},
						{ label: "Manifest Format", slug: "dev/wasm-nodes/manifest" },
						{
							label: "Publishing to Registry",
							slug: "dev/wasm-nodes/registry",
						},
						{
							label: "Language SDKs",
							collapsed: true,
							items: [
								{ label: "Rust", slug: "dev/wasm-nodes/rust" },
								{ label: "Go", slug: "dev/wasm-nodes/go" },
								{ label: "TypeScript", slug: "dev/wasm-nodes/typescript" },
								{ label: "Python", slug: "dev/wasm-nodes/python" },
								{ label: "C++", slug: "dev/wasm-nodes/cpp" },
								{ label: "Zig", slug: "dev/wasm-nodes/zig" },
								{ label: "Swift", slug: "dev/wasm-nodes/swift" },
								{ label: "C#", slug: "dev/wasm-nodes/csharp" },
								{
									label: "AssemblyScript",
									slug: "dev/wasm-nodes/assemblyscript",
								},
								{ label: "Java", slug: "dev/wasm-nodes/java" },
								{ label: "Kotlin", slug: "dev/wasm-nodes/kotlin" },
								{ label: "Lua", slug: "dev/wasm-nodes/lua" },
								{ label: "Grain", slug: "dev/wasm-nodes/grain" },
								{ label: "MoonBit", slug: "dev/wasm-nodes/moonbit" },
								{ label: "Nim", slug: "dev/wasm-nodes/nim" },
							],
						},
						{
							label: "Client SDKs",
							collapsed: false,
							items: [
								{ label: "Overview", slug: "dev/sdks/overview" },
								{ label: "Node.js / TypeScript", slug: "dev/sdks/nodejs" },
								{ label: "Python", slug: "dev/sdks/python" },
							],
						},
						{
							label: "A2UI Development",
							collapsed: true,
							items: [
								{ autogenerate: { directory: "dev/a2ui", collapsed: true } },
							],
						},
						{
							label: "Event Sinks",
							collapsed: true,
							items: [
								{ autogenerate: { directory: "dev/sinks", collapsed: true } },
							],
						},
					],
				},
				// ===== CORE CONTRIBUTORS =====
				{
					label: "Contributing",
					badge: { text: "Core", variant: "danger" },
					items: [
						{ label: "Architecture", slug: "dev/architecture" },
						{ label: "Building from Source", slug: "dev/build" },
						{ label: "Contributing Guide", slug: "dev/contribute" },
						{ label: "Writing Native Nodes", slug: "dev/writing-nodes" },
						{ label: "Rust SDK", slug: "dev/rust" },
						{ label: "Storage Providers", slug: "dev/storage-providers" },
						{ label: "Customization", slug: "dev/customizing" },
						{ label: "Translations", slug: "dev/translations" },
					],
				},
				// ===== ENTERPRISE =====
				{
					label: "Enterprise",
					collapsed: true,
					items: [
						{ autogenerate: { directory: "enterprise", collapsed: true } },
					],
				},
				// ===== REFERENCE =====
				{
					label: "Reference",
					collapsed: true,
					items: [
						{ label: "Security Architecture", slug: "reference/security" },
						{ label: "Benchmarks", slug: "reference/benchmarks" },
						{ label: "Dates & Times", slug: "reference/dates" },
						{
							label: "Markdown Formatting",
							slug: "reference/markdown-formatting",
						},
						{ label: "A2UI Components", slug: "reference/a2ui-components" },
						{ label: "Widget Builder", slug: "reference/widget-builder" },
						{ label: "FlowPilot UI", slug: "reference/flowpilot-ui" },
						{ label: "A2UI Migration", slug: "reference/a2ui-migration" },
					],
				},
				{
					label: "Node Catalog",
					collapsed: true,
					items: [
						{ label: "Overview", slug: "nodes/overview" },
						...generatedNodeSidebar,
					],
				},
			],
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
