import { ChevronDown } from "lucide-react";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
	SelectValue,
} from "../../../../components/ui/select";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";

export function EnumVariable({
	pin,
	value,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	setValue: (value: unknown) => void;
}>) {
	return (
		<div
			className="flex flex-row items-center justify-start ml-1"
			onMouseDown={(e) => e.stopPropagation()}
			onPointerDown={(e) => e.stopPropagation()}
		>
			<Select
				defaultValue={parseUint8ArrayToJson(value)}
				value={parseUint8ArrayToJson(value)}
				onValueChange={(value) => setValue(convertJsonToUint8Array(value))}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="!w-fit !max-w-fit p-0 border-0 text-xs !bg-card text-nowrap text-start max-h-fit h-4 gap-0.5 flex-row items-center"
				>
					<SelectValue placeholder={`Select ${pin.friendly_name}`} />
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground" />
				</SelectTrigger>
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
		</div>
	);
}
