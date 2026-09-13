import {
	NATIVE_WIDGETS_CHANGED,
	NATIVE_WIDGETS_REFRESH,
	NATIVE_WIDGETS_UPDATED,
	type NativeCustomWidget,
	type NativeWidgetDefinition,
	type NativeWidgetPageNode,
	nativeWidgetShell,
	readNativeWidgetDefinitions,
} from "@flow-like/flow-like-ui/lib/native-widget";
import { isNativeWidgetContent } from "@flow-like/flow-like-ui/lib/native-widget-content";
import {
	assertNativeWidgetActive,
	assertNativeWidgetApp,
	loadNativeWidgetChart,
} from "@flow-like/flow-like-ui/lib/native-widget-data";
import {
	NATIVE_WIDGET_PAGE_CAPTURE,
	type NativeWidgetPageCaptureDetail,
	type NativeWidgetPageProjection,
	loadNativeWidgetPage,
	resolveNativeWidgetPageMedia,
} from "@flow-like/flow-like-ui/lib/native-widget-page";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import { createNativeNotificationIconResolver } from "./native-notification-icons";

interface CachedWidget {
	captured?: boolean;
	revision: string;
	widget: NativeCustomWidget;
	pageId?: string;
	pageRevision?: string;
}
const cacheKey = (scope: string) =>
	`flow-like:native-widget-content:v1:${scope}`;
const bytes = (value: unknown) =>
	new TextEncoder().encode(JSON.stringify(value)).length;
const widgetText = (value: string, limit: number) => {
	const encoder = new TextEncoder();
	if (encoder.encode(value).length <= limit) return value;
	let result = "";
	for (const character of value) {
		if (encoder.encode(result + character).length > limit - 3) break;
		result += character;
	}
	return `${result.trimEnd()}…`;
};
const message = (error: unknown) =>
	error instanceof Error
		? widgetText(error.message, 1024)
		: "The widget could not be refreshed. Check your connection and access to its app.";

async function abortable<T>(
	operation: Promise<T>,
	signal: AbortSignal,
): Promise<T> {
	let cancel = () => {};
	const aborted = new Promise<never>((_, reject) => {
		cancel = () =>
			reject(new DOMException("Widget refresh cancelled", "AbortError"));
		signal.addEventListener("abort", cancel, { once: true });
		if (signal.aborted) cancel();
	});
	try {
		return await Promise.race([operation, aborted]);
	} finally {
		signal.removeEventListener("abort", cancel);
	}
}

export function readNativeCustomWidgetCache(scope: string): CachedWidget[] {
	try {
		const raw = localStorage.getItem(cacheKey(scope));
		if (!raw || raw.length > 1_048_576) return [];
		const values: unknown = JSON.parse(raw);
		if (!Array.isArray(values)) return [];
		const definitions = readNativeWidgetDefinitions(scope);
		return values
			.slice(0, 12)
			.filter((entry): entry is CachedWidget =>
				Boolean(
					entry &&
						typeof entry === "object" &&
						typeof entry.revision === "string" &&
						entry.widget &&
						typeof entry.widget.id === "string" &&
						["chart", "page"].includes(entry.widget.kind) &&
						Array.isArray(entry.widget.warnings ?? []) &&
						definitions.some(
							(definition) =>
								definition.updatedAt === entry.revision &&
								isNativeWidgetContent(entry.widget, definition),
						) &&
						bytes(entry) <= 131_072,
				),
			);
	} catch {
		return [];
	}
}

export async function materializeNativeWidgetPage(
	input: NativeWidgetPageProjection,
	signal?: AbortSignal,
	context?: { backend: IBackendState; appId: string },
): Promise<{ page?: NativeWidgetPageNode; warnings: string[] }> {
	assertNativeWidgetActive(signal);
	const projection = context
		? await resolveNativeWidgetPageMedia(
				context.backend.storageState,
				context.appId,
				input,
				signal,
			)
		: input;
	const page = projection.root ? structuredClone(projection.root) : undefined;
	const warnings = projection.warnings
		.slice(0, 8)
		.map((warning) => widgetText(warning, 256));
	if (!page || !projection.media.length) return { page, warnings };
	const resolver = createNativeNotificationIconResolver({ fallback: false });
	const cancel = () => resolver.dispose();
	signal?.addEventListener("abort", cancel, { once: true });
	try {
		if (signal?.aborted) cancel();
		const media = projection.media.slice(0, 4);
		const images = await resolver.resolve(
			"page",
			media.map(({ nodeId, source }) => ({ id: nodeId, icon: source })),
		);
		assertNativeWidgetActive(signal);
		let imageBytes = 0;
		let omitted = false;
		const visit = (node: NativeWidgetPageNode) => {
			if (images[node.id]) {
				const size = bytes(images[node.id]);
				if (imageBytes + size <= 65_536) {
					node.image = images[node.id];
					imageBytes += size;
				} else omitted = true;
			}
			for (const child of node.children ?? []) visit(child);
		};
		visit(page);
		if (omitted)
			warnings.push(
				"Some images were omitted to fit the widget. Choose a smaller container.",
			);
		if (media.some(({ nodeId }) => !images[nodeId]))
			warnings.push("Some images could not be loaded.");
		if (projection.media.length > 4)
			warnings.push(
				"Widgets show up to four images. Select a smaller container to include the important ones.",
			);
		return {
			page,
			warnings: warnings.slice(0, 8).map((warning) => widgetText(warning, 256)),
		};
	} finally {
		signal?.removeEventListener("abort", cancel);
		resolver.dispose();
	}
}

