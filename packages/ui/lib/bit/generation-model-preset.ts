import { type IBit, IBitTypes } from "../schema/bit/bit";
import {
	GENERATION_MODEL_PRESETS,
	type GenerationAssetRole,
	type GenerationModelKind,
	type GenerationModelPreset,
} from "./generation-model-presets";
import { mlxAssetPathError } from "./mlx-model-pack";

export * from "./generation-model-presets";

export interface GenerationAssetDraft {
	key: string;
	role: GenerationAssetRole;
	bit: IBit;
}

export const GENERATION_ASSET_LABELS: Record<GenerationAssetRole, string> = {
	model: "Full model checkpoint",
	diffusion_model: "Diffusion model",
	vae: "Image / video decoder (VAE)",
	clip_l: "CLIP-L text encoder",
	clip_g: "CLIP-G text encoder",
	t5xxl: "T5 / UMT5 text encoder",
	llm: "Language model text encoder",
};

export function isGenerationModelBit(bit: Pick<IBit, "type">): boolean {
	return (
		bit.type === IBitTypes.ImageGeneration ||
		bit.type === IBitTypes.VideoGeneration
	);
}

export function getGenerationModelPreset(id: string): GenerationModelPreset {
	const preset = GENERATION_MODEL_PRESETS.find(
		(candidate) => candidate.id === id,
	);
	if (!preset) throw new Error(`Unknown generation model preset: ${id}`);
	return preset;
}

export function defaultGenerationPreset(
	kind: GenerationModelKind,
): GenerationModelPreset {
	const preset = GENERATION_MODEL_PRESETS.find(
		(candidate) => candidate.kind === kind,
	);
	if (!preset)
		throw new Error(`No ${kind} generation model presets are available`);
	return preset;
}

function dependencyRef(bit: IBit): string {
	return bit.hub ? `${bit.hub}:${bit.id}` : bit.id;
}

function assetRefs(assets: GenerationAssetDraft[]) {
	return assets.map(({ role, bit }) => ({ role, bit: dependencyRef(bit) }));
}

/** Each selected file becomes a dependency Bit with a pinned source revision. */
export function createGenerationAssetDrafts(
	presetId: string,
	createBit: () => IBit,
): GenerationAssetDraft[] {
	const preset = getGenerationModelPreset(presetId);
	return preset.assets.map((asset) => {
		if (!/^[a-f0-9]{40}$/i.test(asset.revision)) {
			throw new Error(
				`Unpinned source revision for ${preset.label}: ${asset.path}`,
			);
		}
		if (
			!/^[\w.-]+\/[\w.-]+$/.test(asset.repo) ||
			mlxAssetPathError(asset.path)
		) {
			throw new Error(
				`Invalid model source for ${preset.label}: ${asset.path}`,
			);
		}
		const bit = createBit();
		const fileName = asset.path.split("/").pop() ?? asset.path;
		return {
			key: bit.id,
			role: asset.role,
			bit: {
				...bit,
				type: IBitTypes.File,
				dependencies: [],
				dependency_tree_hash: "",
				parameters: {},
				file_name: fileName,
				download_link: `https://huggingface.co/${asset.repo}/resolve/${asset.revision}/${asset.path.split("/").map(encodeURIComponent).join("/")}?download=true`,
				repository: `https://huggingface.co/${asset.repo}`,
				license: asset.license ?? preset.license,
				authors: [`https://huggingface.co/${asset.repo.split("/")[0]}`],
				size: asset.size,
				version: asset.revision,
				meta: {
					...bit.meta,
					en: {
						...bit.meta.en,
						name: `${preset.label}: ${GENERATION_ASSET_LABELS[asset.role]}`,
						description: `${GENERATION_ASSET_LABELS[asset.role]} for ${preset.label}.`,
						tags: ["generation-asset", "stablediffusion", asset.role],
					},
				},
			},
		};
	});
}

