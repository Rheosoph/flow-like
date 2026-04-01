export const enDevelopers: Record<string, string> = {
	// Meta
	"dev.meta.title": "For Developers | Flow-Like",
	"dev.meta.description":
		"Two paths to building with Flow-Like: compose workflows visually with existing nodes, or write custom nodes in 15+ languages compiled to WebAssembly.",

	// Hero
	"dev.hero.badge": "Open Source",
	"dev.hero.headline.prefix": "For",
	"dev.hero.headline.highlight": "Developers",
	"dev.hero.description":
		"Whether you build workflows visually or write custom nodes in your language of choice — Flow-Like gives you two clear paths to ship production-grade automation.",
	"dev.hero.cio.prefix": "Looking for the executive overview?",
	"dev.hero.cio.link": "For CIOs →",
	"dev.hero.card.workflows.title": "Compose Workflows",
	"dev.hero.card.workflows.description":
		"Drag, connect, deploy — no code required",
	"dev.hero.card.nodes.title": "Write Custom Nodes",
	"dev.hero.card.nodes.description": "15+ languages, compiled to WebAssembly",
	"dev.hero.converge": "Ship to Production",

	// Path Picker
	"dev.pathpicker.label": "Choose your path",

	// Workflow Path
	"dev.workflow.badge": "Path 2",
	"dev.workflow.headline": "Compose Workflows Visually",
	"dev.workflow.description":
		"Use the visual editor to drag and connect pre-built nodes into powerful automation flows. No code needed — just connect, configure, and deploy.",
	"dev.workflow.feature.dragdrop.title": "Drag & Drop Builder",
	"dev.workflow.feature.dragdrop.description":
		"The visual canvas lets you build complex data and automation pipelines by wiring nodes together. Every connection is type-checked in real time.",
	"dev.workflow.feature.catalog.title": "Rich Node Catalog",
	"dev.workflow.feature.catalog.description":
		"Hundreds of pre-built nodes for databases, APIs, AI/ML, file operations, messaging, and more — ready to drop into your workflow.",
	"dev.workflow.feature.templates.title": "Templates & Sharing",
	"dev.workflow.feature.templates.description":
		"Start from templates or publish your own. Share flows across teams with versioning, so everyone benefits from proven patterns.",
	"dev.workflow.feature.interfaces.title": "Build Interfaces",
	"dev.workflow.feature.interfaces.description":
		"Create custom UIs with the built-in interface editor. Turn any workflow into an interactive app your team can use directly.",
	"dev.workflow.feature.vcs.title": "Version Control",
	"dev.workflow.feature.vcs.description":
		"Every flow is serializable and diffable. Store workflows in Git, review changes in PRs, and roll back confidently.",
	"dev.workflow.feature.typesafe.title": "Type-Safe Execution",
	"dev.workflow.feature.typesafe.description":
		"Inputs and outputs are validated at compile time. Catch mismatches before deployment, not in production.",
	"dev.workflow.howitworks": "How it works",
	"dev.workflow.step1.title": "Open the visual canvas",
	"dev.workflow.step1.description": "In the desktop app or web studio",
	"dev.workflow.step2.title": "Drag nodes from the catalog",
	"dev.workflow.step2.description": "Search, filter, or browse categories",
	"dev.workflow.step3.title": "Connect & configure",
	"dev.workflow.step3.description":
		"Type-safe wiring with real-time validation",
	"dev.workflow.step4.title": "Run or deploy",
	"dev.workflow.step4.description": "Local, self-hosted, or cloud",
	"dev.workflow.cta.docs": "Read the Docs",
	"dev.workflow.cta.download": "Download Flow-Like",

	// Custom Nodes Path
	"dev.nodes.divider": "or",
	"dev.nodes.badge": "Path 1",
	"dev.nodes.headline": "Write Custom Nodes",
	"dev.nodes.description":
		"Extend the engine with your own logic. Write a node in any supported language — it compiles to WebAssembly and runs sandboxed with full access to the host SDK (logging, storage, HTTP, AI models, and more).",
	"dev.nodes.languages.title": "Supported Languages",
	"dev.nodes.languages.description":
		"Every language ships with a project template and SDK. Pick one and start building.",
	"dev.nodes.languages.sdk": "Full SDK",
	"dev.nodes.sdk.title": "Host SDK Capabilities",
	"dev.nodes.sdk.description":
		"Every node gets sandboxed access to these platform APIs through the SDK:",
	"dev.nodes.sdk.logging": "Logging",
	"dev.nodes.sdk.logging.desc": "Structured log output",
	"dev.nodes.sdk.pins": "Pins",
	"dev.nodes.sdk.pins.desc": "Read & write node I/O",
	"dev.nodes.sdk.variables": "Variables",
	"dev.nodes.sdk.variables.desc": "Flow-scoped state",
	"dev.nodes.sdk.cache": "Cache",
	"dev.nodes.sdk.cache.desc": "Persistent KV cache",
	"dev.nodes.sdk.metadata": "Metadata",
	"dev.nodes.sdk.metadata.desc": "Node & flow info",
	"dev.nodes.sdk.streaming": "Streaming",
	"dev.nodes.sdk.streaming.desc": "Stream large data",
	"dev.nodes.sdk.storage": "Storage",
	"dev.nodes.sdk.storage.desc": "File & blob storage",
	"dev.nodes.sdk.ai": "AI Models",
	"dev.nodes.sdk.ai.desc": "LLM & embedding calls",
	"dev.nodes.sdk.http": "HTTP",
	"dev.nodes.sdk.http.desc": "Outbound HTTP requests",
	"dev.nodes.sdk.auth": "Auth",
	"dev.nodes.sdk.auth.desc": "Secrets & credentials",
	"dev.nodes.feature.sandbox.title": "Sandboxed & Safe",
	"dev.nodes.feature.sandbox.description":
		"Nodes run in a WebAssembly sandbox with capability-based permissions. No filesystem or network access unless explicitly granted.",
	"dev.nodes.feature.test.title": "Test & Iterate",
	"dev.nodes.feature.test.description":
		"Every template comes with a test harness. Write fixtures, run locally, and validate before publishing to your catalog.",
	"dev.nodes.feature.publish.title": "Publish & Version",
	"dev.nodes.feature.publish.description":
		"Package your node with metadata, version it semantically, and share it privately or to the public catalog.",
	"dev.nodes.quickstart.title": "Quick Start",
	"dev.nodes.quickstart.description":
		"Open Flow-Like Studio, pick a language template, and start building your custom node:",
	"dev.nodes.quickstart.step1":
		"1. Open Flow-Like Studio and navigate to the Node Developer section",
	"dev.nodes.quickstart.step2":
		"2. Pick a language template from 15+ supported languages",
	"dev.nodes.quickstart.step3":
		"3. Implement your node logic and build the WASM binary",
	"dev.nodes.quickstart.step4":
		"4. Publish directly to your catalog from the Studio",
	"dev.nodes.cta.guide": "Node Development Guide",
	"dev.nodes.cta.templates": "Browse All Templates",

	// CTA
	"dev.cta.headline": "Ready to build?",
	"dev.cta.description":
		"Whether you compose visually or code custom nodes — Flow-Like is open source, local-first, and built for developers who want full control over their automation stack.",
	"dev.cta.download": "Download Flow-Like",
	"dev.cta.github": "View on GitHub",
};
