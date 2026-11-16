import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PrepareAndroidWebDialogsTest {
    @Test
    fun preservesWebViewCallbacksWhileReplacingDialogFactories() {
        val source = """
            val alert = AlertDialog.Builder(view.context)
            alert.setPositiveButton("OK") { _, _ -> result.confirm() }
            val confirm = AlertDialog.Builder(view.context)
            confirm.setOnCancelListener { result.cancel() }
            val prompt = AlertDialog.Builder(view.context)
            val input = EditText(view.context)
            prompt.setView(input).setPositiveButton("OK") { _, _ -> result.confirm(input.text.toString()) }
            override fun onPermissionRequest(request: PermissionRequest) { request.deny() }
        """.trimIndent()
        val prepared = brandedAndroidWebDialogs(source)
        assertEquals(3, prepared.split("NativeUi.dialog(view.context)").size - 1)
        assertTrue(prepared.contains("NativeUi.input(view.context, \"Response\", defaultValue).apply { inputType = android.text.InputType.TYPE_CLASS_TEXT }"))
        assertEquals(source, prepared.replace("NativeUi.dialog(view.context)", "AlertDialog.Builder(view.context)")
            .replace("NativeUi.input(view.context, \"Response\", defaultValue).apply { inputType = android.text.InputType.TYPE_CLASS_TEXT }", "EditText(view.context)"))
    }

    @Test(expected = IllegalArgumentException::class)
    fun refusesChangedUpstreamDialogCode() {
        brandedAndroidWebDialogs("class RustWebChromeClient : WebChromeClient()")
    }
}
