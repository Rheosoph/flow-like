// Control — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace control {
    // === Control ===

    /**
     * Branches the flow based on a condition
     * @node control_branch @alias controlBranch
     * @param condition (optional) — The condition to evaluate
     * @impure has side effects / drives control flow
     */
    function branch({ condition?: bool }): void;

    /**
     * Delays execution for a specified amount of time
     * @node delay @alias delay
     * @param time (optional) — Delay time in milliseconds
     * @impure has side effects / drives control flow
     */
    function delay({ time?: float }): void;

    /**
     * Loops over an Array
     * @node control_for_each @alias controlForEach
     * @param array — Array to Loop
     * @returns value — The current item Value
     * @returns index — Current Array Index
     * @impure has side effects / drives control flow
     */
    function forEach({ array: any[] }): { value: any, index: int };

    /**
     * Loops over an Array in batches, running the body once per slice of up to Batch Size elements
     * @node control_for_each_batch @alias controlForEachBatch
     * @param array — Array to Loop
     * @param batchSize (optional) — Maximum number of elements per batch. Values below 1 are clamped to 1.
     * @returns batch — The current slice, holding up to Batch Size elements
     * @returns index — Zero based index of the current batch
     * @returns startIndex — Index of the first element of this batch inside the source array
     * @impure has side effects / drives control flow
     */
    function forEachBatch({ array: any[], batchSize?: int }): { batch: any[], index: int, startIndex: int };

    /**
     * Loops over all rows of a table
     * @node control_for_each_row @alias controlForEachRow
     * @param table — CSV Table to loop
     * @returns value — Current row object
     * @returns index — Current row index (0-based)
     * @impure has side effects / drives control flow
     */
    function forEachRow({ table: Struct }): { value: Struct, index: int };

    /**
     * Loops over an Array; allows breaking early from inside the loop body.
     * @node control_for_each_with_break @alias controlForEachWithBreak
     * @param break (optional) — Trigger this to terminate the active loop early (callable from inside Loop Body)
     * @param array — Array to Loop
     * @returns value — The current item Value
     * @returns index — Current Array Index
     * @impure has side effects / drives control flow
     */
    function forEachWithBreak({ break?: bool, array: any[] }): { value: any, index: int };

    /**
     * Parallel Execution
     * @node control_par_execution @alias controlParExecution
     * @param threadModel (optional) — Threads
     * @impure has side effects / drives control flow
     */
    function parallel({ threadModel?: string }): void;

    /**
     * Loops over an Array in Parallel
     * @node control_par_for_each @alias controlParForEach
     * @param array — Array to Loop
     * @param maxConcurrent (optional) — Maximum number of concurrent executions (0 = unlimited)
     * @returns value — The current item Value
     * @returns index — Current Array Index
     * @impure has side effects / drives control flow
     */
    function parallelForEach({ array: any[], maxConcurrent?: int }): { value: any, index: int };

    /**
     * Loops over an Array in batches, running the body for multiple batches in parallel
     * @node control_par_for_each_batch @alias controlParForEachBatch
     * @param array — Array to Loop
     * @param batchSize (optional) — Maximum number of elements per batch. Values below 1 are clamped to 1.
     * @param maxConcurrent (optional) — Maximum number of concurrent body executions (0 = unlimited)
     * @returns batch — The current slice, holding up to Batch Size elements
     * @returns index — Zero based index of the current batch
     * @returns startIndex — Index of the first element of this batch inside the source array
     * @impure has side effects / drives control flow
     */
    function parallelForEachBatch({ array: any[], batchSize?: int, maxConcurrent?: int }): { batch: any[], index: int, startIndex: int };

    /**
     * Control Flow Node
     * @node reroute @alias reroute
     * @param routeIn
     * @returns routeOut
     */
    function reroute({ routeIn: any }): any;

    /**
     * Sequential Execution
     * @node control_sequence @alias controlSequence
     * @impure has side effects / drives control flow
     */
    function sequence(): void;

    /**
     * Executes with a timeout, branching based on completion
     * @node control_timeout @alias controlTimeout
     * @param timeoutMs (optional) — Timeout duration in milliseconds
     * @impure has side effects / drives control flow
     */
    function timeout({ timeoutMs?: float }): void;

    /**
     * Loop downstream execution in while loop
     * @node control_while_loop @alias controlWhileLoop
     * @param condition (optional) — Loop while this is true
     * @param maxIter (optional) — Maximum number of iterations
     * @returns iter — Current iteration index
     * @impure has side effects / drives control flow
     */
    function whileLoop({ condition?: bool, maxIter?: int }): int;

    // === Control/Call ===

    /**
     * References a specific call in the flow
     * @node control_call_reference @alias controlCallReference
     * @param fnRef — The function reference to call
     * @impure has side effects / drives control flow
     */
    function callReference({ fnRef: string }): void;

    // === Control/Flow ===

    /**
     * Pass execution the first N triggers, then block; fire 'Completed' on Nth.
     * @node control_do_n @alias controlDoN
     * @param n (optional) — Number of times to allow execution to pass (>= 0)
     * @param startIndex (optional) — Initial index before first pass (commonly 0)
     * @returns index — Current counter after this trigger
     * @returns remaining — How many passes are left until Completed fires
     * @impure has side effects / drives control flow
     */
    function doN({ n?: int, startIndex?: int }): { index: int, remaining: int };

    /**
     * Let execution pass once, then block until Reset.
     * @node control_do_once @alias controlDoOnce
     * @param startClosed (optional) — If true, starts blocked until a Reset arrives
     * @returns hasFired — Whether this node has already allowed a pass (blocked if true)
     * @impure has side effects / drives control flow
     */
    function doOnce({ startClosed?: bool }): bool;

    /**
     * Alternate execution between A and B on successive triggers.
     * @node control_flip_flop @alias controlFlipFlop
     * @param startOnA (optional) — If true, first pass goes to A; otherwise to B
     * @returns isA — Side that will fire on next trigger
     * @returns tick — How many times FlipFlop has executed
     * @impure has side effects / drives control flow
     */
    function flipFlop({ startOnA?: bool }): { isA: bool, tick: int };

    /**
     * Open/close a gate to conditionally pass execution.
     * @node control_gate @alias controlGate
     * @param startClosed (optional) — If true, the gate starts closed (blocked)
     * @returns isOpen — Current open/closed state after this tick
     * @impure has side effects / drives control flow
     */
    function gate({ startClosed?: bool }): bool;

    /**
     * Sends the flow down one branch per value. Wire a dropdown pin and the cases fill in by themselves, otherwise list them below
     * @node control_switch @alias controlSwitch
     * @param value — The value to switch on
     * @param cases (optional) — Comma separated list of values to branch on. Ignored while the wired pin declares its own values
     * @returns matchedCase — The case that was taken, empty when the default ran
     * @impure has side effects / drives control flow
     */
    function switch({ value: any, cases?: string }): string;

    // === Control/Functions ===

    /**
     * Calls a function defined on this board
     * @node control_call_function @alias controlCallFunction
     * @param functionLayerId — The function to call
     */
    function callFunction({ functionLayerId: string }): void;

    // === Control/Parallel ===

    /**
     * Gather all execution states
     * @node control_gather @alias controlGather
     * @impure has side effects / drives control flow
     */
    function gather(): void;
}
