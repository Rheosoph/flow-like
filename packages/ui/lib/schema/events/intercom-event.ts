export interface IIntercomEvent {
	event_id: string;
	event_type: string;
	payload: any;
	timestamp: ISystemTime;
	[property: string]: any;
}

export interface ISystemTime {
	nanos_since_epoch: number;
	secs_since_epoch: number;
	[property: string]: any;
}
