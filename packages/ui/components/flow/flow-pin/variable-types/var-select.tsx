import type { RefObject } from "react";
import { useMemo } from "react";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
} from "../../../../components/ui/select";
import type { IBoard, IVariable } from "../../../../lib/schema/flow/board";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import { PinEditorRow, PinSelectTrigger } from "../pin-chrome";

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
		<PinEditorRow>
			<Select
				disabled={!boardData}
				defaultValue={parseUint8ArrayToJson(value)}
				value={parseUint8ArrayToJson(value)}
				onValueChange={(value) => setValue(convertJsonToUint8Array(value))}
			>
				<PinSelectTrigger
					label={
						boardData
							? (allVariables[parseUint8ArrayToJson(value)]?.name ??
								"No Variable Selected")
							: "Board unavailable"
					}
				/>
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
		</PinEditorRow>
	);
}
