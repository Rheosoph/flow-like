import { useCallback, useEffect, useRef, useState } from "react";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
} from "../../../../components/ui/select";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import {
	type FlowSelectorData,
	type FlowSelectorDataRef,
	bitDisplayName,
	bitRef,
} from "../../flow-selector-data";
import { PinEditorRow, PinSelectTrigger } from "../pin-chrome";

export function BitVariable({
	pin,
	value,
	setValue,
	selectorDataRef,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	setValue: (value: unknown) => void;
	selectorDataRef?: FlowSelectorDataRef;
}>) {
	const [open, setOpen] = useState(false);
	const [, refreshSnapshot] = useState(0);
	const requestedCacheRef = useRef<FlowSelectorData | undefined>(undefined);

	const parsedValue = parseUint8ArrayToJson(value);
	const selectedValue =
		typeof parsedValue === "string" ? parsedValue : undefined;
	const bits = selectorDataRef?.current.bitOptions ?? [];
	const selectedBit =
		selectedValue === undefined
			? undefined
			: selectorDataRef?.current.bitsByRef.get(selectedValue);
	const selectedLabel =
		bitDisplayName(selectedBit) ??
		(selectedValue === undefined ? undefined : selectedValue.split(":").pop());
	const loading = open && (selectorDataRef?.current.bitsLoading ?? false);
	const needsBitLookup = selectedValue !== undefined && !selectedBit;

	// A stored pin value is only a bit reference, so showing a name needs the same
	// profile bit list the dropdown uses. The shared loader collapses this into a
	// single request for the whole board; the cache flags stop every other bit pin
	// from re-requesting an in-flight, finished, or failed load, and keying the
	// guard on the cache object re-arms it when the board swaps caches.
	useEffect(() => {
		const cache = selectorDataRef?.current;
		if (!cache || !needsBitLookup) return;
		if (cache.bitsLoaded || cache.bitsLoading || cache.bitsError) return;
		if (requestedCacheRef.current === cache) return;

		requestedCacheRef.current = cache;
		cache.loadBits().finally(() => refreshSnapshot((version) => version + 1));
	});

	const handleOpenChange = useCallback(
		(isOpen: boolean) => {
			setOpen(isOpen);
			if (!isOpen) return;

			refreshSnapshot((version) => version + 1);
			const loadPromise = selectorDataRef?.current.loadBits(true);
			loadPromise?.finally(() => refreshSnapshot((version) => version + 1));
		},
		[selectorDataRef],
	);

	return (
		<PinEditorRow>
			<Select
				open={open}
				onOpenChange={handleOpenChange}
				value={selectedValue}
				onValueChange={(v) => setValue(convertJsonToUint8Array(v))}
			>
				<PinSelectTrigger
					label={selectedLabel ?? (loading ? "Loading" : "Select a bit")}
				/>
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{bits.map((bit) => {
							const bitId = bitRef(bit);
							return (
								<SelectItem key={bitId} value={bitId}>
									{bitDisplayName(bit) ?? bit.id}
								</SelectItem>
							);
						})}
					</SelectGroup>
				</SelectContent>
			</Select>
		</PinEditorRow>
	);
}
