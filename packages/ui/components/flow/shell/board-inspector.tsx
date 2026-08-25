"use client";

import { useTranslation } from "@flow-like/locales";
import { LockIcon, MousePointerSquareDashedIcon } from "lucide-react";
import { memo, useMemo } from "react";
import type { IBoard } from "../../../lib/schema/flow/board";
import type { INode } from "../../../lib/schema/flow/node";
import {
	type IPin,
	IPinType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import { cn } from "../../../lib/utils";
import { typeToColor } from "../utils";

const SCORE_KEYS = [
	"privacy",
	"security",
	"performance",
	"governance",
	"reliability",
	"cost",
] as const;

function scoreTone(value: number): string {
	if (value >= 8) return "text-emerald-500";
	if (value >= 5) return "text-amber-500";
	return "text-destructive";
}

const Row = memo(function Row({
	label,
	value,
	mono,
}: Readonly<{ label: string; value: React.ReactNode; mono?: boolean }>) {
	return (
		<div className="flex items-baseline gap-2 px-2 py-0.5 text-xs">
			<span className="shrink-0 text-muted-foreground">{label}</span>
			<span
				className={cn(
					"ml-auto truncate text-right text-foreground",
					mono && "font-mono text-[11px]",
				)}
			>
				{value}
			</span>
		</div>
	);
});

const Group = memo(function Group({
	title,
	action,
	children,
}: Readonly<{
	title: string;
	action?: React.ReactNode;
	children: React.ReactNode;
}>) {
	return (
		<section className="border-b py-1 last:border-b-0">
			<header className="flex items-center gap-2 px-2 pb-0.5 pt-1">
				<h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
					{title}
				</h3>
				<span className="flex-1" />
				{action}
			</header>
			{children}
		</section>
	);
});

const PinRow = memo(function PinRow({ pin }: Readonly<{ pin: IPin }>) {
	const { t } = useTranslation("flow");
	const options = pin.options ?? undefined;
	const traits: string[] = [];
	if (options?.enforce_generic_value_type)
		traits.push(t("enforceGenericVT", "Enforce Generic VT"));
	if (options?.enforce_schema)
		traits.push(t("enforceSchema", "Enforce Schema"));
	if (options?.valid_values?.length)
		traits.push(`${options.valid_values.length} ${t("options", "Options…")}`);
	if (options?.range) traits.push(`${options.range[0]} – ${options.range[1]}`);
	if (typeof options?.step === "number")
		traits.push(`${t("step", "Step")} ${options.step}`);
	if (pin.schema) traits.push(t("schema", "Schema"));

	return (
		<div className="px-2 py-1">
			<div className="flex items-center gap-2 text-xs">
				<span
					className="size-2 shrink-0 rounded-full"
					style={{ backgroundColor: typeToColor(pin.data_type) }}
				/>
				<span className="truncate font-medium">{pin.friendly_name}</span>
				{options?.sensitive && (
					<LockIcon className="size-3 shrink-0 text-amber-500" />
				)}
				<span className="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground">
					{pin.value_type === "Normal"
						? pin.data_type
						: `${pin.value_type}<${pin.data_type}>`}
				</span>
			</div>
			{traits.length > 0 && (
				<p className="truncate pl-4 text-[10px] text-muted-foreground">
					{traits.join(" · ")}
				</p>
			)}
			{pin.connected_to.length === 0 &&
				pin.data_type === IVariableType.Execution && (
					<p className="pl-4 text-[10px] text-amber-500">
						{t("notConnected", "Not connected")}
					</p>
				)}
		</div>
	);
});

/**
 * Everything a node carries beyond its position — pins with their value types
 * and constraints, docs, quality scores and the permissions a WASM node
 * declares. All of it used to be crammed onto the node body on the canvas or
 * buried in the layer-editing modal, which is why the shell now reserves a rail
 * for it.
 */
export const BoardInspector = memo(function BoardInspector({
	board,
	selectedNodeIds,
	onRevealNode,
}: Readonly<{
	board?: IBoard;
	selectedNodeIds: string[];
	onRevealNode?: (nodeId: string) => void;
}>) {
	const { t } = useTranslation("flow");

	const node: INode | undefined = useMemo(() => {
		if (!board || selectedNodeIds.length !== 1) return undefined;
		return board.nodes?.[selectedNodeIds[0]];
	}, [board, selectedNodeIds]);

	const { inputs, outputs } = useMemo(() => {
		const pins = Object.values(node?.pins ?? {}).sort(
			(a, b) => a.index - b.index,
		);
		return {
			inputs: pins.filter((pin) => pin.pin_type === IPinType.Input),
			outputs: pins.filter((pin) => pin.pin_type === IPinType.Output),
		};
	}, [node]);

	if (selectedNodeIds.length > 1) {
		return (
			<div className="flex h-full flex-col items-center justify-center gap-1 p-4 text-center">
				<p className="text-sm font-medium">
					{t("nodesSelected", "{{count}} nodes selected", {
						count: selectedNodeIds.length,
					})}
				</p>
				<p className="text-xs text-muted-foreground">
					{t("selectOneNodeToInspect", "Select one node to inspect it.")}
				</p>
			</div>
		);
	}

	if (!node) {
		return (
			<div className="flex h-full flex-col items-center justify-center gap-2 p-4 text-center">
				<MousePointerSquareDashedIcon className="size-5 text-muted-foreground" />
				<p className="text-xs text-muted-foreground">
					{t("selectANodeToInspectIt", "Select a node to inspect it.")}
				</p>
			</div>
		);
	}

	const permissions = node.wasm?.permissions ?? [];

	return (
		<div className="flex flex-col">
			<Group title={t("nodeInfo", "Node Info")}>
				<Row label={t("nodeName", "Node Name")} value={node.friendly_name} />
				<Row label={t("category", "Category")} value={node.category} mono />
				{typeof node.version === "number" && (
					<Row label={t("version", "Version")} value={node.version} mono />
				)}
				{node.description && (
					<p className="px-2 pb-1 pt-0.5 text-[11px] leading-snug text-muted-foreground">
						{node.description}
					</p>
				)}
				{node.error && (
					<p className="px-2 pb-1 text-[11px] leading-snug text-destructive">
						{node.error}
					</p>
				)}
			</Group>

			<Group
				title={`${t("inputs", "Inputs")} · ${inputs.length}`}
				action={
					onRevealNode && (
						<button
							type="button"
							onClick={() => onRevealNode(node.id)}
							className="text-[10px] text-muted-foreground hover:text-foreground"
						>
							{t("navigateToNode", "Navigate to node")}
						</button>
					)
				}
			>
				{inputs.length === 0 ? (
					<p className="px-2 text-[11px] text-muted-foreground">
						{t("noPinsInThisGroup", "No pins in this group.")}
					</p>
				) : (
					inputs.map((pin) => <PinRow key={pin.id} pin={pin} />)
				)}
			</Group>

			<Group title={`${t("outputs", "Outputs")} · ${outputs.length}`}>
				{outputs.length === 0 ? (
					<p className="px-2 text-[11px] text-muted-foreground">
						{t("noPinsInThisGroup", "No pins in this group.")}
					</p>
				) : (
					outputs.map((pin) => <PinRow key={pin.id} pin={pin} />)
				)}
			</Group>

			{node.scores && (
				<Group title={t("score", "Score")}>
					{SCORE_KEYS.map((key) => {
						const value = node.scores?.[key];
						if (typeof value !== "number") return null;
						return (
							<Row
								key={key}
								label={key}
								value={
									<span className={cn("font-mono", scoreTone(value))}>
										{value}
									</span>
								}
							/>
						);
					})}
				</Group>
			)}

			{permissions.length > 0 && (
				<Group title={t("permissions", "Permissions")}>
					{permissions.map((permission) => (
						<Row key={String(permission)} label={String(permission)} value="" />
					))}
				</Group>
			)}
		</div>
	);
});
