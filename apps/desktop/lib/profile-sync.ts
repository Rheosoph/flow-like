import type { IHomeLayout } from "@flow-like/flow-like-ui/components/home/types";
import type { IProfile } from "@flow-like/flow-like-ui";

export type OnlineProfile = {
	id: string;
	name: string;
	description?: string | null;
	icon?: string | null;
	thumbnail?: string | null;
	interests?: string[];
	tags?: string[];
	theme?: any;
	home_layout?: IHomeLayout | null;
	home_default_id?: string | null;
	bit_ids?: string[];
	apps?: any;
	shortcuts?: Array<{
		id: string;
		profileId: string;
		label: string;
		path: string;
		appId?: string;
		icon?: string;
		order: number;
		createdAt: string;
	}>;
	settings?: any;
	hub: string;
	hubs?: string[];
	created_at: string;
	updated_at: string;
	deleted_at?: string | null;
};

export const toLocalProfile = (onlineProfile: OnlineProfile) => ({
	hub_profile: {
		id: onlineProfile.id,
		name: onlineProfile.name,
		description: onlineProfile.description ?? null,
		icon: onlineProfile.icon ?? null,
		thumbnail: onlineProfile.thumbnail ?? null,
		interests: onlineProfile.interests ?? [],
		tags: onlineProfile.tags ?? [],
		theme: onlineProfile.theme ?? null,
		home_layout: onlineProfile.home_layout ?? null,
		home_default_id: onlineProfile.home_default_id ?? null,
		bits: onlineProfile.bit_ids ?? [],
		apps: onlineProfile.apps ?? [],
		shortcuts: onlineProfile.shortcuts ?? [],
		hub: onlineProfile.hub,
		hubs: onlineProfile.hubs ?? [],
		settings: onlineProfile.settings ?? {
			connection_mode: "simplebezier",
		},
		secure: true,
		created: onlineProfile.created_at,
		updated: onlineProfile.updated_at,
	},
	execution_settings: {
		gpu_mode: true,
		max_context_size: 32000,
	},
	updated: onlineProfile.updated_at,
	created: onlineProfile.created_at,
});

export const getDefaultApiBase = () => {
	const baseUrl = process.env.NEXT_PUBLIC_API_URL ?? "api.flow-like.com";
	const full = baseUrl.startsWith("http") ? baseUrl : `https://${baseUrl}`;
	return full.endsWith("/") ? full.slice(0, -1) : full;
};

export function mergeRemoteProfileMetadata(
	local: { hub_profile: IProfile; updated: string },
	remote: OnlineProfile,
	preserveMediaRevision = false,
) {
	Object.assign(local.hub_profile, {
		name: remote.name,
		description: remote.description ?? null,
		interests: remote.interests ?? [],
		tags: remote.tags ?? [],
		theme: remote.theme ?? null,
		home_layout: remote.home_layout ?? null,
		home_default_id: remote.home_default_id ?? null,
		bits: remote.bit_ids ?? [],
		apps: remote.apps ?? [],
		hub: remote.hub,
		hubs: remote.hubs ?? [],
		settings: remote.settings ?? local.hub_profile.settings,
	});
	// Pending image uploads belong to this local revision. Advancing it would
	// prevent the server from returning the upload on the next sync attempt.
	if (!preserveMediaRevision) {
		local.hub_profile.updated = remote.updated_at;
		local.updated = remote.updated_at;
	}
}
