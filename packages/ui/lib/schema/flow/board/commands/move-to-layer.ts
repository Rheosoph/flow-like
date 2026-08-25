export interface IMoveToLayer {
	ids: string[];
	previous?: { [key: string]: null | string };
	target?: null | string;
	[property: string]: any;
}
