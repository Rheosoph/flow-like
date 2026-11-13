import { type IApp, IAppVisibility } from "../../../lib/schema/app/app";
import type { IMetadata } from "../../../lib/schema/bit/bit";

/** Code of the refusal an app that refuses bots answers with, from the hub or this device. */
export const APP_REFUSES_BOTS = "APP_REFUSES_BOTS";

export interface AppOption {
	id: string;
	name: string;
	/** Edit mode only: the owner's role in the app, or null once they left it. */
	role?: string | null;
	/** The app's owner refuses bots, so no bot may be given it. */
	refusesBots?: boolean;
}

/** The library's apps a bot can be given; an account bot needs a hub membership. */
export function libraryAppOptions(
	entries: readonly (readonly [IApp, IMetadata | undefined])[],
	localOnly: boolean,
): AppOption[] {
	return entries
		.filter(([app]) => localOnly || app.visibility !== IAppVisibility.Offline)
		.map(([app, metadata]) => ({
			id: app.id,
			name: metadata?.name ?? "",
			refusesBots: Boolean(app.refuse_bots),
		}));
}

/** An app that refuses bots can be taken away from a bot, never given to one. */
export function canToggleApp(app: AppOption, checked: boolean): boolean {
	return checked || !app.refusesBots;
}

/** Whether `error` is an app's refusal of bots, from the hub or from this device's bot store. */
export function refusesBots(error: unknown): boolean {
	return (
		typeof error === "object" &&
		error !== null &&
		(error as { code?: unknown }).code === APP_REFUSES_BOTS
	);
}
