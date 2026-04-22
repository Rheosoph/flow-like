export const enSecurity = {
	// Meta
	"security.meta.title": "Security & Compliance | Flow-Like",
	"security.meta.description":
		"Built on Rust with memory safety, RBAC, full audit trails, data sovereignty, and open-source transparency. Enterprise security without the enterprise complexity.",

	// Hero
	"security.hero.tagline": "Security First",
	"security.hero.headline": "Security You Can",
	"security.hero.headline.highlight": "Verify",
	"security.hero.description":
		"Flow-Like is open source. Every line of code is auditable. Combine memory-safe Rust internals with role-based access control, encryption at rest, and complete audit trails.",
	"security.hero.cta": "View Source Code",
	"security.hero.cta.report": "Report a Vulnerability",

	// Architecture
	"security.arch.tagline": "Security Architecture",
	"security.arch.headline": "Defense in Depth",
	"security.arch.description":
		"Multiple layers of security \u2014 from the language runtime to the deployment boundary.",

	"security.arch.rust.title": "Memory-Safe Runtime",
	"security.arch.rust.desc":
		"The entire execution engine is written in Rust \u2014 eliminating buffer overflows, use-after-free, and data races at compile time. No garbage collector pauses, no runtime surprises.",

	"security.arch.wasm.title": "Sandboxed Extensions",
	"security.arch.wasm.desc":
		"Custom nodes run in WASM sandboxes with capability-based security. No filesystem or network access unless explicitly granted. Malicious code cannot escape the sandbox.",

	"security.arch.rbac.title": "Role-Based Access Control",
	"security.arch.rbac.desc":
		"Granular permissions for workflows, nodes, secrets, and deployments. Assign roles at the organization, team, or project level. Enforce least-privilege by default.",

	"security.arch.encryption.title": "Encryption Everywhere",
	"security.arch.encryption.desc":
		"TLS 1.3 for data in transit. AES-256 encryption at rest for secrets, credentials, and sensitive workflow data. Keys managed via your KMS or ours.",

	"security.arch.audit.title": "Complete Audit Trail",
	"security.arch.audit.desc":
		"Every workflow execution, configuration change, and access event is logged with timestamps, user identity, and full context. Export to your SIEM or compliance tooling.",

	"security.arch.supply.title": "Supply Chain Security",
	"security.arch.supply.desc":
		"All dependencies are tracked with SBOMs. Third-party licenses are audited continuously. Dependency updates are tested in CI before release.",

	// Data Sovereignty
	"security.data.tagline": "Data Sovereignty",
	"security.data.headline": "Your Data, Your Rules",
	"security.data.description":
		"Flow-Like never requires your data to leave your infrastructure. Run on-premise, in your VPC, or on the desktop \u2014 with zero telemetry unless you opt in.",
	"security.data.local.title": "Local-First Architecture",
	"security.data.local.desc":
		"The desktop app works fully offline. No cloud dependency required. Your workflows, data, and secrets stay on your machine.",
	"security.data.selfhost.title": "Self-Hosted Deployment",
	"security.data.selfhost.desc":
		"Deploy Flow-Like in your own cloud or on-premise infrastructure. Docker, Kubernetes, and bare-metal supported.",
	"security.data.residency.title": "Data Residency Controls",
	"security.data.residency.desc":
		"Choose where your data is processed and stored. Meet GDPR, CCPA, and regulatory requirements with deployment-level controls.",

	// Compliance
	"security.compliance.tagline": "Compliance & Transparency",
	"security.compliance.headline": "Built for Regulated Industries",
	"security.compliance.description":
		"From healthcare to finance to government \u2014 Flow-Like provides the controls regulated environments demand.",
	"security.compliance.gdpr.title": "GDPR Ready",
	"security.compliance.gdpr.desc":
		"Data deletion workflows, consent management, and processing records. Request data deletion at any time.",
	"security.compliance.soc.title": "SOC 2 Controls",
	"security.compliance.soc.desc":
		"Access controls, change management, and monitoring aligned with SOC 2 Trust Service Criteria.",
	"security.compliance.open.title": "Open Source Transparency",
	"security.compliance.open.desc":
		"Every dependency, every license, every line of code \u2014 publicly auditable. View the full third-party notice.",
	"security.compliance.sbom.title": "SBOM Available",
	"security.compliance.sbom.desc":
		"Software Bill of Materials generated for every release. Full dependency tree with license and vulnerability data.",

	// CTA
	"security.cta.headline": "Questions About Security?",
	"security.cta.description":
		"Our security team is ready to discuss your requirements. For vulnerability reports, please use our responsible disclosure process.",
	"security.cta.button": "Contact Security Team",
	"security.cta.thirdparty": "View Third-Party Notices",
} as const;
