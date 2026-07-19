import { ChevronDown } from "lucide-react";
import type { RefObject } from "react";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
} from "../../../../components/ui/select";
import type { IBoard } from "../../../../lib/schema/flow/board";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";

export function FnVariable({
	pin,
	value,
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: unknown) => void;
}>) {
	const boardData = boardRef?.current;

	return (
		<div className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden">
			<Select
				disabled={!boardData}
				defaultValue={parseUint8ArrayToJson(value)}
				value={parseUint8ArrayToJson(value)}
				onValueChange={(value) => setValue(convertJsonToUint8Array(value))}
				onOpenChange={async () => {
					// const nodes = flow.getNodes();
				}}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="w-fit! max-w-full! p-0 border-0 text-xs bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden"
				>
					<small className="text-start text-[10px] m-0! truncate">
						{!boardData && "Board unavailable"}
						{boardData &&
							(boardData.nodes?.[parseUint8ArrayToJson(value)]?.friendly_name ??
								"No Function Selected")}
					</small>
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground shrink-0" />
				</SelectTrigger>
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{Object.values(boardData?.nodes ?? {})
							?.filter((node) => node.start)
							.map((node) => {
								return (
									<SelectItem key={node.id} value={node.id}>
										{node.friendly_name ?? node.name}
									</SelectItem>
								);
							})}
					</SelectGroup>
				</SelectContent>
			</Select>
		</div>
	);
}
