export interface ILogMetadata {
	app_id: string;
	board_id: string;
	end: number;
	event_id: string;
	event_version?: null | string;
	log_level: number;
	logs?: number | null;
	node_id: string;
	nodes?: Array<Array<number | string>> | null;
	payload: number[];
	run_id: string;
	start: number;
	version: string;
	error_message?: string | null;
	is_remote?: boolean;
	/** Set when a bot, not a person, started the run. */
	actor?: { kind: "bot"; id: string; name?: string | null } | null;
	[property: string]: any;
}
