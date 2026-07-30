export interface IApp {
	allow_forking?: boolean;
	authors: string[];
	avg_rating?: number | null;
	bits: string[];
	boards: string[];
	changelog?: null | string;
	created_at: ISystemTime;
	download_count: number;
	events: string[];
	execution_mode: IAppExecutionMode;
	forked_at?: ISystemTime | null;
	forked_from?: null | string;
	frontend?: null | IFrontendConfiguration;
	id: string;
	interactions_count: number;
	page_ids: string[];
	price?: number | null;
	primary_category?: IAppCategory | null;
	rating_count: number;
	rating_sum: number;
	relevance_score?: number | null;
	secondary_category?: IAppCategory | null;
	/** Owner-declared app type. Null/undefined means unclassified. */
	app_type?: IAppType | null;
	status: IAppStatus;
	templates: string[];
	updated_at: ISystemTime;
	version?: null | string;
	visibility: IAppVisibility;
	widget_ids: string[];
	[property: string]: any;
}

export interface ISystemTime {
	nanos_since_epoch: number;
	secs_since_epoch: number;
	[property: string]: any;
}

export enum IAppExecutionMode {
	Any = "Any",
	Local = "Local",
	Remote = "Remote",
}

export interface IFrontendConfiguration {
	landing_page?: null | string;
	[property: string]: any;
}

/**
 * What kind of thing the app is, structurally — orthogonal to
 * {@link IAppCategory}, which says what the app is *about*.
 */
export enum IAppType {
	Agent = "Agent",
	CustomInterface = "CustomInterface",
	DataFocus = "DataFocus",
	DataPipeline = "DataPipeline",
	Analytics = "Analytics",
	Form = "Form",
}

export enum IAppCategory {
	Anime = "Anime",
	Business = "Business",
	Communication = "Communication",
	Education = "Education",
	Entertainment = "Entertainment",
	Finance = "Finance",
	FoodAndDrink = "FoodAndDrink",
	Games = "Games",
	Health = "Health",
	Lifestyle = "Lifestyle",
	Music = "Music",
	News = "News",
	Other = "Other",
	Photography = "Photography",
	Productivity = "Productivity",
	Shopping = "Shopping",
	Social = "Social",
	Sports = "Sports",
	Travel = "Travel",
	Utilities = "Utilities",
	Weather = "Weather",
}

export enum IAppStatus {
	Active = "Active",
	Archived = "Archived",
	Inactive = "Inactive",
}

export enum IAppVisibility {
	Offline = "Offline",
	Private = "Private",
	Prototype = "Prototype",
	Public = "Public",
	PublicRequestAccess = "PublicRequestAccess",
}
