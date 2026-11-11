export interface IAppSearchQuery {
	author?: null | string;
	category?: IAppCategory | null;
	id?: null | string;
	language?: null | string;
	limit?: number | null;
	offset?: number | null;
	query?: null | string;
	sort?: IAppSearchSort | null;
	tag?: null | string;
	[property: string]: any;
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

export enum IAppSearchSort {
	BestRated = "BestRated",
	LeastPopular = "LeastPopular",
	LeastRelevant = "LeastRelevant",
	MostPopular = "MostPopular",
	MostRelevant = "MostRelevant",
	NewestCreated = "NewestCreated",
	NewestUpdated = "NewestUpdated",
	OldestCreated = "OldestCreated",
	OldestUpdated = "OldestUpdated",
	WorstRated = "WorstRated",
}
