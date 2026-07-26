import type { IBit, IBitPack, IDownloadProgress } from "../../lib";
import type { IBitSearchQuery } from "../../lib/schema/hub/bit-search-query";
import type { ISettingsProfile } from "../../types";

export interface IBitState {
	getInstalledBit(bits: IBit[]): Promise<IBit[]>;
	getPackFromBit(bit: IBit): Promise<{
		bits: IBit[];
	}>;
	downloadBit(
		bit: IBit,
		pack: IBitPack,
		cb?: (progress: IDownloadProgress[]) => void,
	): Promise<IBit[]>;
	deleteBit(bit: IBit): Promise<void>;
	getBit(id: string, hub?: string): Promise<IBit>;
	addBit(bit: IBit, profile: ISettingsProfile): Promise<void>;
	removeBit(bit: IBit, profile: ISettingsProfile): Promise<void>;
	getPackSize(bits: IBit[]): Promise<number>;
	getBitSize(bit: IBit): Promise<number>;
	searchBits(type: IBitSearchQuery): Promise<IBit[]>;
	isBitInstalled(bit: IBit): Promise<boolean>;
	getProfileBits(): Promise<IBit[]>;
	repairTtsBitAssets(bit: IBit, force?: boolean): Promise<IBitPack>;
	/** User-owned private model bits (custom providers / local HF models). */
	listCustomBits(): Promise<IBit[]>;
	/**
	 * Creates or updates a user-owned custom bit. Secret provider params
	 * (api_key, ...) travel via `secrets` and are stored encrypted server-side;
	 * on desktop they are kept in the local profile for offline execution.
	 */
	upsertCustomBit(bit: IBit, secrets?: Record<string, unknown>): Promise<IBit>;
	deleteCustomBit(bitId: string): Promise<void>;
}
