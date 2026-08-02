import { type IBit, IBitTypes } from "../schema";

type MlxAsset = Pick<IBit, "download_link" | "file_name">;
type MlxDependencyRef = Pick<IBit, "hub" | "id">;

/**
 * Mirrors `Bit::is_mlx_model` in core. An MLX root carries no `download_link`
 * of its own — its artifacts come from the inline Hugging Face manifest that
 * the backend materializes in `Bit::pack`. Treating it like a hosted/proxied
 * model would skip the download entirely.
 */
export function isMlxModelBit(bit: Pick<IBit, "type" | "parameters">): boolean {
	if (bit.type !== IBitTypes.Llm && bit.type !== IBitTypes.Vlm) return false;
	const provider = (bit.parameters as { provider?: { provider_name?: string } })
		?.provider;
	return provider?.provider_name?.toLowerCase() === "mlx";
}

/** Returns a validation message when an MLX dependency target is unsafe. */
export function mlxAssetPathError(fileName: string): string | undefined {
	if (!fileName) return "Stored path is required";
	if (fileName.includes("\\")) {
		return "Use forward slashes in stored paths";
	}
	if (fileName.startsWith("/")) {
		return "Stored paths must be relative";
	}

	for (const component of fileName.split("/")) {
		if (!component) return "Stored paths cannot contain empty segments";
		if (component === "." || component === "..") {
			return "Stored paths cannot contain . or .. segments";
		}
		if (component.includes(":")) {
			return "Stored paths cannot contain colons";
		}
		if (component.includes("\0")) {
			return "Stored paths cannot contain NUL bytes";
		}
	}

	return undefined;
}

function mlxAssetDownloadUrlError(downloadLink: string): string | undefined {
	if (!downloadLink) return "download URL is required";
	try {
		const url = new URL(downloadLink);
		if (url.protocol !== "http:" && url.protocol !== "https:") {
			return "download URL must use http:// or https://";
		}
	} catch {
		return "download URL must be a valid http(s) URL";
	}
	return undefined;
}

/** Select the most specific registry bit type known for a model-bundle file. */
export function inferMlxAssetBitType(fileName: string): IBitTypes {
	const baseName = fileName.split("/").pop()?.toLowerCase();
	switch (baseName) {
		case "config.json":
			return IBitTypes.Config;
		case "tokenizer.json":
		case "tokenizer.model":
		case "sentencepiece.bpe.model":
		case "spiece.model":
		case "vocab.json":
		case "vocab.txt":
		case "merges.txt":
			return IBitTypes.Tokenizer;
		case "tokenizer_config.json":
			return IBitTypes.TokenizerConfig;
		case "special_tokens_map.json":
			return IBitTypes.SpecialTokensMap;
		case "processor_config.json":
		case "preprocessor_config.json":
			return IBitTypes.PreprocessorConfig;
		default:
			return IBitTypes.File;
	}
}

/**
 * Validate the dependency manifest expected by the MLX materializer.
 * The root LLM/VLM bit is virtual; every concrete model file belongs here.
 */
export function validateMlxModelAssets(
	assets: MlxAsset[],
	isVlm: boolean,
): string[] {
	const errors: string[] = [];
	const targets = new Set<string>();
	const originalTargets = new Map<string, string>();

	for (const [index, asset] of assets.entries()) {
		const label = `Asset ${index + 1}`;
		const fileName = asset.file_name?.trim() ?? "";
		const downloadLink = asset.download_link?.trim() ?? "";
		const downloadUrlError = mlxAssetDownloadUrlError(downloadLink);
		if (downloadUrlError) errors.push(`${label}: ${downloadUrlError}`);

		const pathError = mlxAssetPathError(fileName);
		if (pathError) {
			errors.push(`${label}: ${pathError}`);
			continue;
		}

		const portableTarget = fileName.toLowerCase();
		if (targets.has(portableTarget)) {
			errors.push(`${label}: duplicate stored path "${fileName}"`);
		}
		targets.add(portableTarget);
		originalTargets.set(portableTarget, fileName);
	}

	for (const child of targets) {
		const components = child.split("/");
		for (let depth = 1; depth < components.length; depth += 1) {
			const parent = components.slice(0, depth).join("/");
			if (!targets.has(parent)) continue;
			errors.push(
				`Stored paths "${originalTargets.get(parent)}" and "${originalTargets.get(child)}" conflict because one is a file parent of the other`,
			);
			break;
		}
	}

	const hasExactRootFile = (fileName: string) =>
		originalTargets.get(fileName) === fileName;

	if (!hasExactRootFile("config.json")) {
		errors.push("The bundle requires config.json at its root");
	}
	if (![...targets].some((target) => target.endsWith(".safetensors"))) {
		errors.push("The bundle requires at least one .safetensors weights file");
	}

	if (!hasExactRootFile("tokenizer.json")) {
		errors.push("The bundle requires tokenizer.json at its root");
	}
	if (!hasExactRootFile("tokenizer_config.json")) {
		errors.push("The bundle requires tokenizer_config.json at its root");
	}

	if (
		isVlm &&
		!hasExactRootFile("processor_config.json") &&
		!hasExactRootFile("preprocessor_config.json")
	) {
		errors.push(
			"MLX VLM bundles require processor_config.json or preprocessor_config.json",
		);
	}

	return errors;
}

/** Normalize one concrete MLX model-file Bit before it is registered. */
export function prepareMlxAssetBit(asset: IBit, root: IBit): IBit {
	const fileName = asset.file_name?.trim() ?? "";
	return {
		...asset,
		type: inferMlxAssetBitType(fileName),
		download_link: asset.download_link?.trim() || null,
		file_name: fileName,
		license: root.license,
		authors: root.authors,
		repository: root.repository,
		parameters: {},
	};
}

/**
 * Build the virtual MLX LLM/VLM root after every concrete model-file Bit has
 * been registered. The root carries no artifact of its own and references the
 * registered files through normal Bit dependencies.
 */
export function buildMlxModelRootBit(
	root: IBit,
	dependencies: MlxDependencyRef[],
): IBit {
	return {
		...root,
		download_link: null,
		file_name: null,
		size: 0,
		dependencies: dependencies.map(({ hub, id }) => `${hub}:${id}`),
	};
}
