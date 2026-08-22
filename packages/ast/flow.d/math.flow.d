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
 * Provides a mathematical constant such as Pi or E
 * @param constant (optional) — Which constant to emit
 * @returns value — The value of the constant
 */
declare function floatConstant({ constant?: string }): float;

/**
 * Takes the magnitude of the first float and the sign of the second
 * @param float1 — Input Float
 * @param float2 — Input Float
 * @returns result — Takes the magnitude of the first float and the sign of the second
 */
declare function floatCopysign({ float1: float, float2: float }): float;

/**
 * Divides one float by another
 * @param dividend — The number to be divided
 * @param divisor — The number to divide by
 * @returns quotient — The result of the division
 */
declare function floatDivide({ dividend: float, divisor: float }): float;

/**
 * Raises e to the power of a float
 * @param float — Input Float
 * @returns result — Raises e to the power of a float
 */
declare function floatExp({ float: float }): float;

/**
 * Raises two to the power of a float
 * @param float — Input Float
 * @returns result — Raises two to the power of a float
 */
declare function floatExp2({ float: float }): float;

/**
 * Rounds a float down to the nearest integer
 * @param float — Input Float
 * @returns floor — The floor of the float
 */
declare function floatFloor({ float: float }): int;

/**
 * Keeps only the fractional part of a float
 * @param float — Input Float
 * @returns result — Keeps only the fractional part of a float
 */
declare function floatFract({ float: float }): float;

/**
 * Length of the hypotenuse of a right-angled triangle
 * @param float1 — Input Float
 * @param float2 — Input Float
 * @returns result — Length of the hypotenuse of a right-angled triangle
 */
declare function floatHypot({ float1: float, float2: float }): float;

/**
 * Interpolates linearly between two floats
 * @param from (optional) — Value at t = 0
 * @param to (optional) — Value at t = 1
 * @param t (optional) — Interpolation factor
 * @param clamp (optional) — Clamp the factor into the range 0 to 1
 * @returns result — The interpolated value
 */
declare function floatLerp({ from?: float, to?: float, t?: float, clamp?: bool }): float;

/**
 * Natural logarithm of a float
 * @param float — Input Float
 * @returns result — Natural logarithm of a float
 */
declare function floatLn({ float: float }): float;

/**
 * Logarithm of a float to a custom base
 * @param float — Input Float
 * @param base (optional) — Logarithm base
 * @returns result — Logarithm of a float to a custom base
 */
declare function floatLog({ float: float, base?: float }): float;

/**
 * Base 10 logarithm of a float
 * @param float — Input Float
 * @returns result — Base 10 logarithm of a float
 */
declare function floatLog10({ float: float }): float;

/**
 * Base 2 logarithm of a float
 * @param float — Input Float
 * @returns result — Base 2 logarithm of a float
 */
declare function floatLog2({ float: float }): float;

/**
 * Rescales a value from one range into another
 * @param value — Value to rescale
 * @param inMin (optional) — In Min
 * @param inMax (optional) — In Max
 * @param outMin (optional) — Out Min
 * @param outMax (optional) — Out Max
 * @param clamp (optional) — Keep the result inside the output range
 * @returns result — The rescaled value
 */
declare function floatMapRange({ value: float, inMin?: float, inMax?: float, outMin?: float, outMax?: float, clamp?: bool }): float;

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
 * Remainder of a float division
 * @param float1 — Dividend
 * @param float2 — Divisor
 * @param euclidean (optional) — Return a non-negative remainder
 * @returns remainder — Remainder of the division
 */
declare function floatModulo({ float1: float, float2: float, euclidean?: bool }): float;

/**
 * Multiplies two floats and adds a third in one rounding step
 * @param float — Input Float
 * @param factor (optional) — Multiplied with the input
 * @param addend (optional) — Added to the product
 * @returns result — Input multiplied by the factor plus the addend
 */
declare function floatMulAdd({ float: float, factor?: float, addend?: float }): float;

/**
 * Multiplies two floats together
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns product — The product of the two floats
 */
declare function floatMultiply({ float1: float, float2: float }): float;

