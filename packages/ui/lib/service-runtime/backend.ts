import { z } from "zod";
import type { IBackendState } from "../../state/backend-state";
import type { IEventState } from "../../state/backend-state/event-state";
import type { IPage } from "../../state/backend-state/page-state";
import { IExecutionMode } from "../schema/flow/board";
import type { IElementDemand } from "../schema/flow/element-demand";
import { type IEvent, IEventExecutionMode } from "../schema/flow/event";
import type { PageTrigger } from "../schema/flow/page-trigger";
import {
	type ServiceRequest,
	type ServiceValueMapper,
	consumeServiceStream,
	readServiceJson,
} from "./transport";

/** Mirrors `BODY_LIMIT` in apps/standalone/src/hosting.rs. */
export const MAX_REQUEST_BYTES = 10 * 1024 * 1024;
/**
 * A non-image chat attachment travels base64-encoded (4/3 of its size) in the message and
 * again in its attachment list, so two copies plus the rest of the message must fit.
 */
export const MAX_ATTACHMENT_BYTES = Math.floor(
	((MAX_REQUEST_BYTES - 512 * 1024) * 3) / 8,
);
const megabytes = (bytes: number) =>
	`${Math.floor((bytes / (1024 * 1024)) * 10) / 10} MB`;

export function serializeServiceRequest(body: unknown): string {
	const serialized = JSON.stringify(body);
	if (new TextEncoder().encode(serialized).length > MAX_REQUEST_BYTES)
		throw new Error(
			`This request exceeds the service's ${megabytes(MAX_REQUEST_BYTES)} limit. Remove an attachment or start a new chat, then try again.`,
		);
	return serialized;
}

export interface Inventory {
	project_id: string;
	events: IEvent[];
}

const identifier = z
	.string()
	.regex(/^[A-Za-z0-9_.-]{1,128}$/)
	.refine((id) => id !== "." && id !== "..");
const version = z.tuple([
	z.number().int().min(0).max(0xffffffff),
	z.number().int().min(0).max(0xffffffff),
	z.number().int().min(0).max(0xffffffff),
]);
const eventSchema = z
	.object({
		id: identifier,
		name: z.string().max(4096),
		event_type: z.string().min(1).max(128),
		board_id: identifier,
		node_id: z.string().max(128),
		event_version: version,
		board_version: version.nullish(),
		default_page_id: identifier.nullish(),
		route: z
			.union([
				z.string().max(4096),
				z.object({ method: z.string().max(16), path: z.string().max(4096) }),
			])
			.nullish(),
		config: z
			.array(z.number().int().min(0).max(255))
			.max(MAX_REQUEST_BYTES)
			.optional(),
		inputs: z.array(z.record(z.unknown())).max(1024).optional(),
	})
	.passthrough()
	.transform((event) =>
		typeof event.route === "object" && event.route !== null
			? { ...event, service_route: event.route, route: null }
			: event,
	);

const inventorySchema = z
	.object({
		project_id: identifier,
		events: z.array(eventSchema).max(1024),
	})
	.refine(
		(inventory) =>
			new Set(inventory.events.map((event) => event.id)).size ===
			inventory.events.length,
	);

/** Validate service identity before placing remote content in a renderer's cache. */
export function parseServiceInventory(value: unknown): Inventory {
	const parsed = inventorySchema.safeParse(value);
	if (!parsed.success)
		throw new Error("The service returned an invalid inventory.");
	return parsed.data as unknown as Inventory;
}

export async function readServiceInventory(
	request: ServiceRequest,
	signal: AbortSignal,
): Promise<Inventory> {
	return parseServiceInventory(
		await readServiceJson(await request("/services", { signal }), signal),
	);
}

const PERSON_STARTED = new Set(["quick_action", "generic_form"]);

/** A quick action or form without a Page runs at `POST /run/{id}`. */
export function isPersonStarted(event: IEvent): boolean {
	return !event.default_page_id && PERSON_STARTED.has(event.event_type);
}

