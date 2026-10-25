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
	[property: string]: any;
}
