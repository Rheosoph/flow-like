"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { Copy, Filter } from "lucide-react";
import { useMemo } from "react";
import { toast } from "sonner";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
	RelativeTime,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
} from "../../../ui";
import type { IErrorReportRecord } from "./types";
import { statusCodeTone } from "./types";
import { UserPill } from "./user-pill";

interface ErrorDetailSheetProps {
	errorId: string | null;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	profile: IProfile | undefined;
	onApplyFilter?: (filter: { key: string; value: string }) => void;
}

function safeStringify(v: unknown) {
	try {
		return JSON.stringify(v, null, 2);
	} catch {
		return String(v);
	}
}

export function ErrorDetailSheet({
	errorId,
	open,
	onOpenChange,
	profile,
	onApplyFilter,
}: Readonly<ErrorDetailSheetProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const detail = useQuery<IErrorReportRecord>({
		queryKey: ["admin", "logs", "errors", errorId],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			if (!errorId) throw new Error("No error id");
			return backend.apiState.get<IErrorReportRecord>(
				profile,
				`admin/logs/errors/${encodeURIComponent(errorId)}`,
			);
		},
		enabled: Boolean(profile && errorId && open),
	});

	const tone = detail.data ? statusCodeTone(detail.data.status_code) : null;

	const detailsString = useMemo(() => {
		if (!detail.data?.details) return null;
		if (typeof detail.data.details === "string") return detail.data.details;
		return safeStringify(detail.data.details);
	}, [detail.data?.details]);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent className="w-full overflow-y-auto sm:max-w-xl">
				<SheetHeader>
					<SheetTitle className="flex items-center gap-2">
						{detail.data && tone ? (
							<Badge variant={tone.variant} className="font-mono">
								{detail.data.status_code}
							</Badge>
						) : null}
						{t('errorReport', 'Error report')}
					</SheetTitle>
					<SheetDescription className="font-mono text-xs">
						{errorId}
					</SheetDescription>
				</SheetHeader>

				<div className="space-y-4 px-4 pb-6">
					{detail.isLoading || !detail.data ? (
						<div className="space-y-3">
							<Skeleton className="h-4 w-3/4" />
							<Skeleton className="h-24 w-full" />
							<Skeleton className="h-24 w-full" />
						</div>
					) : (
						(() => {
							const record = detail.data;
							return (
								<>
									<div className="space-y-2 rounded-lg border bg-card/40 p-3 text-sm">
										<div className="flex items-center gap-2">
											<Badge
												variant="outline"
												className="font-mono text-[11px]"
											>
												{record.method}
											</Badge>
											<code className="flex-1 truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]">
												{record.path}
											</code>
											<Button
												size="sm"
												variant="ghost"
												onClick={() =>
													onApplyFilter?.({ key: "path", value: record.path })
												}
												title={t('filterByThisPath', 'Filter by this path')}
											>
												<Filter className="h-3.5 w-3.5" />
											</Button>
										</div>
										<div className="flex flex-wrap items-center gap-2">
											<Badge
												variant="secondary"
												className="cursor-pointer"
												onClick={() =>
													onApplyFilter?.({
														key: "public_code",
														value: record.public_code,
													})
												}
											>
												{record.public_code}
											</Badge>
											<RelativeTime
												value={record.created_at}
												className="text-xs text-muted-foreground"
											/>
											{record.user_id ? (
												<UserPill userId={record.user_id} />
											) : (
												<Badge variant="outline" className="text-xs">
													{t('anonymous', 'Anonymous')}
												</Badge>
											)}
										</div>
									</div>

									<div className="space-y-1">
										<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
											{t('summary', 'Summary')}
										</div>
										<p className="rounded-lg border bg-background/50 p-3 text-sm leading-relaxed whitespace-pre-wrap">
											{record.summary}
										</p>
									</div>

									{detailsString && (
										<div className="space-y-1">
											<div className="flex items-center justify-between">
												<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
													{t('details', 'Details')}
												</div>
												<Button
													variant="ghost"
													size="sm"
													onClick={() => {
														navigator.clipboard
															.writeText(detailsString)
															.catch(() => null);
														toast.success("Details copied");
													}}
												>
													<Copy className="mr-1 h-3 w-3" />
													{t('copy', 'Copy')}
												</Button>
											</div>
											<pre className="max-h-96 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-[11px] leading-relaxed">
												{detailsString}
											</pre>
										</div>
									)}

									<Separator />

									<div className="space-y-2">
										<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
											{t('referenceId', 'Reference id')}
										</div>
										<div className="flex items-center gap-2">
											<code className="flex-1 truncate rounded bg-muted px-2 py-1 font-mono text-xs">
												{record.id}
											</code>
											<Button
												size="sm"
												variant="outline"
												onClick={() => {
													navigator.clipboard
														.writeText(record.id)
														.catch(() => null);
													toast.success("Reference id copied");
												}}
											>
												<Copy className="h-3.5 w-3.5" />
											</Button>
										</div>
										<p className="text-xs text-muted-foreground">
											{t('usersCanShareThisIdPasteItInTheSearchBarToJumpBackToThisError', "Users can share this id; paste it in the search bar to jump back to this error.")}
										</p>
									</div>
								</>
							);
						})()
					)}
				</div>
			</SheetContent>
		</Sheet>
	);
}
