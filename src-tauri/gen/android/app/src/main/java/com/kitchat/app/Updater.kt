package com.kitchat.app

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.PackageInfo
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.os.Build
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit
import org.json.JSONObject

/**
 * Keeps KitChat up to date from the repository's GitHub Releases (docs/releasing.md), since it isn't in an app store.
 *
 * About twice a day it asks GitHub for the latest release. If that's newer than this app, it downloads
 * `KitChat_<version>_android-universal.apk` (on Wi-Fi or another unmetered network only), checks it against
 * `SHA256SUMS.txt`, and checks that it's KitChat, newer, and signed with this app's key. It installs it once KitChat
 * isn't on screen, so the update never closes the app in the middle of a message.
 *
 * On Android 12+ the update installs without asking. Otherwise (Android 9–11, or the first time, while "Install
 * unknown apps" isn't allowed for KitChat) Android needs a tap: a notification opens its install screen. Updating ends
 * KitChat's process, and with it the hub connection, so a notification then asks to open KitChat again ([onReplaced]).
 *
 * Started by [MainActivity] and [HubService]; not in debug builds.
 */
object Updater {
  private const val TAG = "KitUpdater"
  private const val REPO = "derekleeth/kids-messenger"
  private const val APK_SUFFIX = "_android-universal.apk"
  private const val CHANNEL_ID = "updates"
  private const val PROMPT_ID = 4202
  private const val UPDATED_ID = 4203
  private const val CHECK_EVERY_MS = 12 * 60 * 60 * 1000L
  private const val PROMPT_EVERY_MS = 24 * 60 * 60 * 1000L
  const val ACTION_STATUS = "com.kitchat.app.UPDATE_STATUS"

  // Preferences
  private const val LAST_CHECK = "lastCheck"
  private const val READY_VERSION = "readyVersionCode"
  private const val READY_NAME = "readyVersionName"
  private const val SKIP_TAG = "skipTag"
  private const val PROMPTED_AT = "promptedAt"
  private const val INSTALLING = "installing"

  @Volatile
  private var visible = false
  private var executor: ScheduledExecutorService? = null

  @Synchronized
  fun start(context: Context) {
    if (BuildConfig.DEBUG || executor != null) return
    val app = context.applicationContext
    createChannel(app)
    executor = Executors.newSingleThreadScheduledExecutor().also {
      it.scheduleWithFixedDelay({ guarded { tick(app) } }, 1, 60, TimeUnit.MINUTES)
    }
  }

  /** KitChat is on screen ([MainActivity] `onStart`). */
  fun appShown(context: Context) {
    visible = true
    NotificationManagerCompat.from(context).cancel(UPDATED_ID)
  }

  /** KitChat left the screen. A waiting update installs a minute later if it's still not back. */
  fun appHidden(context: Context) {
    visible = false
    val app = context.applicationContext
    executor?.schedule({ guarded { if (!visible) installReady(app) } }, 1, TimeUnit.MINUTES)
  }

  // An exception would cancel every later run of a scheduled task.
  private fun guarded(work: () -> Unit) {
    try {
      work()
    } catch (e: Throwable) {
      Log.w(TAG, "update: $e")
    }
  }

  private fun prefs(context: Context): SharedPreferences = context.getSharedPreferences("updater", Context.MODE_PRIVATE)

  private fun updateDir(context: Context) = File(context.filesDir, "update")

  private fun readyApk(context: Context) = File(updateDir(context), "KitChat.apk")

  private fun tick(context: Context) {
    val prefs = prefs(context)
    if (System.currentTimeMillis() - prefs.getLong(LAST_CHECK, 0) >= CHECK_EVERY_MS) {
      check(context)
    }
    if (!visible) installReady(context)
  }

  private fun currentVersionCode(context: Context): Long =
    context.packageManager.getPackageInfo(context.packageName, 0).longVersionCode

