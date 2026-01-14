# Keep all public methods of Fcm2UpShim (called via reflection and smali hooks)
-keep class com.fcm2up.Fcm2UpShim {
    public static *;
}

# Keep the receiver (referenced in AndroidManifest)
-keep class com.fcm2up.Fcm2UpReceiver {
    public *;
}

# Keep annotations
-keepattributes *Annotation*
