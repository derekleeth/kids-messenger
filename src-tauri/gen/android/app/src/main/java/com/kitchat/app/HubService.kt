package com.kitchat.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * Keeps KitChat's process, and with it the Rust side's connection to the family hub (src-tauri/src/engine.rs), alive
 * while the app is in the background. Android freezes a background app's network after a short while unless a
 * foreground service runs, and KitChat uses no Google push service, so this is how new messages still arrive and get
 * notified. Shows a quiet, ongoing "Connected to your family" notification (which can be hidden in Android's settings
 * by turning off the "Staying connected" category).
 *
 * Started and stopped by [KitPlugin] `keepAlive`, while this device is in a family.
 */
class HubService : Service() {
  companion object {
    private const val CHANNEL_ID = "connection"
    private const val NOTIFICATION_ID = 4201

    @Volatile
    var running = false
      private set
  }

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    running = true
    Updater.start(this)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val channel = NotificationChannel(CHANNEL_ID, "Staying connected", NotificationManager.IMPORTANCE_MIN)
      channel.description = "Shown while KitChat stays connected to get new messages"
      channel.setShowBadge(false)
      getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val type =
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
      else 0
    ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(), type)
    // If Android ends the process, the app (and its hub connection) only comes back when it's opened, so a restarted
    // service on its own would do nothing.
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    running = false
    super.onDestroy()
  }

  private fun notification(): Notification {
    val open = PendingIntent.getActivity(
      this,
      0,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
    return NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(R.drawable.ic_stat_kit)
      .setContentTitle("KitChat")
      .setContentText("Connected to your family")
      .setOngoing(true)
      .setSilent(true)
      .setShowWhen(false)
      .setPriority(NotificationCompat.PRIORITY_MIN)
      .setCategory(NotificationCompat.CATEGORY_SERVICE)
      .setContentIntent(open)
      .build()
  }
}
