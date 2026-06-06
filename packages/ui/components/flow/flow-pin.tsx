"use client";
import { Handle, type HandleType, Position, useReactFlow } from "@xyflow/react";
import { EllipsisVerticalIcon, GripIcon, ListIcon, Trash2 } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke } from "../../hooks";
import { updateNodeCommand } from "../../lib";
import type { ILayer } from "../../lib/schema/flow/board";
import type { INode } from "../../lib/schema/flow/node";
import { type IPin, IPinType, IValueType } from "../../lib/schema/flow/pin";
import { useBackendStore } from "../../state/backend-state";
import { useUndoRedo } from "./flow-history";
import { PinEdit } from "./flow-pin/pin-edit";
import type { FlowSelectorDataRef } from "./flow-selector-data";
import { typeToColor } from "./utils";

/** A Handle that shows a small inner dot while keeping a larger hitbox. */
const SmallDotHandle = memo(function SmallDotHandle({
	dotColor,
	showBorderWhenTransparent = true,
	dotSize = 5,
	isExecution = false,
	...props
}: Omit<React.ComponentProps<typeof Handle>, "children"> & {
	/** Visual color of the inner dot. Use transparent to hide fill. */
	dotColor: string;
	/** Draw a 1px border when dot is transparent (for Execution pins, etc.). */
	showBorderWhenTransparent?: boolean;
	/** Visual size of the inner dot (defaults to 5). */
	dotSize?: number;
	/** Is this an execution pin? */
	isExecution?: boolean;
} & { children?: React.ReactNode }) {
	const { className, style, children } = props as any;

	const size = dotSize;
	const isTransparent = dotColor === "transparent";
	const visualSize = 7; // Data pins size

	return (
		<Handle
			{...props}
			className={`relative ${className ?? ""}`}
			style={{
				width: 12,
				height: 12,
				background: "transparent",
				border: "transparent",
				padding: 0,
				...(style ?? {}),
			}}
		>
			{/* centered visual dot that doesn't catch the mouse */}
			{!isExecution && (
				<span
					className="pointer-events-none absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rounded-full"
					style={{
						width: visualSize,
						height: visualSize,
						background: isTransparent
							? "transparent"
							: `
								radial-gradient(
									circle at 35% 35%,
									color-mix(in oklch, ${dotColor} 100%, white 25%),
									${dotColor} 70%
								)
							`,
						border: `1px solid ${dotColor}`,
						boxShadow: `
							0 0 4px color-mix(in oklch, ${dotColor} 25%, transparent),
							inset 0 0.5px 1px color-mix(in oklch, white 20%, transparent)
						`,
					}}
				/>
			)}
			{children}
		</Handle>
	);
});

