"use client";
import {
	type IBoard,
	type IEvent,
	type IEventMapping,
	type IEventPayload,
	type INode,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	formatEventTypeLabel,
} from "@flow-like/flow-like-ui";
import { IEventExecutionMode } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import type {
	IHub,
	ISupportedSinks,
} from "@flow-like/flow-like-ui/lib/schema/hub/hub";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { useEffect, useMemo } from "react";

/** Map event types to their corresponding sink type for hub lookup */
const EVENT_TYPE_TO_SINK_MAP: Record<string, keyof ISupportedSinks> = {
	api: "http",
	http: "http",
	webhook: "webhook",
	cron: "cron",
	telegram: "telegram",
	discord: "discord",
	slack: "slack",
	email: "email",
	mqtt: "mqtt",
	github: "github",
	rss: "rss",
	rest: "rest",
	mcp: "mcp",
};

/**
 * Determines sink availability based on hub configuration and local capabilities.
 * If hub has supported_sinks, use that to determine remote availability.
 * Local availability is always true if canExecuteLocally is true.
 * When hub hasn't loaded yet, falls back to the static config from EVENT_CONFIG.
 */
function computeSinkAvailability(
	eventType: string,
	hub?: IHub | null,
	canExecuteLocally?: boolean,
	staticConfig?: {
		availability: "local" | "remote" | "both";
		description?: string;
	} | null,
): { availability: "local" | "remote" | "both"; description?: string } | null {
	const sinkType = EVENT_TYPE_TO_SINK_MAP[eventType];
	const supportsLocal = canExecuteLocally ?? false;

	// If hub config is available, use dynamic computation
	if (hub) {
		const supportsRemote =
			sinkType != null && hub.supported_sinks?.[sinkType] === true;

		if (supportsRemote && supportsLocal) {
			return {
				availability: "both",
				description: i18next.t(
					"canRunLocallyOrOnRemoteServer",
					"Can run locally or on remote server",
				),
			};
		}
		if (supportsRemote) {
			return {
				availability: "remote",
				description: i18next.t(
					"runsOnRemoteServerOnly",
					"Runs on remote server only",
				),
			};
		}
		if (supportsLocal) {
			return {
				availability: "local",
				description: i18next.t(
					"runsLocallyOnlyDesktopApp",
					"Runs locally only (desktop app)",
				),
			};
		}
		return null;
	}

	// Hub not loaded yet — fall back to static config
	if (staticConfig) return staticConfig;

	if (supportsLocal) {
		return {
			availability: "local",
			description: i18next.t(
				"runsLocallyOnlyDesktopApp",
				"Runs locally only (desktop app)",
			),
		};
	}

	return null;
}