/** A generation model groups weights, text encoders and a decoder into one Bit. */
export function applyGenerationModelPreset(
	bit: IBit,
	presetId: string,
	assets: GenerationAssetDraft[],
): IBit {
	const preset = getGenerationModelPreset(presetId);
	return {
		...bit,
		type:
			preset.kind === "image"
				? IBitTypes.ImageGeneration
				: IBitTypes.VideoGeneration,
		download_link: null,
		file_name: null,
		size: 0,
		dependency_tree_hash: "",
		dependencies: assets.map(({ bit: asset }) => dependencyRef(asset)),
		authors: preset.authors,
		license: preset.license,
		repository: preset.repository,
		name: preset.label,
		model_slug: null,
		parameters: {
			assets: assetRefs(assets),
			provider: {
				provider_name: "local:stablediffusion",
				model_id: preset.id,
				version: null,
				params: {
					stablediffusion: {
						offload_to_cpu: true,
						diffusion_flash_attention: false,
						startup_timeout_seconds: 600,
						request_timeout_seconds: 3600,
						...preset.config,
					},
					generation_defaults: { ...preset.defaults },
				},
			},
		},
		meta: {
			...bit.meta,
			en: {
				...bit.meta.en,
				name: preset.label,
				description: preset.description,
				long_description: `${preset.description}\n\n${preset.notes}`,
				docs_url: preset.repository,
				website: preset.repository,
				tags: [`${preset.kind}-generation`, "local", "stablediffusion"],
			},
		},
	};
}

export function validateGenerationAssets(
	assets: GenerationAssetDraft[],
): string[] {
	const errors: string[] = [];
	const roles = new Set<string>();
	const refs = new Set<string>();
	for (const { role, bit } of assets) {
		const label = GENERATION_ASSET_LABELS[role] ?? role;
		if (!Object.hasOwn(GENERATION_ASSET_LABELS, role)) {
			errors.push(`Unknown generation asset role: ${role}`);
		}
		if (roles.has(role))
			errors.push(`Duplicate generation asset role: ${label}`);
		roles.add(role);
		if (!bit.id || refs.has(dependencyRef(bit))) {
			errors.push(`${label}: each model file needs a unique Bit ID`);
		}
		refs.add(dependencyRef(bit));
		const pathError = mlxAssetPathError(bit.file_name?.trim() ?? "");
		if (pathError) errors.push(`${label}: ${pathError}`);
		try {
			const url = new URL(bit.download_link?.trim() ?? "");
			if (
				url.protocol !== "https:" ||
				url.username ||
				url.password ||
				url.hash
			) {
				throw new Error("Invalid URL");
			}
		} catch {
			errors.push(
				`${label}: provide an HTTPS download URL without credentials or a fragment`,
			);
		}
	}
	if (!roles.has("model") && !roles.has("diffusion_model")) {
		errors.push(
			"A generation model requires a checkpoint or diffusion weights",
		);
	}
	if (roles.has("model") && roles.has("diffusion_model")) {
		errors.push(
			"Select either a full checkpoint or separate diffusion weights",
		);
	}
	return errors;
}

/** Preserve reviewed metadata and replace draft references with registered IDs. */
export function buildGenerationModelRootBit(
	bit: IBit,
	registeredAssets: GenerationAssetDraft[],
): IBit {
	const errors = validateGenerationAssets(registeredAssets);
	if (!isGenerationModelBit(bit))
		errors.push("Expected an image or video generation model Bit");
	const suppliedRoles = new Set(registeredAssets.map((asset) => asset.role));
	const expectedAssets = bit.parameters?.assets;
	if (Array.isArray(expectedAssets)) {
		for (const asset of expectedAssets) {
			if (!suppliedRoles.has(asset.role))
				errors.push(`Missing required model asset: ${asset.role}`);
		}
	}
	if (errors.length) throw new Error(errors.join(". "));
	return {
		...bit,
		download_link: null,
		file_name: null,
		size: 0,
		dependency_tree_hash: "",
		dependencies: registeredAssets.map(({ bit: asset }) =>
			dependencyRef(asset),
		),
		parameters: { ...bit.parameters, assets: assetRefs(registeredAssets) },
	};
}
