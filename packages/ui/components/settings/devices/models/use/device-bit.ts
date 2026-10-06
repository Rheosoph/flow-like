import { createId } from "@paralleldrive/cuid2";
import { ApiResponseError } from "../../../../../lib/api-error";
import {
	DEVICE_PROVIDER_NAME,
	deviceOfModelBit,
} from "../../../../../lib/bit/local-model-filter";
import type {
	HostedModel,
	ModelKind,
} from "../../../../../lib/device-management/models";
import {
	type IBit,
	IBitTypes,
	type IMetadata,
} from "../../../../../lib/schema/bit/bit";
import { IPooling } from "../../../../../lib/schema/bit/bit/embedding-model-parameters";
import type { IBackendState } from "../../../../../state/backend-state";

/**
 * Provider params of a device Bit (plan §3.8). The Bit carries no secret:
 * calls are authorized by the controller key of whoever runs it.
 */
export interface DeviceModelParams {
	device_id: string;
	/** Hosted model id on the device's model gateway. */
	model: string;
	kind: ModelKind;
	/** Chat and vision models; the gateway speaks OpenAI chat completions. */
	api_surface?: "chat_completions";
}

const BIT_TYPE: Record<ModelKind, IBitTypes> = {
	chat: IBitTypes.Llm,
	vision: IBitTypes.Vlm,
	embedding: IBitTypes.Embedding,
};

/** Used when the hosted model's settings leave the context to the device. */
const DEFAULT_CONTEXT: Record<ModelKind, number> = {
	chat: 8192,
	vision: 8192,
	embedding: 512,
};

export const MAX_CONTEXT = 1 << 20;
export const MAX_VECTOR_LENGTH = 65_536;
export const BIT_NAME_MAX = 128;

/** Every trait at the middle: nothing is known about a self-hosted model, and 0 would rank it last. */
const NEUTRAL_CLASSIFICATION = {
	coding: 0.5,
	cost: 0.5,
	creativity: 0.5,
	factuality: 0.5,
	function_calling: 0.5,
	multilinguality: 0.5,
	openness: 0.5,
	reasoning: 0.5,
	safety: 0.5,
	speed: 0.5,
};

/** A whole number typed into a field, from 1 to `max`; `undefined` otherwise. */
export function wholeNumber(text: string, max: number): number | undefined {
	const value = Number(text.trim());
	return Number.isSafeInteger(value) && value >= 1 && value <= max
		? value
		: undefined;
}

/** The context the device serves per request: the model's setting when it fixes one. */
export function defaultContext(
	model: Pick<HostedModel, "kind" | "settings">,
): number {
	return model.settings.ctx_per_slot ?? DEFAULT_CONTEXT[model.kind];
}

/** The user's Bit for a model hosted on a device, among their custom Bits. */
export function savedDeviceBit(
	bits: readonly IBit[],
	deviceId: string,
	modelId: string,
): IBit | undefined {
	return bits.find(
		(bit) =>
			deviceOfModelBit(bit) === deviceId &&
			bit.parameters?.provider?.params?.model === modelId,
	);
}

/**
 * One Bit per device and model: adding the same model again updates the
 * user's Bit instead of adding a twin. A new Bit gets a random id, as other
 * custom Bits do; Bit ids are global on the hub, so an id anyone could derive
 * from the account, device and model could be taken first.
 */
export function deviceBitIdFor(
	bits: readonly IBit[],
	deviceId: string,
	modelId: string,
): string {
	return savedDeviceBit(bits, deviceId, modelId)?.id ?? createId();
}

export function deviceModelParams(
	deviceId: string,
	model: Pick<HostedModel, "id" | "kind">,
): DeviceModelParams {
	return {
		device_id: deviceId,
		model: model.id,
		kind: model.kind,
		...(model.kind === "embedding"
			? {}
			: { api_surface: "chat_completions" as const }),
	};
}

export interface DeviceBitInput {
	id: string;
	deviceId: string;
	model: Pick<HostedModel, "id" | "kind">;
	name: string;
	description: string;
	/** Tokens per request; an embedding model's input length. */
	contextLength: number;
	/** Embedding models only: numbers per vector. */
	vectorLength?: number;
	/** Unix milliseconds. */
	now: number;
}

