// Utils — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Utils ===

/**
 * Generates a Collision Resistant Unique Identifier
 * @returns cuid — Generated CUID
 * @impure has side effects / drives control flow
 */
declare function cuid(): string;

/**
 * A random identifier
 * @param uppercase (optional) — Write the hex digits in upper case
 * @returns uuid — A random identifier
 * @impure has side effects / drives control flow
 */
declare function uuidV4({ uppercase?: bool }): string;

/**
 * A time ordered identifier — sorts by creation time, which keeps database indexes tidy
 * @param uppercase (optional) — Write the hex digits in upper case
 * @returns uuid — A time ordered identifier — sorts by creation time, which keeps database indexes tidy
 * @impure has side effects / drives control flow
 */
declare function uuidV7({ uppercase?: bool }): string;


// === Utils/Array ===

/**
 * Splits an array into batches of a fixed size
 * @param arrayIn — Your Array
 * @param size (optional) — Elements per batch
 * @returns chunks — One entry per batch, each holding up to Size elements
 * @returns chunkCount — How many batches were produced
 */
declare function arrayChunk({ arrayIn: any[], size?: int }): { chunks: any[], chunkCount: int };

/**
 * Removes all elements from an array
 * @param arrayIn — Your Array
 * @returns arrayOut — Empty Array
 * @impure has side effects / drives control flow
 */
declare function arrayClear({ arrayIn: any[] }): any[];

/**
 * Append an Array to another Array
 * @param arrayIn — Your Array
 * @param values — Value to push
 * @returns arrayOut — Adjusted Array
 * @impure has side effects / drives control flow
 */
declare function arrayExtend({ arrayIn: any[], values: any[] }): any[];

/**
 * Keeps the elements whose key passes a comparison
 * @param arrayIn — Your Array
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
declare function arrayFilterBy({ arrayIn: any[], key?: string, operator?: string, value?: string, compare?: string, ignoreCase?: bool, invert?: bool }): { arrayOut: any[], kept: int, removed: int };

/**
 * Removes a specific field from every struct in an array. Elements without the field are kept unchanged. Returns the filtered array and count of removed fields.
 * @param arrayIn — Array of structs to filter
 * @param field — Field name to remove from each struct
 * @returns arrayOut — Array with the field removed from each struct
 * @returns removedCount — Number of fields that were removed
 * @impure has side effects / drives control flow
 */
declare function arrayFilterField({ arrayIn: Struct[], field: string }): { arrayOut: Struct[], removedCount: int };

/**
 * Removes multiple fields from every struct in an array. Elements without the fields are kept unchanged. Returns the filtered array and count of removed fields.
 * @param arrayIn — Array of structs to filter
 * @param fields — Array of field names to remove from each struct
 * @returns arrayOut — Array with the fields removed from each struct
 * @returns removedCount — Total number of fields that were removed
 * @impure has side effects / drives control flow
 */
declare function arrayFilterFields({ arrayIn: Struct[], fields: string[] }): { arrayOut: Struct[], removedCount: int };

/**
 * Finds the index of an item in an array
 * @param arrayIn — Your Array
 * @param item — Item to find
 * @returns index — Index of the item (-1 if not found)
 * @returns found — Was the item found?
 * @impure has side effects / drives control flow
 */
declare function arrayFindItem({ arrayIn: any[], item: any }): { index: int, found: bool };

/**
 * Pulls nested arrays up into a single array
 * @param arrayIn — Your Array
 * @param depth (optional) — How many levels to flatten, -1 for all of them
 * @returns arrayOut — The flattened array
 */
declare function arrayFlatten({ arrayIn: any[], depth?: int }): any[];

/**
 * Gets an element from an array by index
 * @param arrayIn — Your Array
 * @param index — Index of the element to get
 * @returns element — Element at the specified index
 * @returns success — Was the get successful?
 */
declare function arrayGet({ arrayIn: any[], index: int }): { element: any, success: bool };

/**
 * Groups elements that share the same key value
 * @param arrayIn — Your Array
 * @param key (optional) — Field to read from each element, dot notation for nested fields (customer.address.city). Empty uses the element itself
 * @returns groups — One entry per distinct key, in first-seen order
 * @returns groupCount — How many distinct keys were found
 */
declare function arrayGroupBy({ arrayIn: any[], key?: string }): { groups: Struct[], groupCount: int };

/**
 * Checks if an array includes a certain value
 * @param arrayIn — Your Array
 * @param value — Value to search for
 * @returns includes — Does the array include the value?
 */
declare function arrayIncludes({ arrayIn: any[], value: any }): bool;

/**
 * Matches the elements of two arrays on a shared key, the way a database join does
 * @param arrayLeft — Left Array
 * @param arrayRight — Right Array
 * @param keyLeft (optional) — Field on the left elements, dot notation for nested fields. Empty uses the element itself
 * @param keyRight (optional) — Field on the right elements. Empty reuses the left key
 * @param join (optional) — Inner keeps only matches, Left keeps every left element
 * @returns pairs — One entry per match, holding both sides
 * @returns matched — How many left elements found a partner
 */
declare function arrayJoinBy({ arrayLeft: any[], arrayRight: any[], keyLeft?: string, keyRight?: string, join?: string }): { pairs: Struct[], matched: int };

/**
 * Gets the length of an array
 * @param array — Input Array
 * @returns length — Length of the array
 */
declare function arrayLength({ array: any[] }): int;

/**
 * The element with the largest key
 * @param arrayIn — Your Array
 * @param compare (optional) — How the key values are ordered. Auto reads each value and falls back to text
 * @param nulls (optional) — Where elements without a key value end up
 * @returns element — The element with the largest key
 * @returns index — Position of the element in the array
 * @returns found — False when the array was empty
 */
declare function arrayMaxBy({ arrayIn: any[], compare?: string, nulls?: string }): { element: any, index: int, found: bool };

/**
 * The element with the smallest key
 * @param arrayIn — Your Array
 * @param compare (optional) — How the key values are ordered. Auto reads each value and falls back to text
 * @param nulls (optional) — Where elements without a key value end up
 * @returns element — The element with the smallest key
 * @returns index — Position of the element in the array
 * @returns found — False when the array was empty
 */
declare function arrayMinBy({ arrayIn: any[], compare?: string, nulls?: string }): { element: any, index: int, found: bool };

/**
 * Reads one field out of every element
 * @param arrayIn — Your Array
 * @param key (optional) — Field to read from each element, dot notation for nested fields (customer.address.city). Empty uses the element itself
 * @param skipMissing (optional) — Drop elements that do not have the field instead of emitting null
 * @returns values — The field value of every element
 */
declare function arrayPluck({ arrayIn: any[], key?: string, skipMissing?: bool }): any[];

/**
 * Removes and returns the last element of an array
 * @param arrayIn — Your Array
 * @returns arrayOut — Adjusted Array
 * @returns value — Popped Value
 * @impure has side effects / drives control flow
 */
declare function arrayPop({ arrayIn: any[] }): { arrayOut: any[], value: any };

/**
 * Push an item into your Array
 * @param arrayIn — Your Array
 * @param value — Value to push
 * @returns arrayOut — Adjusted Array
 * @impure has side effects / drives control flow
 */
declare function arrayPush({ arrayIn: any[], value: any }): any[];

/**
 * Removes an element from an array at a specific index
 * @param arrayIn — Your Array
 * @param index — Index to remove
 * @returns arrayOut — Adjusted Array
 * @impure has side effects / drives control flow
 */
declare function arrayRemoveIndex({ arrayIn: any[], index: int }): any[];

/**
 * The reversed array
 * @param arrayIn — Your Array
 * @returns arrayOut — The reversed array
 */
declare function arrayReverse({ arrayIn: any[] }): any[];

/**
 * Sets an element at a specific index in an array
 * @param arrayIn — Your Array
 * @param index — Index to set
 * @param value — Value to set
 * @returns arrayOut — Adjusted Array
 * @impure has side effects / drives control flow
 */
declare function arraySetIndex({ arrayIn: any[], index: int, value: any }): any[];

/**
 * Shuffle Array Items
 * @param arrayIn — Your Array
 * @returns arrayOut — Adjusted Array
 */
declare function arrayShuffle({ arrayIn: any[] }): any[];

/**
 * The selected range of elements
 * @param arrayIn — Your Array
 * @param start (optional) — First index, negative counts from the end
 * @param length (optional) — Number of elements to take, -1 for the rest of the array
 * @returns arrayOut — The selected range of elements
 */
declare function arraySlice({ arrayIn: any[], start?: int, length?: int }): any[];

/**
 * The sorted array
 * @param arrayIn — Your Array
 * @param descending (optional) — Sort from largest to smallest
 * @param compare (optional) — How the key values are ordered. Auto reads each value and falls back to text
 * @param nulls (optional) — Where elements without a key value end up
 * @returns arrayOut — The sorted array
 */
declare function arraySort({ arrayIn: any[], descending?: bool, compare?: string, nulls?: string }): any[];

/**
 * Adds up one numeric field across an array of structs
 * @param arrayIn — Your Array
 * @param field (optional) — Field to add up, empty sums the values themselves
 * @returns sum — Sum of the field
 * @returns counted — How many entries held a number
 */
declare function arraySumField({ arrayIn: any[], field?: string }): { sum: float, counted: int };

/**
 * The array without duplicate values
 * @param arrayIn — Your Array
 * @returns arrayOut — The array without duplicates
 * @returns removed — How many duplicates were dropped
 */
declare function arrayUnique({ arrayIn: any[] }): { arrayOut: any[], removed: int };

/**
 * Pairs up the elements of two arrays, stopping at the shorter one
 * @param arrayFirst — First Array
 * @param arraySecond — Second Array
 * @returns pairs — One entry per index holding both values
 */
declare function arrayZip({ arrayFirst: any[], arraySecond: any[] }): Struct[];

/**
 * Creates an array from individual elements. Add more input pins by connecting to the 'element' pins.
 * @param element — Element to include in the array
 * @param element — Element to include in the array
 * @returns arrayOut — The constructed array
 */
declare function constructArray({ element: any, element: any }): any[];

/**
 * Creates an empty array
 * @returns arrayOut — The created array
 */
declare function makeArray(): any[];


// === Utils/Array/Batch ===

/**
 * Push multiple items into an array in one operation. More efficient than multiple single pushes.
 * @param arrayIn — Your Array
 * @param items — Array of items to push
 * @returns arrayOut — Array with all items pushed
 * @impure has side effects / drives control flow
 */
declare function arrayBatchPush({ arrayIn: any[], items: any[] }): any[];

/**
 * Remove multiple elements at specific indices in one operation. More efficient than multiple single removes. Indices are processed in descending order to maintain correctness.
 * @param arrayIn — Your Array
 * @param indices — Array of indices to remove
 * @returns arrayOut — Array with elements removed
 * @returns removed — Array of removed values
 * @impure has side effects / drives control flow
 */
declare function arrayBatchRemove({ arrayIn: any[], indices: int[] }): { arrayOut: any[], removed: any[] };

/**
 * Set multiple elements at specific indices in one operation. More efficient than multiple single sets.
 * @param arrayIn — Your Array
 * @param indices — Array of indices to set
 * @param values — Array of values to set (must match indices length)
 * @returns arrayOut — Array with all values set
 * @impure has side effects / drives control flow
 */
