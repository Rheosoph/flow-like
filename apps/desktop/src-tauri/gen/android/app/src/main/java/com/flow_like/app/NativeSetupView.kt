package com.flow_like.app

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.drawable.ClipDrawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.LayerDrawable
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.TextView
import androidx.core.content.ContextCompat
import androidx.core.content.res.ResourcesCompat
import androidx.core.view.ViewCompat
import com.google.android.material.button.MaterialButton
import java.text.NumberFormat

/** Kept in the base module so setup has the app's identity before the tools arrive. */
class NativeSetupView(context: Context) : ScrollView(context) {
    private val regular = ResourcesCompat.getFont(context, R.font.flow_inter_regular)
    private val medium = ResourcesCompat.getFont(context, R.font.flow_inter_medium)
    private val textColors = mutableListOf<Pair<TextView, Int>>()
    private val steps = mutableListOf<TextView>()
    private val stages = LinearLayout(context)
    private var currentStage: Int? = 0
    private val card = LinearLayout(context)
    private val percentage = label("", 14f, R.color.setup_accent, emphasized = true)

    val message = label("Checking installed tools…", 18f, emphasized = true).apply {
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        ViewCompat.setAccessibilityHeading(this, true)
    }
    val detail = label("", 14f, R.color.setup_muted)
    val progress = ProgressBar(context, null, android.R.attr.progressBarStyleHorizontal).apply {
        max = 100
        isIndeterminate = true
        contentDescription = "Tool download progress"
    }
    val primary = button().apply { visibility = View.GONE }
    val secondary = button().apply { text = "Close" }
    val storage = button().apply { text = "Manage storage"; visibility = View.GONE }

