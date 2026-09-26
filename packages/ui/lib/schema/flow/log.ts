export interface ILog {
	end: ISystemTime;
	/** Groups repeats of one message; absent on runs recorded before fingerprints. */
	fingerprint?: null | string;
	log_level: ILogLevel;
	message: string;
	node_id?: null | string;
	operation_id?: null | string;
	start: ISystemTime;
	stats?: null | ILogStat;
	[property: string]: any;
}

export interface ISystemTime {
	nanos_since_epoch: number;
	secs_since_epoch: number;
	[property: string]: any;
}

export enum ILogLevel {
	Debug = "Debug",
	Error = "Error",
	Fatal = "Fatal",
	Info = "Info",
	Warn = "Warn",
}

export interface ILogStat {
	bit_ids?: string[] | null;
	token_in?: number | null;
	token_out?: number | null;
	[property: string]: any;
}