declare function arrayBatchSet({ arrayIn: any[], indices: int[], values: any[] }): any[];


// === Utils/Array/By Reference ===

/**
 * Clear all elements directly from a variable array without copying.
 * @param varRef — Reference to the array variable to clear
 * @impure has side effects / drives control flow
 */
declare function arrayClearRef({ varRef: string }): void;

/**
 * Append multiple items directly to a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @param values — Array of values to append
 * @impure has side effects / drives control flow
 */
declare function arrayExtendRef({ varRef: string, values: any[] }): void;

/**
 * Remove and return the last element directly from a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @returns value — The popped value
 * @impure has side effects / drives control flow
 */
declare function arrayPopRef({ varRef: string }): any;

/**
 * Push an item directly into a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @param value — Value to push into the array
 * @impure has side effects / drives control flow
 */
declare function arrayPushRef({ varRef: string, value: any }): void;

/**
 * Remove an element at a specific index directly from a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @param index — Index to remove
 * @returns value — The removed value
 * @impure has side effects / drives control flow
 */
declare function arrayRemoveIndexRef({ varRef: string, index: int }): any;

/**
 * Set an element at a specific index directly in a variable array without copying. Much faster for large arrays.
 * @param varRef — Reference to the array variable to modify
 * @param index — Index to set
 * @param value — Value to set at the index
 * @impure has side effects / drives control flow
 */
declare function arraySetIndexRef({ varRef: string, index: int, value: any }): void;


// === Utils/Bool ===

/**
 * True when every boolean in the array is true
 * @param booleans — Input Booleans
 * @returns result — True when every boolean in the array is true
 */
declare function boolAll({ booleans: bool[] }): bool;

/**
 * Boolean And operation
 * @param boolean (optional) — Input Pin for AND Operation
 * @param boolean (optional) — Input Pin for AND Operation
 * @returns result — AND operation between all boolean inputs
 */
declare function boolAnd({ boolean?: bool, boolean?: bool }): bool;

/**
 * True when at least one boolean in the array is true
 * @param booleans — Input Booleans
 * @returns result — True when at least one boolean in the array is true
 * @returns count — How many values were true
 */
declare function boolAny({ booleans: bool[] }): { result: bool, count: int };

/**
 * Boolean Equal
 * @param boolean (optional) — Input Pin for OR Operation
 * @param boolean (optional) — Input Pin for OR Operation
 * @returns result — == operation between all boolean inputs
 */
declare function boolEqual({ boolean?: bool, boolean?: bool }): bool;

/**
 * False only when the premise is true and the conclusion is false
 * @param premise (optional) — The condition that is assumed
 * @param conclusion (optional) — What has to hold when the premise is true
 * @returns result — True when the implication holds
 */
declare function boolImplies({ premise?: bool, conclusion?: bool }): bool;

/**
 * True unless every input is true
 * @param boolean (optional) — Input Boolean
 * @param boolean (optional) — Input Boolean
 * @returns result — True unless every input is true
 */
declare function boolNand({ boolean?: bool, boolean?: bool }): bool;

/**
 * True only when every input is false
 * @param boolean (optional) — Input Boolean
 * @param boolean (optional) — Input Boolean
 * @returns result — True only when every input is false
 */
declare function boolNor({ boolean?: bool, boolean?: bool }): bool;

/**
 * Boolean NOT
 * @param boolean (optional) — Input Boolean
 * @returns result — NOT operation on the input
 */
declare function boolNot({ boolean?: bool }): bool;

/**
 * Boolean Or operation
 * @param boolean (optional) — Input Pin for OR Operation
 * @param boolean (optional) — Input Pin for OR Operation
 * @returns result — OR operation between all boolean inputs
 */
declare function boolOr({ boolean?: bool, boolean?: bool }): bool;

/**
 * Converts a boolean into 1 or 0
 * @param boolean (optional) — Input Boolean
 * @returns integer — 1 when true, 0 when false
 */
declare function boolToInt({ boolean?: bool }): int;

/**
 * Converts a boolean into text
 * @param boolean (optional) — Input Boolean
 * @param trueText (optional) — Text used when the boolean is true
 * @param falseText (optional) — Text used when the boolean is false
 * @returns string — The text
 */
declare function boolToString({ boolean?: bool, trueText?: string, falseText?: string }): string;

/**
 * Flips a boolean variable in place
 * @param varRef — Reference to the boolean variable to flip
 * @returns newValue — The value the variable holds after flipping
 * @impure has side effects / drives control flow
 */
declare function boolToggle({ varRef: string }): bool;

/**
 * Checks whether two booleans differ
 * @param boolean1 (optional) — Input Boolean
 * @param boolean2 (optional) — Input Boolean
 * @returns result — True when the booleans differ
 */
declare function boolUnequal({ boolean1?: bool, boolean2?: bool }): bool;

/**
 * Boolean XOR
 * @param boolean (optional) — Input Boolean
 * @param boolean (optional) — Input Boolean
 * @returns result — XOR operation between all boolean inputs
 */
declare function boolXor({ boolean?: bool, boolean?: bool }): bool;

/**
 * Converts an integer into a boolean, zero is false
 * @param integer (optional) — Input Integer
 * @returns boolean — False when the integer was zero
 */
declare function intToBool({ integer?: int }): bool;

/**
 * Generates a random boolean value
 * @param probability (optional) — The probability of the boolean being true
 * @returns value — The random boolean value
 */
declare function randomBool({ probability?: float }): bool;


// === Utils/Bytes ===

/**
 * Appends byte buffers to each other
 * @param bytes — Part to append
 * @param bytes — Part to append
 * @returns result — All parts appended in order
 */
declare function bytesConcat({ bytes: bytes[], bytes: bytes[] }): bytes[];

/**
 * Reads the leading bytes to work out what kind of file a buffer holds
 * @param bytes — Input Bytes
 * @returns mimeType — Detected media type, empty when nothing matched
 * @returns extension — Usual file extension for the detected type
 * @returns detected — True when a signature matched
 * @returns isText — True when the first kilobyte reads as UTF-8 text without null bytes
 */
declare function bytesDetectType({ bytes: bytes[] }): { mimeType: string, extension: string, detected: bool, isText: bool };

/**
 * Compares two byte buffers for equality
 * @param bytes — Input Bytes
 * @param other — Input Bytes
 * @returns equal — True when both buffers hold the same bytes
 */
declare function bytesEqual({ bytes: bytes[], other: bytes[] }): bool;

/**
 * Compresses a byte buffer with gzip
 * @param bytes — Input Bytes
 * @param level (optional) — Compression level from 0 (store) to 9 (smallest)
 * @returns result — The compressed bytes
 * @returns ratio — Compressed size divided by original size
 */
declare function bytesGzipCompress({ bytes: bytes[], level?: int }): { result: bytes[], ratio: float };

/**
 * Restores a gzip compressed byte buffer
 * @param bytes — Compressed Bytes
 * @param maxSize (optional) — Refuse to expand beyond this many bytes
 * @returns result — The restored bytes
 */
declare function bytesGzipDecompress({ bytes: bytes[], maxSize?: int }): bytes[];

/**
 * How many bytes the buffer holds
 * @param bytes — Input Bytes
 * @returns length — Number of bytes
 * @returns isEmpty — True when the buffer holds nothing
 */
declare function bytesLength({ bytes: bytes[] }): { length: int, isEmpty: bool };

/**
 * Takes a range out of a byte buffer
 * @param bytes — Input Bytes
 * @param start (optional) — First byte index, negative counts from the end
 * @param length (optional) — Number of bytes to take, -1 for the rest
 * @returns result — The selected bytes
 */
declare function bytesSlice({ bytes: bytes[], start?: int, length?: int }): bytes[];

/**
 * Checks a buffer against a leading byte sequence, for example a file signature
 * @param bytes — Input Bytes
 * @param prefix — Bytes to look for
 * @returns startsWith — True when the buffer begins with the prefix
 */
declare function bytesStartsWith({ bytes: bytes[], prefix: bytes[] }): bool;

/**
 * Reads a byte buffer as UTF-8 text
 * @param bytes — Input Bytes
 * @param lossy (optional) — Replace invalid sequences instead of failing
 * @returns text — The decoded text
 * @returns wasValid — False when the buffer was not valid UTF-8
 */
declare function bytesToText({ bytes: bytes[], lossy?: bool }): { text: string, wasValid: bool };

/**
 * Writes text out as UTF-8 bytes
 * @param text — Input Text
 * @returns bytes — The encoded bytes
 */
declare function textToBytes({ text: string }): bytes[];


// === Utils/CSV ===

/**
 * Stream Read a CSV File
 * @param csv — CSV Path
 * @param chunkSize (optional) — Chunk Size for Buffered Read
 * @param delimiter (optional) — Delimiter for CSV
 * @returns chunk — Chunk
 * @impure has side effects / drives control flow
 */
declare function csvBufferedReader({ csv: Struct, chunkSize?: int, delimiter?: string }): Struct[];


// === Utils/Conversions ===

/**
 * Convert String to Bytes
 * @param bytes — Bytes to convert
 * @returns value — Parsed Value
 */
declare function valFromBytes({ bytes: bytes[] }): any;

/**
 * Convert String to Struct
 * @param string — String to convert
 * @returns valueRef — Value of the Generic
 */
declare function valFromString({ string: string }): any;

/**
 * Convert Struct to Bytes
 * @param value — Input Value
 * @param pretty (optional) — Should the struct be pretty printed?
 * @returns bytes — Output Bytes
 */
declare function valToBytes({ value: any, pretty?: bool }): bytes[];

/**
 * Convert any object to String
 * @param value — Input Value
 * @param pretty (optional) — Should the struct be pretty printed?
 * @returns string — Output String
 */
declare function valToString({ value: any, pretty?: bool }): string;


// === Utils/Crypto ===

/**
 * Decrypts an AES-256-GCM encrypted payload and verifies its authentication tag.
 * @param key — 32-byte symmetric key
 * @param encrypted — Authenticated encrypted payload
 * @returns plaintext — Decrypted bytes
 * @impure has side effects / drives control flow
 */
declare function cryptoAesDecryptBytes({ key: bytes[], encrypted: Struct }): bytes[];

/**
 * Decrypts an AES-256-GCM payload and parses the plaintext as a struct.
 * @param key — 32-byte symmetric key
 * @param encrypted — Authenticated encrypted payload
 * @returns value — Decrypted struct
 * @impure has side effects / drives control flow
 */
declare function cryptoAesDecryptValue({ key: bytes[], encrypted: Struct }): Struct;

/**
 * Encrypts bytes with AES-256-GCM. A fresh nonce is generated internally for every encryption.
 * @param key — 32-byte symmetric key
 * @param plaintext — Bytes to encrypt
 * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
 * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
 * @impure has side effects / drives control flow
 */
declare function cryptoAesEncryptBytes({ key: bytes[], plaintext: bytes[], associatedData?: Struct }): Struct;

/**
 * Serializes and encrypts a struct with AES-256-GCM. A fresh nonce is generated internally for every encryption.
 * @param key — 32-byte symmetric key
 * @param value — Struct to encrypt
 * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
 * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
 * @impure has side effects / drives control flow
 */
