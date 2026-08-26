// std — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace array {
    // === Utils/Array ===

    /**
     * Splits an array into batches of a fixed size
     * @node array_chunk @receiver array_in @alias arrayChunk
     * @param arrayIn — Your Array (receiver: `this` in `x.chunk(...)`)
     * @param size (optional) — Elements per batch
     * @returns chunks — One entry per batch, each holding up to Size elements
     * @returns chunkCount — How many batches were produced
     */
    function chunk(this: any[], { arrayIn: any[], size?: int }): { chunks: any[], chunkCount: int };

    /**
     * Removes all elements from an array
     * @node array_clear @receiver array_in @alias arrayClear
     * @param arrayIn — Your Array (receiver: `this` in `x.clear(...)`)
     * @returns arrayOut — Empty Array
     * @impure has side effects / drives control flow
     */
    function clear(this: any[], { arrayIn: any[] }): any[];

    /**
     * Creates an array from individual elements. Add more input pins by connecting to the 'element' pins.
     * @node construct_array @alias constructArray
     * @param element — Element to include in the array
     * @param element — Element to include in the array
     * @returns arrayOut — The constructed array
     */
    function construct({ element: any, element: any }): any[];

    /**
     * Append an Array to another Array
     * @node array_extend @receiver array_in @alias arrayExtend
     * @param arrayIn — Your Array (receiver: `this` in `x.extend(...)`)
     * @param values — Value to push
     * @returns arrayOut — Adjusted Array
     * @impure has side effects / drives control flow
     */
    function extend(this: any[], { arrayIn: any[], values: any[] }): any[];

    /**
     * Keeps the elements whose key passes a comparison
     * @node array_filter_by @receiver array_in @alias arrayFilterBy
     * @param arrayIn — Your Array (receiver: `this` in `x.filterBy(...)`)
     * @param key (optional) — Field to read from each element, dot notation for nested fields (customer.address.city). Empty uses the element itself
     * @param operator (optional) — How the key is compared against the value
     * @param value (optional) — What to compare against. In List takes a comma separated list
     * @param compare (optional) — Comparator used by the ordering operators
     * @param ignoreCase (optional) — Compare text without regard to upper/lower case
     * @param invert (optional) — Keep the elements that do not pass instead
     * @returns arrayOut — The kept elements
     * @returns kept — How many elements passed
     * @returns removed — How many elements were dropped
     */
    function filterBy(this: any[], { arrayIn: any[], key?: string, operator?: string, value?: string, compare?: string, ignoreCase?: bool, invert?: bool }): { arrayOut: any[], kept: int, removed: int };

    /**
     * Removes a specific field from every struct in an array. Elements without the field are kept unchanged. Returns the filtered array and count of removed fields.
     * @node array_filter_field @receiver array_in @alias arrayFilterField
     * @param arrayIn — Array of structs to filter (receiver: `this` in `x.filterField(...)`)
     * @param field — Field name to remove from each struct
     * @returns arrayOut — Array with the field removed from each struct
     * @returns removedCount — Number of fields that were removed
     * @impure has side effects / drives control flow
     */
    function filterField(this: Struct[], { arrayIn: Struct[], field: string }): { arrayOut: Struct[], removedCount: int };

    /**
     * Removes multiple fields from every struct in an array. Elements without the fields are kept unchanged. Returns the filtered array and count of removed fields.
     * @node array_filter_fields @receiver array_in @alias arrayFilterFields
     * @param arrayIn — Array of structs to filter (receiver: `this` in `x.filterFields(...)`)
     * @param fields — Array of field names to remove from each struct
     * @returns arrayOut — Array with the fields removed from each struct
     * @returns removedCount — Total number of fields that were removed
     * @impure has side effects / drives control flow
     */
    function filterFields(this: Struct[], { arrayIn: Struct[], fields: string[] }): { arrayOut: Struct[], removedCount: int };

    /**
     * Pulls nested arrays up into a single array
     * @node array_flatten @receiver array_in @alias arrayFlatten
     * @param arrayIn — Your Array (receiver: `this` in `x.flatten(...)`)
     * @param depth (optional) — How many levels to flatten, -1 for all of them
     * @returns arrayOut — The flattened array
     */
    function flatten(this: any[], { arrayIn: any[], depth?: int }): any[];

    /**
     * Gets an element from an array by index
     * @node array_get @receiver array_in @alias arrayGet
     * @param arrayIn — Your Array (receiver: `this` in `x.get(...)`)
     * @param index — Index of the element to get
     * @returns element — Element at the specified index
     * @returns success — Was the get successful?
     */
    function get(this: any[], { arrayIn: any[], index: int }): { element: any, success: bool };

    /**
     * Groups elements that share the same key value
     * @node array_group_by @receiver array_in @alias arrayGroupBy
     * @param arrayIn — Your Array (receiver: `this` in `x.groupBy(...)`)
     * @param key (optional) — Field to read from each element, dot notation for nested fields (customer.address.city). Empty uses the element itself
     * @returns groups — One entry per distinct key, in first-seen order
     * @returns groupCount — How many distinct keys were found
     */
    function groupBy(this: any[], { arrayIn: any[], key?: string }): { groups: Struct[], groupCount: int };

    /**
     * Checks if an array includes a certain value
     * @node array_includes @receiver array_in @alias arrayIncludes
     * @param arrayIn — Your Array (receiver: `this` in `x.includes(...)`)
     * @param value — Value to search for
     * @returns includes — Does the array include the value?
     */
    function includes(this: any[], { arrayIn: any[], value: any }): bool;

    /**
     * Finds the index of an item in an array
     * @node array_find_item @receiver array_in @alias arrayFindItem
     * @param arrayIn — Your Array (receiver: `this` in `x.indexOf(...)`)
     * @param item — Item to find
     * @returns index — Index of the item (-1 if not found)
     * @returns found — Was the item found?
     * @impure has side effects / drives control flow
     */
    function indexOf(this: any[], { arrayIn: any[], item: any }): { index: int, found: bool };

    /**
     * Matches the elements of two arrays on a shared key, the way a database join does
     * @node array_join_by @receiver array_left @alias arrayJoinBy
     * @param arrayLeft — Left Array (receiver: `this` in `x.joinBy(...)`)
     * @param arrayRight — Right Array
     * @param keyLeft (optional) — Field on the left elements, dot notation for nested fields. Empty uses the element itself
     * @param keyRight (optional) — Field on the right elements. Empty reuses the left key
     * @param join (optional) — Inner keeps only matches, Left keeps every left element
     * @returns pairs — One entry per match, holding both sides
     * @returns matched — How many left elements found a partner
     */
    function joinBy(this: any[], { arrayLeft: any[], arrayRight: any[], keyLeft?: string, keyRight?: string, join?: string }): { pairs: Struct[], matched: int };

    /**
     * Gets the length of an array
     * @node array_length @receiver array @alias arrayLength
     * @param array — Input Array (receiver: `this` in `x.length(...)`)
     * @returns length — Length of the array
     */
    function length(this: any[], { array: any[] }): int;

    /**
     * Creates an empty array
     * @node make_array @alias makeArray
     * @returns arrayOut — The created array
     */
    function make(): any[];

    /**
     * The element with the largest key
     * @node array_max_by @receiver array_in @alias arrayMaxBy
     * @param arrayIn — Your Array (receiver: `this` in `x.maxBy(...)`)
     * @param compare (optional) — How the key values are ordered. Auto reads each value and falls back to text
     * @param nulls (optional) — Where elements without a key value end up
     * @returns element — The element with the largest key
     * @returns index — Position of the element in the array
     * @returns found — False when the array was empty
     */
    function maxBy(this: any[], { arrayIn: any[], compare?: string, nulls?: string }): { element: any, index: int, found: bool };

    /**
     * The element with the smallest key
     * @node array_min_by @receiver array_in @alias arrayMinBy
     * @param arrayIn — Your Array (receiver: `this` in `x.minBy(...)`)
     * @param compare (optional) — How the key values are ordered. Auto reads each value and falls back to text
     * @param nulls (optional) — Where elements without a key value end up
     * @returns element — The element with the smallest key
     * @returns index — Position of the element in the array
     * @returns found — False when the array was empty
     */
    function minBy(this: any[], { arrayIn: any[], compare?: string, nulls?: string }): { element: any, index: int, found: bool };

    /**
     * Reads one field out of every element
     * @node array_pluck @receiver array_in @alias arrayPluck
     * @param arrayIn — Your Array (receiver: `this` in `x.pluck(...)`)
     * @param key (optional) — Field to read from each element, dot notation for nested fields (customer.address.city). Empty uses the element itself
     * @param skipMissing (optional) — Drop elements that do not have the field instead of emitting null
     * @returns values — The field value of every element
     */
    function pluck(this: any[], { arrayIn: any[], key?: string, skipMissing?: bool }): any[];

    /**
     * Removes and returns the last element of an array
     * @node array_pop @receiver array_in @alias arrayPop
     * @param arrayIn — Your Array (receiver: `this` in `x.pop(...)`)
     * @returns arrayOut — Adjusted Array
     * @returns value — Popped Value
     * @impure has side effects / drives control flow
     */
    function pop(this: any[], { arrayIn: any[] }): { arrayOut: any[], value: any };

    /**
     * Push an item into your Array
     * @node array_push @receiver array_in @alias arrayPush
     * @param arrayIn — Your Array (receiver: `this` in `x.push(...)`)
     * @param value — Value to push
     * @returns arrayOut — Adjusted Array
     * @impure has side effects / drives control flow
     */
    function push(this: any[], { arrayIn: any[], value: any }): any[];

    /**
     * Removes an element from an array at a specific index
     * @node array_remove_index @receiver array_in @alias arrayRemoveIndex
     * @param arrayIn — Your Array (receiver: `this` in `x.removeIndex(...)`)
     * @param index — Index to remove
     * @returns arrayOut — Adjusted Array
     * @impure has side effects / drives control flow
     */
    function removeIndex(this: any[], { arrayIn: any[], index: int }): any[];

    /**
     * The reversed array
     * @node array_reverse @receiver array_in @alias arrayReverse
     * @param arrayIn — Your Array (receiver: `this` in `x.reverse(...)`)
     * @returns arrayOut — The reversed array
     */
    function reverse(this: any[], { arrayIn: any[] }): any[];

    /**
     * Sets an element at a specific index in an array
     * @node array_set_index @receiver array_in @alias arraySetIndex
     * @param arrayIn — Your Array (receiver: `this` in `x.setIndex(...)`)
     * @param index — Index to set
     * @param value — Value to set
     * @returns arrayOut — Adjusted Array
     * @impure has side effects / drives control flow
     */
    function setIndex(this: any[], { arrayIn: any[], index: int, value: any }): any[];

    /**
     * Shuffle Array Items
     * @node array_shuffle @receiver array_in @alias arrayShuffle
     * @param arrayIn — Your Array (receiver: `this` in `x.shuffle(...)`)
     * @returns arrayOut — Adjusted Array
     */
    function shuffle(this: any[], { arrayIn: any[] }): any[];

    /**
     * The selected range of elements
     * @node array_slice @receiver array_in @alias arraySlice
     * @param arrayIn — Your Array (receiver: `this` in `x.slice(...)`)
     * @param start (optional) — First index, negative counts from the end
     * @param length (optional) — Number of elements to take, -1 for the rest of the array
     * @returns arrayOut — The selected range of elements
     */
    function slice(this: any[], { arrayIn: any[], start?: int, length?: int }): any[];

    /**
     * The sorted array
     * @node array_sort @receiver array_in @alias arraySort
     * @param arrayIn — Your Array (receiver: `this` in `x.sort(...)`)
     * @param descending (optional) — Sort from largest to smallest
     * @param compare (optional) — How the key values are ordered. Auto reads each value and falls back to text
     * @param nulls (optional) — Where elements without a key value end up
     * @returns arrayOut — The sorted array
     */
    function sort(this: any[], { arrayIn: any[], descending?: bool, compare?: string, nulls?: string }): any[];

    /**
     * Adds up one numeric field across an array of structs
     * @node array_sum_field @receiver array_in @alias arraySumField
     * @param arrayIn — Your Array (receiver: `this` in `x.sumField(...)`)
     * @param field (optional) — Field to add up, empty sums the values themselves
     * @returns sum — Sum of the field
     * @returns counted — How many entries held a number
     */
    function sumField(this: any[], { arrayIn: any[], field?: string }): { sum: float, counted: int };

    /**
     * The array without duplicate values
     * @node array_unique @receiver array_in @alias arrayUnique
     * @param arrayIn — Your Array (receiver: `this` in `x.unique(...)`)
     * @returns arrayOut — The array without duplicates
     * @returns removed — How many duplicates were dropped
     */
    function unique(this: any[], { arrayIn: any[] }): { arrayOut: any[], removed: int };

    /**
     * Pairs up the elements of two arrays, stopping at the shorter one
     * @node array_zip @receiver array_first @alias arrayZip
     * @param arrayFirst — First Array (receiver: `this` in `x.zip(...)`)
     * @param arraySecond — Second Array
     * @returns pairs — One entry per index holding both values
     */
    function zip(this: any[], { arrayFirst: any[], arraySecond: any[] }): Struct[];

    // === Utils/Array/Batch ===

    /**
     * Push multiple items into an array in one operation. More efficient than multiple single pushes.
     * @node array_batch_push @receiver array_in @alias arrayBatchPush
     * @param arrayIn — Your Array (receiver: `this` in `x.batchPush(...)`)
     * @param items — Array of items to push
     * @returns arrayOut — Array with all items pushed
     * @impure has side effects / drives control flow
     */
    function batchPush(this: any[], { arrayIn: any[], items: any[] }): any[];

    /**
     * Remove multiple elements at specific indices in one operation. More efficient than multiple single removes. Indices are processed in descending order to maintain correctness.
     * @node array_batch_remove @receiver array_in @alias arrayBatchRemove
     * @param arrayIn — Your Array (receiver: `this` in `x.batchRemove(...)`)
     * @param indices — Array of indices to remove
     * @returns arrayOut — Array with elements removed
     * @returns removed — Array of removed values
     * @impure has side effects / drives control flow
     */
    function batchRemove(this: any[], { arrayIn: any[], indices: int[] }): { arrayOut: any[], removed: any[] };

    /**
     * Set multiple elements at specific indices in one operation. More efficient than multiple single sets.
     * @node array_batch_set @receiver array_in @alias arrayBatchSet
     * @param arrayIn — Your Array (receiver: `this` in `x.batchSet(...)`)
     * @param indices — Array of indices to set
     * @param values — Array of values to set (must match indices length)
     * @returns arrayOut — Array with all values set
     * @impure has side effects / drives control flow
     */
    function batchSet(this: any[], { arrayIn: any[], indices: int[], values: any[] }): any[];

    // === Utils/Array/By Reference ===

    /**
     * Clear all elements directly from a variable array without copying.
     * @node array_clear_ref @alias arrayClearRef
     * @param varRef — Reference to the array variable to clear
     * @impure has side effects / drives control flow
     */
    function clearRef({ varRef: string }): void;

    /**
     * Append multiple items directly to a variable array without copying. Much faster for large arrays.
     * @node array_extend_ref @alias arrayExtendRef
     * @param varRef — Reference to the array variable to modify
     * @param values — Array of values to append
     * @impure has side effects / drives control flow
     */
    function extendRef({ varRef: string, values: any[] }): void;

    /**
     * Remove and return the last element directly from a variable array without copying. Much faster for large arrays.
     * @node array_pop_ref @alias arrayPopRef
     * @param varRef — Reference to the array variable to modify
     * @returns value — The popped value
     * @impure has side effects / drives control flow
     */
    function popRef({ varRef: string }): any;

    /**
     * Push an item directly into a variable array without copying. Much faster for large arrays.
     * @node array_push_ref @alias arrayPushRef
     * @param varRef — Reference to the array variable to modify
     * @param value — Value to push into the array
     * @impure has side effects / drives control flow
     */
    function pushRef({ varRef: string, value: any }): void;

    /**
     * Remove an element at a specific index directly from a variable array without copying. Much faster for large arrays.
     * @node array_remove_index_ref @alias arrayRemoveIndexRef
     * @param varRef — Reference to the array variable to modify
     * @param index — Index to remove
     * @returns value — The removed value
     * @impure has side effects / drives control flow
     */
    function removeIndexRef({ varRef: string, index: int }): any;

    /**
     * Set an element at a specific index directly in a variable array without copying. Much faster for large arrays.
     * @node array_set_index_ref @alias arraySetIndexRef
     * @param varRef — Reference to the array variable to modify
     * @param index — Index to set
     * @param value — Value to set at the index
     * @impure has side effects / drives control flow
     */
    function setIndexRef({ varRef: string, index: int, value: any }): void;

    // === Utils/Set ===

    /**
     * Converts an array to a set
     * @node array_to_set @receiver array_in @alias arrayToSet
     * @param arrayIn — (receiver: `this` in `x.toSet(...)`)
     * @returns setOut
     */
    function toSet(this: any[], { arrayIn: any[] }): Set<any>;
}

declare namespace bool {
    // === Utils/Bool ===

    /**
     * True when every boolean in the array is true
     * @node bool_all @alias boolAll
     * @param booleans — Input Booleans
     * @returns result — True when every boolean in the array is true
     */
    function all({ booleans: bool[] }): bool;

    /**
     * Boolean And operation
     * @node bool_and @receiver boolean @alias boolAnd
     * @param boolean (optional) — Input Pin for AND Operation (receiver: `this` in `x.and(...)`)
     * @param boolean (optional) — Input Pin for AND Operation (receiver: `this` in `x.and(...)`)
     * @returns result — AND operation between all boolean inputs
     */
    function and(this: bool, { boolean?: bool, boolean?: bool }): bool;

    /**
     * True when at least one boolean in the array is true
     * @node bool_any @alias boolAny
     * @param booleans — Input Booleans
     * @returns result — True when at least one boolean in the array is true
     * @returns count — How many values were true
     */
    function any({ booleans: bool[] }): { result: bool, count: int };

    /**
     * Boolean Equal
     * @node bool_equal @receiver boolean @alias boolEqual
     * @param boolean (optional) — Input Pin for OR Operation (receiver: `this` in `x.equal(...)`)
     * @param boolean (optional) — Input Pin for OR Operation (receiver: `this` in `x.equal(...)`)
     * @returns result — == operation between all boolean inputs
     */
    function equal(this: bool, { boolean?: bool, boolean?: bool }): bool;

    /**
     * Converts an integer into a boolean, zero is false
     * @node int_to_bool @alias intToBool
     * @param integer (optional) — Input Integer
     * @returns boolean — False when the integer was zero
     */
    function fromInt({ integer?: int }): bool;

    /**
     * False only when the premise is true and the conclusion is false
     * @node bool_implies @receiver premise @alias boolImplies
     * @param premise (optional) — The condition that is assumed (receiver: `this` in `x.implies(...)`)
     * @param conclusion (optional) — What has to hold when the premise is true
     * @returns result — True when the implication holds
     */
    function implies(this: bool, { premise?: bool, conclusion?: bool }): bool;

    /**
     * True unless every input is true
     * @node bool_nand @receiver boolean @alias boolNand
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.nand(...)`)
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.nand(...)`)
     * @returns result — True unless every input is true
     */
    function nand(this: bool, { boolean?: bool, boolean?: bool }): bool;

    /**
     * True only when every input is false
     * @node bool_nor @receiver boolean @alias boolNor
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.nor(...)`)
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.nor(...)`)
     * @returns result — True only when every input is false
     */
    function nor(this: bool, { boolean?: bool, boolean?: bool }): bool;

    /**
     * Boolean NOT
     * @node bool_not @receiver boolean @alias boolNot
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.not(...)`)
     * @returns result — NOT operation on the input
     */
    function not(this: bool, { boolean?: bool }): bool;

    /**
     * Boolean Or operation
     * @node bool_or @receiver boolean @alias boolOr
     * @param boolean (optional) — Input Pin for OR Operation (receiver: `this` in `x.or(...)`)
     * @param boolean (optional) — Input Pin for OR Operation (receiver: `this` in `x.or(...)`)
     * @returns result — OR operation between all boolean inputs
     */
    function or(this: bool, { boolean?: bool, boolean?: bool }): bool;

    /**
     * Generates a random boolean value
     * @node random_bool @alias randomBool
     * @param probability (optional) — The probability of the boolean being true
     * @returns value — The random boolean value
     */
    function random({ probability?: float }): bool;

    /**
     * Converts a boolean into 1 or 0
     * @node bool_to_int @receiver boolean @alias boolToInt
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.toInt(...)`)
     * @returns integer — 1 when true, 0 when false
     */
    function toInt(this: bool, { boolean?: bool }): int;

    /**
     * Converts a boolean into text
     * @node bool_to_string @receiver boolean @alias boolToString
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.toString(...)`)
     * @param trueText (optional) — Text used when the boolean is true
     * @param falseText (optional) — Text used when the boolean is false
     * @returns string — The text
     */
    function toString(this: bool, { boolean?: bool, trueText?: string, falseText?: string }): string;

    /**
     * Flips a boolean variable in place
     * @node bool_toggle @alias boolToggle
     * @param varRef — Reference to the boolean variable to flip
     * @returns newValue — The value the variable holds after flipping
     * @impure has side effects / drives control flow
     */
    function toggle({ varRef: string }): bool;

    /**
     * Checks whether two booleans differ
     * @node bool_unequal @receiver boolean1 @alias boolUnequal
     * @param boolean1 (optional) — Input Boolean (receiver: `this` in `x.unequal(...)`)
     * @param boolean2 (optional) — Input Boolean
     * @returns result — True when the booleans differ
     */
    function unequal(this: bool, { boolean1?: bool, boolean2?: bool }): bool;

    /**
     * Boolean XOR
     * @node bool_xor @receiver boolean @alias boolXor
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.xor(...)`)
     * @param boolean (optional) — Input Boolean (receiver: `this` in `x.xor(...)`)
     * @returns result — XOR operation between all boolean inputs
     */
    function xor(this: bool, { boolean?: bool, boolean?: bool }): bool;
}

declare namespace bytes {
    // === Utils/Bytes ===

    /**
     * Appends byte buffers to each other
     * @node bytes_concat @receiver bytes @alias bytesConcat
     * @param bytes — Part to append (receiver: `this` in `x.concat(...)`)
     * @param bytes — Part to append (receiver: `this` in `x.concat(...)`)
     * @returns result — All parts appended in order
     */
    function concat(this: bytes[], { bytes: bytes[], bytes: bytes[] }): bytes[];

    /**
     * Reads the leading bytes to work out what kind of file a buffer holds
     * @node bytes_detect_type @receiver bytes @alias bytesDetectType
     * @param bytes — Input Bytes (receiver: `this` in `x.detectType(...)`)
     * @returns mimeType — Detected media type, empty when nothing matched
     * @returns extension — Usual file extension for the detected type
     * @returns detected — True when a signature matched
     * @returns isText — True when the first kilobyte reads as UTF-8 text without null bytes
     */
    function detectType(this: bytes[], { bytes: bytes[] }): { mimeType: string, extension: string, detected: bool, isText: bool };

    /**
     * Compares two byte buffers for equality
     * @node bytes_equal @receiver bytes @alias bytesEqual
     * @param bytes — Input Bytes (receiver: `this` in `x.equal(...)`)
     * @param other — Input Bytes
     * @returns equal — True when both buffers hold the same bytes
     */
    function equal(this: bytes[], { bytes: bytes[], other: bytes[] }): bool;

    /**
     * Writes text out as UTF-8 bytes
     * @node text_to_bytes @alias textToBytes
     * @param text — Input Text
     * @returns bytes — The encoded bytes
     */
    function fromText({ text: string }): bytes[];

    /**
     * Compresses a byte buffer with gzip
     * @node bytes_gzip_compress @receiver bytes @alias bytesGzipCompress
     * @param bytes — Input Bytes (receiver: `this` in `x.gzipCompress(...)`)
     * @param level (optional) — Compression level from 0 (store) to 9 (smallest)
     * @returns result — The compressed bytes
     * @returns ratio — Compressed size divided by original size
     */
    function gzipCompress(this: bytes[], { bytes: bytes[], level?: int }): { result: bytes[], ratio: float };

    /**
     * Restores a gzip compressed byte buffer
     * @node bytes_gzip_decompress @receiver bytes @alias bytesGzipDecompress
     * @param bytes — Compressed Bytes (receiver: `this` in `x.gzipDecompress(...)`)
     * @param maxSize (optional) — Refuse to expand beyond this many bytes
     * @returns result — The restored bytes
     */
    function gzipDecompress(this: bytes[], { bytes: bytes[], maxSize?: int }): bytes[];

    /**
     * How many bytes the buffer holds
     * @node bytes_length @receiver bytes @alias bytesLength
     * @param bytes — Input Bytes (receiver: `this` in `x.length(...)`)
     * @returns length — Number of bytes
     * @returns isEmpty — True when the buffer holds nothing
     */
    function length(this: bytes[], { bytes: bytes[] }): { length: int, isEmpty: bool };

    /**
     * Takes a range out of a byte buffer
     * @node bytes_slice @receiver bytes @alias bytesSlice
     * @param bytes — Input Bytes (receiver: `this` in `x.slice(...)`)
     * @param start (optional) — First byte index, negative counts from the end
     * @param length (optional) — Number of bytes to take, -1 for the rest
     * @returns result — The selected bytes
     */
    function slice(this: bytes[], { bytes: bytes[], start?: int, length?: int }): bytes[];

