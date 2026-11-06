"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUpRight, Search, Table2, View } from "lucide-react";
import { useId, useState } from "react";
import { cn } from "../../../../lib/utils";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import { ScrollArea } from "../../../ui/scroll-area";
import type { SqlCatalogTable } from "../../../ui/sql-editor/sql-language";

export function QueryTablePicker({
	tables,
	views = [],
	onSelectTable,
	open: controlledOpen,
	onOpenChange,
	className,
}: Readonly<{
	tables: SqlCatalogTable[];
	views?: SqlCatalogTable[];
	onSelectTable: (name: string) => void;
	open?: boolean;
	onOpenChange?: (open: boolean) => void;
	className?: string;
}>) {
	const { t } = useTranslation("settings");
	const [localOpen, setLocalOpen] = useState(false);
	const open = controlledOpen ?? localOpen;
	const [search, setSearch] = useState("");
	const setOpen = (nextOpen: boolean) => {
		setLocalOpen(nextOpen);
		onOpenChange?.(nextOpen);
		if (!nextOpen) setSearch("");
	};
	const titleId = useId();
	const descriptionId = useId();
	const query = search.trim().toLocaleLowerCase();
	const matches = (table: SqlCatalogTable) =>
		table.name.toLocaleLowerCase().includes(query);
	const groups = [
		{
			label: t("queryCatalogTables", "Tables"),
			entries: tables.filter(matches),
			Icon: Table2,
		},
		{ label: t("views", "Views"), entries: views.filter(matches), Icon: View },
	];
	const count = tables.length + views.length;
	const matchCount = groups.reduce(
		(total, group) => total + group.entries.length,
		0,
	);

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<Button
					variant="ghost"
					size="sm"
					className={cn(
						"h-8 gap-1.5 px-2 text-xs text-muted-foreground hover:text-foreground",
						className,
					)}
					aria-label={t("browseTables", "Browse tables")}
				>
					<Table2 className="size-3.5" />
					{t("browseTables", "Browse tables")}
					{count > 0 && (
						<span className="rounded bg-muted px-1.5 py-0.5 text-[10px] tabular-nums text-muted-foreground">
							{count}
						</span>
					)}
				</Button>
			</PopoverTrigger>
			<PopoverContent
				align="end"
				collisionPadding={12}
				className="w-[360px] max-w-[calc(100vw-2rem)] overflow-hidden p-0"
				aria-labelledby={titleId}
				aria-describedby={descriptionId}
			>
				<div className="space-y-3 border-b p-3">
					<div>
						<h3 id={titleId} className="text-sm font-medium">
							{t("browseTables", "Browse tables")}
						</h3>
						<p
							id={descriptionId}
							className="mt-0.5 text-xs text-muted-foreground"
						>
							{t("chooseTableToStartQuery", "Choose a table to start a query.")}
						</p>
					</div>
					{count > 0 && (
						<div className="relative">
							<Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
							<Input
								value={search}
								onChange={(event) => setSearch(event.target.value)}
								placeholder={t(
									"searchTablesAndViews",
									"Search tables and views…",
								)}
								aria-label={t(
									"searchTablesAndViews",
									"Search tables and views",
								)}
								className="h-8 pl-8 text-xs md:text-xs"
							/>
						</div>
					)}
				</div>
				{matchCount > 0 ? (
					<ScrollArea className="max-h-80" viewportClassName="max-h-80">
						<div className="space-y-2 p-1.5">
							{groups.map(({ label, entries, Icon }) =>
								entries.length ? (
									<section key={label} aria-label={label}>
										<div className="flex items-center gap-1.5 px-2 py-1.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
											{label}
											<span className="font-normal tabular-nums">
												{entries.length}
											</span>
										</div>
										{entries.map((table) => (
											<button
												key={`${table.scope ?? "project"}:${table.name}`}
												type="button"
												aria-label={t("queryTableName", "Query {{name}}", {
													name: table.name,
												})}
												onClick={() => {
													onSelectTable(table.name);
													setOpen(false);
												}}
												className="group flex w-full items-center gap-2.5 rounded-md px-2 py-2 text-left hover:bg-accent focus-visible:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
											>
												<Icon className="size-4 shrink-0 text-muted-foreground" />
												<span className="min-w-0 flex-1">
													<span
														className="block truncate font-mono text-xs"
														title={table.name}
													>
														{table.name}
													</span>
													{table.columns && (
														<span className="mt-0.5 block text-[11px] text-muted-foreground">
															{t("countColumns", {
																count: table.columns.length,
																defaultValue_one: "{{count}} column",
																defaultValue_other: "{{count}} columns",
															})}
														</span>
													)}
												</span>
												<span className="flex shrink-0 items-center gap-1 text-[11px] text-muted-foreground group-hover:text-foreground group-focus-visible:text-foreground">
													{t("query", "Query")}
													<ArrowUpRight className="size-3" />
												</span>
											</button>
										))}
									</section>
								) : null,
							)}
						</div>
					</ScrollArea>
				) : (
					<div className="px-5 py-6 text-center">
						<Table2 className="mx-auto mb-2 size-5 text-muted-foreground" />
						<p className="text-sm font-medium">
							{count === 0
								? t("noTablesAvailable", "No tables available")
								: t("noMatchingTables", "No matching tables or views")}
						</p>
						<p className="mt-1 text-xs text-muted-foreground">
							{count === 0
								? t(
										"chooseSourceOrAddTable",
										"Choose another source or add a table in Sources.",
									)
								: t(
										"tryDifferentTableName",
										"Try a different name or clear the search.",
									)}
						</p>
						{count > 0 && (
							<Button
								variant="ghost"
								size="sm"
								className="mt-2 h-7 text-xs"
								onClick={() => setSearch("")}
							>
								{t("clearSearch", "Clear search")}
							</Button>
						)}
					</div>
				)}
			</PopoverContent>
		</Popover>
	);
}
