package xyz.yzx9.gpm

import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
    private var statusBarInset = 0f // CSS pixels (dp)
    private var navBarInset = 0f // CSS pixels (dp)

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)

        // Block screenshots and screen recording
        window.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )

        // TODO: Extract safe-area insets into a proper Tauri plugin (Rust ↔ Kotlin
        // via register_android_plugin) instead of addJavascriptInterface, so it
        // integrates with Tauri's capability/permission system.
        // Expose insets to WebView via JS interface
        window.decorView.post {
            findWebView(window.decorView)?.addJavascriptInterface(
                this@MainActivity,
                "GpmInsets",
            )
        }

        // Read insets whenever they change (layout, rotation, keyboard)
        val density = resources.displayMetrics.density
        ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { _, insets ->
            statusBarInset =
                insets.getInsets(WindowInsetsCompat.Type.statusBars()).top / density
            navBarInset =
                insets.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom / density
            insets
        }
    }

    /** JS-accessible: returns status bar height in CSS pixels. */
    @JavascriptInterface
    fun getTop(): Float = statusBarInset

    /** JS-accessible: returns navigation bar height in CSS pixels. */
    @JavascriptInterface
    fun getBottom(): Float = navBarInset

    /** Recursively find the WebView in the view tree. */
    private fun findWebView(root: View): WebView? {
        if (root is WebView) return root
        if (root is ViewGroup) {
            for (i in 0 until root.childCount) {
                findWebView(root.getChildAt(i))?.let { return it }
            }
        }
        return null
    }
}
