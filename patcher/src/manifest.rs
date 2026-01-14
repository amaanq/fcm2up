//! AndroidManifest.xml manipulation
//!
//! Adds UnifiedPush receiver and required permissions.

use anyhow::{Context, Result};
use regex::Regex;
use std::path::Path;

/// Add the UnifiedPush receiver to AndroidManifest.xml
pub fn add_unifiedpush_receiver(manifest_path: &Path, _package_name: &str) -> Result<()> {
    let content = std::fs::read_to_string(manifest_path)
        .context("Failed to read AndroidManifest.xml")?;

    // Check if already patched
    if content.contains("com.fcm2up.Fcm2UpReceiver") {
        println!("  Manifest already contains fcm2up receiver, skipping");
        return Ok(());
    }

    let mut new_content = content.clone();

    // Add INTERNET permission if not present
    if !new_content.contains("android.permission.INTERNET") {
        new_content = add_permission(&new_content, "android.permission.INTERNET");
    }

    // Add receiver declaration before </application>
    let receiver_declaration = r#"
        <receiver
            android:name="com.fcm2up.Fcm2UpReceiver"
            android:exported="true">
            <intent-filter>
                <action android:name="org.unifiedpush.android.connector.MESSAGE"/>
                <action android:name="org.unifiedpush.android.connector.NEW_ENDPOINT"/>
                <action android:name="org.unifiedpush.android.connector.REGISTRATION_FAILED"/>
                <action android:name="org.unifiedpush.android.connector.UNREGISTERED"/>
            </intent-filter>
        </receiver>
    "#.to_string();

    // Find </application> and insert before it
    let app_end = new_content
        .find("</application>")
        .context("</application> not found in manifest")?;

    new_content.insert_str(app_end, &receiver_declaration);

    // Add queries for ntfy package (required for Android 11+)
    if !new_content.contains("<queries>") {
        let queries_section = r#"
    <queries>
        <package android:name="io.heckel.ntfy"/>
    </queries>
"#;
        // Insert before <application
        if let Some(app_start) = new_content.find("<application") {
            new_content.insert_str(app_start, queries_section);
        }
    } else if !new_content.contains("io.heckel.ntfy") {
        // Add ntfy to existing queries
        let queries_end = new_content
            .find("</queries>")
            .context("</queries> not found")?;
        new_content.insert_str(
            queries_end,
            r#"        <package android:name="io.heckel.ntfy"/>
    "#,
        );
    }

    std::fs::write(manifest_path, new_content)?;
    Ok(())
}

fn add_permission(manifest: &str, permission: &str) -> String {
    let perm_line = format!(
        r#"    <uses-permission android:name="{}"/>
"#,
        permission
    );

    // Find first <uses-permission or <application to insert before
    if let Some(pos) = manifest.find("<uses-permission") {
        let mut result = manifest.to_string();
        result.insert_str(pos, &perm_line);
        result
    } else if let Some(pos) = manifest.find("<application") {
        let mut result = manifest.to_string();
        result.insert_str(pos, &perm_line);
        result
    } else {
        manifest.to_string()
    }
}

/// Remove split APK requirements from manifest (for base APK patching)
pub fn remove_split_requirements(manifest_path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(manifest_path)?;

    // Remove android:requiredSplitTypes
    let re1 = Regex::new(r#"\s*android:requiredSplitTypes="[^"]*""#)?;
    let content = re1.replace_all(&content, "");

    // Remove android:splitTypes
    let re2 = Regex::new(r#"\s*android:splitTypes="[^"]*""#)?;
    let content = re2.replace_all(&content, "");

    // Remove split configuration metadata
    let re3 = Regex::new(
        r#"<meta-data[^>]*android:name="com\.android\.vending\.splits[^"]*"[^>]*/>\s*"#,
    )?;
    let content = re3.replace_all(&content, "");

    // Remove android:isSplitRequired
    let re4 = Regex::new(r#"\s*android:isSplitRequired="[^"]*""#)?;
    let content = re4.replace_all(&content, "");

    std::fs::write(manifest_path, content.as_ref())?;
    Ok(())
}

/// Get application class name from manifest
pub fn get_application_class(manifest_path: &Path) -> Result<Option<String>> {
    let content = std::fs::read_to_string(manifest_path)?;

    let re = Regex::new(r#"<application[^>]*android:name="([^"]+)""#)?;

    Ok(re
        .captures(&content)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string()))
}
