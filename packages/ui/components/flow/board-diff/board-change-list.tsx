"use client";

import { useTranslation } from "@flow-like/locales";
import {
	FolderTreeIcon,
	LayersIcon,
	SettingsIcon,
	SquareFunctionIcon,
	VariableIcon,
	WorkflowIcon,
} from "lucide-react";
import { type ReactNode, memo, useMemo } from "react";
import type { IBoardChange, IBoardChangeDetail } from "../../../lib/board-diff";
import { type IBoard, ILayerType } from "../../../lib/schema/flow/board";
import { cn } from "../../../lib/utils";
import { computeInlineSegments } from "../../ui/diff-viewer/compute";
import { StatusGlyph, useChangeSummary, useDetailLabel } from "./diff-status";

const LONG_VALUE = 32;

interface ChangeGroup {
	key: string;
	label: string;
	icon: ReactNode;
	changes: IBoardChange[];
}

function layerIcon(type?: ILayerType) {
	if (type === ILayerType.Function)
		return <SquareFunctionIcon className="size-3.5" />;
	if (type === ILayerType.Module)
		return <FolderTreeIcon className="size-3.5" />;
	return <LayersIcon className="size-3.5" />;
}

export function useChangeGroups(
	changes: IBoardChange[],
	head: IBoard,
	base: IBoard,
): ChangeGroup[] {
	const { t } = useTranslation("flow");
	return useMemo(() => {
		const groups = new Map<string, ChangeGroup>();
		const group = (
			key: string,
			make: () => Omit<ChangeGroup, "changes" | "key">,
		) => {
			let found = groups.get(key);
			if (!found) {
				found = { key, changes: [], ...make() };
				groups.set(key, found);
			}
			return found;
		};
		for (const change of changes) {
			if (change.kind === "variable" && !change.layerId) {
				group("variables", () => ({
					label: t("boardDiffGroupVariables", "Variables"),
					icon: <VariableIcon className="size-3.5" />,
				})).changes.push(change);
				continue;
			}
			if (change.kind === "board") {
				group("board", () => ({
					label: t("boardDiffGroupSettings", "Board settings"),
					icon: <SettingsIcon className="size-3.5" />,
				})).changes.push(change);
				continue;
			}
			if (!change.layerId) {
				group("root", () => ({
					label: head.name || base.name || t("boardDiffGroupBoard", "Board"),
					icon: <WorkflowIcon className="size-3.5" />,
				})).changes.push(change);
				continue;
			}
			const layer =
				head.layers?.[change.layerId] ?? base.layers?.[change.layerId];
			group(`layer:${change.layerId}`, () => ({
				label: layer?.name || change.layerId || "",
				icon: layerIcon(layer?.type),
			})).changes.push(change);
		}
		return [...groups.values()];
	}, [base, changes, head, t]);
}

export function InlineValueDiff({
	before,
	after,
}: Readonly<{ before: string; after: string }>) {
	const segments = useMemo(() => {
		let offset = 0;
		return computeInlineSegments(before, after).map((segment) => {
			const key = `${segment.kind}:${offset}`;
			offset += segment.text.length;
			return { ...segment, key };
		});
	}, [after, before]);
	return (
		<span className="whitespace-pre-wrap wrap-break-word">
			{segments.map((segment) =>
				segment.kind === "common" ? (
					<span key={segment.key}>{segment.text}</span>
				) : segment.kind === "added" ? (
					<ins key={segment.key} className="bg-emerald-500/25 no-underline">
						{segment.text}
					</ins>
				) : (
					<del
						key={segment.key}
						className="bg-destructive/20 decoration-destructive/60"
					>
						{segment.text}
					</del>
				),
			)}
		</span>
	);
}

function DetailValue({
	sign,
	value,
}: Readonly<{ sign: "+" | "−"; value: string }>) {
	return (
		<div
			className={cn(
				"flex gap-1.5 rounded-sm px-1.5 py-0.5 font-mono text-[11.5px] leading-relaxed",
				sign === "+" ? "bg-emerald-500/10" : "bg-destructive/10",
			)}
		>
			<span
				className={cn(
					"shrink-0 select-none opacity-70",
					sign === "+"
						? "text-emerald-600 dark:text-emerald-400"
						: "text-destructive",
				)}
			>
				{sign}
			</span>
			<span className="min-w-0 whitespace-pre-wrap wrap-break-word">
				{value}
			</span>
		</div>
	);
}