    /**
     * Checks a buffer against a leading byte sequence, for example a file signature
     * @node bytes_starts_with @receiver bytes @alias bytesStartsWith
     * @param bytes — Input Bytes (receiver: `this` in `x.startsWith(...)`)
     * @param prefix — Bytes to look for
     * @returns startsWith — True when the buffer begins with the prefix
     */
    function startsWith(this: bytes[], { bytes: bytes[], prefix: bytes[] }): bool;

    /**
     * Reads a byte buffer as UTF-8 text
     * @node bytes_to_text @receiver bytes @alias bytesToText
     * @param bytes — Input Bytes (receiver: `this` in `x.toText(...)`)
     * @param lossy (optional) — Replace invalid sequences instead of failing
     * @returns text — The decoded text
     * @returns wasValid — False when the buffer was not valid UTF-8
     */
    function toText(this: bytes[], { bytes: bytes[], lossy?: bool }): { text: string, wasValid: bool };

    // === Utils/Encoding ===

    /**
     * Decodes a Base64 string to raw bytes
     * @node utils_encoding_base64_decode_bytes @alias utilsEncodingBase64DecodeBytes
     * @param input — Base64 encoded string
     * @returns output — Decoded raw bytes
     */
    function fromBase64({ input: string }): bytes[];

    /**
     * Decodes a hexadecimal string to raw bytes
     * @node utils_encoding_hex_decode_bytes @alias utilsEncodingHexDecodeBytes
     * @param input — Hex-encoded string
     * @returns output — Decoded raw bytes
     */
    function fromHex({ input: string }): bytes[];

    /**
     * Encodes raw bytes to a Base64 string
     * @node utils_encoding_base64_encode_bytes @receiver input @alias utilsEncodingBase64EncodeBytes
     * @param input — Raw bytes to encode (receiver: `this` in `x.toBase64(...)`)
     * @returns output — Base64 encoded string
     */
    function toBase64(this: bytes[], { input: bytes[] }): string;

    /**
     * Encodes raw bytes to a hexadecimal string
     * @node utils_encoding_hex_encode_bytes @receiver input @alias utilsEncodingHexEncodeBytes
     * @param input — Raw bytes to encode (receiver: `this` in `x.toHex(...)`)
     * @returns output — Hex-encoded string
     */
    function toHex(this: bytes[], { input: bytes[] }): string;
}

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

declare namespace crypto {
    // === Utils/Crypto ===

    /**
     * Decrypts an AES-256-GCM encrypted payload and verifies its authentication tag.
     * @node crypto_aes_decrypt_bytes @alias cryptoAesDecryptBytes
     * @param key — 32-byte symmetric key
     * @param encrypted — Authenticated encrypted payload
     * @returns plaintext — Decrypted bytes
     * @impure has side effects / drives control flow
     */
    function aesDecryptBytes({ key: bytes[], encrypted: Struct }): bytes[];

    /**
     * Decrypts an AES-256-GCM payload and parses the plaintext as a struct.
     * @node crypto_aes_decrypt_value @alias cryptoAesDecryptValue
     * @param key — 32-byte symmetric key
     * @param encrypted — Authenticated encrypted payload
     * @returns value — Decrypted struct
     * @impure has side effects / drives control flow
     */
    function aesDecryptValue({ key: bytes[], encrypted: Struct }): Struct;

    /**
     * Encrypts bytes with AES-256-GCM. A fresh nonce is generated internally for every encryption.
     * @node crypto_aes_encrypt_bytes @alias cryptoAesEncryptBytes
     * @param key — 32-byte symmetric key
     * @param plaintext — Bytes to encrypt
     * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
     * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
     * @impure has side effects / drives control flow
     */
    function aesEncryptBytes({ key: bytes[], plaintext: bytes[], associatedData?: Struct }): Struct;

    /**
     * Serializes and encrypts a struct with AES-256-GCM. A fresh nonce is generated internally for every encryption.
     * @node crypto_aes_encrypt_value @alias cryptoAesEncryptValue
     * @param key — 32-byte symmetric key
     * @param value — Struct to encrypt
     * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
     * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
     * @impure has side effects / drives control flow
     */
    function aesEncryptValue({ key: bytes[], value: Struct, associatedData?: Struct }): Struct;

    /**
     * Generates a 256-bit symmetric key for AES-256-GCM and XChaCha20-Poly1305.
     * @node crypto_generate_key @alias cryptoGenerateKey
     * @returns key — Random 32-byte symmetric key
     * @impure has side effects / drives control flow
     */
    function generateKey(): bytes[];

    /**
     * Decrypts an XChaCha20-Poly1305 encrypted payload and verifies its authentication tag.
     * @node crypto_xchacha20_decrypt_bytes @alias cryptoXchacha20DecryptBytes
     * @param key — 32-byte symmetric key
     * @param encrypted — Authenticated encrypted payload
     * @returns plaintext — Decrypted bytes
     * @impure has side effects / drives control flow
     */
    function xchacha20DecryptBytes({ key: bytes[], encrypted: Struct }): bytes[];

    /**
     * Decrypts an XChaCha20-Poly1305 payload and parses the plaintext as a struct.
     * @node crypto_xchacha20_decrypt_value @alias cryptoXchacha20DecryptValue
     * @param key — 32-byte symmetric key
     * @param encrypted — Authenticated encrypted payload
     * @returns value — Decrypted struct
     * @impure has side effects / drives control flow
     */
    function xchacha20DecryptValue({ key: bytes[], encrypted: Struct }): Struct;

    /**
     * Encrypts bytes with XChaCha20-Poly1305. A fresh 192-bit nonce is generated internally for every encryption.
     * @node crypto_xchacha20_encrypt_bytes @alias cryptoXchacha20EncryptBytes
     * @param key — 32-byte symmetric key
     * @param plaintext — Bytes to encrypt
     * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
     * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
     * @impure has side effects / drives control flow
     */
    function xchacha20EncryptBytes({ key: bytes[], plaintext: bytes[], associatedData?: Struct }): Struct;

    /**
     * Serializes and encrypts a struct with XChaCha20-Poly1305. A fresh 192-bit nonce is generated internally for every encryption.
     * @node crypto_xchacha20_encrypt_value @alias cryptoXchacha20EncryptValue
     * @param key — 32-byte symmetric key
     * @param value — Struct to encrypt
     * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
     * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
     * @impure has side effects / drives control flow
     */
    function xchacha20EncryptValue({ key: bytes[], value: Struct, associatedData?: Struct }): Struct;
}

declare namespace datetime {
    // === Utils/DateTime ===

    /**
     * Adds or subtracts a duration from a date
     * @node utils_datetime_duration @receiver date @alias utilsDatetimeDuration
     * @param date — Base date (receiver: `this` in `x.add(...)`)
     * @param days (optional) — Days to add (negative to subtract)
     * @param hours (optional) — Hours to add
     * @param minutes (optional) — Minutes to add
     * @param seconds (optional) — Seconds to add
     * @returns result — Resulting date
     */
    function add(this: Date, { date: Date, days?: int, hours?: int, minutes?: int, seconds?: int }): Date;

    /**
     * Moves a date forward or back by working days, skipping weekends
     * @node utils_datetime_add_business_days @receiver date @alias utilsDatetimeAddBusinessDays
     * @param date — Input Date (receiver: `this` in `x.addBusinessDays(...)`)
     * @param days (optional) — Working days to add, negative to go back
     * @returns result — The shifted date, always landing on a working day
     */
    function addBusinessDays(this: Date, { date: Date, days?: int }): Date;

    /**
     * Counts the working days between two dates, skipping weekends
     * @node utils_datetime_business_days_between @receiver start @alias utilsDatetimeBusinessDaysBetween
     * @param start — Start of the range (receiver: `this` in `x.businessDaysBetween(...)`)
     * @param end — End of the range
     * @param includeEnd (optional) — Count the end day itself when it is a working day
     * @returns days — Working days in the range, negative when the end lies before the start
     */
    function businessDaysBetween(this: Date, { start: Date, end: Date, includeEnd?: bool }): int;

    /**
     * Week number, weekend and leap year facts about a date
     * @node utils_datetime_calendar_info @receiver date @alias utilsDatetimeCalendarInfo
     * @param date — Input Date (receiver: `this` in `x.calendarInfo(...)`)
     * @returns isWeekend — True on Saturday and Sunday
     * @returns isLeapYear — True when February has 29 days that year
     * @returns week — ISO 8601 week number
     * @returns isoYear — Year the ISO week belongs to
     * @returns quarter — Quarter of the year, 1 to 4
     * @returns daysInMonth — Length of the month the date falls in
     */
    function calendarInfo(this: Date, { date: Date }): { isWeekend: bool, isLeapYear: bool, week: int, isoYear: int, quarter: int, daysInMonth: int };

    /**
     * Pulls a date into a range, leaving it alone when it already fits
     * @node utils_datetime_clamp @receiver date @alias utilsDatetimeClamp
     * @param date — Input Date (receiver: `this` in `x.clamp(...)`)
     * @param start — Earliest allowed date
     * @param end — Latest allowed date
     * @returns result — The date inside the range
     * @returns wasClamped — True when the date had to be moved
     */
    function clamp(this: Date, { date: Date, start: Date, end: Date }): { result: Date, wasClamped: bool };

    /**
     * Calculates the duration between two dates
     * @node utils_datetime_diff @receiver start @alias utilsDatetimeDiff
     * @param start — Start date (receiver: `this` in `x.diff(...)`)
     * @param end — End date
     * @returns totalSeconds — Total duration in seconds
     * @returns days — Number of days
     * @returns hours — Remaining hours
     * @returns minutes — Remaining minutes
     * @returns seconds — Remaining seconds
     * @returns humanReadable — Human readable duration string
     * @returns errorMessage
     * @impure has side effects / drives control flow
     */
    function diff(this: Date, { start: Date, end: Date }): { totalSeconds: int, days: int, hours: int, minutes: int, seconds: int, humanReadable: string, errorMessage: string };

    /**
     * The last instant of the day, week, month, quarter or year
     * @node utils_datetime_end_of @receiver date @alias utilsDatetimeEndOf
     * @param date — Input Date (receiver: `this` in `x.endOf(...)`)
     * @param unit (optional) — Unit to snap to
     * @returns result — The last instant of the day, week, month, quarter or year
     */
    function endOf(this: Date, { date: Date, unit?: string }): Date;

    /**
     * Converts a DateTime to a formatted string
     * @node utils_datetime_format @receiver date @alias utilsDatetimeFormat
     * @param date — Date to format (receiver: `this` in `x.format(...)`)
     * @param format (optional) — Format string (e.g., '%Y-%m-%d %H:%M:%S', '%Y-%m-%d', 'rfc3339', 'rfc2822')
     * @returns formatted — Formatted string
     */
    function format(this: Date, { date: Date, format?: string }): string;

    /**
     * Builds a date from year, month, day and time components
     * @node utils_datetime_from_parts @alias utilsDatetimeFromParts
     * @param year (optional) — Year
     * @param month (optional) — Month
     * @param day (optional) — Day
     * @param hour (optional) — Hour
     * @param minute (optional) — Minute
     * @param second (optional) — Second
     * @returns date — The assembled date
     */
    function fromParts({ year?: int, month?: int, day?: int, hour?: int, minute?: int, second?: int }): Date;

    /**
     * Converts an epoch timestamp into a date
     * @node utils_datetime_from_unix @alias utilsDatetimeFromUnix
     * @param timestamp (optional) — Epoch timestamp
     * @param unit (optional) — Unit of the timestamp. Auto reads it from the magnitude
     * @returns date — The converted date
     */
    function fromUnix({ timestamp?: int, unit?: string }): Date;

    /**
     * Describes how far a date lies from now, for example "3 days ago"
     * @node utils_datetime_humanize @receiver date @alias utilsDatetimeHumanize
     * @param date — Input Date (receiver: `this` in `x.humanize(...)`)
     * @param reference — What to measure against. Leave empty for the current time
     * @returns text — Relative description of the distance
     * @returns isPast — True when the date lies before the reference
     * @returns seconds — Signed distance in seconds, positive when the date is in the past
     */
    function humanize(this: Date, { date: Date, reference: Date }): { text: string, isPast: bool, seconds: int };

    /**
     * The later of two dates
     * @node utils_datetime_max @receiver date @alias utilsDatetimeMax
     * @param date — Input Date (receiver: `this` in `x.max(...)`)
     * @param other — Input Date
     * @returns result — The later of two dates
     */
    function max(this: Date, { date: Date, other: Date }): Date;

    /**
     * The latest date in an array
     * @node utils_datetime_max_of @alias utilsDatetimeMaxOf
     * @param dates — Input Dates
     * @returns result — The latest date in an array
     * @returns found — False when the array held no readable date
     */
    function maxOf({ dates: Date[] }): { result: Date, found: bool };

    /**
     * The earlier of two dates
     * @node utils_datetime_min @receiver date @alias utilsDatetimeMin
     * @param date — Input Date (receiver: `this` in `x.min(...)`)
     * @param other — Input Date
     * @returns result — The earlier of two dates
     */
    function min(this: Date, { date: Date, other: Date }): Date;

    /**
     * The earliest date in an array
     * @node utils_datetime_min_of @alias utilsDatetimeMinOf
     * @param dates — Input Dates
     * @returns result — The earliest date in an array
     * @returns found — False when the array held no readable date
     */
    function minOf({ dates: Date[] }): { result: Date, found: bool };

    /**
     * Returns the current date and time in UTC
     * @node utils_datetime_now @alias utilsDatetimeNow
     * @returns date — Current UTC date and time
     * @impure has side effects / drives control flow
     */
    function now(): Date;

    /**
     * Parses a string into a DateTime. Auto-detects common formats and epoch timestamps (seconds, milliseconds, microseconds, nanoseconds) or uses a custom format string.
     * @node utils_datetime_parse @alias utilsDatetimeParse
     * @param input — String to parse
     * @param format (optional) — Optional format string (e.g., '%Y-%m-%d %H:%M:%S'). Leave empty for auto-detection.
     * @returns date — Parsed date
     */
    function parse({ input: string, format?: string }): Date;

    /**
     * Calendar-aware shift that keeps the day of month where it exists
     * @node utils_datetime_shift_calendar @receiver date @alias utilsDatetimeShiftCalendar
     * @param date — Input Date (receiver: `this` in `x.shiftCalendar(...)`)
     * @param months (optional) — Months to add, negative to go back
     * @param years (optional) — Years to add, negative to go back
     * @returns result — The shifted date
     */
    function shiftCalendar(this: Date, { date: Date, months?: int, years?: int }): Date;

    /**
     * The first instant of the day, week, month, quarter or year
     * @node utils_datetime_start_of @receiver date @alias utilsDatetimeStartOf
     * @param date — Input Date (receiver: `this` in `x.startOf(...)`)
     * @param unit (optional) — Unit to snap to
     * @returns result — The first instant of the day, week, month, quarter or year
     */
    function startOf(this: Date, { date: Date, unit?: string }): Date;

    /**
     * Extracts date components from a DateTime
     * @node utils_datetime_to_date @receiver date @alias utilsDatetimeToDate
     * @param date — DateTime to extract from (receiver: `this` in `x.toDate(...)`)
     * @returns year — Year
     * @returns month — Month (1-12)
     * @returns day — Day of month (1-31)
     * @returns weekday — Day of week (0=Monday, 6=Sunday)
     * @returns dayOfYear — Day of year (1-366)
     */
    function toDate(this: Date, { date: Date }): { year: int, month: int, day: int, weekday: int, dayOfYear: int };

    /**
     * Extracts time components from a DateTime
     * @node utils_datetime_to_time @receiver date @alias utilsDatetimeToTime
     * @param date — DateTime to extract from (receiver: `this` in `x.toTime(...)`)
     * @returns hour — Hour (0-23)
     * @returns minute — Minute (0-59)
     * @returns second — Second (0-59)
     * @returns nanosecond — Nanosecond (0-999999999)
     */
    function toTime(this: Date, { date: Date }): { hour: int, minute: int, second: int, nanosecond: int };

    /**
     * Reads a date in another timezone. The instant stays the same, the wall clock changes
     * @node utils_datetime_to_timezone @receiver date @alias utilsDatetimeToTimezone
     * @param date — Input Date (receiver: `this` in `x.toTimezone(...)`)
     * @param timezone (optional) — IANA timezone name, for example Europe/Berlin or America/New_York
     * @param format (optional) — Format for the text output, for example %Y-%m-%d %H:%M
     * @returns dateOut — The same instant carrying the target offset
     * @returns formatted — Local wall clock time as text
     * @returns offsetSeconds — Offset from UTC at that instant, daylight saving included
     */
    function toTimezone(this: Date, { date: Date, timezone?: string, format?: string }): { dateOut: Date, formatted: string, offsetSeconds: int };

    /**
     * Converts a date into an epoch timestamp
     * @node utils_datetime_to_unix @receiver date @alias utilsDatetimeToUnix
     * @param date — Input Date (receiver: `this` in `x.toUnix(...)`)
     * @param unit (optional) — Unit of the produced timestamp
     * @returns timestamp — Epoch timestamp in the selected unit
     */
    function toUnix(this: Date, { date: Date, unit?: string }): int;

    // === Utils/DateTime/Comparison ===

    /**
     * True when the first date lies after the second
     * @node utils_datetime_after @receiver date @alias utilsDatetimeAfter
     * @param date — Date to test (receiver: `this` in `x.isAfter(...)`)
     * @param other — Date to compare against
     * @returns result — True when the first date lies after the second
     */
    function isAfter(this: Date, { date: Date, other: Date }): bool;

    /**
     * True when the first date lies before the second
     * @node utils_datetime_before @receiver date @alias utilsDatetimeBefore
     * @param date — Date to test (receiver: `this` in `x.isBefore(...)`)
     * @param other — Date to compare against
     * @returns result — True when the first date lies before the second
     */
    function isBefore(this: Date, { date: Date, other: Date }): bool;

    /**
     * True when a date falls inside a range
     * @node utils_datetime_between @receiver date @alias utilsDatetimeBetween
     * @param date — Date to test (receiver: `this` in `x.isBetween(...)`)
     * @param start — Start of the range
     * @param end — End of the range
     * @param inclusive (optional) — Count the boundaries as inside the range
     * @returns result — True when the date lies in the range
     */
    function isBetween(this: Date, { date: Date, start: Date, end: Date, inclusive?: bool }): bool;

    /**
     * True when both dates fall into the same unit
     * @node utils_datetime_same @receiver date @alias utilsDatetimeSame
     * @param date — Date to test (receiver: `this` in `x.isSame(...)`)
     * @param other — Date to compare against
     * @param unit (optional) — Granularity the comparison runs at
     * @returns result — True when both dates fall into the same unit
     */
    function isSame(this: Date, { date: Date, other: Date, unit?: string }): bool;
}

declare namespace encoding {
    // === Utils/Encoding ===

    /**
     * Decodes a Base64 string back to a UTF-8 string
     * @node utils_encoding_base64_decode @receiver input @alias utilsEncodingBase64Decode
     * @param input — Base64 encoded string (receiver: `this` in `x.base64Decode(...)`)
     * @returns output — Decoded UTF-8 string
     */
    function base64Decode(this: string, { input: string }): string;

    /**
     * Encodes a string to Base64
     * @node utils_encoding_base64_encode @receiver input @alias utilsEncodingBase64Encode
     * @param input — String to encode (receiver: `this` in `x.base64Encode(...)`)
     * @returns output — Base64 encoded string
     */
    function base64Encode(this: string, { input: string }): string;

    /**
     * Decodes a hexadecimal string back to a UTF-8 string
     * @node utils_encoding_hex_decode @receiver input @alias utilsEncodingHexDecode
     * @param input — Hex-encoded string (receiver: `this` in `x.hexDecode(...)`)
     * @returns output — Decoded UTF-8 string
     */
    function hexDecode(this: string, { input: string }): string;

    /**
     * Encodes a string's bytes to a hexadecimal string
     * @node utils_encoding_hex_encode @receiver input @alias utilsEncodingHexEncode
     * @param input — String to encode (receiver: `this` in `x.hexEncode(...)`)
     * @returns output — Hex-encoded string
     */
    function hexEncode(this: string, { input: string }): string;

    /**
     * Decodes HTML entities back to their original characters
     * @node utils_encoding_html_decode @receiver input @alias utilsEncodingHtmlDecode
     * @param input — HTML-encoded string (receiver: `this` in `x.htmlDecode(...)`)
     * @returns output — Decoded string
     */
    function htmlDecode(this: string, { input: string }): string;

    /**
     * Encodes special characters as HTML entities (&amp; &lt; &gt; &quot; &#39;)
     * @node utils_encoding_html_encode @receiver input @alias utilsEncodingHtmlEncode
     * @param input — String to encode (receiver: `this` in `x.htmlEncode(...)`)
     * @returns output — HTML-encoded string
     */
    function htmlEncode(this: string, { input: string }): string;

    /**
     * Decodes a percent-encoded URL string back to plain text
     * @node utils_encoding_url_decode @receiver input @alias utilsEncodingUrlDecode
     * @param input — URL-encoded string (receiver: `this` in `x.urlDecode(...)`)
     * @returns output — Decoded string
     */
    function urlDecode(this: string, { input: string }): string;

    /**
     * Percent-encodes a string for safe use in URLs (RFC 3986)
     * @node utils_encoding_url_encode @receiver input @alias utilsEncodingUrlEncode
     * @param input — String to encode (receiver: `this` in `x.urlEncode(...)`)
     * @returns output — URL-encoded string
     */
    function urlEncode(this: string, { input: string }): string;
}

declare namespace execution {
    // === Utils/Execution ===

    /**
     * Returns the current app identifier.
     * @node utils_execution_get_app_id @alias utilsExecutionGetAppId
     * @returns appId — Current app identifier
     */
    function getAppId(): string;

    /**
     * Returns where and how the current run is executing.
     * @node utils_execution_get_environment @alias utilsExecutionGetEnvironment
     * @returns environment — The execution environment: local, desktop, mobile, browser_sandbox, or server
     * @returns executionMode — The execution mode: sync, async, event, or scheduled
     * @returns isDesktop — True when the run is executing locally in the desktop app
     * @returns isServer — True when the run is executing on the server
     * @returns isMobile — True when the run is executing on a mobile runtime
     * @returns isBrowserSandbox — True when the run is executing in a browser sandbox runtime
     * @returns isLocal — True when the run has local/offline execution context
     * @returns isRemote — True when the run does not have local/offline execution context
     * @returns runId — Current run identifier
     * @returns appId — Current app identifier, if available
     * @returns userId — Current user identifier, if available
     * @returns details — Structured execution environment details
     */
    function getEnvironment(): { environment: string, executionMode: string, isDesktop: bool, isServer: bool, isMobile: bool, isBrowserSandbox: bool, isLocal: bool, isRemote: bool, runId: string, appId: string, userId: string, details: Struct };

    /**
     * Returns the current execution mode.
     * @node utils_execution_get_mode @alias utilsExecutionGetMode
     * @returns mode — The execution mode: sync, async, event, or scheduled
     */
    function getMode(): string;

    /**
     * Returns the current execution run identifier.
     * @node utils_execution_get_run_id @alias utilsExecutionGetRunId
     * @returns runId — Current run identifier
     */
    function getRunId(): string;

    /**
     * Returns the current user identifier, when available.
     * @node utils_execution_get_user_id @alias utilsExecutionGetUserId
     * @returns userId — Current user identifier, or empty when unavailable
     */
    function getUserId(): string;

    /**
     * Returns true when the current run is executing on a local/client runtime.
     * @node utils_execution_is_local_environment @alias utilsExecutionIsLocalEnvironment
     * @returns isLocal — True for local, desktop, mobile, and browser sandbox execution
     */
    function isLocalEnvironment(): bool;

    /**
     * Returns true when the current run is executing on a mobile runtime.
     * @node utils_execution_is_mobile_environment @alias utilsExecutionIsMobileEnvironment
     * @returns isMobile — True for mobile execution
     */
    function isMobileEnvironment(): bool;

    /**
     * Returns true when the current run is executing on the server.
     * @node utils_execution_is_server_environment @alias utilsExecutionIsServerEnvironment
     * @returns isServer — True for server-side execution
     */
    function isServerEnvironment(): bool;
}

declare namespace faker {
    namespace address {
        // === Utils/Faker/Address ===

        /**
         * Generates a random city name for mocking data
         * @node faker_city_name @alias fakerCityName
         * @returns city — Generated city name
         * @impure has side effects / drives control flow
         */
        function cityName(): string;

        /**
         * Generates a random country code (e.g., US, DE, FR) for mocking data
         * @node faker_country_code @alias fakerCountryCode
         * @returns code — Generated country code
         * @impure has side effects / drives control flow
         */
        function countryCode(): string;

        /**
         * Generates a random country name for mocking data
         * @node faker_country_name @alias fakerCountryName
         * @returns country — Generated country name
         * @impure has side effects / drives control flow
         */
        function countryName(): string;

        /**
         * Generates a random latitude coordinate for mocking data
         * @node faker_latitude @alias fakerLatitude
         * @returns latitude — Generated latitude
         * @impure has side effects / drives control flow
         */
        function latitude(): float;

        /**
         * Generates a random longitude coordinate for mocking data
         * @node faker_longitude @alias fakerLongitude
         * @returns longitude — Generated longitude
         * @impure has side effects / drives control flow
         */
        function longitude(): float;

        /**
         * Generates a random postal/zip code for mocking data
         * @node faker_post_code @alias fakerPostCode
         * @returns code — Generated postal code
         * @impure has side effects / drives control flow
         */
        function postCode(): string;

