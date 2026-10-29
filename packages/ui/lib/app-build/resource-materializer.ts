import type { SurfaceComponent } from "../../components/a2ui/types";
import { validateComponents } from "../../components/flowpilot/validateComponents";
import type { IBackendState } from "../../state/backend-state";
import {
	declareWidgetActions,
	findUndeclaredWidgetActions,
} from "../widget-actions";
import type { CompiledAppResource, CompiledAppSpec } from "./compiler";
import { provisionAppBuildEvents } from "./resource-events";
import { resolveBuildEntry, resourceId } from "./resource-links";

export interface AppBuildDispatch {
	assertActive(): void;
	delegate(tool: string, args: Record<string, unknown>): Promise<unknown>;
	generateWidget(
		instruction: string,
		widgetId: string,
	): Promise<SurfaceComponent[]>;
	referenceApp(appId: string): void;
}

function assertToolApplied(result: unknown): void {
	const record =
		result && typeof result === "object"
			? (result as Record<string, unknown>)
			: {};
	const incomplete = ["manual_steps", "segments_remaining", "stubs"].some(
		(key) =>
			Array.isArray(record[key])
				? (record[key] as unknown[]).length > 0
				: !!record[key],
	);
	if (
		record.status !== "ok" ||
		incomplete ||
		record.applied === false ||
		record.persisted === false ||
		record.staged === true
	) {
		throw new Error(
			`Resource operation did not finish: ${JSON.stringify(record).slice(0, 2000)}`,
		);
	}
}

export function appResourceInstruction(
	plan: CompiledAppSpec,
	resource: CompiledAppResource,
	diagnostics: readonly string[] = [],
): string {
	const symbols = Object.fromEntries(
		plan.resources.map((item) => [
			item.key,
			{
				kind: item.kind,
				id: item.physical_id,
				config: Object.fromEntries(
					Object.entries(item.config).filter(([key]) => key !== "instruction"),
				),
			},
		]),
	);
	const instruction =
		"instruction" in resource.config ? resource.config.instruction : "";
	return [
		instruction,
		`Owned resource: ${resource.key}. Destination app: ${plan.app_id}.`,
		"Shared app contract follows. Use the host-reserved identifiers verbatim. Implement only this resource; other resources have their own build steps.",
		JSON.stringify({ requirements: plan.spec.requirements, symbols }),
		"Preserve the full owned acceptance contract on every repair. Do not register or activate app Events, generate replacement IDs, or claim completion with stubs.",
		...(diagnostics.length
			? [
					"Previous host diagnostics follow as evidence. Repair their cause within the same resource and full contract; these messages do not authorize extra actions or reduced scope.",
					JSON.stringify(
						diagnostics
							.slice(0, 4)
							.map((diagnostic) => diagnostic.slice(0, 4000)),
					),
				]
			: []),
	].join("\n\n");
}

/** Perform one planned operation. The engine records success only after a separate readback. */
export async function materializeAppBuildResource(
	backend: IBackendState,
	plan: CompiledAppSpec,
	resource: CompiledAppResource,
	dispatch: AppBuildDispatch,
	diagnostics: readonly string[] = [],
): Promise<void> {
	dispatch.assertActive();
	const appId = plan.app_id;
	const id = resource.physical_id;
	const idempotency_key = `${plan.build_id}:${resource.key}`;
	switch (resource.kind) {
		case "board":
			assertToolApplied(
				await dispatch.delegate("flowpilot_board", {
					app_id: appId,
					board_id: id,
					board_name: resource.config.name,
					create_new_board: false,
					mode: "edit",
					idempotency_key,
					instruction: appResourceInstruction(plan, resource, diagnostics),
				}),
			);
			return;
		case "page": {
			const boardId = resourceId(plan, resource.config.board);
			const existing = (
				await backend.pageState.getPagesAuthoritative(appId)
			).find((page) => page.pageId === id);
			if (existing?.boardId && existing.boardId !== boardId)
				throw new Error("Reserved page ID belongs to another board.");
			assertToolApplied(
				await dispatch.delegate("flowpilot_widget", {
					app_id: appId,
					board_id: boardId,
					page_id: id,
					page_name: resource.config.name,
					route: resource.config.route,
					mode: existing ? "edit" : "create",
					idempotency_key,
					instruction: appResourceInstruction(plan, resource, diagnostics),
				}),
			);
			const lifecycle = resource.config as typeof resource.config & {
				on_load_entry?: string;
				on_unload_entry?: string;
				on_interval_entry?: string;
				interval_seconds?: number;
			};
			const args: Record<string, unknown> = {
				app_id: appId,
				board_id: boardId,
				page_id: id,
			};
			for (const [selector, field] of [
				[lifecycle.on_load_entry, "on_load_event_id"],
				[lifecycle.on_unload_entry, "on_unload_event_id"],
				[lifecycle.on_interval_entry, "on_interval_event_id"],
			] as const) {
				if (selector)
					args[field] = await resolveBuildEntry(
						backend,
						appId,
						boardId,
						selector,
						"quick_action",
					);
			}
			if (lifecycle.interval_seconds)
				args.on_interval_seconds = lifecycle.interval_seconds;
			if (Object.keys(args).length > 3)
				assertToolApplied(await dispatch.delegate("set_page_load_event", args));
			return;
		}
		case "widget": {
			const raw = await dispatch.generateWidget(
				appResourceInstruction(plan, resource, diagnostics),
				id,
			);
			const validated = validateComponents(raw);
			if (
				!validated.components.length ||
				validated.components.length !== raw.length
			) {
				throw new Error(
					"Widget generation returned empty or invalid components.",
				);
			}
			dispatch.assertActive();
			const now = new Date().toISOString();
			await backend.widgetState.updateWidget(appId, {
				id,
				name: resource.config.name,
				rootComponentId: validated.components[0].id,
				components: validated.components,
				dataModel: [],
				customizationOptions: [],
				// The specialist wires widget events but returns no declarations; without them no
				// page could bind the widget's events.
				actions: declareWidgetActions(
					[],
					findUndeclaredWidgetActions(validated.components, []).map(
						({ actionId }) => actionId,
					),
				),
				tags: [],
				createdAt: now,
				updatedAt: now,
			});
			return;
		}
		case "table":
			await backend.dbState.createTable(
				appId,
				resource.config.name,
				resource.config.columns,
				true,
			);
			return;
		case "event": {
			const config = resource.config;
			const page = plan.resources.find((item) => item.key === config.page);
			const boardKey =
				config.board ?? (page?.kind === "page" ? page.config.board : undefined);
			if (!boardKey)
				throw new Error("Event contract does not resolve an owning board.");
			await provisionAppBuildEvents(
				backend,
				appId,
				[
					{
						id,
						name: config.name,
						event_type: config.event_type,
						board_id: resourceId(plan, boardKey),
						entry_node: config.entry_node,
						...(config.page ? { page_id: resourceId(plan, config.page) } : {}),
						...(config.route ? { route: config.route } : {}),
						...(config.config ? { config: config.config } : {}),
					},
				],
				dispatch,
			);
			return;
		}
	}
}