export async function previewNativeCustomWidget(
	backend: IBackendState,
	definition: NativeWidgetDefinition,
	options: {
		viewerId?: string;
		signal?: AbortSignal;
		cached?: CachedWidget;
	} = {},
): Promise<NativeCustomWidget> {
	const signal = options.signal ?? new AbortController().signal;
	if (definition.kind === "chart")
		return abortable(
			loadNativeWidgetChart(backend, definition, options),
			signal,
		);
	await abortable(
		assertNativeWidgetApp(
			backend,
			definition.appId,
			options.viewerId,
			options.signal,
		),
		signal,
	);
	const projection = await abortable(
		loadNativeWidgetPage(backend.pageState, definition),
		signal,
	);
	assertNativeWidgetActive(options.signal);
	if (
		options.cached?.captured &&
		options.cached.pageId === projection.pageId &&
		options.cached.pageRevision === projection.pageRevision &&
		isNativeWidgetContent(options.cached.widget, definition)
	)
		return options.cached.widget;
	const prepared = await materializeNativeWidgetPage(
		projection,
		options.signal,
		{ backend, appId: definition.appId },
	);
	return {
		...nativeWidgetShell(definition),
		...prepared,
		state: projection.supported ? "ready" : "unsupported",
		message: projection.requiresCapture
			? "Open this app page to load its current content."
			: projection.supported
				? undefined
				: "This page has no supported widget content. Choose another page or container.",
	};
}