declare function cryptoAesEncryptValue({ key: bytes[], value: Struct, associatedData?: Struct }): Struct;

/**
 * Generates a 256-bit symmetric key for AES-256-GCM and XChaCha20-Poly1305.
 * @returns key — Random 32-byte symmetric key
 * @impure has side effects / drives control flow
 */
declare function cryptoGenerateKey(): bytes[];

/**
 * Decrypts an XChaCha20-Poly1305 encrypted payload and verifies its authentication tag.
 * @param key — 32-byte symmetric key
 * @param encrypted — Authenticated encrypted payload
 * @returns plaintext — Decrypted bytes
 * @impure has side effects / drives control flow
 */
declare function cryptoXchacha20DecryptBytes({ key: bytes[], encrypted: Struct }): bytes[];

/**
 * Decrypts an XChaCha20-Poly1305 payload and parses the plaintext as a struct.
 * @param key — 32-byte symmetric key
 * @param encrypted — Authenticated encrypted payload
 * @returns value — Decrypted struct
 * @impure has side effects / drives control flow
 */
declare function cryptoXchacha20DecryptValue({ key: bytes[], encrypted: Struct }): Struct;

/**
 * Encrypts bytes with XChaCha20-Poly1305. A fresh 192-bit nonce is generated internally for every encryption.
 * @param key — 32-byte symmetric key
 * @param plaintext — Bytes to encrypt
 * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
 * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
 * @impure has side effects / drives control flow
 */
declare function cryptoXchacha20EncryptBytes({ key: bytes[], plaintext: bytes[], associatedData?: Struct }): Struct;

/**
 * Serializes and encrypts a struct with XChaCha20-Poly1305. A fresh 192-bit nonce is generated internally for every encryption.
 * @param key — 32-byte symmetric key
 * @param value — Struct to encrypt
 * @param associatedData (optional) — Optional authenticated metadata stored alongside the ciphertext
 * @returns encrypted — Authenticated encrypted payload with algorithm and generated nonce
 * @impure has side effects / drives control flow
 */
declare function cryptoXchacha20EncryptValue({ key: bytes[], value: Struct, associatedData?: Struct }): Struct;


// === Utils/DateTime ===

/**
 * Moves a date forward or back by working days, skipping weekends
 * @param date — Input Date
 * @param days (optional) — Working days to add, negative to go back
 * @returns result — The shifted date, always landing on a working day
 */
declare function utilsDatetimeAddBusinessDays({ date: Date, days?: int }): Date;

/**
 * Counts the working days between two dates, skipping weekends
 * @param start — Start of the range
 * @param end — End of the range
 * @param includeEnd (optional) — Count the end day itself when it is a working day
 * @returns days — Working days in the range, negative when the end lies before the start
 */
declare function utilsDatetimeBusinessDaysBetween({ start: Date, end: Date, includeEnd?: bool }): int;

/**
 * Week number, weekend and leap year facts about a date
 * @param date — Input Date
 * @returns isWeekend — True on Saturday and Sunday
 * @returns isLeapYear — True when February has 29 days that year
 * @returns week — ISO 8601 week number
 * @returns isoYear — Year the ISO week belongs to
 * @returns quarter — Quarter of the year, 1 to 4
 * @returns daysInMonth — Length of the month the date falls in
 */
declare function utilsDatetimeCalendarInfo({ date: Date }): { isWeekend: bool, isLeapYear: bool, week: int, isoYear: int, quarter: int, daysInMonth: int };

/**
 * Pulls a date into a range, leaving it alone when it already fits
 * @param date — Input Date
 * @param start — Earliest allowed date
 * @param end — Latest allowed date
 * @returns result — The date inside the range
 * @returns wasClamped — True when the date had to be moved
 */
declare function utilsDatetimeClamp({ date: Date, start: Date, end: Date }): { result: Date, wasClamped: bool };

/**
 * Calculates the duration between two dates
 * @param start — Start date
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
declare function utilsDatetimeDiff({ start: Date, end: Date }): { totalSeconds: int, days: int, hours: int, minutes: int, seconds: int, humanReadable: string, errorMessage: string };

/**
 * Adds or subtracts a duration from a date
 * @param date — Base date
 * @param days (optional) — Days to add (negative to subtract)
 * @param hours (optional) — Hours to add
 * @param minutes (optional) — Minutes to add
 * @param seconds (optional) — Seconds to add
 * @returns result — Resulting date
 */
declare function utilsDatetimeDuration({ date: Date, days?: int, hours?: int, minutes?: int, seconds?: int }): Date;

/**
 * The last instant of the day, week, month, quarter or year
 * @param date — Input Date
 * @param unit (optional) — Unit to snap to
 * @returns result — The last instant of the day, week, month, quarter or year
 */
declare function utilsDatetimeEndOf({ date: Date, unit?: string }): Date;

/**
 * Converts a DateTime to a formatted string
 * @param date — Date to format
 * @param format (optional) — Format string (e.g., '%Y-%m-%d %H:%M:%S', '%Y-%m-%d', 'rfc3339', 'rfc2822')
 * @returns formatted — Formatted string
 */
declare function utilsDatetimeFormat({ date: Date, format?: string }): string;

/**
 * Builds a date from year, month, day and time components
 * @param year (optional) — Year
 * @param month (optional) — Month
 * @param day (optional) — Day
 * @param hour (optional) — Hour
 * @param minute (optional) — Minute
 * @param second (optional) — Second
 * @returns date — The assembled date
 */
declare function utilsDatetimeFromParts({ year?: int, month?: int, day?: int, hour?: int, minute?: int, second?: int }): Date;

/**
 * Converts an epoch timestamp into a date
 * @param timestamp (optional) — Epoch timestamp
 * @param unit (optional) — Unit of the timestamp. Auto reads it from the magnitude
 * @returns date — The converted date
 */
declare function utilsDatetimeFromUnix({ timestamp?: int, unit?: string }): Date;

/**
 * Describes how far a date lies from now, for example "3 days ago"
 * @param date — Input Date
 * @param reference — What to measure against. Leave empty for the current time
 * @returns text — Relative description of the distance
 * @returns isPast — True when the date lies before the reference
 * @returns seconds — Signed distance in seconds, positive when the date is in the past
 */
declare function utilsDatetimeHumanize({ date: Date, reference: Date }): { text: string, isPast: bool, seconds: int };

/**
 * The later of two dates
 * @param date — Input Date
 * @param other — Input Date
 * @returns result — The later of two dates
 */
declare function utilsDatetimeMax({ date: Date, other: Date }): Date;

/**
 * The latest date in an array
 * @param dates — Input Dates
 * @returns result — The latest date in an array
 * @returns found — False when the array held no readable date
 */
declare function utilsDatetimeMaxOf({ dates: Date[] }): { result: Date, found: bool };

/**
 * The earlier of two dates
 * @param date — Input Date
 * @param other — Input Date
 * @returns result — The earlier of two dates
 */
declare function utilsDatetimeMin({ date: Date, other: Date }): Date;

/**
 * The earliest date in an array
 * @param dates — Input Dates
 * @returns result — The earliest date in an array
 * @returns found — False when the array held no readable date
 */
declare function utilsDatetimeMinOf({ dates: Date[] }): { result: Date, found: bool };

/**
 * Returns the current date and time in UTC
 * @returns date — Current UTC date and time
 * @impure has side effects / drives control flow
 */
declare function utilsDatetimeNow(): Date;

/**
 * Parses a string into a DateTime. Auto-detects common formats and epoch timestamps (seconds, milliseconds, microseconds, nanoseconds) or uses a custom format string.
 * @param input — String to parse
 * @param format (optional) — Optional format string (e.g., '%Y-%m-%d %H:%M:%S'). Leave empty for auto-detection.
 * @returns date — Parsed date
 */
declare function utilsDatetimeParse({ input: string, format?: string }): Date;

/**
 * Calendar-aware shift that keeps the day of month where it exists
 * @param date — Input Date
 * @param months (optional) — Months to add, negative to go back
 * @param years (optional) — Years to add, negative to go back
 * @returns result — The shifted date
 */
declare function utilsDatetimeShiftCalendar({ date: Date, months?: int, years?: int }): Date;

/**
 * The first instant of the day, week, month, quarter or year
 * @param date — Input Date
 * @param unit (optional) — Unit to snap to
 * @returns result — The first instant of the day, week, month, quarter or year
 */
declare function utilsDatetimeStartOf({ date: Date, unit?: string }): Date;

/**
 * Extracts date components from a DateTime
 * @param date — DateTime to extract from
 * @returns year — Year
 * @returns month — Month (1-12)
 * @returns day — Day of month (1-31)
 * @returns weekday — Day of week (0=Monday, 6=Sunday)
 * @returns dayOfYear — Day of year (1-366)
 */
declare function utilsDatetimeToDate({ date: Date }): { year: int, month: int, day: int, weekday: int, dayOfYear: int };

/**
 * Extracts time components from a DateTime
 * @param date — DateTime to extract from
 * @returns hour — Hour (0-23)
 * @returns minute — Minute (0-59)
 * @returns second — Second (0-59)
 * @returns nanosecond — Nanosecond (0-999999999)
 */
declare function utilsDatetimeToTime({ date: Date }): { hour: int, minute: int, second: int, nanosecond: int };

/**
 * Reads a date in another timezone. The instant stays the same, the wall clock changes
 * @param date — Input Date
 * @param timezone (optional) — IANA timezone name, for example Europe/Berlin or America/New_York
 * @param format (optional) — Format for the text output, for example %Y-%m-%d %H:%M
 * @returns dateOut — The same instant carrying the target offset
 * @returns formatted — Local wall clock time as text
 * @returns offsetSeconds — Offset from UTC at that instant, daylight saving included
 */
declare function utilsDatetimeToTimezone({ date: Date, timezone?: string, format?: string }): { dateOut: Date, formatted: string, offsetSeconds: int };

/**
 * Converts a date into an epoch timestamp
 * @param date — Input Date
 * @param unit (optional) — Unit of the produced timestamp
 * @returns timestamp — Epoch timestamp in the selected unit
 */
declare function utilsDatetimeToUnix({ date: Date, unit?: string }): int;


// === Utils/DateTime/Comparison ===

/**
 * True when the first date lies after the second
 * @param date — Date to test
 * @param other — Date to compare against
 * @returns result — True when the first date lies after the second
 */
declare function utilsDatetimeAfter({ date: Date, other: Date }): bool;

/**
 * True when the first date lies before the second
 * @param date — Date to test
 * @param other — Date to compare against
 * @returns result — True when the first date lies before the second
 */
declare function utilsDatetimeBefore({ date: Date, other: Date }): bool;

/**
 * True when a date falls inside a range
 * @param date — Date to test
 * @param start — Start of the range
 * @param end — End of the range
 * @param inclusive (optional) — Count the boundaries as inside the range
 * @returns result — True when the date lies in the range
 */
declare function utilsDatetimeBetween({ date: Date, start: Date, end: Date, inclusive?: bool }): bool;

