//! APK manipulation utilities
//!
//! Handles decoding, encoding, and analysis of APK files using apktool.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

/// Decode an APK using apktool
pub fn decode_apk(apk_path: &Path, output_dir: &Path) -> Result<()> {
    println!("  Decoding APK...");

    let status = Command::new("apktool")
        .args(["d", "-f", "-o"])
        .arg(output_dir)
        .arg(apk_path)
        .status()
        .context("Failed to run apktool. Is it installed?")?;

    if !status.success() {
        bail!("apktool decode failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// Build an APK using apktool
pub fn build_apk(decoded_dir: &Path, output_apk: &Path) -> Result<()> {
    println!("  Building APK...");

    let status = Command::new("apktool")
        .args(["b", "-o"])
        .arg(output_apk)
        .arg(decoded_dir)
        .status()
        .context("Failed to run apktool")?;

    if !status.success() {
        bail!("apktool build failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// Sign an APK using apksigner or jarsigner
pub fn sign_apk(
    apk_path: &Path,
    keystore: Option<&Path>,
    keystore_pass: Option<&str>,
    key_alias: Option<&str>,
) -> Result<()> {
    println!("  Signing APK...");

    // Try apksigner first (preferred)
    if let Some(ks) = keystore {
        let mut cmd = Command::new("apksigner");
        cmd.args(["sign", "--ks"]).arg(ks);

        if let Some(pass) = keystore_pass {
            cmd.args(["--ks-pass", &format!("pass:{}", pass)]);
        }

        if let Some(alias) = key_alias {
            cmd.args(["--ks-key-alias", alias]);
        }

        cmd.arg(apk_path);

        let status = cmd.status().context("Failed to run apksigner")?;
        if !status.success() {
            bail!("apksigner failed with exit code: {:?}", status.code());
        }
    } else {
        // Use debug keystore
        let debug_keystore = dirs::home_dir()
            .map(|h| h.join(".android/debug.keystore"))
            .filter(|p| p.exists());

        if let Some(debug_ks) = debug_keystore {
            let status = Command::new("apksigner")
                .args(["sign", "--ks"])
                .arg(&debug_ks)
                .args(["--ks-pass", "pass:android"])
                .args(["--ks-key-alias", "androiddebugkey"])
                .arg(apk_path)
                .status()
                .context("Failed to run apksigner with debug keystore")?;

            if !status.success() {
                bail!("apksigner failed with exit code: {:?}", status.code());
            }
        } else {
            // Create a temporary debug keystore
            println!("  Creating temporary debug keystore...");
            let temp_ks = std::env::temp_dir().join("fcm2up-debug.keystore");

            if !temp_ks.exists() {
                let status = Command::new("keytool")
                    .args(["-genkey", "-v"])
                    .args(["-keystore"])
                    .arg(&temp_ks)
                    .args(["-alias", "fcm2up"])
                    .args(["-keyalg", "RSA"])
                    .args(["-keysize", "2048"])
                    .args(["-validity", "10000"])
                    .args(["-storepass", "fcm2up"])
                    .args(["-keypass", "fcm2up"])
                    .args(["-dname", "CN=FCM2UP, OU=Dev, O=Dev, L=Unknown, ST=Unknown, C=US"])
                    .status()
                    .context("Failed to create debug keystore")?;

                if !status.success() {
                    bail!("keytool failed to create keystore");
                }
            }

            let status = Command::new("apksigner")
                .args(["sign", "--ks"])
                .arg(&temp_ks)
                .args(["--ks-pass", "pass:fcm2up"])
                .args(["--ks-key-alias", "fcm2up"])
                .arg(apk_path)
                .status()
                .context("Failed to sign with temp keystore")?;

            if !status.success() {
                bail!("apksigner failed");
            }
        }
    }

    Ok(())
}

/// Zipalign an APK for optimal loading
pub fn zipalign_apk(apk_path: &Path) -> Result<()> {
    println!("  Zipaligning APK...");

    let aligned_path = apk_path.with_extension("aligned.apk");

    let status = Command::new("zipalign")
        .args(["-f", "4"])
        .arg(apk_path)
        .arg(&aligned_path)
        .status();

    match status {
        Ok(s) if s.success() => {
            std::fs::rename(&aligned_path, apk_path)?;
            Ok(())
        }
        _ => {
            // zipalign is optional, continue without it
            println!("  Warning: zipalign not available, skipping");
            Ok(())
        }
    }
}

/// Find Firebase messaging service class in decompiled APK
/// First checks manifest for registered service, then falls back to smali search
pub fn find_firebase_service(decoded_dir: &Path) -> Result<Option<PathBuf>> {
    // First, try to find the service class from the manifest
    let manifest_path = decoded_dir.join("AndroidManifest.xml");
    if manifest_path.exists() {
        let manifest = std::fs::read_to_string(&manifest_path)?;

        // Look for services with MESSAGING_EVENT intent filter
        // Pattern: <service android:name="com.example.MyService">...<action android:name="com.google.firebase.MESSAGING_EVENT"/>
        if let Some(service_class) = find_fcm_service_from_manifest(&manifest) {
            // Convert class name to smali path
            let smali_path = class_name_to_smali_path(decoded_dir, &service_class);
            if smali_path.exists() {
                return Ok(Some(smali_path));
            }
        }
    }

    // Fall back to searching smali files
    let smali_dirs = find_smali_dirs(decoded_dir);
    let mut candidates = Vec::new();

    for smali_dir in &smali_dirs {
        for entry in WalkDir::new(smali_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "smali"))
        {
            let content = std::fs::read_to_string(entry.path())?;

            // Look for class that extends FirebaseMessagingService
            if content.contains(".super Lcom/google/firebase/messaging/FirebaseMessagingService;") {
                let is_abstract = content.contains(".class public abstract");
                candidates.push((entry.path().to_path_buf(), is_abstract));
            }
        }
    }

    // Prefer non-abstract classes (concrete implementations)
    for (path, is_abstract) in &candidates {
        if !is_abstract {
            return Ok(Some(path.clone()));
        }
    }

    // Fall back to first candidate (even if abstract)
    Ok(candidates.into_iter().next().map(|(p, _)| p))
}

/// Parse manifest to find the FCM service class
fn find_fcm_service_from_manifest(manifest: &str) -> Option<String> {
    // Simple approach: find service with MESSAGING_EVENT action
    // The manifest format after apktool is XML-like

    let mut in_service = false;
    let mut service_name = String::new();
    let mut found_messaging_event = false;

    for line in manifest.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("<service") {
            in_service = true;
            found_messaging_event = false;
            // Extract android:name="..."
            if let Some(start) = trimmed.find("android:name=\"") {
                let rest = &trimmed[start + 14..];
                if let Some(end) = rest.find('"') {
                    service_name = rest[..end].to_string();
                }
            }
        } else if in_service && trimmed.contains("com.google.firebase.MESSAGING_EVENT") {
            found_messaging_event = true;
        } else if trimmed.starts_with("</service>") {
            if in_service && found_messaging_event && !service_name.is_empty() {
                return Some(service_name);
            }
            in_service = false;
            service_name.clear();
        }
    }

    None
}

/// Convert Java class name to smali file path
fn class_name_to_smali_path(decoded_dir: &Path, class_name: &str) -> PathBuf {
    let smali_dirs = find_smali_dirs(decoded_dir);
    let relative_path = class_name.replace('.', "/") + ".smali";

    // Check each smali directory
    for smali_dir in smali_dirs {
        let path = smali_dir.join(&relative_path);
        if path.exists() {
            return path;
        }
    }

    // Default to first smali dir
    decoded_dir.join("smali").join(&relative_path)
}

/// Find all smali directories (for multi-dex APKs)
pub fn find_smali_dirs(decoded_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![decoded_dir.join("smali")];

    // Look for smali_classes2, smali_classes3, etc.
    for i in 2..=10 {
        let dir = decoded_dir.join(format!("smali_classes{}", i));
        if dir.exists() {
            dirs.push(dir);
        }
    }

    dirs
}

/// Get the next available classes dex number
pub fn get_next_dex_number(decoded_dir: &Path) -> u32 {
    let mut max = 1;

    for i in 2..=20 {
        if decoded_dir.join(format!("smali_classes{}", i)).exists() {
            max = i;
        }
    }

    max + 1
}

/// Analyze FCM integration in an APK
pub fn analyze_fcm_integration(apk_path: &Path) -> Result<()> {
    let temp_dir = std::env::temp_dir().join("fcm2up-analyze");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir)?;

    decode_apk(apk_path, &temp_dir)?;

    println!("\nAnalysis Results:");
    println!("=================\n");

    // Find Firebase service
    if let Some(service_path) = find_firebase_service(&temp_dir)? {
        println!("Firebase Messaging Service found:");
        println!("  {}", service_path.display());

        // Extract class name
        let rel_path = service_path.strip_prefix(temp_dir.join("smali"))?;
        let class_name = rel_path
            .to_str()
            .unwrap()
            .replace('/', ".")
            .trim_end_matches(".smali")
            .to_string();
        println!("  Class: {}", class_name);
    } else {
        println!("No FirebaseMessagingService subclass found.");
        println!("This app may use a different FCM integration pattern.");
    }

    // Check for Firebase dependencies
    let manifest_path = temp_dir.join("AndroidManifest.xml");
    if manifest_path.exists() {
        let manifest = std::fs::read_to_string(&manifest_path)?;

        println!("\nManifest Analysis:");
        if manifest.contains("com.google.firebase") {
            println!("  Firebase components found in manifest");
        }
        if manifest.contains("com.google.android.gms") {
            println!("  Google Play Services components found");
        }
        if manifest.contains("FirebaseMessagingService") {
            println!("  FirebaseMessagingService registered");
        }
    }

    // Count smali directories
    let smali_dirs = find_smali_dirs(&temp_dir);
    println!("\nDEX Structure:");
    println!("  {} smali directories found", smali_dirs.len());
    println!("  Next available: smali_classes{}", get_next_dex_number(&temp_dir));

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_dir);

    Ok(())
}