        /**
         * Generates a random state/province name for mocking data
         * @node faker_state_name @alias fakerStateName
         * @returns state — Generated state name
         * @impure has side effects / drives control flow
         */
        function stateName(): string;

        /**
         * Generates a random full street address for mocking data
         * @node faker_street_address @alias fakerStreetAddress
         * @returns address — Generated street address
         * @impure has side effects / drives control flow
         */
        function streetAddress(): string;

        /**
         * Generates a random street name for mocking data
         * @node faker_street_name @alias fakerStreetName
         * @returns street — Generated street name
         * @impure has side effects / drives control flow
         */
        function streetName(): string;
    }

    namespace company {
        // === Utils/Faker/Company ===

        /**
         * Generates a random business buzzword for mocking data
         * @node faker_buzzword @alias fakerBuzzword
         * @returns buzzword — Generated buzzword
         * @impure has side effects / drives control flow
         */
        function buzzword(): string;

        /**
         * Generates a random business catch phrase for mocking data
         * @node faker_catch_phrase @alias fakerCatchPhrase
         * @returns phrase — Generated catch phrase
         * @impure has side effects / drives control flow
         */
        function catchPhrase(): string;

        /**
         * Generates a random industry name for mocking data
         * @node faker_industry @alias fakerIndustry
         * @returns industry — Generated industry name
         * @impure has side effects / drives control flow
         */
        function industry(): string;

        /**
         * Generates a random company name for mocking data
         * @node faker_company_name @alias fakerCompanyName
         * @returns company — Generated company name
         * @impure has side effects / drives control flow
         */
        function name(): string;

        /**
         * Generates a random profession/job title for mocking data
         * @node faker_profession @alias fakerProfession
         * @returns profession — Generated profession
         * @impure has side effects / drives control flow
         */
        function profession(): string;
    }

    namespace internet {
        // === Utils/Faker/Internet ===

        /**
         * Generates a random domain suffix (com, org, net, etc.)
         * @node faker_domain_suffix @alias fakerDomainSuffix
         * @returns suffix — Generated domain suffix
         * @impure has side effects / drives control flow
         */
        function domainSuffix(): string;

        /**
         * Generates a random email address for mocking data
         * @node faker_email @alias fakerEmail
         * @returns email — Generated email address
         * @impure has side effects / drives control flow
         */
        function email(): string;

        /**
         * Generates a random IPv4 address for mocking data
         * @node faker_ipv4 @alias fakerIpv4
         * @returns ip — Generated IPv4 address
         * @impure has side effects / drives control flow
         */
        function ipv4(): string;

        /**
         * Generates a random IPv6 address for mocking data
         * @node faker_ipv6 @alias fakerIpv6
         * @returns ip — Generated IPv6 address
         * @impure has side effects / drives control flow
         */
        function ipv6(): string;

        /**
         * Generates a random password for mocking data
         * @node faker_password @alias fakerPassword
         * @param minLength (optional) — Minimum password length
         * @param maxLength (optional) — Maximum password length
         * @returns password — Generated password
         * @impure has side effects / drives control flow
         */
        function password({ minLength?: int, maxLength?: int }): string;

        /**
         * Generates a random user agent string for mocking data
         * @node faker_user_agent @alias fakerUserAgent
         * @returns userAgent — Generated user agent
         * @impure has side effects / drives control flow
         */
        function userAgent(): string;

        /**
         * Generates a random username for mocking data
         * @node faker_username @alias fakerUsername
         * @returns username — Generated username
         * @impure has side effects / drives control flow
         */
        function username(): string;
    }

    namespace lorem {
        // === Utils/Faker/Lorem ===

        /**
         * Generates a random lorem ipsum paragraph for mocking data
         * @node faker_paragraph @alias fakerParagraph
         * @param minSentences (optional) — Minimum sentences in paragraph
         * @param maxSentences (optional) — Maximum sentences in paragraph
         * @returns paragraph — Generated paragraph
         * @impure has side effects / drives control flow
         */
        function paragraph({ minSentences?: int, maxSentences?: int }): string;

        /**
         * Generates random lorem ipsum paragraphs for mocking data
         * @node faker_paragraphs @alias fakerParagraphs
         * @param minCount (optional) — Minimum number of paragraphs
         * @param maxCount (optional) — Maximum number of paragraphs
         * @returns paragraphs — Generated paragraphs as array
         * @impure has side effects / drives control flow
         */
        function paragraphs({ minCount?: int, maxCount?: int }): any;

        /**
         * Generates a random lorem ipsum sentence for mocking data
         * @node faker_sentence @alias fakerSentence
         * @param minWords (optional) — Minimum words in sentence
         * @param maxWords (optional) — Maximum words in sentence
         * @returns sentence — Generated sentence
         * @impure has side effects / drives control flow
         */
        function sentence({ minWords?: int, maxWords?: int }): string;

        /**
         * Generates random lorem ipsum sentences for mocking data
         * @node faker_sentences @alias fakerSentences
         * @param minCount (optional) — Minimum number of sentences
         * @param maxCount (optional) — Maximum number of sentences
         * @returns sentences — Generated sentences as array
         * @impure has side effects / drives control flow
         */
        function sentences({ minCount?: int, maxCount?: int }): any;

        /**
         * Generates a random lorem ipsum word for mocking data
         * @node faker_word @alias fakerWord
         * @returns word — Generated word
         * @impure has side effects / drives control flow
         */
        function word(): string;

        /**
         * Generates random lorem ipsum words for mocking data
         * @node faker_words @alias fakerWords
         * @param minCount (optional) — Minimum number of words
         * @param maxCount (optional) — Maximum number of words
         * @returns words — Generated words as array
         * @impure has side effects / drives control flow
         */
        function words({ minCount?: int, maxCount?: int }): any;
    }

    namespace name {
        // === Utils/Faker/Name ===

        /**
         * Generates a random first name for mocking data
         * @node faker_first_name @alias fakerFirstName
         * @returns name — Generated first name
         * @impure has side effects / drives control flow
         */
        function firstName(): string;

        /**
         * Generates a random full name for mocking data
         * @node faker_full_name @alias fakerFullName
         * @returns name — Generated full name
         * @impure has side effects / drives control flow
         */
        function fullName(): string;

        /**
         * Generates a random last name for mocking data
         * @node faker_last_name @alias fakerLastName
         * @returns name — Generated last name
         * @impure has side effects / drives control flow
         */
        function lastName(): string;

        /**
         * Generates a random name title (Mr., Mrs., Dr., etc.)
         * @node faker_title @alias fakerTitle
         * @returns title — Generated title
         * @impure has side effects / drives control flow
         */
        function title(): string;
    }

    namespace number {
        // === Utils/Faker/Number ===

        /**
         * Generates a random boolean for mocking data
         * @node faker_boolean @alias fakerBoolean
         * @param probability (optional) — Probability of true (0.0 to 1.0)
         * @returns value — Generated boolean
         * @impure has side effects / drives control flow
         */
        function boolean({ probability?: float }): bool;

        /**
         * Generates a random digit (0-9) for mocking data
         * @node faker_digit @alias fakerDigit
         * @returns digit — Generated digit
         * @impure has side effects / drives control flow
         */
        function digit(): int;

        /**
         * Generates a random float in a specified range for mocking data
         * @node faker_float @alias fakerFloat
         * @param min (optional) — Minimum value (inclusive)
         * @param max (optional) — Maximum value (exclusive)
         * @returns number — Generated float
         * @impure has side effects / drives control flow
         */
        function float({ min?: float, max?: float }): float;

        /**
         * Generates a random integer in a specified range for mocking data
         * @node faker_integer @alias fakerInteger
         * @param min (optional) — Minimum value (inclusive)
         * @param max (optional) — Maximum value (exclusive)
         * @returns number — Generated integer
         * @impure has side effects / drives control flow
         */
        function integer({ min?: int, max?: int }): int;
    }

    namespace phone {
        // === Utils/Faker/Phone ===

        /**
         * Generates a random cell/mobile phone number for mocking data
         * @node faker_cell_number @alias fakerCellNumber
         * @returns phone — Generated cell number
         * @impure has side effects / drives control flow
         */
        function cellNumber(): string;

        /**
         * Generates a random phone number for mocking data
         * @node faker_phone_number @alias fakerPhoneNumber
         * @returns phone — Generated phone number
         * @impure has side effects / drives control flow
         */
        function number(): string;
    }
}

declare namespace files {
    // === Utils/CSV ===

    /**
     * Stream Read a CSV File
     * @node csv_buffered_reader @alias csvBufferedReader
     * @param csv — CSV Path
     * @param chunkSize (optional) — Chunk Size for Buffered Read
     * @param delimiter (optional) — Delimiter for CSV
     * @returns chunk — Chunk
     * @impure has side effects / drives control flow
     */
    function readCsvBuffered({ csv: Struct, chunkSize?: int, delimiter?: string }): Struct[];
}

declare namespace float {
    // === Math/Float ===

    /**
     * Calculates the absolute value of a float
     * @node float_abs @receiver float @alias floatAbs
     * @param float — Input Float (receiver: `this` in `x.abs(...)`)
     * @returns absolute — The absolute value of the float
     */
    function abs(this: float, { float: float }): float;

    /**
     * Adds two floats together
     * @node float_add @receiver float1 @alias floatAdd
     * @param float1 — First Float (receiver: `this` in `x.add(...)`)
     * @param float2 — Second Float
     * @returns sum — The sum of the two floats
     */
    function add(this: float, { float1: float, float2: float }): float;

    /**
     * Rounds a float up to the nearest integer
     * @node float_ceil @receiver float @alias floatCeil
     * @param float — Input Float (receiver: `this` in `x.ceil(...)`)
     * @returns ceiling — The ceiling of the float
     */
    function ceil(this: float, { float: float }): int;

    /**
     * Clamps a float within a given range
     * @node float_clamp @receiver float @alias floatClamp
     * @param float — Input Float (receiver: `this` in `x.clamp(...)`)
     * @param min — Minimum Value
     * @param max — Maximum Value
     * @returns clamped — The clamped float
     */
    function clamp(this: float, { float: float, min: float, max: float }): float;

    /**
     * Provides a mathematical constant such as Pi or E
     * @node float_constant @alias floatConstant
     * @param constant (optional) — Which constant to emit
     * @returns value — The value of the constant
     */
    function constant({ constant?: string }): float;

    /**
     * Takes the magnitude of the first float and the sign of the second
     * @node float_copysign @receiver float1 @alias floatCopysign
     * @param float1 — Input Float (receiver: `this` in `x.copysign(...)`)
     * @param float2 — Input Float
     * @returns result — Takes the magnitude of the first float and the sign of the second
     */
    function copysign(this: float, { float1: float, float2: float }): float;

    /**
     * Divides one float by another
     * @node float_divide @receiver dividend @alias floatDivide
     * @param dividend — The number to be divided (receiver: `this` in `x.divide(...)`)
     * @param divisor — The number to divide by
     * @returns quotient — The result of the division
     */
    function divide(this: float, { dividend: float, divisor: float }): float;

    /**
     * Raises e to the power of a float
     * @node float_exp @receiver float @alias floatExp
     * @param float — Input Float (receiver: `this` in `x.exp(...)`)
     * @returns result — Raises e to the power of a float
     */
    function exp(this: float, { float: float }): float;

    /**
     * Raises two to the power of a float
     * @node float_exp2 @receiver float @alias floatExp2
     * @param float — Input Float (receiver: `this` in `x.exp2(...)`)
     * @returns result — Raises two to the power of a float
     */
    function exp2(this: float, { float: float }): float;

    /**
     * Rounds a float down to the nearest integer
     * @node float_floor @receiver float @alias floatFloor
     * @param float — Input Float (receiver: `this` in `x.floor(...)`)
     * @returns floor — The floor of the float
     */
    function floor(this: float, { float: float }): int;

    /**
     * Keeps only the fractional part of a float
     * @node float_fract @receiver float @alias floatFract
     * @param float — Input Float (receiver: `this` in `x.fract(...)`)
     * @returns result — Keeps only the fractional part of a float
     */
    function fract(this: float, { float: float }): float;

    /**
     * Length of the hypotenuse of a right-angled triangle
     * @node float_hypot @receiver float1 @alias floatHypot
     * @param float1 — Input Float (receiver: `this` in `x.hypot(...)`)
     * @param float2 — Input Float
     * @returns result — Length of the hypotenuse of a right-angled triangle
     */
    function hypot(this: float, { float1: float, float2: float }): float;

    /**
     * Interpolates linearly between two floats
     * @node float_lerp @receiver from @alias floatLerp
     * @param from (optional) — Value at t = 0 (receiver: `this` in `x.lerp(...)`)
     * @param to (optional) — Value at t = 1
     * @param t (optional) — Interpolation factor
     * @param clamp (optional) — Clamp the factor into the range 0 to 1
     * @returns result — The interpolated value
     */
    function lerp(this: float, { from?: float, to?: float, t?: float, clamp?: bool }): float;

    /**
     * Natural logarithm of a float
     * @node float_ln @receiver float @alias floatLn
     * @param float — Input Float (receiver: `this` in `x.ln(...)`)
     * @returns result — Natural logarithm of a float
     */
    function ln(this: float, { float: float }): float;

    /**
     * Logarithm of a float to a custom base
     * @node float_log @receiver float @alias floatLog
     * @param float — Input Float (receiver: `this` in `x.log(...)`)
     * @param base (optional) — Logarithm base
     * @returns result — Logarithm of a float to a custom base
     */
    function log(this: float, { float: float, base?: float }): float;

    /**
     * Base 10 logarithm of a float
     * @node float_log10 @receiver float @alias floatLog10
     * @param float — Input Float (receiver: `this` in `x.log10(...)`)
     * @returns result — Base 10 logarithm of a float
     */
    function log10(this: float, { float: float }): float;

    /**
     * Base 2 logarithm of a float
     * @node float_log2 @receiver float @alias floatLog2
     * @param float — Input Float (receiver: `this` in `x.log2(...)`)
     * @returns result — Base 2 logarithm of a float
     */
    function log2(this: float, { float: float }): float;

    /**
     * Rescales a value from one range into another
     * @node float_map_range @receiver value @alias floatMapRange
     * @param value — Value to rescale (receiver: `this` in `x.mapRange(...)`)
     * @param inMin (optional) — In Min
     * @param inMax (optional) — In Max
     * @param outMin (optional) — Out Min
     * @param outMax (optional) — Out Max
     * @param clamp (optional) — Keep the result inside the output range
     * @returns result — The rescaled value
     */
    function mapRange(this: float, { value: float, inMin?: float, inMax?: float, outMin?: float, outMax?: float, clamp?: bool }): float;

    /**
     * Returns the larger of two floats
     * @node float_max @receiver float1 @alias floatMax
     * @param float1 — First Float (receiver: `this` in `x.max(...)`)
     * @param float2 — Second Float
     * @returns maximum — The larger of the two floats
     */
    function max(this: float, { float1: float, float2: float }): float;

    /**
     * Returns the smaller of two floats
     * @node float_min @receiver float1 @alias floatMin
     * @param float1 — First Float (receiver: `this` in `x.min(...)`)
     * @param float2 — Second Float
     * @returns minimum — The smaller of the two floats
     */
    function min(this: float, { float1: float, float2: float }): float;

    /**
     * Remainder of a float division
     * @node float_modulo @receiver float1 @alias floatModulo
     * @param float1 — Dividend (receiver: `this` in `x.modulo(...)`)
     * @param float2 — Divisor
     * @param euclidean (optional) — Return a non-negative remainder
     * @returns remainder — Remainder of the division
     */
    function modulo(this: float, { float1: float, float2: float, euclidean?: bool }): float;

    /**
     * Multiplies two floats and adds a third in one rounding step
     * @node float_mul_add @receiver float @alias floatMulAdd
     * @param float — Input Float (receiver: `this` in `x.mulAdd(...)`)
     * @param factor (optional) — Multiplied with the input
     * @param addend (optional) — Added to the product
     * @returns result — Input multiplied by the factor plus the addend
     */
    function mulAdd(this: float, { float: float, factor?: float, addend?: float }): float;

    /**
     * Multiplies two floats together
     * @node float_multiply @receiver float1 @alias floatMultiply
     * @param float1 — First Float (receiver: `this` in `x.multiply(...)`)
     * @param float2 — Second Float
     * @returns product — The product of the two floats
     */
    function multiply(this: float, { float1: float, float2: float }): float;

    /**
     * How much a value moved relative to where it started
     * @node float_percent_change @receiver from @alias floatPercentChange
     * @param from (optional) — Earlier value (receiver: `this` in `x.percentChange(...)`)
     * @param to (optional) — Later value
     * @returns percent — Change in percent, negative when the value fell
     * @returns delta — Absolute change
     * @returns defined — False when the earlier value was zero, which has no percentage
     */
    function percentChange(this: float, { from?: float, to?: float }): { percent: float, delta: float, defined: bool };

    /**
     * Calculates the power of a float
     * @node float_power @receiver base @alias floatPower
     * @param base — Base float (receiver: `this` in `x.power(...)`)
     * @param exponent — Exponent float
     * @returns power — Result of the power calculation
     */
    function power(this: float, { base: float, exponent: float }): float;

    /**
     * One divided by a float
     * @node float_recip @receiver float @alias floatRecip
     * @param float — Input Float (receiver: `this` in `x.recip(...)`)
     * @returns result — One divided by a float
     */
    function recip(this: float, { float: float }): float;

    /**
     * Calculates the nth root of a float
     * @node float_root @receiver radicand @alias floatRoot
     * @param radicand — The float to take the root of (receiver: `this` in `x.root(...)`)
     * @param degree — The degree of the root
     * @returns root — Result of the root calculation
     */
    function root(this: float, { radicand: float, degree: int }): float;

    /**
     * Rounds a float to the given number of decimal places
     * @node float_round @receiver float @alias floatRound
     * @param float — Input Float (receiver: `this` in `x.round(...)`)
     * @param decimals (optional) — Number of decimal places to keep
     * @returns rounded — The rounded float
     */
    function round(this: float, { float: float, decimals?: int }): float;

    /**
     * Snaps a value to the nearest multiple, for example the nearest 0.05 or 25
     * @node float_round_to_multiple @receiver value @alias floatRoundToMultiple
     * @param value — Value to snap (receiver: `this` in `x.roundToMultiple(...)`)
     * @param multiple (optional) — Step size to snap to
     * @param mode (optional) — Which direction to snap in
     * @returns result — The snapped value
     */
    function roundToMultiple(this: float, { value: float, multiple?: float, mode?: string }): float;

    /**
     * Returns -1 or 1 depending on the sign of a float
     * @node float_signum @receiver float @alias floatSignum
     * @param float — Input Float (receiver: `this` in `x.signum(...)`)
     * @returns result — Returns -1 or 1 depending on the sign of a float
     */
    function signum(this: float, { float: float }): float;

    /**
     * Square root of a float
     * @node float_sqrt @receiver float @alias floatSqrt
     * @param float — Input Float (receiver: `this` in `x.sqrt(...)`)
     * @returns result — Square root of a float
     */
    function sqrt(this: float, { float: float }): float;

    /**
     * Subtracts one float from another
     * @node float_subtract @receiver float1 @alias floatSubtract
     * @param float1 — First Float (receiver: `this` in `x.subtract(...)`)
     * @param float2 — Second Float
     * @returns difference — The difference between the two floats
     */
    function subtract(this: float, { float1: float, float2: float }): float;

    /**
     * Converts a float into an integer using the selected rounding
     * @node float_to_int @receiver float @alias floatToInt
     * @param float — Input Float (receiver: `this` in `x.toInt(...)`)
     * @param rounding (optional) — How to remove the fractional part
     * @returns integer — The converted value
     */
    function toInt(this: float, { float: float, rounding?: string }): int;

    /**
     * Drops the fractional part of a float
     * @node float_trunc @receiver float @alias floatTrunc
     * @param float — Input Float (receiver: `this` in `x.trunc(...)`)
     * @returns result — Drops the fractional part of a float
     */
    function trunc(this: float, { float: float }): float;

    // === Math/Float/Aggregate ===

    /**
     * Arithmetic mean of every float in an array
     * @node float_average @alias floatAverage
     * @param floats — Input Floats
     * @returns result — Arithmetic mean of every float in an array
     * @returns empty — True when the input array held no values
     */
    function average({ floats: float[] }): { result: float, empty: bool };

    /**
     * Largest float in an array
     * @node float_max_of @alias floatMaxOf
     * @param floats — Input Floats
     * @returns result — Largest float in an array
     * @returns empty — True when the input array held no values
     */
    function maxOf({ floats: float[] }): { result: float, empty: bool };

    /**
     * Middle value of an array, averaging the two middle values for even counts
     * @node float_median @alias floatMedian
     * @param floats — Input Floats
     * @returns result — Middle value of an array, averaging the two middle values for even counts
     * @returns empty — True when the input array held no values
     */
    function median({ floats: float[] }): { result: float, empty: bool };

    /**
     * Smallest float in an array
     * @node float_min_of @alias floatMinOf
     * @param floats — Input Floats
     * @returns result — Smallest float in an array
     * @returns empty — True when the input array held no values
     */
    function minOf({ floats: float[] }): { result: float, empty: bool };

    /**
     * Value at a percentile of an array, interpolating between neighbours
     * @node float_percentile @alias floatPercentile
     * @param floats — Input Floats
     * @param percentile (optional) — Percentile between 0 and 100
     * @returns result — Value at a percentile of an array, interpolating between neighbours
     * @returns empty — True when the input array held no values
     */
    function percentile({ floats: float[], percentile?: float }): { result: float, empty: bool };

    /**
     * Population standard deviation of every float in an array
     * @node float_std_dev @alias floatStdDev
     * @param floats — Input Floats
     * @returns result — Population standard deviation of every float in an array
     * @returns empty — True when the input array held no values
     */
    function stdDev({ floats: float[] }): { result: float, empty: bool };

    /**
     * Adds up every float in an array
     * @node float_sum @alias floatSum
     * @param floats — Input Floats
     * @returns result — Adds up every float in an array
     * @returns empty — True when the input array held no values
     */
    function sum({ floats: float[] }): { result: float, empty: bool };

    /**
     * Population variance of every float in an array
     * @node float_variance @alias floatVariance
     * @param floats — Input Floats
     * @returns result — Population variance of every float in an array
     * @returns empty — True when the input array held no values
     */
    function variance({ floats: float[] }): { result: float, empty: bool };

    // === Math/Float/Comparison ===

    /**
     * Checks if two floats are equal (within a tolerance)
     * @node float_equal @receiver float1 @alias floatEqual
     * @param float1 — First Float (receiver: `this` in `x.equal(...)`)
     * @param float2 — Second Float
     * @param tolerance (optional) — Comparison Tolerance
     * @returns isEqual — True if the floats are equal, false otherwise
     */
    function equal(this: float, { float1: float, float2: float, tolerance?: float }): bool;

    /**
     * Checks if one float is greater than another
     * @node float_greater_than @receiver float1 @alias floatGreaterThan
     * @param float1 — First Float (receiver: `this` in `x.greaterThan(...)`)
     * @param float2 — Second Float
     * @returns isGreater — True if float1 is greater than float2, false otherwise
     */
    function greaterThan(this: float, { float1: float, float2: float }): bool;

    /**
     * Checks if one float is greater than or equal to another
     * @node float_greater_than_or_equal @receiver float1 @alias floatGreaterThanOrEqual
     * @param float1 — First Float (receiver: `this` in `x.greaterThanOrEqual(...)`)
     * @param float2 — Second Float
     * @returns isGreaterOrEqual — True if float1 is greater than or equal to float2, false otherwise
     */
    function greaterThanOrEqual(this: float, { float1: float, float2: float }): bool;

    /**
     * True when the value is a real, finite number
     * @node float_is_finite @receiver float @alias floatIsFinite
     * @param float — Input Float (receiver: `this` in `x.isFinite(...)`)
     * @returns result — True when the value is a real, finite number
     */
    function isFinite(this: float, { float: float }): bool;

    /**
     * True when the value is positive or negative infinity
     * @node float_is_infinite @receiver float @alias floatIsInfinite
     * @param float — Input Float (receiver: `this` in `x.isInfinite(...)`)
     * @returns result — True when the value is positive or negative infinity
     */
    function isInfinite(this: float, { float: float }): bool;

    /**
     * True when the value is missing or not a real number
     * @node float_is_nan @receiver float @alias floatIsNan
     * @param float — Input Float (receiver: `this` in `x.isNan(...)`)
     * @returns result — True when the value is missing or not a real number
     */
    function isNan(this: float, { float: float }): bool;

    /**
     * Checks if one float is less than another
     * @node float_less_than @receiver float1 @alias floatLessThan
     * @param float1 — First Float (receiver: `this` in `x.lessThan(...)`)
     * @param float2 — Second Float
     * @returns isLess — True if float1 is less than float2, false otherwise
     */
    function lessThan(this: float, { float1: float, float2: float }): bool;

    /**
     * Checks if one float is less than or equal to another
     * @node float_less_than_or_equal @receiver float1 @alias floatLessThanOrEqual
     * @param float1 — First Float (receiver: `this` in `x.lessThanOrEqual(...)`)
     * @param float2 — Second Float
     * @returns isLessOrEqual — True if float1 is less than or equal to float2, false otherwise
     */
    function lessThanOrEqual(this: float, { float1: float, float2: float }): bool;

    /**
     * Checks if two floats are unequal (within a tolerance)
     * @node float_unequal @receiver float1 @alias floatUnequal
     * @param float1 — First Float (receiver: `this` in `x.unequal(...)`)
     * @param float2 — Second Float
     * @param tolerance (optional) — Comparison Tolerance
     * @returns isUnequal — True if the floats are unequal, false otherwise
     */
    function unequal(this: float, { float1: float, float2: float, tolerance?: float }): bool;

