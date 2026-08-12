import type {
	IBit,
	IBitPack,
	IBitState,
	IDownloadProgress,
	ISettingsProfile,
} from "@flow-like/flow-like-ui";
import type { IBitSearchQuery } from "@flow-like/flow-like-ui/lib/schema/hub/bit-search-query";

export class EmptyBitState implements IBitState {
	getInstalledBit(bits: IBit[]): Promise<IBit[]> {
		throw new Error("Method not implemented.");
	}
	getPackFromBit(bit: IBit): Promise<{ bits: IBit[] }> {
		throw new Error("Method not implemented.");
	}
	downloadBit(
		bit: IBit,
		pack: IBitPack,
		cb?: (progress: IDownloadProgress[]) => void,
	): Promise<IBit[]> {
		throw new Error("Method not implemented.");
	}
	deleteBit(bit: IBit): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getBit(id: string, hub?: string): Promise<IBit> {
		throw new Error("Method not implemented.");
	}
	addBit(bit: IBit, profile: ISettingsProfile): Promise<void> {
		throw new Error("Method not implemented.");
	}
	removeBit(bit: IBit, profile: ISettingsProfile): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getPackSize(bits: IBit[]): Promise<number> {
		throw new Error("Method not implemented.");
	}
	getBitSize(bit: IBit): Promise<number> {
		throw new Error("Method not implemented.");
	}
	searchBits(type: IBitSearchQuery): Promise<IBit[]> {
		throw new Error("Method not implemented.");
	}
	isBitInstalled(bit: IBit): Promise<boolean> {
		throw new Error("Method not implemented.");
	}
	getProfileBits(): Promise<IBit[]> {
		throw new Error("Method not implemented.");
	}
	repairTtsBitAssets(bit: IBit, force?: boolean): Promise<IBitPack> {
		throw new Error("Method not implemented.");
	}
	listCustomBits(): Promise<IBit[]> {
		throw new Error("Method not implemented.");
	}
	upsertCustomBit(bit: IBit, secrets?: Record<string, unknown>): Promise<IBit> {
		throw new Error("Method not implemented.");
	}
	deleteCustomBit(bitId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
