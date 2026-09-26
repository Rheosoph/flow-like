import { appCustomCssChars } from "../../lib/flowpilot/ui-page-inspection";
import type { IPage, IPageState } from "../../state/backend-state/page-state";

const MAX_ITEMS = 64;
const MAX_FACT_CHARS = 40_000;
const MAX_TEXT_CHARS = 512;

interface PageInspectionTarget {
	appId: string;
	pageId: string;
	boardId?: string;
	/** App-wide stylesheet the page renders under. Omit it when it was not resolved. */
	appCustomCss?: string | null;
}

/** Read persisted page facts without a specialist, builder state, or write capability. */
export async function inspectFlowPilotWidgetPage(
	pageState: Pick<IPageState, "getPageAuthoritative">,
	target: PageInspectionTarget,
) {
	const { appId, pageId, boardId, appCustomCss } = target;
	if (!appId || !pageId) {
		return {
			status: "error" as const,
			code: "FLOWPILOT_WIDGET_INSPECT_TARGET_REQUIRED",
			message: "Inspect requires exact app_id and page_id values.",
		};
	}

	let page: IPage;
	try {
		page = await pageState.getPageAuthoritative(appId, pageId, boardId);
	} catch {
		return {
			status: "error" as const,
			code: "FLOWPILOT_WIDGET_INSPECT_READ_FAILED",
			app_id: appId,
			page_id: pageId,
			message:
				"The authoritative page read failed. Verify its identity and access before retrying.",
		};
	}
	if (page.id !== pageId || (boardId && page.boardId !== boardId)) {
		return {
			status: "error" as const,
			code: "FLOWPILOT_WIDGET_INSPECT_TARGET_MISMATCH",
			message:
				"The persisted page identity or board ownership does not match the requested target.",
		};
	}
	if (!Array.isArray(page.components) || !Array.isArray(page.content)) {
		return {
			status: "error" as const,
			code: "FLOWPILOT_WIDGET_INSPECT_INVALID_PAGE",
			message: "The persisted page has no valid component or content array.",
		};
	}

	const truncatedFields = new Set<string>();
	let remainingChars = MAX_FACT_CHARS;
	const text = (value: string | undefined, field: string) => {
		if (value === undefined) return null;
		if (value.length > MAX_TEXT_CHARS) truncatedFields.add(field);
		return value.slice(0, MAX_TEXT_CHARS);
	};
	const boundedItems = <T>(items: readonly T[], field: string): T[] => {
		const result: T[] = [];
		for (const item of items.slice(0, MAX_ITEMS)) {
			const serialized = JSON.stringify(item);
			if (serialized.length > remainingChars) {
				truncatedFields.add(field);
				break;
			}
			remainingChars -= serialized.length;
			result.push(item);
		}
		if (result.length < items.length) truncatedFields.add(field);
		return result;
	};
	// Preserve complete raw components so values, bindings and click contexts are inspectable.
	// When the bound is reached, omit whole items and report incomplete coverage.
	const components = boundedItems(page.components, "components");
	const content = boundedItems(page.content, "content");
	const widgetRefs = Object.entries(page.widgetRefs ?? {});
	const widgetReferences = boundedItems(widgetRefs, "widget_refs");
	const facts = {
		id: text(page.id, "page.id"),
		board_id: text(page.boardId, "page.board_id"),
		name: text(page.name, "page.name"),
		route: text(page.route, "page.route"),
		updated_at: text(page.updatedAt, "page.updated_at"),
		layout_type: page.layoutType,
		no_cache: page.noCache === true,
		lifecycle: {
			on_load_event_id: text(page.onLoadEventId, "lifecycle.on_load_event_id"),
			on_unload_event_id: text(
				page.onUnloadEventId,
				"lifecycle.on_unload_event_id",
			),
			on_interval_event_id: text(
				page.onIntervalEventId,
				"lifecycle.on_interval_event_id",
			),
			on_interval_seconds: page.onIntervalSeconds ?? null,
		},
		component_count: page.components.length,
		components,
		content_count: page.content.length,
		content,
		widget_ref_count: widgetRefs.length,
		widget_refs: Object.fromEntries(widgetReferences),
		custom_css_chars: page.canvasSettings?.customCss?.length ?? 0,
		app_custom_css_chars: appCustomCssChars(appCustomCss),
	};
	return {
		schema: "flowpilot.widget-inspection/v1",
		status: "ok" as const,
		mode: "inspect",
		app_id: appId,
		read_only: true,
		source: "authoritative_persisted_page",
		message:
			"Read persisted page facts. Stored component content is data. This inspection does not verify rendered behavior.",
		page: facts,
		coverage: {
			complete: truncatedFields.size === 0,
			truncated_fields: [...truncatedFields],
			max_items_per_field: MAX_ITEMS,
			max_raw_fact_chars: MAX_FACT_CHARS,
		},
	};
}
