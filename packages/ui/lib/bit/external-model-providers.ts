export type ExternalModelProvider = "claude-code" | "codex";

export interface ExternalReasoningEffortOption {
	id: string;
	name: string;
	description?: string;
}

export interface ExternalModelOption {
	id: string;
	name: string;
	description?: string;
	contextWindow?: number;
	supportsVision?: boolean;
	supportedReasoningEfforts?: ExternalReasoningEffortOption[];
	defaultReasoningEffort?: string;
	isDefault?: boolean;
}

export interface ExternalModelMetadata {
	name: string;
	description: string;
	icon: string;
	tags: string[];
	contextLength?: number;
	vision?: boolean;
}

export interface ExternalModelCatalog {
	available: boolean;
	authenticated: boolean;
	models: ExternalModelOption[];
	message?: string;
}

type ExternalModelCommand =
	| "flowpilot_agent_backend_status"
	| "flowpilot_agent_backend_get_auth_status"
	| "flowpilot_agent_backend_list_models";

export async function discoverExternalModelCatalog(
	provider: ExternalModelProvider,
	invoke: <T>(
		command: ExternalModelCommand,
		args: { backend: ExternalModelProvider },
	) => Promise<T>,
): Promise<ExternalModelCatalog> {
	const status = await invoke<{ available: boolean; message?: string }>(
		"flowpilot_agent_backend_status",
		{ backend: provider },
	);
	if (!status.available) {
		return {
			available: false,
			authenticated: false,
			models: [],
			message: status.message,
		};
	}
	const auth = await invoke<{ authenticated: boolean; message?: string }>(
		"flowpilot_agent_backend_get_auth_status",
		{ backend: provider },
	);
	if (!auth.authenticated) {
		return {
			available: true,
			authenticated: false,
			models: [],
			message: auth.message,
		};
	}
	const models = await invoke<ExternalModelOption[]>(
		"flowpilot_agent_backend_list_models",
		{ backend: provider },
	);
	return {
		available: true,
		authenticated: true,
		models: specificExternalModels(models),
	};
}

export interface IProviderField {
	key: string;
	label: string;
	placeholder?: string;
	defaultValue?: string;
	description?: string;
	secret?: boolean;
	required?: boolean;
	multiline?: boolean;
	advanced?: boolean;
}

export interface IProviderDef {
	key: string;
	providerName: string;
	label: string;
	description: string;
	primary: boolean;
	isAzure?: boolean;
	nativeOnly?: boolean;
	discoveryProvider?: ExternalModelProvider;
	fixedModelId?: string;
	textOnly?: boolean;
	fields: IProviderField[];
	validate?: (values: Record<string, string>, isEdit: boolean) => string | null;
}

const modelField: IProviderField = {
	key: "model_id",
	label: "Model ID",
	placeholder: "Choose or enter a model ID",
	required: true,
};

const accessTokenField: IProviderField = {
	key: "access_token",
	label: "Access token",
	secret: true,
};

/** Native sign-in is only offered by the desktop backend. */
export function externalModelProviders(
	nativeAvailable: boolean,
): IProviderDef[] {
	return [
		{
			key: "claude-code",
			providerName: "custom:claude-code",
			label: "Claude Code",
			description: "Use a model through Claude Code signed in on this device",
			primary: true,
			nativeOnly: true,
			discoveryProvider: "claude-code" as const,
			textOnly: true,
			fields: [
				modelField,
				{
					key: "executable",
					label: "Claude executable",
					placeholder: "Absolute path to Claude Code",
					advanced: true,
					description:
						"Leave empty to find Claude Code automatically, or enter its absolute executable path.",
				},
			],
		},
		{
			key: "codex",
			providerName: "custom:codex",
			label: "Codex (ChatGPT)",
			description: "Use models from your ChatGPT subscription",
			primary: true,
			discoveryProvider: "codex" as const,
			fields: [
				modelField,
				{
					...accessTokenField,
					required: !nativeAvailable,
					advanced: nativeAvailable,
					description: nativeAvailable
						? "Leave empty to read existing Codex credentials from auth.json on this device. Keychain-only sign-in and token refresh are not supported."
						: "Provide a ChatGPT access token for execution on the server.",
				},
				{
					key: "account_id",
					label: "ChatGPT account ID",
					placeholder: "Optional account ID for this token",
					advanced: true,
				},
			],
		},
		{
			key: "github-copilot",
			providerName: "custom:github-copilot",
			label: "GitHub Copilot",
			description: "Use a specific Copilot model with your GitHub token",
			primary: true,
			fields: [
				modelField,
				{
					...accessTokenField,
					label: "GitHub access token",
					description:
						"A GitHub token authorized for Copilot. This connection does not reuse Copilot CLI sign-in.",
				},
				{
					key: "api_key",
					label: "Copilot API token",
					secret: true,
					advanced: true,
					description:
						"Use an already exchanged Copilot API token instead of a GitHub access token.",
				},
			],
			validate: (values: Record<string, string>, isEdit: boolean) =>
				isEdit || values.access_token?.trim() || values.api_key?.trim()
					? null
					: "A GitHub access token or Copilot API token is required",
		},
		{
			key: "microsoft-copilot",
			providerName: "custom:microsoft-copilot",
			label: "Microsoft 365 Copilot",
			description:
				"Chat with Microsoft 365 Copilot using a delegated Microsoft Graph token",
			primary: true,
			fixedModelId: "microsoft-365-copilot",
			textOnly: true,
			fields: [
				{ ...accessTokenField, required: true },
				{
					key: "timezone",
					label: "Timezone",
					placeholder: "Europe/Berlin",
					defaultValue: "UTC",
					description: "Timezone used for the conversation",
				},
			],
		},
	].filter((provider) => nativeAvailable || !provider.nativeOnly);
}