/**
 * How much a value moved relative to where it started
 * @param from (optional) — Earlier value
 * @param to (optional) — Later value
 * @returns percent — Change in percent, negative when the value fell
 * @returns delta — Absolute change
 * @returns defined — False when the earlier value was zero, which has no percentage
 */
declare function floatPercentChange({ from?: float, to?: float }): { percent: float, delta: float, defined: bool };

/**
 * Calculates the power of a float
 * @param base — Base float
 * @param exponent — Exponent float
 * @returns power — Result of the power calculation
 */
declare function floatPower({ base: float, exponent: float }): float;

/**
 * One divided by a float
 * @param float — Input Float
 * @returns result — One divided by a float
 */
declare function floatRecip({ float: float }): float;

/**
 * Calculates the nth root of a float
 * @param radicand — The float to take the root of
 * @param degree — The degree of the root
 * @returns root — Result of the root calculation
 */
declare function floatRoot({ radicand: float, degree: int }): float;

/**
 * Rounds a float to the given number of decimal places
 * @param float — Input Float
 * @param decimals (optional) — Number of decimal places to keep
 * @returns rounded — The rounded float
 */
declare function floatRound({ float: float, decimals?: int }): float;

/**
 * Snaps a value to the nearest multiple, for example the nearest 0.05 or 25
 * @param value — Value to snap
 * @param multiple (optional) — Step size to snap to
 * @param mode (optional) — Which direction to snap in
 * @returns result — The snapped value
 */
declare function floatRoundToMultiple({ value: float, multiple?: float, mode?: string }): float;

/**
 * Returns -1 or 1 depending on the sign of a float
 * @param float — Input Float
 * @returns result — Returns -1 or 1 depending on the sign of a float
 */
declare function floatSignum({ float: float }): float;

/**
 * Square root of a float
 * @param float — Input Float
 * @returns result — Square root of a float
 */
declare function floatSqrt({ float: float }): float;

/**
 * Subtracts one float from another
 * @param float1 — First Float
 * @param float2 — Second Float
 * @returns difference — The difference between the two floats
 */
declare function floatSubtract({ float1: float, float2: float }): float;

/**
 * Converts a float into an integer using the selected rounding
 * @param float — Input Float
 * @param rounding (optional) — How to remove the fractional part
 * @returns integer — The converted value
 */
declare function floatToInt({ float: float, rounding?: string }): int;

/**
 * Drops the fractional part of a float
 * @param float — Input Float
 * @returns result — Drops the fractional part of a float
 */
declare function floatTrunc({ float: float }): float;


// === Math/Float/Aggregate ===

/**
 * Arithmetic mean of every float in an array
 * @param floats — Input Floats
 * @returns result — Arithmetic mean of every float in an array
 * @returns empty — True when the input array held no values
 */
declare function floatAverage({ floats: float[] }): { result: float, empty: bool };

/**
 * Largest float in an array
 * @param floats — Input Floats
 * @returns result — Largest float in an array
 * @returns empty — True when the input array held no values
 */
declare function floatMaxOf({ floats: float[] }): { result: float, empty: bool };

/**
 * Middle value of an array, averaging the two middle values for even counts
 * @param floats — Input Floats
 * @returns result — Middle value of an array, averaging the two middle values for even counts
 * @returns empty — True when the input array held no values
 */
declare function floatMedian({ floats: float[] }): { result: float, empty: bool };

/**
 * Smallest float in an array
 * @param floats — Input Floats
 * @returns result — Smallest float in an array
 * @returns empty — True when the input array held no values
 */
declare function floatMinOf({ floats: float[] }): { result: float, empty: bool };

/**
 * Value at a percentile of an array, interpolating between neighbours
 * @param floats — Input Floats
 * @param percentile (optional) — Percentile between 0 and 100
 * @returns result — Value at a percentile of an array, interpolating between neighbours
 * @returns empty — True when the input array held no values
 */
declare function floatPercentile({ floats: float[], percentile?: float }): { result: float, empty: bool };

/**
 * Population standard deviation of every float in an array
 * @param floats — Input Floats
 * @returns result — Population standard deviation of every float in an array
 * @returns empty — True when the input array held no values
 */
declare function floatStdDev({ floats: float[] }): { result: float, empty: bool };