    // === Math/Float/Random ===

    /**
     * Generates a random float within a specified range
     * @node float_random_in_range @alias floatRandomInRange
     * @param min — Minimum Value
     * @param max — Maximum Value
     * @returns randomFloat — The generated random float
     */
    function randomInRange({ min: float, max: float }): float;

    // === Math/Float/Trigonometry ===

    /**
     * Arc cosine in radians, input must be between -1 and 1
     * @node float_acos @receiver float @alias floatAcos
     * @param float — Input Float (receiver: `this` in `x.acos(...)`)
     * @returns result — Arc cosine in radians, input must be between -1 and 1
     */
    function acos(this: float, { float: float }): float;

    /**
     * Arc sine in radians, input must be between -1 and 1
     * @node float_asin @receiver float @alias floatAsin
     * @param float — Input Float (receiver: `this` in `x.asin(...)`)
     * @returns result — Arc sine in radians, input must be between -1 and 1
     */
    function asin(this: float, { float: float }): float;

    /**
     * Arc tangent in radians
     * @node float_atan @receiver float @alias floatAtan
     * @param float — Input Float (receiver: `this` in `x.atan(...)`)
     * @returns result — Arc tangent in radians
     */
    function atan(this: float, { float: float }): float;

    /**
     * Angle in radians between the positive x axis and the point (x, y)
     * @node float_atan2 @receiver float1 @alias floatAtan2
     * @param float1 — Input Float (receiver: `this` in `x.atan2(...)`)
     * @param float2 — Input Float
     * @returns result — Angle in radians between the positive x axis and the point (x, y)
     */
    function atan2(this: float, { float1: float, float2: float }): float;

    /**
     * Cosine of an angle in radians
     * @node float_cos @receiver float @alias floatCos
     * @param float — Input Float (receiver: `this` in `x.cos(...)`)
     * @returns result — Cosine of an angle in radians
     */
    function cos(this: float, { float: float }): float;

    /**
     * Hyperbolic cosine
     * @node float_cosh @receiver float @alias floatCosh
     * @param float — Input Float (receiver: `this` in `x.cosh(...)`)
     * @returns result — Hyperbolic cosine
     */
    function cosh(this: float, { float: float }): float;

    /**
     * Sine of an angle in radians
     * @node float_sin @receiver float @alias floatSin
     * @param float — Input Float (receiver: `this` in `x.sin(...)`)
     * @returns result — Sine of an angle in radians
     */
    function sin(this: float, { float: float }): float;

    /**
     * Hyperbolic sine
     * @node float_sinh @receiver float @alias floatSinh
     * @param float — Input Float (receiver: `this` in `x.sinh(...)`)
     * @returns result — Hyperbolic sine
     */
    function sinh(this: float, { float: float }): float;

    /**
     * Tangent of an angle in radians
     * @node float_tan @receiver float @alias floatTan
     * @param float — Input Float (receiver: `this` in `x.tan(...)`)
     * @returns result — Tangent of an angle in radians
     */
    function tan(this: float, { float: float }): float;

    /**
     * Hyperbolic tangent
     * @node float_tanh @receiver float @alias floatTanh
     * @param float — Input Float (receiver: `this` in `x.tanh(...)`)
     * @returns result — Hyperbolic tangent
     */
    function tanh(this: float, { float: float }): float;

    /**
     * Converts radians into degrees
     * @node float_to_degrees @receiver float @alias floatToDegrees
     * @param float — Input Float (receiver: `this` in `x.toDegrees(...)`)
     * @returns result — Converts radians into degrees
     */
    function toDegrees(this: float, { float: float }): float;

    /**
     * Converts degrees into radians
     * @node float_to_radians @receiver float @alias floatToRadians
     * @param float — Input Float (receiver: `this` in `x.toRadians(...)`)
     * @returns result — Converts degrees into radians
     */
    function toRadians(this: float, { float: float }): float;
}

declare namespace fmt {
    // === Utils/Format ===

    /**
     * Turns a byte count into a readable size such as 1.4 MB
     * @node format_bytes @alias formatBytes
     * @param bytes (optional) — Number of bytes
     * @param standard (optional) — Decimal counts in 1000s (MB), Binary in 1024s (MiB)
     * @param decimals (optional) — Decimal places to keep
     * @returns text — The readable size
     * @returns unit — The unit that was chosen
     */
    function bytes({ bytes?: int, standard?: string, decimals?: int }): { text: string, unit: string };

    /**
     * Writes a number of seconds as a readable duration such as 2h 15m
     * @node format_duration @alias formatDuration
     * @param seconds (optional) — Length of the duration in seconds
     * @param style (optional) — Short writes 2h 15m, Long writes 2 hours 15 minutes, Clock writes 02:15:00
     * @param maxParts (optional) — How many units to show before stopping, for example 2 gives 2h 15m instead of 2h 15m 3s
     * @returns text — The readable duration
     */
    function duration({ seconds?: float, style?: string, maxParts?: int }): string;

    /**
     * Renders a number for display with fixed decimals and separators
     * @node format_number @alias formatNumber
     * @param value — Number to format
     * @param decimals (optional) — Decimal places to keep
     * @param thousands (optional) — Inserted every three digits, empty for none
     * @param decimalPoint (optional) — Character between the whole and fractional part
     * @param prefix (optional) — Put in front, for example a currency symbol
     * @param suffix (optional) — Appended, for example a unit
     * @param asPercent (optional) — Multiply by 100 and append a percent sign
     * @returns text — The formatted number
     */
    function number({ value: float, decimals?: int, thousands?: string, decimalPoint?: string, prefix?: string, suffix?: string, asPercent?: bool }): string;

    /**
     * Writes a number as 1st, 2nd, 3rd and so on
     * @node format_ordinal @receiver value @alias formatOrdinal
     * @param value (optional) — Number to write (receiver: `this` in `x.ordinal(...)`)
     * @returns text — The ordinal
     * @returns suffix — Just the two letter suffix
     */
    function ordinal(this: int, { value?: int }): { text: string, suffix: string };
}

declare namespace hash {
    // === Utils/Hash ===

    /**
     * Computes the AHash of the input
     * @node utils_hash_ahash @receiver input @alias utilsHashAhash
     * @param input — Input data to hash (receiver: `this` in `x.ahash(...)`)
     * @param consistent (optional) — Use consistent hashing
     * @param seed (optional) — Seed value for consistent hashing
     * @returns hash — AHash of the input
     * @impure has side effects / drives control flow
     */
    function ahash(this: any, { input: any, consistent?: bool, seed?: int }): int;

    /**
     * Computes the Blake3 hash of the input
     * @node utils_hash_blake3 @receiver input @alias utilsHashBlake3
     * @param input — Input data to hash (receiver: `this` in `x.blake3(...)`)
     * @returns hash — Blake3 hash of the input
     * @impure has side effects / drives control flow
     */
    function blake3(this: any, { input: any }): string;

    /**
     * Computes the MD5 hash of the input string. Note: MD5 is not collision-resistant — use SHA-256 or Blake3 for security-sensitive hashing.
     * @node utils_hash_md5 @receiver input @alias utilsHashMd5
     * @param input — String to hash (receiver: `this` in `x.md5(...)`)
     * @returns hash — MD5 hash as hex string
     * @impure has side effects / drives control flow
     */
    function md5(this: string, { input: string }): string;

    /**
     * Computes the SHA-256 hash of the input string
     * @node utils_hash_sha256 @receiver input @alias utilsHashSha256
     * @param input — String to hash (receiver: `this` in `x.sha256(...)`)
     * @returns hash — SHA-256 hash as hex string
     * @impure has side effects / drives control flow
     */
    function sha256(this: string, { input: string }): string;

    /**
     * Computes the SHA-512 hash of the input string
     * @node utils_hash_sha512 @receiver input @alias utilsHashSha512
     * @param input — String to hash (receiver: `this` in `x.sha512(...)`)
     * @returns hash — SHA-512 hash as hex string
     * @impure has side effects / drives control flow
     */
    function sha512(this: string, { input: string }): string;
}

declare namespace int {
    // === Math/Int ===

    /**
     * Returns the absolute value of an Integer
     * @node int_abs @receiver integer @alias intAbs
     * @param integer — Input Integer (receiver: `this` in `x.abs(...)`)
     * @returns absolute — Absolute Value
     */
    function abs(this: int, { integer: int }): int;

    /**
     * Adds two Integers
     * @node int_add @receiver integer1 @alias intAdd
     * @param integer1 — Input Integer (receiver: `this` in `x.add(...)`)
     * @param integer2 — Input Integer
     * @returns sum — Sum of the two integers
     */
    function add(this: int, { integer1: int, integer2: int }): int;

    /**
     * Clamps an integer within a range
     * @node int_clamp @receiver integer @alias intClamp
     * @param integer — Input Integer (receiver: `this` in `x.clamp(...)`)
     * @param min — Minimum Value
     * @param max — Maximum Value
     * @returns clamped — Clamped Value
     */
    function clamp(this: int, { integer: int, min: int, max: int }): int;

    /**
     * Decreases an integer by a step
     * @node int_decrement @receiver integer @alias intDecrement
     * @param integer — Input Integer (receiver: `this` in `x.decrement(...)`)
     * @param step (optional) — Step width
     * @returns result — Decreases an integer by a step
     */
    function decrement(this: int, { integer: int, step?: int }): int;

    /**
     * Divides two integers and truncates towards zero
     * @node int_div @receiver integer1 @alias intDiv
     * @param integer1 — Dividend (receiver: `this` in `x.div(...)`)
     * @param integer2 — Divisor
     * @returns result — Truncated quotient
     * @returns success — False when the divisor was zero
     */
    function div(this: int, { integer1: int, integer2: int }): { result: int, success: bool };

    /**
     * Divides two integers and rounds towards negative infinity
     * @node int_div_euclid @receiver integer1 @alias intDivEuclid
     * @param integer1 — Dividend (receiver: `this` in `x.divEuclid(...)`)
     * @param integer2 — Divisor
     * @returns result — Euclidean quotient
     * @returns success — False when the divisor was zero
     */
    function divEuclid(this: int, { integer1: int, integer2: int }): { result: int, success: bool };

    /**
     * Divides two Integers (handles division by zero)
     * @node int_divide @receiver integer1 @alias intDivide
     * @param integer1 — Dividend (receiver: `this` in `x.divide(...)`)
     * @param integer2 — Divisor
     * @returns result — Result of the division
     */
    function divide(this: int, { integer1: int, integer2: int }): float;

    /**
     * Checks if two integers are equal
     * @node int_equal @receiver integer1 @alias intEqual
     * @param integer1 — Input Integer (receiver: `this` in `x.equal(...)`)
     * @param integer2 — Input Integer
     * @returns equal — True if the integers are equal, false otherwise
     */
    function equal(this: int, { integer1: int, integer2: int }): bool;

    /**
     * Parses an integer from binary, octal, decimal or hexadecimal text
     * @node int_from_radix @alias intFromRadix
     * @param string — Text to parse
     * @param radix (optional) — Numeric base
     * @returns integer — The parsed integer
     * @returns success — True when the text was a valid number in that base
     */
    function fromRadix({ string: string, radix?: string }): { integer: int, success: bool };

    /**
     * Largest integer that divides both inputs
     * @node int_gcd @receiver integer1 @alias intGcd
     * @param integer1 — Input Integer (receiver: `this` in `x.gcd(...)`)
     * @param integer2 — Input Integer
     * @returns result — Largest integer that divides both inputs
     */
    function gcd(this: int, { integer1: int, integer2: int }): int;

    /**
     * Checks if the first integer is greater than the second
     * @node int_greater_than @receiver integer1 @alias intGreaterThan
     * @param integer1 — Input Integer (receiver: `this` in `x.greaterThan(...)`)
     * @param integer2 — Input Integer
     * @returns greaterThan — True if integer1 > integer2, false otherwise
     */
    function greaterThan(this: int, { integer1: int, integer2: int }): bool;

    /**
     * Checks if the first integer is greater than or equal to the second
     * @node int_greater_than_or_equal @receiver integer1 @alias intGreaterThanOrEqual
     * @param integer1 — Input Integer (receiver: `this` in `x.greaterThanOrEqual(...)`)
     * @param integer2 — Input Integer
     * @returns greaterThanOrEqual — True if integer1 >= integer2, false otherwise
     */
    function greaterThanOrEqual(this: int, { integer1: int, integer2: int }): bool;

    /**
     * Increases an integer by a step
     * @node int_increment @receiver integer @alias intIncrement
     * @param integer — Input Integer (receiver: `this` in `x.increment(...)`)
     * @param step (optional) — Step width
     * @returns result — Increases an integer by a step
     */
    function increment(this: int, { integer: int, step?: int }): int;

    /**
     * Checks whether an integer is divisible by two
     * @node int_is_even @receiver integer @alias intIsEven
     * @param integer — Input Integer (receiver: `this` in `x.isEven(...)`)
     * @returns result — Checks whether an integer is divisible by two
     */
    function isEven(this: int, { integer: int }): bool;

    /**
     * Checks whether an integer is less than zero
     * @node int_is_negative @receiver integer @alias intIsNegative
     * @param integer — Input Integer (receiver: `this` in `x.isNegative(...)`)
     * @returns result — Checks whether an integer is less than zero
     */
    function isNegative(this: int, { integer: int }): bool;

    /**
     * Checks whether an integer is not divisible by two
     * @node int_is_odd @receiver integer @alias intIsOdd
     * @param integer — Input Integer (receiver: `this` in `x.isOdd(...)`)
     * @returns result — Checks whether an integer is not divisible by two
     */
    function isOdd(this: int, { integer: int }): bool;

    /**
     * Checks whether an integer is greater than zero
     * @node int_is_positive @receiver integer @alias intIsPositive
     * @param integer — Input Integer (receiver: `this` in `x.isPositive(...)`)
     * @returns result — Checks whether an integer is greater than zero
     */
    function isPositive(this: int, { integer: int }): bool;

    /**
     * Smallest positive integer that both inputs divide
     * @node int_lcm @receiver integer1 @alias intLcm
     * @param integer1 — Input Integer (receiver: `this` in `x.lcm(...)`)
     * @param integer2 — Input Integer
     * @returns result — Smallest positive integer that both inputs divide
     */
    function lcm(this: int, { integer1: int, integer2: int }): int;

    /**
     * Checks if the first integer is less than the second
     * @node int_less_than @receiver integer1 @alias intLessThan
     * @param integer1 — Input Integer (receiver: `this` in `x.lessThan(...)`)
     * @param integer2 — Input Integer
     * @returns lessThan — True if integer1 < integer2, false otherwise
     */
    function lessThan(this: int, { integer1: int, integer2: int }): bool;

    /**
     * Checks if the first integer is less than or equal to the second
     * @node int_less_than_or_equal @receiver integer1 @alias intLessThanOrEqual
     * @param integer1 — Input Integer (receiver: `this` in `x.lessThanOrEqual(...)`)
     * @param integer2 — Input Integer
     * @returns lessThanOrEqual — True if integer1 <= integer2, false otherwise
     */
    function lessThanOrEqual(this: int, { integer1: int, integer2: int }): bool;

    /**
     * The smallest and largest representable integer
     * @node int_limits @alias intLimits
     * @returns min — Smallest representable integer
     * @returns max — Largest representable integer
     */
    function limits(): { min: int, max: int };

    /**
     * Returns the larger of two integers
     * @node int_max @receiver integer1 @alias intMax
     * @param integer1 — Input Integer (receiver: `this` in `x.max(...)`)
     * @param integer2 — Input Integer
     * @returns maximum — The larger of the two integers
     */
    function max(this: int, { integer1: int, integer2: int }): int;

    /**
     * Returns the smaller of two integers
     * @node int_min @receiver integer1 @alias intMin
     * @param integer1 — Input Integer (receiver: `this` in `x.min(...)`)
     * @param integer2 — Input Integer
     * @returns minimum — The smaller of the two integers
     */
    function min(this: int, { integer1: int, integer2: int }): int;

    /**
     * Calculates the remainder of integer division
     * @node int_modulo @receiver integer1 @alias intModulo
     * @param integer1 — Dividend (receiver: `this` in `x.modulo(...)`)
     * @param integer2 — Divisor
     * @returns remainder — Remainder of the division
     */
    function modulo(this: int, { integer1: int, integer2: int }): int;

    /**
     * Multiplies two Integers
     * @node int_multiply @receiver integer1 @alias intMultiply
     * @param integer1 — Input Integer (receiver: `this` in `x.multiply(...)`)
     * @param integer2 — Input Integer
     * @returns product — Product of the two integers
     */
    function multiply(this: int, { integer1: int, integer2: int }): int;

    /**
     * Flips the sign of an integer
     * @node int_negate @receiver integer @alias intNegate
     * @param integer — Input Integer (receiver: `this` in `x.negate(...)`)
     * @returns result — Flips the sign of an integer
     */
    function negate(this: int, { integer: int }): int;

    /**
     * Calculates the power of an integer
     * @node int_power @receiver base @alias intPower
     * @param base — Base integer (receiver: `this` in `x.power(...)`)
     * @param exponent — Exponent integer
     * @returns power — Result of the power calculation
     */
    function power(this: int, { base: int, exponent: int }): int;

    /**
     * Remainder that is always positive, unlike the % operator
     * @node int_rem_euclid @receiver integer1 @alias intRemEuclid
     * @param integer1 — Dividend (receiver: `this` in `x.remEuclid(...)`)
     * @param integer2 — Divisor
     * @returns result — Non-negative remainder
     * @returns success — False when the divisor was zero
     */
    function remEuclid(this: int, { integer1: int, integer2: int }): { result: int, success: bool };

    /**
     * Calculates the nth root of an integer
     * @node int_root @receiver radicand @alias intRoot
     * @param radicand — The integer to take the root of (receiver: `this` in `x.root(...)`)
     * @param degree — The degree of the root
     * @returns root — Result of the root calculation
     */
    function root(this: int, { radicand: int, degree: int }): float;

    /**
     * Returns -1, 0 or 1 depending on the sign of an integer
     * @node int_signum @receiver integer @alias intSignum
     * @param integer — Input Integer (receiver: `this` in `x.signum(...)`)
     * @returns result — Returns -1, 0 or 1 depending on the sign of an integer
     */
    function signum(this: int, { integer: int }): int;

    /**
     * Subtracts two Integers
     * @node int_subtract @receiver integer1 @alias intSubtract
     * @param integer1 — Minuend (receiver: `this` in `x.subtract(...)`)
     * @param integer2 — Subtrahend
     * @returns difference — Difference of the two integers
     */
    function subtract(this: int, { integer1: int, integer2: int }): int;

    /**
     * Converts an integer into a float
     * @node int_to_float @receiver integer @alias intToFloat
     * @param integer — Input Integer (receiver: `this` in `x.toFloat(...)`)
     * @returns float — The converted value
     */
    function toFloat(this: int, { integer: int }): float;

    /**
     * Formats an integer as binary, octal, decimal or hexadecimal text
     * @node int_to_radix @receiver integer @alias intToRadix
     * @param integer — Input Integer (receiver: `this` in `x.toRadix(...)`)
     * @param radix (optional) — Numeric base
     * @param uppercase (optional) — Use upper case letters for hexadecimal digits
     * @returns string — The formatted number
     */
    function toRadix(this: int, { integer: int, radix?: string, uppercase?: bool }): string;

    /**
     * Checks if two integers are unequal
     * @node int_unequal @receiver integer1 @alias intUnequal
     * @param integer1 — Input Integer (receiver: `this` in `x.unequal(...)`)
     * @param integer2 — Input Integer
     * @returns unequal — True if the integers are unequal, false otherwise
     */
    function unequal(this: int, { integer1: int, integer2: int }): bool;

    // === Math/Int/Aggregate ===

    /**
     * Arithmetic mean of every integer in an array
     * @node int_average @alias intAverage
     * @param integers — Input Integers
     * @returns result — Arithmetic mean of every integer in an array
     * @returns empty — True when the input array held no values
     */
    function average({ integers: int[] }): { result: float, empty: bool };

    /**
     * Largest integer in an array
     * @node int_max_of @alias intMaxOf
     * @param integers — Input Integers
     * @returns result — Largest integer in an array
     * @returns empty — True when the input array held no values
     */
    function maxOf({ integers: int[] }): { result: int, empty: bool };

    /**
     * Smallest integer in an array
     * @node int_min_of @alias intMinOf
     * @param integers — Input Integers
     * @returns result — Smallest integer in an array
     * @returns empty — True when the input array held no values
     */
    function minOf({ integers: int[] }): { result: int, empty: bool };

    /**
     * Multiplies every integer in an array
     * @node int_product @alias intProduct
     * @param integers — Input Integers
     * @returns result — Multiplies every integer in an array
     * @returns empty — True when the input array held no values
     */
    function product({ integers: int[] }): { result: int, empty: bool };

    /**
     * Adds up every integer in an array
     * @node int_sum @alias intSum
     * @param integers — Input Integers
     * @returns result — Adds up every integer in an array
     * @returns empty — True when the input array held no values
     */
    function sum({ integers: int[] }): { result: int, empty: bool };

    // === Math/Int/Bitwise ===

    /**
     * Bitwise AND of two integers
     * @node int_bitand @receiver integer1 @alias intBitand
     * @param integer1 — Input Integer (receiver: `this` in `x.bitand(...)`)
     * @param integer2 — Input Integer
     * @returns result — Bitwise AND of two integers
     */
    function bitand(this: int, { integer1: int, integer2: int }): int;

    /**
     * Inverts every bit of an integer
     * @node int_bitnot @receiver integer @alias intBitnot
     * @param integer — Input Integer (receiver: `this` in `x.bitnot(...)`)
     * @returns result — The integer with all bits inverted
     */
    function bitnot(this: int, { integer: int }): int;

    /**
     * Bitwise OR of two integers
     * @node int_bitor @receiver integer1 @alias intBitor
     * @param integer1 — Input Integer (receiver: `this` in `x.bitor(...)`)
     * @param integer2 — Input Integer
     * @returns result — Bitwise OR of two integers
     */
    function bitor(this: int, { integer1: int, integer2: int }): int;

    /**
     * Bitwise XOR of two integers
     * @node int_bitxor @receiver integer1 @alias intBitxor
     * @param integer1 — Input Integer (receiver: `this` in `x.bitxor(...)`)
     * @param integer2 — Input Integer
     * @returns result — Bitwise XOR of two integers
     */
    function bitxor(this: int, { integer1: int, integer2: int }): int;

    /**
     * Number of bits that are set to one
     * @node int_count_ones @receiver integer @alias intCountOnes
     * @param integer — Input Integer (receiver: `this` in `x.countOnes(...)`)
     * @returns result — Number of bits that are set to one
     */
    function countOnes(this: int, { integer: int }): int;

    /**
     * Number of zero bits before the highest set bit
     * @node int_leading_zeros @receiver integer @alias intLeadingZeros
     * @param integer — Input Integer (receiver: `this` in `x.leadingZeros(...)`)
     * @returns result — Number of zero bits before the highest set bit
     */
    function leadingZeros(this: int, { integer: int }): int;

    /**
     * Shifts the bits of an integer to the left
     * @node int_shl @receiver integer @alias intShl
     * @param integer — Input Integer (receiver: `this` in `x.shl(...)`)
     * @param shift (optional) — Number of bit positions to shift by
     * @returns result — Shifts the bits of an integer to the left
     */
    function shl(this: int, { integer: int, shift?: int }): int;

    /**
     * Shifts the bits of an integer to the right
     * @node int_shr @receiver integer @alias intShr
     * @param integer — Input Integer (receiver: `this` in `x.shr(...)`)
     * @param shift (optional) — Number of bit positions to shift by
     * @returns result — Shifts the bits of an integer to the right
     */
    function shr(this: int, { integer: int, shift?: int }): int;

    /**
     * Number of zero bits after the lowest set bit
     * @node int_trailing_zeros @receiver integer @alias intTrailingZeros
     * @param integer — Input Integer (receiver: `this` in `x.trailingZeros(...)`)
     * @returns result — Number of zero bits after the lowest set bit
     */
    function trailingZeros(this: int, { integer: int }): int;

    // === Math/Int/Overflow ===

    /**
     * Arithmetic that reports overflow and division by zero instead of failing
     * @node int_checked_op @receiver integer1 @alias intCheckedOp
     * @param integer1 — Left hand side (receiver: `this` in `x.checkedOp(...)`)
     * @param integer2 — Right hand side
     * @param operation (optional) — Arithmetic operation to apply
     * @returns result — Arithmetic that reports overflow and division by zero instead of failing
     * @returns success — False on overflow or division by zero
     */
    function checkedOp(this: int, { integer1: int, integer2: int, operation?: string }): { result: int, success: bool };

    /**
     * Arithmetic that clamps to the integer limits instead of overflowing
     * @node int_saturating_op @receiver integer1 @alias intSaturatingOp
     * @param integer1 — Left hand side (receiver: `this` in `x.saturatingOp(...)`)
     * @param integer2 — Right hand side
     * @param operation (optional) — Arithmetic operation to apply
     * @returns result — Arithmetic that clamps to the integer limits instead of overflowing
     */
    function saturatingOp(this: int, { integer1: int, integer2: int, operation?: string }): int;

