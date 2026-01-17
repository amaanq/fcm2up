package com.fcm2up

import android.app.BroadcastOptions
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.os.Build
import android.util.Log
import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.lang.reflect.Method
import java.net.HttpURLConnection
import java.net.URL
import java.util.UUID
import java.util.concurrent.Executors
import org.json.JSONObject

/**
 * FCM-to-UnifiedPush Shim
 *
 * Intercepts FCM tokens and messages, forwards to UnifiedPush.
 * Injected into apps by the fcm2up patcher.
 */
object Fcm2UpShim {
    private const val TAG = "FCM2UP"
    private const val PREFS_NAME = "fcm2up_prefs"

    private const val KEY_ENDPOINT = "up_endpoint"
    private const val KEY_TOKEN = "up_token"
    private const val KEY_FCM_TOKEN = "fcm_token"
    private const val KEY_BRIDGE_FCM_TOKEN = "bridge_fcm_token"
    private const val KEY_BRIDGE_URL = "bridge_url"
    private const val KEY_DISTRIBUTOR = "distributor"
    private const val KEY_FIREBASE_APP_ID = "firebase_app_id"
    private const val KEY_FIREBASE_PROJECT_ID = "firebase_project_id"
    private const val KEY_FIREBASE_API_KEY = "firebase_api_key"
    private const val KEY_FCM_HANDLER_CLASS = "fcm_handler_class"
    private const val KEY_FCM_HANDLER_METHOD = "fcm_handler_method"
    private const val KEY_FCM_SERVICE_CLASS = "fcm_service_class"
    private const val KEY_CERT_SHA1 = "cert_sha1"
    private const val KEY_TOKEN_DELIVERED = "token_delivered"

    // Actions we SEND to the distributor (ntfy)
    private const val ACTION_REGISTER = "org.unifiedpush.android.distributor.REGISTER"
    private const val ACTION_UNREGISTER = "org.unifiedpush.android.distributor.UNREGISTER"

    // Re-entry guard: set to true when WE are triggering onNewToken
    // This prevents infinite loops when our injected hook fires
    @Volatile
    private var isInjectingToken = false

    private const val DEFAULT_DISTRIBUTOR = "io.heckel.ntfy"

    private val executor = Executors.newSingleThreadExecutor()

    // Helper to avoid kotlin stdlib StringsKt
    private fun notEmpty(s: String?): Boolean = s != null && s.length > 0
    private fun preview(s: String?): String {
        if (s == null) return "null"
        return if (s.length > 20) s.substring(0, 20) + "..." else s
    }

    // Helper to try getting a method without throwing
    private fun tryGetMethod(clazz: Class<*>, name: String): java.lang.reflect.Method? {
        return try {
            clazz.getDeclaredMethod(name, String::class.java)
        } catch (e: NoSuchMethodException) {
            null
        }
    }

    /**
     * Configure the shim with Firebase credentials, FCM service class, and original cert SHA1.
     */
    @JvmStatic
    fun configure(
        context: Context,
        bridgeUrl: String,
        distributor: String,
        firebaseAppId: String?,
        firebaseProjectId: String?,
        firebaseApiKey: String?,
        fcmServiceClass: String?,
        certSha1: String?
    ) {
        val prefs = getPrefs(context)
        val editor = prefs.edit()
        editor.putString(KEY_BRIDGE_URL, bridgeUrl)
        editor.putString(KEY_DISTRIBUTOR, if (distributor.length > 0) distributor else DEFAULT_DISTRIBUTOR)
        if (notEmpty(firebaseAppId)) editor.putString(KEY_FIREBASE_APP_ID, firebaseAppId)
        if (notEmpty(firebaseProjectId)) editor.putString(KEY_FIREBASE_PROJECT_ID, firebaseProjectId)
        if (notEmpty(firebaseApiKey)) editor.putString(KEY_FIREBASE_API_KEY, firebaseApiKey)
        if (notEmpty(fcmServiceClass)) editor.putString(KEY_FCM_SERVICE_CLASS, fcmServiceClass)
        if (notEmpty(certSha1)) editor.putString(KEY_CERT_SHA1, certSha1)
        editor.apply()

        Log.i(TAG, "Configured: bridge=$bridgeUrl, distributor=$distributor, firebase_app_id=${preview(firebaseAppId)}, fcm_service=${preview(fcmServiceClass)}, cert=${preview(certSha1)}")
    }

