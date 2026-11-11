export interface IMoveNode {
	current_layer?: null | string;
	from_coordinates?: number[] | null;
	node_id: string;
	to_coordinates: number[];
	[property: string]: any;
}
