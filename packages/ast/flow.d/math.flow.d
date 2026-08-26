// Math — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

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

declare namespace math {
    // === Math ===

    /**
     * Evaluates a mathematical expression
     * @node eval @alias eval
     * @param expression — Mathematical expression
     * @returns result — Result of the expression
     */
    function eval({ expression: string }): float;
}