    /**
     * Intercept onNewToken call and replace with bridge token if available.
     * Called from patched smali before the original onNewToken implementation.
     * Returns the token that should be used (bridge token or original).
     */
    @JvmStatic
    fun interceptToken(context: Context, originalToken: String): String {
        val prefs = getPrefs(context)
        val bridgeToken = prefs.getString(KEY_BRIDGE_FCM_TOKEN, null)

        // If we have a bridge token, use it instead of Google's token
        if (notEmpty(bridgeToken) && bridgeToken != null) {
            Log.i(TAG, "Intercepting onNewToken: replacing Google token with bridge token")
            Log.d(TAG, "  Original: ${preview(originalToken)}")
            Log.d(TAG, "  Bridge:   ${preview(bridgeToken)}")
            return bridgeToken
        }

        // No bridge token yet - store the Google token and trigger registration
        Log.d(TAG, "onNewToken intercepted with Google token: ${preview(originalToken)}")
        prefs.edit().putString(KEY_FCM_TOKEN, originalToken).apply()

        if (getEndpoint(context) != null) {
            sendRegistrationToBridge(context)
        }

        return originalToken
    }

    /**
     * Called when app receives new FCM token from Google.
     * @deprecated Use interceptToken instead - this is kept for compatibility
     */
    @JvmStatic
    fun onToken(context: Context, fcmToken: String) {
        // Check re-entry guard: if WE triggered this call, ignore it
        if (isInjectingToken) {
            Log.d(TAG, "Ignoring re-entrant onToken call (we triggered this)")
            return
        }

        Log.d(TAG, "FCM token received from Google: ${preview(fcmToken)}")
        getPrefs(context).edit().putString(KEY_FCM_TOKEN, fcmToken).apply()

        if (getEndpoint(context) != null) {
            sendRegistrationToBridge(context)
        }
    }

    /**
     * Get the FCM token that the app should use (bridge's token).
     */
    @JvmStatic
    fun getEffectiveFcmToken(context: Context): String? {
        val prefs = getPrefs(context)
        val bridgeToken = prefs.getString(KEY_BRIDGE_FCM_TOKEN, null)
        return if (bridgeToken != null) bridgeToken else prefs.getString(KEY_FCM_TOKEN, null)
    }

    /**
     * Called when app receives FCM message.
     */
    @JvmStatic
    fun onFcmMessage(context: Context, data: Map<String, String>) {
        Log.d(TAG, "FCM message received directly (unusual path)")
        val json = mapToJson(data)
        forwardToFcmHandler(context, json.toByteArray())
    }

    /**
     * Register with UnifiedPush distributor.
     */
    @JvmStatic
    fun register(context: Context) {
        Log.i(TAG, "Registering with UnifiedPush")

        val prefs = getPrefs(context)
        var token = prefs.getString(KEY_TOKEN, null)
        if (token == null) {
            token = UUID.randomUUID().toString()
            prefs.edit().putString(KEY_TOKEN, token).apply()
        }

        val distributor = prefs.getString(KEY_DISTRIBUTOR, DEFAULT_DISTRIBUTOR)!!

        val intent = Intent(ACTION_REGISTER)
        intent.`package` = distributor
        intent.putExtra("token", token)
        intent.putExtra("application", context.packageName)

        if (Build.VERSION.SDK_INT >= 34) {
            val options = BroadcastOptions.makeBasic()
            options.setShareIdentityEnabled(true)
            context.sendBroadcast(intent, null, options.toBundle())
        } else {
            context.sendBroadcast(intent)
        }

        Log.d(TAG, "Sent REGISTER to $distributor with token $token")
    }

