import type { RefObject } from "react";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
} from "../../../../components/ui/select";
import type { IBoard } from "../../../../lib/schema/flow/board";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import { PinEditorRow, PinSelectTrigger } from "../pin-chrome";

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
		<PinEditorRow>
			<Select
				disabled={!boardData}
				defaultValue={parseUint8ArrayToJson(value)}
				value={parseUint8ArrayToJson(value)}
				onValueChange={(value) => setValue(convertJsonToUint8Array(value))}
				onOpenChange={async () => {
					// const nodes = flow.getNodes();
				}}
			>
				<PinSelectTrigger
					label={
						boardData
							? (boardData.nodes?.[parseUint8ArrayToJson(value)]
									?.friendly_name ?? "No Function Selected")
							: "Board unavailable"
					}
				/>
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
		</PinEditorRow>
	);
}
