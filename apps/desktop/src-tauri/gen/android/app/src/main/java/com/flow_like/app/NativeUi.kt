package com.flow_like.app

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.DialogInterface
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.os.Build
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.TextView
import androidx.core.content.ContextCompat
import androidx.core.content.res.ResourcesCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import com.google.android.material.button.MaterialButton

/** Native entry points share the same fonts and colors as the first-open screen. */
object NativeUi {
    fun dp(context: Context, value: Int) = (value * context.resources.displayMetrics.density + 0.5f).toInt()
    fun color(context: Context, resource: Int) = ContextCompat.getColor(context, resource)
    fun font(context: Context, emphasized: Boolean = false) = ResourcesCompat.getFont(context,
        if (emphasized) R.font.flow_inter_medium else R.font.flow_inter_regular)

    fun text(context: Context, text: CharSequence, size: Float = 16f, muted: Boolean = false,
        emphasized: Boolean = false) = TextView(context).apply {
        this.text = text
        textSize = size
        typeface = font(context, emphasized)
        includeFontPadding = false
        setTextColor(color(context, if (muted) R.color.setup_muted else R.color.setup_text))
        setLineSpacing(dp(context, 3).toFloat(), 1f)
    }

    fun input(context: Context, hint: String, text: String = "", minLines: Int = 1) = EditText(context).apply {
        this.hint = hint
        contentDescription = hint
        setText(text)
        textSize = 16f
        typeface = font(context)
        setTextColor(color(context, R.color.setup_text))
        setHintTextColor(color(context, R.color.setup_muted))
        this.minLines = minLines
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES or
            (if (minLines > 1) InputType.TYPE_TEXT_FLAG_MULTI_LINE else 0)
        gravity = Gravity.TOP or Gravity.START
        minimumHeight = dp(context, 56)
        setPadding(dp(context, 16), dp(context, 16), dp(context, 16), dp(context, 16))
        backgroundTintList = null
        fun outline(focused: Boolean) = rounded(context, R.color.setup_background, 12).apply {
            setStroke(dp(context, if (focused) 2 else 1),
                color(context, if (focused) R.color.setup_accent else R.color.setup_border))
        }
        background = outline(false)
        onFocusChangeListener = View.OnFocusChangeListener { _, focused -> background = outline(focused) }
    }

    fun button(context: Context, text: String, primary: Boolean = false, onClick: () -> Unit = {}) =
        MaterialButton(context).apply {
            this.text = text
            typeface = font(context, true)
            textSize = 15f
            isAllCaps = false
            letterSpacing = 0f
            minHeight = dp(context, 52)
            minimumHeight = dp(context, 52)
            insetTop = 0
            insetBottom = 0
            cornerRadius = dp(context, 12)
            setPadding(dp(context, 16), dp(context, 14), dp(context, 16), dp(context, 14))
            stateListAnimator = null
            elevation = 0f
            styleButton(this, primary)
            setOnClickListener { onClick() }
        }

    fun styleButton(button: MaterialButton, primary: Boolean) {
        val context = button.context
        val states = arrayOf(intArrayOf(-android.R.attr.state_enabled), intArrayOf())
        val foreground = color(context, if (primary) R.color.setup_on_accent else R.color.setup_text)
        button.backgroundTintList = ColorStateList(states, intArrayOf(color(context, R.color.setup_track),
            color(context, if (primary) R.color.setup_accent else R.color.setup_surface)))
        button.setTextColor(ColorStateList(states, intArrayOf(color(context, R.color.setup_muted), foreground)))
        button.strokeWidth = if (primary) 0 else dp(context, 1)
        button.strokeColor = ColorStateList.valueOf(color(context, R.color.setup_border))
        button.rippleColor = ColorStateList.valueOf((foreground and 0x00FFFFFF) or 0x22000000)
    }

    fun rounded(context: Context, tint: Int = R.color.setup_surface, radius: Int = 20) = GradientDrawable().apply {
        setColor(color(context, tint))
        cornerRadius = dp(context, radius).toFloat()
    }

