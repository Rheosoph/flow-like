import { IExecutionMode } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import type { IElementDemand } from "@flow-like/flow-like-ui/lib/schema/flow/element-demand";
import type { IEvent } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import { IEventExecutionMode } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import type { PageTrigger } from "@flow-like/flow-like-ui/lib/schema/flow/page-trigger";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import type { IEventState } from "@flow-like/flow-like-ui/state/backend-state/event-state";
import type { IPage } from "@flow-like/flow-like-ui/state/backend-state/page-state";
import { type ServiceRequest, consumeServiceStream } from "./transport";

export interface Inventory {
	project_id: string;
	events: IEvent[];
}
export interface PageBootstrap {
	project_id: string;
	event_id: string;
	event: IEvent;
	page: IPage;
	execution_revision: string;
	element_demand: Pick<IElementDemand, "selectors" | "dynamic">;
	route?: string;
}

function supportedTrigger(trigger: PageTrigger, revision?: string): boolean {
	if (trigger.manifestRevision !== revision) return false;
	if (trigger.kind !== "action" || !trigger.capabilityJwt) return true;
	return (
		/^da1_[a-f0-9]{32}$/.test(trigger.actionId) &&
		/^sac1\.[a-f0-9]{32}\.[A-Za-z0-9_-]{43}$/.test(trigger.capabilityJwt)
	);
}

function restricted<T extends object>(methods: Partial<T> = {}): T {
	return new Proxy(methods, {
		get: (target, property) =>
			property in target
				? Reflect.get(target, property)
				: async () => {
						throw new Error(
							"This operation is unavailable in the service interface.",
						);
					},
	}) as T;
}

