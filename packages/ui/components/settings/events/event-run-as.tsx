"use client";

import { useTranslation } from "@flow-like/locales";
import { queryOptions, useQuery } from "@tanstack/react-query";
import { Loader2Icon, PlayIcon, UserRoundIcon } from "lucide-react";
import { useCallback, useMemo } from "react";
import { toast } from "sonner";
import { userLookupQueryOptions } from "../../../hooks/use-user-lookup";
import {
	getHostedFrontendKind,
	parseFrontendHosting,
} from "../../../lib/frontend-hosting";
import {
	type IEvent,
	IEventExecutionMode,
	IEventExposure,
} from "../../../lib/schema/flow/event";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
} from "../../../lib/user-display";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type {
	IEventSinkStatus,
	IEventState,
} from "../../../state/backend-state/event-state";
import { Button } from "../../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { UserAvatar } from "../../ui/user-identity";
import { Banner } from "../devices/primitives/banner";
import { isLocalTriggerDismissal } from "./local-trigger-token";
import type { useSinkActivation } from "./use-sink-activation";

/**
 * Whom a hub trigger runs as. `released`: the hub paused it because that user can no longer
 * run it. `legacy`: it still runs with a token saved before run-as users existed. `unbound`:
 * it runs without a user.
 */
export type EventRunAs =
	| { kind: "bound"; userId: string }
	| { kind: "released" }
	| { kind: "legacy" }
	| { kind: "unbound" };

/**
 * Whom a run of a bound hub sink acts for. `public_calls`: anonymous calls to a PUBLIC REST or
 * MCP Event run as its run-as user, callers signed in to Flow-Like as themselves.
 * `calling_person`: an INTERNAL one runs as the person the calling app passes along, and as the
 * run-as user only when it passes none. `visitors`: anonymous visitors of a hosted page use the
 * run-as user's access. `run_as_user`: every run is the run-as user's.
 */
export type RunAsAudience =
	| "public_calls"
	| "calling_person"
	| "visitors"
	| "run_as_user";

type RequestToggle = ReturnType<typeof useSinkActivation>["requestToggle"];

/** One cache entry per stored event, shared by the list and the editor. */
export function eventSinkStatusQuery(
	eventState: IEventState,
	appId: string,
	event: IEvent,
) {
	return queryOptions<IEventSinkStatus>({
		queryKey: [
			"eventSinkStatus",
			appId,
			event.id,
			{
				active: event.active,
				event_type: event.event_type,
				execution_mode: event.execution_mode,
				config: event.config,
				event_version: event.event_version,
			},
		],
		queryFn: () => eventState.getEventSinkStatus(event.id, { appId, event }),
		enabled: !!appId && event.active,
		retry: false,
		staleTime: 0,
		refetchInterval: 30_000,
	});
}

/**
 * Visitors of the app's hosted link run it, so their runs use its run-as user's access.
 * Mirrors the hub's per-event `hosting()` in packages/api/src/routes/frontend.rs.
 */
export function servesHostedVisitors(
	event: Pick<
		IEvent,
		"config" | "event_type" | "default_page_id" | "execution_mode" | "exposure"
	>,
): boolean {
	return (
		event.execution_mode === IEventExecutionMode.Remote &&
		(event.exposure ?? IEventExposure.Public) === IEventExposure.Public &&
		event.event_type !== "ontology_action" &&
		getHostedFrontendKind(event) !== null &&
		parseFrontendHosting(parseUint8ArrayToJson(event.config)).enabled
	);
}

/**
 * REST and MCP calls pick their identity per caller (packages/api/src/routes/inbound.rs), which a
 * hosted page on the same Event does not change. `hostedVisitors` is `servesHostedVisitors(event)`.
 */
export function eventRunAsAudience(
	event: Pick<IEvent, "event_type" | "exposure">,
	hostedVisitors: boolean,
): RunAsAudience {
	if (event.event_type === "rest" || event.event_type === "mcp")
		return event.exposure === IEventExposure.Internal
			? "calling_person"
			: "public_calls";
	return hostedVisitors ? "visitors" : "run_as_user";
}

/** Whom the hub's sink runs the event as; null when no hub sink answered. */
export function eventRunAs(
	event: Pick<IEvent, "active">,
	status?: IEventSinkStatus,
): EventRunAs | null {
	const userId = status?.run_as_user_id;
	if (!status || userId === undefined) return null;
	if (userId) return status.active ? { kind: "bound", userId } : null;
	if (status.legacy_token) return status.active ? { kind: "legacy" } : null;
	if (status.active) return { kind: "unbound" };
	return event.active ? { kind: "released" } : null;
}

