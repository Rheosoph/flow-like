import type { IProfile } from "@flow-like/flow-like-ui";
import {
	type LessonAction,
	type UserCourseEnrollment,
	translateId,
} from "@flow-like/flow-like-ui/lib/learn/types";
import type { AuthContextProps } from "react-oidc-context";
import { learnApi } from "./learn-api";

export interface DispatcherContext {
	readonly router: { push: (url: string) => void };
	readonly profile?: IProfile | null;
	readonly auth?: AuthContextProps;
	readonly courseId?: string;
	readonly enrollment?: UserCourseEnrollment | null;
	readonly aliasForApp?: (appId: string) => string | null;
	readonly onLinked?: () => void;
}

async function resolveActionAppId(
	action: {
		readonly appId?: string | null;
		readonly appAlias?: string;
	},
	ctx: DispatcherContext,
): Promise<string | null> {
	if (action.appId) return action.appId;
	if (!action.appAlias || !ctx.profile || !ctx.auth || !ctx.courseId) {
		return null;
	}

	const res = await learnApi.openSharedApp(
		ctx.profile,
		ctx.auth,
		ctx.courseId,
		action.appAlias,
	);
	ctx.onLinked?.();
	return res.app_id;
}

/**
 * Phase 1 implementation: navigate via the Next.js router. Phase 4 will plug
 * write actions (ADD_NODE, CREATE_EVENT, OPEN_OR_CLONE_APP) into the board
 * state and event endpoints.
 */
export async function dispatchLessonAction(
	action: LessonAction,
	ctx: DispatcherContext,
): Promise<void> {
	switch (action.kind) {
		case "NAVIGATE": {
			const appId = await resolveActionAppId(action, ctx);
			if (!appId) return;
			const params = new URLSearchParams({ id: appId, ...(action.params ?? {}) });
			const subpath = action.subpath.startsWith("/") ? action.subpath : `/library/config/${action.subpath}`;
			ctx.router.push(`${subpath}?${params.toString()}`);
			return;
		}
		case "FOCUS_NODE": {
			const appId = await resolveActionAppId(action, ctx);
			if (!appId) return;
			const alias = action.appAlias ?? ctx.aliasForApp?.(appId) ?? null;
			const boardId =
				translateId(ctx.enrollment ?? null, alias, "boards", action.boardId) ??
				action.boardId;
			const nodeId =
				translateId(ctx.enrollment ?? null, alias, "nodes", action.nodeId) ??
				action.nodeId;
			const params = new URLSearchParams({
				id: boardId,
				app: appId,
				focus: nodeId,
			});
			ctx.router.push(`/flow?${params.toString()}`);
			return;
		}
		case "ADD_NODE": {
			const appId = await resolveActionAppId(action, ctx);
			if (!appId) return;
			const alias = action.appAlias ?? ctx.aliasForApp?.(appId) ?? null;
			const boardId =
				translateId(ctx.enrollment ?? null, alias, "boards", action.boardId) ??
				action.boardId;
			const params = new URLSearchParams({
				id: boardId,
				app: appId,
				addNode: action.nodeTypeId,
				...(action.coords
					? { coordsX: String(action.coords[0]), coordsY: String(action.coords[1]) }
					: {}),
			});
			ctx.router.push(`/flow?${params.toString()}`);
			return;
		}
		case "CREATE_EVENT": {
			const appId = await resolveActionAppId(action, ctx);
			if (!appId) return;
			const params = new URLSearchParams({
				id: appId,
				newEvent: encodeURIComponent(JSON.stringify(action.template)),
			});
			ctx.router.push(`/library/config/events?${params.toString()}`);
			return;
		}
		case "OPEN_OR_CLONE_APP": {
			let appId = action.sharedAppId;
			if (ctx.profile && ctx.auth && ctx.courseId && action.alias) {
				try {
					const res = await learnApi.openSharedApp(
						ctx.profile,
						ctx.auth,
						ctx.courseId,
						action.alias,
					);
					appId = res.app_id;
					ctx.onLinked?.();
				} catch (err) {
					console.error("openSharedApp failed", err);
				}
			}
			if (!appId) return;
			const params = new URLSearchParams({ id: appId });
			ctx.router.push(`/use?${params.toString()}`);
			return;
		}
	}
}
