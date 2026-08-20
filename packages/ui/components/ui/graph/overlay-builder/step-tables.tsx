"use client";

import { useTranslation } from "@flow-like/locales";
import { Database, User } from "lucide-react";
import { Badge } from "../../badge";
import { Checkbox } from "../../checkbox";
import { ScrollArea } from "../../scroll-area";

export interface TableInfo {
	name: string;
	userScoped?: boolean;
}

export interface StepTablesProps {
	tables: TableInfo[];
	selected: Set<string>;
	onToggle: (key: string) => void;
}

function tableKey(t: TableInfo): string {
	return t.userScoped ? `user:${t.name}` : t.name;
}

export function StepTables({ tables, selected, onToggle }: StepTablesProps) {
	const { t } = useTranslation("common");
	return (
		<div className="space-y-4">
			<div>
				<h3 className="text-sm font-medium mb-1">
					{t("selectTables", "Select Tables")}
				</h3>
				<p className="text-xs text-muted-foreground">
					{t(
						"chooseWhichDatabaseTablesToIncludeInThisGraphOverlaySelectedTablesCanBeMappedAsNodeOrEdgeSources",
						"Choose which database tables to include in this graph overlay. Selected tables can be mapped as node or edge sources.",
					)}
				</p>
			</div>
			<ScrollArea className="max-h-[400px]">
				<div className="space-y-2 pr-2">
					{tables.map((table) => {
						const key = tableKey(table);
						return (
							<label
								key={key}
								className="flex items-center gap-3 rounded-lg border p-3 cursor-pointer hover:bg-accent/50 transition-colors"
							>
								<Checkbox
									checked={selected.has(key)}
									onCheckedChange={() => onToggle(key)}
								/>
								<Database className="h-4 w-4 text-muted-foreground shrink-0" />
								<span className="text-sm truncate flex-1">{table.name}</span>
								{table.userScoped && (
									<Badge
										variant="outline"
										className="shrink-0 text-[10px] gap-1"
									>
										<User className="h-3 w-3" />
										{t("user", "User")}
									</Badge>
								)}
							</label>
						);
					})}
					{tables.length === 0 && (
						<p className="text-sm text-muted-foreground text-center py-4">
							{t("noTablesFound", "No tables found")}
						</p>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}