    /**
     * Arithmetic that wraps around the integer limits
     * @node int_wrapping_op @receiver integer1 @alias intWrappingOp
     * @param integer1 — Left hand side (receiver: `this` in `x.wrappingOp(...)`)
     * @param integer2 — Right hand side
     * @param operation (optional) — Arithmetic operation to apply
     * @returns result — Arithmetic that wraps around the integer limits
     */
    function wrappingOp(this: int, { integer1: int, integer2: int, operation?: string }): int;

    // === Math/Int/Random ===

    /**
     * Generates a random integer within a specified range
     * @node int_random_in_range @alias intRandomInRange
     * @param min — Minimum Value
     * @param max — Maximum Value
     * @returns randomInteger — The generated random integer
     */
    function randomInRange({ min: int, max: int }): int;
}

declare namespace json {
    // === Utils/Conversions ===

    /**
     * Convert String to Bytes
     * @node val_from_bytes @alias valFromBytes
     * @param bytes — Bytes to convert
     * @returns value — Parsed Value
     */
    function fromBytes({ bytes: bytes[] }): any;

    /**
     * Convert String to Struct
     * @node val_from_string @alias valFromString
     * @param string — String to convert
     * @returns valueRef — Value of the Generic
     */
    function parse({ string: string }): any;

    /**
     * Convert any object to String
     * @node val_to_string @alias valToString
     * @param value — Input Value
     * @param pretty (optional) — Should the struct be pretty printed?
     * @returns string — Output String
     */
    function stringify({ value: any, pretty?: bool }): string;

    /**
     * Convert Struct to Bytes
     * @node val_to_bytes @alias valToBytes
     * @param value — Input Value
     * @param pretty (optional) — Should the struct be pretty printed?
     * @returns bytes — Output Bytes
     */
    function toBytes({ value: any, pretty?: bool }): bytes[];

    // === Utils/JSON ===

    /**
     * Generate Tool Definitions for Tool Calls
     * @node utils_json_make_schema @alias utilsJsonMakeSchema
     * @param exampleJson — Example JSON to infer schema from
     * @returns schema — Generated JSON Schema / Tool Definition
     * @impure has side effects / drives control flow
     */
    function makeSchema({ exampleJson: string }): Struct;

    /**
     * Parse JSON input Data With JSON/OpenAI Schema and Return Value
     * @node parse_with_schema @alias parseWithSchema
     * @param schema — JSON Schema or OpenAI Function Definition
     * @param data — JSON Input Data to be parsed
     * @returns parsed — Parsed and Validated JSON
     * @impure has side effects / drives control flow
     */
    function parseWithSchema({ schema: string, data: string }): Struct;

    /**
     * Attempts to repair and parse potentially malformed JSON
     * @node repair_parse @alias repairParse
     * @param jsonString — String containing potentially malformed JSON
     * @returns result — The parsed JSON structure
     * @impure has side effects / drives control flow
     */
    function repairParse({ jsonString: string }): Struct;
}

declare namespace log {
    // === Logging ===

    /**
     * Logs / Prints an Error
     * @node log_error @alias logError
     * @param message (optional) — Print Error Message
     * @param toast (optional) — Should the user see a toast popping up?
     * @impure has side effects / drives control flow
     */
    function error({ message?: any, toast?: bool }): void;

    /**
     * Print Debugging Information
     * @node log_info @alias logInfo
     * @param message (optional) — The message to log
     * @param toast (optional) — Should the user see a toast popping up?
     * @impure has side effects / drives control flow
     */
    function info({ message?: any, toast?: bool }): void;

    /**
     * Shows a progress toast to the user that can be updated
     * @node log_progress @alias logProgress
     * @param id (optional) — Unique identifier for this progress. Use the same ID to update the progress.
     * @param message (optional) — The message shown to the user
     * @param progress (optional) — Progress value between 0 and 100. Leave empty to show indeterminate progress.
     * @impure has side effects / drives control flow
     */
    function progress({ id?: string, message?: string, progress?: int }): void;

    /**
     * Completes a progress toast with a success or error state
     * @node log_progress_done @alias logProgressDone
     * @param id (optional) — The ID of the progress toast to complete (must match the ID used in Show Progress)
     * @param message (optional) — Final message to show (e.g., 'Completed!' or 'Failed')
     * @param success (optional) — Whether the operation was successful (true shows success toast, false shows error)
     * @impure has side effects / drives control flow
     */
    function progressDone({ id?: string, message?: string, success?: bool }): void;

    /**
     * Logs a Warning
     * @node log_warning @alias logWarning
     * @param message (optional) — Print Warning
     * @param toast (optional) — Should the user see a toast popping up?
     * @impure has side effects / drives control flow
     */
    function warn({ message?: any, toast?: bool }): void;
}

declare namespace map {
    // === Utils/Map ===

    /**
     * Removes all entries from a map
     * @node map_clear @receiver map_in @alias mapClear
     * @param mapIn — Your Map (receiver: `this` in `x.clear(...)`)
     * @returns mapOut — Empty Map
     * @impure has side effects / drives control flow
     */
    function clear(this: Map<string, any>, { mapIn: Map<string, any> }): Map<string, any>;

    /**
     * Gets a value from a map by key
     * @node map_get @receiver map_in @alias mapGet
     * @param mapIn — Your Map (receiver: `this` in `x.get(...)`)
     * @param key — Key to get
     * @returns value — Value at the specified key
     * @returns found — Was the key found in the map?
     */
    function get(this: Map<string, any>, { mapIn: Map<string, any>, key: string }): { value: any, found: bool };

    /**
     * Checks if a key exists in the map
     * @node map_has_key @receiver map_in @alias mapHasKey
     * @param mapIn — Your Map (receiver: `this` in `x.has(...)`)
     * @param key — Key to check
     * @returns hasKey — Does the map contain the key?
     */
    function has(this: Map<string, any>, { mapIn: Map<string, any>, key: string }): bool;

    /**
     * Gets all keys from the map as an array
     * @node map_keys @receiver map_in @alias mapKeys
     * @param mapIn — Your Map (receiver: `this` in `x.keys(...)`)
     * @returns keys — Array of all keys
     */
    function keys(this: Map<string, any>, { mapIn: Map<string, any> }): any[];

    /**
     * Creates an empty map (string keys)
     * @node make_map @alias makeMap
     * @returns mapOut — The created map
     */
    function make(): Map<string, any>;

    /**
     * Removes a key from the map
     * @node map_remove @receiver map_in @alias mapRemove
     * @param mapIn — Your Map (receiver: `this` in `x.remove(...)`)
     * @param key — Key to remove
     * @returns mapOut — Adjusted Map
     * @returns value — The removed value (null if key not found)
     * @returns wasPresent — Was the key in the map?
     * @impure has side effects / drives control flow
     */
    function remove(this: Map<string, any>, { mapIn: Map<string, any>, key: string }): { mapOut: Map<string, any>, value: any, wasPresent: bool };

    /**
     * Sets a value in a map at the given key
     * @node map_set @receiver map_in @alias mapSet
     * @param mapIn — Your Map (receiver: `this` in `x.set(...)`)
     * @param key — Key to set
     * @param value — Value to set
     * @returns mapOut — Adjusted Map
     * @returns replaced — Was an existing value replaced?
     * @impure has side effects / drives control flow
     */
    function set(this: Map<string, any>, { mapIn: Map<string, any>, key: string, value: any }): { mapOut: Map<string, any>, replaced: bool };

    /**
     * Gets the number of entries in the map
     * @node map_size @receiver map_in @alias mapSize
     * @param mapIn — Your Map (receiver: `this` in `x.size(...)`)
     * @returns size — Number of entries in the map
     */
    function size(this: Map<string, any>, { mapIn: Map<string, any> }): int;

    /**
     * Gets all values from the map as an array
     * @node map_values @receiver map_in @alias mapValues
     * @param mapIn — Your Map (receiver: `this` in `x.values(...)`)
     * @returns values — Array of all values
     */
    function values(this: Map<string, any>, { mapIn: Map<string, any> }): any[];

    // === Utils/Map/By Reference ===

    /**
     * Clear all entries directly from a variable map without copying.
     * @node map_clear_ref @alias mapClearRef
     * @param varRef — Reference to the map variable to clear
     * @impure has side effects / drives control flow
     */
    function clearRef({ varRef: string }): void;

    /**
     * Remove a key directly from a variable map without copying. Much faster for large maps.
     * @node map_remove_ref @alias mapRemoveRef
     * @param varRef — Reference to the map variable to modify
     * @param key — Key to remove
     * @returns value — The removed value (null if key not found)
     * @returns wasPresent — Was the key in the map?
     * @impure has side effects / drives control flow
     */
    function removeRef({ varRef: string, key: string }): { value: any, wasPresent: bool };

    /**
     * Set a value directly in a variable map without copying. Much faster for large maps.
     * @node map_set_ref @alias mapSetRef
     * @param varRef — Reference to the map variable to modify
     * @param key — Key to set
     * @param value — Value to set at the key
     * @impure has side effects / drives control flow
     */
    function setRef({ varRef: string, key: string, value: any }): void;
}

declare namespace math {
    // === Math ===

    /**
     * Evaluates a mathematical expression
     * @node eval @alias eval
     * @param expression — Mathematical expression
     * @returns result — Result of the expression
     */
    function eval({ expression: string }): float;

    namespace vector {
        // === Utils/Math/Vector ===

        /**
         * Adds two float vectors together element-wise
         * @node float_vector_addition @alias floatVectorAddition
         * @param vector1 — First float vector
         * @param vector2 — Second float vector
         * @returns resultVector — Sum of the two vectors
         */
        function add({ vector1: float[], vector2: float[] }): float[];

        /**
         * Calculates the cosine similarity of two float vectors
         * @node float_vector_cosine_similarity @receiver vector1 @alias floatVectorCosineSimilarity
         * @param vector1 — First float vector (receiver: `this` in `x.cosineSimilarity(...)`)
         * @param vector2 — Second float vector
         * @returns similarity — Cosine similarity of the two vectors
         */
        function cosineSimilarity(this: float[], { vector1: float[], vector2: float[] }): float;

        /**
         * Calculates the cross product of two float vectors
         * @node float_vector_cross_product @receiver vector1 @alias floatVectorCrossProduct
         * @param vector1 — First float vector (receiver: `this` in `x.cross(...)`)
         * @param vector2 — Second float vector
         * @returns resultVector — Cross product of the two vectors
         */
        function cross(this: float[], { vector1: float[], vector2: float[] }): float[];

        /**
         * Calculates the dot product of two float vectors
         * @node float_vector_dot_product @receiver vector1 @alias floatVectorDotProduct
         * @param vector1 — First float vector (receiver: `this` in `x.dot(...)`)
         * @param vector2 — Second float vector
         * @returns result — Dot product of the two vectors
         */
        function dot(this: float[], { vector1: float[], vector2: float[] }): float;

        /**
         * Multiplies two float vectors element-wise
         * @node float_vector_multiplication @alias floatVectorMultiplication
         * @param vector1 — First float vector
         * @param vector2 — Second float vector
         * @returns resultVector — Element-wise product of the two vectors
         */
        function multiply({ vector1: float[], vector2: float[] }): float[];

        /**
         * Normalizes a float vector
         * @node float_vector_normalize @receiver vector @alias floatVectorNormalize
         * @param vector — Float vector to normalize (receiver: `this` in `x.normalize(...)`)
         * @returns normalizedVector — Normalized float vector
         */
        function normalize(this: float[], { vector: float[] }): float[];

        /**
         * Subtracts one float vector from another element-wise
         * @node float_vector_subtraction @alias floatVectorSubtraction
         * @param vector1 — First float vector
         * @param vector2 — Second float vector
         * @returns resultVector — Element-wise difference of the two vectors
         */
        function subtract({ vector1: float[], vector2: float[] }): float[];
    }
}

declare namespace md {
    // === Utils/Markdown ===

    /**
     * Attempts to convert HTML to Markdown, removing unwanted tags
     * @node utils_md_html_to_md @alias utilsMdHtmlToMd
     * @param html — Html to Parse
     * @param skippedTags (optional) — Tags to skip
     * @returns markdown — The parsed Markdown
     * @impure has side effects / drives control flow
     */
    function fromHtml({ html: string, skippedTags?: string[] }): string;

    /**
     * Converts a rich text document (plate_json) into GitHub-flavoured Markdown
     * @node utils_md_plate_to_md @alias utilsMdPlateToMd
     * @param document — Rich text document, with or without the plate_json:: prefix
     * @param images (optional) — How to render image nodes
     * @returns markdown — The converted Markdown
     * @returns media — Every image, video, audio and file reference found in the document
     * @impure has side effects / drives control flow
     */
    function fromPlate({ document: string, images?: string }): { markdown: string, media: string[] };

    /**
     * Converts a rich text document (plate_json) into HTML, keeping alignment, colours, columns and table spans that Markdown cannot express
     * @node utils_md_plate_to_html @alias utilsMdPlateToHtml
     * @param document — Rich text document, with or without the plate_json:: prefix
     * @param images (optional) — How to render image nodes
     * @param fullDocument (optional) — Wrap the output in a complete HTML document with default styling
     * @param title (optional) — Document title, used only when Full Document is enabled
     * @returns html — The converted HTML
     * @returns media — Every image, video, audio and file reference found in the document
     * @impure has side effects / drives control flow
     */
    function plateToHtml({ document: string, images?: string, fullDocument?: bool, title?: string }): { html: string, media: string[] };

    /**
     * Renders GitHub-flavoured Markdown as HTML
     * @node utils_md_md_to_html @receiver markdown @alias utilsMdMdToHtml
     * @param markdown — Markdown source to render (receiver: `this` in `x.toHtml(...)`)
     * @param allowHtml (optional) — Pass raw HTML in the source through to the output. Leave off for untrusted input.
     * @param smartPunctuation (optional) — Convert quotes, dashes and ellipses to typographic equivalents
     * @returns html — The rendered HTML
     * @impure has side effects / drives control flow
     */
    function toHtml(this: string, { markdown: string, allowHtml?: bool, smartPunctuation?: bool }): string;
}

declare namespace notify {
    // === Notifications ===

    /**
     * Send a notification to a specific user in this project
     * @node notify_project_user @alias notifyProjectUser
     * @param flowUserSub (optional) — Project user to notify
     * @param title (optional) — Notification title
     * @param description (optional) — Notification description (optional)
     * @param icon — FlowPath to a notification icon image (optional)
     * @param link (optional) — Relative path for the notification link (e.g. /dashboard or /store?item=abc)
     * @returns success — Whether the notification was sent successfully
     * @impure has side effects / drives control flow
     */
    function projectUser({ flowUserSub?: string, title?: string, description?: string, icon: Struct, link?: string }): bool;

    /**
     * Send a notification to the user who executed this workflow
     * @node notify_user @alias notifyUser
     * @param title (optional) — Notification title
     * @param description (optional) — Notification description (optional)
     * @param icon — FlowPath to a notification icon image (optional)
     * @param link (optional) — Relative path for the notification link (e.g. /dashboard or /store?item=abc)
     * @param showDesktop (optional) — Show desktop notification if available
     * @returns success — Whether the notification was sent successfully
     * @impure has side effects / drives control flow
     */
    function user({ title?: string, description?: string, icon: Struct, link?: string, showDesktop?: bool }): bool;
}

declare namespace random {
    // === Utils ===

    /**
     * Generates a Collision Resistant Unique Identifier
     * @node cuid @alias cuid
     * @returns cuid — Generated CUID
     * @impure has side effects / drives control flow
     */
    function cuid(): string;

    /**
     * A random identifier
     * @node uuid_v4 @alias uuidV4
     * @param uppercase (optional) — Write the hex digits in upper case
     * @returns uuid — A random identifier
     * @impure has side effects / drives control flow
     */
    function uuidV4({ uppercase?: bool }): string;

    /**
     * A time ordered identifier — sorts by creation time, which keeps database indexes tidy
     * @node uuid_v7 @alias uuidV7
     * @param uppercase (optional) — Write the hex digits in upper case
     * @returns uuid — A time ordered identifier — sorts by creation time, which keeps database indexes tidy
     * @impure has side effects / drives control flow
     */
    function uuidV7({ uppercase?: bool }): string;

    // === Utils/Random ===

    /**
     * Picks elements out of an array at random
     * @node random_choice @alias randomChoice
     * @param arrayIn — Your Array
     * @param count (optional) — How many elements to draw
     * @param allowRepeats (optional) — Draw with replacement, so the same element can come up twice
     * @returns element — The first drawn element
     * @returns elements — Every drawn element
     * @impure has side effects / drives control flow
     */
    function choice({ arrayIn: any[], count?: int, allowRepeats?: bool }): { element: any, elements: any[] };

    /**
     * Generates a random string, for example a token or a short code
     * @node random_string @alias randomString
     * @param length (optional) — How many characters to generate
     * @param alphabet (optional) — Characters to draw from. Unambiguous leaves out l, I, 1, O and 0
     * @param customAlphabet (optional) — Use exactly these characters instead, when set
     * @returns result — The generated string
     * @impure has side effects / drives control flow
     */
    function string({ length?: int, alphabet?: string, customAlphabet?: string }): string;
}

declare namespace set {
    // === Utils/Set ===

    /**
     * Removes / Clears all elements from a set
     * @node set_clear @receiver set_in @alias setClear
     * @param setIn — Your Set (receiver: `this` in `x.clear(...)`)
     * @returns setOut — Empty Set
     * @impure has side effects / drives control flow
     */
    function clear(this: Set<any>, { setIn: Set<any> }): Set<any>;

    /**
     * Creates a set from the difference of 2 sets
     * @node difference @receiver set_in_1 @alias difference
     * @param setIn1 — Your First Set (receiver: `this` in `x.difference(...)`)
     * @param setIn2 — Your Second Set
     * @returns setOut — The difference set
     * @impure has side effects / drives control flow
     */
    function difference(this: Set<any>, { setIn1: Set<any>, setIn2: Set<any> }): Set<any>;

    /**
     * Discards an element of a set
     * @node set_discard @receiver set_in @alias setDiscard
     * @param setIn — Your Set (receiver: `this` in `x.discard(...)`)
     * @param value — Value to remove
     * @returns setOut — Adjusted Set
     * @returns hasRemoved — If the element was removed
     * @impure has side effects / drives control flow
     */
    function discard(this: Set<any>, { setIn: Set<any>, value: any }): { setOut: Set<any>, hasRemoved: bool };

    /**
     * Checks if an element is present in the set
     * @node set_has @receiver set_in @alias setHas
     * @param setIn — Your Set (receiver: `this` in `x.has(...)`)
     * @param value — Value to search for
     * @returns contains — Does the set include the value?
     */
    function has(this: Set<any>, { setIn: Set<any>, value: any }): bool;

    /**
     * Inserts an element to the set
     * @node insert @receiver set_in @alias insert
     * @param setIn — Your Set (receiver: `this` in `x.insert(...)`)
     * @param value — Value to push
     * @returns setOut — Adjusted Set
     * @returns existedBefore — Was the element there before?
     * @impure has side effects / drives control flow
     */
    function insert(this: Set<any>, { setIn: Set<any>, value: any }): { setOut: Set<any>, existedBefore: bool };

    /**
     * Checks if a hash set is empty or not
     * @node set_is_empty @receiver set_in @alias setIsEmpty
     * @param setIn — Your Set (receiver: `this` in `x.isEmpty(...)`)
     * @returns isEmpty — Does it have any values or not?
     */
    function isEmpty(this: Set<any>, { setIn: Set<any> }): bool;

    /**
     * Checks if one of the hash sets has at least one mutual element
     * @node is_mutual @receiver set_in_1 @alias isMutual
     * @param setIn1 — (receiver: `this` in `x.isMutual(...)`)
     * @param setIn2
     * @returns isMutual — Does it include a mutual element that both sets share or not?
     * @impure has side effects / drives control flow
     */
    function isMutual(this: Set<any>, { setIn1: Set<any>, setIn2: Set<any> }): bool;

    /**
     * Checks if a hash set is a subset from a supposed bigger one
     * @node set_is_subset @receiver set_in_1 @alias setIsSubset
     * @param setIn1 — Your Smaller Set (receiver: `this` in `x.isSubset(...)`)
     * @param setIn2 — Your Bigger Set
     * @returns isSubset — Is the first set a subset of the second?
     */
    function isSubset(this: Set<any>, { setIn1: Set<any>, setIn2: Set<any> }): bool;

    /**
     * Checks if a hash set is a superset from a supposed smaller one
     * @node set_is_superset @receiver set_in_1 @alias setIsSuperset
     * @param setIn1 — Your Bigger Set (receiver: `this` in `x.isSuperset(...)`)
     * @param setIn2 — Your Smaller Set
     * @returns isSuperset — Is the first set a superset of the second?
     */
    function isSuperset(this: Set<any>, { setIn1: Set<any>, setIn2: Set<any> }): bool;

    /**
     * Creates an empty set
     * @node make_set @alias makeSet
     * @returns setOut — The created set
     */
    function make(): Set<any>;

    /**
     * Pops a random element of a set
     * @node set_pop @receiver set_in @alias setPop
     * @param setIn — Your Set (receiver: `this` in `x.pop(...)`)
     * @returns setOut — Adjusted Set
     * @impure has side effects / drives control flow
     */
    function pop(this: Set<any>, { setIn: Set<any> }): Set<any>;

    /**
     * Gets the size of the hash set (how many elements)
     * @node set_get_size @receiver set_in @alias setGetSize
     * @param setIn — Your Set (receiver: `this` in `x.size(...)`)
     * @returns size — How many elements does it have
     */
    function size(this: Set<any>, { setIn: Set<any> }): int;

    /**
     * Converts a set to an array
     * @node set_to_array @receiver set_in @alias setToArray
     * @param setIn — (receiver: `this` in `x.toArray(...)`)
     * @returns arrayOut
     */
    function toArray(this: Set<any>, { setIn: Set<any> }): any[];

    /**
     * Combines 2 sets into one unified hash set
     * @node union @receiver set_in_1 @alias union
     * @param setIn1 — Your First Set (receiver: `this` in `x.union(...)`)
     * @param setIn2 — Your Second Set
     * @returns setOut — Combined Set
     * @impure has side effects / drives control flow
     */
    function union(this: Set<any>, { setIn1: Set<any>, setIn2: Set<any> }): Set<any>;

    // === Utils/Set/By Reference ===

    /**
     * Clear all elements directly from a variable set without copying.
     * @node set_clear_ref @alias setClearRef
     * @param varRef — Reference to the set variable to clear
     * @impure has side effects / drives control flow
     */
    function clearRef({ varRef: string }): void;

    /**
     * Remove an element directly from a variable set without copying. Much faster for large sets.
     * @node set_discard_ref @alias setDiscardRef
     * @param varRef — Reference to the set variable to modify
     * @param value — Value to remove from the set
     * @returns wasPresent — True if the element was in the set and removed
     * @impure has side effects / drives control flow
     */
    function discardRef({ varRef: string, value: any }): bool;

    /**
     * Insert an element directly into a variable set without copying. Much faster for large sets.
     * @node set_insert_ref @alias setInsertRef
     * @param varRef — Reference to the set variable to modify
     * @param value — Value to insert into the set
     * @returns wasNew — True if the element was not already in the set
     * @impure has side effects / drives control flow
     */
    function insertRef({ varRef: string, value: any }): bool;
}

declare namespace string {
    // === Utils/String ===

    /**
     * Returns the character at a index. Negative indices count from the end
     * @node string_char_at @receiver string @alias stringCharAt
     * @param string — Input String (receiver: `this` in `x.charAt(...)`)
     * @param index (optional) — Character index, negative counts from the end
     * @returns character — The character at the index, empty when out of range
     * @returns found — True when the index was in range
     */
    function charAt(this: string, { string: string, index?: int }): { character: string, found: bool };

    /**
     * Appends strings to each other without a separator
     * @node string_concat @receiver string @alias stringConcat
     * @param string (optional) — Part to append (receiver: `this` in `x.concat(...)`)
     * @param string (optional) — Part to append (receiver: `this` in `x.concat(...)`)
     * @returns concatenated — All parts appended in order
     */
    function concat(this: string, { string?: string, string?: string }): string;

    /**
     * Checks if a string contains a substring
     * @node string_contains @receiver string @alias stringContains
     * @param string — Input String (receiver: `this` in `x.contains(...)`)
     * @param substring — Substring to search for
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns contains — Does the string contain the substring?
     */
    function contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;

    /**
     * Checks whether a string contains any of the given substrings
     * @node string_contains_any @receiver string @alias stringContainsAny
     * @param string — Input String (receiver: `this` in `x.containsAny(...)`)
     * @param substrings — Substrings to search for
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns contains — True when at least one substring occurs
     * @returns matched — The first substring that occurred
     */
    function containsAny(this: string, { string: string, substrings: string[], ignoreCase?: bool }): { contains: bool, matched: string };

    /**
     * Counts non-overlapping occurrences of a substring
     * @node string_count_matches @receiver string @alias stringCountMatches
     * @param string — Input String (receiver: `this` in `x.countMatches(...)`)
     * @param substring — Substring to count
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns count — Number of non-overlapping occurrences
     */
    function countMatches(this: string, { string: string, substring: string, ignoreCase?: bool }): int;

    /**
     * Shortens a string that is longer than the given number of characters and marks the cut with an ellipsis. A string that already fits is returned unchanged
     * @node string_ellipsis @receiver string @alias stringEllipsis
     * @param string — Input String (receiver: `this` in `x.ellipsis(...)`)
     * @param maxLength (optional) — Longest the result may be, counted in characters and including the ellipsis itself
     * @param ellipsis (optional) — Appended in place of what was cut
     * @returns result — The shortened string, or the input unchanged when it already fits
     */
    function ellipsis(this: string, { string: string, maxLength?: int, ellipsis?: string }): string;

