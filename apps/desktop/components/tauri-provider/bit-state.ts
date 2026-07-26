import type {
	IBit,
	IBitPack,
	IBitState,
	IDownloadProgress,
	IIntercomEvent,
	IProfile,
	ISettingsProfile,
	ITtsAssetRef,
} from "@flow-like/flow-like-ui";
import {
	IBitTypes,
	bitDependencyRef,
	createTtsRepairMarkerBit,
	createTtsRepairReplacementBit,
	getTtsAssetRefs,
	getTtsAssetRepairPlan,
	localTtsAssetId,
} from "@flow-like/flow-like-ui";
import type { IBitSearchQuery } from "@flow-like/flow-like-ui/lib/schema/hub/bit-search-query";
import { invoke } from "@tauri-apps/api/core";
import { type UnlistenFn, listen } from "@tauri-apps/api/event";
import type { TauriBackend } from "../tauri-provider";

export class BitState implements IBitState {
	constructor(private readonly backend: TauriBackend) {}

	private async upsertAdminBit(
		profile: IProfile,
		bit: IBit,
		routeId = bit.id,
	): Promise<IBit> {
		let finalBit = bit;
		let receivedFinalBit = false;

		await this.backend.apiState.stream<Record<string, unknown>>(
			profile,
			`admin/bit/${routeId}`,
			{
				method: "PUT",
				body: JSON.stringify(bit),
			},
			(data) => {
				if (typeof data?.id === "string") {
					finalBit = data as unknown as IBit;
					receivedFinalBit = true;
				}
			},
		);

		if (!receivedFinalBit) {
			throw new Error(`Bit update did not complete for ${bit.id}`);
		}

		return finalBit;
	}

	private async pushEnglishMeta(profile: IProfile, bit: IBit): Promise<void> {
		const meta = bit.meta?.en ?? Object.values(bit.meta ?? {})[0];
		if (!meta) return;

		await this.backend.apiState.put(profile, `admin/bit/${bit.id}/en`, meta);
	}

	private async deleteAdminBitBestEffort(
		profile: IProfile,
		bitId: string,
	): Promise<void> {
		try {
			await this.backend.apiState.del(profile, `admin/bit/${bitId}`);
		} catch (error) {
			console.warn(`Failed to delete replaced TTS bit ${bitId}`, error);
		}
	}

	private async replaceCurrentProfileBitBestEffort(
		oldBit: IBit,
		newBit: IBit,
	): Promise<void> {
		try {
			const profile = await invoke<ISettingsProfile>("get_current_profile");
			const currentBits = profile.hub_profile.bits ?? [];
			const hasOldBit = currentBits.some(
				(reference) => reference.split(":").pop() === oldBit.id,
			);
			if (!hasOldBit) return;

			await this.removeBit(oldBit, profile);
			await this.addBit(newBit, profile);

			const updatedBits = currentBits
				.filter((reference) => reference.split(":").pop() !== oldBit.id)
				.concat(bitDependencyRef(newBit));
			const activeProfile = this.backend.profile;
			if (activeProfile && activeProfile.id === profile.hub_profile.id) {
				this.backend.profile = {
					...activeProfile,
					bits: Array.from(new Set(updatedBits)),
				};
			}
		} catch (error) {
			console.warn(
				`Failed to replace current profile TTS bit ${oldBit.id}`,
				error,
			);
		}
	}

	async getInstalledBit(bits: IBit[]): Promise<IBit[]> {
		return await invoke("get_installed_bit", {
			bits: bits,
		});
	}
	async downloadBit(
		bit: IBit,
		pack: IBitPack,
		cb?: (progress: IDownloadProgress[]) => void,
	): Promise<IBit[]> {
		const unlistenFn: UnlistenFn[] = [];

		for (const deps of pack.bits) {
			unlistenFn.push(
				await listen(`download:${deps.hash}`, (event) => {
					const payload = event.payload as IIntercomEvent[];
					const downloadProgressEvents = payload.map((item) => item.payload);
					if (cb) cb(downloadProgressEvents);
				}),
			);
		}

		const bits: IBit[] = await invoke("download_bit", {
			bit: bit,
		});

		for (const unlisten of unlistenFn) {
			unlisten();
		}

		return bits;
	}

	async getPackFromBit(bit: IBit): Promise<{ bits: IBit[] }> {
		console.log("Getting pack from bit:", bit);
		const pack = await invoke<{ bits: IBit[] }>("get_pack_from_bit", {
			bit: bit,
		});
		console.log("Pack retrieved:", pack);
		return pack;
	}

	async deleteBit(bit: IBit): Promise<void> {
		throw new Error("Method not implemented.");
	}
	async getBit(id: string, hub?: string): Promise<IBit> {
		return await invoke("get_bit", {
			bit: id,
			hub: hub,
		});
	}
	async addBit(bit: IBit, profile: ISettingsProfile): Promise<void> {
		await invoke("add_bit", {
			bit: bit,
			profile: profile,
		});
	}
	async removeBit(bit: IBit, profile: ISettingsProfile): Promise<void> {
		await invoke("remove_bit", {
			bit: bit,
			profile: profile,
		});
	}
	async getPackSize(bits: IBit[]): Promise<number> {
		const size: number = await invoke("get_bit_size", {
			bits: bits,
		});
		return size;
	}
	async getBitSize(bit: IBit): Promise<number> {
		return await invoke("get_bit_size", {
			bit: bit,
		});
	}
	async searchBits(query: IBitSearchQuery): Promise<IBit[]> {
		return await invoke("search_bits", {
			query,
		});
	}
	async isBitInstalled(bit: IBit): Promise<boolean> {
		return await invoke("is_bit_installed", {
			bit: bit,
		});
	}
	async getProfileBits(): Promise<IBit[]> {
		return await invoke("get_bits_in_current_profile");
	}

