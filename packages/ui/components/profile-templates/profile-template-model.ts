import { createId } from "@paralleldrive/cuid2";
import {
	IConnectionMode,
	type IProfile,
} from "../../lib/schema/profile/profile";

export function createProfileTemplate(hub = "", source?: IProfile): IProfile {
	const now = new Date().toISOString();
	return {
		...source,
		id: createId(),
		name: source ? `${source.name} copy`.slice(0, 120) : "",
		description: source?.description ?? null,
		icon: source?.icon ?? null,
		thumbnail: source?.thumbnail ?? null,
		hub: source?.hub ?? hub,
		hubs: [...(source?.hubs ?? [])],
		bits: [...(source?.bits ?? [])],
		apps: structuredClone(source?.apps ?? []),
		settings: structuredClone(
			source?.settings ?? { connection_mode: IConnectionMode.Simplebezier },
		),
		tags: [...(source?.tags ?? [])],
		interests: [...(source?.interests ?? [])],
		custom_bits: undefined,
		shortcuts: undefined,
		home_layout: null,
		home_default_id: null,
		created: now,
		updated: now,
	};
}

export function prepareProfileTemplate(profile: IProfile): IProfile {
	return {
		...profile,
		name: profile.name.trim(),
		description: profile.description?.trim() || null,
		hub: profile.hub?.trim() ?? "",
		updated: new Date().toISOString(),
	};
}

export function filterProfileTemplates(
	profiles: IProfile[],
	search: string,
	sort: string,
) {
	const needle = search.trim().toLocaleLowerCase();
	return profiles
		.filter((profile) =>
			[
				profile.name,
				profile.description,
				profile.id,
				profile.hub,
				...(profile.tags ?? []),
				...(profile.interests ?? []),
			].some((value) => value?.toLocaleLowerCase().includes(needle)),
		)
		.sort((a, b) =>
			sort === "name"
				? a.name.localeCompare(b.name)
				: (Date.parse(b.updated) || 0) - (Date.parse(a.updated) || 0),
		);
}