    /**
     * Checks if a string ends with a specific string
     * @node string_ends_with @receiver string @alias stringEndsWith
     * @param string — Input String (receiver: `this` in `x.endsWith(...)`)
     * @param suffix — String to check against
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns endsWith — Does the string end with the suffix?
     */
    function endsWith(this: string, { string: string, suffix: string, ignoreCase?: bool }): bool;

    /**
     * Compares two Strings
     * @node equal_string @receiver string @alias equalString
     * @param string — Input (receiver: `this` in `x.equal(...)`)
     * @param string — Input (receiver: `this` in `x.equal(...)`)
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns equal — Are the strings equal?
     */
    function equal(this: string, { string: string, string: string, ignoreCase?: bool }): bool;

    /**
     * Escapes special characters in a string (newlines, tabs, carriage returns, backslashes, quotes).
     * @node string_escape @receiver string @alias stringEscape
     * @param string — Input String (receiver: `this` in `x.escape(...)`)
     * @returns escaped — String with special characters escaped
     */
    function escape(this: string, { string: string }): string;

    /**
     * Pulls every email, link, number or handle out of a text
     * @node string_extract @receiver string @alias stringExtract
     * @param string — Input String (receiver: `this` in `x.extract(...)`)
     * @param pattern (optional) — What to look for
     * @param unique (optional) — Drop repeated matches
     * @returns matches — Everything that matched, in order
     * @returns count — How many matches were found
     */
    function extract(this: string, { string: string, pattern?: string, unique?: bool }): { matches: string[], count: int };

    /**
     * Formats a string with placeholders
     * @node string_format @receiver format_string @alias stringFormat
     * @param formatString — String with placeholders (receiver: `this` in `x.format(...)`)
     * @returns formattedString — Formatted string
     */
    function format(this: string, { formatString: string }): string;

    /**
     * Converts a byte array to a string using the UTF-8 lossy strategy
     * @node utf8_lossy @alias utf8Lossy
     * @param bytes
     * @returns string — Input String
     */
    function fromUtf8Lossy({ bytes: bytes[] }): string;

    /**
     * Finds the character index of the first occurrence of a substring
     * @node string_index_of @receiver string @alias stringIndexOf
     * @param string — Input String (receiver: `this` in `x.indexOf(...)`)
     * @param substring — Substring to search for
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns index — Character index of the match, -1 when not found
     * @returns found — True when the substring occurs in the string
     */
    function indexOf(this: string, { string: string, substring: string, ignoreCase?: bool }): { index: int, found: bool };

    /**
     * Checks whether every character is a letter or a digit
     * @node string_is_alphanumeric @receiver string @alias stringIsAlphanumeric
     * @param string — Input String (receiver: `this` in `x.isAlphanumeric(...)`)
     * @returns result — True when all characters are alphanumeric
     */
    function isAlphanumeric(this: string, { string: string }): bool;

    /**
     * Checks whether a string only contains ASCII characters
     * @node string_is_ascii @receiver string @alias stringIsAscii
     * @param string — Input String (receiver: `this` in `x.isAscii(...)`)
     * @returns result — True when the string is pure ASCII
     */
    function isAscii(this: string, { string: string }): bool;

    /**
     * Checks whether a string looks like an email address
     * @node string_is_email @receiver string @alias stringIsEmail
     * @param string — Input String (receiver: `this` in `x.isEmail(...)`)
     * @returns result — True when the string is a plausible email address
     */
    function isEmail(this: string, { string: string }): bool;

    /**
     * Checks whether a string contains no characters
     * @node string_is_empty @receiver string @alias stringIsEmpty
     * @param string — Input String (receiver: `this` in `x.isEmpty(...)`)
     * @param ignoreWhitespace (optional) — Treat whitespace-only strings as empty
     * @returns isEmpty — True when the string is empty
     */
    function isEmpty(this: string, { string: string, ignoreWhitespace?: bool }): bool;

    /**
     * Checks whether a string is an IPv4 or IPv6 address
     * @node string_is_ip @receiver string @alias stringIsIp
     * @param string — Input String (receiver: `this` in `x.isIp(...)`)
     * @returns result — True when the string is an IP address
     */
    function isIp(this: string, { string: string }): bool;

    /**
     * Checks whether a string parses as JSON
     * @node string_is_json @receiver string @alias stringIsJson
     * @param string — Input String (receiver: `this` in `x.isJson(...)`)
     * @returns result — True when the string is valid JSON
     */
    function isJson(this: string, { string: string }): bool;

    /**
     * Checks whether a string can be read as a number
     * @node string_is_numeric @receiver string @alias stringIsNumeric
     * @param string — Input String (receiver: `this` in `x.isNumeric(...)`)
     * @returns result — True when the string parses as a number
     */
    function isNumeric(this: string, { string: string }): bool;

    /**
     * Checks whether a string is a URL with a scheme and a host
     * @node string_is_url @receiver string @alias stringIsUrl
     * @param string — Input String (receiver: `this` in `x.isUrl(...)`)
     * @returns result — True when the string is a plausible URL
     */
    function isUrl(this: string, { string: string }): bool;

    /**
     * Checks whether a string is a UUID
     * @node string_is_uuid @receiver string @alias stringIsUuid
     * @param string — Input String (receiver: `this` in `x.isUuid(...)`)
     * @returns result — True when the string is a UUID
     */
    function isUuid(this: string, { string: string }): bool;

    /**
     * Joins multiple strings together
     * @node string_join @receiver strings @alias stringJoin
     * @param strings — Strings to join (receiver: `this` in `x.join(...)`)
     * @param separator — String to separate by
     * @returns joinedString — Concatenated string
     */
    function join(this: string[], { strings: string[], separator: string }): string;

    /**
     * Finds the character index of the last occurrence of a substring
     * @node string_last_index_of @receiver string @alias stringLastIndexOf
     * @param string — Input String (receiver: `this` in `x.lastIndexOf(...)`)
     * @param substring — Substring to search for
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns index — Character index of the last match, -1 when not found
     * @returns found — True when the substring occurs in the string
     */
    function lastIndexOf(this: string, { string: string, substring: string, ignoreCase?: bool }): { index: int, found: bool };

    /**
     * Calculates the length of a string
     * @node string_length @receiver string @alias stringLength
     * @param string — Input String (receiver: `this` in `x.length(...)`)
     * @param mode (optional) — Characters counts code points, Graphemes counts what a reader sees, Bytes counts UTF-8 bytes
     * @returns length — Length of the string
     */
    function length(this: string, { string: string, mode?: string }): int;

    /**
     * Splits a string into its lines
     * @node string_lines @receiver string @alias stringLines
     * @param string — Input String (receiver: `this` in `x.lines(...)`)
     * @param skipEmpty (optional) — Drop lines that are empty or whitespace only
     * @returns lines — One entry per line
     */
    function lines(this: string, { string: string, skipEmpty?: bool }): string[];

    /**
     * Hides the middle of a value, keeping a few characters visible
     * @node string_mask @receiver string @alias stringMask
     * @param string — Input String (receiver: `this` in `x.mask(...)`)
     * @param keepStart (optional) — Characters left visible at the start
     * @param keepEnd (optional) — Characters left visible at the end
     * @param maskCharacter (optional) — Character used for the hidden part
     * @param fixedWidth (optional) — Always use this many mask characters so the length is not leaked, 0 keeps the real length
     * @returns masked — The masked value
     */
    function mask(this: string, { string: string, keepStart?: int, keepEnd?: int, maskCharacter?: string, fixedWidth?: int }): string;

    /**
     * Collapses runs of whitespace into single spaces and trims the result
     * @node string_normalize_whitespace @receiver string @alias stringNormalizeWhitespace
     * @param string — Input String (receiver: `this` in `x.normalizeWhitespace(...)`)
     * @returns normalized — The normalized string
     */
    function normalizeWhitespace(this: string, { string: string }): string;

    /**
     * Fills up a string at the end until it reaches the target length
     * @node string_pad_end @receiver string @alias stringPadEnd
     * @param string — Input String (receiver: `this` in `x.padEnd(...)`)
     * @param length (optional) — Target length in characters
     * @param padding (optional) — Characters used to fill up the string
     * @returns padded — The padded string, unchanged when it is already long enough
     */
    function padEnd(this: string, { string: string, length?: int, padding?: string }): string;

    /**
     * Fills up a string at the start until it reaches the target length
     * @node string_pad_start @receiver string @alias stringPadStart
     * @param string — Input String (receiver: `this` in `x.padStart(...)`)
     * @param length (optional) — Target length in characters
     * @param padding (optional) — Characters used to fill up the string
     * @returns padded — The padded string, unchanged when it is already long enough
     */
    function padStart(this: string, { string: string, length?: int, padding?: string }): string;

    /**
     * Template Engine based on Jinja Templates
     * @node string_render_template @receiver template @alias stringRenderTemplate
     * @param template — Jinja Template String (receiver: `this` in `x.renderTemplate(...)`)
     * @returns rendered — Rendered String
     */
    function renderTemplate(this: string, { template: string }): string;

    /**
     * Repeats a string a number of times
     * @node string_repeat @receiver string @alias stringRepeat
     * @param string — Input String (receiver: `this` in `x.repeat(...)`)
     * @param count (optional) — How often the string is repeated
     * @returns repeated — The repeated string
     */
    function repeat(this: string, { string: string, count?: int }): string;

    /**
     * Replaces occurrences of a substring or regex pattern within a string.
     * @node string_replace @receiver string @alias stringReplace
     * @param string — Input String (receiver: `this` in `x.replace(...)`)
     * @param pattern — Substring or regex pattern to replace
     * @param replacement — Replacement string (supports $1, $2, ... for regex capture groups)
     * @param isRegex (optional) — Treat the pattern as a regular expression
     * @returns newString — String with replacements
     */
    function replace(this: string, { string: string, pattern: string, replacement: string, isRegex?: bool }): string;

    /**
     * Reverses the characters of a string
     * @node string_reverse @receiver string @alias stringReverse
     * @param string — Input String (receiver: `this` in `x.reverse(...)`)
     * @returns reversed — The reversed string
     */
    function reverse(this: string, { string: string }): string;

    /**
     * Turns text into a URL safe slug
     * @node string_slugify @receiver string @alias stringSlugify
     * @param string — Input String (receiver: `this` in `x.slugify(...)`)
     * @param separator (optional) — Placed between words
     * @param maxLength (optional) — Cut the slug at a word boundary, 0 for no limit
     * @returns slug — The slug
     */
    function slugify(this: string, { string: string, separator?: string, maxLength?: int }): string;

    /**
     * Splits a string into substrings
     * @node string_split @receiver string @alias stringSplit
     * @param string — Input String (receiver: `this` in `x.split(...)`)
     * @param separator — String to split by, an empty separator splits into single characters
     * @param isRegex (optional) — Treat the separator as a regular expression
     * @param limit (optional) — Maximum number of parts, 0 for no limit. The last part keeps the rest
     * @param skipEmpty (optional) — Drop parts that are empty
     * @returns substrings — Array of substrings
     */
    function split(this: string, { string: string, separator: string, isRegex?: bool, limit?: int, skipEmpty?: bool }): string[];

    /**
     * Splits a string into two halves at a character index
     * @node string_split_at @receiver string @alias stringSplitAt
     * @param string — Input String (receiver: `this` in `x.splitAt(...)`)
     * @param index (optional) — Character index to split at, negative counts from the end
     * @returns before — Characters before the index
     * @returns after — Characters from the index onwards
     */
    function splitAt(this: string, { string: string, index?: int }): { before: string, after: string };

    /**
     * Splits a string at the first (or last) occurrence of a separator
     * @node string_split_once @receiver string @alias stringSplitOnce
     * @param string — Input String (receiver: `this` in `x.splitOnce(...)`)
     * @param separator — String to split at
     * @param fromEnd (optional) — Split at the last occurrence instead of the first
     * @returns before — Text before the separator, the whole string when it was not found
     * @returns after — Text after the separator
     * @returns found — True when the separator was found
     */
    function splitOnce(this: string, { string: string, separator: string, fromEnd?: bool }): { before: string, after: string, found: bool };

    /**
     * Splits a string into words, collapsing runs of whitespace
     * @node string_split_whitespace @receiver string @alias stringSplitWhitespace
     * @param string — Input String (receiver: `this` in `x.splitWhitespace(...)`)
     * @returns words — The separated words
     */
    function splitWhitespace(this: string, { string: string }): string[];

    /**
     * Checks if a string starts with a specific string
     * @node string_starts_with @receiver string @alias stringStartsWith
     * @param string — Input String (receiver: `this` in `x.startsWith(...)`)
     * @param prefix — String to check against
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns startsWith — Does the string start with the prefix?
     */
    function startsWith(this: string, { string: string, prefix: string, ignoreCase?: bool }): bool;

    /**
     * Checks whether a string starts with any of the given prefixes
     * @node string_starts_with_any @receiver string @alias stringStartsWithAny
     * @param string — Input String (receiver: `this` in `x.startsWithAny(...)`)
     * @param prefixes — Prefixes to test
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns startsWith — True when the string starts with one of the prefixes
     * @returns matched — The first prefix that matched
     */
    function startsWithAny(this: string, { string: string, prefixes: string[], ignoreCase?: bool }): { startsWith: bool, matched: string };

    /**
     * Removes a prefix from a string if it is present
     * @node string_strip_prefix @receiver string @alias stringStripPrefix
     * @param string — Input String (receiver: `this` in `x.stripPrefix(...)`)
     * @param prefix — Prefix to remove
     * @returns result — String without the prefix
     * @returns stripped — True when the prefix was present
     */
    function stripPrefix(this: string, { string: string, prefix: string }): { result: string, stripped: bool };

    /**
     * Removes a suffix from a string if it is present
     * @node string_strip_suffix @receiver string @alias stringStripSuffix
     * @param string — Input String (receiver: `this` in `x.stripSuffix(...)`)
     * @param suffix — Suffix to remove
     * @returns result — String without the suffix
     * @returns stripped — True when the suffix was present
     */
    function stripSuffix(this: string, { string: string, suffix: string }): { result: string, stripped: bool };

    /**
     * Extracts a range of characters from a string. Negative start counts from the end, length -1 runs to the end.
     * @node string_substring @receiver string @alias stringSubstring
     * @param string — Input String (receiver: `this` in `x.substring(...)`)
     * @param start (optional) — First character index, negative counts from the end
     * @param length (optional) — Number of characters to take, -1 for the rest of the string
     * @returns substring — The extracted characters
     */
    function substring(this: string, { string: string, start?: int, length?: int }): string;

    /**
     * Parses a string into a boolean. Accepts true/false, 1/0, yes/no and on/off
     * @node string_to_bool @receiver string @alias stringToBool
     * @param string — String to parse (receiver: `this` in `x.toBool(...)`)
     * @param fallback (optional) — Value used when parsing fails
     * @returns boolean — The parsed boolean
     * @returns success — True when the string was a recognized boolean
     */
    function toBool(this: string, { string: string, fallback?: bool }): { boolean: bool, success: bool };

    /**
     * Splits a string into an array of single characters
     * @node string_to_chars @receiver string @alias stringToChars
     * @param string — Input String (receiver: `this` in `x.toChars(...)`)
     * @returns characters — One entry per character
     */
    function toChars(this: string, { string: string }): string[];

    /**
     * Parses a string into a float
     * @node string_to_float @receiver string @alias stringToFloat
     * @param string — String to parse (receiver: `this` in `x.toFloat(...)`)
     * @param fallback (optional) — Value used when parsing fails
     * @returns float — The parsed float
     * @returns success — True when the string was a valid float
     */
    function toFloat(this: string, { string: string, fallback?: float }): { float: float, success: bool };

    /**
     * Parses a string into an integer
     * @node string_to_int @receiver string @alias stringToInt
     * @param string — String to parse (receiver: `this` in `x.toInt(...)`)
     * @param fallback (optional) — Value used when parsing fails
     * @returns integer — The parsed integer
     * @returns success — True when the string was a valid integer
     */
    function toInt(this: string, { string: string, fallback?: int }): { integer: int, success: bool };

    /**
     * Converts a string to lowercase
     * @node string_to_lower @receiver string @alias stringToLower
     * @param string — Input String (receiver: `this` in `x.toLower(...)`)
     * @returns lowercaseString — String in lowercase
     */
    function toLower(this: string, { string: string }): string;

    /**
     * Converts a string to uppercase
     * @node string_to_upper @receiver string @alias stringToUpper
     * @param string — Input String (receiver: `this` in `x.toUpper(...)`)
     * @returns uppercaseString — String in uppercase
     */
    function toUpper(this: string, { string: string }): string;

    /**
     * Removes leading and trailing whitespace from a string
     * @node string_trim @receiver string @alias stringTrim
     * @param string — Input String (receiver: `this` in `x.trim(...)`)
     * @returns trimmedString — String without leading/trailing whitespace
     */
    function trim(this: string, { string: string }): string;

    /**
     * Removes trailing whitespace from a string
     * @node string_trim_end @receiver string @alias stringTrimEnd
     * @param string — Input String (receiver: `this` in `x.trimEnd(...)`)
     * @returns trimmedString — String without trailing whitespace
     */
    function trimEnd(this: string, { string: string }): string;

    /**
     * Removes the given characters from the start and/or end of a string
     * @node string_trim_matches @receiver string @alias stringTrimMatches
     * @param string — Input String (receiver: `this` in `x.trimMatches(...)`)
     * @param characters (optional) — Set of characters to strip
     * @param side (optional) — Where to strip
     * @returns trimmedString — String without the stripped characters
     */
    function trimMatches(this: string, { string: string, characters?: string, side?: string }): string;

    /**
     * Removes leading whitespace from a string
     * @node string_trim_start @receiver string @alias stringTrimStart
     * @param string — Input String (receiver: `this` in `x.trimStart(...)`)
     * @returns trimmedString — String without leading whitespace
     */
    function trimStart(this: string, { string: string }): string;

    /**
     * Shortens a string to a maximum number of characters, appending an ellipsis when it was cut
     * @node string_truncate @receiver string @alias stringTruncate
     * @param string — Input String (receiver: `this` in `x.truncate(...)`)
     * @param maxLength (optional) — Maximum number of characters including the ellipsis
     * @param ellipsis (optional) — Appended when the string was cut
     * @returns truncated — The shortened string
     * @returns wasTruncated — True when characters were removed
     */
    function truncate(this: string, { string: string, maxLength?: int, ellipsis?: string }): { truncated: string, wasTruncated: bool };

    /**
     * Compares two Strings
     * @node not_equal_string @receiver string @alias notEqualString
     * @param string — Input (receiver: `this` in `x.unequal(...)`)
     * @param string — Input (receiver: `this` in `x.unequal(...)`)
     * @param ignoreCase (optional) — Compare without regard to upper/lower case
     * @returns unequal — Are the strings equal?
     */
    function unequal(this: string, { string: string, string: string, ignoreCase?: bool }): bool;

    /**
     * Unescapes special character sequences in a string (\n, \t, \r, \\, \").
     * @node string_unescape @receiver string @alias stringUnescape
     * @param string — Input String (receiver: `this` in `x.unescape(...)`)
     * @returns unescaped — String with escape sequences resolved to actual characters
     */
    function unescape(this: string, { string: string }): string;

    /**
     * Counts words, sentences and reading time
     * @node string_word_count @receiver string @alias stringWordCount
     * @param string — Input String (receiver: `this` in `x.wordCount(...)`)
     * @param wordsPerMinute (optional) — Reading speed used for the estimate
     * @returns words — Number of words
     * @returns characters — Number of characters
     * @returns sentences — Number of sentences
     * @returns readingSeconds — Estimated reading time in seconds
     */
    function wordCount(this: string, { string: string, wordsPerMinute?: int }): { words: int, characters: int, sentences: int, readingSeconds: int };

    // === Utils/String/Case ===

    /**
     * Converts a string to camelCase or PascalCase
     * @node string_camel_case @receiver string @alias stringCamelCase
     * @param string — Input String (receiver: `this` in `x.camelCase(...)`)
     * @param pascalCase (optional) — Upper case the first word as well
     * @returns result — The converted string
     */
    function camelCase(this: string, { string: string, pascalCase?: bool }): string;

    /**
     * Upper cases the first character and leaves the rest untouched
     * @node string_capitalize @receiver string @alias stringCapitalize
     * @param string — Input String (receiver: `this` in `x.capitalize(...)`)
     * @returns result — The converted string
     */
    function capitalize(this: string, { string: string }): string;

    /**
     * Rewrites a string in the chosen case style. The input's own style is detected automatically, so any of the supported styles can be fed in
     * @node string_convert_case @receiver string @alias stringConvertCase
     * @param string — Input String (receiver: `this` in `x.convertCase(...)`)
     * @param targetCase (optional) — The case style to write the string in
     * @returns result — The converted string
     * @returns detectedCase — The case style the input was written in, or "undetermined" when it carries no evidence of one
     */
    function convertCase(this: string, { string: string, targetCase?: string }): { result: string, detectedCase: string };

    /**
     * Names the case style a string is written in
     * @node string_detect_case @receiver string @alias stringDetectCase
     * @param string — Input String (receiver: `this` in `x.detectCase(...)`)
     * @returns detectedCase — The detected case style, or "undetermined" when the string carries no evidence of one
     */
    function detectCase(this: string, { string: string }): string;

    /**
     * Converts a string to kebab-case
     * @node string_kebab_case @receiver string @alias stringKebabCase
     * @param string — Input String (receiver: `this` in `x.kebabCase(...)`)
     * @returns result — The converted string
     */
    function kebabCase(this: string, { string: string }): string;

    /**
     * Converts a string to snake_case
     * @node string_snake_case @receiver string @alias stringSnakeCase
     * @param string — Input String (receiver: `this` in `x.snakeCase(...)`)
     * @returns result — The converted string
     */
    function snakeCase(this: string, { string: string }): string;

    /**
     * Converts a string to Title Case
     * @node string_title_case @receiver string @alias stringTitleCase
     * @param string — Input String (receiver: `this` in `x.titleCase(...)`)
     * @returns result — The converted string
     */
    function titleCase(this: string, { string: string }): string;

    // === Utils/String/Regex ===

    /**
     * Extracts the capture groups of the first regular expression match
     * @node string_regex_captures @receiver string @alias stringRegexCaptures
     * @param string — Input String (receiver: `this` in `x.regexCaptures(...)`)
     * @param pattern — Regular expression pattern
     * @returns groups — Capture groups, index 0 is the whole match
     * @returns found — True when the pattern matched
     */
    function regexCaptures(this: string, { string: string, pattern: string }): { groups: string[], found: bool };

    /**
     * Returns every match of a regular expression in a string
     * @node string_regex_find_all @receiver string @alias stringRegexFindAll
     * @param string — Input String (receiver: `this` in `x.regexFindAll(...)`)
     * @param pattern — Regular expression pattern
     * @returns matches — All matching substrings
     * @returns count — Number of matches
     */
    function regexFindAll(this: string, { string: string, pattern: string }): { matches: string[], count: int };

    /**
     * Checks whether a regular expression matches a string
     * @node string_regex_match @receiver string @alias stringRegexMatch
     * @param string — Input String (receiver: `this` in `x.regexMatch(...)`)
     * @param pattern — Regular expression pattern
     * @returns isMatch — True when the pattern matches
     * @returns firstMatch — The first matching text, empty when there is no match
     */
    function regexMatch(this: string, { string: string, pattern: string }): { isMatch: bool, firstMatch: string };

    // === Utils/String/Similarity ===

    /**
     * Calculates the Damerau-Levenshtein distance between two strings
     * @node damerau_levenshtein_distance @receiver string1 @alias damerauLevenshteinDistance
     * @param string1 — First String (receiver: `this` in `x.damerauLevenshteinDistance(...)`)
     * @param string2 — Second String
     * @param normalize (optional) — Normalize the Distance
     * @returns distance — Damerau-Levenshtein Distance
     */
    function damerauLevenshteinDistance(this: string, { string1: string, string2: string, normalize?: bool }): float;

    /**
     * Calculates the Hamming distance between two strings
     * @node hamming_distance @receiver string1 @alias hammingDistance
     * @param string1 — First String (receiver: `this` in `x.hammingDistance(...)`)
     * @param string2 — Second String
     * @returns distance — Hamming Distance
     */
    function hammingDistance(this: string, { string1: string, string2: string }): float;

    /**
     * Calculates the Jaro distance between two strings
     * @node jaro_distance @receiver string1 @alias jaroDistance
     * @param string1 — First String (receiver: `this` in `x.jaroDistance(...)`)
     * @param string2 — Second String
     * @returns distance — Jaro Distance
     */
    function jaroDistance(this: string, { string1: string, string2: string }): float;

    /**
     * Calculates the Jaro-Winkler distance between two strings
     * @node jaro_winkler_distance @receiver string1 @alias jaroWinklerDistance
     * @param string1 — First String (receiver: `this` in `x.jaroWinklerDistance(...)`)
     * @param string2 — Second String
     * @returns distance — Jaro-Winkler Distance
     */
    function jaroWinklerDistance(this: string, { string1: string, string2: string }): float;

    /**
     * Calculates the Levenshtein distance between two strings
     * @node levenshtein_distance @receiver string1 @alias levenshteinDistance
     * @param string1 — First String (receiver: `this` in `x.levenshteinDistance(...)`)
     * @param string2 — Second String
     * @param normalize (optional) — Normalize the Distance
     * @returns distance — Levenshtein Distance
     */
    function levenshteinDistance(this: string, { string1: string, string2: string, normalize?: bool }): float;

