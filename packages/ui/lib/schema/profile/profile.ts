import type { IBit } from "../bit/bit";

export interface IProfile {
	apps?: IProfileApp[] | null;
	bits: string[];
	created: string;
	/**
	 * User-owned private bits (custom providers / local HF models). Hydrated
	 * with provider secrets only server-side and on the owner's desktop —
	 * never in the browser client.
	 */
	custom_bits?: IBit[];
	description?: null | string;
	hub?: string;
	secure?: boolean;
	hubs?: string[];
	icon?: null | string;
	id?: string;
	interests?: string[];
	name: string;
	settings?: ISettings;
	shortcuts?: IProfileShortcut[] | null;
	tags?: string[];
	theme?: any;
	thumbnail?: null | string;
	updated: string;
	[property: string]: any;
}

export interface IProfileShortcut {
	id: string;
	profileId: string;
	label: string;
	path: string;
	appId?: string | null;
	icon?: string | null;
	order: number;
	createdAt: string;
	[property: string]: any;
}

export interface IProfileApp {
	app_id: string;
	favorite: boolean;
	favorite_order?: number | null;
	pinned: boolean;
	pinned_order?: number | null;
	[property: string]: any;
}

export interface ISettings {
	connection_mode: IConnectionMode;
	[property: string]: any;
}

export enum IConnectionMode {
	Default = "default",
	Simplebezier = "simplebezier",
	Smoothstep = "smoothstep",
	Step = "step",
	Straight = "straight",
}
