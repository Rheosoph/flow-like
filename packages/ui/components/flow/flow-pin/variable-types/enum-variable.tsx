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
import { PinEditorRow, PinSelectTrigger } from "../pin-chrome";

export function EnumVariable({
	pin,
	value,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	setValue: (value: unknown) => void;
}>) {
	const selected = parseUint8ArrayToJson(value);
	const label =
		typeof selected === "string" ? selected : `Select ${pin.friendly_name}`;
	return (
		<PinEditorRow>
			<Select
				defaultValue={selected}
				value={selected}
				onValueChange={(value) => setValue(convertJsonToUint8Array(value))}
			>
				<PinSelectTrigger label={label} />
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{pin.options?.valid_values?.map((option) => {
							return (
								<SelectItem key={option} value={option}>
									{option}
								</SelectItem>
							);
						})}
					</SelectGroup>
				</SelectContent>
			</Select>
		</PinEditorRow>
	);
}
