// Utils — FlowScript node declarations (generated, do not edit).
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
