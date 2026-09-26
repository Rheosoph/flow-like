import { useTranslation } from "@flow-like/locales";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
	type MouseEvent,
	type ReactNode,
	type Ref,
	memo,
	useCallback,
	useEffect,
	useImperativeHandle,
	useRef,
} from "react";
import type { ILog } from "../../../lib/schema/flow/log";
import type { ILogGroup } from "../../../lib/schema/flow/log-query";
import type { INode } from "../../../lib/schema/flow/node";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuTrigger,
} from "../../ui/context-menu";
import { logKey } from "./log-format";
import {
	type IRowActions,
	LogRow,
	LogRowSkeleton,
	ROW_HEIGHT,
} from "./log-row";
import type { ILogWindow } from "./use-log-window";

const OVERSCAN = 20;

export interface ILogListHandle {
	scrollToIndex(index: number): void;
}

export interface IRowView {
	selectedKeys: ReadonlySet<string>;
	focusedKey?: string;
	relative: boolean;
	base: number;
	nodeOf(nodeId: string): INode | undefined;
	nodeName(nodeId: string): string | undefined;
	/** The repeat group a row heads, and whether it is folded right now. */
	headOf(log: ILog): { group: ILogGroup; folded: boolean } | undefined;
}

export const LogList = memo(function LogList({
	handleRef,
	window: list,
	view,
	actions,
	onContextRow,
	menu,
	empty,
}: Readonly<{
	handleRef: Ref<ILogListHandle>;
	window: ILogWindow;
	view: IRowView;
	actions: IRowActions;
	onContextRow: (index: number, log: ILog, selectedText: string) => void;
	menu: ReactNode;
	empty: ReactNode;
}>) {
	const { t } = useTranslation("flow");
	const scrollRef = useRef<HTMLDivElement>(null);
	const virtualizer = useVirtualizer({
		count: list.count,
		getScrollElement: () => scrollRef.current,
		estimateSize: () => ROW_HEIGHT,
		overscan: OVERSCAN,
	});

	useImperativeHandle(
		handleRef,
		() => ({
			scrollToIndex: (index: number) =>
				virtualizer.scrollToIndex(index, { align: "center" }),
		}),
		[virtualizer],
	);

	const items = virtualizer.getVirtualItems();
	const first = items[0]?.index ?? 0;
	const last = items.at(-1)?.index ?? -1;
	const { ensure, cacheKey } = list;

	useEffect(() => {
		if (cacheKey) ensure(first, last);
	}, [ensure, cacheKey, first, last]);

	const onContextMenu = useCallback(
		(event: MouseEvent<HTMLDivElement>) => {
			const row = (event.target as HTMLElement).closest("[data-log-index]");
			const index = Number(row?.getAttribute("data-log-index"));
			const log = Number.isInteger(index) ? list.row(index) : undefined;
			if (!row || !log) {
				event.preventDefault();
				return;
			}
			onContextRow(index, log, globalThis.getSelection?.()?.toString() ?? "");
		},
		[list, onContextRow],
	);

	if (list.count === 0 && !list.loading) {
		return <div className="min-h-0 flex-1">{empty}</div>;
	}

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div
					ref={scrollRef}
					role="log"
					aria-label={t("logViewRunLogs", "Run logs")}
					onContextMenu={onContextMenu}
					className="relative h-full min-h-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-contain py-0.5 [scrollbar-width:thin]"
				>
					<div
						className="relative w-full"
						style={{ height: virtualizer.getTotalSize() }}
					>
						{items.map((item) => {
							const log = list.row(item.index);
							if (!log) {
								return (
									<LogRowSkeleton
										key={item.key}
										top={item.start}
										failed={list.failed(item.index)}
									/>
								);
							}
							const key = logKey(log);
							const head = view.headOf(log);
							const nodeId = log.node_id ?? undefined;
							return (
								<LogRow
									key={item.key}
									index={item.index}
									top={item.start}
									log={log}
									selected={view.selectedKeys.has(key)}
									focused={view.focusedKey === key}
									node={nodeId ? view.nodeOf(nodeId) : undefined}
									nodeName={nodeId ? view.nodeName(nodeId) : undefined}
									group={head?.group}
									folded={head?.folded ?? false}
									relative={view.relative}
									base={view.base}
									actions={actions}
								/>
							);
						})}
					</div>
				</div>
			</ContextMenuTrigger>
			<ContextMenuContent className="w-60">{menu}</ContextMenuContent>
		</ContextMenu>
	);
});
