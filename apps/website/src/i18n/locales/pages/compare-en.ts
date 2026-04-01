export const enCompare = {
	// Meta
	"compare.meta.title": "Flow-Like vs Alternatives | Full Comparison",
	"compare.meta.description":
		"See how Flow-Like compares to Zapier, n8n, Retool, Power Apps, Airflow, and more. The only platform that combines workflow automation with app building.",

	// Hero
	"compare.hero.tagline": "The Full Picture",
	"compare.hero.headline": "How does Flow-Like",
	"compare.hero.headline.highlight": "compare?",
	"compare.hero.description":
		"Most tools make you choose: automate OR build apps. Flow-Like is the only platform that does both—with the performance and governance enterprises demand.",
	"compare.hero.cta": "Try Flow-Like free",
	"compare.hero.cta.demo": "See it in action",

	// Category Headers
	"compare.category.execution": "Execution Engines",
	"compare.category.execution.desc":
		"Tools designed to run workflows and automate tasks. Great at connecting systems, but limited when you need to build actual applications.",
	"compare.category.lowcode": "Low-Code App Builders",
	"compare.category.lowcode.desc":
		"Tools for building internal apps and dashboards. Great for UIs, but limited automation and workflow capabilities.",
	"compare.category.orchestration": "Data & Workflow Orchestration",
	"compare.category.orchestration.desc":
		"Enterprise-grade tools for complex data pipelines and workflow orchestration. Powerful but require engineering expertise.",

	// Legend
	"compare.legend.native": "Native support",
	"compare.legend.partial": "Partial / Add-on",
	"compare.legend.none": "Not supported",

	// Capability Labels
	"compare.cap.visual_workflow": "Visual workflow builder",
	"compare.cap.replayable": "Replayable Execution",
	"compare.cap.high_volume": "High-volume dataflows",
	"compare.cap.compiled": "Compiled business logic",
	"compare.cap.ai_agents": "AI agents built-in",
	"compare.cap.ui_builder": "End-user UI builder",
	"compare.cap.full_apps": "Ship full applications",
	"compare.cap.customer_facing": "Customer-facing apps",
	"compare.cap.desktop": "Desktop applications",
	"compare.cap.mobile": "Mobile applications",
	"compare.cap.offline": "Offline execution",
	"compare.cap.local_first": "Local-first architecture",
	"compare.cap.file_native": "File-native state",
	"compare.cap.data_science": "Data-science workflows",
	"compare.cap.governance": "Governance & RBAC",
	"compare.cap.self_hosted": "Self-hosted option",
	"compare.cap.lock_in": "No vendor lock-in",

	// Flow-Like Values
	"compare.fl.visual_workflow": "Typed IR",
	"compare.fl.high_volume": "255k/s",
	"compare.fl.compiled": "Rust",
	"compare.fl.desktop": "Tauri",
	"compare.fl.file_native": "Object store",
	"compare.fl.governance": "Graph + data scoped",
	"compare.fl.lock_in": "None",

	// Capability Explanations
	"compare.explain.visual_workflow.title": "Visual Workflow Builder",
	"compare.explain.visual_workflow.what":
		"Design automations by connecting nodes on a canvas instead of writing code.",
	"compare.explain.visual_workflow.flow":
		"Flow-Like uses a typed intermediate representation (IR). Every connection knows what data types it can accept, preventing errors before they happen. It's like having a spell-checker for your automations.",

	"compare.explain.replayable.title": "Replayable Execution",
	"compare.explain.replayable.what":
		"Re-run any past execution with identical results for debugging and audits.",
	"compare.explain.replayable.flow":
		"Flow-Like records every execution with full event sourcing. Replay any run with the exact same data and get identical results—essential for debugging production issues and compliance audits.",

	"compare.explain.high_volume.title": "High-Volume Dataflows",
	"compare.explain.high_volume.what":
		"Process thousands of events per second without breaking a sweat.",
	"compare.explain.high_volume.flow":
		"Flow-Like's Rust engine benchmarks at 255,000 events per second on standard hardware. That's 1,000× faster than typical workflow engines—and proportionally cheaper on cloud bills.",

	"compare.explain.compiled.title": "Compiled Business Logic",
	"compare.explain.compiled.what":
		"Your automation logic runs as native, optimized code—not interpreted scripts.",
	"compare.explain.compiled.flow":
		"Flow-Like compiles workflows to native Rust code. No JavaScript interpreter overhead, no cold starts. Your automations run at near-hardware speed.",

	"compare.explain.ai_agents.title": "AI Agents Built-In",
	"compare.explain.ai_agents.what":
		"Orchestrate LLMs and AI models as first-class workflow participants.",
	"compare.explain.ai_agents.flow":
		"Flow-Like has native AI agent nodes with guardrails, rate limits, and full logging. Build RAG pipelines, chatbots, or autonomous agents with complete observability—no plugins required.",

	"compare.explain.ui_builder.title": "End-User UI Builder",
	"compare.explain.ui_builder.what":
		"Create interfaces for your automations without writing frontend code.",
	"compare.explain.ui_builder.flow":
		"Flow-Like's Interface Builder lets you design modern, interactive UIs that connect directly to your workflows. Build forms, dashboards, and full applications visually.",

	"compare.explain.full_apps.title": "Ship Full Applications",
	"compare.explain.full_apps.what":
		"Package your automation + UI as a standalone, deployable application.",
	"compare.explain.full_apps.flow":
		"Export your Flow-Like project as a complete application. Deploy to web, desktop, or mobile. No separate frontend deployment needed.",

	"compare.explain.customer_facing.title": "Customer-Facing Apps",
	"compare.explain.customer_facing.what":
		"Build apps your customers can use, not just internal tools.",
	"compare.explain.customer_facing.flow":
		"Flow-Like apps can be fully branded and deployed publicly. Build customer portals, SaaS products, or public-facing tools with the same platform.",

	"compare.explain.desktop.title": "Desktop Applications",
	"compare.explain.desktop.what":
		"Run your automations as native desktop software.",
	"compare.explain.desktop.flow":
		"Flow-Like uses Tauri to compile to native desktop apps for Windows, macOS, and Linux. Your users get a fast, native experience with full offline capability.",

	"compare.explain.mobile.title": "Mobile Applications",
	"compare.explain.mobile.what": "Deploy to iOS and Android devices.",
	"compare.explain.mobile.flow":
		"Build once, deploy everywhere. Flow-Like compiles to mobile apps that work offline and sync when connected—perfect for field workers and mobile teams.",

	"compare.explain.offline.title": "Offline Execution",
	"compare.explain.offline.what":
		"Automations keep running even without internet access.",
	"compare.explain.offline.flow":
		"Flow-Like stores everything locally first. Your workflows execute offline, queue changes, and sync automatically when connectivity returns. No cloud dependency.",

	"compare.explain.local_first.title": "Local-First Architecture",
	"compare.explain.local_first.what":
		"Data lives on your device by default, not in someone else's cloud.",
	"compare.explain.local_first.flow":
		"Your projects, data, and executions stay on your hardware. Choose when and what to sync. Full control over your data residency.",

	"compare.explain.file_native.title": "File-Native State",
	"compare.explain.file_native.what":
		"Project state stored as regular files you can version, backup, and inspect.",
	"compare.explain.file_native.flow":
		"Flow-Like uses an object store format. Your entire project is portable files—put them in Git, Dropbox, or any backup system. No proprietary database required.",

	"compare.explain.data_science.title": "Data-Science Workflows",
	"compare.explain.data_science.what":
		"Process datasets, run ML models, and build analytics pipelines.",
	"compare.explain.data_science.flow":
		"Native support for data transformations, ML model inference, and visualization. Connect to data lakes, run pandas-style operations, and output to any format.",

	"compare.explain.governance.title": "Governance & RBAC",
	"compare.explain.governance.what":
		"Control who can see, edit, and run what—down to individual data fields.",
	"compare.explain.governance.flow":
		"Flow-Like's permission system is graph-aware and data-scoped. Grant access to specific nodes, workflows, or even data fields. Every action is logged for audit trails.",

	"compare.explain.self_hosted.title": "Self-Hosted Option",
	"compare.explain.self_hosted.what":
		"Run everything on your own infrastructure.",
	"compare.explain.self_hosted.flow":
		"Deploy Flow-Like on-prem, in your private cloud, or fully air-gapped. Your data never has to leave your infrastructure. Zero external dependencies.",

	"compare.explain.lock_in.title": "No Vendor Lock-In",
	"compare.explain.lock_in.what":
		"How difficult is it to leave if you need to?",
	"compare.explain.lock_in.flow":
		"Flow-Like exports to standard formats. Your workflows are portable files, not locked in a SaaS database. Switch providers or self-host anytime without data loss.",

	// Insight Section
	"compare.insight.tagline": "The Key Insight",
	"compare.insight.headline": "Other tools make you choose",
	"compare.insight.description":
		"n8n, Node-RED, Zapier, Airflow, Temporal are execution engines. Retool, Power Apps, Appsmith are UI shells. Flow-Like is the only system that is both—and can ship the result as a real application.",

	// CTA Section
	"compare.cta.tagline": "Ready to see the difference?",
	"compare.cta.headline": "Try Flow-Like today",
	"compare.cta.description":
		"Download the desktop app and start building. No credit card required. No cloud signup needed.",
	"compare.cta.download": "Download free",
	"compare.cta.enterprise": "Enterprise demo",
	"compare.cta.note": "Works offline. Runs on your hardware.",

	// Competitor Descriptions
	"compare.competitor.zapier":
		"The most popular integration platform. Great for simple automations, but limited customization and high costs at scale.",
	"compare.competitor.n8n":
		"Open-source workflow automation. Self-hostable and flexible, but JavaScript-based with no native app building.",
	"compare.competitor.nodered":
		"IoT-focused flow programming. Excellent for hardware projects, limited for business applications.",
	"compare.competitor.retool":
		"Fast internal tool builder. Great for admin panels, but limited automation and SaaS-dependent.",
	"compare.competitor.powerapps":
		"Microsoft's low-code platform. Deep Office 365 integration, but high lock-in and limited outside Microsoft ecosystem.",
	"compare.competitor.superblocks":
		"Modern internal tool builder. Clean interface, but limited workflow capabilities and primarily cloud-based.",
	"compare.competitor.appsmith":
		"Open-source app builder. Self-hostable and customizable, but focused on internal tools only.",
	"compare.competitor.airflow":
		"Industry-standard workflow orchestration. Powerful for data pipelines, but requires Python expertise and no UI building.",
	"compare.competitor.temporal":
		"Durable execution platform. Excellent for complex workflows, but developer-focused with steep learning curve.",

	// Capability Deep Dive
	"compare.explain.how_does_it": "How Flow-Like does it",
};
