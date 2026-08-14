import { useTranslation } from "@flow-like/locales";
import { Handle, Position } from "@xyflow/react";
import { EllipsisVerticalIcon, GripIcon, ListIcon } from "lucide-react";
import { useEffect, useState } from "react";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuLabel,
	ContextMenuTrigger,
} from "../../../components/ui/context-menu";
import type { INode } from "../../../lib/schema/flow/node";
import { type IPin, IValueType } from "../../../lib/schema/flow/pin";
import { DynamicImage } from "../../ui/dynamic-image";
import { typeToColor } from "../utils";
import { PinPreviewEdit } from "./preview-pin-edit";

export function FlowPinInner({
	pin,
	index,
	boardId,
	node,
}: Readonly<{ pin: IPin; index: number; boardId: string; node: INode }>) {
	const [defaultValue, setDefaultValue] = useState(pin.default_value);

	useEffect(() => {
		setDefaultValue(pin.default_value);
	}, [pin]);

	return (
		<Handle
			type={pin.pin_type === "Input" ? "target" : "source"}
			position={pin.pin_type === "Input" ? Position.Left : Position.Right}
			id={pin.id}
			style={{
				marginTop: "1.75rem",
				top: index * 15,
				background:
					pin.data_type === "Execution" || pin.value_type !== IValueType.Normal
						? "transparent"
						: typeToColor(pin.data_type),
			}}
			className="flex flex-row items-center gap-1"
		>
			{pin.data_type === "Execution" && (
				<DynamicImage
					url="/flow/pin.svg"
					className="w-2 h-2 absolute left-0 -translate-x-[15%] pointer-events-none bg-foreground"
				/>
			)}
			{pin.value_type === IValueType.Array && (
				<GripIcon
					strokeWidth={3}
					className={
						"w-2 h-2 absolute left-0 -translate-x-[30%] pointer-events-none bg-background"
					}
					style={{
						color: typeToColor(pin.data_type),
						backgroundColor:
							"var(--xy-node-background-color, var(--xy-node-background-color-default))",
					}}
				/>
			)}
			{pin.value_type === IValueType.HashSet && (
				<EllipsisVerticalIcon
					strokeWidth={3}
					className={
						"w-2 h-2 absolute left-0 -translate-x-[30%] pointer-events-none bg-background"
					}
					style={{
						color: typeToColor(pin.data_type),
						backgroundColor:
							"var(--xy-node-background-color, var(--xy-node-background-color-default))",
					}}
				/>
			)}
			{pin.value_type === IValueType.HashMap && (
				<ListIcon
					strokeWidth={3}
					className={
						"w-2 h-2 absolute left-0 -translate-x-[30%] pointer-events-none"
					}
					style={{
						color: typeToColor(pin.data_type),
						backgroundColor:
							"var(--xy-node-background-color, var(--xy-node-background-color-default))",
					}}
				/>
			)}
			{pin.name !== "exec_in" &&
				pin.name !== "exec_out" &&
				pin.name !== "var_ref" && (
					<div
						className={`flex flex-row items-center transition-all gap-1 max-w-40 ${pin.pin_type === "Input" ? "ml-2" : "translate-x-[-115%]"}`}
					>
						<PinPreviewEdit
							pin={pin}
							defaultValue={defaultValue}
							changeDefaultValue={(value) => {
								setDefaultValue(value);
							}}
						/>
					</div>
				)}
		</Handle>
	);
}

export function FlowPreviewPin({
	pin,
	index,
	boardId,
	node,
	onPinRemove,
}: Readonly<{
	pin: IPin;
	index: number;
	boardId: string;
	node: INode;
	onPinRemove: (pin: IPin) => Promise<void>;
}>) {
	const { t } = useTranslation("flow");
	if (pin.dynamic)
		return (
			<ContextMenu>
				<ContextMenuTrigger>
					<FlowPinInner pin={pin} index={index} boardId={boardId} node={node} />
				</ContextMenuTrigger>
				<ContextMenuContent>
					<ContextMenuLabel>{t('pinActions', 'Pin Actions')}</ContextMenuLabel>
					<ContextMenuItem
						onClick={() => {
							onPinRemove(pin);
						}}
					>
						{t('removePin2', 'Remove Pin')}
					</ContextMenuItem>
				</ContextMenuContent>
			</ContextMenu>
		);

	return FlowPinInner({ pin, index, boardId, node });
}
