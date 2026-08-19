"use client";
import { useTranslation } from "@flow-like/locales";
import { ChevronRightIcon, WorkflowIcon } from "lucide-react";
import type { RefObject } from "react";
import { memo, useMemo } from "react";
import {
	ContextMenuItem,
	ContextMenuSub,
	ContextMenuSubContent,
	ContextMenuSubTrigger,
} from "../../components/ui/context-menu";
import type { INode } from "../../lib/schema/flow/node";
import type { IPin } from "../../lib/schema/flow/pin";
import { DynamicImage } from "../ui/dynamic-image";

const MAX_SEARCH_RESULTS = 50;

export const FlowContextMenuNodes = memo(function FlowContextMenuNodes({
	items,
	filter,
	pin,
	onNodePlace,
	menuBlockedRef,
}: Readonly<{
	items: INode[];
	filter: string;
	pin?: IPin;
	onNodePlace: (node: INode) => Promise<void>;
	menuBlockedRef?: RefObject<boolean>;
}>) {
	const { t } = useTranslation("flow");
	const { leafs, sortedCategories } = useMemo(() => {
		const leafs: INode[] = [];
		const nodes = new Map<string, INode[]>();

		for (const item of items) {
			const itemCopy = { ...item };
			const category = itemCopy.category.trim().split("/");

			if (category.length === 0 || category[0] === "") {
				leafs.push(itemCopy);
				continue;
			}

			const root = category.shift() as string;
			itemCopy.category = category.join("/");

			if (!nodes.has(root)) {
				nodes.set(root, []);
			}
			nodes.get(root)?.push(itemCopy);
		}

		const sortedCategories = Array.from(nodes).sort(([a], [b]) =>
			a.localeCompare(b),
		);

		return { leafs, sortedCategories };
	}, [items]);

	if (filter !== "") {
		const displayItems =
			items.length > MAX_SEARCH_RESULTS
				? items.slice(0, MAX_SEARCH_RESULTS)
				: items;
		return (
			<>
				{displayItems.map((node) => (
					<ContextMenuItem
						key={node.id}
						id={node.id}
						onSelect={(event) => {
							if (menuBlockedRef?.current) {
								event.preventDefault();
								return;
							}
							onNodePlace(node);
						}}
					>
						{node.icon ? (
							<DynamicImage
								url={node.icon}
								className="h-4 w-4 mr-2 bg-foreground"
							/>
						) : (
							<WorkflowIcon className="h-4 w-4 mr-2" />
						)}
						{node.friendly_name}
					</ContextMenuItem>
				))}
				{items.length > MAX_SEARCH_RESULTS && (
					<div className="px-2 py-1.5 text-xs text-muted-foreground text-center">{t('showingMax_search_resultsOfLengthRefineYourSearch', 'Showing {{MAX_SEARCH_RESULTS}} of {{length}} — refine your search', { MAX_SEARCH_RESULTS, length: items.length })}</div>
				)}
			</>
		);
	}

	return (
		<>
			{sortedCategories.map(([category, node]) => (
				<ContextMenuSub key={category + node.length}>
					<ContextMenuSubTrigger>
						<ChevronRightIcon className="h-4 w-4 mr-1" />
						{category}
					</ContextMenuSubTrigger>
					<ContextMenuSubContent className="w-48" key={category}>
						<div className="max-h-96 overflow-y-auto">
							<FlowContextMenuNodes
								items={node}
								filter={filter}
								pin={pin}
								onNodePlace={onNodePlace}
								menuBlockedRef={menuBlockedRef}
							/>
						</div>
					</ContextMenuSubContent>
				</ContextMenuSub>
			))}
			{leafs.map((node) => (
				<ContextMenuItem
					key={`context${node.id}`}
					id={node.id}
					onSelect={async (event) => {
						if (menuBlockedRef?.current) {
							event.preventDefault();
							return;
						}
						await onNodePlace(node);
					}}
				>
					{node.icon ? (
						<DynamicImage
							url={node.icon}
							className="min-h-4 min-w-4 mr-2 bg-foreground"
						/>
					) : (
						<WorkflowIcon className="h-4 w-4 mr-2" />
					)}
					{node.friendly_name}
				</ContextMenuItem>
			))}
		</>
	);
});