function FlowPinInnerComponent({
	pin,
	boardId,
	appId,
	node,
	skipOffset,
	onPinRemove,
	version,
	currentLayerId,
	selectorDataRef,
	selectorDataVersion,
}: Readonly<{
	pin: IPin;
	boardId: string;
	appId: string;
	node: INode | ILayer;
	skipOffset?: boolean;
	onPinRemove?: (pin: IPin) => Promise<void>;
	version?: [number, number, number];
	currentLayerId?: string;
	selectorDataRef?: FlowSelectorDataRef;
	selectorDataVersion?: number;
}>) {
	const { pushCommand } = useUndoRedo(appId, boardId);
	const invalidate = useInvalidateInvoke();
	const { getNode } = useReactFlow();

	const [defaultValue, setDefaultValue] = useState(pin.default_value);

	// compute vertical offsets + color; we no longer rely on Handle background
	const handleStyle = useMemo(() => {
		// keep your existing offsets/positions exactly as before
		if (node?.name === "reroute") {
			return {
				background: "transparent",
			};
		}

		if (skipOffset) {
			return {
				marginTop: "1.75rem",
				top: (pin.index - 1) * 15,
			} as React.CSSProperties;
		}

		return {
			marginTop: "1.75rem",
			top: (pin.index - 1) * 15,
		} as React.CSSProperties;
	}, [pin.index, node?.name, skipOffset]);

	// visible dot color follows your previous logic
	const dotColor = useMemo(
		() =>
			pin.data_type === "Execution" || pin.value_type !== IValueType.Normal
				? "transparent"
				: typeToColor(pin.data_type),
		[pin.data_type, pin.value_type],
	);

	const iconStyle = useMemo(
		() => ({
			color: typeToColor(pin.data_type),
			marginLeft: pin.pin_type === IPinType.Input ? "0.4rem" : "0.4rem",
			backgroundColor:
				"var(--xy-node-background-color, var(--xy-node-background-color-default))",
		}),
		[pin.data_type, pin.pin_type],
	);

	const shouldRenderPinEdit = useMemo(
		() =>
			pin.name !== "exec_in" &&
			pin.name !== "exec_out" &&
			node?.name !== "reroute",
		[pin.name, node?.name],
	);

	const pinEditContainerClassName = useMemo(
		() =>
			`flex flex-row items-center gap-1 max-w-[10rem] ${
				pin.pin_type === "Input" ? "ml-2.5" : "translate-x-[calc(-100%+0.2rem)]"
			}`,
		[pin.pin_type],
	);

	const refetchBoard = useCallback(async () => {
		const backend = useBackendStore.getState().backend;
		if (!backend) return;
		invalidate(backend.boardState.getBoard, [appId, boardId]);
	}, [appId, boardId, invalidate]);

	const updateNode = useCallback(
		async (value: any) => {
			if (typeof version !== "undefined") {
				return;
			}

			if (node.nodes) return;
			const currentNode = getNode(node.id);
			if (!currentNode) return;
			const translatedNode = currentNode?.data?.node as INode | undefined;
			if (!translatedNode) {
				toast.error("Node not found");
				return;
			}
			if (value === undefined) return;
			if (value === null) return;
			if (value === pin.default_value) return;
			const backend = useBackendStore.getState().backend;
			if (!backend) return;
			const command = updateNodeCommand({
				node: {
					...translatedNode,
					hash: undefined,
					coordinates: [currentNode.position.x, currentNode.position.y, 0],
					pins: {
						...translatedNode.pins,
						[pin.id]: { ...pin, default_value: value },
					},
				},
			});

			const result = await backend.boardState.executeCommand(
				currentNode.data.appId as string,
				boardId,
				command,
			);
			await pushCommand(result, false);
			await refetchBoard();
		},
		[pin.id, refetchBoard, boardId, pushCommand, getNode, node, pin, version],
	);

	useEffect(() => {
		setDefaultValue(pin.default_value);
	}, [pin]);

	const pinTypeProps = useMemo(
		() => ({
			type: pin.pin_type === "Input" ? "target" : "source",
			position: pin.pin_type === "Input" ? Position.Left : Position.Right,
		}),
		[pin.pin_type],
	);

	// Memoized pin icons
	const pinIcons = useMemo(
		() => (
			<>
				{pin.data_type === "Execution" && node?.name !== "reroute" && (
					<div
						className="absolute left-1/2 top-1/2 pointer-events-none"
						style={{
							width: 8,
							height: 8,
							transform: "translate(-50%, -50%) rotate(45deg)",
							background: `
								linear-gradient(
									135deg,
									color-mix(in oklch, var(--foreground) 100%, white 15%),
									var(--foreground) 70%
								)
							`,
							border: "1.5px solid var(--foreground)",
							borderRadius: "1.5px",
							boxShadow: `
								0 0 5px color-mix(in oklch, var(--foreground) 25%, transparent),
								inset 0 0.5px 1px color-mix(in oklch, white 15%, transparent)
							`,
						}}
					/>
				)}
				{pin.value_type === IValueType.Array && (
					<GripIcon
						strokeWidth={3}
						className={`w-2 h-2 absolute left-0 -translate-x-[50%] pointer-events-none bg-background ${pin.pin_type === IPinType.Input ? "ml-0.5" : "ml-1"}`}
						style={iconStyle}
					/>
				)}
				{pin.value_type === IValueType.HashSet && (
					<EllipsisVerticalIcon
						strokeWidth={3}
						className="w-2 h-2 absolute left-0 -translate-x-[50%] pointer-events-none bg-background"
						style={iconStyle}
					/>
				)}
				{pin.value_type === IValueType.HashMap && (
					<ListIcon
						strokeWidth={3}
						className="w-2 h-2 absolute left-0 -translate-x-[50%] pointer-events-none"
						style={iconStyle}
					/>
				)}
			</>
		),
		[pin.data_type, pin.value_type, iconStyle, node?.name, pin.pin_type],
	);

	const isExecution = useMemo(
		() => pin.data_type === "Execution",
		[pin.data_type],
	);

	return (
		<SmallDotHandle
			type={pinTypeProps.type as HandleType}
			position={pinTypeProps.position}
			id={pin.id}
			style={handleStyle}
			className="flex flex-row items-center gap-1 group"
			dotColor={dotColor}
			showBorderWhenTransparent
			isExecution={isExecution}
		>
			{pinIcons}
			{shouldRenderPinEdit && (
				<div className={pinEditContainerClassName}>
					<PinEdit
						nodeId={node.id}
						pin={pin}
						appId={appId}
						boardId={boardId}
						defaultValue={defaultValue}
						changeDefaultValue={setDefaultValue}
						saveDefaultValue={async (value) => {
							await updateNode(value);
						}}
						currentLayerId={currentLayerId}
						selectorDataRef={selectorDataRef}
						selectorDataVersion={selectorDataVersion}
					/>
					{pin.dynamic && onPinRemove && (
						<button
							className="opacity-0 bg-background border p-0.5 rounded-full group-hover:opacity-100 hover:text-primary"
							title="Delete Pin"
							onClick={() => onPinRemove(pin)}
						>
							<Trash2 className="w-1.5 h-1.5" />
						</button>
					)}
				</div>
			)}
			{!shouldRenderPinEdit && onPinRemove && pin.dynamic && (
				<button
					className={`opacity-0 bg-background border p-0.5 rounded-full group-hover:opacity-100 hover:text-primary ${
						pin.pin_type === IPinType.Input
							? "ml-2.5"
							: "mr-2.5 right-0 absolute"
					}`}
					title="Delete Pin"
					onClick={() => onPinRemove(pin)}
				>
					<Trash2 className="w-1.5 h-1.5" />
				</button>
			)}
		</SmallDotHandle>
	);
}