/** Turns a released trigger back on through the activation flow, which binds the caller. */
export function useTurnOnAsMe(requestToggle: RequestToggle) {
	const { t } = useTranslation("settings");
	return useCallback(
		(event: IEvent) =>
			requestToggle(event, {
				active: true,
				requiresSink: true,
				onSettled: (error) => {
					if (isLocalTriggerDismissal(error)) return;
					if (error) {
						toast.error(
							t("couldNotTurnOnEvent", "Could not turn on {{name}}.", {
								name: event.name,
							}),
						);
						return;
					}
					toast.success(
						t("eventNowRunsAsYou", "{{name}} now runs as you.", {
							name: event.name,
						}),
					);
				},
			}),
		[requestToggle, t],
	);
}

interface RunAsUser {
	name: string;
	avatarUrl?: string;
	initials: string;
}

/** The run-as user's name and avatar: undefined while looked up, null when no name resolves. Never an account id. */
function useRunAsUser(userId?: string): RunAsUser | null | undefined {
	const backend = useBackend();
	const lookup = useQuery({
		...userLookupQueryOptions(backend.userState, userId),
		retry: false,
	});
	return useMemo(() => {
		if (!userId || lookup.isPending) return undefined;
		const name = lookup.data ? userDisplayName(lookup.data, "") : "";
		if (!name) return null;
		return {
			name,
			avatarUrl: userAvatarUrl(lookup.data),
			initials: userInitials(name),
		};
	}, [userId, lookup.isPending, lookup.data]);
}

/** The bound chip's label and its tooltip hint; `name` is undefined when it does not resolve. */
function useRunAsWording(
	audience: RunAsAudience,
	name?: string,
): { label: string; hint?: string } {
	const { t } = useTranslation("settings");
	const member = t("runsAsAMember", "Runs as a member of this app");
	switch (audience) {
		case "public_calls":
			return {
				label: name
					? t("publicCallsRunAsName", "Public calls run as {{name}}", { name })
					: member,
				hint: t(
					"signedInCallersRunAsThemselves",
					"Callers signed in to Flow-Like run as themselves.",
				),
			};
		case "calling_person":
			return {
				label: t("runsAsTheCallingPerson", "Runs as the calling person"),
				hint: name
					? t(
							"callsWithoutAPersonRunAsName",
							"Calls from other apps that pass no person run as {{name}}.",
							{ name },
						)
					: t(
							"callsWithoutAPersonRunAsAMember",
							"Calls from other apps that pass no person run as a member of this app.",
						),
			};
		case "visitors":
			return {
				label: name
					? t(
							"visitorsUseAccess",
							"Visitors use {{name}}'s access to run events",
							{ name },
						)
					: member,
			};
		case "run_as_user":
			return {
				label: name ? t("runsAsName", "Runs as {{name}}", { name }) : member,
			};
	}
}

/** An INTERNAL REST/MCP chip shows no avatar: its run-as user only covers calls that pass no person. */
function BoundRunAsChip({
	user,
	audience,
	className,
}: Readonly<{
	user: RunAsUser | null;
	audience: RunAsAudience;
	className?: string;
}>) {
	const { label, hint } = useRunAsWording(audience, user?.name);
	const avatar = audience === "calling_person" ? null : user;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span
					data-run-as="bound"
					className={cn(
						"inline-flex max-w-56 items-center gap-1 bg-muted text-muted-foreground",
						className,
					)}
				>
					{avatar ? (
						<span aria-hidden className="inline-flex shrink-0">
							<UserAvatar
								avatarUrl={avatar.avatarUrl}
								initials={avatar.initials}
								label={avatar.name}
								size="xs"
								className="size-4 ring-1 ring-background"
							/>
						</span>
					) : (
						<UserRoundIcon className="size-3 shrink-0" aria-hidden />
					)}
					<span data-run-as-label className="truncate">
						{label}
					</span>
				</span>
			</TooltipTrigger>
			<TooltipContent side="bottom" className="max-w-64">
				<p className="font-medium">{label}</p>
				{hint && <p className="text-primary-foreground/80">{hint}</p>}
			</TooltipContent>
		</Tooltip>
	);
}