/**
 * Adds up every float in an array
 * @param floats — Input Floats
 * @returns result — Adds up every float in an array
 * @returns empty — True when the input array held no values
 */
declare function floatSum({ floats: float[] }): { result: float, empty: bool };

/**
 * Population variance of every float in an array
 * @param floats — Input Floats
 * @returns result — Population variance of every float in an array
 * @returns empty — True when the input array held no values
 */
declare function floatVariance({ floats: float[] }): { result: float, empty: bool };


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
 * True when the value is a real, finite number
 * @param float — Input Float
 * @returns result — True when the value is a real, finite number
 */
declare function floatIsFinite({ float: float }): bool;

/**
 * True when the value is positive or negative infinity
 * @param float — Input Float
 * @returns result — True when the value is positive or negative infinity
 */
declare function floatIsInfinite({ float: float }): bool;

/**
 * True when the value is missing or not a real number
 * @param float — Input Float
 * @returns result — True when the value is missing or not a real number
 */
declare function floatIsNan({ float: float }): bool;

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


// === Math/Float/Trigonometry ===

/**
 * Arc cosine in radians, input must be between -1 and 1
 * @param float — Input Float
 * @returns result — Arc cosine in radians, input must be between -1 and 1
 */
declare function floatAcos({ float: float }): float;

/**
 * Arc sine in radians, input must be between -1 and 1
 * @param float — Input Float
 * @returns result — Arc sine in radians, input must be between -1 and 1
 */
declare function floatAsin({ float: float }): float;

/**
 * Arc tangent in radians
 * @param float — Input Float
 * @returns result — Arc tangent in radians
 */
declare function floatAtan({ float: float }): float;

/**
 * Angle in radians between the positive x axis and the point (x, y)
 * @param float1 — Input Float
 * @param float2 — Input Float
 * @returns result — Angle in radians between the positive x axis and the point (x, y)
 */
declare function floatAtan2({ float1: float, float2: float }): float;

/**
 * Cosine of an angle in radians
 * @param float — Input Float
 * @returns result — Cosine of an angle in radians
 */
declare function floatCos({ float: float }): float;

/**
 * Hyperbolic cosine
 * @param float — Input Float
 * @returns result — Hyperbolic cosine
 */
declare function floatCosh({ float: float }): float;

/**
 * Sine of an angle in radians
 * @param float — Input Float
 * @returns result — Sine of an angle in radians
 */
declare function floatSin({ float: float }): float;

/**
 * Hyperbolic sine
 * @param float — Input Float
 * @returns result — Hyperbolic sine
 */
declare function floatSinh({ float: float }): float;

/**
 * Tangent of an angle in radians
 * @param float — Input Float
 * @returns result — Tangent of an angle in radians
 */
declare function floatTan({ float: float }): float;

/**
 * Hyperbolic tangent
 * @param float — Input Float
 * @returns result — Hyperbolic tangent
 */
declare function floatTanh({ float: float }): float;

/**
 * Converts radians into degrees
 * @param float — Input Float
 * @returns result — Converts radians into degrees
 */
declare function floatToDegrees({ float: float }): float;

/**
 * Converts degrees into radians
 * @param float — Input Float
 * @returns result — Converts degrees into radians
 */
declare function floatToRadians({ float: float }): float;


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
 * Decreases an integer by a step
 * @param integer — Input Integer
 * @param step (optional) — Step width
 * @returns result — Decreases an integer by a step
 */
declare function intDecrement({ integer: int, step?: int }): int;

/**
 * Divides two integers and truncates towards zero
 * @param integer1 — Dividend
 * @param integer2 — Divisor
 * @returns result — Truncated quotient
 * @returns success — False when the divisor was zero
 */
declare function intDiv({ integer1: int, integer2: int }): { result: int, success: bool };

/**
 * Divides two integers and rounds towards negative infinity
 * @param integer1 — Dividend
 * @param integer2 — Divisor
 * @returns result — Euclidean quotient
 * @returns success — False when the divisor was zero
 */
declare function intDivEuclid({ integer1: int, integer2: int }): { result: int, success: bool };

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
 * Parses an integer from binary, octal, decimal or hexadecimal text
 * @param string — Text to parse
 * @param radix (optional) — Numeric base
 * @returns integer — The parsed integer
 * @returns success — True when the text was a valid number in that base
 */