  /** Looks for a newer release and downloads it, unless one is already downloaded. */
  private fun check(context: Context) {
    val prefs = prefs(context)
    val release = JSONObject(fetch("https://api.github.com/repos/$REPO/releases/latest").toString(Charsets.UTF_8))
    val tag = release.getString("tag_name")
    if (release.optBoolean("draft") || release.optBoolean("prerelease") || !newer(tag, BuildConfig.VERSION_NAME)) {
      clearReady(context)
      prefs.edit().putLong(LAST_CHECK, System.currentTimeMillis()).apply()
      return
    }
    if (tag == prefs.getString(SKIP_TAG, null) ||
      (prefs.getString(READY_NAME, null) == tag && readyApk(context).exists())
    ) {
      prefs.edit().putLong(LAST_CHECK, System.currentTimeMillis()).apply()
      return
    }
    // A universal APK is large; leave it for Wi-Fi. LAST_CHECK stays unset, so it's tried again within the hour.
    val connectivity = context.getSystemService(ConnectivityManager::class.java)
    if (connectivity.isActiveNetworkMetered) return

    val assets = release.getJSONArray("assets")
    var apkUrl: String? = null
    var apkName: String? = null
    var sumsUrl: String? = null
    for (i in 0 until assets.length()) {
      val asset = assets.getJSONObject(i)
      val name = asset.getString("name")
      // A "-TEST-KEY" APK doesn't end in the suffix, so it's never picked.
      if (name.endsWith(APK_SUFFIX)) {
        apkName = name
        apkUrl = asset.getString("browser_download_url")
      } else if (name == "SHA256SUMS.txt") {
        sumsUrl = asset.getString("browser_download_url")
      }
    }
    if (apkUrl == null || apkName == null || sumsUrl == null) {
      Log.i(TAG, "$tag has no APK or checksums")
      prefs.edit().putLong(LAST_CHECK, System.currentTimeMillis()).apply()
      return
    }
    val expected = fetch(sumsUrl).toString(Charsets.UTF_8).lineSequence()
      .map { it.trim().split(Regex("\\s+\\*?"), limit = 2) }
      .firstOrNull { it.size == 2 && it[1] == apkName }
      ?.get(0)?.lowercase()
      ?: throw IllegalStateException("$apkName isn't in SHA256SUMS.txt")

    val dir = updateDir(context)
    dir.mkdirs()
    // Ends in .apk: Android reads the downloaded file's package details before it's kept.
    val part = File(dir, "download.apk")
    val actual = download(apkUrl, part)
    if (actual != expected) {
      part.delete()
      prefs.edit().putLong(LAST_CHECK, System.currentTimeMillis()).apply()
      throw IllegalStateException("$apkName doesn't match its checksum")
    }

    val code = acceptable(context, part)
    if (code == null) {
      part.delete()
      // Not one this app can update to (this app is a build with a different key, say): don't download it again.
      Log.i(TAG, "$tag can't update this app")
      prefs.edit().putLong(LAST_CHECK, System.currentTimeMillis()).putString(SKIP_TAG, tag).apply()
      return
    }
    val apk = readyApk(context)
    apk.delete()
    if (!part.renameTo(apk)) throw IllegalStateException("couldn't keep the download")
    prefs.edit()
      .putLong(LAST_CHECK, System.currentTimeMillis())
      .putString(READY_NAME, tag)
      .putLong(READY_VERSION, code)
      .remove(PROMPTED_AT)
      .apply()
    Log.i(TAG, "downloaded $tag")
  }

  /** The APK's version code if it's KitChat, newer than this app and signed with its key; otherwise null. */
  private fun acceptable(context: Context, apk: File): Long? {
    val pm = context.packageManager
    @Suppress("DEPRECATION")
    val archive = pm.getPackageArchiveInfo(apk.path, PackageManager.GET_SIGNING_CERTIFICATES) ?: return null
    if (archive.packageName != context.packageName) return null
    if (archive.longVersionCode <= currentVersionCode(context)) return null
    @Suppress("DEPRECATION")
    val ours = pm.getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
    if (signers(archive).intersect(signers(ours)).isEmpty()) return null
    return archive.longVersionCode
  }

  private fun signers(info: PackageInfo): Set<String> {
    val signing = info.signingInfo ?: return emptySet()
    val certs = if (signing.hasMultipleSigners()) signing.apkContentsSigners else signing.signingCertificateHistory
    return certs.orEmpty().map { hex(MessageDigest.getInstance("SHA-256").digest(it.toByteArray())) }.toSet()
  }

  private fun clearReady(context: Context) {
    updateDir(context).deleteRecursively()
    prefs(context).edit().remove(READY_NAME).remove(READY_VERSION).remove(PROMPTED_AT).apply()
  }

