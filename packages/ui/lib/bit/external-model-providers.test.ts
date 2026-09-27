import { describe, expect, test } from "bun:test";
import {
	discoverExternalModelCatalog,
	externalModelProviders,
	providerFieldValues,
	specificExternalModels,
} from "./external-model-providers";

describe("native model discovery", () => {
	function commandStub(responses: unknown[]) {
		const calls: { command: string; backend: string }[] = [];
		return {
			calls,
			invoke: async <T>(
				command: string,
				args: { backend: string },
			): Promise<T> => {
				calls.push({ command, backend: args.backend });
				const response = responses.shift();
				if (response instanceof Error) throw response;
				return response as T;
			},
		};
	}

	test("an unavailable runtime produces no selectable model or sign-in action", async () => {
		const stub = commandStub([
			{ available: false, message: "Install Claude Code" },
		]);
		expect(
			await discoverExternalModelCatalog("claude-code", stub.invoke),
		).toEqual({
			available: false,
			authenticated: false,
			models: [],
			message: "Install Claude Code",
		});
		expect(stub.calls).toEqual([
			{ command: "flowpilot_agent_backend_status", backend: "claude-code" },
		]);
	});

	test("an installed but signed-out runtime is not advertised as model-ready", async () => {
		const stub = commandStub([
			{ available: true, running: false },
			{ authenticated: false, message: "Sign in to Codex" },
		]);
		expect(await discoverExternalModelCatalog("codex", stub.invoke)).toEqual({
			available: true,
			authenticated: false,
			models: [],
			message: "Sign in to Codex",
		});
		expect(stub.calls.map((call) => call.command)).toEqual([
			"flowpilot_agent_backend_status",
			"flowpilot_agent_backend_get_auth_status",
		]);
	});

	test("a signed-in account exposes only discovered model IDs without FlowPilot prefixes", async () => {
		const stub = commandStub([
			{ available: true },
			{ authenticated: true },
			[
				{ id: "default", name: "Configured default" },
				{ id: "gpt-5.4", name: "GPT 5.4" },
			],
		]);
		expect(await discoverExternalModelCatalog("codex", stub.invoke)).toEqual({
			available: true,
			authenticated: true,
			models: [{ id: "gpt-5.4", name: "GPT 5.4" }],
		});
		expect(stub.calls.map((call) => call.backend)).toEqual([
			"codex",
			"codex",
			"codex",
		]);
	});

	test("discovery failures remain errors rather than invented model choices", async () => {
		const stub = commandStub([
			{ available: true },
			{ authenticated: true },
			new Error("Catalog unavailable"),
		]);
		await expect(
			discoverExternalModelCatalog("codex", stub.invoke),
		).rejects.toThrow("Catalog unavailable");
	});
});

function provider(key: string, native = true) {
	const result = externalModelProviders(native).find(
		(entry) => entry.key === key,
	);
	if (!result) throw new Error(`Missing provider: ${key}`);
	return result;
}

describe("external model connection setup", () => {
	test("browser setup requires a Codex token and excludes Claude Code", () => {
		expect(
			externalModelProviders(false).map((entry) => entry.providerName),
		).toEqual([
			"custom:codex",
			"custom:github-copilot",
			"custom:microsoft-copilot",
		]);
		expect(
			provider("codex", false).fields.find(
				(field) => field.key === "access_token",
			)?.required,
		).toBe(true);
		expect(
			provider("codex", true).fields.find(
				(field) => field.key === "access_token",
			)?.required,
		).toBe(false);
	});

	test("Codex credentials travel as secrets while its exact model and account remain configured", () => {
		const connection = providerFieldValues(provider("codex"), {
			model_id: "  gpt-5.4  ",
			access_token: " fixture-secret ",
			account_id: "fixture-account",
		});
		expect(connection).toEqual({
			modelId: "gpt-5.4",
			version: null,
			params: { model_id: "gpt-5.4", account_id: "fixture-account" },
			secrets: { access_token: "fixture-secret" },
		});
	});

	test("Copilot requires explicit credentials even when a native CLI is available", () => {
		const copilot = provider("github-copilot");
		expect(copilot.validate?.({ model_id: "gpt-4.1" }, false)).toContain(
			"token",
		);
		expect(copilot.validate?.({ access_token: "  " }, false)).toContain(
			"token",
		);
		expect(
			copilot.validate?.({ access_token: "fixture-github" }, false),
		).toBeNull();
		expect(
			copilot.validate?.({ api_key: "fixture-copilot" }, false),
		).toBeNull();
		expect(copilot.validate?.({}, true)).toBeNull();
		const connection = providerFieldValues(copilot, {
			model_id: "gpt-4.1",
			api_key: "fixture-copilot",
		});
		expect(connection.params).toEqual({ model_id: "gpt-4.1" });
		expect(connection.secrets).toEqual({ api_key: "fixture-copilot" });
	});

	test("Microsoft 365 uses its service ID and never accepts an underlying model override", () => {
		const microsoft = provider("microsoft-copilot", false);
		expect(microsoft.fields.some((field) => field.key === "model_id")).toBe(
			false,
		);
		const connection = providerFieldValues(microsoft, {
			model_id: "invented-model",
			access_token: "fixture-microsoft",
			timezone: "Europe/Berlin",
		});
		expect(connection.modelId).toBe("microsoft-365-copilot");
		expect(connection.params).toEqual({
			model_id: "microsoft-365-copilot",
			timezone: "Europe/Berlin",
		});
		expect(connection.secrets).toEqual({ access_token: "fixture-microsoft" });
		expect(microsoft.textOnly).toBe(true);
	});

	test("native Claude configuration retains its executable without inventing credentials", () => {
		const connection = providerFieldValues(provider("claude-code"), {
			model_id: "claude-sonnet-4-6",
			executable: "/Applications/Claude Code/claude",
		});
		expect(connection.params).toEqual({
			model_id: "claude-sonnet-4-6",
			executable: "/Applications/Claude Code/claude",
		});
		expect(connection.secrets).toEqual({});
	});

	test("existing Azure configuration keeps its deployment and secret separation", () => {
		const connection = providerFieldValues(
			{
				key: "azure",
				providerName: "custom:openai",
				label: "Azure",
				description: "",
				primary: true,
				isAzure: true,
				fields: [
					{ key: "model_id", label: "Deployment" },
					{ key: "version", label: "Version" },
					{ key: "api_key", label: "Key", secret: true },
				],
			},
			{ model_id: "deployment", version: "2025-01-01", api_key: "fixture-key" },
		);
		expect(connection.params).toEqual({
			model_id: "deployment",
			version: "2025-01-01",
			is_azure: true,
		});
		expect(connection.secrets).toEqual({ api_key: "fixture-key" });
		expect(connection.modelId).toBe("deployment");
		expect(connection.version).toBe("2025-01-01");
	});
});

test("native discovery excludes generic defaults, deduplicates IDs and supplies missing labels", () => {
	expect(
		specificExternalModels([
			{ id: "default", name: "Configured default" },
			{ id: " DEFAULT ", name: "Configured default" },
			{ id: "", name: "No model" },
			{ id: " gpt-5.4 ", name: "GPT 5.4" },
			{ id: "gpt-5.4", name: "GPT 5.4" },
			{ id: "claude-sonnet-4-6", name: " " },
		]),
	).toEqual([
		{ id: "gpt-5.4", name: "GPT 5.4" },
		{ id: "claude-sonnet-4-6", name: "claude-sonnet-4-6" },
	]);
	expect(specificExternalModels([])).toEqual([]);
});
