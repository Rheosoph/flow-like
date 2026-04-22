import type {
	IBit,
	IBitPack,
	IBitState,
	IDownloadProgress,
} from "@tm9657/flow-like-ui";
import type { IBitSearchQuery } from "@tm9657/flow-like-ui/lib/schema/hub/bit-search-query";
import type { ISettingsProfile } from "@tm9657/flow-like-ui/types";
import { type WebBackendRef, apiGet, apiPost } from "./api-utils";

export class WebBitState implements IBitState {
	constructor(private readonly backend: WebBackendRef) {}

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
}
