export type ExternalModelProvider = "claude-code" | "codex";

export interface ExternalModelOption {
	id: string;
	name: string;
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

/** Keep credentials out of the serialized Bit sent to the API. */
export function providerFieldValues(
	provider: IProviderDef,
	values: Record<string, string>,
) {
	const params: Record<string, unknown> = {};
	const secrets: Record<string, unknown> = {};
	let modelId: string | null = provider.fixedModelId ?? null;
	let version: string | null = null;
	for (const field of provider.fields) {
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
		const id = model.id.trim();
		if (!id || id.toLowerCase() === "default") continue;
		unique.set(id, { id, name: model.name.trim() || id });
	}
	return [...unique.values()];
}
