export interface IRemoveComment {
    comment: IComment;
    [property: string]: any;
}

export interface IComment {
    author?:      null | string;
    color?:       null | string;
    comment_type: ICommentType;
    content:      string;
    coordinates:  any[];
    hash?:        number | null;
    height?:      number | null;
    id:           string;
    is_locked?:   boolean | null;
    layer?:       null | string;
    /**
     * Soft reference to a board node this comment is attached to (e.g. the statement a
     * FlowScript thread anchors on). Presentation metadata only — dangling references are
     * legal (the node may be deleted later); consumers must treat a missing node as unanchored.
     */
    node_id?:  null | string;
    timestamp: ITimestamp;
    width?:    number | null;
    z_index?:  number | null;
    [property: string]: any;
}

export enum ICommentType {
    Image = "Image",
    Text = "Text",
    Video = "Video",
}

export interface ITimestamp {
    nanos_since_epoch: number;
    secs_since_epoch:  number;
    [property: string]: any;
}
