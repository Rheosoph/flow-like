// Bit — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace ai {
    // === Bit ===

    /**
     * Checks if the Bit is of the specified type and branches the execution flow accordingly.
     * @node is_bit_of_type @alias isBitOfType
     * @param bit — Input Bit
     * @param bitType — Type to check (e.g., "Llm", "Vlm")
     * @returns bitOut — Output Bit
     * @impure has side effects / drives control flow
     */
    function isBitOfType({ bit: Struct, bitType: string }): Struct;

    /**
     * Loads a Bit from a string ID
     * @node bit_from_string @alias bitFromString
     * @param bitId — Input String
     * @returns outputBit — Output Bit
     * @impure has side effects / drives control flow
     */
    function loadBit({ bitId: string }): Struct;

    /**
     * Routes execution based on the type of the Bit
     * @node switch_on_bit @alias switchOnBit
     * @param bit — Input Bit
     * @returns bitOut — Output Bit
     * @impure has side effects / drives control flow
     */
    function switchOnBit({ bit: Struct }): Struct;
}