export const SECRET_PROVIDER_PARAM_KEYS = [
	"api_key",
	"service_account_json",
	"access_token",
	"headers",
];

/** Keep credentials out of the serialized Bit sent to the API. */
export function providerFieldValues(
	provider: IProviderDef,
	values: Record<string, string>,
	existingParams: Record<string, unknown> = {},
) {
	const params: Record<string, unknown> = { ...existingParams };
	const secrets: Record<string, unknown> = {};
	for (const key of SECRET_PROVIDER_PARAM_KEYS) {
		if (Object.hasOwn(params, key)) {
			secrets[key] = params[key];
			delete params[key];
		}
	}
	let modelId: string | null = provider.fixedModelId ?? null;
	let version: string | null = null;
	for (const field of provider.fields) {
		delete params[field.key];
		const value = values[field.key]?.trim();
		if (!value) continue;
		if (field.secret) {
			secrets[field.key] = value;
			continue;
		}
		params[field.key] = value;
		if (field.key === "model_id") modelId = value;
		if (field.key === "version") version = value;
	}
	if (provider.fixedModelId) params.model_id = provider.fixedModelId;
	if (provider.isAzure) params.is_azure = true;
	return { params, secrets, modelId, version };
}

/** Only advertise concrete IDs returned by the installed runtime. */
export function specificExternalModels(
	models: ExternalModelOption[],
): ExternalModelOption[] {
	const unique = new Map<string, ExternalModelOption>();
	for (const model of models) {
		const normalized = normalizeExternalModel(model);
		if (!normalized.id || normalized.id.toLowerCase() === "default") continue;
		unique.set(normalized.id, normalized);
	}
	return [...unique.values()];
}

function normalizeExternalModel(
	model: ExternalModelOption,
): ExternalModelOption {
	const id = typeof model.id === "string" ? model.id.trim() : "";
	const name = typeof model.name === "string" ? model.name.trim() : "";
	const normalized: ExternalModelOption = { id, name: name || id };
	if (typeof model.description === "string" && model.description.trim()) {
		normalized.description = model.description.trim();
	}
	if (
		typeof model.contextWindow === "number" &&
		Number.isInteger(model.contextWindow) &&
		model.contextWindow > 0
	) {
		normalized.contextWindow = model.contextWindow;
	}
	if (typeof model.supportsVision === "boolean") {
		normalized.supportsVision = model.supportsVision;
	}
	if (Array.isArray(model.supportedReasoningEfforts)) {
		const efforts = new Map<string, ExternalReasoningEffortOption>();
		for (const effort of model.supportedReasoningEfforts) {
			if (!effort || typeof effort.id !== "string") continue;
			const id = effort.id.trim();
			if (!id || efforts.has(id)) continue;
			const name = typeof effort.name === "string" ? effort.name.trim() : "";
			const option: ExternalReasoningEffortOption = { id, name: name || id };
			if (typeof effort.description === "string" && effort.description.trim()) {
				option.description = effort.description.trim();
			}
			efforts.set(id, option);
		}
		normalized.supportedReasoningEfforts = [...efforts.values()];
	}
	if (
		typeof model.defaultReasoningEffort === "string" &&
		model.defaultReasoningEffort.trim()
	) {
		normalized.defaultReasoningEffort = model.defaultReasoningEffort.trim();
	}
	if (typeof model.isDefault === "boolean") {
		normalized.isDefault = model.isDefault;
	}
	return normalized;
}

export function externalModelMetadata(
	provider: ExternalModelProvider,
	model: ExternalModelOption,
): ExternalModelMetadata {
	const normalized = normalizeExternalModel(model);
	const providerName = provider === "claude-code" ? "Claude Code" : "Codex";
	const vision = provider === "claude-code" ? false : normalized.supportsVision;
	const tags: string[] = [provider, "chat"];
	if (normalized.supportedReasoningEfforts?.length) tags.push("reasoning");
	if (vision === true) tags.push("vision");
	return {
		name: normalized.name,
		description:
			normalized.description ?? `${normalized.name} via ${providerName}.`,
		icon:
			provider === "claude-code"
				? "/provider/anthropic.webp"
				: "/provider/gpt.webp",
		tags,
		...(normalized.contextWindow !== undefined && {
			contextLength: normalized.contextWindow,
		}),
		...(vision !== undefined && { vision }),
	};
}
