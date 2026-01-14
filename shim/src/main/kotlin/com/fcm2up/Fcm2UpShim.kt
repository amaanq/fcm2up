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

    // UnifiedPush actions
    private const val ACTION_REGISTER = "org.unifiedpush.android.connector.REGISTER"
    private const val ACTION_UNREGISTER = "org.unifiedpush.android.connector.UNREGISTER"

    // Default distributor (ntfy)
    private const val DEFAULT_DISTRIBUTOR = "io.heckel.ntfy"

    private val executor = Executors.newSingleThreadExecutor()

    // ==================== Configuration ====================

    /**
     * Configure the shim with Firebase credentials.
     * Called by patcher-generated init code.
     */
    @JvmStatic
    fun configure(
        context: Context,
        bridgeUrl: String,
        distributor: String = DEFAULT_DISTRIBUTOR,
        firebaseAppId: String? = null,
        firebaseProjectId: String? = null,
        firebaseApiKey: String? = null
    ) {
        val prefs = getPrefs(context)
        prefs.edit().apply {
            putString(KEY_BRIDGE_URL, bridgeUrl)
            putString(KEY_DISTRIBUTOR, distributor.ifEmpty { DEFAULT_DISTRIBUTOR })
            // Store Firebase credentials for bridge registration
            if (!firebaseAppId.isNullOrEmpty()) putString(KEY_FIREBASE_APP_ID, firebaseAppId)
            if (!firebaseProjectId.isNullOrEmpty()) putString(KEY_FIREBASE_PROJECT_ID, firebaseProjectId)
            if (!firebaseApiKey.isNullOrEmpty()) putString(KEY_FIREBASE_API_KEY, firebaseApiKey)
            apply()
        }
        Log.i(TAG, "Configured: bridge=$bridgeUrl, distributor=$distributor, firebase_app_id=${firebaseAppId?.take(20)}...")
    }

    // ==================== FCM Token Interception ====================

    /**
     * Called when app receives new FCM token from Google.
     * Hook this into FirebaseMessagingService.onNewToken()
     *
     * IMPORTANT: We store Google's token but don't use it.
     * Instead, we return the bridge's FCM token to the app.
     */
    @JvmStatic
    fun onToken(context: Context, fcmToken: String) {
        Log.d(TAG, "FCM token received from Google: ${fcmToken.take(20)}...")

        // Store Google's FCM token (we need it for... actually we might not need it)
        // The bridge creates its own FCM registration
        getPrefs(context).edit().putString(KEY_FCM_TOKEN, fcmToken).apply()

        // If we have an endpoint, register with bridge
        if (getEndpoint(context) != null) {
            sendRegistrationToBridge(context)
        }
    }

    /**
     * Get the FCM token that the app should use.
     * This returns the BRIDGE's FCM token, not Google's token.
     * The app's backend should send FCM messages to this token,
     * which the bridge will receive and forward via UnifiedPush.
     */
    @JvmStatic
    fun getEffectiveFcmToken(context: Context): String? {
        val prefs = getPrefs(context)
        // Prefer bridge's FCM token, fall back to Google's
        return prefs.getString(KEY_BRIDGE_FCM_TOKEN, null)
            ?: prefs.getString(KEY_FCM_TOKEN, null)
    }

    /**
     * Called when app receives FCM message.
     * This is typically NOT needed since messages come via UnifiedPush,
     * but provided for completeness.
     */
    @JvmStatic
    fun onFcmMessage(context: Context, data: Map<String, String>) {
        Log.d(TAG, "FCM message received directly (unusual path)")
        // Convert to bytes and forward
        val json = mapToJson(data)
        forwardToFcmHandler(context, json.toByteArray())
    }

    // ==================== UnifiedPush Registration ====================

    /**
     * Register with UnifiedPush distributor.
     * Call this on app startup.
     */
    @JvmStatic
    fun register(context: Context) {
        Log.i(TAG, "Registering with UnifiedPush")

        // Generate or retrieve persistent token
        val prefs = getPrefs(context)
        var token = prefs.getString(KEY_TOKEN, null)
        if (token == null) {
            token = UUID.randomUUID().toString()
            prefs.edit().putString(KEY_TOKEN, token).apply()
        }

        val distributor = prefs.getString(KEY_DISTRIBUTOR, DEFAULT_DISTRIBUTOR)!!

        // Create registration intent
        val intent = Intent(ACTION_REGISTER).apply {
            `package` = distributor
            putExtra("token", token)
            putExtra("application", context.packageName)
        }

        // For SDK 34+, use BroadcastOptions with share identity
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
        val token = prefs.getString(KEY_TOKEN, null) ?: return
        val distributor = prefs.getString(KEY_DISTRIBUTOR, DEFAULT_DISTRIBUTOR)!!

        val intent = Intent(ACTION_UNREGISTER).apply {
            `package` = distributor
            putExtra("token", token)
            putExtra("application", context.packageName)
        }

        context.sendBroadcast(intent)
        Log.i(TAG, "Sent UNREGISTER to $distributor")
    }

    // ==================== UnifiedPush Callbacks ====================

    /**
     * Called when we receive a new endpoint from UnifiedPush.
     */
    @JvmStatic
    fun onNewEndpoint(context: Context, endpoint: String) {
        Log.i(TAG, "New endpoint: $endpoint")

        // Store endpoint
        getPrefs(context).edit().putString(KEY_ENDPOINT, endpoint).apply()

        // Send Firebase credentials + endpoint to bridge
        sendRegistrationToBridge(context)
    }

    /**
     * Called when we receive a message from UnifiedPush.
     * This is the main path for FCM messages via the bridge.
     */
    @JvmStatic
    fun onMessage(context: Context, message: ByteArray) {
        Log.d(TAG, "UP message received: ${message.size} bytes")

        // Forward to app's FCM handler
        forwardToFcmHandler(context, message)
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
        getPrefs(context).edit()
            .remove(KEY_ENDPOINT)
            .remove(KEY_TOKEN)
            .remove(KEY_BRIDGE_FCM_TOKEN)
            .apply()
    }

    // ==================== Bridge Communication ====================

    private fun sendRegistrationToBridge(context: Context) {
        val prefs = getPrefs(context)
        val endpoint = prefs.getString(KEY_ENDPOINT, null)
        val bridgeUrl = prefs.getString(KEY_BRIDGE_URL, null)
        val firebaseAppId = prefs.getString(KEY_FIREBASE_APP_ID, null)
        val firebaseProjectId = prefs.getString(KEY_FIREBASE_PROJECT_ID, null)
        val firebaseApiKey = prefs.getString(KEY_FIREBASE_API_KEY, null)

        if (endpoint == null || bridgeUrl == null) {
            Log.d(TAG, "Missing data for bridge registration: endpoint=$endpoint, bridgeUrl=$bridgeUrl")
            return
        }

        if (firebaseAppId == null || firebaseProjectId == null || firebaseApiKey == null) {
            Log.w(TAG, "Missing Firebase credentials - bridge won't be able to receive FCM")
        }

        executor.execute {
            try {
                val url = URL("$bridgeUrl/register")
                val conn = url.openConnection() as HttpURLConnection
                conn.apply {
                    requestMethod = "POST"
                    doOutput = true
                    setRequestProperty("Content-Type", "application/json")
                    connectTimeout = 10000
                    readTimeout = 10000
                }

                // Build JSON with Firebase credentials
                val jsonObj = JSONObject().apply {
                    put("endpoint", endpoint)
                    put("app_id", context.packageName)
                    firebaseAppId?.let { put("firebase_app_id", it) }
                    firebaseProjectId?.let { put("firebase_project_id", it) }
                    firebaseApiKey?.let { put("firebase_api_key", it) }
                }

                OutputStreamWriter(conn.outputStream).use { writer ->
                    writer.write(jsonObj.toString())
                    writer.flush()
                }

                val responseCode = conn.responseCode
                if (responseCode == 200) {
                    // Parse response to get bridge's FCM token
                    val responseBody = BufferedReader(InputStreamReader(conn.inputStream)).use { it.readText() }
                    try {
                        val response = JSONObject(responseBody)
                        val bridgeFcmToken = response.optString("fcm_token", null)
                        if (!bridgeFcmToken.isNullOrEmpty()) {
                            // Store bridge's FCM token - this is what the app should use!
                            prefs.edit().putString(KEY_BRIDGE_FCM_TOKEN, bridgeFcmToken).apply()
                            Log.i(TAG, "Got bridge FCM token: ${bridgeFcmToken.take(20)}...")
                        }
                        Log.i(TAG, "Registered with bridge: ${response.optString("message", "success")}")
                    } catch (e: Exception) {
                        Log.w(TAG, "Could not parse bridge response: $responseBody")
                    }
                } else {
                    val error = BufferedReader(InputStreamReader(conn.errorStream)).use { it.readText() }
                    Log.e(TAG, "Bridge registration failed: $responseCode - $error")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Bridge registration error", e)
            }
        }
    }

    // ==================== FCM Handler Forwarding ====================

    /**
     * Set the app's FCM handler for message forwarding.
     */
    @JvmStatic
    fun setFcmHandler(context: Context, handlerClass: String, handlerMethod: String) {
        getPrefs(context).edit()
            .putString(KEY_FCM_HANDLER_CLASS, handlerClass)
            .putString(KEY_FCM_HANDLER_METHOD, handlerMethod)
            .apply()
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

            // Try different method signatures
            val method: Method? = try {
                // Try (Context, byte[])
                clazz.getDeclaredMethod(handlerMethod, Context::class.java, ByteArray::class.java)
            } catch (e: NoSuchMethodException) {
                try {
                    // Try (Context, String)
                    clazz.getDeclaredMethod(handlerMethod, Context::class.java, String::class.java)
                } catch (e2: NoSuchMethodException) {
                    // Try static (byte[])
                    clazz.getDeclaredMethod(handlerMethod, ByteArray::class.java)
                }
            }

            method?.isAccessible = true

            when (method?.parameterCount) {
                2 -> {
                    if (method.parameterTypes[1] == ByteArray::class.java) {
                        method.invoke(null, context, message)
                    } else {
                        method.invoke(null, context, String(message))
                    }
                }
                1 -> method.invoke(null, message)
                else -> Log.e(TAG, "Unsupported FCM handler signature")
            }

            Log.d(TAG, "Forwarded message to $handlerClass.$handlerMethod")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to forward to FCM handler", e)
        }
    }

    // ==================== Utilities ====================

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

    private fun mapToJson(map: Map<String, String>): String {
        val entries = map.entries.joinToString(",") { (k, v) ->
            "\"${escapeJson(k)}\":\"${escapeJson(v)}\""
        }
        return "{$entries}"
    }

    private fun escapeJson(s: String): String {
        return s.replace("\\", "\\\\")
            .replace("\"", "\\\"")
            .replace("\n", "\\n")
            .replace("\r", "\\r")
            .replace("\t", "\\t")
    }
}
