/**
 * One summary row per run, kept in the host's [`RunIndex`]. `payload` is the
 * recorded event input; listings return it empty and readers fetch the
 * sidecar written by [`run_index::write_run_payload`].
 */
export interface ILogMetadata {
    app_id:   string;
    board_id: string;
    end:      number;
    /**
     * Why the run failed when its logs cannot say: a rejected trigger, a
     * crashed executor, a run the sweeper timed out.
     */
    error_message?: null | string;
    event_id:       string;
    event_version?: null | string;
    /**
     * Runtime-only field - not stored, set based on fetch source
     */
    is_remote?: boolean;
    log_level:  number;
    logs?:      number | null;
    node_id:    string;
    nodes?:     Array<any[]> | null;
    payload:    number[];
    run_id:     string;
    start:      number;
    version:    string;
    [property: string]: any;
}
