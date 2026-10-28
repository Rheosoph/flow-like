"use client";

import { useTranslation } from "@flow-like/locales";
import { queryOptions, useQuery } from "@tanstack/react-query";
import { Loader2Icon, PlayIcon, UserRoundIcon } from "lucide-react";
import { useCallback } from "react";
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
import { userDisplayName } from "../../../lib/user-display";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type {
	IEventSinkStatus,
	IEventState,
} from "../../../state/backend-state/event-state";
import { Button } from "../../ui/button";
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

/** "Runs as {name}", or the hosted wording; undefined while the name is looked up, never an account id. */
function useRunsAsLabel(userId?: string, hosted = false): string | undefined {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const lookup = useQuery({
		...userLookupQueryOptions(backend.userState, userId),
		retry: false,
	});
	if (!userId || lookup.isPending) return undefined;
	const name = lookup.data ? userDisplayName(lookup.data, "") : "";
	if (!name) return t("runsAsAMember", "Runs as a member of this app");
	return hosted
		? t("visitorsUseAccess", "Visitors use {{name}}'s access to run events", {
				name,
			})
		: t("runsAsName", "Runs as {{name}}", { name });
}

export function RunAsChip({
	runAs,
	hosted = false,
	className,
}: Readonly<{ runAs: EventRunAs; hosted?: boolean; className?: string }>) {
	const { t } = useTranslation("settings");
	const label = useRunsAsLabel(
		runAs.kind === "bound" ? runAs.userId : undefined,
		hosted,
	);
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
	if (!label) return null;
	return (
		<span
			data-run-as="bound"
			title={label}
			className={cn(
				"inline-flex max-w-56 items-center gap-1 bg-muted text-muted-foreground",
				className,
			)}
		>
			<UserRoundIcon className="size-3 shrink-0" aria-hidden />
			<span className="truncate">{label}</span>
		</span>
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
