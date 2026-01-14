//! fcm2up-patcher - Patch Android APKs for UnifiedPush support
//!
//! This tool patches Android applications to use UnifiedPush instead of FCM.
//! It injects a Kotlin shim library and hooks the app's Firebase messaging service.

mod apk;
mod extract;
mod manifest;
mod patch;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "fcm2up")]
#[command(about = "Patch Android APKs for UnifiedPush support")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Patch an APK for UnifiedPush support
    Patch {
        /// Input APK file
        #[arg(short, long)]
        input: PathBuf,

        /// Output APK file (default: <input>-patched.apk)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Bridge server URL
        #[arg(short, long, default_value = "https://fcm-bridge.example.com")]
        bridge_url: String,

        /// UnifiedPush distributor package
        #[arg(short, long, default_value = "io.heckel.ntfy")]
        distributor: String,

        /// Path to pre-built shim DEX (optional, uses embedded if not specified)
        #[arg(long)]
        shim_dex: Option<PathBuf>,

        /// Keystore for signing (optional, uses debug key if not specified)
        #[arg(long)]
        keystore: Option<PathBuf>,

        /// Keystore password
        #[arg(long)]
        keystore_pass: Option<String>,

        /// Key alias
        #[arg(long)]
        key_alias: Option<String>,
    },

    /// Extract Firebase credentials from an APK (for analysis)
    Extract {
        /// Input APK file
        #[arg(short, long)]
        input: PathBuf,
    },

    /// Analyze an APK's FCM integration
    Analyze {
        /// Input APK file
        #[arg(short, long)]
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Patch {
            input,
            output,
            bridge_url,
            distributor,
            shim_dex,
            keystore,
            keystore_pass,
            key_alias,
        } => {
            let output = output.unwrap_or_else(|| {
                let stem = input.file_stem().unwrap().to_str().unwrap();
                input.with_file_name(format!("{}-patched.apk", stem))
            });

            println!("Patching APK: {}", input.display());
            println!("Output: {}", output.display());
            println!("Bridge URL: {}", bridge_url);
            println!("Distributor: {}", distributor);

            let config = patch::PatchConfig {
                input,
                output,
                bridge_url,
                distributor,
                shim_dex,
                keystore,
                keystore_pass,
                key_alias,
            };

            patch::patch_apk(config)?;
        }

        Commands::Extract { input } => {
            println!("Extracting Firebase credentials from: {}", input.display());
            let creds = extract::extract_firebase_credentials(&input)?;
            println!("{}", serde_json::to_string_pretty(&creds)?);
        }

        Commands::Analyze { input } => {
            println!("Analyzing FCM integration in: {}", input.display());
            apk::analyze_fcm_integration(&input)?;
        }
    }

    Ok(())
}
