export interface IFileMetadata {
	e_tag?: null | string;
	last_modified: string;
	location: string;
	size: number;
	version?: null | string;
	[property: string]: any;
}
