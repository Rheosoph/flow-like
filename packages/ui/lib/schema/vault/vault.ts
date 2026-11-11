export interface IVault {
	author: string;
	bits: string[];
	boards: string[];
	created_at: ISystemTime;
	description: string;
	id: string;
	name: string;
	tags: string[];
	updated_at: ISystemTime;
	[property: string]: any;
}

export interface ISystemTime {
	nanos_since_epoch: number;
	secs_since_epoch: number;
	[property: string]: any;
}