declare function intFromRadix({ string: string, radix?: string }): { integer: int, success: bool };

/**
 * Largest integer that divides both inputs
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns result — Largest integer that divides both inputs
 */
declare function intGcd({ integer1: int, integer2: int }): int;

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
 * Increases an integer by a step
 * @param integer — Input Integer
 * @param step (optional) — Step width
 * @returns result — Increases an integer by a step
 */
declare function intIncrement({ integer: int, step?: int }): int;

/**
 * Checks whether an integer is divisible by two
 * @param integer — Input Integer
 * @returns result — Checks whether an integer is divisible by two
 */
declare function intIsEven({ integer: int }): bool;

/**
 * Checks whether an integer is less than zero
 * @param integer — Input Integer
 * @returns result — Checks whether an integer is less than zero
 */
declare function intIsNegative({ integer: int }): bool;

/**
 * Checks whether an integer is not divisible by two
 * @param integer — Input Integer
 * @returns result — Checks whether an integer is not divisible by two
 */
declare function intIsOdd({ integer: int }): bool;

/**
 * Checks whether an integer is greater than zero
 * @param integer — Input Integer
 * @returns result — Checks whether an integer is greater than zero
 */
declare function intIsPositive({ integer: int }): bool;

/**
 * Smallest positive integer that both inputs divide
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns result — Smallest positive integer that both inputs divide
 */
declare function intLcm({ integer1: int, integer2: int }): int;

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
 * The smallest and largest representable integer
 * @returns min — Smallest representable integer
 * @returns max — Largest representable integer
 */
declare function intLimits(): { min: int, max: int };

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
 * Flips the sign of an integer
 * @param integer — Input Integer
 * @returns result — Flips the sign of an integer
 */
declare function intNegate({ integer: int }): int;

/**
 * Calculates the power of an integer
 * @param base — Base integer
 * @param exponent — Exponent integer
 * @returns power — Result of the power calculation
 */
declare function intPower({ base: int, exponent: int }): int;

/**
 * Remainder that is always positive, unlike the % operator
 * @param integer1 — Dividend
 * @param integer2 — Divisor
 * @returns result — Non-negative remainder
 * @returns success — False when the divisor was zero
 */
declare function intRemEuclid({ integer1: int, integer2: int }): { result: int, success: bool };

/**
 * Calculates the nth root of an integer
 * @param radicand — The integer to take the root of
 * @param degree — The degree of the root
 * @returns root — Result of the root calculation
 */
declare function intRoot({ radicand: int, degree: int }): float;

/**
 * Returns -1, 0 or 1 depending on the sign of an integer
 * @param integer — Input Integer
 * @returns result — Returns -1, 0 or 1 depending on the sign of an integer
 */
declare function intSignum({ integer: int }): int;

/**
 * Subtracts two Integers
 * @param integer1 — Minuend
 * @param integer2 — Subtrahend
 * @returns difference — Difference of the two integers
 */
declare function intSubtract({ integer1: int, integer2: int }): int;

/**
 * Converts an integer into a float
 * @param integer — Input Integer
 * @returns float — The converted value
 */
declare function intToFloat({ integer: int }): float;

/**
 * Formats an integer as binary, octal, decimal or hexadecimal text
 * @param integer — Input Integer
 * @param radix (optional) — Numeric base
 * @param uppercase (optional) — Use upper case letters for hexadecimal digits
 * @returns string — The formatted number
 */
declare function intToRadix({ integer: int, radix?: string, uppercase?: bool }): string;

/**
 * Checks if two integers are unequal
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns unequal — True if the integers are unequal, false otherwise
 */
declare function intUnequal({ integer1: int, integer2: int }): bool;


// === Math/Int/Aggregate ===

/**
 * Arithmetic mean of every integer in an array
 * @param integers — Input Integers
 * @returns result — Arithmetic mean of every integer in an array
 * @returns empty — True when the input array held no values
 */
declare function intAverage({ integers: int[] }): { result: float, empty: bool };

/**
 * Largest integer in an array
 * @param integers — Input Integers
 * @returns result — Largest integer in an array
 * @returns empty — True when the input array held no values
 */
