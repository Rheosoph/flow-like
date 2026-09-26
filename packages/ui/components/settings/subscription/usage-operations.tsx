"use client";
import { UsageOperationDetails } from "./usage-operation-details";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../../hooks/use-invoke";
import { formatQuota } from "../../../lib/quota";
import { asArray } from "../../../lib/response-shape";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import {
	usageFundingLabel,
	usageStatusLabel,
	useUsageNames,
} from "./use-usage-names";
import { ChevronDown } from "lucide-react";

export function UsageOperations() {
	const backend = useBackend();
	const auth = useAuth();
	const [open, setOpen] = useState(false);
	const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
	const query = useInvoke(
		backend.userState.getQuotaOperations,
		backend.userState,
		[cursors[cursors.length - 1]],
		open && !!auth.user?.profile.sub,
		[auth.user?.profile.sub],
	);
	const items = asArray(query.data?.items);
	const names = useUsageNames(open ? items : []);
	return (
		<details
			className="group rounded-xl border"
			onToggle={(event) => setOpen(event.currentTarget.open)}
		>
			<summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-5 py-4 font-medium [&::-webkit-details-marker]:hidden">
				Operation history and pending usage
				<ChevronDown
					className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180"
					aria-hidden="true"
				/>
			</summary>
			<div className="border-t p-5 space-y-4">
				<p className="text-sm text-muted-foreground">
					See settled usage and pending reservations for each operation. Open an
					operation's details for token counts and costs.
				</p>
				{query.isLoading ? (
					<p>Loading operations…</p>
				) : query.isError ? (
					<Button variant="outline" onClick={() => query.refetch()}>
						Retry loading history
					</Button>
				) : (
					<>
						<div className="overflow-auto">
							<table className="w-full text-sm text-left">
								<thead>
									<tr>
										{[
											"Started",
											"App / model",
											"Status",
											"Cloud runtime",
											"Hosted AI usage",
											"AI operations",
											"Pending AI",
										].map((label) => (
											<th key={label} className="p-2">
												{label}
											</th>
										))}
									</tr>
								</thead>
								<tbody>
									{items.map((item) => (
										<tr key={item.id} className="border-t">
											<td className="p-2 whitespace-nowrap">
												{new Date(item.createdAt).toLocaleDateString(
													undefined,
													{ month: "short", day: "numeric" },
												)}
												<span className="block text-xs text-muted-foreground">
													{new Date(item.createdAt).toLocaleTimeString(
														undefined,
														{ hour: "2-digit", minute: "2-digit" },
													)}
												</span>
											</td>
											<td className="p-2">
												<p>{names.appName(item.appId)}</p>
												<p className="text-muted-foreground">
													{names.modelName(item.modelId)} ·{" "}
													{usageFundingLabel(item.fundingClass)}
												</p>
												<UsageOperationDetails
													id={item.id}
													appName={names.appName(item.appId)}
													modelName={names.modelName(item.modelId)}
												/>
											</td>
											<td className="p-2">
												<span className="inline-flex rounded-full bg-muted px-2 py-1 text-xs">
													{usageStatusLabel(item.status)}
												</span>
											</td>
											<td className="p-2">
												{formatQuota(item.used.runtimeMs, "cloud_runtime_ms")}
												{item.reserved.runtimeMs > 0 && (
													<span className="block text-xs text-muted-foreground">
														{formatQuota(
															item.reserved.runtimeMs,
															"cloud_runtime_ms",
														)}{" "}
														pending
													</span>
												)}
												{item.reserved.cloudStarts > 0 && (
													<span className="block text-xs text-muted-foreground">
														{item.reserved.cloudStarts.toLocaleString()} cloud{" "}
														{item.reserved.cloudStarts === 1
															? "start"
															: "starts"}{" "}
														pending
													</span>
												)}
											</td>
											<td className="p-2">
												{formatQuota(
													item.used.aiCostMicros,
													"hosted_ai_cost_micros",
												)}
											</td>
											<td className="p-2">
												{item.used.aiCalls.toLocaleString()}
												{item.reserved.aiCalls > 0 && (
													<span className="block text-xs text-muted-foreground">
														{item.reserved.aiCalls.toLocaleString()} pending
													</span>
												)}
											</td>
											<td className="p-2">
												{formatQuota(
													item.reserved.aiCostMicros,
													"hosted_ai_cost_micros",
												)}
											</td>
										</tr>
									))}
								</tbody>
							</table>
							{query.data && items.length === 0 && (
								<p className="py-8 text-center text-sm text-muted-foreground">
									No cloud operations recorded yet.
								</p>
							)}
						</div>
						<p className="text-xs text-muted-foreground">
							Pending reservations hold capacity until the outcome is confirmed.
							Closing a tab does not erase usage.
						</p>
						<div className="flex justify-between">
							<Button
								size="sm"
								variant="outline"
								disabled={cursors.length === 1}
								onClick={() => setCursors((value) => value.slice(0, -1))}
							>
								Previous
							</Button>
							<Button
								size="sm"
								variant="outline"
								disabled={!query.data?.nextCursor}
								onClick={() =>
									setCursors((value) => [
										...value,
										query.data?.nextCursor ?? undefined,
									])
								}
							>
								Next
							</Button>
						</div>
					</>
				)}
			</div>
		</details>
	);
}