/**
 * True when both dates fall into the same unit
 * @param date — Date to test
 * @param other — Date to compare against
 * @param unit (optional) — Granularity the comparison runs at
 * @returns result — True when both dates fall into the same unit
 */
declare function utilsDatetimeSame({ date: Date, other: Date, unit?: string }): bool;


// === Utils/Encoding ===

/**
 * Decodes a Base64 string back to a UTF-8 string
 * @param input — Base64 encoded string
 * @returns output — Decoded UTF-8 string
 */
declare function utilsEncodingBase64Decode({ input: string }): string;

/**
 * Decodes a Base64 string to raw bytes
 * @param input — Base64 encoded string
 * @returns output — Decoded raw bytes
 */
declare function utilsEncodingBase64DecodeBytes({ input: string }): bytes[];

/**
 * Encodes a string to Base64
 * @param input — String to encode
 * @returns output — Base64 encoded string
 */
declare function utilsEncodingBase64Encode({ input: string }): string;

/**
 * Encodes raw bytes to a Base64 string
 * @param input — Raw bytes to encode
 * @returns output — Base64 encoded string
 */
declare function utilsEncodingBase64EncodeBytes({ input: bytes[] }): string;

/**
 * Decodes a hexadecimal string back to a UTF-8 string
 * @param input — Hex-encoded string
 * @returns output — Decoded UTF-8 string
 */
declare function utilsEncodingHexDecode({ input: string }): string;

/**
 * Decodes a hexadecimal string to raw bytes
 * @param input — Hex-encoded string
 * @returns output — Decoded raw bytes
 */
declare function utilsEncodingHexDecodeBytes({ input: string }): bytes[];

/**
 * Encodes a string's bytes to a hexadecimal string
 * @param input — String to encode
 * @returns output — Hex-encoded string
 */
declare function utilsEncodingHexEncode({ input: string }): string;

/**
 * Encodes raw bytes to a hexadecimal string
 * @param input — Raw bytes to encode
 * @returns output — Hex-encoded string
 */
declare function utilsEncodingHexEncodeBytes({ input: bytes[] }): string;

/**
 * Decodes HTML entities back to their original characters
 * @param input — HTML-encoded string
 * @returns output — Decoded string
 */
declare function utilsEncodingHtmlDecode({ input: string }): string;

/**
 * Encodes special characters as HTML entities (&amp; &lt; &gt; &quot; &#39;)
 * @param input — String to encode
 * @returns output — HTML-encoded string
 */
declare function utilsEncodingHtmlEncode({ input: string }): string;

/**
 * Decodes a percent-encoded URL string back to plain text
 * @param input — URL-encoded string
 * @returns output — Decoded string
 */
declare function utilsEncodingUrlDecode({ input: string }): string;

/**
 * Percent-encodes a string for safe use in URLs (RFC 3986)
 * @param input — String to encode
 * @returns output — URL-encoded string
 */
declare function utilsEncodingUrlEncode({ input: string }): string;


// === Utils/Execution ===

/**
 * Returns the current app identifier.
 * @returns appId — Current app identifier
 */
declare function utilsExecutionGetAppId(): string;

/**
 * Returns where and how the current run is executing.
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
declare function utilsExecutionGetEnvironment(): { environment: string, executionMode: string, isDesktop: bool, isServer: bool, isMobile: bool, isBrowserSandbox: bool, isLocal: bool, isRemote: bool, runId: string, appId: string, userId: string, details: Struct };

/**
 * Returns the current execution mode.
 * @returns mode — The execution mode: sync, async, event, or scheduled
 */
declare function utilsExecutionGetMode(): string;

/**
 * Returns the current execution run identifier.
 * @returns runId — Current run identifier
 */
declare function utilsExecutionGetRunId(): string;

/**
 * Returns the current user identifier, when available.
 * @returns userId — Current user identifier, or empty when unavailable
 */
declare function utilsExecutionGetUserId(): string;

/**
 * Returns true when the current run is executing on a local/client runtime.
 * @returns isLocal — True for local, desktop, mobile, and browser sandbox execution
 */
declare function utilsExecutionIsLocalEnvironment(): bool;

/**
 * Returns true when the current run is executing on a mobile runtime.
 * @returns isMobile — True for mobile execution
 */
declare function utilsExecutionIsMobileEnvironment(): bool;

/**
 * Returns true when the current run is executing on the server.
 * @returns isServer — True for server-side execution
 */
declare function utilsExecutionIsServerEnvironment(): bool;


// === Utils/Faker/Address ===

/**
 * Generates a random city name for mocking data
 * @returns city — Generated city name
 * @impure has side effects / drives control flow
 */
declare function fakerCityName(): string;

/**
 * Generates a random country code (e.g., US, DE, FR) for mocking data
 * @returns code — Generated country code
 * @impure has side effects / drives control flow
 */
declare function fakerCountryCode(): string;

/**
 * Generates a random country name for mocking data
 * @returns country — Generated country name
 * @impure has side effects / drives control flow
 */
declare function fakerCountryName(): string;

/**
 * Generates a random latitude coordinate for mocking data
 * @returns latitude — Generated latitude
 * @impure has side effects / drives control flow
 */
declare function fakerLatitude(): float;

/**
 * Generates a random longitude coordinate for mocking data
 * @returns longitude — Generated longitude
 * @impure has side effects / drives control flow
 */
declare function fakerLongitude(): float;

/**
 * Generates a random postal/zip code for mocking data
 * @returns code — Generated postal code
 * @impure has side effects / drives control flow
 */
declare function fakerPostCode(): string;

/**
 * Generates a random state/province name for mocking data
 * @returns state — Generated state name
 * @impure has side effects / drives control flow
 */
declare function fakerStateName(): string;

/**
 * Generates a random full street address for mocking data
 * @returns address — Generated street address
 * @impure has side effects / drives control flow
 */
declare function fakerStreetAddress(): string;

/**
 * Generates a random street name for mocking data
 * @returns street — Generated street name
 * @impure has side effects / drives control flow
 */
declare function fakerStreetName(): string;


// === Utils/Faker/Company ===

/**
 * Generates a random business buzzword for mocking data
 * @returns buzzword — Generated buzzword
 * @impure has side effects / drives control flow
 */
declare function fakerBuzzword(): string;

/**
 * Generates a random business catch phrase for mocking data
 * @returns phrase — Generated catch phrase
 * @impure has side effects / drives control flow
 */
declare function fakerCatchPhrase(): string;

/**
 * Generates a random company name for mocking data
 * @returns company — Generated company name
 * @impure has side effects / drives control flow
 */
declare function fakerCompanyName(): string;

/**
 * Generates a random industry name for mocking data
 * @returns industry — Generated industry name
 * @impure has side effects / drives control flow
 */
declare function fakerIndustry(): string;

/**
 * Generates a random profession/job title for mocking data
 * @returns profession — Generated profession
 * @impure has side effects / drives control flow
 */
declare function fakerProfession(): string;


// === Utils/Faker/Internet ===

/**
 * Generates a random domain suffix (com, org, net, etc.)
 * @returns suffix — Generated domain suffix
 * @impure has side effects / drives control flow
 */
declare function fakerDomainSuffix(): string;

/**
 * Generates a random email address for mocking data
 * @returns email — Generated email address
 * @impure has side effects / drives control flow
 */
declare function fakerEmail(): string;

/**
 * Generates a random IPv4 address for mocking data
 * @returns ip — Generated IPv4 address
 * @impure has side effects / drives control flow
 */
declare function fakerIpv4(): string;

/**
 * Generates a random IPv6 address for mocking data
 * @returns ip — Generated IPv6 address
 * @impure has side effects / drives control flow
 */
declare function fakerIpv6(): string;

/**
 * Generates a random password for mocking data
 * @param minLength (optional) — Minimum password length
 * @param maxLength (optional) — Maximum password length
 * @returns password — Generated password
 * @impure has side effects / drives control flow
 */
declare function fakerPassword({ minLength?: int, maxLength?: int }): string;

/**
 * Generates a random user agent string for mocking data
 * @returns userAgent — Generated user agent
 * @impure has side effects / drives control flow
 */
declare function fakerUserAgent(): string;

/**
 * Generates a random username for mocking data
 * @returns username — Generated username
 * @impure has side effects / drives control flow
 */
declare function fakerUsername(): string;


// === Utils/Faker/Lorem ===

/**
 * Generates a random lorem ipsum paragraph for mocking data
 * @param minSentences (optional) — Minimum sentences in paragraph
 * @param maxSentences (optional) — Maximum sentences in paragraph
 * @returns paragraph — Generated paragraph
 * @impure has side effects / drives control flow
 */
declare function fakerParagraph({ minSentences?: int, maxSentences?: int }): string;

/**
 * Generates random lorem ipsum paragraphs for mocking data
 * @param minCount (optional) — Minimum number of paragraphs
 * @param maxCount (optional) — Maximum number of paragraphs
 * @returns paragraphs — Generated paragraphs as array
 * @impure has side effects / drives control flow
 */
declare function fakerParagraphs({ minCount?: int, maxCount?: int }): any;

/**
 * Generates a random lorem ipsum sentence for mocking data
 * @param minWords (optional) — Minimum words in sentence
 * @param maxWords (optional) — Maximum words in sentence
 * @returns sentence — Generated sentence
 * @impure has side effects / drives control flow
 */
declare function fakerSentence({ minWords?: int, maxWords?: int }): string;

/**
 * Generates random lorem ipsum sentences for mocking data
 * @param minCount (optional) — Minimum number of sentences
 * @param maxCount (optional) — Maximum number of sentences
 * @returns sentences — Generated sentences as array
 * @impure has side effects / drives control flow
 */
declare function fakerSentences({ minCount?: int, maxCount?: int }): any;

/**
 * Generates a random lorem ipsum word for mocking data
 * @returns word — Generated word
 * @impure has side effects / drives control flow
 */
declare function fakerWord(): string;

/**
 * Generates random lorem ipsum words for mocking data
 * @param minCount (optional) — Minimum number of words
 * @param maxCount (optional) — Maximum number of words
 * @returns words — Generated words as array
 * @impure has side effects / drives control flow
 */
declare function fakerWords({ minCount?: int, maxCount?: int }): any;


// === Utils/Faker/Name ===

/**
 * Generates a random first name for mocking data
 * @returns name — Generated first name
 * @impure has side effects / drives control flow
 */
declare function fakerFirstName(): string;

/**
 * Generates a random full name for mocking data
 * @returns name — Generated full name
 * @impure has side effects / drives control flow
 */
declare function fakerFullName(): string;

/**
 * Generates a random last name for mocking data
 * @returns name — Generated last name
 * @impure has side effects / drives control flow
 */
declare function fakerLastName(): string;

/**
 * Generates a random name title (Mr., Mrs., Dr., etc.)
 * @returns title — Generated title
 * @impure has side effects / drives control flow
 */
declare function fakerTitle(): string;


// === Utils/Faker/Number ===

/**
 * Generates a random boolean for mocking data
 * @param probability (optional) — Probability of true (0.0 to 1.0)
 * @returns value — Generated boolean
 * @impure has side effects / drives control flow
 */
declare function fakerBoolean({ probability?: float }): bool;

/**
 * Generates a random digit (0-9) for mocking data
 * @returns digit — Generated digit
 * @impure has side effects / drives control flow
 */
