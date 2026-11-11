export interface IApp {
    /**
     * Project-level opt-in for the Fork-an-app feature. Apps default to
     * `false`; the owner must explicitly allow forking before any other
     * permission check applies.
     */
    allow_forking?: boolean;
    /**
     * Owner-declared app type. `None` means unclassified.
     */
    app_type?:   IAppTypeEnum | null;
    authors:     string[];
    avg_rating?: number | null;
    bits:        string[];
    boards:      string[];
    /**
     * Set on a bot's workspace: the id of the bot it belongs to. IApp lists hide such apps.
     */
    bot_id?:        null | string;
    changelog?:     null | string;
    created_at:     ICreatedAt;
    download_count: number;
    events:         string[];
    execution_mode: IExecutionMode;
    /**
     * For forked apps: when the fork was created.
     */
    forked_at?: null | ICreatedAt;
    /**
     * For forked apps: the source app's id (for lineage / attribution).
     */
    forked_from?:       null | string;
    frontend?:          null | IFrontendObject;
    id:                 string;
    interactions_count: number;
    /**
     * WASM packages required by this app: package_id -> version
     */
    packages?:           { [key: string]: string };
    page_ids?:           string[];
    price?:              number | null;
    primary_category?:   IPrimaryCategoryEnum | null;
    rating_count:        number;
    rating_sum:          number;
    relevance_score?:    number | null;
    secondary_category?: IPrimaryCategoryEnum | null;
    status:              IStatus;
    templates:           string[];
    updated_at:          ICreatedAt;
    version?:            null | string;
    visibility:          IVisibility;
    widget_ids?:         string[];
    [property: string]: any;
}

/**
 * What kind of thing the app is, structurally — an agent, a pipeline, a form.
 * Orthogonal to [`AppCategory`], which says what the app is *about*. Left
 * unset until the owner classifies it; the UI derives a suggestion from the
 * app's contents in the meantime.
 */
export enum IAppTypeEnum {
    Agent = "Agent",
    Analytics = "Analytics",
    CustomInterface = "CustomInterface",
    DataFocus = "DataFocus",
    DataPipeline = "DataPipeline",
    Form = "Form",
}

export interface ICreatedAt {
    nanos_since_epoch: number;
    secs_since_epoch:  number;
    [property: string]: any;
}

export enum IExecutionMode {
    Any = "Any",
    Local = "Local",
    Remote = "Remote",
}

export interface IFrontendObject {
    /**
     * App-wide stylesheet. Injected once above every page surface and scoped
     * to the app root, so page-level CSS stays later in document order and
     * keeps winning ties.
     */
    custom_css?:   null | string;
    landing_page?: null | string;
    /**
     * The app publisher's legal notice (imprint), linked from hosted pages.
     */
    legal_notice_url?: null | string;
    /**
     * The app publisher's privacy policy, linked from hosted pages.
     */
    privacy_policy_url?: null | string;
    [property: string]: any;
}

export enum IPrimaryCategoryEnum {
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

export enum IStatus {
    Active = "Active",
    Archived = "Archived",
    Inactive = "Inactive",
}

export enum IVisibility {
    Offline = "Offline",
    Private = "Private",
    Prototype = "Prototype",
    Public = "Public",
    PublicRequestAccess = "PublicRequestAccess",
}
