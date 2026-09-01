"use client";

import { useTranslation } from "@flow-like/locales";
import { Network, Play, Table2 } from "lucide-react";
import { useCallback, useState } from "react";
import { Button } from "../button";
import { ScrollArea } from "../scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../tabs";

export interface GraphQueryPanelProps {
	onRunCypher: (query: string) => void;
	results: unknown[] | null;
	loading?: boolean;
	error?: string | null;
	/**
	 * Puts the current results onto the canvas. Offered only when the rows
	 * could be resolved back into nodes and edges of this ontology.
	 */
	onAddToCanvas?: () => void;
	addToCanvasCount?: number;
}

export function GraphQueryPanel({
	onRunCypher,
	results,
	loading,
	error,
	onAddToCanvas,
	addToCanvasCount,
}: GraphQueryPanelProps) {
	const { t } = useTranslation("common");
	const [query, setQuery] = useState("");
	const [activeTab, setActiveTab] = useState("table");

	const handleRun = useCallback(() => {
		if (query.trim()) {
			onRunCypher(query.trim());
		}
	}, [query, onRunCypher]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent<HTMLTextAreaElement>) => {
			if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
				e.preventDefault();
				handleRun();
			}
		},
		[handleRun],
	);

	return (
		<div className="flex flex-col border rounded-lg bg-background overflow-hidden">
			<div className="p-3 border-b space-y-2">
				<div className="flex items-center justify-between gap-2">
					<p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
						{t("cypherQuery", "Cypher Query")}
					</p>
					<div className="flex items-center gap-1.5">
						{onAddToCanvas && (addToCanvasCount ?? 0) > 0 && (
							<Button
								size="sm"
								variant="outline"
								onClick={onAddToCanvas}
								title={t(
									"drawTheseResultsOnTheGraphCanvas",
									"Draw these results on the graph canvas",
								)}
							>
								<Network className="h-3.5 w-3.5 mr-1" />
								{t("addCountToCanvas", "Add {{count}} to canvas", {
									count: addToCanvasCount,
								})}
							</Button>
						)}
						<Button
							size="sm"
							onClick={handleRun}
							disabled={loading || !query.trim()}
						>
							<Play className="h-3.5 w-3.5 mr-1" />
							{loading ? "Running..." : "Run"}
						</Button>
					</div>
				</div>
				<textarea
					value={query}
					onChange={(e) => setQuery(e.target.value)}
					onKeyDown={handleKeyDown}
					placeholder="MATCH (n:Person)-[r]->(m) RETURN n, r, m LIMIT 100"
					className="w-full min-h-[80px] max-h-[200px] rounded-md border bg-muted/50 px-3 py-2 text-sm font-mono resize-y focus:outline-none focus:ring-2 focus:ring-ring"
					spellCheck={false}
				/>
			</div>
			{error && (
				<div className="px-3 py-2 bg-destructive/10 text-destructive text-xs border-b">
					{error}
				</div>
			)}
			{results && results.length > 0 && (
				<div className="flex-1 min-h-0">
					<Tabs
						value={activeTab}
						onValueChange={setActiveTab}
						className="h-full flex flex-col"
					>
						<TabsList className="mx-3 mt-2 w-fit">
							<TabsTrigger value="table" className="text-xs gap-1">
								<Table2 className="h-3.5 w-3.5" />
								{t("table", "Table")}
							</TabsTrigger>
							<TabsTrigger value="json" className="text-xs gap-1">
								<Network className="h-3.5 w-3.5" />
								{`JSON`}
							</TabsTrigger>
						</TabsList>
						<TabsContent value="table" className="flex-1 m-0 p-3 min-h-0">
							<ScrollArea className="max-h-[300px]">
								<div className="border rounded overflow-auto">
									<table className="w-full text-xs">
										<thead>
											<tr className="bg-muted/50">
												{results[0] &&
												typeof results[0] === "object" &&
												results[0] !== null ? (
													Object.keys(results[0]).map((key) => (
														<th
															key={key}
															className="px-3 py-2 text-left font-medium text-muted-foreground"
														>
															{key}
														</th>
													))
												) : (
													<th className="px-3 py-2 text-left font-medium text-muted-foreground">
														{t("value2", "Value")}
													</th>
												)}
											</tr>
										</thead>
										<tbody>
											{results.map((row, i) => (
												<tr key={i} className="border-t">
													{typeof row === "object" && row !== null ? (
														Object.values(row).map((val, j) => (
															<td
																key={j}
																className="px-3 py-1.5 max-w-[200px] truncate"
															>
																{typeof val === "object"
																	? JSON.stringify(val)
																	: String(val ?? "")}
															</td>
														))
													) : (
														<td className="px-3 py-1.5">{String(row)}</td>
													)}
												</tr>
											))}
										</tbody>
									</table>
								</div>
							</ScrollArea>
						</TabsContent>
						<TabsContent value="json" className="flex-1 m-0 p-3 min-h-0">
							<ScrollArea className="max-h-[300px]">
								<pre className="text-xs font-mono bg-muted/50 rounded p-3 whitespace-pre-wrap">
									{JSON.stringify(results, null, 2)}
								</pre>
							</ScrollArea>
						</TabsContent>
					</Tabs>
				</div>
			)}
			{results && results.length === 0 && (
				<div className="p-4 text-center text-sm text-muted-foreground">
					{t("queryReturnedNoResults", "Query returned no results")}
				</div>
			)}
		</div>
	);
}
