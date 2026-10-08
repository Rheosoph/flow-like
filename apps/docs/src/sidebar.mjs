import { generatedNodeSidebar } from "./generated/node-sidebar.mjs";

export const sidebar = [
	{
		label: "Start",
		items: [
			{
				label: "Quick Start",
				slug: "start/getting-started",
			},
			{
				label: "What is Flow-Like?",
				slug: "start/what-is-flow-like",
			},
			{
				label: "Download & Install",
				slug: "start/get",
			},
			{
				label: "Linux Troubleshooting",
				slug: "start/linux-troubleshooting",
			},
			{
				label: "First Steps",
				slug: "start/first-use",
			},
			{
				label: "Developer Mode",
				slug: "start/developer-mode",
			},
			{
				label: "Login & Accounts",
				slug: "start/login",
			},
			{
				label: "AI Models",
				slug: "start/models",
			},
			{
				label: "Profiles",
				slug: "start/profiles",
			},
			{
				label: "Customize your Home",
				slug: "start/home",
			},
			{
				label: "Explore",
				slug: "start/explore",
			},
			{
				label: "Get Support",
				slug: "start/support",
			},
		],
		collapsed: true,
	},
	{
		label: "Build Apps",
		collapsed: true,
		items: [
			{
				label: "Apps",
				collapsed: true,
				items: [
					{
						label: "Overview",
						slug: "apps/overview",
					},
					{
						label: "Creating Apps",
						slug: "apps/create",
					},
					{
						label: "Boards & Flows",
						slug: "apps/boards",
					},
					{
						label: "Runtime Variables",
						slug: "apps/runtime-variables",
					},
					{
						label: "Pages",
						slug: "apps/pages",
					},
					{
						label: "Routes",
						slug: "apps/routes",
					},
					{
						label: "Widgets",
						slug: "apps/widgets",
					},
					{
						label: "Chat UI",
						slug: "apps/chat-ui",
					},
					{
						label: "Custom UI (A2UI)",
						slug: "apps/a2ui",
					},
					{
						label: "Events",
						slug: "apps/events",
					},
					{
						label: "Templates",
						slug: "apps/templates",
					},
					{
						label: "Storage",
						slug: "apps/storage",
					},
					{
						label: "Sharing",
						slug: "apps/share",
					},
					{
						label: "Audit Trail & Export",
						slug: "apps/audit-trail",
					},
					{
						label: "Offline & Online",
						slug: "apps/offline-online",
					},
					{
						label: "Offline access",
						slug: "apps/offline-access",
					},
					{
						label: "Release and roll back Events",
						slug: "apps/event-releases",
					},
				],
			},
			{
				label: "Studio",
				collapsed: true,
				items: [
					{
						label: "Overview",
						slug: "studio/overview",
					},
					{
						label: "FlowPilot AI",
						slug: "studio/flowpilot",
					},
					{
						label: "Claude Code & Codex Setup",
						slug: "studio/flowpilot-external-agents",
					},
					{
						label: "FlowScript",
						slug: "studio/flowscript",
					},
					{
						label: "Working with Nodes",
						slug: "studio/nodes",
					},
					{
						label: "Connecting Pins",
						slug: "studio/connecting",
					},
					{
						label: "Layers & Organization",
						slug: "studio/layers",
					},
					{
						label: "Variables",
						slug: "studio/variables",
					},
					{
						label: "Local-Only Execution",
						slug: "studio/local-execution",
					},
					{
						label: "Logging & Debugging",
						slug: "studio/logging",
					},
					{
						label: "Version Control",
						slug: "studio/versioning",
					},
				],
			},
			{
				label: "Data Studio",
				collapsed: true,
				items: [
					{
						label: "Overview",
						slug: "apps/data-studio",
					},
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
				label: "Packages & Extensions",
				collapsed: true,
				items: [
					{
						label: "Package Store",
						slug: "start/packages-store",
					},
					{
						label: "Packages",
						slug: "start/packages-library",
					},
				],
			},
		],
	},
	{
		label: "Node Catalog",
		collapsed: true,
		items: [
			{ label: "Browse all nodes", slug: "nodes/overview" },
			...generatedNodeSidebar.map((group) => ({
				label: group.label,
				slug: group.items[0].slug,
			})),
		],
	},
	{
		label: "Guides & Integrations",
		collapsed: true,
		items: [
			{
				label: "GenAI",
				collapsed: true,
				items: [
					{
						label: "Overview",
						slug: "topics/genai/overview",
					},
					{
						label: "AI Models & Setup",
						slug: "topics/genai/models",
					},
					{
						label: "Chat & Conversations",
						slug: "topics/genai/chat",
					},
					{
						label: "RAG & Knowledge Bases",
						slug: "topics/genai/rag",
					},
					{
						label: "AI Agents",
						slug: "topics/genai/agents",
					},
					{
						label: "Extraction & Structured Output",
						slug: "topics/genai/extraction",
					},
					{
						label: "Prompt Templates",
						slug: "topics/genai/prompt-templates",
					},
					{
						label: "Local Diffusion",
						slug: "topics/genai/local-diffusion",
					},
				],
			},
			{
				label: "Data Science",
				collapsed: true,
				items: [
					{
						label: "Overview",
						slug: "topics/datascience/overview",
					},
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
				label: "Internal Tools",
				slug: "topics/internal-tools/overview",
			},
			{
				label: "Desktop Automation",
				slug: "topics/desktop-automation/overview",
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
						label: "API Integrations",
						slug: "topics/api-integrations/overview",
					},
					{
						label: "Microsoft Teams",
						slug: "topics/api-integrations/teams",
					},
					{
						label: "Industrial Protocols",
						slug: "topics/api-integrations/industrial-protocols",
					},
				],
			},
			{
				label: "Chatbots",
				slug: "topics/chatbots/overview",
			},
			{
				label: "Data Pipelines",
				slug: "topics/data-pipelines/overview",
			},
			{
				label: "Business Intelligence",
				slug: "topics/business-intelligence/overview",
			},
			{
				label: "Coming From",
				collapsed: true,
				items: [
					{
						label: "UiPath",
						slug: "topics/coming-from/uipath",
					},
					{
						label: "LangChain",
						slug: "topics/coming-from/langchain",
					},
					{
						label: "Developers",
						slug: "topics/coming-from/developers",
					},
					{
						label: "n8n",
						slug: "topics/coming-from/n8n",
					},
					{
						label: "Unreal Blueprints",
						slug: "topics/coming-from/unreal",
					},
				],
			},
			{
				label: "Native Integrations",
				collapsed: true,
				items: [
					{
						label: "Siri, Shortcuts and Handoff",
						slug: "apps/native-integrations",
					},
					{
						label: "Native widgets",
						slug: "apps/native-widgets",
					},
				],
			},
		],
	},
	{
		label: "Operate",
		badge: {
			text: "DevOps",
			variant: "caution",
		},
		items: [
			{
				label: "Overview",
				slug: "self-hosting/overview",
			},
			{
				label: "Device Deployment",
				collapsed: true,
				items: [
					{ label: "Overview", slug: "devices" },
					{ label: "Set Up a Device", slug: "devices/setup" },
					{ label: "Deploy and Operate", slug: "devices/deployments" },
					{ label: "Host Models", slug: "devices/models" },
					{
						label: "Service Access & Port Forwarding",
						slug: "devices/service-access",
					},
					{ label: "Security & Networking", slug: "devices/security" },
					{
						label: "Company Edge & Home Automation",
						slug: "devices/use-cases",
					},
				],
			},
			{
				label: "Execution Backends",
				slug: "self-hosting/execution-backends",
			},
			{
				label: "Desktop Client",
				slug: "self-hosting/desktop-client",
			},
			{
				label: "Container Releases",
				slug: "self-hosting/containers",
			},
			{
				label: "Signaling",
				slug: "self-hosting/signaling",
			},
			{
				label: "Audit Trail Storage",
				slug: "self-hosting/audit-trail",
			},
			{
				label: "Local Backend Development",
				slug: "self-hosting/local-development",
			},
			{
				label: "AWS",
				collapsed: true,
				items: [
					{
						autogenerate: {
							directory: "self-hosting/aws",
						},
					},
				],
			},
			{
				label: "Azure",
				collapsed: true,
				items: [
					{
						autogenerate: {
							directory: "self-hosting/azure",
						},
					},
				],
			},
			{
				label: "GCP",
				collapsed: true,
				items: [
					{
						autogenerate: {
							directory: "self-hosting/gcp",
						},
					},
				],
			},
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
			{
				label: "Event Sinks",
				collapsed: true,
				items: [
					{
						autogenerate: {
							directory: "dev/sinks",
							collapsed: true,
						},
					},
				],
			},
			{
				label: "Platform Administration",
				collapsed: true,
				items: [
					{
						label: "Overview",
						slug: "dev/platform-administration",
					},
					{
						autogenerate: {
							directory: "self-hosting/administration",
						},
					},
				],
			},
		],
		collapsed: true,
	},
	{
		label: "Extend",
		badge: {
			text: "Devs",
			variant: "success",
		},
		items: [
			{
				label: "WASM Nodes Overview",
				slug: "dev/wasm-nodes/overview",
			},
			{
				label: "Component Model vs Core Modules",
				slug: "dev/wasm-nodes/runtime-models",
			},
			{
				label: "Sandboxing & Permissions",
				slug: "dev/wasm-nodes/sandboxing",
			},
			{
				label: "Manifest Format",
				slug: "dev/wasm-nodes/manifest",
			},
			{
				label: "Publishing to Registry",
				slug: "dev/wasm-nodes/registry",
			},
			{
				label: "Language SDKs",
				collapsed: true,
				items: [
					{
						label: "Rust",
						slug: "dev/wasm-nodes/rust",
					},
					{
						label: "Go",
						slug: "dev/wasm-nodes/go",
					},
					{
						label: "TypeScript",
						slug: "dev/wasm-nodes/typescript",
					},
					{
						label: "Python",
						slug: "dev/wasm-nodes/python",
					},
					{
						label: "C++",
						slug: "dev/wasm-nodes/cpp",
					},
					{
						label: "Zig",
						slug: "dev/wasm-nodes/zig",
					},
					{
						label: "Swift",
						slug: "dev/wasm-nodes/swift",
					},
					{
						label: "C#",
						slug: "dev/wasm-nodes/csharp",
					},
					{
						label: "AssemblyScript",
						slug: "dev/wasm-nodes/assemblyscript",
					},
					{
						label: "Java",
						slug: "dev/wasm-nodes/java",
					},
					{
						label: "Kotlin",
						slug: "dev/wasm-nodes/kotlin",
					},
					{
						label: "Lua",
						slug: "dev/wasm-nodes/lua",
					},
					{
						label: "Grain",
						slug: "dev/wasm-nodes/grain",
					},
					{
						label: "MoonBit",
						slug: "dev/wasm-nodes/moonbit",
					},
					{
						label: "Nim",
						slug: "dev/wasm-nodes/nim",
					},
				],
			},
			{
				label: "Client SDKs",
				collapsed: true,
				items: [
					{
						label: "Overview",
						slug: "dev/sdks/overview",
					},
					{
						label: "Node.js / TypeScript",
						slug: "dev/sdks/nodejs",
					},
					{
						label: "Python",
						slug: "dev/sdks/python",
					},
				],
			},
			{
				label: "A2UI Development",
				collapsed: true,
				items: [
					{
						autogenerate: {
							directory: "dev/a2ui",
							collapsed: true,
						},
					},
				],
			},
			{
				label: "Package Widgets",
				collapsed: true,
				items: [
					{
						autogenerate: {
							directory: "dev/package-widgets",
						},
					},
				],
			},
		],
		collapsed: true,
	},
	{
		label: "Contribute",
		badge: {
			text: "Core",
			variant: "danger",
		},
		items: [
			{
				label: "Architecture",
				slug: "dev/architecture",
			},
			{
				label: "Building from Source",
				slug: "dev/build",
			},
			{
				label: "Contributing Guide",
				slug: "dev/contribute",
			},
			{
				label: "Writing Native Nodes",
				slug: "dev/writing-nodes",
			},
			{
				label: "Rust SDK",
				slug: "dev/rust",
			},
			{
				label: "Storage Providers",
				slug: "dev/storage-providers",
			},
			{
				label: "Customization",
				slug: "dev/customizing",
			},
			{
				label: "Translations",
				slug: "dev/translations",
			},
			{
				label: "Contributor Tools",
				collapsed: true,
				items: [
					{
						label: "Site Content",
						slug: "dev/documentation-assets",
					},
					{
						label: "Documentation Screenshots",
						slug: "dev/documentation-screenshots",
					},
					{
						label: "University Courses",
						slug: "dev/university-courses",
					},
					{
						label: "FlowPilot Development",
						slug: "dev/flowpilot-development",
					},
				],
			},
		],
		collapsed: true,
	},
	{
		label: "Reference",
		collapsed: true,
		items: [
			{
				label: "Security Architecture",
				slug: "reference/security",
			},
			{
				label: "Benchmarks",
				slug: "reference/benchmarks",
			},
			{
				label: "Dates & Times",
				slug: "reference/dates",
			},
			{
				label: "Markdown Formatting",
				slug: "reference/markdown-formatting",
			},
			{
				label: "A2UI Components",
				slug: "reference/a2ui-components",
			},
			{
				label: "Widget Builder",
				slug: "reference/widget-builder",
			},
			{
				label: "FlowPilot UI",
				slug: "reference/flowpilot-ui",
			},
			{
				label: "A2UI Migration",
				slug: "reference/a2ui-migration",
			},
			{
				label: "Geometry",
				slug: "reference/geometry",
			},
			{
				label: "Home styling",
				slug: "reference/home-styling",
			},
			{
				label: "Enterprise",
				collapsed: true,
				items: [
					{
						autogenerate: {
							directory: "enterprise",
							collapsed: true,
						},
					},
				],
			},
		],
	},
];
