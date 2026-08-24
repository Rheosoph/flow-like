// Structs — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace struct {
    // === Structs ===

    /**
     * Breaks a struct into its individual fields based on the schema
     * @node struct_break @receiver struct_in @alias structBreak
     * @param structIn — The struct to break apart (receiver: `this` in `x.break(...)`)
     */
    function break(this: Struct, { structIn: Struct }): void;

    /**
     * Creates a new struct
     * @node struct_make @alias structMake
     * @returns struct — Struct Output
     */
    function make(): Struct;

    /**
     * Creates a struct from individual fields based on a connected schema
     * @node struct_make_from_schema @alias structMakeFromSchema
     * @returns structOut — The constructed struct
     */
    function makeFromSchema(): Struct;

    /**
     * Lays structs over each other, later ones winning. Useful for defaults plus overrides
     * @node struct_merge @receiver struct @alias structMerge
     * @param struct — Base struct (receiver: `this` in `x.merge(...)`)
     * @param struct — Laid over the base (receiver: `this` in `x.merge(...)`)
     * @param deep (optional) — Merge nested structs field by field instead of replacing them
     * @param skipNull (optional) — Ignore fields that are null in a later struct
     * @returns merged — The combined struct
     */
    function merge(this: Struct, { struct: Struct, struct: Struct, deep?: bool, skipNull?: bool }): Struct;

    // === Structs/Fields ===

    /**
     * Fetches a field from a struct (supports dot notation and array access)
     * @node struct_get @receiver struct @alias structGet
     * @param struct — Struct Output (receiver: `this` in `x.get(...)`)
     * @param field — Field selector (e.g., 'message.content' or 'items[0].name')
     * @returns value — Value of the Struct
     * @returns found — Indicates if the value was found
     */
    function get(this: Struct, { struct: Struct, field: string }): { value: any, found: bool };

    /**
     * Fetches fields from a struct
     * @node struct_get_fields @receiver struct @alias structGetFields
     * @param struct — Struct Output (receiver: `this` in `x.getFields(...)`)
     * @returns fieldNames — Fields
     * @returns fields — Fields
     */
    function getFields(this: Struct, { struct: Struct }): { fieldNames: string[], fields: any[] };

    /**
     * Checks if a field exists in a struct (supports dot notation and array access)
     * @node struct_has @receiver struct @alias structHas
     * @param struct — Struct Output (receiver: `this` in `x.has(...)`)
     * @param field — Field selector (e.g., 'message.content' or 'items[0].name')
     * @returns found — Indicates if the value was found
     */
    function has(this: Struct, { struct: Struct, field: string }): bool;

    /**
     * Keeps only the listed fields, dropping everything else. Use before logging or sending a struct on
     * @node struct_pick @receiver struct @alias structPick
     * @param struct — Input Struct (receiver: `this` in `x.pick(...)`)
     * @param fields — Top level field names to keep
     * @param mode (optional) — Keep only these fields, or drop them and keep the rest
     * @returns result — The projected struct
     */
    function pick(this: Struct, { struct: Struct, fields: string[], mode?: string }): Struct;

    /**
     * Removes a field from a struct (supports dot notation and array access)
     * @node struct_remove @receiver struct_in @alias structRemove
     * @param structIn — Struct In (receiver: `this` in `x.remove(...)`)
     * @param field — Field selector to remove (e.g., 'message.content' or 'items[0]')
     * @returns structOut — Struct Out
     * @returns removedValue — The value that was removed (null if field didn't exist)
     * @impure has side effects / drives control flow
     */
    function remove(this: Struct, { structIn: Struct, field: string }): { structOut: Struct, removedValue: any };

    /**
     * Sets a field in a struct (supports dot notation and array access)
     * @node struct_set @receiver struct_in @alias structSet
     * @param structIn — Struct In (receiver: `this` in `x.set(...)`)
     * @param field — Field selector (e.g., 'message.content' or 'items[0].name')
     * @param value — Value to set
     * @returns structOut — Struct Out
     * @impure has side effects / drives control flow
     */
    function set(this: Struct, { structIn: Struct, field: string, value: any }): Struct;
}