export function RunAsChip({
	runAs,
	audience,
	className,
}: Readonly<{
	runAs: EventRunAs;
	audience: RunAsAudience;
	className?: string;
}>) {
	const { t } = useTranslation("settings");
	const user = useRunAsUser(runAs.kind === "bound" ? runAs.userId : undefined);
	if (runAs.kind === "released") {
		const paused = t(
			"pausedRunAsCannotRun",
			"Paused: the person it ran as can no longer run it",
		);
		return (
			<span
				data-run-as="released"
				title={paused}
				className={cn(
					className,
					"max-w-56 truncate bg-warning-bg text-warning",
				)}
			>
				{paused}
			</span>
		);
	}
	if (runAs.kind === "legacy") {
		const legacy = t(
			"legacyTokenNotice",
			"Runs with a saved token until someone saves or turns it on",
		);
		return (
			<span
				data-run-as="legacy"
				title={legacy}
				className={cn(
					className,
					"max-w-56 truncate bg-muted text-muted-foreground",
				)}
			>
				{legacy}
			</span>
		);
	}
	if (runAs.kind === "unbound")
		return (
			<span
				data-run-as="unbound"
				title={t(
					"runsWithoutAUserHint",
					"Saving it or turning it on again runs it as you.",
				)}
				className={cn(className, "bg-muted text-muted-foreground")}
			>
				{t("runsWithoutAUser", "Runs without a user")}
			</span>
		);
	if (user === undefined) return null;
	return (
		<BoundRunAsChip user={user} audience={audience} className={className} />
	);
}

interface TurnOnProps {
	busy: boolean;
	canWrite: boolean;
	writeDeniedMessage: string;
}

function TurnOnButton({
	label,
	busy,
	canWrite,
	writeDeniedMessage,
	onClick,
}: Readonly<TurnOnProps & { label: string; onClick: () => void }>) {
	const Icon = busy ? Loader2Icon : PlayIcon;
	return (
		<Button
			variant="outline"
			size="sm"
			disabled={busy || !canWrite}
			title={canWrite ? undefined : writeDeniedMessage}
			onClick={onClick}
		>
			<Icon className={cn("h-4 w-4", busy && "animate-spin")} />
			{label}
		</Button>
	);
}

/** The editor's notice under its status: a paused trigger to turn back on, or a legacy one. */
export function EventRunAsNotice({
	runAs,
	onTurnOn,
	...turnOn
}: Readonly<TurnOnProps & { runAs: EventRunAs; onTurnOn: () => void }>) {
	const { t } = useTranslation("settings");
	const note = "basis-full text-[0.7rem] leading-tight text-muted-foreground";
	if (runAs.kind === "unbound")
		return (
			<p className={note}>
				{t(
					"runsWithoutAUserHint",
					"Saving it or turning it on again runs it as you.",
				)}
			</p>
		);
	if (runAs.kind === "legacy")
		return (
			<p className={note}>
				{t(
					"legacyTokenNotice",
					"Runs with a saved token until someone saves or turns it on",
				)}
			</p>
		);
	if (runAs.kind !== "released") return null;
	return (
		<Banner
			tone="warning"
			className="basis-full"
			title={t(
				"pausedRunAsCannotRun",
				"Paused: the person it ran as can no longer run it",
			)}
			actions={
				<TurnOnButton
					{...turnOn}
					label={t("turnOn", "Turn on")}
					onClick={onTurnOn}
				/>
			}
		>
			{t(
				"runsWithoutAUserHint",
				"Saving it or turning it on again runs it as you.",
			)}
		</Banner>
	);
}

/** The list's banner for every trigger the hub paused because its user can no longer run it. */
export function ReleasedEventsBanner({
	events,
	pendingId,
	canWrite,
	writeDeniedMessage,
	onTurnOn,
}: Readonly<{
	events: IEvent[];
	pendingId: string | null;
	canWrite: boolean;
	writeDeniedMessage: string;
	onTurnOn: (event: IEvent) => void;
}>) {
	const { t } = useTranslation("settings");
	if (events.length === 0) return null;
	return (
		<Banner
			tone="warning"
			className="shrink-0"
			title={t("countEventsPaused", {
				defaultValue_one: "1 event paused",
				defaultValue_other: "{{count}} events paused",
				count: events.length,
			})}
			actions={events.map((event) => (
				<TurnOnButton
					key={event.id}
					busy={pendingId === event.id}
					canWrite={canWrite}
					writeDeniedMessage={writeDeniedMessage}
					label={t("turnOnName", "Turn on {{name}}", { name: event.name })}
					onClick={() => onTurnOn(event)}
				/>
			))}
		>
			{t(
				"eventsPausedRunAsCannotRunDetail",
				"The person each one ran as can no longer run it. Turning one on runs it as you.",
			)}
		</Banner>
	);
}