    /**
     * Unregister from UnifiedPush distributor.
     */
    @JvmStatic
    fun unregister(context: Context) {
        val prefs = getPrefs(context)
        val token = prefs.getString(KEY_TOKEN, null)
        if (token == null) return

        val distributor = prefs.getString(KEY_DISTRIBUTOR, DEFAULT_DISTRIBUTOR)!!

        val intent = Intent(ACTION_UNREGISTER)
        intent.`package` = distributor
        intent.putExtra("token", token)
        intent.putExtra("application", context.packageName)

        context.sendBroadcast(intent)
        Log.i(TAG, "Sent UNREGISTER to $distributor")
    }

    /**
     * Called when we receive a new endpoint from UnifiedPush.
     */
    @JvmStatic
    fun onNewEndpoint(context: Context, endpoint: String) {
        Log.i(TAG, "New endpoint: $endpoint")
        getPrefs(context).edit().putString(KEY_ENDPOINT, endpoint).apply()
        sendRegistrationToBridge(context)
    }

    /**
     * Called when we receive a message from UnifiedPush.
     */
    @JvmStatic
    fun onMessage(context: Context, message: ByteArray) {
        Log.d(TAG, "UP message received: ${message.size} bytes")

        // Log message content for debugging
        val messageStr = String(message)
        Log.d(TAG, "Message content: $messageStr")

        // Try to start the FCM service with the message data
        val prefs = getPrefs(context)
        val fcmServiceClass = prefs.getString(KEY_FCM_SERVICE_CLASS, null)

        if (fcmServiceClass != null && fcmServiceClass.length > 0) {
            try {
                val intent = Intent()
                intent.setClassName(context.packageName, fcmServiceClass)
                intent.action = "com.fcm2up.FCM_MESSAGE"
                intent.putExtra("message", message)
                intent.putExtra("message_string", messageStr)
                context.startService(intent)
                Log.i(TAG, "Started FCM service with message")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to start FCM service with message", e)
                // Fall back to showing notification directly
                showNotificationFromMessage(context, messageStr)
            }
        } else {
            Log.w(TAG, "No FCM service class configured")
            showNotificationFromMessage(context, messageStr)
        }
    }

    /**
     * Parse FCM message data and show a notification directly.
     * Fallback when we can't start the app's FCM service.
     */
    private fun showNotificationFromMessage(context: Context, messageStr: String) {
        try {
            val json = JSONObject(messageStr)

            // Try to extract notification fields from FCM data
            val title = json.optString("title", json.optString("notification_title", "GitHub"))
            val body = json.optString("body", json.optString("notification_body", json.optString("message", "")))

            if (body.length > 0) {
                showNotification(context, title, body)
            } else {
                Log.d(TAG, "No notification body in message, raw data only")
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to parse message as JSON", e)
        }
    }

    /**
     * Show a notification using Android's notification API.
     */
    private fun showNotification(context: Context, title: String, body: String) {
        try {
            val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as android.app.NotificationManager

            // Create notification channel for Android O+
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
                val channel = android.app.NotificationChannel(
                    "fcm2up_channel",
                    "Push Notifications",
                    android.app.NotificationManager.IMPORTANCE_DEFAULT
                )
                notificationManager.createNotificationChannel(channel)
            }

            // Create launch intent
            val launchIntent = context.packageManager.getLaunchIntentForPackage(context.packageName)
            val pendingIntent = if (launchIntent != null) {
                android.app.PendingIntent.getActivity(
                    context, 0, launchIntent,
                    android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_IMMUTABLE
                )
            } else null

            val notification = android.app.Notification.Builder(context, "fcm2up_channel")
                .setContentTitle(title)
                .setContentText(body)
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setAutoCancel(true)
                .setContentIntent(pendingIntent)
                .build()

            notificationManager.notify(System.currentTimeMillis().toInt(), notification)
            Log.i(TAG, "Showed notification: $title - $body")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to show notification", e)
        }
    }

