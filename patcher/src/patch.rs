//! APK patching logic
//!
//! Orchestrates the complete patching process:
//! 1. Decode APK
//! 2. Inject shim DEX
//! 3. Patch smali hooks
//! 4. Update manifest
//! 5. Build and sign

use crate::{apk, extract, manifest};
use anyhow::{bail, Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Configuration for patching an APK
pub struct PatchConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub bridge_url: String,
    pub distributor: String,
    pub shim_dex: Option<PathBuf>,
    pub keystore: Option<PathBuf>,
    pub keystore_pass: Option<String>,
    pub key_alias: Option<String>,
}

/// Patch an APK for UnifiedPush support
pub fn patch_apk(config: PatchConfig) -> Result<()> {
    // Create temp directory
    let temp_dir = std::env::temp_dir().join("fcm2up-patch");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir)?;

    let decoded_dir = temp_dir.join("decoded");

    // Step 1: Decode APK
    println!("\n[1/6] Decoding APK...");
    apk::decode_apk(&config.input, &decoded_dir)?;

    // Get package name
    let package_name = extract::extract_package_name(&decoded_dir)?;
    println!("  Package: {}", package_name);

    // Step 2: Find Firebase messaging service
    println!("\n[2/6] Analyzing FCM integration...");
    let firebase_service = apk::find_firebase_service(&decoded_dir)?;

    if let Some(ref service_path) = firebase_service {
        println!("  Found: {}", service_path.display());
    } else {
        println!("  Warning: No FirebaseMessagingService found");
        println!("  The app may use a different FCM pattern");
    }

    // Step 3: Inject shim DEX
    println!("\n[3/6] Injecting shim...");
    inject_shim_dex(&decoded_dir, config.shim_dex.as_deref())?;

    // Step 4: Patch smali hooks
    println!("\n[4/6] Patching hooks...");
    if let Some(service_path) = firebase_service {
        patch_firebase_service(&service_path)?;
    }
    patch_application_class(&decoded_dir, &config.bridge_url, &config.distributor)?;

    // Step 5: Update manifest
    println!("\n[5/6] Updating manifest...");
    let manifest_path = decoded_dir.join("AndroidManifest.xml");
    manifest::remove_split_requirements(&manifest_path)?;
    manifest::add_unifiedpush_receiver(&manifest_path, &package_name)?;

    // Step 6: Build and sign
    println!("\n[6/6] Building APK...");
    apk::build_apk(&decoded_dir, &config.output)?;
    apk::zipalign_apk(&config.output)?;
    apk::sign_apk(
        &config.output,
        config.keystore.as_deref(),
        config.keystore_pass.as_deref(),
        config.key_alias.as_deref(),
    )?;

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir);

    println!("\nDone! Patched APK: {}", config.output.display());
    println!("\nNext steps:");
    println!("  1. Install the patched APK on your device");
    println!("  2. Ensure ntfy (or your distributor) is installed");
    println!("  3. Configure your bridge server at: {}", config.bridge_url);

    Ok(())
}

/// Inject the shim DEX into the decoded APK
fn inject_shim_dex(decoded_dir: &Path, shim_dex_path: Option<&Path>) -> Result<()> {
    let next_dex_num = apk::get_next_dex_number(decoded_dir);
    let target_smali_dir = decoded_dir.join(format!("smali_classes{}", next_dex_num));

    fs::create_dir_all(&target_smali_dir)?;

    // Get shim DEX path
    let shim_dex = if let Some(path) = shim_dex_path {
        path.to_path_buf()
    } else {
        // Look for embedded shim in common locations
        let possible_paths = [
            PathBuf::from("fcm2up-shim.dex"),
            PathBuf::from("shim/fcm2up-shim.dex"),
            dirs::data_dir()
                .map(|d| d.join("fcm2up/fcm2up-shim.dex"))
                .unwrap_or_default(),
        ];

        possible_paths
            .into_iter()
            .find(|p| p.exists())
            .context("Shim DEX not found. Specify --shim-dex path or build the shim first.")?
    };

    println!("  Using shim: {}", shim_dex.display());

    // Convert DEX to smali using baksmali
    let status = std::process::Command::new("baksmali")
        .args(["d", "-o"])
        .arg(&target_smali_dir)
        .arg(&shim_dex)
        .status()
        .context("Failed to run baksmali. Is it installed?")?;

    if !status.success() {
        bail!("baksmali failed to disassemble shim DEX");
    }

    // Count injected classes
    let class_count = WalkDir::new(&target_smali_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "smali"))
        .count();

    println!(
        "  Injected {} classes into smali_classes{}",
        class_count, next_dex_num
    );

    Ok(())
}