	private static mergeSecretParams(
		bit: IBit,
		secrets?: Record<string, unknown>,
	): IBit {
		if (!secrets || Object.keys(secrets).length === 0) return bit;
		const parameters = (bit.parameters ?? {}) as Record<string, unknown>;
		const provider = {
			...((parameters.provider ?? {}) as Record<string, unknown>),
		};
		provider.params = {
			...((provider.params ?? {}) as Record<string, unknown>),
			...secrets,
		};
		return {
			...bit,
			parameters: { ...parameters, provider },
		};
	}

	/**
	 * The user-wide custom-model library: configured once, offline-capable, and
	 * independent of which profile activates a model. With a session it also
	 * syncs from the API (stored encrypted) so the same library follows the
	 * user to the browser.
	 */
	async listCustomBits(): Promise<IBit[]> {
		try {
			const profile = this.backend.profile;
			if (profile) {
				const remote = await this.backend.apiState.get<IBit[]>(
					profile,
					"user/bits?include_secrets=true",
				);
				for (const bit of remote ?? []) {
					await invoke("upsert_custom_bit", { bit });
				}
			}
		} catch (error) {
			console.warn("Custom bit sync from API failed, using local only", error);
		}
		return await invoke<IBit[]>("get_custom_bits");
	}

	async upsertCustomBit(
		bit: IBit,
		secrets?: Record<string, unknown>,
	): Promise<IBit> {
		const localBit = BitState.mergeSecretParams(bit, secrets);
		await invoke("upsert_custom_bit", { bit: localBit });

		try {
			const profile = this.backend.profile;
			if (profile) {
				await this.backend.apiState.put(profile, `user/bits/${bit.id}`, {
					bit,
					secrets,
				});
			}
		} catch (error) {
			console.warn(
				`Custom bit ${bit.id} not synced to API (local only)`,
				error,
			);
		}

		return localBit;
	}

	async deleteCustomBit(bitId: string): Promise<void> {
		await invoke("remove_custom_bit", { bitId });

		try {
			const profile = this.backend.profile;
			if (profile) {
				await this.backend.apiState.del(profile, `user/bits/${bitId}`);
			}
		} catch (error) {
			console.warn(`Custom bit ${bitId} not deleted from API`, error);
		}
	}

	async repairTtsBitAssets(bit: IBit, force = false): Promise<IBitPack> {
		const profile = this.backend.profile;
		if (!profile) {
			throw new Error("Profile is required to repair TTS bit assets");
		}

		const parentBit = await this.backend.apiState.get<IBit>(
			profile,
			`bit/${bit.id}`,
		);
		if (parentBit.type !== IBitTypes.Tts) {
			throw new Error(`Bit ${bit.id} is not a TTS bit`);
		}

		const repairedBits: IBit[] = [];
		const finalAssets: ITtsAssetRef[] = [];
		const finalDependencies: string[] = [];
		const parentAssets = getTtsAssetRefs(parentBit);
		const originalAssetIds = new Set(
			parentAssets.map((asset) => asset.bit.split(":").pop()),
		);
		for (const asset of parentAssets) {
			const assetId = localTtsAssetId(asset.bit, parentBit.hub);
			if (!assetId) {
				finalAssets.push(asset);
				finalDependencies.push(asset.bit);
				continue;
			}

			let assetBit: IBit;
			try {
				assetBit = await this.backend.apiState.get<IBit>(
					profile,
					`bit/${assetId}`,
				);
			} catch (error) {
				if (asset.required) throw error;
				finalAssets.push(asset);
				continue;
			}

			const plan = getTtsAssetRepairPlan(parentBit, asset, assetBit, force);
			let finalAssetBit = assetBit;
			if (plan?.shouldRepair) {
				finalAssetBit = await this.upsertAdminBit(profile, {
					...assetBit,
					dependency_tree_hash: "",
					download_link: plan.sourceUrl,
					file_name: plan.fileName,
					hash: "",
					repository: plan.repository,
				});
				repairedBits.push(finalAssetBit);
			}

			finalAssets.push({ ...asset, bit: bitDependencyRef(finalAssetBit) });
			finalDependencies.push(bitDependencyRef(finalAssetBit));
		}
		for (const dependency of parentBit.dependencies ?? []) {
			if (originalAssetIds.has(dependency.split(":").pop())) continue;
			finalDependencies.push(dependency);
		}

		const markerBit = await this.upsertAdminBit(
			profile,
			createTtsRepairMarkerBit(parentBit),
		);
		const replacementDraft = createTtsRepairReplacementBit(
			parentBit,
			finalAssets,
			finalDependencies,
			markerBit,
		);
		const replacementBit = await this.upsertAdminBit(profile, replacementDraft);
		await this.pushEnglishMeta(profile, {
			...replacementBit,
			meta: parentBit.meta,
		});
		await this.replaceCurrentProfileBitBestEffort(parentBit, replacementBit);

		if (replacementBit.id !== parentBit.id) {
			await this.deleteAdminBitBestEffort(profile, parentBit.id);
		}

		return { bits: [replacementBit, markerBit, ...repairedBits] };
	}
}
