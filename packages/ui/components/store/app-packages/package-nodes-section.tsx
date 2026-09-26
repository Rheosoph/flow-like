"use client";

import { useTranslation } from "@flow-like/locales";
import { Search, Zap } from "lucide-react";
import { useMemo, useState } from "react";
import {
	type NodeCategoryGroup,
	filterNodeGroups,
	nodeDisplayName,
} from "../../../lib/app-package-overview";
import type { INode } from "../../../lib/schema/flow/node";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Skeleton } from "../../ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { EmptyHint, Section } from "./parts";

const COMPACT_NODE_LIMIT = 5;
const SKELETON_KEYS = ["a", "b", "c", "d"] as const;

interface NodeSectionProps {
	groups: readonly NodeCategoryGroup[];
	packageNames: ReadonlyMap<string, string>;
	/** Packages that are stale or expired; their categories render muted. */
	mutedPackageIds: ReadonlySet<string>;
	loading: boolean;
}

function useNodeTotals(groups: readonly NodeCategoryGroup[]) {
	return useMemo(
		() => groups.reduce((sum, group) => sum + group.nodes.length, 0),
		[groups],
	);
}

function packagesLabel(
	group: NodeCategoryGroup,
	packageNames: ReadonlyMap<string, string>,
): string {
	return group.packageIds.map((id) => packageNames.get(id) ?? id).join(", ");
}

function nodeKey(node: INode): string {
	return `${node.wasm?.package_id ?? ""}/${node.name}`;
}

function isMuted(group: NodeCategoryGroup, muted: ReadonlySet<string>) {
	return group.packageIds.every((id) => muted.has(id));
}

function NodesMeta({
	groups,
}: Readonly<{ groups: readonly NodeCategoryGroup[] }>) {
	const { t } = useTranslation("store");
	const total = useNodeTotals(groups);
	return (
		<>
			{t("appPackagesNodesMeta", {
				defaultValue_one: "{{count}} node across {{categories}}",
				defaultValue_other: "{{count}} nodes across {{categories}}",
				count: total,
				categories: t("appPackagesCategoryCount", {
					defaultValue_one: "{{count}} category",
					defaultValue_other: "{{count}} categories",
					count: groups.length,
				}),
			})}
		</>
	);
}

/** Overview: one card per category with the first few nodes. */
export function PackageNodeCategories({
	groups,
	packageNames,
	mutedPackageIds,
	loading,
	onShowAll,
}: Readonly<NodeSectionProps & { onShowAll: () => void }>) {
	const { t } = useTranslation("store");

	return (
		<Section
			title={t("appPackagesNodes", "Nodes")}
			meta={groups.length > 0 ? <NodesMeta groups={groups} /> : undefined}
		>
			{loading && groups.length === 0 ? (
				<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
					{SKELETON_KEYS.map((key) => (
						<Skeleton key={key} className="h-44 rounded-xl" />
					))}
				</div>
			) : groups.length === 0 ? (
				<EmptyHint>
					{t("appPackagesNoNodes", "These packages add no nodes.")}
				</EmptyHint>
			) : (
				<div className="grid items-start gap-4 sm:grid-cols-2 xl:grid-cols-4">
					{groups.map((group) => {
						const hidden = group.nodes.length - COMPACT_NODE_LIMIT;
						const muted = isMuted(group, mutedPackageIds);
						return (
							<article
								key={group.category}
								className={cn(
									"flex flex-col gap-3 rounded-xl border bg-card p-4 shadow-xs",
									muted && "border-dashed bg-card/60",
								)}
							>
								<div className="min-w-0">
									<div className="flex items-baseline justify-between gap-2">
										<h3
											className={cn(
												"truncate text-sm font-semibold",
												muted && "text-muted-foreground",
											)}
										>
											{group.label}
										</h3>
										<span className="text-xs tabular-nums text-muted-foreground">
											{group.nodes.length}
										</span>
									</div>
									<p className="truncate text-xs text-muted-foreground">
										{packagesLabel(group, packageNames)}
									</p>
								</div>
								<ul className="flex flex-col gap-2 text-sm">
									{group.nodes.slice(0, COMPACT_NODE_LIMIT).map((node) => (
										<li key={nodeKey(node)} className="min-w-0">
											<NodeName
												name={nodeDisplayName(node)}
												description={node.description}
												muted={muted}
											/>
										</li>
									))}
								</ul>
								{hidden > 0 && (
									<Button
										variant="link"
										size="sm"
										className="h-auto self-start p-0 text-xs"
										onClick={onShowAll}
									>
										{t("appPackagesMoreNodes", "Show {{hidden}} more", {
											hidden,
										})}
									</Button>
								)}
							</article>
						);
					})}
				</div>
			)}
		</Section>
	);
}