function pinPropsAreEqual(prevProps: any, nextProps: any) {
	if (
		prevProps.boardId !== nextProps.boardId ||
		prevProps.node?.id !== nextProps.node?.id ||
		prevProps.pin.id !== nextProps.pin.id ||
		prevProps.pin.index !== nextProps.pin.index ||
		prevProps.pin.name !== nextProps.pin.name ||
		prevProps.pin.friendly_name !== nextProps.pin.friendly_name ||
		prevProps.pin.default_value !== nextProps.pin.default_value ||
		prevProps.pin.data_type !== nextProps.pin.data_type ||
		prevProps.pin.value_type !== nextProps.pin.value_type ||
		prevProps.pin.pin_type !== nextProps.pin.pin_type ||
		prevProps.pin.schema !== nextProps.pin.schema
	) {
		return false;
	}

	if (prevProps.selectorDataRef !== nextProps.selectorDataRef) {
		return false;
	}

	if (prevProps.selectorDataVersion !== nextProps.selectorDataVersion) {
		return false;
	}

	// Compare connection state (connected_to / depends_on) by length + contents
	const prevConn = prevProps.pin.connected_to;
	const nextConn = nextProps.pin.connected_to;
	if (prevConn.length !== nextConn.length) return false;
	for (let i = 0; i < prevConn.length; i++) {
		if (prevConn[i] !== nextConn[i]) return false;
	}

	const prevDeps = prevProps.pin.depends_on;
	const nextDeps = nextProps.pin.depends_on;
	if (prevDeps.length !== nextDeps.length) return false;
	for (let i = 0; i < prevDeps.length; i++) {
		if (prevDeps[i] !== nextDeps[i]) return false;
	}

	return true;
}

export const FlowPinInner = memo(FlowPinInnerComponent, pinPropsAreEqual);

function FlowPin({
	pin,
	boardId,
	appId,
	node,
	onPinRemove,
	skipOffset,
	version,
	currentLayerId,
	selectorDataRef,
	selectorDataVersion,
}: Readonly<{
	pin: IPin;
	boardId: string;
	appId: string;
	node: INode | ILayer;
	skipOffset?: boolean;
	onPinRemove?: (pin: IPin) => Promise<void>;
	version?: [number, number, number];
	currentLayerId?: string;
	selectorDataRef?: FlowSelectorDataRef;
	selectorDataVersion?: number;
}>) {
	if (pin.dynamic) {
		return (
			<FlowPinInner
				key={pin.id}
				appId={appId}
				pin={pin}
				boardId={boardId}
				node={node}
				skipOffset={skipOffset}
				onPinRemove={onPinRemove}
				version={version}
				currentLayerId={currentLayerId}
				selectorDataRef={selectorDataRef}
				selectorDataVersion={selectorDataVersion}
			/>
		);
	}

	return (
		<FlowPinInner
			key={pin.id}
			appId={appId}
			pin={pin}
			boardId={boardId}
			node={node}
			skipOffset={skipOffset}
			version={version}
			currentLayerId={currentLayerId}
			selectorDataRef={selectorDataRef}
			selectorDataVersion={selectorDataVersion}
		/>
	);
}

const pin = memo(FlowPin);
export { pin as FlowPin };
