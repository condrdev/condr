package dev.condr.companion.push

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailability
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import dev.condr.companion.CompanionApplication
import dev.condr.companion.MainActivity
import dev.condr.companion.R
import dev.condr.companion.model.PushedPane
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/** One channel for every push (ADR 0048): an Agent that finished, failed or needs you. */
object Notifications {
  const val CHANNEL = "agents"
  const val DEVICE = "device"
  const val PANE = "pane"

  fun createChannel(context: Context) {
    val channel = NotificationChannel(CHANNEL, "Agents", NotificationManager.IMPORTANCE_HIGH)
    channel.description = "An agent finished, failed or needs you"
    context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
  }

  /** Whether the phone can receive pushes at all: Google Play services and a Firebase app. */
  fun reachable(context: Context): Boolean =
    GoogleApiAvailability.getInstance().isGooglePlayServicesAvailable(context) ==
      ConnectionResult.SUCCESS && FirebaseApp.getApps(context).isNotEmpty()

  fun granted(context: Context): Boolean =
    Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
      ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
        PackageManager.PERMISSION_GRANTED

  /** Asks FCM for this launch's token; the app's `Push` hears the answer. */
  fun fetchToken(context: Context, received: suspend (String) -> Unit, failed: () -> Unit) {
    android.util.Log.i("condr.push", "fetching token")
    if (!reachable(context)) {
      android.util.Log.i(
        "condr.push",
        "unreachable: play=${GoogleApiAvailability.getInstance().isGooglePlayServicesAvailable(context)} firebase=${FirebaseApp.getApps(context).size}",
      )
      failed()
      return
    }
    FirebaseMessaging.getInstance()
      .token
      .addOnSuccessListener { token ->
        CoroutineScope(Dispatchers.Main.immediate).launch { received(token) }
      }
      .addOnFailureListener { failed() }
  }

  /** The Pane a launching or resumed Intent names, from a tapped notification (ADR 0047). */
  fun pushed(intent: Intent?): PushedPane? =
    PushedPane.from(intent?.getStringExtra(DEVICE), intent?.getStringExtra(PANE))

  /** A notification as the system would show it in the background, for the foreground's few. */
  fun show(context: Context, title: String, body: String?, tag: String?, pushed: PushedPane?) {
    val open =
      Intent(context, MainActivity::class.java).apply {
        flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP
        pushed?.let {
          putExtra(DEVICE, it.device)
          putExtra(PANE, it.pane.toString())
        }
      }
    val notification =
      NotificationCompat.Builder(context, CHANNEL)
        .setSmallIcon(R.drawable.ic_launcher_monochrome)
        .setContentTitle(title)
        .setContentText(body)
        .setStyle(NotificationCompat.BigTextStyle().bigText(body))
        .setAutoCancel(true)
        .setPriority(NotificationCompat.PRIORITY_HIGH)
        .setContentIntent(
          PendingIntent.getActivity(
            context,
            tag.hashCode(),
            open,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
          )
        )
        .build()
    context.getSystemService(NotificationManager::class.java).notify(tag, 0, notification)
  }
}

/**
 * FCM's callbacks (ADR 0047): a new token registers again, and a push that arrives while the app is
 * in the foreground shows only while its Device is not connected here.
 */
class CondrMessagingService : FirebaseMessagingService() {
  override fun onNewToken(token: String) {
    val stores = (application as CompanionApplication).current() ?: return
    CoroutineScope(Dispatchers.Main.immediate).launch { stores.push?.received(token) }
  }

  override fun onMessageReceived(message: RemoteMessage) {
    val stores = (application as CompanionApplication).current()
    val pushed =
      PushedPane.from(message.data[Notifications.DEVICE], message.data[Notifications.PANE])
    if (stores != null && pushed != null && !stores.session.presents(pushed)) return
    val notification = message.notification ?: return
    val key = notification.titleLocalizationKey
    val id = key?.let { resources.getIdentifier(it, "string", packageName) } ?: 0
    val title =
      if (id != 0) getString(id, *(notification.titleLocalizationArgs ?: emptyArray()))
      else notification.title ?: getString(R.string.app_name)
    Notifications.show(this, title, notification.body, notification.tag, pushed)
  }
}
