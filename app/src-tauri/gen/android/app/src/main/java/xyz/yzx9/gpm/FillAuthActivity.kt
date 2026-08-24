// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import android.content.Intent
import android.os.Bundle
import android.service.autofill.Dataset
import android.text.Editable
import android.text.TextWatcher
import android.view.Gravity
import android.view.WindowManager
import android.view.autofill.AutofillId
import android.view.autofill.AutofillManager
import android.view.autofill.AutofillValue
import android.widget.ArrayAdapter
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.RemoteViews
import android.widget.TextView
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.biometric.BiometricManager
import androidx.core.content.IntentCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The native fill surface (R056): a STRONG biometric gate before the list
 * (once per process — [VaultKeyCache]; entry names are store metadata), then
 * the full unsorted entry list + substring filter, decrypt via
 * [AutofillBridge], and the dataset auth-result round trip back into the
 * target app's focused fields. Never shows a password, never touches the
 * clipboard; FLAG_SECURE + excludeFromRecents mirror the app's screenshot
 * posture.
 */
@RequiresApi(26)
class FillAuthActivity : AppCompatActivity() {
    private var targets: FillTargets<AutofillId>? = null
    private var masterKeyB64: String? = null
    private lateinit var adapter: ArrayAdapter<FillEntry>

    override fun onCreate(savedInstanceState: Bundle?) {
        // Raised before any content renders — decrypted values transit this
        // activity only as the final dataset, but keep the same posture as
        // MainActivity.
        window.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )
        super.onCreate(savedInstanceState)

