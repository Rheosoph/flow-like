import type { IHomeLayout } from "../../../components/home/types";
import type { IBit } from "../bit/bit";

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
	// biome-ignore lint/suspicious/noExplicitAny: the hub stores any JSON here (`Option<Value>`), and the settings page and theme loaders read it as an ITheme without narrowing; typing it means narrowing at those call sites.
	theme?: any;
	thumbnail?: null | string;
	updated: string;
	[property: string]: unknown;
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
}

export interface IProfileApp {
	app_id: string;
	favorite: boolean;
	favorite_order?: number | null;
	pinned: boolean;
	pinned_order?: number | null;
	[property: string]: unknown;
}

export interface ISettings {
	connection_mode: IConnectionMode;
	[property: string]: unknown;
}

export enum IConnectionMode {
	Default = "default",
	Simplebezier = "simplebezier",
	Smoothstep = "smoothstep",
	Step = "step",
	Straight = "straight",
}
