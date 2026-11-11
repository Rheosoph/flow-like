export interface IStorageItem {
    e_tag?:        null | string;
    is_dir:        boolean;
    last_modified: string;
    location:      string;
    size:          number;
    version?:      null | string;
    [property: string]: any;
}