export function ChangeDetails({ change }: Readonly<{ change: IBoardChange }>) {
	const { t } = useTranslation("flow");
	const detailLabel = useDetailLabel();
	const rows = change.details.map((detail: IBoardChangeDetail) => {
		const long = [detail.before, detail.after].some(
			(v) => (v?.length ?? 0) > LONG_VALUE,
		);
		const both = detail.before !== undefined && detail.after !== undefined;
		return (
			<div key={detail.field} className="grid gap-1">
				<div className="text-[11px] font-medium text-muted-foreground">
					{detailLabel(detail)}
				</div>
				{long && both && !detail.masked ? (
					<div className="rounded-sm bg-muted px-1.5 py-1 font-mono text-[11.5px] leading-relaxed">
						<InlineValueDiff
							before={detail.before ?? ""}
							after={detail.after ?? ""}
						/>
					</div>
				) : (
					<>
						{detail.before !== undefined && (
							<DetailValue sign="−" value={detail.before} />
						)}
						{detail.after !== undefined && (
							<DetailValue sign="+" value={detail.after} />
						)}
					</>
				)}
			</div>
		);
	});
	if (change.move && change.status !== "moved") {
		rows.push(
			<div key="__move" className="text-[11.5px] text-muted-foreground">
				{t("boardDiffAlsoMovedLong", "Also moved on the canvas")}
			</div>,
		);
	}
	if (!rows.length) return null;
	return <div className="grid gap-2">{rows}</div>;
}

export const BoardChangeList = memo(function BoardChangeList({
	groups,
	selectedKey,
	onSelect,
	onHover,
	expandSelected = true,
	renderMeta,
	emptyText,
}: Readonly<{
	groups: ChangeGroup[];
	selectedKey?: string;
	onSelect: (change: IBoardChange) => void;
	onHover?: (change?: IBoardChange) => void;
	expandSelected?: boolean;
	renderMeta?: (change: IBoardChange) => ReactNode;
	emptyText: string;
}>) {
	const summary = useChangeSummary();
	if (!groups.length) {
		return (
			<p className="px-3 py-8 text-center text-sm text-muted-foreground">
				{emptyText}
			</p>
		);
	}
	return (
		<div className="grid gap-1" onPointerLeave={() => onHover?.(undefined)}>
			{groups.map((group) => (
				<section key={group.key} className="grid gap-0.5">
					<h4 className="flex items-center gap-1.5 px-2 pb-1 pt-3 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
						{group.icon}
						<span className="truncate normal-case tracking-normal">
							{group.label}
						</span>
					</h4>
					{group.changes.map((change) => {
						const active = change.key === selectedKey;
						return (
							<div
								key={change.key}
								data-change-key={change.key}
								onPointerEnter={() => onHover?.(change)}
								className={cn(
									"rounded-md border border-transparent transition-colors",
									active
										? "border-border bg-card shadow-sm"
										: "hover:bg-muted/60",
								)}
							>
								<button
									type="button"
									aria-current={active}
									onClick={() => onSelect(change)}
									onFocus={() => onHover?.(change)}
									className="grid w-full cursor-pointer grid-cols-[18px_minmax(0,1fr)_auto] gap-x-2.5 gap-y-0.5 rounded-md px-2 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
								>
									<StatusGlyph status={change.status} className="mt-px" />
									<span className="min-w-0 truncate text-[13px] font-semibold">
										{change.title}
									</span>
									<span>{renderMeta?.(change)}</span>
									<span className="col-start-2 col-end-4 text-xs text-muted-foreground wrap-anywhere">
										{summary(change)}
									</span>
								</button>
								{active && expandSelected && (
									<div className="pb-2.5 pl-9.5 pr-2">
										<ChangeDetails change={change} />
									</div>
								)}
							</div>
						);
					})}
				</section>
			))}
		</div>
	);
});

export type { ChangeGroup };
