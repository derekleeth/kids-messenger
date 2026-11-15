package com.kitchat.app

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.webkit.MimeTypeMap
import android.webkit.WebView
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import com.google.android.gms.common.moduleinstall.ModuleInstall
import com.google.android.gms.common.moduleinstall.ModuleInstallRequest
import com.google.mlkit.common.MlKitException
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import java.io.File
import java.util.UUID

private const val NOTIFICATIONS = "notifications"
private const val LOCAL_NETWORK = "localNetwork"
private const val MESSAGES_CHANNEL = "messages"
private const val TAG = "KitPlugin"
private const val MAX_SHARED_BYTES = 30L * 1024 * 1024

@InvokeArg
class NotifyArgs {
  var title: String = ""
  var body: String = ""
}

@InvokeArg
class KeepAliveArgs {
  var on: Boolean = false
}

/**
 * Everything Android-specific KitChat needs, used from Rust (src-tauri/src/platform.rs):
 * - `notify`: a new-message notification.
 * - `keepAlive`: runs [HubService] while this device is in a family, so messages keep arriving in the background.
 *   Also asks once for the notification and (Android 17+) local network permissions.
 * - `takeShared`: pictures and text shared in from other apps. Pictures are copied to `cache/shared/` (the Rust side
 *   reads them from `app_cache_dir()/shared` and deletes them).
 * - `localNetwork`: Android 17's permission to reach the home network, asked before joining a hub there.
 * - `scan`: a join QR code, through Google's code scanner (no camera permission needed).
 */
