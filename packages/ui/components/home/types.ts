export interface IHomeWidget {
	id: string;
	type: string;
	title?: string;
	description?: string;
	size: {
		columns: number;
		rows: number;
		heightMode?: "auto" | "content" | "fixed";
		height?: number;
	};
	appearance: { variant: string; accent: string };
	config: Record<string, unknown>;
}

export interface IHomeLayout {
	version: 1;
	title?: string;
	description?: string;
	widgets: IHomeWidget[];
}

export interface IHomeDefault {
	id: string;
	layout: IHomeLayout;
	revision: string;
}

export interface IHomeDefaults {
	main: IHomeDefault | null;
	profile: IHomeDefault | null;
}

export type HomeWidgetCategory =
	| "assistant"
	| "apps"
	| "data"
	| "content"
	| "activity";

export interface HomeWidgetPreset {
	id: string;
	type: string;
	name: string;
	description: string;
	category: HomeWidgetCategory;
	icon: string;
	columns: number;
	rows: number;
	variant?: string;
	accent?: string;
	config: Record<string, unknown>;
}
