export type OnlineProfile = {
	id: string;
	name: string;
	description?: string | null;
	icon?: string | null;
	thumbnail?: string | null;
	interests?: string[];
	tags?: string[];
	theme?: any;
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
