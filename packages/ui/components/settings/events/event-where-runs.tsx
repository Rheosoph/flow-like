"use client";

import { useTranslation } from "@flow-like/locales";
import { Cloud, Loader2, Monitor, MonitorSmartphone } from "lucide-react";
import { useCallback, useId } from "react";
import { toast } from "sonner";
import type { IOAuthConsentStore } from "../../../db/oauth-db";
import { useInvalidateInvoke } from "../../../hooks/use-invoke";
import {
	type EventDefinition,
	type EventSinkDefinition,
	isServerOnlyEventType,
	sinkSupportsEventExecution,
} from "../../../lib/event-definitions";
import type {
	IOAuthProvider,
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "../../../lib/oauth/types";
import {
	type IEvent,
	IEventExecutionMode,
} from "../../../lib/schema/flow/event";
import type { IHub } from "../../../lib/schema/hub/hub";
import { useBackend } from "../../../state/backend-state";
import { OAuthConsentDialog } from "../../oauth/oauth-consent-dialog";
import { PatSelectorDialog } from "../../pat-selector-dialog";
import { Button } from "../../ui/button";
import { useSinkActivation } from "./use-sink-activation";

type Target = "hub" | "computer";

export interface EventWhereRunsProps {
	appId: string;
	event: IEvent;
	/** The event definition of the trigger node, for the sink types it can use. */
	mapping?: Pick<EventDefinition, "withSink" | "sinkAvailability">;
	/** The flow's own execution mode, when it pins one. */
	boardExecutionMode?: string | null;
	/** `null` while the check is still running. */
	isOffline: boolean | null;
	hub?: IHub;
	canWrite: boolean;
	writeDeniedMessage: string;
	/** The editor holds changes that are not saved: taking the event back would drop them. */
	dirty?: boolean;
	tokenStore?: IOAuthTokenStoreWithPending;
	consentStore?: IOAuthConsentStore;
	onStartOAuth?: (provider: IOAuthProvider) => Promise<void>;
	onRefreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
	onOpenRunsOn: () => void;
	/** The event as stored once it no longer is device-only. */
	onTakenBack: (saved: IEvent) => void | Promise<void>;
}

/**
 * Where a device-only event runs, and the way back: the hub or this computer
 * can take it over, through the same PAT and OAuth consent as Activate.
 */
export function EventWhereRuns({
	appId,
	event,
	mapping,
	boardExecutionMode,
	isOffline,
	hub,
	canWrite,
	writeDeniedMessage,
	dirty = false,
	tokenStore,
	consentStore,
	onStartOAuth,
	onRefreshToken,
	onOpenRunsOn,
	onTakenBack,
}: Readonly<EventWhereRunsProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const reasonsId = useId();
	const canExecuteLocally = backend.capabilities().canExecuteLocally;
	const eventType = event.event_type;
	const requiresSink = mapping?.withSink.includes(eventType) ?? false;
	const sink = mapping?.sinkAvailability?.[eventType];

	const { requestToggle, pendingId, dialogProps } = useSinkActivation({
		appId,
		tokenStore,
		consentStore,
		hub,
		onStartOAuth,
		onRefreshToken,
		onChanged: async (saved) => {
			await invalidate(backend.eventState.getEvents, [appId]);
			await onTakenBack(saved);
		},
	});

	const facts: WhereFacts = {
		eventType,
		requiresSink,
		sink,
		canExecuteLocally,
		boardExecutionMode,
		isOffline,
		hub,
	};
	const unavailable = (target: Target): string | null =>
		(!canWrite ? writeDeniedMessage : null) ??
		(dirty
			? t(
					"saveBeforeMovingEvent",
					"Save or discard your changes before changing where it runs.",
				)
			: null) ??
		(target === "hub" ? hubBlocker(t, facts) : computerBlocker(t, facts));

	const moveTo = useCallback(
		(target: Target) => {
			const hubTarget = target === "hub";
			void requestToggle(
				{
					...event,
					execution_mode: hubTarget
						? IEventExecutionMode.Remote
						: IEventExecutionMode.Local,
				},
				{
					active: event.active,
					requiresSink,
					upsertOptions: { source: "default" },
					onSettled: (error) => {
						if (error) {
							toast.error(
								t(
									"couldNotMoveEvent",
									"Could not change where {{name}} runs.",
									{
										name: event.name,
									},
								),
							);
							return;
						}
						toast.success(
							hubTarget
								? t("eventNowRunsOnHub", "{{name}} now runs on the hub.", {
										name: event.name,
									})
								: t(
										"eventNowRunsOnComputer",
										"{{name}} now runs on this computer.",
										{ name: event.name },
									),
						);
					},
				},
			);
		},
		[event, requestToggle, requiresSink, t],
	);

	const busy = pendingId === event.id;
	const options = [
		{
			target: "hub" as const,
			label: t("runOnTheHub", "Run on the hub"),
			Icon: Cloud,
		},
		{
			target: "computer" as const,
			label: t("runOnThisComputer", "Run on this computer"),
			Icon: Monitor,
		},
	].map((option) => ({ ...option, reason: unavailable(option.target) }));

	return (
		<div className="basis-full space-y-2.5 border-t pt-3" data-device-only-card>
			<p className="text-sm text-muted-foreground">
				{t(
					"deviceOnlyEventExplained",
					"Runs only on the devices you deploy it to. The hub and this computer don't start it.",
				)}
			</p>
			<div className="flex flex-wrap items-center gap-2">
				<Button variant="outline" size="sm" onClick={onOpenRunsOn}>
					<MonitorSmartphone className="h-4 w-4" />
					{t("openRunsOn", "Open Runs on")}
				</Button>
				{options.map(({ target, label, Icon, reason }) => (
					<Button
						key={target}
						variant="outline"
						size="sm"
						disabled={!!reason || busy}
						aria-describedby={reason ? `${reasonsId}-${target}` : undefined}
						onClick={() => moveTo(target)}
					>
						{busy ? (
							<Loader2 className="h-4 w-4 animate-spin" />
						) : (
							<Icon className="h-4 w-4" />
						)}
						{label}
					</Button>
				))}
			</div>
			{options.map(({ target, label, reason }) =>
				reason ? (
					<p
						key={target}
						id={`${reasonsId}-${target}`}
						className="text-xs text-muted-foreground"
					>
						{`${label}: ${reason}`}
					</p>
				) : null,
			)}
			<PatSelectorDialog
				{...dialogProps.pat}
				title={t("authorizeThisChange", "Authorize this change")}
				description={t(
					"registeringOrRemovingAnEventSinkNeedsAPersonalAccessToken",
					"Registering or removing an event sink needs a Personal Access Token.",
				)}
			/>
			<OAuthConsentDialog {...dialogProps.consent} />
		</div>
	);
}

type Translate = (key: string, defaultValue: string) => string;

interface WhereFacts {
	eventType: string;
	requiresSink: boolean;
	sink: EventSinkDefinition | undefined;
	canExecuteLocally: boolean;
	boardExecutionMode?: string | null;
	isOffline: boolean | null;
	hub?: IHub;
}

const firstReason = (checks: ReadonlyArray<readonly [boolean, string]>) =>
	checks.find(([blocked]) => blocked)?.[1] ?? null;

function hubBlocker(t: Translate, facts: WhereFacts): string | null {
	const { eventType, sink, hub } = facts;
	return firstReason([
		[
			facts.isOffline === null,
			t("checkingAppOnline", "Checking whether this app is online…"),
		],
		[
			facts.isOffline === true,
			t(
				"hubNeedsOnlineApp",
				"This app is not synced to a hub, so the hub cannot run it.",
			),
		],
		[
			facts.boardExecutionMode === "Local",
			t("flowRunsLocallyOnly", "This flow is set to run locally only."),
		],
		[
			facts.requiresSink &&
				!sinkSupportsEventExecution(sink, "Remote", facts.canExecuteLocally),
			t("triggerNotOnHub", "This trigger cannot run on the hub."),
		],
		[
			isServerOnlyEventType(eventType) &&
				!!hub &&
				hub.supported_sinks?.[eventType] !== true,
			t("hubLacksEventType", "This hub does not support this event type."),
		],
	]);
}

function computerBlocker(t: Translate, facts: WhereFacts): string | null {
	return firstReason([
		[
			!facts.canExecuteLocally,
			t("computerCannotRunFlows", "This app cannot run flows on this device."),
		],
		[
			facts.boardExecutionMode === "Remote",
			t("flowRunsOnHubOnly", "This flow is set to run on the hub only."),
		],
		[
			isServerOnlyEventType(facts.eventType),
			t("typeRunsOnHubOnly", "This event type runs only on the hub."),
		],
		[
			facts.requiresSink &&
				!sinkSupportsEventExecution(
					facts.sink,
					"Local",
					facts.canExecuteLocally,
				),
			t("triggerNotOnComputer", "This trigger cannot run on this computer."),
		],
	]);
}
