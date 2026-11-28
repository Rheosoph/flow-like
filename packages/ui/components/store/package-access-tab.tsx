"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Inbox, X } from "lucide-react";
import type { AccessRequest } from "../../lib/schema/wasm";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../../lib/user-display";
import { Button, RelativeTime, Skeleton } from "../ui";
import { UserAvatar } from "../ui/user-identity";
import { LoadError } from "./package-users-tab";
import type { PackageAccessModel } from "./package-workspace/use-package-access";
import {
	CountBadge,
	WorkspaceSection,
} from "./package-workspace/workspace-parts";

function RequestRow({
	request,
	busy,
	onAnswer,
}: Readonly<{
	request: AccessRequest;
	busy: boolean;
	onAnswer: (approve: boolean) => void;
}>) {
	const { t } = useTranslation("store");
	const name = userDisplayName(request, request.userId);
	const secondary = userSecondaryLabel(request);

	return (
		<li className="flex flex-wrap items-center gap-x-3 gap-y-2 py-2.5">
			<div className="flex min-w-0 flex-1 basis-56 items-center gap-3">
				<UserAvatar
					avatarUrl={userAvatarUrl(request)}
					initials={userInitials(request)}
					label={name}
				/>
				<div className="min-w-0">
					<p className="truncate text-sm font-medium" title={name}>
						{name}
					</p>
					<p className="truncate text-xs text-muted-foreground">
						{secondary && <span>{secondary} · </span>}
						<RelativeTime
							value={request.createdAt}
							className="whitespace-nowrap"
						/>
					</p>
					{request.comment && (
						<p className="mt-1 line-clamp-2 text-xs text-foreground/80">
							“{request.comment}”
						</p>
					)}
				</div>
			</div>
			<div className="ml-auto flex shrink-0 gap-1.5">
				<Button
					size="sm"
					variant="outline"
					disabled={busy}
					onClick={() => onAnswer(false)}
					aria-label={t("packageDeclineRequestFrom", {
						defaultValue: "Decline the request from {{name}}",
						name,
					})}
				>
					<X className="size-3.5" />
					{t("decline", "Decline")}
				</Button>
				<Button
					size="sm"
					disabled={busy}
					onClick={() => onAnswer(true)}
					aria-label={t("packageApproveRequestFrom", {
						defaultValue: "Approve the request from {{name}}",
						name,
					})}
				>
					<Check className="size-3.5" />
					{t("approve", "Approve")}
				</Button>
			</div>
		</li>
	);
}

/** People asking for access to a request-access package; approving makes them a User. */
export function PackageAccessTab({
	access,
}: Readonly<{ access: PackageAccessModel }>) {
	const { t } = useTranslation("store");
	const { requests, answerRequest } = access;

	return (
		<WorkspaceSection
			icon={Inbox}
			title={
				<span className="flex items-center gap-2">
					{t("accessRequests", "Access Requests")}
					{requests.list.length > 0 && (
						<CountBadge value={requests.list.length} />
					)}
				</span>
			}
		>
			{requests.isLoading ? (
				<Skeleton className="h-10 w-full" />
			) : requests.isError ? (
				<LoadError
					message={t(
						"packageAccessRequestsLoadFailed",
						"Couldn't load access requests.",
					)}
					onRetry={() => void requests.refetch()}
				/>
			) : requests.list.length === 0 ? (
				<p className="text-sm text-muted-foreground">
					{t(
						"packageNoAccessRequests",
						"Nobody is waiting for access. Requests arrive here while the package's visibility is “On request”.",
					)}
				</p>
			) : (
				<ul className="divide-y divide-border/60">
					{requests.list.map((request) => (
						<RequestRow
							key={request.id}
							request={request}
							busy={answerRequest.isPending}
							onAnswer={(approve) =>
								answerRequest.mutate({ id: request.id, approve })
							}
						/>
					))}
				</ul>
			)}
		</WorkspaceSection>
	);
}
