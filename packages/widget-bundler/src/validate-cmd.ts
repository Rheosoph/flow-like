import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { unzipSync } from "fflate";
import {
	BUNDLE_FORMAT_VERSION,
	BUNDLE_MANIFEST_PATH,
	type WidgetBundleManifest,
	isSafeEntryPath,
} from "./bundle-format";
import {
	WIDGET_PROTOCOL,
	type WidgetContract,
	validateContract,
} from "./contract-types";
import { extractContract } from "./extract";
import {
	discoverGroupWidgets,
	discoverGroups,
	entryHash,
	readPackageInfo,
} from "./pack";

export interface ProjectValidation {
	ok: boolean;
	errors: string[];
	warnings: string[];
	widgets: { id: string; group: string }[];
}

/** Validate contract extraction for every widget in a project (no build required). */
export function validateProject(projectDir: string): ProjectValidation {
	const errors: string[] = [];
	const warnings: string[] = [];
	const widgets: { id: string; group: string }[] = [];

	try {
		readPackageInfo(projectDir);
	} catch (e) {
		errors.push(e instanceof Error ? e.message : String(e));
	}

	const groups = discoverGroups(projectDir);
	if (groups.length === 0) {
		errors.push(
			`No framework groups found under ${join(projectDir, "widgets")} (expected widgets/<group>/package.json)`,
		);
	}

	const seenIds = new Set<string>();
	for (const group of groups) {
		for (const widgetId of discoverGroupWidgets(group.dir)) {
			const configPath = join(
				group.dir,
				"src",
				"widgets",
				widgetId,
				"widget.config.ts",
			);
			try {
				const extracted = extractContract(configPath);
				warnings.push(...extracted.warnings);
				if (extracted.config.id !== widgetId) {
					errors.push(
						`Widget id '${extracted.config.id}' in ${configPath} does not match its directory name '${widgetId}'`,
					);
				}
				if (seenIds.has(widgetId)) {
					errors.push(
						`Duplicate widget id across framework groups: ${widgetId}`,
					);
				}
				seenIds.add(widgetId);
				widgets.push({ id: widgetId, group: group.name });
			} catch (e) {
				errors.push(e instanceof Error ? e.message : String(e));
			}
		}
	}

	if (groups.length > 0 && widgets.length === 0 && errors.length === 0) {
		errors.push(
			"No widgets found (expected widgets/<group>/src/widgets/<id>/widget.config.ts)",
		);
	}

	return { ok: errors.length === 0, errors, warnings, widgets };
}

export interface BundleValidation {
	ok: boolean;
	errors: string[];
	manifest: WidgetBundleManifest | null;
}

/**
 * Validate a built `.flwb`: manifest shape, per-entry hashes, contract
 * validity, entry paths. Mirrors `WidgetBundleReader::validate` in
 * packages/wasm/schema/src/widget_bundle.rs.
 */
export function validateBundle(flwbPath: string): BundleValidation {
	const errors: string[] = [];
	if (!existsSync(flwbPath)) {
		return {
			ok: false,
			errors: [`Bundle not found: ${flwbPath}`],
			manifest: null,
		};
	}

	let entries: Record<string, Uint8Array>;
	try {
		entries = unzipSync(new Uint8Array(readFileSync(flwbPath)));
	} catch (e) {
		return {
			ok: false,
			errors: [
				`Failed to read widget bundle ZIP: ${e instanceof Error ? e.message : e}`,
			],
			manifest: null,
		};
	}

	const manifestBytes = entries[BUNDLE_MANIFEST_PATH];
	if (!manifestBytes) {
		return {
			ok: false,
			errors: [`Widget bundle is missing ${BUNDLE_MANIFEST_PATH}`],
			manifest: null,
		};
	}
	let manifest: WidgetBundleManifest;
	try {
		manifest = JSON.parse(new TextDecoder().decode(manifestBytes));
	} catch (e) {
		return {
			ok: false,
			errors: [
				`Failed to parse bundle.json: ${e instanceof Error ? e.message : e}`,
			],
			manifest: null,
		};
	}

	if (
		manifest.formatVersion === 0 ||
		manifest.formatVersion > BUNDLE_FORMAT_VERSION
	) {
		errors.push(
			`Unsupported bundle format version ${manifest.formatVersion} (supported: 1..=${BUNDLE_FORMAT_VERSION})`,
		);
	}
	if (manifest.protocol !== WIDGET_PROTOCOL) {
		errors.push(
			`Unsupported widget protocol '${manifest.protocol}' (expected '${WIDGET_PROTOCOL}')`,
		);
	}
	if (!manifest.packageId) {
		errors.push("Bundle manifest is missing packageId");
	}
	if (!manifest.widgets || manifest.widgets.length === 0) {
		errors.push("Bundle contains no widgets");
	}

	const sharedPaths = new Set<string>();
	for (const shared of manifest.shared ?? []) {
		sharedPaths.add(shared.path);
		if (!isSafeEntryPath(shared.path) || !shared.path.startsWith("shared/")) {
			errors.push(`Invalid shared chunk path: ${shared.path}`);
			continue;
		}
		const data = entries[shared.path];
		if (!data) {
			errors.push(`Widget bundle entry not found: ${shared.path}`);
			continue;
		}
		const actual = entryHash(data);
		if (actual !== shared.hash) {
			errors.push(
				`Hash mismatch for shared chunk ${shared.path}: expected ${shared.hash}, got ${actual}`,
			);
		}
	}

	const seenIds = new Set<string>();
	for (const widget of manifest.widgets ?? []) {
		if (seenIds.has(widget.id)) {
			errors.push(`Duplicate widget id in bundle: ${widget.id}`);
		}
		seenIds.add(widget.id);
		const prefix = `widgets/${widget.id}/`;
		if (!widget.entry.startsWith(prefix) || !isSafeEntryPath(widget.entry)) {
			errors.push(
				`Widget '${widget.id}' entry path '${widget.entry}' must live under ${prefix}`,
			);
		}
		if (
			!widget.contract.startsWith(prefix) ||
			!isSafeEntryPath(widget.contract)
		) {
			errors.push(
				`Widget '${widget.id}' contract path '${widget.contract}' must live under ${prefix}`,
			);
		}

		const entryData = entries[widget.entry];
		if (!entryData) {
			errors.push(`Widget bundle entry not found: ${widget.entry}`);
		} else {
			const actual = entryHash(entryData);
			if (actual !== widget.entryHash) {
				errors.push(
					`Hash mismatch for widget entry ${widget.entry}: expected ${widget.entryHash}, got ${actual}`,
				);
			}
		}

		const contractData = entries[widget.contract];
		if (!contractData) {
			errors.push(`Widget bundle entry not found: ${widget.contract}`);
		} else {
			try {
				const contract: WidgetContract = JSON.parse(
					new TextDecoder().decode(contractData),
				);
				if (contract.id !== widget.id) {
					errors.push(
						`Contract id '${contract.id}' does not match widget id '${widget.id}'`,
					);
				}
				errors.push(...validateContract(contract));
			} catch (e) {
				errors.push(
					`Failed to parse contract for widget '${widget.id}': ${e instanceof Error ? e.message : e}`,
				);
			}
		}

		for (const asset of widget.assets ?? []) {
			if (!sharedPaths.has(asset)) {
				errors.push(
					`Widget '${widget.id}' references undeclared asset: ${asset}`,
				);
			}
		}
	}

	return { ok: errors.length === 0, errors, manifest };
}
