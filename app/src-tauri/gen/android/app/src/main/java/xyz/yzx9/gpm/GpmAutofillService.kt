// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import android.app.PendingIntent
import android.app.assist.AssistStructure.ViewNode
import android.content.Intent
import android.os.CancellationSignal
import android.service.autofill.AutofillService
import android.service.autofill.Dataset
import android.service.autofill.FillCallback
import android.service.autofill.FillRequest
import android.service.autofill.FillResponse
import android.service.autofill.SaveCallback
import android.service.autofill.SaveRequest
import android.view.autofill.AutofillId
import android.view.autofill.AutofillValue
import android.widget.RemoteViews
import androidx.annotation.RequiresApi

/**
 * The gpm autofill service (R056 minimal MVP). Deliberately trivial: a hint
 * scan and ONE auth-required dataset — zero store access, zero precondition
 * checks — so the OS's fill dispatch can never block or crash on us; every
 * precondition surfaces later as an empty state in [FillAuthActivity].
 *
 * The service is inert below API 26 (the framework never binds it there —
 * no manifest guard needed); the app's minSdk is 24.
 */
@RequiresApi(26)
class GpmAutofillService : AutofillService() {
    override fun onFillRequest(
        request: FillRequest,
        cancellationSignal: CancellationSignal,
        callback: FillCallback,
    ) {
        // Every exit path is onSuccess(...) — never onFailure, never a throw:
        // a failure here surfaces as an error toast inside someone else's app.
        runCatching {
            val targets = HintScan.classify(collectFields(request))
            val username = targets.usernameField
            val password = targets.passwordField
            if (username == null && password == null) {
                callback.onSuccess(null)
                return
            }
            val ids = ArrayList<AutofillId>()
            val hints = ArrayList<String>()
            username?.let {
                ids.add(it)
                hints.add(FillContract.HINT_USERNAME)
            }
            password?.let {
                ids.add(it)
                hints.add(FillContract.HINT_PASSWORD)
            }

            // Auth-deferred dataset: null values + an auth IntentSender. The
            // extras are always set — Android 12+ crashes the fill flow on a
            // null-extras auth intent — and the embedded intent needs
            // NEW_TASK (the framework fires the sender from a service
            // context).
            val intent =
                Intent(this, FillAuthActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    .putParcelableArrayListExtra(FillContract.EXTRA_AUTOFILL_IDS, ids)
                    .putStringArrayListExtra(FillContract.EXTRA_AUTOFILL_HINTS, hints)
            val sender =
                PendingIntent
                    .getActivity(
                        this,
                        0,
                        intent,
                        // UPDATE_CURRENT is load-bearing: a PendingIntent
                        // matches on (component, requestCode) — extras are NOT
                        // identity — so without it every fill after the first
                        // reuses the first screen's AutofillId extras.
                        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                    )
                    .intentSender

            val dataset =
                Dataset.Builder()
                    .setAuthentication(sender)
                    .apply {
                        username?.let { setValue(it, null as AutofillValue?, presentation()) }
                        password?.let { setValue(it, null as AutofillValue?, presentation()) }
                    }.build()
            callback.onSuccess(FillResponse.Builder().addDataset(dataset).build())
        }.onFailure { callback.onSuccess(null) }
    }

    /** Never offered (no SaveInfo is ever sent); defensive no-op. */
    override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
        callback.onSuccess()
    }

    /** The dropdown row the OS inflates outside our process. */
    private fun presentation(): RemoteViews =
        RemoteViews(packageName, android.R.layout.simple_list_item_1).apply {
            setTextViewText(android.R.id.text1, getString(R.string.autofill_dataset_label))
        }

    /** Walk the focused screen's view tree, extracting `(id, hints)` pairs. */
    private fun collectFields(request: FillRequest): List<FieldHints<AutofillId>> {
        val fields = mutableListOf<FieldHints<AutofillId>>()
        for (context in request.fillContexts) {
            val structure = context.structure
            for (i in 0 until structure.windowNodeCount) {
                walkNode(structure.getWindowNodeAt(i).rootViewNode, fields)
            }
        }
        return fields
    }

    private fun walkNode(node: ViewNode?, fields: MutableList<FieldHints<AutofillId>>) {
        node ?: return
        node.autofillId?.let { id ->
            fields.add(FieldHints(id, node.autofillHints?.toList() ?: emptyList()))
        }
        for (i in 0 until node.childCount) {
            walkNode(node.getChildAt(i), fields)
        }
    }
}
