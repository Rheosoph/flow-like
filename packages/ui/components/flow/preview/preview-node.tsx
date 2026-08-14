"use client";

import { useTranslation } from "@flow-like/locales";
import { useDebounce } from "@uidotdev/usehooks";
import type { Node, NodeProps } from "@xyflow/react";
import {
	ClockIcon,
	PlayCircleIcon,
	SquareCheckIcon,
	WorkflowIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { useEffect, useRef, useState } from "react";
import PuffLoader from "react-spinners/PuffLoader";
import { toastSuccess } from "../../../lib/messages";
import type { INode } from "../../../lib/schema/flow/node";
import type { IPin } from "../../../lib/schema/flow/pin";
import type { ITrace } from "../../../lib/schema/flow/run";
import { useRunExecutionStore } from "../../../state/run-execution-state";
import { DynamicImage } from "../../ui/dynamic-image";
import { FlowPinAction } from "../flow-node/flow-node-pin-action";
import { FlowPreviewPin } from "./preview-pin";

export interface IPinAction {
	action: "create";
	pin: IPin;
	onAction: (pin: IPin) => Promise<void>;
}

export type FlowNode = Node<
	{
		node: INode;
		boardId: string;
		traces: ITrace[];
	},
	"node"
>;

export function PreviewFlowNode(props: NodeProps<FlowNode>) {
	const { t } = useTranslation("flow");
	const { resolvedTheme } = useTheme();
	const [executing, setExecuting] = useState(false);
	const [isExec, setIsExec] = useState(false);
	const [inputPins, setInputPins] = useState<(IPin | IPinAction)[]>([]);
	const [outputPins, setOutputPins] = useState<(IPin | IPinAction)[]>([]);
	const { runs } = useRunExecutionStore();
	const [executionState, setExecutionState] = useState<
		"done" | "running" | "none"
	>("none");
	const debouncedExecutionState = useDebounce(executionState, 100);
	const div = useRef<HTMLDivElement>(null);
	function sortPins(a: IPin, b: IPin) {
		// Step 1: Compare by type - Input comes before Output
		if (a.pin_type === "Input" && b.pin_type === "Output") return -1;
		if (a.pin_type === "Output" && b.pin_type === "Input") return 1;

		// Step 2: If types are the same, compare by index
		return a.index - b.index;
	}

	useEffect(() => {
		const height = Math.max(inputPins.length, outputPins.length);
		if (div.current)
			div.current.style.height = `calc(${height * 15}px + 1.25rem + 0.5rem)`;
	}, [inputPins, outputPins]);

	useEffect(() => {
		parsePins(Object.values(props.data.node?.pins || []));
	}, [props.data.node.pins, props.positionAbsoluteX, props.positionAbsoluteY]);

	useEffect(() => {
		let isRunning = false;
		let already_executed = false;

		for (const [_, run] of runs) {
			if (run.nodes.has(props.id)) {
				isRunning = true;
				break;
			}

			if (run.already_executed.has(props.id)) {
				already_executed = true;
			}
		}

		if (isRunning) {
			setExecutionState("running");
			return;
		}

		if (already_executed) {
			setExecutionState("done");
			return;
		}

		setExecutionState("none");
	}, [runs, props.id]);

	function parsePins(pins: IPin[]) {
		const inputPins: (IPin | IPinAction)[] = [];
		const outputPins: (IPin | IPinAction)[] = [];
		let isExec = false;

		let pastPinWithCount: [string, number, IPin | undefined] = [
			"",
			0,
			undefined,
		];

		Object.values(pins)
			.sort(sortPins)
			.forEach((pin, index) => {
				if (pin.data_type === "Execution") isExec = true;

				const pastPinId = `${pin.name}_${pin.pin_type}`;

				if (pastPinWithCount[0] === pastPinId) {
					pastPinWithCount[1] += 1;
				}

				// update to past pin information
				if (pastPinWithCount[0] !== pastPinId)
					pastPinWithCount = [pastPinId, 0, pin];
				pin = { ...pin, dynamic: pastPinWithCount[1] > 1 };

				if (pin.pin_type === "Input") {
					inputPins.push(pin);
				} else {
					outputPins.push(pin);
				}
			});

		setInputPins(inputPins);
		setOutputPins(outputPins);
		setIsExec(isExec);
	}

	function isPinAction(pin: IPin | IPinAction): pin is IPinAction {
		return typeof (pin as IPinAction).onAction === "function";
	}

	return (
		<div
			key={`${props.id}__node`}
			ref={div}
			className={`bg-card p-2 react-flow__node-default selectable focus:ring-2 relative rounded-md group ${props.selected && "border-primary! border-2"} ${isExec ? "" : "bg-emerald-900"} ${executionState === "done" ? "opacity-60" : "opacity-100"}`}
		>
			{props.data.node.long_running && (
				<div className="absolute top-0 z-10 translate-y-[calc(-50%)] translate-x-[calc(-50%)] left-0 text-center bg-background rounded-full">
					<ClockIcon className="w-2 h-2 text-foreground" />
				</div>
			)}
			{props.data.node.comment && (
				<div className="absolute top-0 translate-y-[calc(-100%-0.5rem)] left-3 right-3 mb-2 text-center bg-foreground/70 text-background p-1 rounded-md">
					<small className="font-normal text-extra-small leading-extra-small">
						{props.data.node.comment}
					</small>
				</div>
			)}
			{props.data.node.error && (
				<div className="absolute bottom-0 translate-y-[calc(100%+1rem)] left-3 right-3 mb-2 text-destructive-foreground bg-destructive p-1 rounded-md">
					<small className="font-normal text-extra-small leading-extra-small">
						{props.data.node.error}
					</small>
				</div>
			)}
			{inputPins
				.filter((pin) => isPinAction(pin) || pin.name !== "var_ref")
				.map((pin, index) =>
					isPinAction(pin) ? (
						<FlowPinAction
							key={`${pin.pin.id}__action`}
							action={pin}
							index={index}
							input
						/>
					) : (
						<FlowPreviewPin
							key={pin.id}
							node={props.data.node}
							boardId={props.data.boardId}
							index={index}
							pin={pin}
							onPinRemove={async () => {}}
						/>
					),
				)}
			<div
				className={`absolute top-0 left-0 right-0 h-4 gap-1 mt-0! flex flex-row items-center border-b border-b-foreground p-1 justify-between rounded-md rounded-b-none bg-card ${!isExec && "bg-linear-to-r  from-card via-emerald-300/50 to-emerald-300 dark:via-tertiary/50 dark:to-tertiary"} ${props.data.node.start && "bg-linear-to-r  from-card via-rose-300/50 to-rose-300 dark:via-primary/50 dark:to-primary"}`}
			>
				<div className={"flex flex-row items-center mt-0! gap-1"}>
					{props.data.node?.icon && (
						<DynamicImage
							className="w-2 h-2 bg-foreground"
							url={props.data.node?.icon}
						/>
					)}
					{!props.data.node?.icon && <WorkflowIcon className="w-2 h-2" />}
					<small className="font-medium leading-none mt-0!">
						{props.data.node?.friendly_name}
					</small>
				</div>
				<div className="flex flex-row items-center gap-1">
					{props.data.node.start && !executing && (
						<PlayCircleIcon
							className="w-2 h-2 cursor-pointer hover:text-primary"
							onClick={async (e) => {
								toastSuccess(
									"Node executed",
									<PlayCircleIcon className="w-4 h-4" />,
								);
							}}
						/>
					)}
					{debouncedExecutionState === "running" && (
						<PuffLoader
							color={resolvedTheme === "dark" ? "white" : "black"}
							size={10}
							speedMultiplier={1}
						/>
					)}
					{debouncedExecutionState === "done" && (
						<SquareCheckIcon className="w-2 h-2 text-primary" />
					)}
				</div>
			</div>
			{outputPins.map((pin, index) =>
				isPinAction(pin) ? (
					<FlowPinAction
						action={pin}
						index={index}
						input={false}
						key={`${pin.pin.id}__action`}
					/>
				) : (
					<FlowPreviewPin
						node={props.data.node}
						boardId={props.data.boardId}
						index={index}
						pin={pin}
						key={pin.id}
						onPinRemove={async () => {}}
					/>
				),
			)}
		</div>
	);
}