declare function fakerDigit(): int;

/**
 * Generates a random float in a specified range for mocking data
 * @param min (optional) — Minimum value (inclusive)
 * @param max (optional) — Maximum value (exclusive)
 * @returns number — Generated float
 * @impure has side effects / drives control flow
 */
declare function fakerFloat({ min?: float, max?: float }): float;

/**
 * Generates a random integer in a specified range for mocking data
 * @param min (optional) — Minimum value (inclusive)
 * @param max (optional) — Maximum value (exclusive)
 * @returns number — Generated integer
 * @impure has side effects / drives control flow
 */
declare function fakerInteger({ min?: int, max?: int }): int;


// === Utils/Faker/Phone ===

/**
 * Generates a random cell/mobile phone number for mocking data
 * @returns phone — Generated cell number
 * @impure has side effects / drives control flow
 */
declare function fakerCellNumber(): string;

/**
 * Generates a random phone number for mocking data
 * @returns phone — Generated phone number
 * @impure has side effects / drives control flow
 */
declare function fakerPhoneNumber(): string;


// === Utils/Format ===

/**
 * Turns a byte count into a readable size such as 1.4 MB
 * @param bytes (optional) — Number of bytes
 * @param standard (optional) — Decimal counts in 1000s (MB), Binary in 1024s (MiB)
 * @param decimals (optional) — Decimal places to keep
 * @returns text — The readable size
 * @returns unit — The unit that was chosen
 */
declare function formatBytes({ bytes?: int, standard?: string, decimals?: int }): { text: string, unit: string };

/**
 * Writes a number of seconds as a readable duration such as 2h 15m
 * @param seconds (optional) — Length of the duration in seconds
 * @param style (optional) — Short writes 2h 15m, Long writes 2 hours 15 minutes, Clock writes 02:15:00
 * @param maxParts (optional) — How many units to show before stopping, for example 2 gives 2h 15m instead of 2h 15m 3s
 * @returns text — The readable duration
 */
declare function formatDuration({ seconds?: float, style?: string, maxParts?: int }): string;

/**
 * Renders a number for display with fixed decimals and separators
 * @param value — Number to format
 * @param decimals (optional) — Decimal places to keep
 * @param thousands (optional) — Inserted every three digits, empty for none
 * @param decimalPoint (optional) — Character between the whole and fractional part
 * @param prefix (optional) — Put in front, for example a currency symbol
 * @param suffix (optional) — Appended, for example a unit
 * @param asPercent (optional) — Multiply by 100 and append a percent sign
 * @returns text — The formatted number
 */
declare function formatNumber({ value: float, decimals?: int, thousands?: string, decimalPoint?: string, prefix?: string, suffix?: string, asPercent?: bool }): string;

/**
 * Writes a number as 1st, 2nd, 3rd and so on
 * @param value (optional) — Number to write
 * @returns text — The ordinal
 * @returns suffix — Just the two letter suffix
 */
declare function formatOrdinal({ value?: int }): { text: string, suffix: string };


// === Utils/Hash ===

/**
 * Computes the AHash of the input
 * @param input — Input data to hash
 * @param consistent (optional) — Use consistent hashing
 * @param seed (optional) — Seed value for consistent hashing
 * @returns hash — AHash of the input
 * @impure has side effects / drives control flow
 */
declare function utilsHashAhash({ input: any, consistent?: bool, seed?: int }): int;

/**
 * Computes the Blake3 hash of the input
 * @param input — Input data to hash
 * @returns hash — Blake3 hash of the input
 * @impure has side effects / drives control flow
 */
declare function utilsHashBlake3({ input: any }): string;

/**
 * Computes the MD5 hash of the input string. Note: MD5 is not collision-resistant — use SHA-256 or Blake3 for security-sensitive hashing.
 * @param input — String to hash
 * @returns hash — MD5 hash as hex string
 * @impure has side effects / drives control flow
 */
declare function utilsHashMd5({ input: string }): string;

/**
 * Computes the SHA-256 hash of the input string
 * @param input — String to hash
 * @returns hash — SHA-256 hash as hex string
 * @impure has side effects / drives control flow
 */
declare function utilsHashSha256({ input: string }): string;

/**
 * Computes the SHA-512 hash of the input string
 * @param input — String to hash
 * @returns hash — SHA-512 hash as hex string
 * @impure has side effects / drives control flow
 */
declare function utilsHashSha512({ input: string }): string;


// === Utils/JSON ===

/**
 * Parse JSON input Data With JSON/OpenAI Schema and Return Value
 * @param schema — JSON Schema or OpenAI Function Definition
 * @param data — JSON Input Data to be parsed
 * @returns parsed — Parsed and Validated JSON
 * @impure has side effects / drives control flow
 */
declare function parseWithSchema({ schema: string, data: string }): Struct;

/**
 * Attempts to repair and parse potentially malformed JSON
 * @param jsonString — String containing potentially malformed JSON
 * @returns result — The parsed JSON structure
 * @impure has side effects / drives control flow
 */
declare function repairParse({ jsonString: string }): Struct;

/**
 * Generate Tool Definitions for Tool Calls
 * @param exampleJson — Example JSON to infer schema from
 * @returns schema — Generated JSON Schema / Tool Definition
 * @impure has side effects / drives control flow
 */
declare function utilsJsonMakeSchema({ exampleJson: string }): Struct;


// === Utils/Map ===

/**
 * Creates an empty map (string keys)
 * @returns mapOut — The created map
 */
declare function makeMap(): Map<string, any>;

/**
 * Removes all entries from a map
 * @param mapIn — Your Map
 * @returns mapOut — Empty Map
 * @impure has side effects / drives control flow
 */
declare function mapClear({ mapIn: Map<string, any> }): Map<string, any>;

/**
 * Gets a value from a map by key
 * @param mapIn — Your Map
 * @param key — Key to get
 * @returns value — Value at the specified key
 * @returns found — Was the key found in the map?
 */
declare function mapGet({ mapIn: Map<string, any>, key: string }): { value: any, found: bool };

/**
 * Checks if a key exists in the map
 * @param mapIn — Your Map
 * @param key — Key to check
 * @returns hasKey — Does the map contain the key?
 */
declare function mapHasKey({ mapIn: Map<string, any>, key: string }): bool;

/**
 * Gets all keys from the map as an array
 * @param mapIn — Your Map
 * @returns keys — Array of all keys
 */
declare function mapKeys({ mapIn: Map<string, any> }): any[];

/**
 * Removes a key from the map
 * @param mapIn — Your Map
 * @param key — Key to remove
 * @returns mapOut — Adjusted Map
 * @returns value — The removed value (null if key not found)
 * @returns wasPresent — Was the key in the map?
 * @impure has side effects / drives control flow
 */
declare function mapRemove({ mapIn: Map<string, any>, key: string }): { mapOut: Map<string, any>, value: any, wasPresent: bool };

/**
 * Sets a value in a map at the given key
 * @param mapIn — Your Map
 * @param key — Key to set
 * @param value — Value to set
 * @returns mapOut — Adjusted Map
 * @returns replaced — Was an existing value replaced?
 * @impure has side effects / drives control flow
 */
declare function mapSet({ mapIn: Map<string, any>, key: string, value: any }): { mapOut: Map<string, any>, replaced: bool };

/**
 * Gets the number of entries in the map
 * @param mapIn — Your Map
 * @returns size — Number of entries in the map
 */
declare function mapSize({ mapIn: Map<string, any> }): int;

/**
 * Gets all values from the map as an array
 * @param mapIn — Your Map
 * @returns values — Array of all values
 */
declare function mapValues({ mapIn: Map<string, any> }): any[];


// === Utils/Map/By Reference ===

/**
 * Clear all entries directly from a variable map without copying.
 * @param varRef — Reference to the map variable to clear
 * @impure has side effects / drives control flow
 */
declare function mapClearRef({ varRef: string }): void;

/**
 * Remove a key directly from a variable map without copying. Much faster for large maps.
 * @param varRef — Reference to the map variable to modify
 * @param key — Key to remove
 * @returns value — The removed value (null if key not found)
 * @returns wasPresent — Was the key in the map?
 * @impure has side effects / drives control flow
 */
declare function mapRemoveRef({ varRef: string, key: string }): { value: any, wasPresent: bool };

/**
 * Set a value directly in a variable map without copying. Much faster for large maps.
 * @param varRef — Reference to the map variable to modify
 * @param key — Key to set
 * @param value — Value to set at the key
 * @impure has side effects / drives control flow
 */
declare function mapSetRef({ varRef: string, key: string, value: any }): void;


// === Utils/Markdown ===

/**
 * Attempts to convert HTML to Markdown, removing unwanted tags
 * @param html — Html to Parse
 * @param skippedTags (optional) — Tags to skip
 * @returns markdown — The parsed Markdown
 * @impure has side effects / drives control flow
 */
declare function utilsMdHtmlToMd({ html: string, skippedTags?: string[] }): string;

/**
 * Renders GitHub-flavoured Markdown as HTML
 * @param markdown — Markdown source to render
 * @param allowHtml (optional) — Pass raw HTML in the source through to the output. Leave off for untrusted input.
 * @param smartPunctuation (optional) — Convert quotes, dashes and ellipses to typographic equivalents
 * @returns html — The rendered HTML
 * @impure has side effects / drives control flow
 */
declare function utilsMdMdToHtml({ markdown: string, allowHtml?: bool, smartPunctuation?: bool }): string;

/**
 * Converts a rich text document (plate_json) into HTML, keeping alignment, colours, columns and table spans that Markdown cannot express
 * @param document — Rich text document, with or without the plate_json:: prefix
 * @param images (optional) — How to render image nodes
 * @param fullDocument (optional) — Wrap the output in a complete HTML document with default styling
 * @param title (optional) — Document title, used only when Full Document is enabled
 * @returns html — The converted HTML
 * @returns media — Every image, video, audio and file reference found in the document
 * @impure has side effects / drives control flow
 */
declare function utilsMdPlateToHtml({ document: string, images?: string, fullDocument?: bool, title?: string }): { html: string, media: string[] };

/**
 * Converts a rich text document (plate_json) into GitHub-flavoured Markdown
 * @param document — Rich text document, with or without the plate_json:: prefix
 * @param images (optional) — How to render image nodes
 * @returns markdown — The converted Markdown
 * @returns media — Every image, video, audio and file reference found in the document
 * @impure has side effects / drives control flow
 */
declare function utilsMdPlateToMd({ document: string, images?: string }): { markdown: string, media: string[] };


// === Utils/Math/Vector ===

/**
 * Adds two float vectors together element-wise
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns resultVector — Sum of the two vectors
 */
declare function floatVectorAddition({ vector1: float[], vector2: float[] }): float[];

/**
 * Calculates the cosine similarity of two float vectors
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns similarity — Cosine similarity of the two vectors
 */
declare function floatVectorCosineSimilarity({ vector1: float[], vector2: float[] }): float;

/**
 * Calculates the cross product of two float vectors
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns resultVector — Cross product of the two vectors
 */
declare function floatVectorCrossProduct({ vector1: float[], vector2: float[] }): float[];

