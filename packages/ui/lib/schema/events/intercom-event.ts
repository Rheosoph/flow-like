export interface IIntercomEvent {
    event_id:   string;
    event_type: string;
    payload:    any;
    timestamp:  ITimestamp;
    [property: string]: any;
}

export interface ITimestamp {
    nanos_since_epoch: number;
    secs_since_epoch:  number;
    [property: string]: any;
}