    /**
     * Calculates the Optimal String Alignment distance between two strings
     * @node optimal_string_alignment_distance @receiver string1 @alias optimalStringAlignmentDistance
     * @param string1 — First String (receiver: `this` in `x.optimalStringAlignmentDistance(...)`)
     * @param string2 — Second String
     * @returns distance — Optimal String Alignment Distance
     */
    function optimalStringAlignmentDistance(this: string, { string1: string, string2: string }): float;

    /**
     * Calculates the Sørensen-Dice coefficient between two strings
     * @node sorensen_dice_coefficient @receiver string1 @alias sorensenDiceCoefficient
     * @param string1 — First String (receiver: `this` in `x.sorensenDiceCoefficient(...)`)
     * @param string2 — Second String
     * @returns coefficient — Sørensen-Dice Coefficient
     */
    function sorensenDiceCoefficient(this: string, { string1: string, string2: string }): float;
}

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

declare namespace test {
    // === Utils/Testing ===

    /**
     * Asserts a condition inside a flow. On pass it logs `ASSERT_OK {label}` (Info) and continues; on fail it logs `ASSERT_FAIL {label} {details}` (Error) and halts the run with an error. Test runners grep these stable marker prefixes. Name test events with a `test` prefix so they are discoverable by test tooling.
     * @node flow_assert @alias flowAssert
     * @param condition (optional) — The condition that must hold.
     * @param label (optional) — Stable name for this assertion, echoed in the ASSERT_OK/ASSERT_FAIL log markers.
     * @param details (optional) — Optional context logged when the assertion fails.
     * @impure has side effects / drives control flow
     */
    function assert({ condition?: bool, label?: string, details?: any }): void;
}

declare namespace types {
    // === Utils/Types ===

    /**
     * Returns the input value if valid, otherwise returns the fallback default. Useful for handling optional values or error recovery.
     * @node utils_types_fallback @receiver value @alias utilsTypesFallback
     * @param value — The primary value to use if available and valid (receiver: `this` in `x.fallback(...)`)
     * @param default — Fallback value used when the primary value is null, missing, or invalid
     * @returns result — The resolved value (primary if valid, otherwise default)
     * @returns usedFallback — True if the fallback value was used
     */
    function fallback(this: any, { value: any, default: any }): { result: any, usedFallback: bool };

    /**
     * True for null, an empty string, an empty array and an empty struct
     * @node utils_types_is_empty @alias utilsTypesIsEmpty
     * @param value — Value to inspect
     * @param trim (optional) — Treat whitespace-only text as empty
     * @returns isEmpty — True when the value holds nothing
     */
    function isEmpty({ value: any, trim?: bool }): bool;

    /**
     * Selects between two values based on a boolean condition. Returns A if true, B if false.
     * @node utils_types_select @alias utilsTypesSelect
     * @param a — Value returned when condition is true
     * @param b — Value returned when condition is false
     * @param condition (optional) — If true, returns A. If false, returns B.
     * @returns result — The selected value (A if true, B if false)
     */
    function select({ a: any, b: any, condition?: bool }): any;

    /**
     * Tries to transform cast types.
     * @node utils_types_try_transform @alias utilsTypesTryTransform
     * @param typeIn — Type to transform
     * @returns typeOut — If the type was successfully transformed, transformed type
     * @returns success — Determines of tje transformation was successful
     */
    function tryTransform({ typeIn: any }): { typeOut: any, success: bool };

    /**
     * Reports what a value actually is — useful for data coming back from an API or a model
     * @node utils_types_type_of @receiver value @alias utilsTypesTypeOf
     * @param value — Value to inspect (receiver: `this` in `x.typeOf(...)`)
     * @returns type — One of null, boolean, number, string, array or object
     * @returns isNull — True when the value is missing
     * @returns size — Elements for an array, fields for an object, characters for a string, otherwise 0
     */
    function typeOf(this: any, { value: any }): { type: string, isNull: bool, size: int };
}

declare namespace ui {
    // === UI/Component ===

    /**
     * Creates an A2UI component with ID, style, and component data
     * @node a2ui_create_component @alias a2uiCreateComponent
     * @param componentId — Unique identifier for the component
     * @param componentType (optional) — Component type (row, column, text, button, etc.)
     * @param props — Component properties as JSON
     * @param style — Optional style for the component
     * @returns component — The created component
     * @impure has side effects / drives control flow
     */
    function createComponent({ componentId: string, componentType?: string, props: Struct, style: Struct }): Struct;

    // === UI/Container ===

    /**
     * Creates a new widget instance for dynamic insertion into containers. The dropdown lists project widgets and widgets from packages added to the project; selecting one auto-generates typed input pins (exposed props and customizations for project widgets, contract inputs for package widgets).
     * @node a2ui_instantiate_widget @alias a2uiInstantiateWidget
     * @param widgetSelector — Select a widget from the project or from packages added to the project
     * @param instanceId — Unique ID for this widget instance
     * @returns elementRef — Element reference for the instantiated widget (connect to Push To Container)
     * @impure has side effects / drives control flow
     */
    function instantiateWidget({ widgetSelector: string, instanceId: string }): Struct;

    /**
     * Dynamically adds an element to a container's children list
     * @node a2ui_push_to_container @alias a2uiPushToContainer
     * @param containerRef — Reference to the container element (ID or element object)
     * @param elementRef — Reference to the element to add (e.g. from Instantiate Widget)
     * @param position (optional) — Position to insert: -1 for end, 0 for start, or specific index
     * @returns success — Whether the element was successfully added
     * @impure has side effects / drives control flow
     */
    function pushToContainer({ containerRef: Struct, elementRef: Struct, position?: int }): bool;

    /**
     * Removes an element from a container's children list
     * @node a2ui_remove_from_container @alias a2uiRemoveFromContainer
     * @param containerId — ID of the container element to remove from
     * @param elementId — ID of the element to remove
     * @returns success — Whether the element was successfully removed
     * @impure has side effects / drives control flow
     */
    function removeFromContainer({ containerId: string, elementId: string }): bool;

    /**
     * Resolves an element inside a widget instance (from Instantiate Widget). The output plugs into any element node (Set Element Value, Update GeoMap, Push CSV To Chart, …).
     * @node a2ui_widget_get_element @alias a2uiWidgetGetElement
     * @param elementRef — Widget instance reference (from Instantiate Widget)
     * @param elementId — ID of the element inside the widget (e.g. 'chart-1')
     * @returns element — The element reference (connect to element nodes)
     * @returns exists — Whether the element exists in the widget
     */
    function widgetGetElement({ elementRef: Struct, elementId: string }): { element: Struct, exists: bool };

    /**
     * Reads a typed query result from a package widget instance. Connect Element Ref from Instantiate Widget, or Element from Get Element for a widget placed in the visual builder, then select a contract query.
     * @node a2ui_widget_query @alias a2uiWidgetQuery
     * @param elementRef — Package widget reference from Instantiate Widget, or a visual-builder widget from Get Element
     * @param query — Contract query to run on the widget instance
     * @returns value — The query result, typed by the contract's result schema
     * @impure has side effects / drives control flow
     */
    function widgetQuery({ elementRef: Struct, query: string }): any;

    /**
     * Sets the text of an element inside a widget instance (from Instantiate Widget) before it is pushed to the frontend
     * @node a2ui_widget_set_text @alias a2uiWidgetSetText
     * @param elementRef — Widget instance reference (from Instantiate Widget)
     * @param elementId — ID of the element inside the widget (e.g. 'title-text')
     * @param text (optional) — The text to set
     * @returns elementRefOut — The updated widget instance reference (connect to Push Widget / Push To Container)
     * @impure has side effects / drives control flow
     */
    function widgetSetText({ elementRef: Struct, elementId: string, text?: string }): Struct;

    /**
     * Sends a typed input patch to a package widget instance. Connect the Element Ref from Instantiate Widget to generate one optional pin per contract input; only set pins are included in the patch.
     * @node a2ui_widget_update_inputs @alias a2uiWidgetUpdateInputs
     * @param elementRef — Element reference of a package widget instance (from Instantiate Widget)
     * @impure has side effects / drives control flow
     */
    function widgetUpdateInputs({ elementRef: Struct }): void;

    // === UI/Data ===

    /**
     * Updates data in a surface's data model
     * @node a2ui_data_update @alias a2uiDataUpdate
     * @param surfaceId (optional) — ID of the surface to update
     * @param path — Data path to update (e.g., 'user/name')
     * @param value — New value to set at the path
     * @impure has side effects / drives control flow
     */
    function dataUpdate({ surfaceId?: string, path: string, value: any }): void;

    /**
     * Requests element values from the frontend before processing
     * @node a2ui_request_elements @alias a2uiRequestElements
     * @param elementIds — Array of element IDs to request (e.g., ['main/input-field', 'main/checkbox'])
     * @impure has side effects / drives control flow
     */
    function requestElements({ elementIds: string[] }): void;

    /**
     * Updates or inserts an element value in the frontend
     * @node a2ui_upsert_element @alias a2uiUpsertElement
     * @param elementId — ID of the element to update (e.g., 'main/status-text')
     * @param value — New value for the element
     * @impure has side effects / drives control flow
     */
    function upsertElement({ elementId: string, value: any }): void;

    // === UI/Elements ===

    /**
     * Clones an existing element and adds it to a container
     * @node a2ui_clone_element @alias a2uiCloneElement
     * @param sourceElement — The element to clone (format: surfaceId/elementId)
     * @param newElementId — ID for the cloned element
     * @param parentId — Container to add the cloned element to (optional, uses source parent if empty)
     * @param index — Position in parent container (-1 for end)
     * @returns clonedElementRef — Reference to the cloned element
     * @impure has side effects / drives control flow
     */
    function cloneElement({ sourceElement: string, newElementId: string, parentId: string, index: int }): string;

    /**
     * Creates a new element and adds it to a parent container
     * @node a2ui_create_element @alias a2uiCreateElement
     * @param surfaceId — The surface to create the element in
     * @param parentId — Parent element ID string or element object from Get Element
     * @param elementId — Unique ID for the new element
     * @param componentType — The component type (e.g., 'Text', 'Button', 'Container')
     * @param props — Component properties as JSON object
     * @param index — Optional index to insert at (default: append at end)
     * @returns createdId — The ID of the created element
     * @impure has side effects / drives control flow
     */
    function createElement({ surfaceId: string, parentId: any, elementId: string, componentType: string, props: any, index: int }): string;

    /**
     * Gets an element's data from the page
     * @node a2ui_get_element @alias a2uiGetElement
     * @param elementRef — Reference to the page element
     * @returns element — The element data
     * @returns exists — Whether the element exists
     */
    function getElement({ elementRef: string }): { element: Struct, exists: bool };

    /**
     * Gets the text content of an element
     * @node a2ui_get_element_text @alias a2uiGetElementText
     * @param elementRef — Reference to the text element
     * @returns text — The text content of the element
     * @returns exists — Whether the element exists
     */
    function getElementText({ elementRef: Struct }): { text: string, exists: bool };

    /**
     * Gets the value of an input element
     * @node a2ui_get_element_value @alias a2uiGetElementValue
     * @param elementRef — Reference to the input element
     * @returns value — The current value of the input
     * @returns exists — Whether the element exists
     */
    function getElementValue({ elementRef: Struct }): { value: any, exists: bool };

    /**
     * Removes an element from the page
     * @node a2ui_remove_element @alias a2uiRemoveElement
     * @param surfaceId — The surface containing the element
     * @param elementId — Element ID string or element object from Get Element
     * @impure has side effects / drives control flow
     */
    function removeElement({ surfaceId: string, elementId: any }): void;

    /**
     * Dynamically sets the legacy default action or a named event action of an interactive element
     * @node a2ui_set_element_action @alias a2uiSetElementAction
     * @param elementRef — Reference to the element (ID string or element object from Get Element)
     * @param eventName (optional) — Optional named component event (for example click, change, open, or delete). Leave empty to update the legacy default action.
     * @param actionType (optional) — Type of action: navigate_page, external_link, workflow_event, or clear to remove action
     * @param route — For navigate_page: the route path (e.g., /about, /products/123)
     * @param queryParams — For navigate_page: optional JSON object of query parameters
     * @param url — For external_link: the external URL to open
     * @param nodeId — For workflow_event: the ID of the workflow node to trigger
     * @impure has side effects / drives control flow
     */
    function setElementAction({ elementRef: Struct, eventName?: string, actionType?: string, route: string, queryParams: string, url: string, nodeId: string }): void;

    /**
     * Enables or disables an element
     * @node a2ui_set_element_disabled @alias a2uiSetElementDisabled
     * @param elementRef — Element ID string or element object from Get Element
     * @param disabled — Whether the element should be disabled
     * @impure has side effects / drives control flow
     */
    function setElementDisabled({ elementRef: Struct, disabled: bool }): void;

    /**
     * Sets the loading state of a button element
     * @node a2ui_set_element_loading @alias a2uiSetElementLoading
     * @param elementRef — Element ID string or element object from Get Element
     * @param loading — Whether the element is in loading state
     * @impure has side effects / drives control flow
     */
    function setElementLoading({ elementRef: Struct, loading: bool }): void;

    /**
     * Sets style properties of an element
     * @node a2ui_set_element_style @alias a2uiSetElementStyle
     * @param elementRef — Element ID string or element object from Get Element
     * @param style — Style properties to set (JSON object)
     * @impure has side effects / drives control flow
     */
    function setElementStyle({ elementRef: any, style: Struct }): void;

    /**
     * Sets the text content of an element
     * @node a2ui_set_element_text @alias a2uiSetElementText
     * @param elementRef — Reference to the text element (ID string or element object from Get Element)
     * @param text — The new text content
     * @impure has side effects / drives control flow
     */
    function setElementText({ elementRef: Struct, text: string }): void;

    /**
     * Sets the value of an input element
     * @node a2ui_set_element_value @alias a2uiSetElementValue
     * @param elementRef — Element ID string or element object from Get Element
     * @param value — The new value for the input
     * @impure has side effects / drives control flow
     */
    function setElementValue({ elementRef: Struct, value: string }): void;

    /**
     * Shows or hides an element
     * @node a2ui_set_element_visibility @alias a2uiSetElementVisibility
     * @param elementRef — Element ID string or element object from Get Element
     * @param visible (optional) — Whether the element should be visible
     * @impure has side effects / drives control flow
     */
    function setElementVisibility({ elementRef: any, visible?: bool }): void;

    // === UI/Elements/Button ===

    /**
     * Gets whether a button element is disabled
     * @node a2ui_get_button_disabled @alias a2uiGetButtonDisabled
     * @param elementRef — Reference to the button element
     * @returns disabled — Whether the button is disabled
     */
    function getButtonDisabled({ elementRef: Struct }): bool;

    /**
     * Gets the label text of a button element
     * @node a2ui_get_button_label @alias a2uiGetButtonLabel
     * @param elementRef — Reference to the button element
     * @returns label — The button's label text
     */
    function getButtonLabel({ elementRef: Struct }): string;

    /**
     * Gets whether a button element is in loading state
     * @node a2ui_get_button_loading @alias a2uiGetButtonLoading
     * @param elementRef — Reference to the button element
     * @returns loading — Whether the button is loading
     */
    function getButtonLoading({ elementRef: Struct }): bool;

    /**
     * Sets the label text of a button element
     * @node a2ui_set_button_label @alias a2uiSetButtonLabel
     * @param elementRef — Element ID string or element object from Get Element
     * @param label — The new label text
     * @impure has side effects / drives control flow
     */
    function setButtonLabel({ elementRef: Struct, label: string }): void;

    // === UI/Elements/Calendar ===

    /**
     * Add, remove, or update calendar events and view configuration
     * @node a2ui_update_calendar @alias a2uiUpdateCalendar
     * @param elementRef — Reference to the calendar element
     * @param operation (optional) — What operation to perform
     * @param events — Array of events
     * @impure has side effects / drives control flow
     */
    function updateCalendar({ elementRef: Struct, operation?: string, events: Struct[] }): void;

    // === UI/Elements/Charts ===

    /**
     * Push data to a Nivo or Plotly chart. Select JSON for pre-formatted data or CSV for auto-transformation.
     * @node a2ui_push_csv_to_chart @alias a2uiPushCsvToChart
     * @param elementRef — Reference to the chart element
     * @param library (optional) — Nivo or Plotly
     * @param format (optional) — Data format: JSON (passthrough) or CSV (auto-transform)
     * @param data — Chart data as JSON array/object or JSON string
     * @impure has side effects / drives control flow
     */
    function pushCsvToChart({ elementRef: Struct, library?: string, format?: string, data: Struct }): void;

    /**
     * Sets the layout configuration for a Plotly chart
     * @node a2ui_set_chart_layout @alias a2uiSetChartLayout
     * @param elementRef — Reference to the chart element (ID or element object)
     * @param layout — Chart layout object (Plotly layout format)
     * @impure has side effects / drives control flow
     */
    function setChartLayout({ elementRef: Struct, layout: any }): void;

    /**
     * Configure Nivo chart appearance
     * @node a2ui_set_chart_style @alias a2uiSetChartStyle
     * @param elementRef — Reference to the NivoChart element
     * @param chartType (optional) — Type of chart to style
     * @param barStyle — Bar chart styling options
     * @impure has side effects / drives control flow
     */
    function setChartStyle({ elementRef: Struct, chartType?: string, barStyle: Struct }): void;

    /**
     * Sets configuration options for a Nivo chart
     * @node a2ui_set_nivo_config @alias a2uiSetNivoConfig
     * @param elementRef — Reference to the Nivo chart element
     * @param config — Full Nivo configuration object (merged with defaults)
     * @param chartType (optional) — Chart type (bar, line, pie, radar, etc.)
     * @param colors (optional) — Color scheme name or array of colors
     * @param height (optional) — Chart height (e.g., '400px')
     * @impure has side effects / drives control flow
     */
    function setNivoConfig({ elementRef: Struct, config: any, chartType?: string, colors?: any, height?: string }): void;

    // === UI/Elements/Charts/Agent ===

    /**
     * Uses an LLM to write and run SQL against a DataFusion session, returning chart-ready struct data.
     * @node a2ui_chart_data_agent @alias a2uiChartDataAgent
     * @param model — LLM model (Bit)
     * @param session — DataFusion session to query
     * @param table — Table name within the session to query
     * @param description — Natural language task (e.g. 'monthly sales by region')
     * @param chartType (optional) — Target chart type
     * @param element — Chart element reference (from Get Element) to bind the data to
     * @returns data — Query results as an array of row structs (chart-ready)
     * @returns sql — Generated SQL query
     * @returns explanation — AI explanation of the query
     * @impure has side effects / drives control flow
     */
    function chartDataAgent({ model: Struct, session: Struct, table: string, description: string, chartType?: string, element: Struct }): { data: Struct, sql: string, explanation: string };

    // === UI/Elements/Checkbox ===

    /**
     * Set or toggle checkbox/switch checked state
     * @node a2ui_update_toggle @alias a2uiUpdateToggle
     * @param elementRef — Reference to checkbox or switch element
     * @param operation (optional) — What operation to perform
     * @param checked (optional) — New checked state
     * @impure has side effects / drives control flow
     */
    function updateToggle({ elementRef: Struct, operation?: string, checked?: bool }): void;

    // === UI/Elements/Containers ===

    /**
     * Removes all children from a container element
     * @node a2ui_clear_children @alias a2uiClearChildren
     * @param containerRef — Reference to the container element (ID or element object)
     * @impure has side effects / drives control flow
     */
    function clearChildren({ containerRef: any }): void;

    /**
     * Gets a child element at a specific index from a container
     * @node a2ui_get_child_at_index @alias a2uiGetChildAtIndex
     * @param containerRef — Reference to the container element
     * @param index — The index of the child to get (0-based)
     * @returns child — The child element at the specified index
     * @returns childId — The ID of the child element
     * @returns found — Whether a child was found at the index
     */
    function getChildAtIndex({ containerRef: string, index: int }): { child: Struct, childId: string, found: bool };

    /**
     * Appends a child element to a container
     * @node a2ui_push_child @alias a2uiPushChild
     * @param containerRef — Reference to the container element (ID or element object)
     * @param childRef — Reference to the child element to append
     * @impure has side effects / drives control flow
     */
    function pushChild({ containerRef: any, childRef: any }): void;

    /**
     * Inserts a child element at a specific index in a container
     * @node a2ui_push_child_at_index @alias a2uiPushChildAtIndex
     * @param containerRef — Reference to the container element (ID or element object)
     * @param childRef — Reference to the child element to insert
     * @param index — The index at which to insert the child (0-based)
     * @impure has side effects / drives control flow
     */
    function pushChildAtIndex({ containerRef: any, childRef: any, index: int }): void;

    /**
     * Removes a child element at a specific index from a container
     * @node a2ui_remove_child_at_index @alias a2uiRemoveChildAtIndex
     * @param containerRef — Reference to the container element (ID or element object)
     * @param index — The index of the child to remove (0-based)
     * @impure has side effects / drives control flow
     */
    function removeChildAtIndex({ containerRef: any, index: int }): void;

    // === UI/Elements/Display ===

    /**
     * Sets the content/text of a badge element
     * @node a2ui_set_badge_content @alias a2uiSetBadgeContent
     * @param elementRef — Reference to the badge element
     * @param content — The badge content (text or number)
     * @impure has side effects / drives control flow
     */
    function setBadgeContent({ elementRef: Struct, content: string }): void;

    /**
     * Sets the original and modified content of a diff view element
     * @node a2ui_set_diff_content @alias a2uiSetDiffContent
     * @param elementRef — Reference to the diff view element
     * @param original — Left / old content (text or document URL)
     * @param modified — Right / new content (text or document URL)
     * @impure has side effects / drives control flow
     */
    function setDiffContent({ elementRef: Struct, original: string, modified: string }): void;

    /**
     * Sets the icon name of an icon element
     * @node a2ui_set_icon @alias a2uiSetIcon
     * @param elementRef — Reference to the icon element
     * @param name — The icon name (e.g., 'check', 'x', 'star')
     * @impure has side effects / drives control flow
     */
    function setIcon({ elementRef: Struct, name: string }): void;

    /**
     * Sets the markdown content of a markdown element
     * @node a2ui_set_markdown_content @alias a2uiSetMarkdownContent
     * @param elementRef — Reference to the markdown element
     * @param content — The markdown content
     * @impure has side effects / drives control flow
     */
    function setMarkdownContent({ elementRef: Struct, content: string }): void;

    /**
     * Sets the value of a progress bar (0-100)
     * @node a2ui_set_progress @alias a2uiSetProgress
     * @param elementRef — Reference to the progress bar element
     * @param value — Progress value (0-100)
     * @impure has side effects / drives control flow
     */
    function setProgress({ elementRef: Struct, value: float }): void;

    // === UI/Elements/Files ===

    /**
     * Gets uploaded files, signed URLs, and FlowPaths from an A2UI fileInput or voiceInput element
     * @node a2ui_get_file_input_files @alias a2uiGetFileInputFiles
     * @param elementRef — File or voice input element ID or element object from Get Element
     * @returns files — Uploaded file objects
     * @returns signedUrls — Signed or local URLs for the uploaded files
     * @returns flowPaths — Temporary FlowPaths for uploaded files when available
     * @returns exists — Whether the file input element exists
     */
    function getFileInputFiles({ elementRef: Struct }): { files: Struct[], signedUrls: string[], flowPaths: Struct[], exists: bool };

    // === UI/Elements/Game ===

    /**
     * Update any property of a 3D model
     * @node a2ui_update_model3d @alias a2uiUpdateModel3d
     * @param elementRef — Reference to the 3D model element
     * @param property (optional) — Which property to update
     * @param src — GLTF/GLB model URL
     * @impure has side effects / drives control flow
     */
    function updateModel3d({ elementRef: Struct, property?: string, src: string }): void;

    /**
     * Update any property of a 3D scene
     * @node a2ui_update_scene3d @alias a2uiUpdateScene3d
     * @param elementRef — Reference to the 3D scene element
     * @param property (optional) — Which property to update
     * @param camera — Camera type, position, and target
     * @impure has side effects / drives control flow
     */
    function updateScene3d({ elementRef: Struct, property?: string, camera: Struct }): void;

    /**
     * Update any property of a sprite
     * @node a2ui_update_sprite @alias a2uiUpdateSprite
     * @param elementRef — Reference to the sprite element
     * @param property (optional) — Which property to update
     * @param src — Image URL
     * @impure has side effects / drives control flow
     */
    function updateSprite({ elementRef: Struct, property?: string, src: string }): void;

    // === UI/Elements/Gantt ===

    /**
     * Add, remove, or update gantt tasks, dependencies and configuration
     * @node a2ui_update_gantt @alias a2uiUpdateGantt
     * @param elementRef — Reference to the gantt element
     * @param operation (optional) — What operation to perform
     * @param tasks — Array of tasks
     * @impure has side effects / drives control flow
     */
    function updateGantt({ elementRef: Struct, operation?: string, tasks: Struct[] }): void;

    // === UI/Elements/GeoMap ===

    /**
     * Update markers, routes, or viewport of a map
     * @node a2ui_update_geomap @alias a2uiUpdateGeomap
     * @param elementRef — Reference to the map element
     * @param property (optional) — Which property to update
     * @param markers — Array of map markers
     * @impure has side effects / drives control flow
     */
    function updateGeomap({ elementRef: Struct, property?: string, markers: Struct[] }): void;

    // === UI/Elements/Get ===