/**
 * Calculates the dot product of two float vectors
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns result — Dot product of the two vectors
 */
declare function floatVectorDotProduct({ vector1: float[], vector2: float[] }): float;

/**
 * Multiplies two float vectors element-wise
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns resultVector — Element-wise product of the two vectors
 */
declare function floatVectorMultiplication({ vector1: float[], vector2: float[] }): float[];

/**
 * Normalizes a float vector
 * @param vector — Float vector to normalize
 * @returns normalizedVector — Normalized float vector
 */
declare function floatVectorNormalize({ vector: float[] }): float[];

/**
 * Subtracts one float vector from another element-wise
 * @param vector1 — First float vector
 * @param vector2 — Second float vector
 * @returns resultVector — Element-wise difference of the two vectors
 */
declare function floatVectorSubtraction({ vector1: float[], vector2: float[] }): float[];


// === Utils/Random ===

/**
 * Picks elements out of an array at random
 * @param arrayIn — Your Array
 * @param count (optional) — How many elements to draw
 * @param allowRepeats (optional) — Draw with replacement, so the same element can come up twice
 * @returns element — The first drawn element
 * @returns elements — Every drawn element
 * @impure has side effects / drives control flow
 */
declare function randomChoice({ arrayIn: any[], count?: int, allowRepeats?: bool }): { element: any, elements: any[] };

/**
 * Generates a random string, for example a token or a short code
 * @param length (optional) — How many characters to generate
 * @param alphabet (optional) — Characters to draw from. Unambiguous leaves out l, I, 1, O and 0
 * @param customAlphabet (optional) — Use exactly these characters instead, when set
 * @returns result — The generated string
 * @impure has side effects / drives control flow
 */
declare function randomString({ length?: int, alphabet?: string, customAlphabet?: string }): string;


// === Utils/Set ===

/**
 * Converts an array to a set
 * @param arrayIn
 * @returns setOut
 */
declare function arrayToSet({ arrayIn: any[] }): Set<any>;

/**
 * Creates a set from the difference of 2 sets
 * @param setIn1 — Your First Set
 * @param setIn2 — Your Second Set
 * @returns setOut — The difference set
 * @impure has side effects / drives control flow
 */
declare function difference({ setIn1: Set<any>, setIn2: Set<any> }): Set<any>;

/**
 * Inserts an element to the set
 * @param setIn — Your Set
 * @param value — Value to push
 * @returns setOut — Adjusted Set
 * @returns existedBefore — Was the element there before?
 * @impure has side effects / drives control flow
 */
declare function insert({ setIn: Set<any>, value: any }): { setOut: Set<any>, existedBefore: bool };

/**
 * Checks if one of the hash sets has at least one mutual element
 * @param setIn1
 * @param setIn2
 * @returns isMutual — Does it include a mutual element that both sets share or not?
 * @impure has side effects / drives control flow
 */
declare function isMutual({ setIn1: Set<any>, setIn2: Set<any> }): bool;

/**
 * Creates an empty set
 * @returns setOut — The created set
 */
declare function makeSet(): Set<any>;

/**
 * Removes / Clears all elements from a set
 * @param setIn — Your Set
 * @returns setOut — Empty Set
 * @impure has side effects / drives control flow
 */
declare function setClear({ setIn: Set<any> }): Set<any>;

/**
 * Discards an element of a set
 * @param setIn — Your Set
 * @param value — Value to remove
 * @returns setOut — Adjusted Set
 * @returns hasRemoved — If the element was removed
 * @impure has side effects / drives control flow
 */
declare function setDiscard({ setIn: Set<any>, value: any }): { setOut: Set<any>, hasRemoved: bool };

/**
 * Gets the size of the hash set (how many elements)
 * @param setIn — Your Set
 * @returns size — How many elements does it have
 */
declare function setGetSize({ setIn: Set<any> }): int;

/**
 * Checks if an element is present in the set
 * @param setIn — Your Set
 * @param value — Value to search for
 * @returns contains — Does the set include the value?
 */
declare function setHas({ setIn: Set<any>, value: any }): bool;

/**
 * Checks if a hash set is empty or not
 * @param setIn — Your Set
 * @returns isEmpty — Does it have any values or not?
 */
declare function setIsEmpty({ setIn: Set<any> }): bool;

/**
 * Checks if a hash set is a subset from a supposed bigger one
 * @param setIn1 — Your Smaller Set
 * @param setIn2 — Your Bigger Set
 * @returns isSubset — Is the first set a subset of the second?
 */
declare function setIsSubset({ setIn1: Set<any>, setIn2: Set<any> }): bool;

/**
 * Checks if a hash set is a superset from a supposed smaller one
 * @param setIn1 — Your Bigger Set
 * @param setIn2 — Your Smaller Set
 * @returns isSuperset — Is the first set a superset of the second?
 */
declare function setIsSuperset({ setIn1: Set<any>, setIn2: Set<any> }): bool;

/**
 * Pops a random element of a set
 * @param setIn — Your Set
 * @returns setOut — Adjusted Set
 * @impure has side effects / drives control flow
 */
declare function setPop({ setIn: Set<any> }): Set<any>;

/**
 * Converts a set to an array
 * @param setIn
 * @returns arrayOut
 */
declare function setToArray({ setIn: Set<any> }): any[];

/**
 * Combines 2 sets into one unified hash set
 * @param setIn1 — Your First Set
 * @param setIn2 — Your Second Set
 * @returns setOut — Combined Set
 * @impure has side effects / drives control flow
 */
declare function union({ setIn1: Set<any>, setIn2: Set<any> }): Set<any>;


// === Utils/Set/By Reference ===

/**
 * Clear all elements directly from a variable set without copying.
 * @param varRef — Reference to the set variable to clear
 * @impure has side effects / drives control flow
 */
declare function setClearRef({ varRef: string }): void;

/**
 * Remove an element directly from a variable set without copying. Much faster for large sets.
 * @param varRef — Reference to the set variable to modify
 * @param value — Value to remove from the set
 * @returns wasPresent — True if the element was in the set and removed
 * @impure has side effects / drives control flow
 */
declare function setDiscardRef({ varRef: string, value: any }): bool;

/**
 * Insert an element directly into a variable set without copying. Much faster for large sets.
 * @param varRef — Reference to the set variable to modify
 * @param value — Value to insert into the set
 * @returns wasNew — True if the element was not already in the set
 * @impure has side effects / drives control flow
 */
declare function setInsertRef({ varRef: string, value: any }): bool;


// === Utils/String ===

/**
 * Compares two Strings
 * @param string — Input
 * @param string — Input
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns equal — Are the strings equal?
 */
declare function equalString({ string: string, string: string, ignoreCase?: bool }): bool;

/**
 * Compares two Strings
 * @param string — Input
 * @param string — Input
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns unequal — Are the strings equal?
 */
declare function notEqualString({ string: string, string: string, ignoreCase?: bool }): bool;

/**
 * Returns the character at a index. Negative indices count from the end
 * @param string — Input String
 * @param index (optional) — Character index, negative counts from the end
 * @returns character — The character at the index, empty when out of range
 * @returns found — True when the index was in range
 */
declare function stringCharAt({ string: string, index?: int }): { character: string, found: bool };

/**
 * Appends strings to each other without a separator
 * @param string (optional) — Part to append
 * @param string (optional) — Part to append
 * @returns concatenated — All parts appended in order
 */
declare function stringConcat({ string?: string, string?: string }): string;

/**
 * Checks if a string contains a substring
 * @param string — Input String
 * @param substring — Substring to search for
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns contains — Does the string contain the substring?
 */
declare function stringContains({ string: string, substring: string, ignoreCase?: bool }): bool;

/**
 * Checks whether a string contains any of the given substrings
 * @param string — Input String
 * @param substrings — Substrings to search for
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns contains — True when at least one substring occurs
 * @returns matched — The first substring that occurred
 */
declare function stringContainsAny({ string: string, substrings: string[], ignoreCase?: bool }): { contains: bool, matched: string };

/**
 * Counts non-overlapping occurrences of a substring
 * @param string — Input String
 * @param substring — Substring to count
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns count — Number of non-overlapping occurrences
 */
declare function stringCountMatches({ string: string, substring: string, ignoreCase?: bool }): int;

/**
 * Shortens a string that is longer than the given number of characters and marks the cut with an ellipsis. A string that already fits is returned unchanged
 * @param string — Input String
 * @param maxLength (optional) — Longest the result may be, counted in characters and including the ellipsis itself
 * @param ellipsis (optional) — Appended in place of what was cut
 * @returns result — The shortened string, or the input unchanged when it already fits
 */
declare function stringEllipsis({ string: string, maxLength?: int, ellipsis?: string }): string;

/**
 * Checks if a string ends with a specific string
 * @param string — Input String
 * @param suffix — String to check against
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns endsWith — Does the string end with the suffix?
 */
declare function stringEndsWith({ string: string, suffix: string, ignoreCase?: bool }): bool;

/**
 * Escapes special characters in a string (newlines, tabs, carriage returns, backslashes, quotes).
 * @param string — Input String
 * @returns escaped — String with special characters escaped
 */
declare function stringEscape({ string: string }): string;

/**
 * Pulls every email, link, number or handle out of a text
 * @param string — Input String
 * @param pattern (optional) — What to look for
 * @param unique (optional) — Drop repeated matches
 * @returns matches — Everything that matched, in order
 * @returns count — How many matches were found
 */
declare function stringExtract({ string: string, pattern?: string, unique?: bool }): { matches: string[], count: int };

/**
 * Formats a string with placeholders
 * @param formatString — String with placeholders
 * @returns formattedString — Formatted string
 */
declare function stringFormat({ formatString: string }): string;

/**
 * Finds the character index of the first occurrence of a substring
 * @param string — Input String
 * @param substring — Substring to search for
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns index — Character index of the match, -1 when not found
 * @returns found — True when the substring occurs in the string
 */
declare function stringIndexOf({ string: string, substring: string, ignoreCase?: bool }): { index: int, found: bool };

/**
 * Checks whether every character is a letter or a digit
 * @param string — Input String
 * @returns result — True when all characters are alphanumeric
 */
declare function stringIsAlphanumeric({ string: string }): bool;

/**
 * Checks whether a string only contains ASCII characters
 * @param string — Input String
 * @returns result — True when the string is pure ASCII
 */
declare function stringIsAscii({ string: string }): bool;

/**
 * Checks whether a string looks like an email address
 * @param string — Input String
 * @returns result — True when the string is a plausible email address
 */
declare function stringIsEmail({ string: string }): bool;

/**
 * Checks whether a string contains no characters
 * @param string — Input String
 * @param ignoreWhitespace (optional) — Treat whitespace-only strings as empty
 * @returns isEmpty — True when the string is empty
 */
declare function stringIsEmpty({ string: string, ignoreWhitespace?: bool }): bool;

/**
 * Checks whether a string is an IPv4 or IPv6 address
 * @param string — Input String
 * @returns result — True when the string is an IP address
 */
declare function stringIsIp({ string: string }): bool;

/**
 * Checks whether a string parses as JSON
 * @param string — Input String
 * @returns result — True when the string is valid JSON
 */
declare function stringIsJson({ string: string }): bool;

