// Math — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Math ===

/**
 * Evaluates a mathematical expression
 * @param expression — Mathematical expression
 * @returns result — Result of the expression
 */
declare function eval({ expression: string }): float;


// === Math/Float ===

/**
 * Calculates the absolute value of a float
 * @param float — Input Float
 * @returns absolute — The absolute value of the float
 */
declare function floatAbs({ float: float }): float;

/**
 * Adds two floats together
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns sum — The sum of the two floats
 */
declare function floatAdd({ float1: float, float2: float }): float;

/**
 * Rounds a float up to the nearest integer
 * @param float — Input Float
 * @returns ceiling — The ceiling of the float
 */
declare function floatCeil({ float: float }): int;

/**
 * Clamps a float within a given range
 * @param float — Input Float
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns clamped — The clamped float
 */
declare function floatClamp({ float: float, min: float, max: float }): float;

/**
 * Divides one float by another
 * @param dividend — The number to be divided
 * @param divisor — The number to divide by
 * @returns quotient — The result of the division
 */
declare function floatDivide({ dividend: float, divisor: float }): float;

/**
 * Rounds a float down to the nearest integer
 * @param float — Input Float
 * @returns floor — The floor of the float
 */
declare function floatFloor({ float: float }): int;

/**
 * Returns the larger of two floats
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns maximum — The larger of the two floats
 */
declare function floatMax({ float1: float, float2: float }): float;

/**
 * Returns the smaller of two floats
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns minimum — The smaller of the two floats
 */
declare function floatMin({ float1: float, float2: float }): float;

/**
 * Multiplies two floats together
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns product — The product of the two floats
 */
declare function floatMultiply({ float1: float, float2: float }): float;

/**
 * Calculates the power of a float
 * @param base — Base float
 * @param exponent — Exponent float
 * @returns power — Result of the power calculation
 */
declare function floatPower({ base: float, exponent: float }): float;

/**
 * Calculates the nth root of a float
 * @param radicand — The float to take the root of
 * @param degree — The degree of the root
 * @returns root — Result of the root calculation
 */
declare function floatRoot({ radicand: float, degree: int }): float;

/**
 * Rounds a float to the nearest integer
 * @param float — Input Float
 * @returns rounded — The rounded float
 */
declare function floatRound({ float: float }): float;

/**
 * Subtracts one float from another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns difference — The difference between the two floats
 */
declare function floatSubtract({ float1: float, float2: float }): float;


// === Math/Float/Comparison ===

/**
 * Checks if two floats are equal (within a tolerance)
 * @param float1 — First Float
 * @param float2 — Second Float
 * @param tolerance — Comparison Tolerance
 * @returns isEqual — True if the floats are equal, false otherwise
 */
declare function floatEqual({ float1: float, float2: float, tolerance: float }): bool;

/**
 * Checks if one float is greater than another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns isGreater — True if float1 is greater than float2, false otherwise
 */
declare function floatGreaterThan({ float1: float, float2: float }): bool;

/**
 * Checks if one float is greater than or equal to another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns isGreaterOrEqual — True if float1 is greater than or equal to float2, false otherwise
 */
declare function floatGreaterThanOrEqual({ float1: float, float2: float }): bool;

/**
 * Checks if one float is less than another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns isLess — True if float1 is less than float2, false otherwise
 */
declare function floatLessThan({ float1: float, float2: float }): bool;

/**
 * Checks if one float is less than or equal to another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns isLessOrEqual — True if float1 is less than or equal to float2, false otherwise
 */
declare function floatLessThanOrEqual({ float1: float, float2: float }): bool;

/**
 * Checks if two floats are unequal (within a tolerance)
 * @param float1 — First Float
 * @param float2 — Second Float
 * @param tolerance — Comparison Tolerance
 * @returns isUnequal — True if the floats are unequal, false otherwise
 */
declare function floatUnequal({ float1: float, float2: float, tolerance: float }): bool;


// === Math/Float/Random ===

/**
 * Generates a random float within a specified range
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns randomFloat — The generated random float
 */
declare function floatRandomInRange({ min: float, max: float }): float;


// === Math/Int ===

/**
 * Returns the absolute value of an Integer
 * @param integer — Input Integer
 * @returns absolute — Absolute Value
 */
declare function intAbs({ integer: int }): int;

/**
 * Adds two Integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns sum — Sum of the two integers
 */
declare function intAdd({ integer1: int, integer2: int }): int;

/**
 * Clamps an integer within a range
 * @param integer — Input Integer
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns clamped — Clamped Value
 */
declare function intClamp({ integer: int, min: int, max: int }): int;

/**
 * Divides two Integers (handles division by zero)
 * @param integer1 — Dividend
 * @param integer2 — Divisor
 * @returns result — Result of the division
 */
declare function intDivide({ integer1: int, integer2: int }): float;

/**
 * Checks if two integers are equal
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns equal — True if the integers are equal, false otherwise
 */
declare function intEqual({ integer1: int, integer2: int }): bool;

/**
 * Checks if the first integer is greater than the second
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns greaterThan — True if integer1 > integer2, false otherwise
 */
declare function intGreaterThan({ integer1: int, integer2: int }): bool;

/**
 * Checks if the first integer is greater than or equal to the second
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns greaterThanOrEqual — True if integer1 >= integer2, false otherwise
 */
declare function intGreaterThanOrEqual({ integer1: int, integer2: int }): bool;

/**
 * Checks if the first integer is less than the second
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns lessThan — True if integer1 < integer2, false otherwise
 */
declare function intLessThan({ integer1: int, integer2: int }): bool;

/**
 * Checks if the first integer is less than or equal to the second
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns lessThanOrEqual — True if integer1 <= integer2, false otherwise
 */
declare function intLessThanOrEqual({ integer1: int, integer2: int }): bool;

/**
 * Returns the larger of two integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns maximum — The larger of the two integers
 */
declare function intMax({ integer1: int, integer2: int }): int;

/**
 * Returns the smaller of two integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns minimum — The smaller of the two integers
 */
declare function intMin({ integer1: int, integer2: int }): int;

/**
 * Calculates the remainder of integer division
 * @param integer1 — Dividend
 * @param integer2 — Divisor
 * @returns remainder — Remainder of the division
 */
declare function intModulo({ integer1: int, integer2: int }): int;

/**
 * Multiplies two Integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns product — Product of the two integers
 */
declare function intMultiply({ integer1: int, integer2: int }): int;

/**
 * Calculates the power of an integer
 * @param base — Base integer
 * @param exponent — Exponent integer
 * @returns power — Result of the power calculation
 */
declare function intPower({ base: int, exponent: int }): int;

/**
 * Calculates the nth root of an integer
 * @param radicand — The integer to take the root of
 * @param degree — The degree of the root
 * @returns root — Result of the root calculation
 */
declare function intRoot({ radicand: int, degree: int }): float;

/**
 * Subtracts two Integers
 * @param integer1 — Minuend
 * @param integer2 — Subtrahend
 * @returns difference — Difference of the two integers
 */
declare function intSubtract({ integer1: int, integer2: int }): int;

/**
 * Checks if two integers are unequal
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns unequal — True if the integers are unequal, false otherwise
 */
declare function intUnequal({ integer1: int, integer2: int }): bool;


// === Math/Int/Random ===

/**
 * Generates a random integer within a specified range
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns randomInteger — The generated random integer
 */
declare function intRandomInRange({ min: int, max: int }): int;

