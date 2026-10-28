import type { IBit } from "../bit/bit";
import type { IHomeLayout } from "../../../components/home/types";

export interface IProfile {
	apps?: IProfileApp[] | null;
	bits: string[];
	created: string;
	/**
	 * User-owned private bits (custom providers / local HF models). Only the
	 * owner's desktop and the hub hold their provider secrets. Server runs carry
	 * secret-free views and call them through the hub's model relay; the browser
	 * client never sees a secret.
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
	home_layout?: IHomeLayout | null;
	home_default_id?: string | null;
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
