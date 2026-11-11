export interface IMoveNode {
    current_layer?:    null | string;
    from_coordinates?: any[] | null;
    node_id:           string;
    to_coordinates:    any[];
    [property: string]: any;
}
