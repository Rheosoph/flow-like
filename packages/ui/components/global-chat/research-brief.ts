/** Mirrors the shared Rust research_agent_prompt contract. */
export function researchAgentPrompt(value: unknown): string {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error("research_agent expects an object with a question string");
	}
	const args = value as Record<string, unknown>;
	if (
		Object.keys(args).some((key) => key !== "question" && key !== "context")
	) {
		throw new Error(
			"research_agent accepts only question and optional context",
		);
	}
	if (typeof args.question !== "string" || !args.question.trim()) {
		throw new Error(
			"research_agent requires a nonempty question that resolves references from the conversation",
		);
	}
	if (args.context !== undefined && typeof args.context !== "string") {
		throw new Error("research_agent context must be a string when supplied");
	}
	const question = args.question.trim();
	const context = typeof args.context === "string" ? args.context.trim() : "";
	return context
		? `Research question:\n${question}\n\nRelevant public context:\n${context}`
		: `Research question:\n${question}`;
}
