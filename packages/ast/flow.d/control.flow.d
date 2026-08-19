// Control — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Control ===

/**
 * Branches the flow based on a condition
 * @param condition (optional) — The condition to evaluate
 * @impure has side effects / drives control flow
 */
declare function controlBranch({ condition?: bool }): void;

/**
 * Loops over an Array
 * @param array — Array to Loop
 * @returns value — The current item Value
 * @returns index — Current Array Index
 * @impure has side effects / drives control flow
 */
declare function controlForEach({ array: any[] }): { value: any, index: int };

/**
 * Loops over an Array in batches, running the body once per slice of up to Batch Size elements
 * @param array — Array to Loop
 * @param batchSize (optional) — Maximum number of elements per batch. Values below 1 are clamped to 1.
 * @returns batch — The current slice, holding up to Batch Size elements
 * @returns index — Zero based index of the current batch
 * @returns startIndex — Index of the first element of this batch inside the source array
 * @impure has side effects / drives control flow
 */
declare function controlForEachBatch({ array: any[], batchSize?: int }): { batch: any[], index: int, startIndex: int };

/**
 * Loops over all rows of a table
 * @param table — CSV Table to loop
 * @returns value — Current row object
 * @returns index — Current row index (0-based)
 * @impure has side effects / drives control flow
 */
declare function controlForEachRow({ table: Struct }): { value: Struct, index: int };

/**
 * Loops over an Array; allows breaking early from inside the loop body.
 * @param break (optional) — Trigger this to terminate the active loop early (callable from inside Loop Body)
 * @param array — Array to Loop
 * @returns value — The current item Value
 * @returns index — Current Array Index
 * @impure has side effects / drives control flow
 */
declare function controlForEachWithBreak({ break?: bool, array: any[] }): { value: any, index: int };

/**
 * Parallel Execution
 * @param threadModel (optional) — Threads
 * @impure has side effects / drives control flow
 */
declare function controlParExecution({ threadModel?: string }): void;

/**
 * Loops over an Array in Parallel
 * @param array — Array to Loop
 * @param maxConcurrent (optional) — Maximum number of concurrent executions (0 = unlimited)
 * @returns value — The current item Value
 * @returns index — Current Array Index
 * @impure has side effects / drives control flow
 */
declare function controlParForEach({ array: any[], maxConcurrent?: int }): { value: any, index: int };

/**
 * Loops over an Array in batches, running the body for multiple batches in parallel
 * @param array — Array to Loop
 * @param batchSize (optional) — Maximum number of elements per batch. Values below 1 are clamped to 1.
 * @param maxConcurrent (optional) — Maximum number of concurrent body executions (0 = unlimited)
 * @returns batch — The current slice, holding up to Batch Size elements
 * @returns index — Zero based index of the current batch
 * @returns startIndex — Index of the first element of this batch inside the source array
 * @impure has side effects / drives control flow
 */
declare function controlParForEachBatch({ array: any[], batchSize?: int, maxConcurrent?: int }): { batch: any[], index: int, startIndex: int };

/**
 * Sequential Execution
 * @impure has side effects / drives control flow
 */
declare function controlSequence(): void;

/**
 * Executes with a timeout, branching based on completion
 * @param timeoutMs (optional) — Timeout duration in milliseconds
 * @impure has side effects / drives control flow
 */
declare function controlTimeout({ timeoutMs?: float }): void;

/**
 * Loop downstream execution in while loop
 * @param condition (optional) — Loop while this is true
 * @param maxIter (optional) — Maximum number of iterations
 * @returns iter — Current iteration index
 * @impure has side effects / drives control flow
 */
declare function controlWhileLoop({ condition?: bool, maxIter?: int }): int;

/**
 * Delays execution for a specified amount of time
 * @param time (optional) — Delay time in milliseconds
 * @impure has side effects / drives control flow
 */
declare function delay({ time?: float }): void;

/**
 * Control Flow Node
 * @param routeIn
 * @returns routeOut
 */
declare function reroute({ routeIn: any }): any;


// === Control/Call ===

/**
 * References a specific call in the flow
 * @param fnRef — The function reference to call
 * @impure has side effects / drives control flow
 */
declare function controlCallReference({ fnRef: string }): void;


// === Control/Flow ===

/**
 * Pass execution the first N triggers, then block; fire 'Completed' on Nth.
 * @param n (optional) — Number of times to allow execution to pass (>= 0)
 * @param startIndex (optional) — Initial index before first pass (commonly 0)
 * @returns index — Current counter after this trigger
 * @returns remaining — How many passes are left until Completed fires
 * @impure has side effects / drives control flow
 */
declare function controlDoN({ n?: int, startIndex?: int }): { index: int, remaining: int };

/**
 * Let execution pass once, then block until Reset.
 * @param startClosed (optional) — If true, starts blocked until a Reset arrives
 * @returns hasFired — Whether this node has already allowed a pass (blocked if true)
 * @impure has side effects / drives control flow
 */
declare function controlDoOnce({ startClosed?: bool }): bool;

/**
 * Alternate execution between A and B on successive triggers.
 * @param startOnA (optional) — If true, first pass goes to A; otherwise to B
 * @returns isA — Side that will fire on next trigger
 * @returns tick — How many times FlipFlop has executed
 * @impure has side effects / drives control flow
 */
declare function controlFlipFlop({ startOnA?: bool }): { isA: bool, tick: int };

/**
 * Open/close a gate to conditionally pass execution.
 * @param startClosed (optional) — If true, the gate starts closed (blocked)
 * @returns isOpen — Current open/closed state after this tick
 * @impure has side effects / drives control flow
 */
declare function controlGate({ startClosed?: bool }): bool;


// === Control/Functions ===

/**
 * Calls a function defined on this board
 * @param functionLayerId — The function to call
 */
declare function controlCallFunction({ functionLayerId: string }): void;


// === Control/Parallel ===

/**
 * Gather all execution states
 * @impure has side effects / drives control flow
 */
declare function controlGather(): void;

