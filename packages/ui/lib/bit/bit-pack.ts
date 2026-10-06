import type { IBackendState } from "../../state/backend-state";
import type { IBit } from "../schema/bit/bit";
import type { IBitPack } from "../schema/bit/bit-pack";
import { Bit } from "./bit";

export class BitPack implements IBitPack {
	bits: IBit[] = [];
	backend: IBackendState | undefined;

	public setBackend(backend?: IBackendState) {
		this.backend = backend;
		return this;
	}

	async get_installed(): Promise<Bit[]> {
		const installed = await this.backend?.bitState?.getInstalledBit(this.bits);
		if (!installed) {
			throw new Error("No installed bits found");
		}
		return installed.map((bit) => Bit.fromObject(bit).setBackend(this.backend));
	}

	async size(): Promise<number> {
		const size = await this.backend?.bitState?.getPackSize(this.bits);
		if (!size) {
			throw new Error("No size found");
		}
		return size;
	}

	public static fromJson(json: string): BitPack {
		const object = JSON.parse(json);
		return BitPack.fromObject(object);
	}

	public toJson(): string {
		const object = this.toObject();
		return JSON.stringify(object);
	}

	public static fromObject(obj: IBitPack): BitPack {
		const bitpack = new BitPack();
		const values = bitpack as unknown as Record<string, unknown>;

		for (const key of Object.keys(obj)) {
			values[key] = obj[key];
		}

		return bitpack;
	}

	public toObject(): IBitPack {
		const obj: Record<string, unknown> = {};
		const values = this as unknown as Record<string, unknown>;
		for (const key of Object.keys(this)) {
			if (typeof values[key] !== "function") {
				obj[key] = values[key];
			}
		}
		return obj as IBitPack;
	}
}
