import type { MutableRefObject } from "react";
import type { IBit } from "../../lib/schema/bit/bit";
import type { SurfaceComponent } from "../a2ui/types";

export interface FlowElementOption {
	id: string;
	rawId: string;
	type: string;
	label: string;
	pageName?: string;
	pagePath?: string;
}

export interface FlowSelectorData {
	elementOptions: FlowElementOption[];
	elementsLoaded: boolean;
	elementsLoading: boolean;
	elementsError?: unknown;
	loadElements: () => Promise<FlowElementOption[]>;

	bitOptions: IBit[];
	bitsByRef: Map<string, IBit>;
	bitsLoaded: boolean;
	bitsLoading: boolean;
	bitsError?: unknown;
	loadBits: () => Promise<IBit[]>;
}

export type FlowSelectorDataRef = MutableRefObject<FlowSelectorData>;

export function createEmptyFlowSelectorData(): FlowSelectorData {
	return {
		elementOptions: [],
		elementsLoaded: false,
		elementsLoading: false,
		loadElements: async () => [],
		bitOptions: [],
		bitsByRef: new Map(),
		bitsLoaded: false,
		bitsLoading: false,
		loadBits: async () => [],
	};
}

export function flattenPageElements(
	components: SurfaceComponent[],
): FlowElementOption[] {
	const elements: FlowElementOption[] = [];

	for (const component of components) {
		const componentObj = component.component;
		if (typeof componentObj === "object" && componentObj !== null) {
			const type =
				((componentObj as unknown as Record<string, unknown>).type as string) ||
				"unknown";
			elements.push({
				id: component.id,
				type,
				label: component.id,
				rawId: component.id,
			});
		}
	}

	return elements;
}

export function bitRef(bit: IBit): string {
	return `${bit.hub}:${bit.id}`;
}

export function bitDisplayName(bit?: IBit): string | undefined {
	return (
		bit?.meta?.en?.name ??
		(bit?.meta ? Object.values(bit.meta)[0]?.name : undefined)
	);
}

export function indexBitsByRef(bits: IBit[]): Map<string, IBit> {
	const byRef = new Map<string, IBit>();

	for (const bit of bits) {
		byRef.set(bit.id, bit);
		byRef.set(bitRef(bit), bit);
	}

	return byRef;
}