export function EventTypeConfiguration({
	eventConfig,
	node,
	event,
	disabled,
	onUpdate,
	hub,
	canExecuteLocally,
	eventExecutionMode,
	compact = false,
}: Readonly<{
	eventConfig: IEventMapping;
	node: INode;
	disabled: boolean;
	event: IEvent;
	onUpdate: (type: string, config: Partial<IEventPayload>) => void;
	/** Hub configuration for determining remote sink availability */
	hub?: IHub | null;
	/** Whether local execution is available (desktop app) */
	canExecuteLocally?: boolean;
	/** Event's own execution mode; filters event types to matching availability. */
	eventExecutionMode?: IEventExecutionMode;
	/** Render label and select on one compact row. */
	compact?: boolean;
}>) {
	const { t } = useTranslation("interfaces");
	const foundConfig = eventConfig[node?.name];

	useEffect(() => {
		const eventTypes = eventConfig[node?.name];
		if (!eventTypes) {
			console.warn(`No event types configured for node: ${node?.name}`);
			return;
		}

		if (!eventTypes.eventTypes.includes(event.event_type)) {
			onUpdate(
				eventTypes.defaultEventType,
				eventTypes.configs[eventTypes.defaultEventType] ?? {},
			);
		}
	}, [node?.name, event.event_type]);

	if (foundConfig?.eventTypes.length <= 1) return null;

	const matchesExecutionMode = (
		availability: "local" | "remote" | "both",
	): boolean => {
		if (!eventExecutionMode) return true;
		if (availability === "both") return true;
		if (eventExecutionMode === IEventExecutionMode.Local) {
			return availability === "local";
		}
		return availability === "remote";
	};

	// Filter event types to only those that have at least one available sink
	// AND match the event's execution mode (a Remote event must not offer
	// local-only types like IMAP/Discord).
	const availableEventTypes = foundConfig?.eventTypes.filter((type) => {
		if (foundConfig?.withSink?.includes(type)) {
			const staticCfg = foundConfig?.sinkAvailability?.[type] ?? null;
			const sinkConfig = computeSinkAvailability(
				type,
				hub,
				canExecuteLocally,
				staticCfg,
			);
			if (sinkConfig === null) return false;
			return matchesExecutionMode(sinkConfig.availability);
		}
		return true;
	});

	return (
		<div className={compact ? "flex shrink-0 items-center gap-2" : "space-y-3"}>
			<Label
				htmlFor="event_type"
				className={
					compact
						? "text-xs text-muted-foreground whitespace-nowrap"
						: undefined
				}
			>
				{compact ? "Type" : "Event Type"}
			</Label>
			<Select
				disabled={disabled}
				value={event.event_type}
				onValueChange={(value) => {
					onUpdate(value, foundConfig.configs[value] ?? {});
				}}
			>
				<SelectTrigger
					size={compact ? "sm" : "default"}
					className={compact ? "w-32 text-xs" : "w-full"}
				>
					<SelectValue
						placeholder={t("selectEventType", "Select event type")}
					/>
				</SelectTrigger>
				<SelectContent>
					{availableEventTypes?.map((type) => (
						<SelectItem key={type} value={type}>
							{formatEventTypeLabel(type)}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);
}

export function EventTranslation({
	appId,
	eventConfig,
	eventType,
	editing,
	board,
	nodeId,
	config,
	onUpdate,
	hub,
	eventId,
	canExecuteLocally,
	eventExecutionMode,
	section,
}: Readonly<{
	appId: string;
	eventConfig: IEventMapping;
	eventType: string;
	editing: boolean;
	config: Partial<IEventPayload>;
	board: IBoard;
	nodeId?: string;
	onUpdate: (payload: Partial<IEventPayload>) => void;
	hub?: IHub | null;
	eventId?: string;
	canExecuteLocally?: boolean;
	eventExecutionMode?: IEventExecutionMode;
	/** Slice of the config to render — see IConfigInterfaceProps.section. */
	section?: string;
}>) {
	const { t } = useTranslation("interfaces");
	// Fully controlled by `config`. Holding a local copy meant a parent reset —
	// Discard, or reloading the saved event — never reached the fields, so the
	// form kept showing edits the event no longer had.
	const node: INode | undefined = board.nodes[nodeId ?? ""];

	const foundEventConfig = useMemo(() => {
		return eventConfig[node?.name];
	}, [node?.name]);

	const ConfigInterface = useMemo(() => {
		if (!foundEventConfig) return null;
		return foundEventConfig.configInterfaces[eventType] || null;
	}, [foundEventConfig, eventType]);

	const configProps = useMemo(
		() => ({
			isEditing: editing,
			appId,
			boardId: board.id,
			config,
			node: node,
			nodeId: nodeId ?? "",
			hub,
			eventId,
			canExecuteLocally,
			eventExecutionMode,
			section,
			onConfigUpdate: (payload: Partial<IEventPayload>) => {
				onUpdate?.(payload);
			},
		}),
		[
			editing,
			board.app_id,
			board.id,
			config,
			node,
			nodeId,
			onUpdate,
			hub,
			eventId,
			canExecuteLocally,
			eventExecutionMode,
			section,
		],
	);

	if (!node) {
		return (
			<p className="text-red-500">{t("nodeNotFound", "Node not found.")}</p>
		);
	}

	if (!foundEventConfig || !ConfigInterface) {
		return (
			<div className="w-full space-y-4">
				<p className="text-sm text-muted-foreground">
					{t(
						"noSpecificConfigurationAvailableForThisEventType",
						"No specific configuration available for this event type.",
					)}
				</p>
			</div>
		);
	}

	return (
		<div className="w-full space-y-4">
			<ConfigInterface {...configProps} />
		</div>
	);
}
