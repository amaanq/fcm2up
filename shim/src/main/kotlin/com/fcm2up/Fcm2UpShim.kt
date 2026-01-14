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

    private const val ACTION_REGISTER = "org.unifiedpush.android.connector.REGISTER"
    private const val ACTION_UNREGISTER = "org.unifiedpush.android.connector.UNREGISTER"

    private const val DEFAULT_DISTRIBUTOR = "io.heckel.ntfy"

    private val executor = Executors.newSingleThreadExecutor()

    // Helper to avoid kotlin stdlib StringsKt
    private fun notEmpty(s: String?): Boolean = s != null && s.length > 0
    private fun preview(s: String?, len: Int = 20): String {
        if (s == null) return "null"
        return if (s.length > len) s.substring(0, len) + "..." else s
    }

    /**
     * Configure the shim with Firebase credentials.
     */
    @JvmStatic
    fun configure(
        context: Context,
        bridgeUrl: String,
        distributor: String,
        firebaseAppId: String?,
        firebaseProjectId: String?,
        firebaseApiKey: String?
    ) {
        val prefs = getPrefs(context)
        val editor = prefs.edit()
        editor.putString(KEY_BRIDGE_URL, bridgeUrl)
        editor.putString(KEY_DISTRIBUTOR, if (distributor.length > 0) distributor else DEFAULT_DISTRIBUTOR)
        if (notEmpty(firebaseAppId)) editor.putString(KEY_FIREBASE_APP_ID, firebaseAppId)
        if (notEmpty(firebaseProjectId)) editor.putString(KEY_FIREBASE_PROJECT_ID, firebaseProjectId)
        if (notEmpty(firebaseApiKey)) editor.putString(KEY_FIREBASE_API_KEY, firebaseApiKey)
        editor.apply()

        Log.i(TAG, "Configured: bridge=$bridgeUrl, distributor=$distributor, firebase_app_id=${preview(firebaseAppId)}")
    }

    /**
     * Called when app receives new FCM token from Google.
     */
    @JvmStatic
    fun onToken(context: Context, fcmToken: String) {
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

        if (endpoint == null || bridgeUrl == null) {
            Log.d(TAG, "Missing data for bridge registration")
            return
        }

        if (firebaseAppId == null || firebaseProjectId == null || firebaseApiKey == null) {
            Log.w(TAG, "Missing Firebase credentials - bridge won't be able to receive FCM")
        }

        val packageName = context.packageName

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