/**
 * Checks whether a string can be read as a number
 * @param string — Input String
 * @returns result — True when the string parses as a number
 */
declare function stringIsNumeric({ string: string }): bool;

/**
 * Checks whether a string is a URL with a scheme and a host
 * @param string — Input String
 * @returns result — True when the string is a plausible URL
 */
declare function stringIsUrl({ string: string }): bool;

/**
 * Checks whether a string is a UUID
 * @param string — Input String
 * @returns result — True when the string is a UUID
 */
declare function stringIsUuid({ string: string }): bool;

/**
 * Joins multiple strings together
 * @param strings — Strings to join
 * @param separator — String to separate by
 * @returns joinedString — Concatenated string
 */
declare function stringJoin({ strings: string[], separator: string }): string;

/**
 * Finds the character index of the last occurrence of a substring
 * @param string — Input String
 * @param substring — Substring to search for
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns index — Character index of the last match, -1 when not found
 * @returns found — True when the substring occurs in the string
 */
declare function stringLastIndexOf({ string: string, substring: string, ignoreCase?: bool }): { index: int, found: bool };

/**
 * Calculates the length of a string
 * @param string — Input String
 * @param mode (optional) — Characters counts code points, Graphemes counts what a reader sees, Bytes counts UTF-8 bytes
 * @returns length — Length of the string
 */
declare function stringLength({ string: string, mode?: string }): int;

/**
 * Splits a string into its lines
 * @param string — Input String
 * @param skipEmpty (optional) — Drop lines that are empty or whitespace only
 * @returns lines — One entry per line
 */
declare function stringLines({ string: string, skipEmpty?: bool }): string[];

/**
 * Hides the middle of a value, keeping a few characters visible
 * @param string — Input String
 * @param keepStart (optional) — Characters left visible at the start
 * @param keepEnd (optional) — Characters left visible at the end
 * @param maskCharacter (optional) — Character used for the hidden part
 * @param fixedWidth (optional) — Always use this many mask characters so the length is not leaked, 0 keeps the real length
 * @returns masked — The masked value
 */
declare function stringMask({ string: string, keepStart?: int, keepEnd?: int, maskCharacter?: string, fixedWidth?: int }): string;

/**
 * Collapses runs of whitespace into single spaces and trims the result
 * @param string — Input String
 * @returns normalized — The normalized string
 */
declare function stringNormalizeWhitespace({ string: string }): string;

/**
 * Fills up a string at the end until it reaches the target length
 * @param string — Input String
 * @param length (optional) — Target length in characters
 * @param padding (optional) — Characters used to fill up the string
 * @returns padded — The padded string, unchanged when it is already long enough
 */
declare function stringPadEnd({ string: string, length?: int, padding?: string }): string;

/**
 * Fills up a string at the start until it reaches the target length
 * @param string — Input String
 * @param length (optional) — Target length in characters
 * @param padding (optional) — Characters used to fill up the string
 * @returns padded — The padded string, unchanged when it is already long enough
 */
declare function stringPadStart({ string: string, length?: int, padding?: string }): string;

/**
 * Template Engine based on Jinja Templates
 * @param template — Jinja Template String
 * @returns rendered — Rendered String
 */
declare function stringRenderTemplate({ template: string }): string;

/**
 * Repeats a string a number of times
 * @param string — Input String
 * @param count (optional) — How often the string is repeated
 * @returns repeated — The repeated string
 */
declare function stringRepeat({ string: string, count?: int }): string;

/**
 * Replaces occurrences of a substring or regex pattern within a string.
 * @param string — Input String
 * @param pattern — Substring or regex pattern to replace
 * @param replacement — Replacement string (supports $1, $2, ... for regex capture groups)
 * @param isRegex (optional) — Treat the pattern as a regular expression
 * @returns newString — String with replacements
 */
declare function stringReplace({ string: string, pattern: string, replacement: string, isRegex?: bool }): string;

/**
 * Reverses the characters of a string
 * @param string — Input String
 * @returns reversed — The reversed string
 */
declare function stringReverse({ string: string }): string;

/**
 * Turns text into a URL safe slug
 * @param string — Input String
 * @param separator (optional) — Placed between words
 * @param maxLength (optional) — Cut the slug at a word boundary, 0 for no limit
 * @returns slug — The slug
 */
declare function stringSlugify({ string: string, separator?: string, maxLength?: int }): string;

/**
 * Splits a string into substrings
 * @param string — Input String
 * @param separator — String to split by, an empty separator splits into single characters
 * @param isRegex (optional) — Treat the separator as a regular expression
 * @param limit (optional) — Maximum number of parts, 0 for no limit. The last part keeps the rest
 * @param skipEmpty (optional) — Drop parts that are empty
 * @returns substrings — Array of substrings
 */
declare function stringSplit({ string: string, separator: string, isRegex?: bool, limit?: int, skipEmpty?: bool }): string[];

/**
 * Splits a string into two halves at a character index
 * @param string — Input String
 * @param index (optional) — Character index to split at, negative counts from the end
 * @returns before — Characters before the index
 * @returns after — Characters from the index onwards
 */
declare function stringSplitAt({ string: string, index?: int }): { before: string, after: string };

/**
 * Splits a string at the first (or last) occurrence of a separator
 * @param string — Input String
 * @param separator — String to split at
 * @param fromEnd (optional) — Split at the last occurrence instead of the first
 * @returns before — Text before the separator, the whole string when it was not found
 * @returns after — Text after the separator
 * @returns found — True when the separator was found
 */
declare function stringSplitOnce({ string: string, separator: string, fromEnd?: bool }): { before: string, after: string, found: bool };

/**
 * Splits a string into words, collapsing runs of whitespace
 * @param string — Input String
 * @returns words — The separated words
 */
declare function stringSplitWhitespace({ string: string }): string[];

/**
 * Checks if a string starts with a specific string
 * @param string — Input String
 * @param prefix — String to check against
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns startsWith — Does the string start with the prefix?
 */
declare function stringStartsWith({ string: string, prefix: string, ignoreCase?: bool }): bool;

/**
 * Checks whether a string starts with any of the given prefixes
 * @param string — Input String
 * @param prefixes — Prefixes to test
 * @param ignoreCase (optional) — Compare without regard to upper/lower case
 * @returns startsWith — True when the string starts with one of the prefixes
 * @returns matched — The first prefix that matched
 */
declare function stringStartsWithAny({ string: string, prefixes: string[], ignoreCase?: bool }): { startsWith: bool, matched: string };

/**
 * Removes a prefix from a string if it is present
 * @param string — Input String
 * @param prefix — Prefix to remove
 * @returns result — String without the prefix
 * @returns stripped — True when the prefix was present
 */
declare function stringStripPrefix({ string: string, prefix: string }): { result: string, stripped: bool };

/**
 * Removes a suffix from a string if it is present
 * @param string — Input String
 * @param suffix — Suffix to remove
 * @returns result — String without the suffix
 * @returns stripped — True when the suffix was present
 */
declare function stringStripSuffix({ string: string, suffix: string }): { result: string, stripped: bool };

/**
 * Extracts a range of characters from a string. Negative start counts from the end, length -1 runs to the end.
 * @param string — Input String
 * @param start (optional) — First character index, negative counts from the end
 * @param length (optional) — Number of characters to take, -1 for the rest of the string
 * @returns substring — The extracted characters
 */
declare function stringSubstring({ string: string, start?: int, length?: int }): string;

/**
 * Parses a string into a boolean. Accepts true/false, 1/0, yes/no and on/off
 * @param string — String to parse
 * @param fallback (optional) — Value used when parsing fails
 * @returns boolean — The parsed boolean
 * @returns success — True when the string was a recognized boolean
 */
declare function stringToBool({ string: string, fallback?: bool }): { boolean: bool, success: bool };

/**
 * Splits a string into an array of single characters
 * @param string — Input String
 * @returns characters — One entry per character
 */
declare function stringToChars({ string: string }): string[];

/**
 * Parses a string into a float
 * @param string — String to parse
 * @param fallback (optional) — Value used when parsing fails
 * @returns float — The parsed float
 * @returns success — True when the string was a valid float
 */
declare function stringToFloat({ string: string, fallback?: float }): { float: float, success: bool };

/**
 * Parses a string into an integer
 * @param string — String to parse
 * @param fallback (optional) — Value used when parsing fails
 * @returns integer — The parsed integer
 * @returns success — True when the string was a valid integer
 */
declare function stringToInt({ string: string, fallback?: int }): { integer: int, success: bool };

/**
 * Converts a string to lowercase
 * @param string — Input String
 * @returns lowercaseString — String in lowercase
 */
declare function stringToLower({ string: string }): string;

/**
 * Converts a string to uppercase
 * @param string — Input String
 * @returns uppercaseString — String in uppercase
 */
declare function stringToUpper({ string: string }): string;

/**
 * Removes leading and trailing whitespace from a string
 * @param string — Input String
 * @returns trimmedString — String without leading/trailing whitespace
 */
declare function stringTrim({ string: string }): string;

/**
 * Removes trailing whitespace from a string
 * @param string — Input String
 * @returns trimmedString — String without trailing whitespace
 */
declare function stringTrimEnd({ string: string }): string;

/**
 * Removes the given characters from the start and/or end of a string
 * @param string — Input String
 * @param characters (optional) — Set of characters to strip
 * @param side (optional) — Where to strip
 * @returns trimmedString — String without the stripped characters
 */
declare function stringTrimMatches({ string: string, characters?: string, side?: string }): string;

/**
 * Removes leading whitespace from a string
 * @param string — Input String
 * @returns trimmedString — String without leading whitespace
 */
declare function stringTrimStart({ string: string }): string;

/**
 * Shortens a string to a maximum number of characters, appending an ellipsis when it was cut
 * @param string — Input String
 * @param maxLength (optional) — Maximum number of characters including the ellipsis
 * @param ellipsis (optional) — Appended when the string was cut
 * @returns truncated — The shortened string
 * @returns wasTruncated — True when characters were removed
 */
declare function stringTruncate({ string: string, maxLength?: int, ellipsis?: string }): { truncated: string, wasTruncated: bool };

/**
 * Unescapes special character sequences in a string (\n, \t, \r, \\, \").
 * @param string — Input String
 * @returns unescaped — String with escape sequences resolved to actual characters
 */
declare function stringUnescape({ string: string }): string;

/**
 * Counts words, sentences and reading time
 * @param string — Input String
 * @param wordsPerMinute (optional) — Reading speed used for the estimate
 * @returns words — Number of words
 * @returns characters — Number of characters
 * @returns sentences — Number of sentences
 * @returns readingSeconds — Estimated reading time in seconds
 */
declare function stringWordCount({ string: string, wordsPerMinute?: int }): { words: int, characters: int, sentences: int, readingSeconds: int };

/**
 * Converts a byte array to a string using the UTF-8 lossy strategy
 * @param bytes
 * @returns string — Input String
 */
declare function utf8Lossy({ bytes: bytes[] }): string;


// === Utils/String/Case ===

/**
 * Converts a string to camelCase or PascalCase
 * @param string — Input String
 * @param pascalCase (optional) — Upper case the first word as well
 * @returns result — The converted string
 */