function NodeName({
	name,
	description,
	muted,
}: Readonly<{ name: string; description?: string; muted: boolean }>) {
	const label = (
		<span
			className={cn(
				"flex min-w-0 items-center gap-2",
				muted && "text-muted-foreground",
			)}
		>
			<Zap
				className="size-3.5 shrink-0 text-muted-foreground"
				aria-hidden="true"
			/>
			<span className="truncate">{name}</span>
		</span>
	);
	if (!description) return label;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					className="block w-full min-w-0 cursor-default rounded-sm text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					{label}
				</button>
			</TooltipTrigger>
			<TooltipContent side="bottom" className="max-w-xs">
				{description}
			</TooltipContent>
		</Tooltip>
	);
}

/** Nodes tab: every node with its description, filterable. */
export function PackageNodeList({
	groups,
	packageNames,
	mutedPackageIds,
	loading,
}: Readonly<NodeSectionProps>) {
	const { t } = useTranslation("store");
	const [query, setQuery] = useState("");
	const filtered = useMemo(
		() => filterNodeGroups(groups, query, packageNames),
		[groups, query, packageNames],
	);

	return (
		<Section
			title={t("appPackagesNodes", "Nodes")}
			meta={groups.length > 0 ? <NodesMeta groups={groups} /> : undefined}
			action={
				<div className="relative w-full sm:w-72">
					<Search
						className="absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-muted-foreground"
						aria-hidden="true"
					/>
					<Input
						type="search"
						value={query}
						onChange={(event) => setQuery(event.target.value)}
						placeholder={t(
							"appPackagesSearchNodes",
							"Search nodes, categories or packages",
						)}
						aria-label={t("appPackagesSearchNodesLabel", "Search nodes")}
						className="h-8 pl-8 text-sm"
					/>
				</div>
			}
		>
			{loading && groups.length === 0 ? (
				<div className="flex flex-col gap-3">
					{SKELETON_KEYS.map((key) => (
						<Skeleton key={key} className="h-24 rounded-xl" />
					))}
				</div>
			) : filtered.length === 0 ? (
				<EmptyHint>
					{groups.length === 0
						? t("appPackagesNoNodes", "These packages add no nodes.")
						: t("appPackagesNoNodeMatches", "No nodes match “{{query}}”.", {
								query: query.trim(),
							})}
				</EmptyHint>
			) : (
				<div className="flex flex-col gap-4">
					{filtered.map((group) => {
						const muted = isMuted(group, mutedPackageIds);
						return (
							<section
								key={group.category}
								aria-label={group.label}
								className={cn(
									"overflow-hidden rounded-xl border bg-card shadow-xs",
									muted && "border-dashed bg-card/60",
								)}
							>
								<header className="flex flex-wrap items-baseline justify-between gap-2 border-b bg-muted/30 px-4 py-2.5">
									<h3 className="text-sm font-semibold">{group.label}</h3>
									<span className="text-xs text-muted-foreground">
										{`${packagesLabel(group, packageNames)} · ${group.nodes.length}`}
									</span>
								</header>
								<ul className="divide-y">
									{group.nodes.map((node) => (
										<li
											key={nodeKey(node)}
											className="flex items-start gap-3 px-4 py-3"
										>
											<span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
												<Zap className="size-3.5" aria-hidden="true" />
											</span>
											<div className="min-w-0 flex-1">
												<p
													className={cn(
														"text-sm font-medium",
														muted && "text-muted-foreground",
													)}
												>
													{nodeDisplayName(node)}
												</p>
												{node.description && (
													<p className="text-sm text-muted-foreground">
														{node.description}
													</p>
												)}
											</div>
										</li>
									))}
								</ul>
							</section>
						);
					})}
				</div>
			)}
		</Section>
	);
}
