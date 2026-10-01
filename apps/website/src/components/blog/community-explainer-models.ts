export interface FeedEntry {
	id: string;
	title: string;
	category: string;
	date: string | null;
	format: string;
}

export const exampleFeed: FeedEntry[] = [
	{
		id: "release",
		title: "A database release to review",
		category: "Engineering",
		date: "2026-09-28",
		format: "RSS",
	},
	{
		id: "research",
		title: "Research notes on retrieval",
		category: "Research",
		date: "2026-09-30",
		format: "Atom",
	},
	{
		id: "release",
		title: "A database release to review",
		category: "Engineering",
		date: "2026-09-28",
		format: "JSON Feed",
	},
	{
		id: "operations",
		title: "Keeping a service available",
		category: "Engineering",
		date: "2026-09-29",
		format: "Atom",
	},
	{
		id: "undated",
		title: "A research reading list",
		category: "Research",
		date: null,
		format: "RSS",
	},
];

export function selectFeed(
	entries: FeedEntry[],
	query: string,
	category: string,
	limit: number,
) {
	const seen = new Set<string>();
	return entries
		.filter((entry) =>
			entry.title.toLowerCase().includes(query.trim().toLowerCase()),
		)
		.filter((entry) => !category || entry.category === category)
		.filter((entry) => {
			if (seen.has(entry.id)) return false;
			seen.add(entry.id);
			return true;
		})
		.sort((a, b) => (b.date ?? "").localeCompare(a.date ?? ""))
		.slice(0, limit);
}

export interface CacheState {
	clock: number;
	entries: Record<string, { value: string; expires: number }>;
	computations: number;
	hits: number;
	last: string;
}

export const emptyCache: CacheState = {
	clock: 0,
	entries: {},
	computations: 0,
	hits: 0,
	last: "Choose a source version and run the example.",
};

export function runCache(state: CacheState, version: string): CacheState {
	const key = `example-team:handbook:${version}`;
	const cached = state.entries[key];
	if (cached && cached.expires > state.clock) {
		return {
			...state,
			hits: state.hits + 1,
			last: `Hit: return the stored summary for ${version}. The compute branch stays closed.`,
		};
	}
	return {
		...state,
		computations: state.computations + 1,
		entries: {
			...state.entries,
			[key]: { value: `Summary of ${version}`, expires: state.clock + 1 },
		},
		last: `Miss: compute a summary for ${version}, then store it for one simulated hour.`,
	};
}

export type ChannelStatus =
	| "idle"
	| "waiting"
	| "approved"
	| "declined"
	| "timed-out";
export interface ChannelState {
	request: number;
	status: ChannelStatus;
	log: string[];
}
export const emptyChannel: ChannelState = {
	request: 0,
	status: "idle",
	log: [],
};
export type ChannelAction =
	| { type: "start" }
	| { type: "timeout" }
	| { type: "reply"; request: number; approved: boolean };

export function channelTransition(
	state: ChannelState,
	action: ChannelAction,
): ChannelState {
	if (action.type === "start") {
		if (state.status === "waiting") return state;
		const request = state.request + 1;
		return {
			request,
			status: "waiting",
			log: [`Request ${request}: waiting for a matching browser response.`],
		};
	}
	if (action.type === "timeout") {
		if (state.status !== "waiting") return state;
		return {
			...state,
			status: "timed-out",
			log: [...state.log, "Wait ended. Take the timeout path."],
		};
	}
	if (state.status !== "waiting" || action.request !== state.request) {
		return {
			...state,
			log: [
				...state.log,
				`Ignored response for request ${action.request}; no matching open wait.`,
			].slice(-5),
		};
	}
	const status = action.approved ? "approved" : "declined";
	return {
		...state,
		status,
		log: [
			...state.log,
			`Request ${state.request}: ${status}. Continue through that branch.`,
		],
	};
}