    fun card(context: Context) = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(context, 20), dp(context, 22), dp(context, 20), dp(context, 22))
        background = rounded(context).apply { setStroke(dp(context, 1), color(context, R.color.setup_border)) }
    }

    fun loading(context: Context, title: String, description: String) = card(context).apply {
        val row = LinearLayout(context).apply { gravity = Gravity.CENTER_VERTICAL }
        row.addView(ProgressBar(context).apply {
            indeterminateTintList = ColorStateList.valueOf(color(context, R.color.setup_accent))
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }, LinearLayout.LayoutParams(dp(context, 28), dp(context, 28)))
        row.addView(text(context, title, 16f, emphasized = true),
            LinearLayout.LayoutParams(0, -2, 1f).apply { marginStart = dp(context, 14) })
        addView(row)
        add(this, text(context, description, 14f, muted = true), 14)
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }

    fun brand(context: Context, compact: Boolean = false) = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        val size = if (compact) 36 else 44
        addView(ImageView(context).apply {
            setImageResource(R.drawable.ic_notification)
            scaleType = ImageView.ScaleType.FIT_CENTER
            val pad = dp(context, 4)
            setPadding(pad, pad, pad, pad)
            background = GradientDrawable(GradientDrawable.Orientation.TL_BR,
                intArrayOf(Color.rgb(255, 141, 40), Color.rgb(255, 56, 60))).apply {
                cornerRadius = dp(context, if (compact) 10 else 13).toFloat()
            }
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }, LinearLayout.LayoutParams(dp(context, size), dp(context, size)))
        addView(text(context, "Flow-Like", if (compact) 15f else 19f, emphasized = true),
            LinearLayout.LayoutParams(0, -2, 1f).apply { marginStart = dp(context, 12) })
    }

    fun add(parent: LinearLayout, view: View, top: Int = 12) {
        parent.addView(view, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(parent.context, top) })
    }

    fun page(activity: Activity, title: String, subtitle: String? = null): NativePage {
        val page = NativePage(activity, title, subtitle)
        activity.setContentView(page.root)
        WindowCompat.setDecorFitsSystemWindows(activity.window, false)
        systemBars(activity)
        ViewCompat.setOnApplyWindowInsetsListener(page.root) { view, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime())
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            insets
        }
        return page
    }

    fun systemBars(activity: Activity, configuration: Configuration = activity.resources.configuration) {
        val night = configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES
        WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
            isAppearanceLightStatusBars = !night
            isAppearanceLightNavigationBars = !night
        }
        if (Build.VERSION.SDK_INT < 26) {
            // Android 7 cannot draw dark navigation buttons on a light background.
            @Suppress("DEPRECATION")
            activity.window.navigationBarColor = Color.rgb(17, 19, 24)
        }
    }

    fun dialog(context: Context) = NativeDialogBuilder(context)
}

class NativePage(context: Context, title: String, subtitle: String?) {
    val heading = NativeUi.text(context, title, 28f, emphasized = true)
    val description = NativeUi.text(context, subtitle.orEmpty(), 15f, muted = true).apply {
        visibility = if (subtitle.isNullOrBlank()) View.GONE else View.VISIBLE
    }
    val content = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    val root = ScrollView(context).apply {
        isFillViewport = true
        clipToPadding = true
        setBackgroundColor(NativeUi.color(context, R.color.setup_background))
        val column = object : LinearLayout(context) {
            override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
                val width = minOf(MeasureSpec.getSize(widthMeasureSpec), NativeUi.dp(context, 560))
                super.onMeasure(MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY), heightMeasureSpec)
            }
        }.apply {
            orientation = LinearLayout.VERTICAL
            val pad = NativeUi.dp(context, 24)
            setPadding(pad, pad, pad, NativeUi.dp(context, 32))
            addView(NativeUi.brand(context))
            NativeUi.add(this, heading, 32)
            NativeUi.add(this, description, 12)
            NativeUi.add(this, content, 24)
        }
        addView(FrameLayout(context).apply {
            addView(column, FrameLayout.LayoutParams(-1, -2, Gravity.TOP or Gravity.CENTER_HORIZONTAL))
        }, ViewGroup.LayoutParams(-1, -2))
    }
    init { ViewCompat.setAccessibilityHeading(heading, true) }
}

