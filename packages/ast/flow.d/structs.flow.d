// Structs — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Structs ===

/**
 * Breaks a struct into its individual fields based on the schema
 * @param structIn — The struct to break apart
 */
declare function structBreak({ structIn: Struct }): void;

/**
 * Creates a new struct
 * @returns struct — Struct Output
 */
declare function structMake(): Struct;

/**
 * Creates a struct from individual fields based on a connected schema
 * @returns structOut — The constructed struct
 */
declare function structMakeFromSchema(): Struct;


// === Structs/Fields ===

/**
 * Fetches a field from a struct (supports dot notation and array access)
 * @param struct — Struct Output
 * @param field — Field path (e.g., 'message.content' or 'items[0].name')
 * @returns value — Value of the Struct
 * @returns found — Indicates if the value was found
 */
declare function structGet({ struct: Struct, field: string }): { value: any, found: bool };

/**
 * Fetches fields from a struct
 * @param struct — Struct Output
 * @returns fieldNames — Fields
 * @returns fields — Fields
 */
declare function structGetFields({ struct: Struct }): { fieldNames: string[], fields: any[] };

/**
 * Checks if a field exists in a struct (supports dot notation and array access)
 * @param struct — Struct Output
 * @param field — Field path (e.g., 'message.content' or 'items[0].name')
 * @returns found — Indicates if the value was found
 */
declare function structHas({ struct: Struct, field: string }): bool;

/**
 * Removes a field from a struct (supports dot notation and array access)
 * @param structIn — Struct In
 * @param field — Field path to remove (e.g., 'message.content' or 'items[0]')
 * @returns structOut — Struct Out
 * @returns removedValue — The value that was removed (null if field didn't exist)
 * @impure has side effects / drives control flow
 */
declare function structRemove({ structIn: Struct, field: string }): { structOut: Struct, removedValue: any };

/**
 * Sets a field in a struct (supports dot notation and array access)
 * @param structIn — Struct In
 * @param field — Field path (e.g., 'message.content' or 'items[0].name')
 * @param value — Value to set
 * @returns structOut — Struct Out
 * @impure has side effects / drives control flow
 */
declare function structSet({ structIn: Struct, field: string, value: any }): Struct;