/// Patch the FirebaseMessagingService to call our shim
fn patch_firebase_service(service_path: &Path) -> Result<()> {
    let content = fs::read_to_string(service_path)?;

    // Find onNewToken method and inject our hook
    let hook_code = r#"
    # FCM2UP: Forward token to shim
    invoke-static {p0, p1}, Lcom/fcm2up/Fcm2UpShim;->onToken(Landroid/content/Context;Ljava/lang/String;)V
"#;

    // Look for onNewToken method
    let on_new_token_pattern = r"\.method[^\n]*onNewToken\(Ljava/lang/String;\)V";
    let re = Regex::new(on_new_token_pattern)?;

    let new_content = if re.is_match(&content) {
        // Find the method body and inject after .locals line
        let locals_pattern = r"(\.method[^\n]*onNewToken\(Ljava/lang/String;\)V[^\n]*\n\s*\.locals \d+)";
        let re_locals = Regex::new(locals_pattern)?;

        if re_locals.is_match(&content) {
            re_locals
                .replace(&content, |caps: &regex::Captures| {
                    format!("{}{}", &caps[1], hook_code)
                })
                .to_string()
        } else {
            println!("  Warning: Could not find .locals in onNewToken, hook may not work");
            content
        }
    } else {
        println!("  Warning: onNewToken method not found in Firebase service");
        content
    };

    fs::write(service_path, new_content)?;
    println!("  Hooked onNewToken in Firebase service");

    Ok(())
}

/// Patch the Application class to initialize fcm2up
fn patch_application_class(decoded_dir: &Path, bridge_url: &str, distributor: &str) -> Result<()> {
    let manifest_path = decoded_dir.join("AndroidManifest.xml");

    // Find application class
    let app_class = manifest::get_application_class(&manifest_path)?;

    if let Some(class_name) = app_class {
        println!("  Application class: {}", class_name);

        // Convert class name to smali path
        let smali_path = class_name_to_smali_path(decoded_dir, &class_name)?;

        if let Some(path) = smali_path {
            patch_application_on_create(&path, bridge_url, distributor)?;
        } else {
            println!("  Warning: Could not find Application class smali file");
            create_init_provider(decoded_dir, bridge_url, distributor)?;
        }
    } else {
        println!("  No custom Application class, using ContentProvider init");
        create_init_provider(decoded_dir, bridge_url, distributor)?;
    }

    Ok(())
}

/// Convert a Java class name to smali file path
fn class_name_to_smali_path(decoded_dir: &Path, class_name: &str) -> Result<Option<PathBuf>> {
    let relative_path = class_name.replace('.', "/") + ".smali";

    // Search in all smali directories
    for smali_dir in apk::find_smali_dirs(decoded_dir) {
        let full_path = smali_dir.join(&relative_path);
        if full_path.exists() {
            return Ok(Some(full_path));
        }
    }

    Ok(None)
}