    init {
        isFillViewport = true
        clipToPadding = false
        val content = object : LinearLayout(context) {
            override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
                val width = minOf(MeasureSpec.getSize(widthMeasureSpec), dp(496))
                super.onMeasure(MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY), heightMeasureSpec)
            }
        }.apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24), dp(32), dp(24), dp(32))
        }
        val brand = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        brand.addView(ImageView(context).apply {
            setImageResource(R.drawable.ic_notification)
            scaleType = ImageView.ScaleType.FIT_CENTER
            setPadding(dp(5), dp(5), dp(5), dp(5))
            background = GradientDrawable(GradientDrawable.Orientation.TL_BR,
                intArrayOf(Color.rgb(255, 141, 40), Color.rgb(255, 56, 60))).apply {
                cornerRadius = dp(15).toFloat()
            }
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }, LinearLayout.LayoutParams(dp(52), dp(52)))
        brand.addView(label("Flow-Like", 21f, emphasized = true),
            LinearLayout.LayoutParams(-2, -2).apply { marginStart = dp(14) })
        content.addView(brand)
        content.addView(label("ON THIS DEVICE", 11f, R.color.setup_accent, emphasized = true).apply {
            letterSpacing = 0.12f
        }, row(top = 36))
        content.addView(label("Set up Flow-Like", 30f, emphasized = true).apply {
            ViewCompat.setAccessibilityHeading(this, true)
        }, row(top = 10))
        content.addView(label("Download the tools for bots, Events, and editing. They stay on this device after setup.",
            16f, R.color.setup_muted), row(top = 14))

        card.orientation = LinearLayout.VERTICAL
        card.setPadding(dp(22), dp(24), dp(22), dp(22))
        stages.orientation = LinearLayout.HORIZONTAL
        listOf("1  Download", "2  Install", "3  Open").forEach { title ->
            val step = label(title, 12f, R.color.setup_muted, emphasized = true)
            steps.add(step)
            stages.addView(step, LinearLayout.LayoutParams(0, -2, 1f).apply { marginEnd = dp(4) })
        }
        card.addView(stages)
        card.addView(message, row(top = 26))
        card.addView(progress, LinearLayout.LayoutParams(-1, dp(8)).apply { topMargin = dp(20) })
        card.addView(percentage, row(top = 10))
        card.addView(detail, row(top = 10))
        card.addView(primary, row(top = 22))
        card.addView(storage, row(top = 12))
        card.addView(secondary, row(top = 12))
        content.addView(card, row(top = 28))
        content.addView(label("Flow-Like will open when setup is complete.", 12f, R.color.setup_muted).apply {
            gravity = Gravity.CENTER
        }, row(top = 22))

        addView(FrameLayout(context).apply {
            addView(content, FrameLayout.LayoutParams(-1, -2, Gravity.CENTER))
        }, LayoutParams(-1, -2))
        applyPalette()
        showProgress(null)
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val contentWidth = minOf(MeasureSpec.getSize(widthMeasureSpec) - paddingLeft - paddingRight, dp(496))
        val available = contentWidth - dp(92)
        val needed = steps.maxOf { it.paint.measureText(it.text.toString()) + dp(8) } * steps.size
        val orientation = if (available < needed) LinearLayout.VERTICAL else LinearLayout.HORIZONTAL
        if (stages.orientation != orientation) {
            stages.orientation = orientation
            steps.forEachIndexed { index, view ->
                view.layoutParams = if (orientation == LinearLayout.VERTICAL) {
                    LinearLayout.LayoutParams(-1, -2).apply { if (index < steps.lastIndex) bottomMargin = dp(8) }
                } else {
                    LinearLayout.LayoutParams(0, -2, 1f).apply { marginEnd = dp(4) }
                }
            }
        }
        super.onMeasure(widthMeasureSpec, heightMeasureSpec)
    }

    fun showStage(stage: Int?) {
        currentStage = stage
        steps.forEachIndexed { index, text ->
            text.setTextColor(color(if (stage != null && index <= stage) R.color.setup_accent else R.color.setup_muted))
        }
    }

    fun showProgress(percent: Int?) {
        progress.isIndeterminate = percent == null
        percentage.visibility = if (percent == null) View.GONE else View.VISIBLE
        if (percent != null) {
            progress.progress = percent
            percentage.text = NumberFormat.getPercentInstance().format(percent / 100.0)
        }
    }

    fun applyPalette() {
        setBackgroundColor(color(R.color.setup_background))
        textColors.forEach { (view, resource) -> view.setTextColor(color(resource)) }
        card.background = rounded(color(R.color.setup_surface), 22).apply {
            setStroke(dp(1), color(R.color.setup_border))
        }
        progress.progressDrawable = LayerDrawable(arrayOf(
            rounded(color(R.color.setup_track), 4),
            ClipDrawable(rounded(color(R.color.setup_accent), 4), Gravity.START, ClipDrawable.HORIZONTAL),
        )).apply {
            setId(0, android.R.id.background)
            setId(1, android.R.id.progress)
        }
        progress.indeterminateTintList = ColorStateList.valueOf(color(R.color.setup_accent))
        styleButton(primary, filled = true)
        styleButton(secondary, filled = false)
        styleButton(storage, filled = false)
        showStage(currentStage)
    }

    private fun label(text: String, size: Float, tint: Int = R.color.setup_text, emphasized: Boolean = false) =
        TextView(context).apply {
            this.text = text
            textSize = size
            typeface = if (emphasized) medium else regular
            includeFontPadding = false
            setLineSpacing(dp(3).toFloat(), 1f)
            this@NativeSetupView.textColors.add(this to tint)
        }

    private fun button() = MaterialButton(context).apply {
        typeface = medium
        textSize = 14f
        isAllCaps = false
        letterSpacing = 0f
        minHeight = dp(52)
        minimumHeight = dp(52)
        insetTop = 0
        insetBottom = 0
        cornerRadius = dp(12)
        setPadding(dp(16), dp(14), dp(16), dp(14))
        stateListAnimator = null
        elevation = 0f
    }

    private fun styleButton(button: MaterialButton, filled: Boolean) {
        val fill = color(if (filled) R.color.setup_accent else R.color.setup_surface)
        val text = color(if (filled) R.color.setup_on_accent else R.color.setup_text)
        val states = arrayOf(intArrayOf(-android.R.attr.state_enabled), intArrayOf())
        button.backgroundTintList = ColorStateList(states,
            intArrayOf(color(R.color.setup_track), fill))
        button.setTextColor(ColorStateList(states, intArrayOf(color(R.color.setup_muted), text)))
        button.strokeWidth = if (filled) 0 else dp(1)
        button.strokeColor = ColorStateList.valueOf(color(R.color.setup_border))
        button.rippleColor = ColorStateList.valueOf((text and 0x00FFFFFF) or 0x22000000)
    }

    private fun row(top: Int = 0) = LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(top) }
    private fun rounded(tint: Int, radius: Int) = GradientDrawable().apply {
        setColor(tint)
        cornerRadius = dp(radius).toFloat()
    }
    private fun color(resource: Int) = ContextCompat.getColor(context, resource)
    private fun dp(value: Int) = (value * resources.displayMetrics.density + 0.5f).toInt()
}
