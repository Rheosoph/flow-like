"use client";
import {
	Badge,
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
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@tm9657/flow-like-ui";
import type {
	IHub,
	ISupportedSinks,
} from "@tm9657/flow-like-ui/lib/schema/hub/hub";
import { Cloud, Laptop, MonitorSmartphone } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

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
	staticConfig?: { availability: "local" | "remote" | "both"; description?: string } | null,
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
				description: "Can run locally or on remote server",
			};
		}
		if (supportsRemote) {
			return {
				availability: "remote",
				description: "Runs on remote server only",
			};
		}
		if (supportsLocal) {
			return {
				availability: "local",
				description: "Runs locally only (desktop app)",
			};
		}
		return null;
	}

	// Hub not loaded yet — fall back to static config
	if (staticConfig) return staticConfig;

	if (supportsLocal) {
		return {
			availability: "local",
			description: "Runs locally only (desktop app)",
		};
	}

	return null;
}

function SinkAvailabilityBadge({
	availability,
	description,
}: Readonly<{
	availability: "local" | "remote" | "both";
	description?: string;
}>) {
	const config = {
		local: {
			icon: Laptop,
			label: "Local",
			variant: "secondary" as const,
		},
		remote: {
			icon: Cloud,
			label: "Remote",
			variant: "default" as const,
		},
		both: {
			icon: MonitorSmartphone,
			label: "Both",
			variant: "outline" as const,
		},
	}[availability];

	const Icon = config.icon;

	const badge = (
		<Badge variant={config.variant} className="text-xs gap-1 ml-2">
			<Icon className="h-3 w-3" />
			{config.label}
		</Badge>
	);

	if (description) {
		return (
			<TooltipProvider>
				<Tooltip>
					<TooltipTrigger asChild>{badge}</TooltipTrigger>
					<TooltipContent>
						<p>{description}</p>
					</TooltipContent>
				</Tooltip>
			</TooltipProvider>
		);
	}

	return badge;
}

export function EventTypeConfiguration({
	eventConfig,
	node,
	event,
	disabled,
	onUpdate,
	hub,
	canExecuteLocally,
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
}>) {
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

	// Filter event types to only those that have at least one available sink
	const availableEventTypes = foundConfig?.eventTypes.filter((type) => {
		// If this event type has a sink requirement, check availability
		if (foundConfig?.withSink?.includes(type)) {
			const staticCfg = foundConfig?.sinkAvailability?.[type] ?? null;
			const sinkConfig = computeSinkAvailability(type, hub, canExecuteLocally, staticCfg);
			return sinkConfig !== null;
		}
		// Event types without sinks are always available
		return true;
	});

	const getSinkAvailability = (type: string) => {
		if (!foundConfig?.withSink?.includes(type)) return null;
		const staticCfg = foundConfig?.sinkAvailability?.[type] ?? null;
		return computeSinkAvailability(type, hub, canExecuteLocally, staticCfg);
	};

	return (
		<div className="space-y-3">
			<Label htmlFor="event_type">Event Type</Label>
			<Select
				disabled={disabled}
				value={event.event_type}
				onValueChange={(value) => {
					onUpdate(value, foundConfig.configs[value] ?? {});
				}}
			>
				<SelectTrigger className="w-full">
					<SelectValue placeholder="Select event type" />
				</SelectTrigger>
				<SelectContent>
					{availableEventTypes?.map((type) => {
						const sinkConfig = getSinkAvailability(type);
						return (
							<SelectItem key={type} value={type}>
								<span className="flex items-center">
									{type
										.replace(/_/g, " ")
										.replace(/\b\w/g, (c) => c.toUpperCase())}
									{sinkConfig && (
										<SinkAvailabilityBadge
											availability={sinkConfig.availability}
											description={sinkConfig.description}
										/>
									)}
								</span>
							</SelectItem>
						);
					})}
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
}>) {
	const [intermediateConfig, setIntermediateConfig] =
		useState<Partial<IEventPayload>>(config);
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
			config: intermediateConfig,
			node: node,
			nodeId: nodeId ?? "",
			hub,
			eventId,
			canExecuteLocally,
			onConfigUpdate: (payload: Partial<IEventPayload>) => {
				setIntermediateConfig(payload);
				if (onUpdate) {
					onUpdate(payload);
				}
			},
		}),
		[
			editing,
			board.app_id,
			board.id,
			intermediateConfig,
			node,
			nodeId,
			onUpdate,
			hub,
			eventId,
			canExecuteLocally,
		],
	);

	if (!node) {
		return <p className="text-red-500">Node not found.</p>;
	}

	if (!foundEventConfig || !ConfigInterface) {
		return (
			<div className="w-full space-y-4">
				<p className="text-sm text-muted-foreground">
					No specific configuration available for this event type.
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
