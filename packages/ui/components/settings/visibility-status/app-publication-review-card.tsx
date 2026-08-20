"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckCircle2,
	Clock3,
	MessageSquare,
	PauseCircle,
	XCircle,
} from "lucide-react";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
} from "../../../lib/user-display";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	RelativeTime,
	Skeleton,
} from "../../ui";

export interface RawAppPublicationActor {
	userId?: string;
	user_id?: string;
	username?: string | null;
	name?: string | null;
	avatar?: string | null;
}

export interface RawAppPublicationLogItem {
	id: string;
	authorId?: string | null;
	author_id?: string | null;
	author?: RawAppPublicationActor | null;
	message?: string | null;
	visibility?: string | null;
	createdAt?: string;
	created_at?: string;
	updatedAt?: string;
	updated_at?: string;
}

export interface RawAppPublicationRequestItem {
	id: string;
	targetVisibility?: string;
	target_visibility?: string;
	status: string;
	approverId?: string | null;
	approver_id?: string | null;
	createdAt?: string;
	created_at?: string;
	updatedAt?: string;
	updated_at?: string;
	logs?: RawAppPublicationLogItem[];
}

export interface AppPublicationActor {
	userId: string;
	username?: string;
	name?: string;
	avatar?: string;
}

export interface AppPublicationLogItem {
	id: string;
	authorId?: string;
	author?: AppPublicationActor;
	message?: string;
	visibility?: string;
	createdAt: string;
	updatedAt: string;
}

export interface AppPublicationRequestItem {
	id: string;
	targetVisibility: string;
	status: string;
	approverId?: string;
	createdAt: string;
	updatedAt: string;
	logs: AppPublicationLogItem[];
}

function normalizeActor(
	actor?: RawAppPublicationActor | null,
): AppPublicationActor | undefined {
	if (!actor) return undefined;

	const userId = actor.userId ?? actor.user_id;
	if (!userId) return undefined;

	return {
		userId,
		username: actor.username ?? undefined,
		name: actor.name ?? undefined,
		avatar: actor.avatar ?? undefined,
	};
}

function normalizeLabel(value?: string | null) {
	return value?.toLowerCase() ?? "";
}

export function normalizeAppPublicationRequests(
	requests: RawAppPublicationRequestItem[],
): AppPublicationRequestItem[] {
	return requests.map((request) => ({
		id: request.id,
		targetVisibility: normalizeLabel(
			request.targetVisibility ?? request.target_visibility,
		),
		status: normalizeLabel(request.status),
		approverId: request.approverId ?? request.approver_id ?? undefined,
		createdAt: request.createdAt ?? request.created_at ?? "",
		updatedAt: request.updatedAt ?? request.updated_at ?? "",
		logs: (request.logs ?? []).map((log) => ({
			id: log.id,
			authorId: log.authorId ?? log.author_id ?? undefined,
			author: normalizeActor(log.author),
			message: log.message ?? undefined,
			visibility: normalizeLabel(log.visibility),
			createdAt: log.createdAt ?? log.created_at ?? "",
			updatedAt: log.updatedAt ?? log.updated_at ?? "",
		})),
	}));
}

function formatLabel(value: string) {
	return value.replaceAll("_", " ");
}

function statusVariant(
	status: string,
): "default" | "secondary" | "destructive" {
	switch (status) {
		case "accepted":
			return "default";
		case "rejected":
			return "destructive";
		default:
			return "secondary";
	}
}

function statusIcon(status: string) {
	switch (status) {
		case "accepted":
			return <CheckCircle2 className="h-4 w-4" />;
		case "rejected":
			return <XCircle className="h-4 w-4" />;
		case "on_hold":
			return <PauseCircle className="h-4 w-4" />;
		default:
			return <Clock3 className="h-4 w-4" />;
	}
}

function actorLabel(log: AppPublicationLogItem) {
	return userDisplayName(log.author, "System");
}

export function AppPublicationReviewCard({
	requests,
	isLoading,
	error,
}: {
	requests: AppPublicationRequestItem[];
	isLoading?: boolean;
	error?: string | null;
}) {
	const { t } = useTranslation("settings");
	if (!isLoading && !error && requests.length === 0) {
		return null;
	}

	return (
		<Card className="border-amber-500/30 bg-amber-500/5">
			<CardHeader>
				<CardTitle className="flex items-center gap-2 text-base">
					<MessageSquare className="h-4 w-4" />
					{t("publicationReview", "Publication Review")}
				</CardTitle>
				<CardDescription>
					{t(
						"currentPublicationRequestsAndAuditorCommentsForThisAppAppearHere",
						"Current publication requests and auditor comments for this app appear here.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{isLoading ? (
					<Skeleton className="h-28 w-full" />
				) : error ? (
					<p className="text-sm text-destructive">{error}</p>
				) : (
					<div className="space-y-4">
						{requests.map((request) => (
							<div
								key={request.id}
								className="rounded-lg border bg-background/80 p-4"
							>
								<div className="flex flex-wrap items-center gap-2">
									<Badge variant={statusVariant(request.status)}>
										<span className="flex items-center gap-1">
											{statusIcon(request.status)}
											{formatLabel(request.status)}
										</span>
									</Badge>
									<Badge variant="outline">
										{t("target", "Target:")}{" "}
										{formatLabel(request.targetVisibility)}
									</Badge>
									<span className="text-xs text-muted-foreground">
										{t("submitted", "Submitted")}{" "}
										<RelativeTime
											value={request.createdAt}
											fallback={request.createdAt || "Unknown"}
										/>
									</span>
								</div>

								{request.logs.length === 0 ? (
									<p className="mt-3 text-sm text-muted-foreground">
										{t(
											"noReviewEventsRecordedYet",
											"No review events recorded yet.",
										)}
									</p>
								) : (
									<div className="mt-4 space-y-3">
										{request.logs.map((log) => {
											const label = actorLabel(log);

											return (
												<div key={log.id} className="flex items-start gap-3">
													<Avatar className="h-8 w-8">
														<AvatarImage
															src={userAvatarUrl(log.author)}
															alt={label}
														/>
														<AvatarFallback>
															{userInitials(label)}
														</AvatarFallback>
													</Avatar>
													<div className="min-w-0 flex-1 space-y-1">
														<div className="flex flex-wrap items-center gap-2 text-sm">
															<span className="font-medium">{label}</span>
															<span className="text-muted-foreground">
																<RelativeTime
																	value={log.createdAt}
																	fallback={log.createdAt || "Unknown"}
																/>
															</span>
															{log.visibility ? (
																<Badge variant="outline">
																	{formatLabel(log.visibility)}
																</Badge>
															) : null}
														</div>
														{log.message ? (
															<p className="text-sm text-muted-foreground">
																{log.message}
															</p>
														) : (
															<p className="text-sm text-muted-foreground">
																{t("noCommentProvided", "No comment provided.")}
															</p>
														)}
													</div>
												</div>
											);
										})}
									</div>
								)}
							</div>
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}
