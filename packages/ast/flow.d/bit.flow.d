// Bit — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Bit ===

/**
 * Loads a Bit from a string ID
 * @param bitId — Input String
 * @returns outputBit — Output Bit
 * @impure has side effects / drives control flow
 */
declare function bitFromString({ bitId: string }): Struct;

/**
 * Checks if the Bit is of the specified type and branches the execution flow accordingly.
 * @param bit — Input Bit
 * @param bitType — Type to check (e.g., "Llm", "Vlm")
 * @returns bitOut — Output Bit
 * @impure has side effects / drives control flow
 */
declare function isBitOfType({ bit: Struct, bitType: string }): Struct;

/**
 * Routes execution based on the type of the Bit
 * @param bit — Input Bit
 * @returns bitOut — Output Bit
 * @impure has side effects / drives control flow
 */
declare function switchOnBit({ bit: Struct }): Struct;

