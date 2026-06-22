import { ChevronDown } from "lucide-react";
import type { RefObject } from "react";
import { useMemo } from "react";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
} from "../../../../components/ui/select";
import type { IBoard, IVariable } from "../../../../lib/schema/flow/board";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";

export function VarVariable({
	pin,
	value,
	boardRef,
	currentLayerId,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	boardRef?: RefObject<IBoard | undefined>;
	currentLayerId?: string;
	setValue: (value: unknown) => void;
}>) {
	const boardData = boardRef?.current;

	const allVariables = useMemo<Record<string, IVariable>>(() => {
		if (!boardData) return {};
		if (currentLayerId) {
			const layer = boardData.layers?.[currentLayerId];
			return { ...boardData.variables, ...layer?.variables };
		}
		return { ...boardData.variables };
	}, [boardData, currentLayerId]);

	return (
		<div className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden">
			<Select
				disabled={!boardData}
				defaultValue={parseUint8ArrayToJson(value)}
				value={parseUint8ArrayToJson(value)}
				onValueChange={(value) => setValue(convertJsonToUint8Array(value))}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="w-fit! max-w-full! p-0 border-0 text-xs bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden"
				>
					<small className="text-start text-[10px] m-0! truncate">
						{!boardData && "Board unavailable"}
						{boardData &&
							(allVariables[parseUint8ArrayToJson(value)]?.name ??
								"No Variable Selected")}
					</small>
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground shrink-0" />
				</SelectTrigger>
				<SelectContent className="bg-background">
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{Object.values(allVariables).map((variable) => (
							<SelectItem key={variable.id} value={variable.id}>
								{variable.name}
							</SelectItem>
						))}
					</SelectGroup>
				</SelectContent>
			</Select>
		</div>
	);
}