/** The route and body of a run that is not a Page action: a chat message, or a form's fields. */
function directRun(
	event: IEvent,
	payload: unknown,
	trigger: unknown,
): { path: string; body: unknown } {
	const id = encodeURIComponent(event.id);
	if (!trigger && isPersonStarted(event))
		return { path: `/run/${id}`, body: payload ?? {} };
	if (event.event_type !== "simple_chat" || trigger)
		throw new Error("This event cannot run through this interface.");
	return { path: `/chat/${id}`, body: payload };
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

const bootstrapSchema = z
	.object({
		project_id: identifier,
		event_id: identifier,
		event: eventSchema,
		page: z
			.object({ id: identifier, components: z.array(z.unknown()).max(100_000) })
			.passthrough(),
		execution_revision: z.string().min(1).max(4096),
		element_demand: z
			.object({
				selectors: z.array(z.string().max(4096)).max(100_000),
				dynamic: z.boolean(),
			})
			.passthrough(),
		route: z.string().max(4096).nullish(),
	})
	.passthrough();

export interface ServiceBackendOptions {
	/** A unique renderer/cache identity; device requests remain bound to project_id. */
	visibleAppId?: string;
	mapValue?: ServiceValueMapper;
	resolveResource?: (path: string, signal: AbortSignal) => Promise<string>;
}

const sameVersion = (
	left: number[] | null | undefined,
	right: number[] | null | undefined,
) =>
	left == null
		? right == null
		: right != null &&
			left.length === right.length &&
			left.every((part, index) => part === right[index]);

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
	sourceInventory: Inventory,
	request: ServiceRequest,
	signal: AbortSignal,
	options: ServiceBackendOptions = {},
) {
	const inventory = parseServiceInventory(sourceInventory);
	const visibleAppId = options.visibleAppId ?? inventory.project_id;
	const runs = new Map<string, AbortController>();
	const assertApp = (id: string) => {
		signal.throwIfAborted();
		if (id !== visibleAppId)
			throw new Error("This request belongs to another project.");
	};
	const eventFor = (app: string, id: string, requestedVersion?: number[]) => {
		assertApp(app);
		const event = inventory.events.find((event) => event.id === id);
		if (!event) throw new Error("This event is not deployed here.");
		if (requestedVersion && !sameVersion(event.event_version, requestedVersion))
			throw new Error("This event version is not deployed here.");
		return event;
	};
	const bootstrap = async (app: string, id: string): Promise<PageBootstrap> => {
		const event = eventFor(app, id);
		if (!event.default_page_id) throw new Error("This event has no Page.");
		const parsed = bootstrapSchema.safeParse(
			await readServiceJson(
				await request(`/pages/${encodeURIComponent(id)}/bootstrap`, { signal }),
				signal,
			),
		);
		if (!parsed.success)
			throw new Error("The service returned an invalid Page.");
		const data = parsed.data as unknown as PageBootstrap;
		if (
			data.project_id !== inventory.project_id ||
			data.event_id !== id ||
			data.event.id !== id ||
			data.event.board_id !== event.board_id ||
			data.event.node_id !== event.node_id ||
			data.event.event_type !== event.event_type ||
			!sameVersion(data.event.event_version, event.event_version) ||
			!sameVersion(data.event.board_version, event.board_version) ||
			data.event.default_page_id !== event.default_page_id ||
			data.page.id !== event.default_page_id ||
			!data.execution_revision
		)
			throw new Error("The Page does not match this deployment.");
		const mapped = options.mapValue
			? ((await options.mapValue(data, signal)) as PageBootstrap)
			: data;
		signal.throwIfAborted();
		return mapped;
	};
	const eventState = restricted<IEventState>({
		alwaysRemote: true,
		checkEventOAuth: undefined,
		checkOAuthRequirements: undefined,
		getEvent: async (app, id, version) => eventFor(app, id, version),
		getEventAuthoritative: async (app, id, version) =>
			eventFor(app, id, version),
		getEvents: async (app) => {
			assertApp(app);
			return inventory.events;
		},
		getEventsAuthoritative: async (app) => {
			assertApp(app);
			return inventory.events;
		},
		prerunEvent: async (app, id, version, trigger) => {
			const event = eventFor(app, id, version);
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
				({ path, body } = directRun(event, payload.payload, trigger));
			}
			const serialized = serializeServiceRequest(body);
			const controller = new AbortController();
			const abort = () => controller.abort();
			signal.addEventListener("abort", abort, { once: true });
			if (signal.aborted) controller.abort();
			let runId: string | undefined;
			try {
				beforeDispatch?.();
				const response = await request(path, {
					method: "POST",
					body: serialized,
					signal: controller.signal,
				});
				if (!response.body)
					throw new Error("The service returned no execution stream.");
				await consumeServiceStream(
					response.body,
					onEvents,
					(id) => {
						runId = id;
						runs.set(id, controller);
						onRunId?.(id);
					},
					{ signal: controller.signal, mapValue: options.mapValue },
				);
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
		storageState: restricted<IBackendState["storageState"]>({
			downloadStorageItems: async (app, paths) => {
				assertApp(app);
				if (!options.resolveResource || paths.length > 512)
					throw new Error(
						"These resources cannot be read through this service.",
					);
				const items = [];
				for (const path of paths) {
					const url = await options.resolveResource(path, signal);
					signal.throwIfAborted();
					items.push({ prefix: path, url });
				}
				return items;
			},
		}),
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
				hub: typeof window === "undefined" ? "" : window.location.origin,
				created: new Date().toISOString(),
				updated: new Date().toISOString(),
			}),
		}),
		helperState: restricted<IBackendState["helperState"]>({
			fileToTemporaryFile: undefined,
			filesToTemporaryFiles: undefined,
			fileToUrl: async (file: File) => {
				signal.throwIfAborted();
				if (file.size > MAX_ATTACHMENT_BYTES)
					throw new Error(
						`Attachments must be smaller than ${megabytes(MAX_ATTACHMENT_BYTES)}.`,
					);
				return new Promise<string>((resolve, reject) => {
					const reader = new FileReader();
					const cleanup = () => signal.removeEventListener("abort", abort);
					const abort = () => {
						reader.abort();
						cleanup();
						reject(signal.reason);
					};
					signal.addEventListener("abort", abort, { once: true });
					reader.onload = () => {
						cleanup();
						resolve(String(reader.result));
					};
					reader.onerror = () => {
						cleanup();
						reject(new Error("The attachment could not be read."));
					};
					reader.readAsDataURL(file);
				});
			},
		}),
		pageState: restricted<IBackendState["pageState"]>({
			getPageBootstrap: async (app, route, eventId) => {
				assertApp(app);
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
