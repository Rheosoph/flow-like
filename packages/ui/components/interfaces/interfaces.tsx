import type { JSX, ReactElement, RefObject } from "react";
import type { IEvent, IEventPayload, INode } from "../../lib";
import type { IEventExecutionMode } from "../../lib/schema/flow/event";
import type { IHub } from "../../lib/schema/hub/hub";

export interface IToolBarActions {
	pushToolbarElements: (elements: ReactElement[]) => void;
	pushNavElements: (elements: ReactElement[]) => void;
}

export interface ISidebarActions {
	pushSidebar: (sidebar?: ReactElement) => void;
	toggleOpen: () => void;
	isMobile: () => boolean;
	isOpen: () => boolean;
}

export interface IUseInterfaceProps {
	appId: string;
	event: IEvent;
	config?: Partial<IEventPayload>;
	toolbarRef?: RefObject<IToolBarActions | null>;
	sidebarRef?: RefObject<ISidebarActions | null>;
}

export interface IConfigInterfaceProps {
	isEditing: boolean;
	appId: string;
	boardId: string;
	nodeId: string;
	node: INode;
	config: Partial<IEventPayload>;
	onConfigUpdate: (payload: IEventPayload) => void;
	/** Hub configuration for determining remote endpoints */
	hub?: IHub | null;
	/** Event ID for constructing webhook URLs */
	eventId?: string;
	/** Whether local execution is available (desktop app) */
	canExecuteLocally?: boolean;
	/**
	 * The event's own execution mode. Drives which URL/tutorial to render:
	 * Local events show localhost + tunnel setup; Remote events show the
	 * server endpoint. Defaults to Local when unset.
	 */
	eventExecutionMode?: IEventExecutionMode;
	/**
	 * Which slice of this config to render, from `getEventSections`. Components
	 * that declare sections render only the matching part; the rest ignore it and
	 * render everything, so splitting a component is opt-in and incremental.
	 */
	section?: string;
}

/** Where a sink can run */
export type SinkAvailability = "local" | "remote" | "both";

/** Sink availability configuration */
export interface ISinkConfig {
	/** Where this sink can run */
	availability: SinkAvailability;
	/** Description for users about execution context */
	description?: string;
}

export type IEventMapping = Record<
	string,
	{
		configs: Record<string, Partial<IEventPayload>>;
		eventTypes: string[];
		defaultEventType: string;
		useInterfaces: Record<
			string,
			(props: IUseInterfaceProps) => JSX.Element | null
		>;
		configInterfaces: Record<
			string,
			(props: IConfigInterfaceProps) => JSX.Element | null
		>;
		withSink: string[];
		/** Sink availability map - if not specified, defaults to "local" */
		sinkAvailability?: Record<string, ISinkConfig>;
	}
>;