function metadata(input: DeviceBitInput): IMetadata {
	const at = {
		secs_since_epoch: Math.floor(input.now / 1000),
		nanos_since_epoch: 0,
	};
	return {
		name: input.name,
		description: input.description,
		long_description: null,
		icon: null,
		thumbnail: null,
		tags: [],
		preview_media: [],
		age_rating: null,
		docs_url: null,
		release_notes: null,
		support_url: null,
		use_case: null,
		website: null,
		organization_specific_values: null,
		created_at: at,
		updated_at: at,
	};
}

function parameters(input: DeviceBitInput) {
	const provider = {
		provider_name: DEVICE_PROVIDER_NAME,
		model_id: input.model.id,
		version: null,
		params: deviceModelParams(input.deviceId, input.model),
	};
	if (input.model.kind !== "embedding")
		return {
			context_length: input.contextLength,
			provider,
			model_classification: { ...NEUTRAL_CLASSIFICATION },
		};
	if (!input.vectorLength)
		throw new Error(
			`Device embedding Bit ${input.id} for model ${input.model.id} needs a vector length.`,
		);
	return {
		languages: [],
		vector_length: input.vectorLength,
		input_length: input.contextLength,
		prefix: { query: "", paragraph: "" },
		pooling: IPooling.Mean,
		provider,
	};
}

/** The user's own Bit for a model hosted on one of their devices: no file, no link, no secret. */
export function deviceModelBit(input: DeviceBitInput): IBit {
	const at = new Date(input.now).toISOString();
	return {
		id: input.id,
		type: BIT_TYPE[input.model.kind],
		meta: { en: metadata(input) },
		parameters: parameters(input),
		download_link: null,
		file_name: null,
		size: null,
		repository: null,
		dependencies: [],
		hash: "",
		dependency_tree_hash: "",
		authors: [],
		hub: "",
		version: null,
		license: null,
		created: at,
		updated: at,
	};
}

/* Saving it. */

export type DeviceBitFailure =
	/** The hub took no Bit of the `device` provider: hubs before device models refuse it. */
	| "hub_refused"
	/** The hub says this account may not use the device. */
	| "not_allowed"
	| "failed";

export class DeviceBitError extends Error {
	constructor(
		readonly code: DeviceBitFailure,
		message: string,
		options?: { cause?: unknown },
	) {
		super(message, options);
		this.name = "DeviceBitError";
	}
}

const REFUSED = new Set([400, 404, 405, 409, 422, 501]);
const NOT_ALLOWED = new Set([401, 403]);

/** Why the hub didn't save the Bit, from its answer. */
export function deviceBitFailure(error: unknown): DeviceBitFailure {
	if (!(error instanceof ApiResponseError)) return "failed";
	if (NOT_ALLOWED.has(error.status)) return "not_allowed";
	return REFUSED.has(error.status) ? "hub_refused" : "failed";
}

export interface SavedDeviceBit {
	bit: IBit;
	/** `false`: saved to the user's models, but the profile in use did not take it. */
	inProfile: boolean;
}

/**
 * Saves the Bit to the user's models and turns it on in the profile in use,
 * as a newly configured model is. The desktop keeps it locally even when its
 * hub refuses it; the web only has the hub, so a refusal is a `DeviceBitError`.
 */
export async function saveDeviceModelBit(
	backend: Pick<IBackendState, "bitState" | "userState">,
	bit: IBit,
): Promise<SavedDeviceBit> {
	let saved: IBit;
	try {
		saved = (await backend.bitState.upsertCustomBit(bit)) ?? bit;
	} catch (error) {
		throw new DeviceBitError(
			deviceBitFailure(error),
			`Saving device model Bit ${bit.id} failed: ${error instanceof Error ? error.message : String(error)}`,
			{ cause: error },
		);
	}
	try {
		const profile = await backend.userState.getSettingsProfile();
		await backend.bitState.addBit(saved, profile);
		return { bit: saved, inProfile: true };
	} catch {
		return { bit: saved, inProfile: false };
	}
}