    /**
     * Called when registration fails.
     */
    @JvmStatic
    fun onRegistrationFailed(context: Context, reason: String?) {
        Log.e(TAG, "Registration failed: $reason")
    }

    /**
     * Called when we're unregistered.
     */
    @JvmStatic
    fun onUnregistered(context: Context) {
        Log.i(TAG, "Unregistered from UnifiedPush")
        val editor = getPrefs(context).edit()
        editor.remove(KEY_ENDPOINT)
        editor.remove(KEY_TOKEN)
        editor.remove(KEY_BRIDGE_FCM_TOKEN)
        editor.apply()
    }

    private fun sendRegistrationToBridge(context: Context) {
        val prefs = getPrefs(context)
        val endpoint = prefs.getString(KEY_ENDPOINT, null)
        val bridgeUrl = prefs.getString(KEY_BRIDGE_URL, null)
        val firebaseAppId = prefs.getString(KEY_FIREBASE_APP_ID, null)
        val firebaseProjectId = prefs.getString(KEY_FIREBASE_PROJECT_ID, null)
        val firebaseApiKey = prefs.getString(KEY_FIREBASE_API_KEY, null)
        val certSha1 = prefs.getString(KEY_CERT_SHA1, null)

        if (endpoint == null || bridgeUrl == null) {
            Log.d(TAG, "Missing data for bridge registration")
            return
        }

        if (firebaseAppId == null || firebaseProjectId == null || firebaseApiKey == null) {
            Log.w(TAG, "Missing Firebase credentials - bridge won't be able to receive FCM")
        }

        val packageName = context.packageName

        // Get app version info - computed upfront as final vals to avoid Ref$ObjectRef
        val packageInfo = try { context.packageManager.getPackageInfo(packageName, 0) } catch (e: Exception) { null }
        val applicationInfo = try { context.packageManager.getApplicationInfo(packageName, 0) } catch (e: Exception) { null }

        val appVersion: Int? = if (packageInfo != null) {
            if (Build.VERSION.SDK_INT >= 28) {
                packageInfo.longVersionCode.toInt()
            } else {
                @Suppress("DEPRECATION")
                packageInfo.versionCode
            }
        } else null

        val appVersionName: String? = packageInfo?.versionName
        val targetSdk: Int? = applicationInfo?.targetSdkVersion

        if (appVersion != null) {
            Log.d(TAG, "App info: version=$appVersion, versionName=$appVersionName, targetSdk=$targetSdk")
        }

        executor.execute {
            try {
                val url = URL("$bridgeUrl/register")
                val conn = url.openConnection() as HttpURLConnection
                conn.requestMethod = "POST"
                conn.doOutput = true
                conn.setRequestProperty("Content-Type", "application/json")
                conn.connectTimeout = 10000
                conn.readTimeout = 10000

                val jsonObj = JSONObject()
                jsonObj.put("endpoint", endpoint)
                jsonObj.put("app_id", packageName)
                if (firebaseAppId != null) jsonObj.put("firebase_app_id", firebaseAppId)
                if (firebaseProjectId != null) jsonObj.put("firebase_project_id", firebaseProjectId)
                if (firebaseApiKey != null) jsonObj.put("firebase_api_key", firebaseApiKey)
                if (certSha1 != null) jsonObj.put("cert_sha1", certSha1)
                if (appVersion != null) jsonObj.put("app_version", appVersion)
                if (appVersionName != null) jsonObj.put("app_version_name", appVersionName)
                if (targetSdk != null) jsonObj.put("target_sdk", targetSdk)

                val writer = OutputStreamWriter(conn.outputStream)
                writer.write(jsonObj.toString())
                writer.flush()
                writer.close()

                val responseCode = conn.responseCode
                if (responseCode == 200) {
                    val reader = BufferedReader(InputStreamReader(conn.inputStream))
                    val sb = StringBuilder()
                    var line: String? = reader.readLine()
                    while (line != null) {
                        sb.append(line)
                        line = reader.readLine()
                    }
                    reader.close()

                    val responseBody = sb.toString()
                    try {
                        val response = JSONObject(responseBody)
                        val bridgeFcmToken = response.optString("fcm_token", null)
                        if (notEmpty(bridgeFcmToken)) {
                            prefs.edit().putString(KEY_BRIDGE_FCM_TOKEN, bridgeFcmToken).apply()
                            Log.i(TAG, "Got bridge FCM token: ${preview(bridgeFcmToken)}")

                            // CRITICAL: Trigger the app's onNewToken with the bridge's token
                            // This makes the app send the bridge's token to its backend
                            triggerAppOnNewToken(context, bridgeFcmToken)
                        }
                        Log.i(TAG, "Registered with bridge: ${response.optString("message", "success")}")
                    } catch (e: Exception) {
                        Log.w(TAG, "Could not parse bridge response: $responseBody")
                    }
                } else {
                    val reader = BufferedReader(InputStreamReader(conn.errorStream))
                    val sb = StringBuilder()
                    var line: String? = reader.readLine()
                    while (line != null) {
                        sb.append(line)
                        line = reader.readLine()
                    }
                    reader.close()
                    Log.e(TAG, "Bridge registration failed: $responseCode - $sb")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Bridge registration error", e)
            }
        }
    }

    /**
     * Set the app's FCM handler for message forwarding.
     */
    @JvmStatic
    fun setFcmHandler(context: Context, handlerClass: String, handlerMethod: String) {
        val editor = getPrefs(context).edit()
        editor.putString(KEY_FCM_HANDLER_CLASS, handlerClass)
        editor.putString(KEY_FCM_HANDLER_METHOD, handlerMethod)
        editor.apply()
    }

    /**
     * Forward message to app's original FCM handler via reflection.
     */
    @JvmStatic
    fun forwardToFcmHandler(context: Context, message: ByteArray) {
        val prefs = getPrefs(context)
        val handlerClass = prefs.getString(KEY_FCM_HANDLER_CLASS, null)
        val handlerMethod = prefs.getString(KEY_FCM_HANDLER_METHOD, null)

        if (handlerClass == null || handlerMethod == null) {
            Log.w(TAG, "No FCM handler configured, message not forwarded")
            return
        }

        try {
            val clazz = Class.forName(handlerClass)

            var method: Method? = null
            try {
                method = clazz.getDeclaredMethod(handlerMethod, Context::class.java, ByteArray::class.java)
            } catch (e: NoSuchMethodException) {
                try {
                    method = clazz.getDeclaredMethod(handlerMethod, Context::class.java, String::class.java)
                } catch (e2: NoSuchMethodException) {
                    method = clazz.getDeclaredMethod(handlerMethod, ByteArray::class.java)
                }
            }

            if (method != null) {
                method.isAccessible = true
                val paramCount = method.parameterTypes.size
                if (paramCount == 2) {
                    if (method.parameterTypes[1] == ByteArray::class.java) {
                        method.invoke(null, context, message)
                    } else {
                        method.invoke(null, context, String(message))
                    }
                } else if (paramCount == 1) {
                    method.invoke(null, message as Any)
                }
                Log.d(TAG, "Forwarded message to $handlerClass.$handlerMethod")
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to forward to FCM handler", e)
        }
    }

    /**
     * Trigger the app's onNewToken callback with the bridge's token.
     * This makes the app send the bridge's token to its backend.
     *
     * Strategy: Use our patched onStartCommand in the FCM service.
     * When we send an intent with action "com.fcm2up.INJECT_TOKEN",
     * the patched service will extract the token and call onNewToken() directly.
     * This works on non-GMS devices because the service is properly initialized
     * through Android's lifecycle (DI injection happens in onCreate).
     */
    private fun triggerAppOnNewToken(context: Context, bridgeToken: String) {
        val prefs = getPrefs(context)
        val fcmServiceClass = prefs.getString(KEY_FCM_SERVICE_CLASS, null)

        if (fcmServiceClass == null || fcmServiceClass.length == 0) {
            Log.w(TAG, "No FCM service class configured, cannot inject bridge token")
            return
        }

        Log.i(TAG, "Triggering onNewToken via patched service: ${preview(bridgeToken)}")

        try {
            // Create intent with our special action
            // The patched onStartCommand will recognize this and call onNewToken()
            val intent = Intent()
            intent.setClassName(context.packageName, fcmServiceClass)
            intent.action = "com.fcm2up.INJECT_TOKEN"
            intent.putExtra("token", bridgeToken)

            // Start the service - this triggers:
            // 1. Service.onCreate() if not running (DI injection happens here)
            // 2. Our patched onStartCommand() which calls onNewToken(bridgeToken)
            context.startService(intent)
            Log.i(TAG, "Started FCM service with INJECT_TOKEN action")

        } catch (e: Exception) {
            Log.e(TAG, "Failed to trigger onNewToken via service", e)
        }
    }

    private fun getPrefs(context: Context): SharedPreferences {
        return context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    }

    @JvmStatic
    fun getEndpoint(context: Context): String? {
        return getPrefs(context).getString(KEY_ENDPOINT, null)
    }

    @JvmStatic
    fun getFcmToken(context: Context): String? {
        return getPrefs(context).getString(KEY_FCM_TOKEN, null)
    }

    @JvmStatic
    fun getBridgeUrl(context: Context): String? {
        return getPrefs(context).getString(KEY_BRIDGE_URL, null)
    }

    /**
     * Check for a pending bridge token that needs to be delivered.
     * Called from smali on every FCM service start.
     * Returns the token if pending, null otherwise.
     * Automatically marks the token as delivered to prevent repeated calls.
     */
    @JvmStatic
    fun getPendingBridgeToken(context: Context): String? {
        val prefs = getPrefs(context)
        val bridgeToken = prefs.getString(KEY_BRIDGE_FCM_TOKEN, null)
        val delivered = prefs.getBoolean(KEY_TOKEN_DELIVERED, false)

        if (bridgeToken != null && bridgeToken.length > 0 && !delivered) {
            // Mark as delivered FIRST to prevent re-entry
            prefs.edit().putBoolean(KEY_TOKEN_DELIVERED, true).apply()
            Log.i(TAG, "Pending bridge token found, will deliver: ${preview(bridgeToken)}")
            return bridgeToken
        }

        return null
    }

    /**
     * Reset the token delivered flag.
     * Call this when re-registration is needed.
     */
    @JvmStatic
    fun resetTokenDelivery(context: Context) {
        getPrefs(context).edit().putBoolean(KEY_TOKEN_DELIVERED, false).apply()
    }

    private fun mapToJson(map: Map<String, String>): String {
        val sb = StringBuilder("{")
        var first = true
        for ((k, v) in map) {
            if (!first) sb.append(",")
            first = false
            sb.append("\"").append(escapeJson(k)).append("\":\"").append(escapeJson(v)).append("\"")
        }
        sb.append("}")
        return sb.toString()
    }

    private fun escapeJson(s: String): String {
        val sb = StringBuilder()
        for (c in s) {
            when (c) {
                '\\' -> sb.append("\\\\")
                '"' -> sb.append("\\\"")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                else -> sb.append(c)
            }
        }
        return sb.toString()
    }
}
