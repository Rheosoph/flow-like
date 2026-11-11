export interface IStorageItem {
	e_tag?: null | string;
	last_modified: string;
	location: string;
	size: number;
	version?: null | string;
	is_dir?: boolean;
	[property: string]: any;
}