declare function stringCamelCase({ string: string, pascalCase?: bool }): string;

/**
 * Upper cases the first character and leaves the rest untouched
 * @param string — Input String
 * @returns result — The converted string
 */
declare function stringCapitalize({ string: string }): string;

/**
 * Rewrites a string in the chosen case style. The input's own style is detected automatically, so any of the supported styles can be fed in
 * @param string — Input String
 * @param targetCase (optional) — The case style to write the string in
 * @returns result — The converted string
 * @returns detectedCase — The case style the input was written in, or "undetermined" when it carries no evidence of one
 */
declare function stringConvertCase({ string: string, targetCase?: string }): { result: string, detectedCase: string };

/**
 * Names the case style a string is written in
 * @param string — Input String
 * @returns detectedCase — The detected case style, or "undetermined" when the string carries no evidence of one
 */
declare function stringDetectCase({ string: string }): string;

/**
 * Converts a string to kebab-case
 * @param string — Input String
 * @returns result — The converted string
 */
declare function stringKebabCase({ string: string }): string;

/**
 * Converts a string to snake_case
 * @param string — Input String
 * @returns result — The converted string
 */
declare function stringSnakeCase({ string: string }): string;

/**
 * Converts a string to Title Case
 * @param string — Input String
 * @returns result — The converted string
 */
declare function stringTitleCase({ string: string }): string;


// === Utils/String/Regex ===

/**
 * Extracts the capture groups of the first regular expression match
 * @param string — Input String
 * @param pattern — Regular expression pattern
 * @returns groups — Capture groups, index 0 is the whole match
 * @returns found — True when the pattern matched
 */
declare function stringRegexCaptures({ string: string, pattern: string }): { groups: string[], found: bool };

/**
 * Returns every match of a regular expression in a string
 * @param string — Input String
 * @param pattern — Regular expression pattern
 * @returns matches — All matching substrings
 * @returns count — Number of matches
 */
declare function stringRegexFindAll({ string: string, pattern: string }): { matches: string[], count: int };

/**
 * Checks whether a regular expression matches a string
 * @param string — Input String
 * @param pattern — Regular expression pattern
 * @returns isMatch — True when the pattern matches
 * @returns firstMatch — The first matching text, empty when there is no match
 */
declare function stringRegexMatch({ string: string, pattern: string }): { isMatch: bool, firstMatch: string };


// === Utils/String/Similarity ===

/**
 * Calculates the Damerau-Levenshtein distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @param normalize (optional) — Normalize the Distance
 * @returns distance — Damerau-Levenshtein Distance
 */
declare function damerauLevenshteinDistance({ string1: string, string2: string, normalize?: bool }): float;

/**
 * Calculates the Hamming distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns distance — Hamming Distance
 */
declare function hammingDistance({ string1: string, string2: string }): float;

/**
 * Calculates the Jaro distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns distance — Jaro Distance
 */
declare function jaroDistance({ string1: string, string2: string }): float;

/**
 * Calculates the Jaro-Winkler distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns distance — Jaro-Winkler Distance
 */
declare function jaroWinklerDistance({ string1: string, string2: string }): float;

/**
 * Calculates the Levenshtein distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @param normalize (optional) — Normalize the Distance
 * @returns distance — Levenshtein Distance
 */
declare function levenshteinDistance({ string1: string, string2: string, normalize?: bool }): float;

/**
 * Calculates the Optimal String Alignment distance between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns distance — Optimal String Alignment Distance
 */
declare function optimalStringAlignmentDistance({ string1: string, string2: string }): float;

/**
 * Calculates the Sørensen-Dice coefficient between two strings
 * @param string1 — First String
 * @param string2 — Second String
 * @returns coefficient — Sørensen-Dice Coefficient
 */
declare function sorensenDiceCoefficient({ string1: string, string2: string }): float;


// === Utils/Types ===

/**
 * Returns the input value if valid, otherwise returns the fallback default. Useful for handling optional values or error recovery.
 * @param value — The primary value to use if available and valid
 * @param default — Fallback value used when the primary value is null, missing, or invalid
 * @returns result — The resolved value (primary if valid, otherwise default)
 * @returns usedFallback — True if the fallback value was used
 */
declare function utilsTypesFallback({ value: any, default: any }): { result: any, usedFallback: bool };

/**
 * True for null, an empty string, an empty array and an empty struct
 * @param value — Value to inspect
 * @param trim (optional) — Treat whitespace-only text as empty
 * @returns isEmpty — True when the value holds nothing
 */
declare function utilsTypesIsEmpty({ value: any, trim?: bool }): bool;

/**
 * Selects between two values based on a boolean condition. Returns A if true, B if false.
 * @param a — Value returned when condition is true
 * @param b — Value returned when condition is false
 * @param condition (optional) — If true, returns A. If false, returns B.
 * @returns result — The selected value (A if true, B if false)
 */
declare function utilsTypesSelect({ a: any, b: any, condition?: bool }): any;

/**
 * Tries to transform cast types.
 * @param typeIn — Type to transform
 * @returns typeOut — If the type was successfully transformed, transformed type
 * @returns success — Determines of tje transformation was successful
 */
declare function utilsTypesTryTransform({ typeIn: any }): { typeOut: any, success: bool };

/**
 * Reports what a value actually is — useful for data coming back from an API or a model
 * @param value — Value to inspect
 * @returns type — One of null, boolean, number, string, array or object
 * @returns isNull — True when the value is missing
 * @returns size — Elements for an array, fields for an object, characters for a string, otherwise 0
 */
declare function utilsTypesTypeOf({ value: any }): { type: string, isNull: bool, size: int };


// === Utils/User ===

/**
 * Checks whether a project user has the specified role ID or exact role name.
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
declare function utilsUserCheckUserHasRole({ appId?: string, userId?: string, role?: string }): { hasRole: bool, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Checks whether a project user effectively has a permission. Owner and Admin imply all permissions.
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
declare function utilsUserCheckUserPermission({ appId?: string, userId?: string, permission?: string }): { hasPermission: bool, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Gets the current runtime user and, when available, their project membership, role, effective permissions, and attributes.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @returns currentUser — Current runtime user with project membership details when available.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetCurrentUser({ appId?: string }): { currentUser: Struct, success: bool, statusCode: int, error: string };

/**
 * Fetches the current user's persisted user information from the configured FlowLike hub's /api/v1/user/info endpoint when an execution token is available.
 * @returns userInfo — The user record returned by /api/v1/user/info
 * @returns success — True when user info was fetched successfully
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made
 * @returns error — Error message when user info could not be fetched
 */
declare function utilsUserGetCurrentUserInfo(): { userInfo: Struct, success: bool, statusCode: int, error: string };

/**
 * Gets a project user's effective permission bitfield and expanded permission names. Owner and Admin imply all permissions.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @returns userPermissions — Effective permissions for the project user.
 * @returns found — True when the user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetEffectiveUserPermissions({ appId?: string, userId?: string }): { userPermissions: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Gets the user context of the current execution. Returns a typed struct containing sub (user ID), role, permissions, attributes, and details of the calling principal. Use 'Break Struct' to access individual fields.
 * @returns userContext — The complete user execution context. Use 'Break Struct' to access: sub, role (with id, name, permissions, attributes), isTechnicalUser, keyId, principal, originAppId, onBehalfOf
 * @returns hasUser — True if user context is available
 */
declare function utilsUserGetExecutingUser(): { userContext: Struct, hasUser: bool };

/**
 * Gets a project user membership by user ID/sub.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
 * @returns found — True when a matching project user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetProjectUser({ appId?: string, userId?: string }): { projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Checks for one custom role attribute on a project user.
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
declare function utilsUserGetUserAttribute({ appId?: string, userId?: string, attribute?: string }): { hasAttribute: bool, attributeValue: string, projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Gets custom role attributes assigned to a project user.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @returns userAttributes — Role attributes for the project user.
 * @returns found — True when the user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetUserAttributes({ appId?: string, userId?: string }): { userAttributes: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Gets the project role assigned to a user.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param userId (optional) — User subject / user ID within the project.
 * @returns userRoles — Role assignment for the project user. Current projects have one role per user.
 * @returns found — True when the user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserGetUserRoles({ appId?: string, userId?: string }): { userRoles: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Checks if the executing user's role has a specific attribute (tag). Attributes are custom string tags assigned to roles for flexible authorization. Returns false if no user context is available or the user has no role.
 * @param attribute (optional) — The attribute (tag) to check for
 * @returns hasAttribute — True if the user's role has the specified attribute
 */
declare function utilsUserHasAttribute({ attribute?: string }): bool;

/**
 * Checks if the executing user has a specific permission. Admin and Owner roles automatically have all permissions. Returns false if no user context is available.
 * @param permission (optional) — The permission to check for
 * @returns hasPermission — True if the user has the specified permission (or is Admin/Owner)
 */
declare function utilsUserHasPermission({ permission?: string }): bool;

/**
 * Checks whether a machine rather than a person triggered this run. Machine callers have no human identity (sub): an API key reports its Key ID, an app calling through an app connection reports the calling app instead.
 * @returns isTechnical — True if a machine triggered the run (API key or app connection), false for a person
 * @returns keyId — The API key identifier, empty for every other caller
 * @returns principal — How the caller authenticated: 'user', 'apiKey' or 'connectedApp'
 * @returns originAppId — The app that made the call when the principal is 'connectedApp', empty otherwise
 * @returns onBehalfOf — The user the caller reported as the initiator: an API key's creator, or the user an app connection passed through. Attribution only — never authorize against it
 */
declare function utilsUserIsTechnicalUser(): { isTechnical: bool, keyId: string, principal: string, originAppId: string, onBehalfOf: string };

/**
 * Lists project users with pagination.
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
declare function utilsUserListProjectUsers({ appId?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

/**
 * Lists project users whose assigned role contains a custom attribute.
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
declare function utilsUserListUsersWithAttribute({ appId?: string, attribute?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

/**
 * Lists project users assigned to a role ID or exact role name.
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
declare function utilsUserListUsersWithRole({ appId?: string, role?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

/**
 * Resolves a project user by user ID/sub or by email when email is exposed by platform lookup settings. Email matching is constrained to project members.
 * @param appId (optional) — Project/app ID. Leave empty to use the current execution app.
 * @param identifier (optional) — Email, sub, or user ID to resolve within the project.
 * @param identifierType (optional) — How to interpret the identifier.
 * @returns projectUser — Project membership, sanitized user ref, role, effective permissions, and attributes.
 * @returns found — True when a matching project user was found.
 * @returns success — True when the read operation completed successfully.
 * @returns statusCode — HTTP status code returned by the hub, or 0 if no request was made.
 * @returns error — Error message when the read operation could not complete.
 */
declare function utilsUserResolveUser({ appId?: string, identifier?: string, identifierType?: string }): { projectUser: Struct, found: bool, success: bool, statusCode: int, error: string };

/**
 * Searches project users by exposed profile fields. Email is only searchable when the platform returns email in user lookup results.
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
declare function utilsUserSearchUsers({ appId?: string, query?: string, offset?: int, limit?: int }): { users: Struct[], count: int, nextOffset: int, hasMore: bool, success: bool, statusCode: int, error: string };

