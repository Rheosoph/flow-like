import type {
	IBit,
	IBitPack,
	IBitState,
	IDownloadProgress,
	IProfile,
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
import type { ISettingsProfile } from "@flow-like/flow-like-ui/types";
import { type WebBackendRef, apiGet, apiPost } from "./api-utils";
import { WebApiState } from "./api-state";

export class WebBitState implements IBitState {
	constructor(private readonly backend: WebBackendRef) {}

	private async upsertAdminBit(
		profile: IProfile,
		bit: IBit,
		routeId = bit.id,
	): Promise<IBit> {
		let finalBit = bit;
		let receivedFinalBit = false;
		const apiState = new WebApiState(this.backend);

		await apiState.stream<Record<string, unknown>>(
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

		const apiState = new WebApiState(this.backend);
		await apiState.put(profile, `admin/bit/${bit.id}/en`, meta);
	}

	private async deleteAdminBitBestEffort(
		profile: IProfile,
		bitId: string,
	): Promise<void> {
		try {
			const apiState = new WebApiState(this.backend);
			await apiState.del(profile, `admin/bit/${bitId}`);
		} catch (error) {
			console.warn(`Failed to delete replaced TTS bit ${bitId}`, error);
		}
	}

	private async replaceCurrentProfileBitBestEffort(
		profile: IProfile,
		oldBit: IBit,
		newBit: IBit,
	): Promise<void> {
		if (!profile.id) return;

		try {
			const currentBits = profile.bits ?? [];
			const hasOldBit = currentBits.some(
				(reference) => reference.split(":").pop() === oldBit.id,
			);
			if (!hasOldBit) return;

			const updatedBits = currentBits.map((reference) => {
				if (reference.split(":").pop() !== oldBit.id) return reference;
				return reference.includes(":") ? bitDependencyRef(newBit) : newBit.id;
			});

			await apiPost(
				`profile/${profile.id}`,
				{ bit_ids: Array.from(new Set(updatedBits)) },
				this.backend.auth,
			);
			this.backend.profile = {
				...profile,
				bits: Array.from(new Set(updatedBits)),
			};
		} catch (error) {
			console.warn(
				`Failed to replace current profile TTS bit ${oldBit.id}`,
				error,
			);
		}
	}

	async getInstalledBit(bits: IBit[]): Promise<IBit[]> {
		// In web mode, bits are managed server-side
		return bits;
	}

	async getPackFromBit(bit: IBit): Promise<{ bits: IBit[] }> {
		try {
			return await apiGet<{ bits: IBit[] }>(
				`bit/${bit.id}/dependencies`,
				this.backend.auth,
			);
		} catch {
			return { bits: [bit] };
		}
	}

	async downloadBit(
		bit: IBit,
		pack: IBitPack,
		cb?: (progress: IDownloadProgress[]) => void,
	): Promise<IBit[]> {
		// In web mode, bits are streamed from server - no local download needed
		cb?.([{ hash: bit.id, max: 100, downloaded: 100, path: "" }]);
		return [bit];
	}

	async deleteBit(bit: IBit): Promise<void> {
		// In web mode, bit deletion is handled by profile management
	}

	async getBit(id: string, hub?: string): Promise<IBit> {
		const params = hub ? `?hub=${encodeURIComponent(hub)}` : "";
		return apiGet<IBit>(`bit/${id}${params}`, this.backend.auth);
	}

	async addBit(bit: IBit, profile: ISettingsProfile): Promise<void> {
		const profileId = profile.hub_profile.id;
		if (!profileId) return;
		const currentBits = profile.hub_profile.bits ?? [];
		if (currentBits.some((id) => id.split(":").pop() === bit.id)) return;
		const updatedBits = [...currentBits, bit.id];
		await apiPost(
			`profile/${profileId}`,
			{ bit_ids: updatedBits },
			this.backend.auth,
		);
	}

	async removeBit(bit: IBit, profile: ISettingsProfile): Promise<void> {
		const profileId = profile.hub_profile.id;
		if (!profileId) return;
		const currentBits = profile.hub_profile.bits ?? [];
		const updatedBits = currentBits.filter(
			(id) => id.split(":").pop() !== bit.id,
		);
		await apiPost(
			`profile/${profileId}`,
			{ bit_ids: updatedBits },
			this.backend.auth,
		);
	}

	async getPackSize(bits: IBit[]): Promise<number> {
		// Size calculation not needed for web - streaming from server
		return 0;
	}

	async getBitSize(bit: IBit): Promise<number> {
		// Size calculation not needed for web - streaming from server
		return 0;
	}

	async searchBits(query: IBitSearchQuery): Promise<IBit[]> {
		try {
			const result = await apiPost<IBit[]>("bit", query, this.backend.auth);
			return result ?? [];
		} catch {
			return [];
		}
	}

	async isBitInstalled(bit: IBit): Promise<boolean> {
		// In web mode, bits are always "installed" (streamed from server)
		return true;
	}

	async getProfileBits(): Promise<IBit[]> {
		try {
			const profileId = this.backend.profile?.id;
			if (!profileId) return [];
			return (
				(await apiGet<IBit[]>(
					`profile/${profileId}/bits?limit=100`,
					this.backend.auth,
				)) ?? []
			);
		} catch {
			return [];
		}
	}

	async repairTtsBitAssets(bit: IBit, force = false): Promise<IBitPack> {
		const profile = this.backend.profile;
		if (!profile) {
			throw new Error("Profile is required to repair TTS bit assets");
		}

		const parentBit = await apiGet<IBit>(`bit/${bit.id}`, this.backend.auth);
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
				assetBit = await apiGet<IBit>(`bit/${assetId}`, this.backend.auth);
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
		await this.replaceCurrentProfileBitBestEffort(
			profile,
			parentBit,
			replacementBit,
		);

		if (replacementBit.id !== parentBit.id) {
			await this.deleteAdminBitBestEffort(profile, parentBit.id);
		}

		return { bits: [replacementBit, markerBit, ...repairedBits] };
	}
}
