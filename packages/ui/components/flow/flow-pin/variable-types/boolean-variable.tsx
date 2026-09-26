import { Checkbox } from "../../../../components/ui/checkbox";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import { VariableDescription } from "./default-text";

export function BooleanVariable({
	pin,
	value,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	setValue: (value: any) => void;
}>) {
	return (
		<>
			<VariableDescription pin={pin} />
			<Checkbox
				checked={parseUint8ArrayToJson(value) ?? false}
				onCheckedChange={(checked) =>
					setValue(convertJsonToUint8Array(checked))
				}
				className="size-2.5 shrink-0 rounded-[2px] [&_svg]:size-2"
			/>
		</>
	);
}
