export interface ILog {
    end: IEnd;
    /**
     * Groups repeats of one message ([`super::log_summary::fingerprint`]); set when stored.
     */
    fingerprint?:  null | string;
    log_level:     ILogLevelEnum;
    message:       string;
    node_id?:      null | string;
    operation_id?: null | string;
    start:         IEnd;
    stats?:        null | IStatsObject;
    [property: string]: any;
}

export interface IEnd {
    nanos_since_epoch: number;
    secs_since_epoch:  number;
    [property: string]: any;
}

export enum ILogLevelEnum {
    Debug = "Debug",
    Error = "Error",
    Fatal = "Fatal",
    Info = "Info",
    Warn = "Warn",
}

export interface IStatsObject {
    bit_ids?:   string[] | null;
    token_in?:  number | null;
    token_out?: number | null;
    [property: string]: any;
}