declare function intMaxOf({ integers: int[] }): { result: int, empty: bool };

/**
 * Smallest integer in an array
 * @param integers — Input Integers
 * @returns result — Smallest integer in an array
 * @returns empty — True when the input array held no values
 */
declare function intMinOf({ integers: int[] }): { result: int, empty: bool };

/**
 * Multiplies every integer in an array
 * @param integers — Input Integers
 * @returns result — Multiplies every integer in an array
 * @returns empty — True when the input array held no values
 */
declare function intProduct({ integers: int[] }): { result: int, empty: bool };

/**
 * Adds up every integer in an array
 * @param integers — Input Integers
 * @returns result — Adds up every integer in an array
 * @returns empty — True when the input array held no values
 */
declare function intSum({ integers: int[] }): { result: int, empty: bool };


// === Math/Int/Bitwise ===

/**
 * Bitwise AND of two integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns result — Bitwise AND of two integers
 */
declare function intBitand({ integer1: int, integer2: int }): int;

/**
 * Inverts every bit of an integer
 * @param integer — Input Integer
 * @returns result — The integer with all bits inverted
 */
declare function intBitnot({ integer: int }): int;

/**
 * Bitwise OR of two integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns result — Bitwise OR of two integers
 */
declare function intBitor({ integer1: int, integer2: int }): int;

/**
 * Bitwise XOR of two integers
 * @param integer1 — Input Integer
 * @param integer2 — Input Integer
 * @returns result — Bitwise XOR of two integers
 */
declare function intBitxor({ integer1: int, integer2: int }): int;

/**
 * Number of bits that are set to one
 * @param integer — Input Integer
 * @returns result — Number of bits that are set to one
 */
declare function intCountOnes({ integer: int }): int;

/**
 * Number of zero bits before the highest set bit
 * @param integer — Input Integer
 * @returns result — Number of zero bits before the highest set bit
 */
declare function intLeadingZeros({ integer: int }): int;

/**
 * Shifts the bits of an integer to the left
 * @param integer — Input Integer
 * @param shift (optional) — Number of bit positions to shift by
 * @returns result — Shifts the bits of an integer to the left
 */
declare function intShl({ integer: int, shift?: int }): int;

/**
 * Shifts the bits of an integer to the right
 * @param integer — Input Integer
 * @param shift (optional) — Number of bit positions to shift by
 * @returns result — Shifts the bits of an integer to the right
 */
declare function intShr({ integer: int, shift?: int }): int;

/**
 * Number of zero bits after the lowest set bit
 * @param integer — Input Integer
 * @returns result — Number of zero bits after the lowest set bit
 */
declare function intTrailingZeros({ integer: int }): int;


// === Math/Int/Overflow ===

/**
 * Arithmetic that reports overflow and division by zero instead of failing
 * @param integer1 — Left hand side
 * @param integer2 — Right hand side
 * @param operation (optional) — Arithmetic operation to apply
 * @returns result — Arithmetic that reports overflow and division by zero instead of failing
 * @returns success — False on overflow or division by zero
 */
declare function intCheckedOp({ integer1: int, integer2: int, operation?: string }): { result: int, success: bool };

/**
 * Arithmetic that clamps to the integer limits instead of overflowing
 * @param integer1 — Left hand side
 * @param integer2 — Right hand side
 * @param operation (optional) — Arithmetic operation to apply
 * @returns result — Arithmetic that clamps to the integer limits instead of overflowing
 */
declare function intSaturatingOp({ integer1: int, integer2: int, operation?: string }): int;

/**
 * Arithmetic that wraps around the integer limits
 * @param integer1 — Left hand side
 * @param integer2 — Right hand side
 * @param operation (optional) — Arithmetic operation to apply
 * @returns result — Arithmetic that wraps around the integer limits
 */
declare function intWrappingOp({ integer1: int, integer2: int, operation?: string }): int;


// === Math/Int/Random ===

/**
 * Generates a random integer within a specified range
 * @param min — Minimum Value
 * @param max — Maximum Value
 * @returns randomInteger — The generated random integer
 */
declare function intRandomInRange({ min: int, max: int }): int;