        val ids =
            IntentCompat.getParcelableArrayListExtra(
                intent,
                FillContract.EXTRA_AUTOFILL_IDS,
                AutofillId::class.java,
            )
        val hints = intent.getStringArrayListExtra(FillContract.EXTRA_AUTOFILL_HINTS)
        if (ids.isNullOrEmpty() || hints == null || ids.size != hints.size) {
            finishCancelled()
            return
        }
        targets = HintScan.classify(ids.zip(hints) { id, hint -> FieldHints(id, listOf(hint)) })
        buildUi()
        // The entry list is store metadata: with App Lock on it sits behind
        // the same biometric wall as the in-app list. The gate is cheap after
        // the first fill — a cache hit never re-prompts.
        if (VaultBootstrap.isVaultSealed(this)) {
            resolveVaultKey { loadList() }
        } else {
            loadList()
        }
    }

    // ------------------------------------------------------------- list ----

    private fun loadList() {
        val key = HeadlessBootstrap.loadAuthFreeMasterKey(this)
        if (key == null) {
            showEmptyState(R.string.fill_empty_not_set_up)
            return
        }
        masterKeyB64 = key
        lifecycleScope.launch(Dispatchers.IO) {
            // A JNI-level failure resolves to null and a malformed body
            // throws — either way degrade to the error state, never crash.
            val outcome =
                try {
                    AutofillJson.parseList(AutofillBridge.nativeListEntries(dataDir(), key))
                } catch (e: Throwable) {
                    FillListOutcome.Error("bridge failure")
                }
            withContext(Dispatchers.Main) {
                when (outcome) {
                    is FillListOutcome.Ok -> showList(outcome.entries)
                    is FillListOutcome.Skipped ->
                        when (outcome.reason) {
                            "no_repositories" -> showEmptyState(R.string.fill_empty_no_repo)
                            "not_ready" -> showEmptyState(R.string.fill_empty_open_app)
                            "mid_migrate" -> showEmptyState(R.string.fill_empty_mid_migrate)
                            else -> showEmptyState(R.string.fill_empty_open_app)
                        }
                    is FillListOutcome.Error -> showEmptyState(R.string.fill_error_generic)
                }
            }
        }
    }

    private fun showList(entries: List<FillEntry>) {
        if (entries.isEmpty()) {
            showEmptyState(R.string.fill_empty_no_entries)
            return
        }
        adapter.clear()
        adapter.addAll(entries)
    }

    private fun onEntryPicked(repoId: String, name: String) {
        resolveVaultKey { vaultKeyB64 -> decryptAndFill(repoId, name, vaultKeyB64) }
    }

    // ---------------------------------------------------- vault + decrypt ----

    /** Cache hit → no prompt; App Lock off → no key needed; else prompt. */
    private fun resolveVaultKey(then: (String) -> Unit) {
        VaultKeyCache.get()?.let { cached ->
            then(cached)
            return
        }
        if (!VaultBootstrap.isVaultSealed(this)) {
            // App Lock off: the identity is master-sealed and decrypts with
            // the auth-free key alone (the Rust probe never injects a vault
            // key in this mode).
            then("")
            return
        }
        val canAuth =
            BiometricManager
                .from(this)
                .canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)
        if (!VaultBootstrap.strongBiometricAvailable(canAuth)) {
            showEmptyState(R.string.fill_empty_no_biometrics)
            return
        }
        VaultBootstrap.unsealVaultKey(
            this,
            onUnsealed = { keyB64 ->
                VaultKeyCache.set(keyB64)
                then(keyB64)
            },
            onError = { _, _ -> finishCancelled() },
        )
    }

    private fun decryptAndFill(repoId: String, name: String, vaultKeyB64: String) {
        val key = masterKeyB64 ?: run {
            finishCancelled()
            return
        }
        lifecycleScope.launch(Dispatchers.IO) {
            val outcome =
                try {
                    AutofillJson.parseDecrypt(
                        AutofillBridge.nativeDecryptEntry(dataDir(), key, vaultKeyB64, repoId, name),
                    )
                } catch (e: Throwable) {
                    FillDecryptOutcome.Error("bridge failure")
                }
            withContext(Dispatchers.Main) {
                when (outcome) {
                    is FillDecryptOutcome.Ok ->
                        finishWithDataset(outcome.password, outcome.username)
                    is FillDecryptOutcome.Skipped ->
                        when (outcome.reason) {
                            "passphrase_locked" -> showEmptyState(R.string.fill_empty_passphrase)
                            "app_locked" -> showEmptyState(R.string.fill_empty_app_locked)
                            else -> showEmptyState(R.string.fill_empty_open_app)
                        }
                    is FillDecryptOutcome.Error -> showEmptyState(R.string.fill_error_generic)
                }
            }
        }
    }

    // ---------------------------------------------------------- result ----

    private fun finishWithDataset(password: String, username: String) {
        val targets = targets ?: run {
            finishCancelled()
            return
        }
        val values = FillContract.mapFillValues(targets, username, password)
        if (values.isEmpty()) {
            finishCancelled()
            return
        }
        val presentation =
            RemoteViews(packageName, android.R.layout.simple_list_item_1).apply {
                setTextViewText(android.R.id.text1, getString(R.string.autofill_dataset_label))
            }
        val builder = Dataset.Builder()
        for ((id, value) in values) {
            builder.setValue(id, AutofillValue.forText(value), presentation)
        }
        setResult(
            RESULT_OK,
            Intent().putExtra(AutofillManager.EXTRA_AUTHENTICATION_RESULT, builder.build()),
        )
        finish()
    }

    /** Any abort path: cancel → close, no retry loop (the user re-taps). */
    private fun finishCancelled() {
        setResult(RESULT_CANCELED)
        finish()
    }

    // --------------------------------------------------------------- UI ----

    private fun buildUi() {
        val pad = (16 * resources.displayMetrics.density).toInt()
        val root =
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(pad, pad, pad, pad)
            }
        adapter = ArrayAdapter(this, android.R.layout.simple_list_item_1, mutableListOf())
        val list =
            ListView(this).apply {
                adapter = this@FillAuthActivity.adapter
                setOnItemClickListener { _, _, position, _ ->
                    this@FillAuthActivity.adapter.getItem(position)?.let {
                        onEntryPicked(it.repoId, it.name)
                    }
                }
            }
        val filter =
            EditText(this).apply {
                hint = getString(R.string.fill_filter_hint)
                setSingleLine()
                addTextChangedListener(
                    object : TextWatcher {
                        override fun afterTextChanged(s: Editable?) {
                            // ArrayAdapter's built-in filter: case-insensitive
                            // substring over toString() (= the entry name).
                            this@FillAuthActivity.adapter.filter.filter(s?.toString() ?: "")
                        }

                        override fun beforeTextChanged(
                            s: CharSequence?,
                            start: Int,
                            count: Int,
                            after: Int,
                        ) {}

                        override fun onTextChanged(
                            s: CharSequence?,
                            start: Int,
                            before: Int,
                            count: Int,
                        ) {}
                    },
                )
            }
        root.addView(filter)
        root.addView(
            list,
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f),
        )
        setContentView(root)
    }

    private fun showEmptyState(resId: Int) {
        val pad = (16 * resources.displayMetrics.density).toInt()
        setContentView(
            TextView(this).apply {
                setText(resId)
                setPadding(pad, pad, pad, pad)
                gravity = Gravity.CENTER
            },
        )
    }

    /**
     * The Rust config dir on Android: Tauri's `app_config_dir()` resolves to
     * the plain data dir with no bundle suffix (its `PathPlugin.getConfigDir`
     * returns `activity.dataDir`) — the same string
     * `reschedule_background_sync` passes the worker. Re-check on a Tauri
     * upgrade.
     */
    private fun dataDir(): String = applicationContext.dataDir.absolutePath
}
