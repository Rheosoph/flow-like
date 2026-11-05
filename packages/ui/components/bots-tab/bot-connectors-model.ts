import type {
	BotConnectorPolicy,
	IBotConnector,
	IBotConnectorTool,
} from "../../state/backend-state/bot-state";

export const CONNECTOR_POLICIES: readonly BotConnectorPolicy[] = [
	"read",
	"ask",
	"never",
];

/** What a tool does without an explicit choice: the server's hints, else the connector's default. */
export function defaultToolPolicy(
	tool: IBotConnectorTool,
	connectorDefault: BotConnectorPolicy,
): BotConnectorPolicy {
	if (tool.destructive_hint) return "ask";
	if (tool.read_only_hint) return "read";
	return connectorDefault;
}

/** The policy the bot runs with for one tool, and whether the owner chose it. */
export function toolPolicy(
	connector: Pick<IBotConnector, "tool_policies" | "default_policy">,
	tool: IBotConnectorTool,
): { policy: BotConnectorPolicy; chosen: boolean } {
	const chosen = connector.tool_policies[tool.name];
	if (chosen) return { policy: chosen, chosen: true };
	return {
		policy: defaultToolPolicy(tool, connector.default_policy),
		chosen: false,
	};
}

/** The tool's schema changed since the owner set or confirmed its policy: it asks every time. */
export function isDrifted(
	connector: Pick<IBotConnector, "confirmed_hashes">,
	tool: IBotConnectorTool,
): boolean {
	const confirmed = connector.confirmed_hashes?.[tool.name];
	return Boolean(
		confirmed && tool.schema_hash && confirmed !== tool.schema_hash,
	);
}

/** A tool that changes things must not run unasked without the owner knowing it. */
export function readsWriteTool(
	tool: IBotConnectorTool,
	policy: BotConnectorPolicy,
): boolean {
	return policy === "read" && !tool.read_only_hint;
}

/** Account connectors have two pseudo tools: GET and HEAD, and every other method. */
export const ACCOUNT_POLICY_KEYS = ["read", "write"] as const;

export function accountPolicy(
	connector: Pick<IBotConnector, "tool_policies">,
	key: (typeof ACCOUNT_POLICY_KEYS)[number],
): { policy: BotConnectorPolicy; chosen: boolean } {
	const chosen = connector.tool_policies[key];
	if (chosen) return { policy: chosen, chosen: true };
	return { policy: key === "read" ? "read" : "ask", chosen: false };
}

/** The policies after the owner changed one; choosing the default again removes the entry. */
export function withToolPolicy(
	current: Record<string, BotConnectorPolicy>,
	name: string,
	policy: BotConnectorPolicy,
): Record<string, BotConnectorPolicy> {
	return { ...current, [name]: policy };
}

/** Wire format of an MCP URL typed by the owner: trimmed, with a scheme. */
export function normalizeMcpUrl(input: string): string {
	const trimmed = input.trim();
	if (!trimmed) return "";
	return /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)
		? trimmed
		: `https://${trimmed}`;
}