/** Matches the small Builder API used by native entry points without platform-default content. */
class NativeDialogBuilder(private val context: Context) {
    private var title: CharSequence = ""
    private var message: CharSequence = ""
    private var view: View? = null
    private var items: Array<String>? = null
    private var itemClick: DialogInterface.OnClickListener? = null
    private var onCancel: DialogInterface.OnCancelListener? = null
    private val actions = linkedMapOf<Int, Pair<CharSequence, DialogInterface.OnClickListener?>>()
    fun setTitle(value: CharSequence) = apply { title = value }
    fun setMessage(value: CharSequence) = apply { message = value }
    fun setView(value: View) = apply { view = value }
    fun setItems(values: Array<String>, listener: DialogInterface.OnClickListener) = apply { items = values; itemClick = listener }
    fun setPositiveButton(text: CharSequence, listener: DialogInterface.OnClickListener?) = apply { actions[DialogInterface.BUTTON_POSITIVE] = text to listener }
    fun setNegativeButton(text: CharSequence, listener: DialogInterface.OnClickListener?) = apply { actions[DialogInterface.BUTTON_NEGATIVE] = text to listener }
    fun setNeutralButton(text: CharSequence, listener: DialogInterface.OnClickListener?) = apply { actions[DialogInterface.BUTTON_NEUTRAL] = text to listener }
    fun setOnCancelListener(listener: DialogInterface.OnCancelListener) = apply { onCancel = listener }
    fun create(): AlertDialog {
        val buttons = LinkedHashMap(actions)
        if (items != null && DialogInterface.BUTTON_NEGATIVE !in buttons) {
            buttons[DialogInterface.BUTTON_NEGATIVE] = "Cancel" to DialogInterface.OnClickListener { dialog, _ -> dialog.cancel() }
        }
        return NativeDialog(context, title, message, view, items, itemClick, buttons).apply {
            setOnCancelListener(onCancel)
            setCanceledOnTouchOutside(true)
        }
    }
    fun show(): AlertDialog = create().apply { show() }
}

private class NativeDialog(
    context: Context,
    private val title: CharSequence,
    private val message: CharSequence,
    private val customView: View?,
    private val items: Array<String>?,
    private val itemClick: DialogInterface.OnClickListener?,
    private val actions: Map<Int, Pair<CharSequence, DialogInterface.OnClickListener?>>,
) : AlertDialog(context, R.style.Theme_FlowLike_NativeDialog) {
    private val buttons = mutableMapOf<Int, Button>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val content = NativeUi.card(context)
        content.addView(NativeUi.brand(context, compact = true))
        if (title.isNotEmpty()) NativeUi.add(content,
            NativeUi.text(context, title, 22f, emphasized = true).apply { ViewCompat.setAccessibilityHeading(this, true) }, 24)
        if (message.isNotEmpty()) NativeUi.add(content, NativeUi.text(context, message, 16f, muted = true), 14)
        customView?.let { NativeUi.add(content, it, 20) }
        items?.forEachIndexed { index, label ->
            NativeUi.add(content, NativeUi.button(context, label) {
                itemClick?.onClick(this, index)
                dismiss()
            }.apply { gravity = Gravity.START or Gravity.CENTER_VERTICAL }, if (index == 0) 24 else 10)
        }
        for (which in listOf(DialogInterface.BUTTON_POSITIVE, DialogInterface.BUTTON_NEUTRAL, DialogInterface.BUTTON_NEGATIVE)) {
            val action = actions[which] ?: continue
            val button = NativeUi.button(context, action.first.toString(), primary = which == DialogInterface.BUTTON_POSITIVE) {
                action.second?.onClick(this, which)
                dismiss()
            }
            buttons[which] = button
            NativeUi.add(content, button, if (buttons.size == 1) 24 else 10)
        }
        val scroll = object : ScrollView(context) {
            override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
                val screenHeight = resources.displayMetrics.heightPixels - NativeUi.dp(context, 64)
                val available = if (MeasureSpec.getMode(heightMeasureSpec) == MeasureSpec.UNSPECIFIED) screenHeight
                    else minOf(screenHeight, MeasureSpec.getSize(heightMeasureSpec))
                super.onMeasure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(available, MeasureSpec.AT_MOST))
            }
        }.apply { addView(content); isFillViewport = false; clipToOutline = true; background = NativeUi.rounded(context) }
        setContentView(scroll)
        window?.apply {
            setBackgroundDrawableResource(android.R.color.transparent)
            clearFlags(WindowManager.LayoutParams.FLAG_ALT_FOCUSABLE_IM)
            setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
            setTitle(title.ifEmpty { "Flow-Like" })
        }
    }

    override fun onStart() {
        super.onStart()
        window?.setLayout(minOf(context.resources.displayMetrics.widthPixels - NativeUi.dp(context, 40),
            NativeUi.dp(context, 520)), ViewGroup.LayoutParams.WRAP_CONTENT)
    }

    override fun getButton(whichButton: Int): Button? = buttons[whichButton]
}