/// Patch Application.onCreate to initialize fcm2up
fn patch_application_on_create(
    smali_path: &Path,
    bridge_url: &str,
    distributor: &str,
) -> Result<()> {
    let content = fs::read_to_string(smali_path)?;

    // Check if already patched
    if content.contains("Lcom/fcm2up/Fcm2UpShim;->configure") {
        println!("  Application already patched, skipping");
        return Ok(());
    }

    // Inject initialization code into onCreate
    let init_code = format!(
        r#"
    # FCM2UP: Initialize shim
    const-string v0, "{}"
    const-string v1, "{}"
    const/4 v2, 0x0
    const/4 v3, 0x0
    invoke-static {{p0, v0, v1, v2, v3}}, Lcom/fcm2up/Fcm2UpShim;->configure(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V

    # FCM2UP: Register with UnifiedPush
    invoke-static {{p0}}, Lcom/fcm2up/Fcm2UpShim;->register(Landroid/content/Context;)V
"#,
        bridge_url, distributor
    );

    // Find onCreate and inject after super.onCreate()
    let super_oncreate_pattern =
        r"(invoke-\w+ \{[^}]*\}, L[^;]+;->onCreate\(\)V)";
    let re = Regex::new(super_oncreate_pattern)?;

    let new_content = if re.is_match(&content) {
        re.replace(&content, |caps: &regex::Captures| {
            format!("{}{}", &caps[1], init_code)
        })
        .to_string()
    } else {
        // Try to find the start of onCreate method
        let oncreate_start = r"(\.method[^\n]*onCreate\(\)V[^\n]*\n\s*\.locals \d+)";
        let re2 = Regex::new(oncreate_start)?;

        if re2.is_match(&content) {
            re2.replace(&content, |caps: &regex::Captures| {
                format!("{}{}", &caps[1], init_code)
            })
            .to_string()
        } else {
            println!("  Warning: Could not find suitable injection point in onCreate");
            content
        }
    };

    // Ensure we have enough registers
    let new_content = ensure_registers(&new_content, "onCreate", 4)?;

    fs::write(smali_path, new_content)?;
    println!("  Injected init code into Application.onCreate");

    Ok(())
}

/// Ensure a method has at least N registers
fn ensure_registers(content: &str, method_name: &str, min_registers: u32) -> Result<String> {
    let pattern = format!(
        r"(\.method[^\n]*{}\([^\)]*\)[^\n]*\n\s*)\.locals (\d+)",
        regex::escape(method_name)
    );
    let re = Regex::new(&pattern)?;

    let result = re.replace(content, |caps: &regex::Captures| {
        let current: u32 = caps[2].parse().unwrap_or(0);
        let new_count = current.max(min_registers);
        format!("{}.locals {}", &caps[1], new_count)
    });

    Ok(result.to_string())
}

/// Create a ContentProvider to initialize fcm2up if no Application class
fn create_init_provider(decoded_dir: &Path, bridge_url: &str, distributor: &str) -> Result<()> {
    // Create a ContentProvider that initializes on app start
    let provider_smali = format!(
        r#".class public Lcom/fcm2up/Fcm2UpInitProvider;
.super Landroid/content/ContentProvider;
.source "Fcm2UpInitProvider.java"

.method public constructor <init>()V
    .locals 0
    invoke-direct {{p0}}, Landroid/content/ContentProvider;-><init>()V
    return-void
.end method

.method public onCreate()Z
    .locals 4

    # Get context
    invoke-virtual {{p0}}, Landroid/content/ContentProvider;->getContext()Landroid/content/Context;
    move-result-object v0

    # Configure shim
    const-string v1, "{}"
    const-string v2, "{}"
    const/4 v3, 0x0
    invoke-static {{v0, v1, v2, v3, v3}}, Lcom/fcm2up/Fcm2UpShim;->configure(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V

    # Register with UnifiedPush
    invoke-static {{v0}}, Lcom/fcm2up/Fcm2UpShim;->register(Landroid/content/Context;)V

    const/4 v0, 0x1
    return v0
.end method

.method public delete(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I
    .locals 0
    const/4 v0, 0x0
    return v0
.end method

.method public getType(Landroid/net/Uri;)Ljava/lang/String;
    .locals 0
    const/4 v0, 0x0
    return-object v0
.end method

.method public insert(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;
    .locals 0
    const/4 v0, 0x0
    return-object v0
.end method

.method public query(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;
    .locals 0
    const/4 v0, 0x0
    return-object v0
.end method

.method public update(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I
    .locals 0
    const/4 v0, 0x0
    return v0
.end method
"#,
        bridge_url, distributor
    );

    // Find the best smali directory to add it to
    let next_dex = apk::get_next_dex_number(decoded_dir);
    let target_dir = decoded_dir.join(format!("smali_classes{}/com/fcm2up", next_dex));
    fs::create_dir_all(&target_dir)?;

    fs::write(target_dir.join("Fcm2UpInitProvider.smali"), provider_smali)?;

    // Add provider to manifest
    let manifest_path = decoded_dir.join("AndroidManifest.xml");
    let manifest = fs::read_to_string(&manifest_path)?;

    if !manifest.contains("Fcm2UpInitProvider") {
        let package_re = Regex::new(r#"package="([^"]+)""#)?;
        let package_name = package_re
            .captures(&manifest)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str())
            .unwrap_or("com.example");

        let provider_decl = format!(
            r#"
        <provider
            android:name="com.fcm2up.Fcm2UpInitProvider"
            android:authorities="{}.fcm2up.init"
            android:exported="false"
            android:initOrder="9999"/>
    "#,
            package_name
        );

        let new_manifest = manifest.replace("</application>", &format!("{}</application>", provider_decl));
        fs::write(&manifest_path, new_manifest)?;
    }

    println!("  Created init ContentProvider");
    Ok(())
}