    /**
     * Gets the src URL of an iframe element
     * @node a2ui_get_iframe_src @alias a2uiGetIframeSrc
     * @param elementRef — Reference to the iframe element
     * @returns src — The iframe's source URL
     */
    function getIframeSrc({ elementRef: Struct }): string;

    /**
     * Gets the content text of a tooltip element
     * @node a2ui_get_tooltip_content @alias a2uiGetTooltipContent
     * @param elementRef — Reference to the tooltip element
     * @returns content — The tooltip's content text
     * @returns side — The tooltip's side position (top, bottom, left, right)
     */
    function getTooltipContent({ elementRef: Struct }): { content: string, side: string };

    // === UI/Elements/Graph ===

    /**
     * Update the nodes, edges or label styles of a graph
     * @node a2ui_update_graph @alias a2uiUpdateGraph
     * @param elementRef — Reference to the graph element
     * @param property (optional) — Which property to update
     * @param nodes — Array of graph nodes
     * @impure has side effects / drives control flow
     */
    function updateGraph({ elementRef: Struct, property?: string, nodes: Struct[] }): void;

    // === UI/Elements/Hotspot ===

    /**
     * Add, remove, or manage hotspots on an ImageHotspot element
     * @node a2ui_update_hotspot @alias a2uiUpdateHotspot
     * @param elementRef — Reference to the ImageHotspot element
     * @param operation (optional) — What operation to perform
     * @param hotspot — Hotspot to add
     * @impure has side effects / drives control flow
     */
    function updateHotspot({ elementRef: Struct, operation?: string, hotspot: Struct }): void;

    // === UI/Elements/Input ===

    /**
     * Clears the value of an input element
     * @node a2ui_clear_input @alias a2uiClearInput
     * @param elementRef — Element ID string or element object from Get Element
     * @impure has side effects / drives control flow
     */
    function clearInput({ elementRef: any }): void;

    /**
     * Gets the placeholder text of an input element
     * @node a2ui_get_input_placeholder @alias a2uiGetInputPlaceholder
     * @param elementRef — Reference to the input element
     * @returns placeholder — The input's placeholder text
     */
    function getInputPlaceholder({ elementRef: Struct }): string;

    /**
     * Sets the placeholder text of an input element
     * @node a2ui_set_input_placeholder @alias a2uiSetInputPlaceholder
     * @param elementRef — Element ID string or element object from Get Element
     * @param placeholder — The new placeholder text
     * @impure has side effects / drives control flow
     */
    function setInputPlaceholder({ elementRef: Struct, placeholder: string }): void;

    /**
     * Sets the error state or message of a text field
     * @node a2ui_set_textfield_error @alias a2uiSetTextfieldError
     * @param elementRef — Reference to the text field element
     * @param error — Error message (empty string clears error)
     * @impure has side effects / drives control flow
     */
    function setTextfieldError({ elementRef: Struct, error: string }): void;

    // === UI/Elements/Labeler ===

    /**
     * Add, remove, or manage bounding boxes on an ImageLabeler element
     * @node a2ui_update_labeler @alias a2uiUpdateLabeler
     * @param elementRef — Reference to the ImageLabeler element
     * @param operation (optional) — What operation to perform
     * @param box — Bounding box to add
     * @impure has side effects / drives control flow
     */
    function updateLabeler({ elementRef: Struct, operation?: string, box: Struct }): void;

    // === UI/Elements/Media ===

    /**
     * Sets the source URL of an iframe element
     * @node a2ui_set_iframe_src @alias a2uiSetIframeSrc
     * @param elementRef — Reference to the iframe element
     * @param src — The URL to load in the iframe
     * @impure has side effects / drives control flow
     */
    function setIframeSrc({ elementRef: Struct, src: string }): void;

    /**
     * Sets raw HTML content of an iframe element for previewing generated HTML
     * @node a2ui_set_iframe_srcdoc @alias a2uiSetIframeSrcdoc
     * @param elementRef — Reference to the iframe element
     * @param html — Raw HTML content to render inside the iframe
     * @impure has side effects / drives control flow
     */
    function setIframeSrcdoc({ elementRef: Struct, html: string }): void;

    /**
     * Signs a FlowPath and sets it as the source for image, video, avatar, iframe, lottie, or file preview elements
     * @node a2ui_set_media_source @alias a2uiSetMediaSource
     * @param elementRef — Reference to the media element
     * @param file — FlowPath to sign and use as the element source
     * @param expiration (optional) — Expiration time for the signed URL
     * @returns signedUrl — The generated signed URL
     * @returns mimeType — Detected MIME type from the FlowPath extension
     * @returns mediaKind — Detected media kind: image, video, audio, pdf, text, or file
     * @impure has side effects / drives control flow
     */
    function setMediaSource({ elementRef: Struct, file: Struct, expiration?: int }): { signedUrl: string, mimeType: string, mediaKind: string };

    // === UI/Elements/Overlay ===

    /**
     * Set, push, or clear bounding boxes on a BoundingBoxOverlay element
     * @node a2ui_update_overlay @alias a2uiUpdateOverlay
     * @param elementRef — Reference to the BoundingBoxOverlay element
     * @param operation (optional) — What operation to perform
     * @param boxes — Array of detection bounding boxes
     * @impure has side effects / drives control flow
     */
    function updateOverlay({ elementRef: Struct, operation?: string, boxes: Struct[] }): void;

    // === UI/Elements/Query ===

    /**
     * Gets all child elements of a container
     * @node a2ui_query_children @alias a2uiQueryChildren
     * @param elementRef — Reference to the container element
     * @returns children — Array of child elements
     * @returns childIds — Array of child element IDs
     * @returns count — Number of children
     */
    function queryChildren({ elementRef: string }): { children: Struct, childIds: string[], count: int };

    /**
     * Gets elements whose IDs match a pattern
     * @node a2ui_query_elements_by_id @alias a2uiQueryElementsById
     * @param pattern — The pattern to match element IDs against
     * @param matchType — How to match: 'starts_with', 'ends_with', 'contains', or 'exact'
     * @returns elements — Array of matching elements
     * @returns elementIds — Array of matching element IDs
     * @returns count — Number of matching elements
     */
    function queryElementsById({ pattern: string, matchType: string }): { elements: Struct, elementIds: string[], count: int };

    /**
     * Gets all elements of a specific component type
     * @node a2ui_query_elements_by_type @alias a2uiQueryElementsByType
     * @param componentType — The type of component to query (e.g., 'button', 'text', 'textField')
     * @returns elements — Array of matching elements
     * @returns count — Number of matching elements
     */
    function queryElementsByType({ componentType: string }): { elements: Struct, count: int };

    /**
     * Gets the parent element of an element
     * @node a2ui_query_parent @alias a2uiQueryParent
     * @param elementRef — Reference to the element to find parent of
     * @returns parent — The parent element data
     * @returns parentId — ID of the parent element
     * @returns hasParent — Whether a parent was found
     */
    function queryParent({ elementRef: string }): { parent: Struct, parentId: string, hasParent: bool };

    // === UI/Elements/Select ===

    /**
     * Gets the selected value of a select element
     * @node a2ui_get_select_value @alias a2uiGetSelectValue
     * @param elementRef — Reference to the select element
     * @returns value — The currently selected value
     * @returns hasSelection — Whether a value is selected
     */
    function getSelectValue({ elementRef: Struct }): { value: string, hasSelection: bool };

    /**
     * Sets the available options in a select element
     * @node a2ui_set_select_options @alias a2uiSetSelectOptions
     * @param elementRef — Reference to the select element
     * @param options — Array of options [{value, label}] or simple strings
     * @impure has side effects / drives control flow
     */
    function setSelectOptions({ elementRef: Struct, options: any }): void;

    /**
     * Sets the selected value of a select element
     * @node a2ui_set_select_value @alias a2uiSetSelectValue
     * @param elementRef — Element ID string or element object from Get Element
     * @param value — The value to select
     * @impure has side effects / drives control flow
     */
    function setSelectValue({ elementRef: Struct, value: string }): void;

    // === UI/Elements/Set ===

    /**
     * Sets the content text of a tooltip element
     * @node a2ui_set_tooltip_content @alias a2uiSetTooltipContent
     * @param elementRef — Reference to the tooltip element
     * @param content — The content text to set
     * @impure has side effects / drives control flow
     */
    function setTooltipContent({ elementRef: Struct, content: string }): void;

    // === UI/Elements/Slider ===

    /**
     * Sets the value of a slider element
     * @node a2ui_set_slider_value @alias a2uiSetSliderValue
     * @param elementRef — Reference to the slider element
     * @param value — The new slider value
     * @impure has side effects / drives control flow
     */
    function setSliderValue({ elementRef: Struct, value: float }): void;

    // === UI/Elements/Table ===

    /**
     * Add, remove, or update table data and structure
     * @node a2ui_update_table @alias a2uiUpdateTable
     * @param elementRef — Reference to the table element
     * @param operation (optional) — What operation to perform
     * @param data — Array of row objects
     * @impure has side effects / drives control flow
     */
    function updateTable({ elementRef: Struct, operation?: string, data: Struct }): void;

    /**
     * Push CSV or Table data directly to a table element
     * @node a2ui_write_csv_to_table @alias a2uiWriteCsvToTable
     * @param elementRef — Reference to the table element
     * @param csv — CSV text with headers
     * @param table — Table data from DataFusion query
     * @param delimiter (optional) — CSV delimiter (default: comma)
     * @impure has side effects / drives control flow
     */
    function writeCsvToTable({ elementRef: Struct, csv: string, table: Struct, delimiter?: string }): void;

    // === UI/Navigation ===

    /**
     * Closes an open dialog. If no dialog ID is specified, closes the topmost dialog.
     * @node a2ui_close_dialog @alias a2uiCloseDialog
     * @param dialogId — Optional ID of the specific dialog to close. If empty, closes the topmost dialog.
     * @impure has side effects / drives control flow
     */
    function closeDialog({ dialogId: string }): void;

    /**
     * Gets the current page route from the execution context
     * @node a2ui_get_current_route @alias a2uiGetCurrentRoute
     * @returns route — The current route path
     * @impure has side effects / drives control flow
     */
    function getCurrentRoute(): string;

    /**
     * Gets query parameters from the current URL
     * @node a2ui_get_query_params @alias a2uiGetQueryParams
     * @param paramName — The name of the query parameter to get (optional - if empty, returns all params)
     * @returns value — The parameter value (string if param_name specified, object if all params)
     * @returns exists — Whether the parameter exists
     * @impure has side effects / drives control flow
     */
    function getQueryParam({ paramName: string }): { value: any, exists: bool };

    /**
     * Gets route parameters from the current URL
     * @node a2ui_get_route_params @alias a2uiGetRouteParams
     * @param paramName — The name of the route parameter to get (optional - if empty, returns all params)
     * @returns value — The parameter value (string if param_name specified, object if all params)
     * @returns exists — Whether the parameter exists
     * @impure has side effects / drives control flow
     */
    function getRouteParam({ paramName: string }): { value: any, exists: bool };

    /**
     * Navigates to a page route
     * @node a2ui_navigate_to @alias a2uiNavigateTo
     * @param route — The route to navigate to (e.g., /dashboard, /users/123)
     * @param queryParams (optional) — Optional query parameters as key-value pairs (e.g., {"tab": "settings", "id": "123"})
     * @param replace (optional) — If true, replaces the current history entry instead of adding a new one
     * @impure has side effects / drives control flow
     */
    function navigateTo({ route: string, queryParams?: Struct, replace?: bool }): void;

    /**
     * Opens a route/page as a modal dialog overlay
     * @node a2ui_open_dialog @alias a2uiOpenDialog
     * @param route — The route path to open in the dialog (e.g., /settings, /edit/123)
     * @param title — Optional dialog title (shown in header)
     * @param queryParams — Optional JSON object of query parameters to pass to the route
     * @param dialogId — Optional unique ID for the dialog (for closing specific dialogs)
     * @impure has side effects / drives control flow
     */
    function openDialog({ route: string, title: string, queryParams: string, dialogId: string }): void;

    /**
     * Sets or updates a query parameter in the URL
     * @node a2ui_set_query_param @alias a2uiSetQueryParam
     * @param key — The query parameter key to set
     * @param value — The value to set (empty string removes the param)
     * @param replace — If true, replaces the current history entry instead of adding a new one
     * @impure has side effects / drives control flow
     */
    function setQueryParam({ key: string, value: string, replace: bool }): void;

    /**
     * Decodes a URL-encoded (percent-encoded) string
     * @node a2ui_url_decode @alias a2uiUrlDecode
     * @param input — The URL-encoded string to decode
     * @returns decoded — The decoded string
     * @returns success — Whether the decoding was successful
     */
    function urlDecode({ input: string }): { decoded: string, success: bool };

    /**
     * Encodes a string for safe use in URLs (percent-encoding)
     * @node a2ui_url_encode @alias a2uiUrlEncode
     * @param input — The string to URL-encode
     * @returns encoded — The URL-encoded string
     */
    function urlEncode({ input: string }): string;

    // === UI/State ===

    /**
     * Gets a value from global state by key
     * @node a2ui_get_global_state @alias a2uiGetGlobalState
     * @param key — The key to retrieve from global state
     * @returns value — The value stored at the key
     * @returns exists — Whether the key exists in global state
     */
    function getGlobalState({ key: string }): { value: any, exists: bool };

    /**
     * Gets a value from page-local state by key
     * @node a2ui_get_page_state @alias a2uiGetPageState
     * @param key — The key to retrieve from page state
     * @returns value — The value stored at the key
     * @returns exists — Whether the key exists in page state
     */
    function getPageState({ key: string }): { value: any, exists: bool };

    /**
     * Sets a value in global state by key
     * @node a2ui_set_global_state @alias a2uiSetGlobalState
     * @param key — The key to store the value at
     * @param value — The value to store
     * @impure has side effects / drives control flow
     */
    function setGlobalState({ key: string, value: any }): void;

    /**
     * Sets a value in page-local state by key
     * @node a2ui_set_page_state @alias a2uiSetPageState
     * @param key — The key to store the value at
     * @param value — The value to store
     * @impure has side effects / drives control flow
     */
    function setPageState({ key: string, value: any }): void;

    // === UI/Surface ===

    /**
     * Sends a surface to the frontend to begin rendering
     * @node a2ui_begin_rendering @alias a2uiBeginRendering
     * @param surface — The surface to render
     * @param components — Array of components to include
     * @param dataModel — Initial data model for bindings
     * @impure has side effects / drives control flow
     */
    function beginRendering({ surface: Struct, components: Struct[], dataModel: Struct }): void;

    /**
     * Creates a new A2UI surface with an ID and root component
     * @node a2ui_create_surface @alias a2uiCreateSurface
     * @param surfaceId (optional) — Unique identifier for the surface
     * @param rootComponentId (optional) — ID of the root component in the surface
     * @param catalogId — Optional custom component catalog
     * @returns surface — The created surface for adding components
     * @impure has side effects / drives control flow
     */
    function createSurface({ surfaceId?: string, rootComponentId?: string, catalogId: string }): Struct;

    /**
     * Removes a surface from the frontend
     * @node a2ui_delete_surface @alias a2uiDeleteSurface
     * @param surfaceId (optional) — ID of the surface to delete
     * @impure has side effects / drives control flow
     */
    function deleteSurface({ surfaceId?: string }): void;

    /**
     * Sets or clears scoped custom CSS for a custom UI surface at runtime
     * @node a2ui_set_surface_custom_css @alias a2uiSetSurfaceCustomCss
     * @param surfaceId (optional) — ID of the custom UI surface to update
     * @param customCss (optional) — CSS to apply to the surface. Leave empty to clear it.
     * @impure has side effects / drives control flow
     */
    function setSurfaceCustomCss({ surfaceId?: string, customCss?: string }): void;

    /**
     * Shows the current frontend screen while the workflow continues running
     * @node a2ui_show_screen @alias a2uiShowScreen
     * @impure has side effects / drives control flow
     */
    function showScreen(): void;

    /**
     * Updates components in an existing surface
     * @node a2ui_surface_update @alias a2uiSurfaceUpdate
     * @param surfaceId (optional) — ID of the surface to update
     * @param components — Components to add or update
     * @impure has side effects / drives control flow
     */
    function surfaceUpdate({ surfaceId?: string, components: Struct[] }): void;
}

declare namespace user {
    // === Utils/User ===

    /**
     * Checks whether a project user effectively has a permission. Owner and Admin imply all permissions.
     * @node utils_user_check_user_permission @alias utilsUserCheckUserPermission
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param userId (optional) — User subject / user ID within the project.
     * @param permission (optional) — Permission name or bit value to check.
     * @returns hasPermission — True when the user effectively has the requested permission.
     * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
     * @returns found — True when a matching project user was found.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function checkPermission({ appId?: string, userId?: string, permission?: string }): { hasPermission: bool, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

    /**
     * Checks whether a project user has the specified role ID or exact role name.
     * @node utils_user_check_user_has_role @alias utilsUserCheckUserHasRole
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param userId (optional) — User subject / user ID within the project.
     * @param role (optional) — Role ID or exact role name.
     * @returns hasRole — True when the user has the requested role.
     * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
     * @returns found — True when a matching project user was found.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function checkRole({ appId?: string, userId?: string, role?: string }): { hasRole: bool, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

    /**
     * Checks for one custom role attribute on a project user.
     * @node utils_user_get_user_attribute @alias utilsUserGetUserAttribute
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param userId (optional) — User subject / user ID within the project.
     * @param attribute (optional) — Role attribute to read.
     * @returns hasAttribute — True when the user has the requested attribute.
     * @returns attributeValue — The matching attribute when present.
     * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
     * @returns found — True when a matching project user was found.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function getAttribute({ appId?: string, userId?: string, attribute?: string }): { hasAttribute: bool, attributeValue: string, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

    /**
     * Gets custom role attributes assigned to a project user.
     * @node utils_user_get_user_attributes @alias utilsUserGetUserAttributes
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param userId (optional) — User subject / user ID within the project.
     * @returns userAttributes — Role attributes for the project user.
     * @returns found — True when the user was found.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function getAttributes({ appId?: string, userId?: string }): { userAttributes: Struct, found: bool, success: bool, statusCode: int, error: string };

    /**
     * Gets the current runtime user and, when available, their project membership, role, effective permissions, and attributes.
     * @node utils_user_get_current_user @alias utilsUserGetCurrentUser
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @returns currentUser — Current runtime user with project membership details when available.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function getCurrent({ appId?: string }): { currentUser: Struct, success: bool, statusCode: int, error: string };

    /**
     * Fetches the current user's persisted user information from the configured FlowLike hub's /api/v1/user/info endpoint when an execution token is available.
     * @node utils_user_get_current_user_info @alias utilsUserGetCurrentUserInfo
     * @returns userInfo — The user record returned by /api/v1/user/info
     * @returns success — True when user info was fetched successfully
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made
     * @returns error — Error message when user info could not be fetched
     */
    function getCurrentInfo(): { userInfo: Struct, success: bool, statusCode: int, error: string };

    /**
     * Gets a project user's effective permission bitfield and expanded permission names. Owner and Admin imply all permissions.
     * @node utils_user_get_effective_user_permissions @alias utilsUserGetEffectiveUserPermissions
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param userId (optional) — User subject / user ID within the project.
     * @returns userPermissions — Effective permissions for the project user.
     * @returns found — True when the user was found.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function getEffectivePermissions({ appId?: string, userId?: string }): { userPermissions: Struct, found: bool, success: bool, statusCode: int, error: string };

    /**
     * Gets the user context of the current execution. Returns a typed struct containing sub (user ID), role, permissions, attributes, and details of the calling principal. Use 'Break Struct' to access individual fields.
     * @node utils_user_get_executing_user @alias utilsUserGetExecutingUser
     * @returns userContext — The complete user execution context. Use 'Break Struct' to access: sub, role (with id, name, permissions, attributes), isTechnicalUser, keyId, principal, originAppId, onBehalfOf
     * @returns hasUser — True if user context is available
     */
    function getExecuting(): { userContext: Struct, hasUser: bool };

    /**
     * Gets a project user membership by user ID/sub.
     * @node utils_user_get_project_user @alias utilsUserGetProjectUser
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param userId (optional) — User subject / user ID within the project.
     * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
     * @returns found — True when a matching project user was found.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function getProjectUser({ appId?: string, userId?: string }): { projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

    /**
     * Gets the project role assigned to a user.
     * @node utils_user_get_user_roles @alias utilsUserGetUserRoles
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param userId (optional) — User subject / user ID within the project.
     * @returns userRoles — Role assignment for the project user. Current projects have one role per user.
     * @returns found — True when the user was found.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function getRoles({ appId?: string, userId?: string }): { userRoles: Struct, found: bool, success: bool, statusCode: int, error: string };

    /**
     * Checks if the executing user's role has a specific attribute (tag). Attributes are custom string tags assigned to roles for flexible authorization. Returns false if no user context is available or the user has no role.
     * @node utils_user_has_attribute @alias utilsUserHasAttribute
     * @param attribute (optional) — The attribute (tag) to check for
     * @returns hasAttribute — True if the user's role has the specified attribute
     */
    function hasAttribute({ attribute?: string }): bool;

    /**
     * Checks if the executing user has a specific permission. Admin and Owner roles automatically have all permissions. Returns false if no user context is available.
     * @node utils_user_has_permission @alias utilsUserHasPermission
     * @param permission (optional) — The permission to check for
     * @returns hasPermission — True if the user has the specified permission (or is Admin/Owner)
     */
    function hasPermission({ permission?: string }): bool;

    /**
     * Checks whether a machine rather than a person triggered this run. Machine callers have no human identity (sub): an API key reports its Key ID, an app calling through an app connection reports the calling app instead.
     * @node utils_user_is_technical_user @alias utilsUserIsTechnicalUser
     * @returns isTechnical — True if a machine triggered the run (API key or app connection), false for a person
     * @returns keyId — The API key identifier, empty for every other caller
     * @returns principal — How the caller authenticated: 'user', 'apiKey' or 'connectedApp'
     * @returns originAppId — The app that made the call when the principal is 'connectedApp', empty otherwise
     * @returns onBehalfOf — The user the caller reported as the initiator: an API key's creator, or the user an app connection passed through. Attribution only — never authorize against it
     */
    function isTechnical(): { isTechnical: bool, keyId: string, principal: string, originAppId: string, onBehalfOf: string };

    /**
     * Lists project users with pagination.
     * @node utils_user_list_project_users @alias utilsUserListProjectUsers
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param offset (optional) — Number of matching users to skip.
     * @param limit (optional) — Maximum number of users to return, capped at 100.
     * @returns users — Matching project users.
     * @returns count — Number of users returned.
     * @returns nextOffset — Offset to use for the next page.
     * @returns hasMore — True when another page may contain more matching users.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function listProjectUsers({ appId?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

    /**
     * Lists project users whose assigned role contains a custom attribute.
     * @node utils_user_list_users_with_attribute @alias utilsUserListUsersWithAttribute
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param attribute (optional) — Role attribute to match.
     * @param offset (optional) — Number of matching users to skip.
     * @param limit (optional) — Maximum number of users to return, capped at 100.
     * @returns users — Matching project users.
     * @returns count — Number of users returned.
     * @returns nextOffset — Offset to use for the next page.
     * @returns hasMore — True when another page may contain more matching users.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function listWithAttribute({ appId?: string, attribute?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

    /**
     * Lists project users assigned to a role ID or exact role name.
     * @node utils_user_list_users_with_role @alias utilsUserListUsersWithRole
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param role (optional) — Role ID or exact role name. Leave empty to return all project users.
     * @param offset (optional) — Number of matching users to skip.
     * @param limit (optional) — Maximum number of users to return, capped at 100.
     * @returns users — Matching project users.
     * @returns count — Number of users returned.
     * @returns nextOffset — Offset to use for the next page.
     * @returns hasMore — True when another page may contain more matching users.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function listWithRole({ appId?: string, role?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

    /**
     * Resolves a project user by user ID/sub or by email when email is exposed by platform lookup settings. Email matching is constrained to project members.
     * @node utils_user_resolve_user @alias utilsUserResolveUser
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param identifier (optional) — Email, sub, or user ID to resolve within the project.
     * @param identifierType (optional) — How to interpret the identifier.
     * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
     * @returns found — True when a matching project user was found.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function resolve({ appId?: string, identifier?: string, identifierType?: string }): { projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

    /**
     * Searches project users by exposed profile fields. Email is only searchable when the platform returns email in user lookup results.
     * @node utils_user_search_users @alias utilsUserSearchUsers
     * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
     * @param query (optional) — Search text matched against project user ID, username, preferred username, name, visible email, or role name.
     * @param offset (optional) — Number of matching users to skip.
     * @param limit (optional) — Maximum number of users to return, capped at 100.
     * @returns users — Matching project users.
     * @returns count — Number of users returned.
     * @returns nextOffset — Offset to use for the next page.
     * @returns hasMore — True when another page may contain more matching users.
     * @returns success — True when the read operation completed successfully.
     * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
     * @returns error — Error message when the read operation could not complete.
     */
    function search({ appId?: string, query?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };
}

declare namespace variable {
    // === Variable ===

    /**
     * Get Variable Value
     * @node variable_get @alias variableGet
     * @param varRef — The reference to the variable
     * @returns valueRef — The value of the variable
     */
    function get({ varRef: string }): any;

    /**
     * Set Variable Value
     * @node variable_set @alias variableSet
     * @param varRef — The reference to the variable
     * @param valueIn — The value of the variable
     * @returns valueRef — The newly set value
     * @impure has side effects / drives control flow
     */
    function set({ varRef: string, valueIn: any }): any;
}
