package com.flow_like.app

import org.json.JSONObject
import kotlin.math.max
import kotlin.math.min

internal data class NativeChartDomain(val minimum: Double, val maximum: Double) {
    fun fraction(value: Double): Float = ((value - minimum) / (maximum - minimum)).coerceIn(0.0, 1.0).toFloat()
    companion object {
        fun calculate(points: List<JSONObject>, stacked: Boolean): NativeChartDomain {
            val finite = points.filter { it.optDouble("value").isFinite() }
            val values = if (stacked) finite.groupBy { it.optString("label") }.values.flatMap { group ->
                listOf(group.sumOf { max(0.0, it.optDouble("value")) }, group.sumOf { min(0.0, it.optDouble("value")) })
            } else finite.map { it.optDouble("value") }
            val low = min(0.0, values.minOrNull() ?: 0.0)
            val high = max(0.0, values.maxOrNull() ?: 0.0)
            return if (low == high) NativeChartDomain(0.0, 1.0) else NativeChartDomain(low, high)
        }
    }
}