  /** Installs a downloaded update, if there is one and it's still newer than this app. */
  private fun installReady(context: Context) {
    val prefs = prefs(context)
    val apk = readyApk(context)
    val code = prefs.getLong(READY_VERSION, -1)
    if (!apk.exists() || code <= currentVersionCode(context)) return
    val tag = prefs.getString(READY_NAME, null) ?: return
    if (tag == prefs.getString(SKIP_TAG, null)) return
    // Waiting for a tap on the notification: don't post it again every time the app is left.
    if (System.currentTimeMillis() - prefs.getLong(PROMPTED_AT, 0) < PROMPT_EVERY_MS) return

    val installer = context.packageManager.packageInstaller
    installer.mySessions.forEach { runCatching { installer.abandonSession(it.sessionId) } }
    val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
    params.setAppPackageName(context.packageName)
    params.setSize(apk.length())
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
    }
    val id = installer.createSession(params)
    installer.openSession(id).use { session ->
      apk.inputStream().use { input ->
        session.openWrite("KitChat.apk", 0, apk.length()).use { out ->
          input.copyTo(out)
          session.fsync(out)
        }
      }
      prefs.edit().putBoolean(INSTALLING, true).commit()
      val status = Intent(context, UpdateReceiver::class.java).setAction(ACTION_STATUS)
      // Mutable: the installer adds the result to it.
      val flags = PendingIntent.FLAG_UPDATE_CURRENT or
        (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0)
      session.commit(PendingIntent.getBroadcast(context, 0, status, flags).intentSender)
    }
    Log.i(TAG, "installing $tag")
  }

  /** The installer's answer to [installReady]. */
  fun onStatus(context: Context, intent: Intent) {
    val prefs = prefs(context)
    val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
    when (status) {
      PackageInstaller.STATUS_PENDING_USER_ACTION -> {
        @Suppress("DEPRECATION")
        val confirm = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT) ?: return
        prefs.edit().putLong(PROMPTED_AT, System.currentTimeMillis()).apply()
        val tap = PendingIntent.getActivity(
          context,
          1,
          confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
          PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        notify(context, PROMPT_ID, "A KitChat update is ready", "Tap to install it.", tap)
      }
      PackageInstaller.STATUS_SUCCESS -> Unit // Android ends this process; onReplaced runs in the new version.
      else -> {
        prefs.edit().putBoolean(INSTALLING, false).apply()
        Log.w(TAG, "install failed ($status): ${intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)}")
        // These won't go better next time; anything else (no space, declined) is tried again later.
        if (status == PackageInstaller.STATUS_FAILURE_CONFLICT ||
          status == PackageInstaller.STATUS_FAILURE_INCOMPATIBLE ||
          status == PackageInstaller.STATUS_FAILURE_INVALID
        ) {
          prefs.edit().putString(SKIP_TAG, prefs.getString(READY_NAME, null)).apply()
          updateDir(context).deleteRecursively()
        }
      }
    }
  }

  /** Runs in the new version just after an update. */
  fun onReplaced(context: Context) {
    val prefs = prefs(context)
    val ours = prefs.getBoolean(INSTALLING, false)
    clearReady(context)
    prefs.edit().putBoolean(INSTALLING, false).apply()
    NotificationManagerCompat.from(context).cancel(PROMPT_ID)
    if (!ours) return
    // The update ended the hub connection; it comes back when KitChat is opened.
    createChannel(context)
    val open = PendingIntent.getActivity(
      context,
      0,
      Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
    notify(
      context,
      UPDATED_ID,
      "KitChat was updated to ${BuildConfig.VERSION_NAME}",
      "Tap to open it so new messages keep coming.",
      open,
    )
  }

  private fun notify(context: Context, id: Int, title: String, text: String, tap: PendingIntent) {
    val notification = NotificationCompat.Builder(context, CHANNEL_ID)
      .setSmallIcon(R.drawable.ic_stat_kit)
      .setContentTitle(title)
      .setContentText(text)
      .setAutoCancel(true)
      .setOnlyAlertOnce(true)
      .setCategory(NotificationCompat.CATEGORY_STATUS)
      .setContentIntent(tap)
      .build()
    try {
      NotificationManagerCompat.from(context).notify(id, notification)
    } catch (e: SecurityException) {
      // Notifications not allowed.
    }
  }

  private fun createChannel(context: Context) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val channel = NotificationChannel(CHANNEL_ID, "App updates", NotificationManager.IMPORTANCE_DEFAULT)
    channel.description = "When a KitChat update needs a tap"
    context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
  }

  /** Whether release tag [tag] ("v0.5.0") is a later version than [current] ("0.4.0"). */
  internal fun newer(tag: String, current: String): Boolean {
    fun parts(v: String) = v.removePrefix("v").substringBefore('-').split('.').map { it.toIntOrNull() ?: 0 }
    val a = parts(tag)
    val b = parts(current)
    for (i in 0 until maxOf(a.size, b.size)) {
      val x = a.getOrElse(i) { 0 }
      val y = b.getOrElse(i) { 0 }
      if (x != y) return x > y
    }
    return false
  }

  private fun open(url: String): HttpURLConnection {
    val connection = URL(url).openConnection() as HttpURLConnection
    connection.connectTimeout = 30_000
    connection.readTimeout = 60_000
    connection.setRequestProperty("User-Agent", "KitChat-Android/${BuildConfig.VERSION_NAME}")
    connection.setRequestProperty("Accept", "application/vnd.github+json, application/octet-stream")
    if (connection.responseCode !in 200..299) {
      connection.disconnect()
      throw IllegalStateException("$url: HTTP ${connection.responseCode}")
    }
    return connection
  }

  private fun fetch(url: String): ByteArray {
    val connection = open(url)
    try {
      return connection.inputStream.use { it.readBytes() }
    } finally {
      connection.disconnect()
    }
  }

  /** Downloads [url] to [to] and returns its SHA-256. */
  private fun download(url: String, to: File): String {
    val digest = MessageDigest.getInstance("SHA-256")
    val connection = open(url)
    try {
      connection.inputStream.use { input ->
        to.outputStream().use { out ->
          val buffer = ByteArray(64 * 1024)
          while (true) {
            val n = input.read(buffer)
            if (n < 0) break
            digest.update(buffer, 0, n)
            out.write(buffer, 0, n)
          }
        }
      }
    } finally {
      connection.disconnect()
    }
    return hex(digest.digest())
  }

  private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }
}

/** The installer's results ([Updater.onStatus]) and "this app was just updated" ([Updater.onReplaced]). */
class UpdateReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    when (intent.action) {
      Updater.ACTION_STATUS -> Updater.onStatus(context, intent)
      Intent.ACTION_MY_PACKAGE_REPLACED -> Updater.onReplaced(context)
    }
  }
}
