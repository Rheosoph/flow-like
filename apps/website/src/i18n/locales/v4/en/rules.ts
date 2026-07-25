export default {
	"v4.rules.kicker": "Layer 6 · The rules",
	"v4.rules.headline":
		"Every part asks permission. Every action leaves a record.",
	"v4.rules.body":
		"Before any building block can reach the network, your files, or an AI model, it has to say so — and the system enforces that while it runs, not on paper. Every AI request is recorded with what was asked, what was answered, and what it cost. When the auditor asks what happened on March 12th, the answer is a report, not a meeting. And because the full source is available under BSL 1.1, your own experts — or your regulator’s — can inspect exactly how it works instead of taking our word for it.",
	"v4.rules.progress": "checks passed",
	"v4.rules.progressDone": "All clear",
	"v4.rules.dialog.aria":
		"Example of a permission dialog shown before a building block runs",
	"v4.rules.dialog.title": "This block asks to use the internet",
	"v4.rules.dialog.body":
		"“Fetch supplier prices” wants to call an outside web address.",
	"v4.rules.dialog.grant": "NetworkHttp",
	"v4.rules.dialog.deny": "Deny",
	"v4.rules.dialog.allow": "Allow",
	"v4.rules.dialog.scoresLabel": "Quality scores",
	"v4.rules.score.privacy": "Privacy",
	"v4.rules.score.security": "Security",
	"v4.rules.score.performance": "Performance",
	"v4.rules.score.governance": "Governance",
	"v4.rules.score.reliability": "Reliability",
	"v4.rules.score.cost": "Cost",
	"v4.rules.item1.title": "Permission by permission",
	"v4.rules.item1.desc":
		"Each block asks before it touches anything; the system enforces the answer.",
	"v4.rules.item2.title": "Quality scores",
	"v4.rules.item2.desc":
		"Six ratings on every building block, visible before you rely on it — reviewed before deploy, not after the incident.",
	"v4.rules.item3.title": "A complete record of every run",
	"v4.rules.item3.desc":
		"Who ran what, what it read, what it decided, what it cost — permanent and exportable.",
	"v4.rules.item4.title": "Every ingredient listed",
	"v4.rules.item4.desc":
		"A published list of every piece of software inside each release.",
	"v4.rules.item5.title": "Your data stays under your law",
	"v4.rules.item5.desc":
		"Built in Germany, EU jurisdiction, runs fully disconnected where required.",
	"v4.rules.item6.title": "Compliance, stated honestly",
	"v4.rules.item6.desc":
		"SOC 2 aligned, TISAX controls, continuously checked — posture, never certification claims.",
	"v4.rules.uth":
		"WASM enforces declared permissions at runtime; six node scores, per-call model logs, a release SBOM and zero telemetry make the record inspectable.",
} as const;
