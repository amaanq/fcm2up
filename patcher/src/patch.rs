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
    println!("\n[1/7] Decoding APK...");
    apk::decode_apk(&config.input, &decoded_dir)?;

    // Get package name
    let package_name = extract::extract_package_name(&decoded_dir)?;
    println!("  Package: {}", package_name);

    // Step 2: Extract Firebase credentials
    println!("\n[2/7] Extracting Firebase credentials...");
    let firebase_creds = extract::extract_firebase_credentials_from_decoded(&decoded_dir)?;
    if firebase_creds.app_id.is_some() {
        println!("  App ID: {}", firebase_creds.app_id.as_ref().unwrap());
        println!("  Project: {}", firebase_creds.project_id.as_deref().unwrap_or("unknown"));
        println!("  API Key: {}...", &firebase_creds.api_key.as_deref().unwrap_or("none")[..20.min(firebase_creds.api_key.as_deref().unwrap_or("").len())]);
    } else {
        println!("  Warning: Could not extract Firebase credentials");
        println!("  The bridge may not be able to receive FCM messages");
    }

    // Step 3: Find Firebase messaging service
    println!("\n[3/7] Analyzing FCM integration...");
    let firebase_service = apk::find_firebase_service(&decoded_dir)?;

    if let Some(ref service_path) = firebase_service {
        println!("  Found: {}", service_path.display());
    } else {
        println!("  Warning: No FirebaseMessagingService found");
        println!("  The app may use a different FCM pattern");
    }

    // Step 4: Inject shim DEX
    println!("\n[4/7] Injecting shim...");
    inject_shim_dex(&decoded_dir, config.shim_dex.as_deref())?;

    // Step 5: Patch smali hooks
    println!("\n[5/7] Patching hooks...");
    if let Some(service_path) = firebase_service {
        patch_firebase_service(&service_path)?;
    }
    patch_application_class(&decoded_dir, &config.bridge_url, &config.distributor, &firebase_creds)?;

    // Step 6: Update manifest
    println!("\n[6/7] Updating manifest...");
    let manifest_path = decoded_dir.join("AndroidManifest.xml");
    manifest::remove_split_requirements(&manifest_path)?;
    manifest::add_unifiedpush_receiver(&manifest_path, &package_name)?;

    // Step 7: Build and sign
    println!("\n[7/7] Building APK...");
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

    // Look for pre-generated smali files next to the DEX
    let shim_smali_dir = shim_dex.parent().map(|p| p.join("smali"));

    if let Some(ref smali_dir) = shim_smali_dir {
        if smali_dir.exists() && smali_dir.is_dir() {
            // Copy pre-generated smali files
            println!("  Using pre-generated smali from: {}", smali_dir.display());
            copy_dir_recursive(smali_dir, &target_smali_dir)?;
        } else {
            // Fall back to baksmali
            let status = std::process::Command::new("baksmali")
                .args(["d", "-o"])
                .arg(&target_smali_dir)
                .arg(&shim_dex)
                .status()
                .context("Failed to run baksmali. Is it installed? Or provide pre-generated smali files.")?;

            if !status.success() {
                bail!("baksmali failed to disassemble shim DEX");
            }
        }
    } else {
        bail!("Invalid shim DEX path");
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
fn patch_application_class(
    decoded_dir: &Path,
    bridge_url: &str,
    distributor: &str,
    firebase_creds: &extract::FirebaseCredentials,
) -> Result<()> {
    let manifest_path = decoded_dir.join("AndroidManifest.xml");

    // Find application class
    let app_class = manifest::get_application_class(&manifest_path)?;

    if let Some(class_name) = app_class {
        println!("  Application class: {}", class_name);

        // Convert class name to smali path
        let smali_path = class_name_to_smali_path(decoded_dir, &class_name)?;

        if let Some(path) = smali_path {
            patch_application_on_create(&path, bridge_url, distributor, firebase_creds)?;
        } else {
            println!("  Warning: Could not find Application class smali file");
            create_init_provider(decoded_dir, bridge_url, distributor, firebase_creds)?;
        }
    } else {
        println!("  No custom Application class, using ContentProvider init");
        create_init_provider(decoded_dir, bridge_url, distributor, firebase_creds)?;
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
    firebase_creds: &extract::FirebaseCredentials,
) -> Result<()> {
    let content = fs::read_to_string(smali_path)?;

    // Check if already patched
    if content.contains("Lcom/fcm2up/Fcm2UpShim;->configure") {
        println!("  Application already patched, skipping");
        return Ok(());
    }

    // First, find the current .locals count for onCreate
    let locals_pattern = r"\.method[^\n]*onCreate\(\)V[^\n]*\n\s*\.locals (\d+)";
    let re_locals = Regex::new(locals_pattern)?;
    let current_locals: u32 = re_locals
        .captures(&content)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(4);

    // We need 7 registers for our code (context + 6 string args)
    // Use registers at the end of the range to avoid clobbering
    let base_reg = current_locals;
    let new_locals = current_locals + 7;

    // Get Firebase credential strings (or null placeholders)
    let fb_app_id = firebase_creds.app_id.as_deref().unwrap_or("");
    let fb_project_id = firebase_creds.project_id.as_deref().unwrap_or("");
    let fb_api_key = firebase_creds.api_key.as_deref().unwrap_or("");

    // Generate init code using high registers and invoke-static/range
    // Note: const/4 only works with v0-v15, use const/16 for high registers
    // Configure signature: (Context, bridgeUrl, distributor, firebaseAppId, firebaseProjectId, firebaseApiKey)
    let init_code = format!(
        r#"
    # FCM2UP: Initialize shim with Firebase credentials
    move-object/from16 v{base}, p0
    const-string v{url}, "{bridge_url}"
    const-string v{dist}, "{distributor}"
    const-string v{app_id}, "{fb_app_id}"
    const-string v{proj_id}, "{fb_project_id}"
    const-string v{api_key}, "{fb_api_key}"
    invoke-static/range {{v{base} .. v{api_key}}}, Lcom/fcm2up/Fcm2UpShim;->configure(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V

    # FCM2UP: Register with UnifiedPush
    invoke-static/range {{v{base} .. v{base}}}, Lcom/fcm2up/Fcm2UpShim;->register(Landroid/content/Context;)V
"#,
        base = base_reg,
        url = base_reg + 1,
        dist = base_reg + 2,
        app_id = base_reg + 3,
        proj_id = base_reg + 4,
        api_key = base_reg + 5,
        bridge_url = bridge_url,
        distributor = distributor,
        fb_app_id = fb_app_id,
        fb_project_id = fb_project_id,
        fb_api_key = fb_api_key,
    );

    // Find onCreate and inject after super.onCreate()
    let super_oncreate_pattern = r"(invoke-\w+ \{[^}]*\}, L[^;]+;->onCreate\(\)V)";
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

    // Update .locals count
    let new_content = re_locals
        .replace(&new_content, |caps: &regex::Captures| {
            caps[0].replace(&format!(".locals {}", current_locals), &format!(".locals {}", new_locals))
        })
        .to_string();

    fs::write(smali_path, new_content)?;
    println!("  Injected init code into Application.onCreate (using v{}-v{})", base_reg, base_reg + 5);

    Ok(())
}

/// Create a ContentProvider to initialize fcm2up if no Application class
fn create_init_provider(
    decoded_dir: &Path,
    bridge_url: &str,
    distributor: &str,
    firebase_creds: &extract::FirebaseCredentials,
) -> Result<()> {
    let fb_app_id = firebase_creds.app_id.as_deref().unwrap_or("");
    let fb_project_id = firebase_creds.project_id.as_deref().unwrap_or("");
    let fb_api_key = firebase_creds.api_key.as_deref().unwrap_or("");

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
    .locals 7

    # Get context
    invoke-virtual {{p0}}, Landroid/content/ContentProvider;->getContext()Landroid/content/Context;
    move-result-object v0

    # Configure shim with Firebase credentials
    const-string v1, "{bridge_url}"
    const-string v2, "{distributor}"
    const-string v3, "{fb_app_id}"
    const-string v4, "{fb_project_id}"
    const-string v5, "{fb_api_key}"
    invoke-static/range {{v0 .. v5}}, Lcom/fcm2up/Fcm2UpShim;->configure(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V

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
        bridge_url = bridge_url,
        distributor = distributor,
        fb_app_id = fb_app_id,
        fb_project_id = fb_project_id,
        fb_api_key = fb_api_key,
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

/// Recursively copy a directory
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in WalkDir::new(src) {
        let entry = entry?;
        let src_path = entry.path();
        let relative = src_path.strip_prefix(src)?;
        let dst_path = dst.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dst_path)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(src_path, &dst_path)?;
        }
    }

    Ok(())
}
