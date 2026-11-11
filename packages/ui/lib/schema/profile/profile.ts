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
	/**
	 * Bots this profile leaves out of its crew, pickers and Home. A custom bot goes by its id,
	 * a built-in one by its system key, so the choice holds for its account and device rows.
	 * Hiding is a view filter: the bot keeps running and stays reachable through its chats.
	 */
	hidden_bots?: string[];
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