export function createServiceBackend(
	inventory: Inventory,
	request: ServiceRequest,
	signal: AbortSignal,
) {
	const runs = new Map<string, AbortController>();
	const assertApp = (id: string) => {
		if (id !== inventory.project_id)
			throw new Error("This request belongs to another project.");
	};
	const eventFor = (app: string, id: string) => {
		assertApp(app);
		const event = inventory.events.find((event) => event.id === id);
		if (!event) throw new Error("This event is not deployed here.");
		return event;
	};
	const bootstrap = async (app: string, id: string): Promise<PageBootstrap> => {
		const event = eventFor(app, id);
		if (!event.default_page_id) throw new Error("This event has no Page.");
		const data = (await (
			await request(`/pages/${encodeURIComponent(id)}/bootstrap`, { signal })
		).json()) as PageBootstrap;
		if (
			data.project_id !== app ||
			data.event_id !== id ||
			data.page.id !== event.default_page_id ||
			!data.execution_revision
		)
			throw new Error("The Page does not match this deployment.");
		return data;
	};
	const eventState = restricted<IEventState>({
		checkEventOAuth: undefined,
		getEvent: async (app, id) => eventFor(app, id),
		getEventAuthoritative: async (app, id) => eventFor(app, id),
		getEvents: async (app) => {
			assertApp(app);
			return inventory.events;
		},
		getEventsAuthoritative: async (app) => {
			assertApp(app);
			return inventory.events;
		},
		prerunEvent: async (app, id, _version, trigger) => {
			const event = eventFor(app, id);
			const page = trigger ? await bootstrap(app, id) : undefined;
			if (trigger && !supportedTrigger(trigger, page?.execution_revision))
				throw new Error(
					"The Page has changed. Reload it before invoking this action.",
				);
			return {
				board_id: event.board_id,
				runtime_variables: [],
				oauth_requirements: [],
				requires_local_execution: false,
				execution_mode: IExecutionMode.Remote,
				event_execution_mode: IEventExecutionMode.Remote,
				can_execute_locally: false,
				manifest_revision: page?.execution_revision,
				page_id: page?.page.id,
			};
		},
		executeEvent: async (
			app,
			id,
			payload,
			_stream,
			onRunId,
			onEvents,
			_consent,
			trigger,
			beforeDispatch,
		) => {
			const event = eventFor(app, id);
			let path: string;
			let body: unknown;
			if (event.default_page_id) {
				if (!trigger) throw new Error("Page execution requires a Page action.");
				const page = await bootstrap(app, id);
				if (!supportedTrigger(trigger, page.execution_revision))
					throw new Error(
						"The Page action is stale or unsupported. Reload the Page.",
					);
				path = `/pages/${encodeURIComponent(id)}/invoke`;
				body = {
					manifest_revision: trigger.manifestRevision,
					trigger:
						trigger.kind === "action"
							? {
									kind: "action",
									action_id: trigger.actionId,
									...(trigger.capabilityJwt
										? { capability_jwt: trigger.capabilityJwt }
										: {}),
								}
							: { kind: "special", special_event: trigger.specialEvent },
					payload: payload.payload,
				};
			} else {
				if (event.event_type !== "simple_chat" || trigger)
					throw new Error("This event cannot run through this interface.");
				path = `/chat/${encodeURIComponent(id)}`;
				body = payload.payload;
			}
			const controller = new AbortController();
			const abort = () => controller.abort();
			signal.addEventListener("abort", abort, { once: true });
			if (signal.aborted) controller.abort();
			let runId: string | undefined;
			try {
				beforeDispatch?.();
				const response = await request(path, {
					method: "POST",
					body: JSON.stringify(body),
					signal: controller.signal,
				});
				if (!response.body)
					throw new Error("The service returned no execution stream.");
				await consumeServiceStream(response.body, onEvents, (id) => {
					runId = id;
					runs.set(id, controller);
					onRunId?.(id);
				});
				return undefined;
			} finally {
				if (runId) runs.delete(runId);
				signal.removeEventListener("abort", abort);
			}
		},
		cancelExecution: async (id) => {
			runs.get(id)?.abort();
		},
	});
	const backend: IBackendState = {
		appState: restricted(),
		apiState: restricted(),
		apiKeyState: restricted(),
		bitState: restricted(),
		boardState: restricted(),
		teamState: restricted(),
		roleState: restricted(),
		storageState: restricted(),
		templateState: restricted(),
		aiState: restricted(),
		dbState: restricted(),
		graphState: restricted(),
		queryState: restricted(),
		eventState,
		userState: restricted<IBackendState["userState"]>({
			getProfile: async () => ({
				name: "Service interface",
				bits: [],
				hub: window.location.origin,
				created: new Date().toISOString(),
				updated: new Date().toISOString(),
			}),
		}),
		helperState: restricted<IBackendState["helperState"]>({
			fileToUrl: async (file: File) => {
				if (file.size > 5 * 1024 * 1024)
					throw new Error("Attachments must be smaller than 5 MB.");
				return new Promise<string>((resolve, reject) => {
					const reader = new FileReader();
					reader.onload = () => resolve(String(reader.result));
					reader.onerror = () =>
						reject(new Error("The attachment could not be read."));
					reader.readAsDataURL(file);
				});
			},
		}),
		pageState: restricted<IBackendState["pageState"]>({
			getPageBootstrap: async (app, route, eventId) => {
				const event = eventId
					? eventFor(app, eventId)
					: inventory.events.find((event) => event.route === route);
				if (!event) throw new Error("This route is not deployed here.");
				const page = await bootstrap(app, event.id);
				return {
					event: page.event,
					page: { ...page.page, noCache: true },
					executionRevision: page.execution_revision,
					revision: page.execution_revision,
					elementDemand: page.element_demand,
					routeMiss: false,
					canonicalRoute: page.route,
				};
			},
		}),
		widgetState: restricted(),
		registryState: restricted(),
		routeState: restricted<IBackendState["routeState"]>({
			getRoutes: async (app) => {
				assertApp(app);
				return inventory.events
					.filter((e) => e.default_page_id && e.route)
					.map((e) => ({ path: e.route as string, eventId: e.id }));
			},
		}),
		capabilities: () => ({
			needsSignIn: false,
			canExecuteLocally: false,
			canHostEmbeddings: false,
			canHostLlamaCPP: false,
			canHostMLX: false,
		}),
		isOffline: async () => false,
	};
	return { backend, bootstrap };
}