/** Queries run in the signed-in app. The extension receives bounded display content only. */
export function createNativeCustomWidgetPublisher(options: {
	backend: IBackendState;
	scope: string;
	viewerId?: string;
	isCurrent: () => boolean;
	publish: (widgets: NativeCustomWidget[]) => Promise<void>;
}) {
	let stopped = false;
	let definitions = readNativeWidgetDefinitions(options.scope);
	const cache = new Map(
		readNativeCustomWidgetCache(options.scope).map((entry) => [
			entry.widget.id,
			entry,
		]),
	);
	const running = new Map<string, AbortController>();
	const lastChecked = new Map<string, { revision: string; at: number }>();
	let lastPublished = "";
	let timer: ReturnType<typeof setInterval> | undefined;
	const active = () => !stopped && options.isCurrent();
	const matching = (definition: NativeWidgetDefinition) => {
		const entry = cache.get(definition.id);
		return entry?.revision === definition.updatedAt &&
			entry.widget.kind === definition.kind &&
			entry.widget.appId === definition.appId &&
			Date.parse(entry.widget.expiresAt) > Date.now()
			? entry
			: undefined;
	};
	const publish = async () => {
		if (!active()) return;
		const entries = definitions.map(
			(definition) =>
				matching(definition) ?? {
					revision: definition.updatedAt,
					widget: {
						...nativeWidgetShell(definition),
						message: "Open Flow Like to refresh this widget.",
					},
				},
		);
		let size = 0;
		for (const entry of entries) {
			const definition = definitions.find(
				(value) => value.id === entry.widget.id,
			);
			if (entry.widget.state !== "ready") {
				entry.widget.page = undefined;
				entry.widget.chart = undefined;
			}
			if (
				bytes(entry) > 131_072 ||
				size + bytes(entry) > 1_048_576 ||
				(definition && !isNativeWidgetContent(entry.widget, definition))
			) {
				if (definition)
					entry.widget = {
						...nativeWidgetShell(definition),
						state: "error",
						message:
							"This widget contains too much content. Reduce its groups, images, or page container.",
					};
			}
			size += bytes(entry);
		}
		const encoded = JSON.stringify(entries);
		if (encoded === lastPublished) return;
		try {
			localStorage.setItem(cacheKey(options.scope), encoded);
		} catch {
			/* Native cache publication can still succeed when web storage is full. */
		}
		if (!active()) return;
		await options.publish(entries.map((entry) => entry.widget));
		if (!active()) return;
		lastPublished = encoded;
		window.dispatchEvent(
			new CustomEvent(NATIVE_WIDGETS_UPDATED, {
				detail: { scope: options.scope },
			}),
		);
	};
	const reload = () => {
		definitions = readNativeWidgetDefinitions(options.scope);
		for (const [id, controller] of running)
			if (!definitions.some((definition) => definition.id === id)) {
				controller.abort();
				running.delete(id);
			}
		for (const id of cache.keys())
			if (!definitions.some((definition) => definition.id === id))
				cache.delete(id);
	};
	const requestCurrent = (
		definition: NativeWidgetDefinition,
		controller: AbortController,
	) =>
		active() &&
		!controller.signal.aborted &&
		running.get(definition.id) === controller &&
		readNativeWidgetDefinitions(options.scope).some(
			(item) =>
				item.id === definition.id && item.updatedAt === definition.updatedAt,
		);
	const refreshOne = async (definition: NativeWidgetDefinition) => {
		if (
			!active() ||
			!readNativeWidgetDefinitions(options.scope).some(
				(item) =>
					item.id === definition.id && item.updatedAt === definition.updatedAt,
			)
		)
			return;
		const controller = new AbortController();
		running.get(definition.id)?.abort();
		running.set(definition.id, controller);
		const timeout = setTimeout(() => controller.abort(), 30_000);
		try {
			let entry: CachedWidget;
			if (definition.kind === "chart") {
				entry = {
					revision: definition.updatedAt,
					widget: await abortable(
						loadNativeWidgetChart(options.backend, definition, {
							viewerId: options.viewerId,
							signal: controller.signal,
						}),
						controller.signal,
					),
				};
			} else {
				await abortable(
					assertNativeWidgetApp(
						options.backend,
						definition.appId,
						options.viewerId,
						controller.signal,
					),
					controller.signal,
				);
				const projection = await abortable(
					loadNativeWidgetPage(options.backend.pageState, definition),
					controller.signal,
				);
				assertNativeWidgetActive(controller.signal);
				const previous = matching(definition);
				if (
					previous?.captured &&
					previous?.pageId === projection.pageId &&
					previous.pageRevision === projection.pageRevision &&
					previous.widget.page
				) {
					entry = previous;
				} else {
					const prepared = await materializeNativeWidgetPage(
						projection,
						controller.signal,
						{ backend: options.backend, appId: definition.appId },
					);
					entry = {
						revision: definition.updatedAt,
						pageId: projection.pageId,
						pageRevision: projection.pageRevision,
						widget: {
							...nativeWidgetShell(definition),
							...prepared,
							state: projection.supported ? "ready" : "unsupported",
							message: projection.requiresCapture
								? "Open this app page to load its current content."
								: projection.supported
									? undefined
									: "This page has no supported widget content. Choose another page or container.",
						},
					};
				}
			}
			if (requestCurrent(definition, controller)) {
				cache.set(definition.id, entry);
				lastChecked.set(definition.id, {
					revision: definition.updatedAt,
					at: Date.now(),
				});
				await publish();
			}
		} catch (error) {
			if (requestCurrent(definition, controller)) {
				cache.set(definition.id, {
					revision: definition.updatedAt,
					widget: {
						...nativeWidgetShell(definition),
						state: "error",
						message: message(error),
					},
				});
				await publish();
			}
		} finally {
			clearTimeout(timeout);
			if (running.get(definition.id) === controller)
				running.delete(definition.id);
		}
	};
	const refresh = async (force = false) => {
		if (!active() || document.visibilityState !== "visible") return;
		reload();
		await publish();
		if (navigator.onLine === false) return;
		const due = definitions.filter(
			(definition) =>
				!running.has(definition.id) &&
				(force ||
					!matching(definition) ||
					(Date.parse(matching(definition)?.widget.staleAt ?? "") <=
						Date.now() &&
						(lastChecked.get(definition.id)?.revision !==
							definition.updatedAt ||
							Date.now() - (lastChecked.get(definition.id)?.at ?? 0) >=
								definition.refreshMinutes * 60_000))),
		);
		for (let offset = 0; offset < due.length && active(); offset += 2)
			await Promise.allSettled(due.slice(offset, offset + 2).map(refreshOne));
	};
	const capture = async (detail: NativeWidgetPageCaptureDetail) => {
		if (
			!active() ||
			!detail ||
			detail.scope !== options.scope ||
			!Number.isFinite(detail.capturedAt) ||
			Math.abs(Date.now() - detail.capturedAt) > 60_000
		)
			return;
		reload();
		const definition = definitions.find(
			(item) =>
				item.id === detail.definitionId &&
				item.kind === "page" &&
				item.updatedAt === detail.revision,
		);
		if (!definition || definition.kind !== "page") return;
		const controller = new AbortController();
		running.get(definition.id)?.abort();
		running.set(definition.id, controller);
		const timeout = setTimeout(() => controller.abort(), 30_000);
		try {
			await abortable(
				assertNativeWidgetApp(
					options.backend,
					definition.appId,
					options.viewerId,
					controller.signal,
				),
				controller.signal,
			);
			const authenticated = await abortable(
				loadNativeWidgetPage(options.backend.pageState, definition),
				controller.signal,
			);
			if (
				authenticated.pageId !== detail.pageId ||
				authenticated.pageRevision !== detail.pageRevision
			) {
				if (requestCurrent(definition, controller)) {
					cache.set(definition.id, {
						revision: definition.updatedAt,
						widget: {
							...nativeWidgetShell(definition),
							state: "unavailable",
							message:
								"This page has changed. Open it in Flow Like to refresh the widget.",
						},
					});
					await publish();
				}
				return;
			}
			const prepared = await materializeNativeWidgetPage(
				detail.result,
				controller.signal,
				{ backend: options.backend, appId: definition.appId },
			);
			if (!requestCurrent(definition, controller)) return;
			lastChecked.set(definition.id, {
				revision: definition.updatedAt,
				at: Date.now(),
			});
			cache.set(definition.id, {
				captured: true,
				revision: definition.updatedAt,
				pageId: detail.pageId,
				pageRevision: detail.pageRevision,
				widget: {
					...nativeWidgetShell(definition, new Date(detail.capturedAt)),
					...prepared,
					state: detail.result.supported ? "ready" : "unsupported",
					message: detail.result.supported
						? undefined
						: "This page has no supported widget content.",
				},
			});
			await publish();
		} catch (error) {
			if (requestCurrent(definition, controller)) {
				cache.set(definition.id, {
					revision: definition.updatedAt,
					widget: {
						...nativeWidgetShell(definition),
						state: "error",
						message: message(error),
					},
				});
				await publish();
			}
		} finally {
			clearTimeout(timeout);
			if (running.get(definition.id) === controller)
				running.delete(definition.id);
		}
	};
	const onCapture = (event: Event) => {
		void capture((event as CustomEvent<NativeWidgetPageCaptureDetail>).detail);
	};
	const onChange = (event: Event) => {
		if (
			(event as CustomEvent<{ scope?: string }>).detail?.scope !== options.scope
		)
			return;
		for (const controller of running.values()) controller.abort();
		running.clear();
		void refresh(true);
	};
	const onFocus = () => {
		void refresh();
	};
	const onStorage = (event: StorageEvent) => {
		if (
			event.key === `flow-like:native-widget-definitions:v1:${options.scope}`
		) {
			for (const controller of running.values()) controller.abort();
			running.clear();
			void refresh(true);
		}
	};
	return {
		refresh,
		start() {
			timer = setInterval(onFocus, 60_000);
			window.addEventListener(NATIVE_WIDGETS_CHANGED, onChange);
			window.addEventListener(NATIVE_WIDGETS_REFRESH, onChange);
			window.addEventListener(NATIVE_WIDGET_PAGE_CAPTURE, onCapture);
			window.addEventListener("storage", onStorage);
			window.addEventListener("focus", onFocus);
			document.addEventListener("visibilitychange", onFocus);
			void refresh();
		},
		dispose() {
			stopped = true;
			clearInterval(timer);
			for (const controller of running.values()) controller.abort();
			running.clear();
			window.removeEventListener(NATIVE_WIDGETS_CHANGED, onChange);
			window.removeEventListener(NATIVE_WIDGETS_REFRESH, onChange);
			window.removeEventListener(NATIVE_WIDGET_PAGE_CAPTURE, onCapture);
			window.removeEventListener("storage", onStorage);
			window.removeEventListener("focus", onFocus);
			document.removeEventListener("visibilitychange", onFocus);
		},
	};
}