@TauriPlugin(
  permissions = [
    Permission(strings = ["android.permission.POST_NOTIFICATIONS"], alias = NOTIFICATIONS),
    Permission(strings = ["android.permission.ACCESS_LOCAL_NETWORK"], alias = LOCAL_NETWORK),
  ],
)
class KitPlugin(private val activity: Activity) : Plugin(activity) {
  companion object {
    @Volatile
    var instance: KitPlugin? = null
      private set

    private val pendingUris = mutableListOf<Pair<Uri, String>>()
    private val pendingTexts = mutableListOf<String>()
    private var nextNotification = 100

    /** Keeps what a share intent carries until the app asks for it. Says whether there was anything. */
    fun received(intent: Intent?): Boolean {
      if (intent == null) return false
      val action = intent.action
      if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return false
      val type = intent.type ?: ""
      var got = false
      synchronized(pendingUris) {
        if (action == Intent.ACTION_SEND) {
          @Suppress("DEPRECATION")
          (intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))?.let {
            pendingUris.add(it to type)
            got = true
          }
        } else {
          @Suppress("DEPRECATION")
          intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.forEach {
            pendingUris.add(it to type)
            got = true
          }
        }
        val text = intent.getStringExtra(Intent.EXTRA_TEXT)?.trim()
        if (!text.isNullOrEmpty()) {
          pendingTexts.add(text)
          got = true
        }
      }
      return got
    }
  }

  private var askedForPermissions = false
  private var page: WebView? = null

  override fun load(webView: WebView) {
    super.load(webView)
    page = webView
    instance = this
    createChannel()
  }

  /** Tells the page something new was shared while it was already open (src/components/ShareSheet.tsx). */
  fun announceShared() {
    Handler(Looper.getMainLooper()).post {
      page?.evaluateJavascript("window.__kitSharedIn && window.__kitSharedIn()", null)
    }
  }

  private fun createChannel() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val channel = NotificationChannel(MESSAGES_CHANNEL, "Messages", NotificationManager.IMPORTANCE_HIGH)
    channel.description = "New messages from your family"
    activity.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
  }

  @Command
  fun notify(invoke: Invoke) {
    val args = invoke.parseArgs(NotifyArgs::class.java)
    val open = PendingIntent.getActivity(
      activity,
      0,
      Intent(activity, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
    val notification = NotificationCompat.Builder(activity, MESSAGES_CHANNEL)
      .setSmallIcon(R.drawable.ic_stat_kit)
      .setContentTitle(args.title)
      .setContentText(args.body)
      .setStyle(NotificationCompat.BigTextStyle().bigText(args.body))
      .setAutoCancel(true)
      .setPriority(NotificationCompat.PRIORITY_HIGH)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setContentIntent(open)
      .build()
    try {
      NotificationManagerCompat.from(activity).notify(nextNotification++, notification)
    } catch (e: SecurityException) {
      // Notifications not allowed; nothing to show.
    }
    invoke.resolve()
  }

  @Command
  fun keepAlive(invoke: Invoke) {
    val on = invoke.parseArgs(KeepAliveArgs::class.java).on
    val intent = Intent(activity, HubService::class.java)
    try {
      if (on) ContextCompat.startForegroundService(activity, intent) else activity.stopService(intent)
    } catch (e: Exception) {
      // Android only lets a foreground service start while the app is in front; it starts the next time it is.
      Log.w(TAG, "background service: ${e.message}")
    }
    val missing = mutableListOf<String>()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && getPermissionState(NOTIFICATIONS) != PermissionState.GRANTED) {
      missing.add(NOTIFICATIONS)
    }
    if (Build.VERSION.SDK_INT >= 37 && getPermissionState(LOCAL_NETWORK) != PermissionState.GRANTED) {
      missing.add(LOCAL_NETWORK)
    }
    if (on && !askedForPermissions && missing.isNotEmpty()) {
      askedForPermissions = true
      requestPermissionForAliases(missing.toTypedArray(), invoke, "permissionsAnswered")
      return
    }
    invoke.resolve()
  }

  @PermissionCallback
  private fun permissionsAnswered(invoke: Invoke) {
    // The service's notification was posted before the answer; post it again now that it may be allowed.
    if (HubService.running) {
      try {
        ContextCompat.startForegroundService(activity, Intent(activity, HubService::class.java))
      } catch (e: Exception) {
        Log.w(TAG, "background service: ${e.message}")
      }
    }
    invoke.resolve()
  }

  @Command
  fun takeShared(invoke: Invoke) {
    val uris: List<Pair<Uri, String>>
    val texts: List<String>
    synchronized(pendingUris) {
      uris = pendingUris.toList()
      texts = pendingTexts.toList()
      pendingUris.clear()
      pendingTexts.clear()
    }
    // Copying a few pictures can take a moment; not on the main thread.
    Thread {
      val items = JSArray()
      val dir = File(activity.cacheDir, "shared").apply { mkdirs() }
      for ((uri, intentType) in uris) {
        try {
          val mime = activity.contentResolver.getType(uri) ?: intentType
          if (!mime.startsWith("image/")) continue
          val ext = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime) ?: "img"
          val file = File(dir, "${UUID.randomUUID()}.$ext")
          var copied = 0L
          activity.contentResolver.openInputStream(uri)?.use { input ->
            file.outputStream().use { output ->
              val buf = ByteArray(64 * 1024)
              while (true) {
                val n = input.read(buf)
                if (n < 0) break
                copied += n
                if (copied > MAX_SHARED_BYTES) throw IllegalStateException("picture too big")
                output.write(buf, 0, n)
              }
            }
          }
          val item = JSObject()
          item.put("kind", "image")
          item.put("path", file.absolutePath)
          item.put("mime", mime)
          item.put("name", uri.lastPathSegment ?: "")
          items.put(item)
        } catch (e: Exception) {
          Log.w(TAG, "shared picture: ${e.message}")
        }
      }
      for (text in texts) {
        val item = JSObject()
        item.put("kind", "text")
        item.put("text", text)
        items.put(item)
      }
      val result = JSObject()
      result.put("items", items)
      invoke.resolve(result)
    }.start()
  }

  /** `{ granted }`. Android 17 (API 37) blocks connections to 192.168.x.x and the like without it. */
  @Command
  fun localNetwork(invoke: Invoke) {
    if (Build.VERSION.SDK_INT < 37 || getPermissionState(LOCAL_NETWORK) == PermissionState.GRANTED) {
      invoke.resolve(JSObject().put("granted", true))
    } else {
      requestPermissionForAlias(LOCAL_NETWORK, invoke, "localNetworkAnswered")
    }
  }

  @PermissionCallback
  private fun localNetworkAnswered(invoke: Invoke) {
    invoke.resolve(JSObject().put("granted", getPermissionState(LOCAL_NETWORK) == PermissionState.GRANTED))
  }

  @Command
  fun scan(invoke: Invoke) {
    val options = GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build()
    val scanner = try {
      GmsBarcodeScanning.getClient(activity, options)
    } catch (e: Exception) {
      Log.w(TAG, "scanner client", e)
      invoke.reject("The QR scanner isn't available on this device (${e.javaClass.simpleName}). Type the code instead.")
      return
    }
    // Google Play services downloads the scanner the first time an app needs it. Ask for it, then scan; if the
    // request itself fails, try scanning anyway and let that report what's wrong.
    val request = ModuleInstallRequest.newBuilder().addApi(scanner).build()
    ModuleInstall.getClient(activity).installModules(request)
      .addOnSuccessListener { startScan(scanner, invoke) }
      .addOnFailureListener { e ->
        Log.w(TAG, "scanner module install", e)
        startScan(scanner, invoke)
      }
  }

  private fun startScan(scanner: com.google.mlkit.vision.codescanner.GmsBarcodeScanner, invoke: Invoke) {
    val canceled = {
      val result = JSObject()
      result.put("canceled", true)
      invoke.resolve(result)
    }
    scanner.startScan()
      .addOnSuccessListener { barcode ->
        val result = JSObject()
        result.put("text", barcode.rawValue ?: "")
        invoke.resolve(result)
      }
      .addOnCanceledListener { canceled() }
      .addOnFailureListener { e ->
        Log.w(TAG, "scan failed", e)
        // Back from the scanner fails it with ML Kit's INTERNAL error rather than cancelling (seen in TerminalLynx).
        val backedOut = (e is MlKitException &&
          (e.errorCode == MlKitException.CODE_SCANNER_CANCELLED || e.errorCode == MlKitException.INTERNAL)) ||
          (e is ApiException && e.statusCode == CommonStatusCodes.CANCELED)
        val code = (e as? MlKitException)?.errorCode ?: (e as? ApiException)?.statusCode
        when {
          backedOut -> canceled()
          e is MlKitException && e.errorCode == MlKitException.CODE_SCANNER_UNAVAILABLE ->
            invoke.reject("The QR scanner is still being set up by Google Play services. Try again in a minute, or type the code.")
          else -> invoke.reject("Couldn't open the QR scanner (error ${code ?: e.javaClass.simpleName}). Type the code instead.")
        }
      }
  }
}
