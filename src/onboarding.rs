use crate::config::Config;
use dialoguer::{theme::ColorfulTheme, Input, Password, Select};
use std::io::Result;

/// Run the interactive onboarding wizard to generate a config.json file.
pub fn run_wizard() -> Result<()> {
    println!("Welcome to Ultraclaw! 🦀");
    println!("Let's get you set up with a configuration file.\n");

    let theme = ColorfulTheme::default();
    
    // Load existing config or default
    let mut config = Config::load().unwrap_or_default();

    loop {
        let choices = vec![
            "Configure Chat Platform (Discord/Telegram)",
            "Configure AI Brain (LLM)",
            "Configure Media Providers",
            "Save & Exit"
        ];

        let selection = Select::with_theme(&theme)
            .with_prompt("Main Menu")
            .default(0)
            .items(&choices)
            .interact()
            .map_err(map_err)?;

        match selection {
            0 => configure_chat_platform(&mut config, &theme)?,
            1 => configure_llm(&mut config, &theme)?,
            2 => configure_media(&mut config, &theme)?,
            3 => break,
            _ => unreachable!(),
        }
    }

    // --- Save ---
    println!("\nSaving configuration to config.json...");
    if let Err(e) = config.save() {
        eprintln!("Error saving config: {}", e);
        return Ok(());
    }
    println!("Configuration saved successfully! settings will be loaded on next startup.");
    Ok(())
}

fn map_err(e: dialoguer::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, format!("Dialoguer error: {}", e))
}

fn configure_chat_platform(_config: &mut Config, _theme: &ColorfulTheme) -> Result<()> {
    println!("\n--- Chat Platform Configuration ---");
    println!("Matrix support has been removed per user instructions.");
    println!("Please configure Discord, Telegram, or Webhooks via environment variables or config.json directly.");
    Ok(())
}

fn configure_llm(config: &mut Config, theme: &ColorfulTheme) -> Result<()> {
    println!("\n--- LLM Configuration ---");
    let llm_types = vec!["Cloud (OpenAI, Anthropic, etc.)", "Local (Ollama, LM Studio, etc.)"];
    let llm_selection = Select::with_theme(theme)
        .with_prompt("Select your primary LLM backend")
        .default(0)
        .items(&llm_types)
        .interact().map_err(map_err)?;

    if llm_selection == 0 {
        // Cloud
        config.cloud_base_url = Input::with_theme(theme)
            .with_prompt("API Base URL")
            .default("https://api.openai.com/v1".to_string())
            .interact_text().map_err(map_err)?;
        
        config.cloud_api_key = Password::with_theme(theme)
            .with_prompt("API Key")
            .interact().map_err(map_err)?;

        config.cloud_model = Input::with_theme(theme)
            .with_prompt("Model Name")
            .default("gpt-4o".to_string())
            .interact_text().map_err(map_err)?;
    } else {
        // Local
        println!("Configuring for local OpenAI-compatible endpoint (e.g. Ollama).");
        config.cloud_base_url = Input::with_theme(theme)
            .with_prompt("Local API Base URL")
            .default("http://localhost:11434/v1".to_string())
            .interact_text().map_err(map_err)?;
        
        // Local usually doesn't need a key, but some do
        config.cloud_api_key = Input::with_theme(theme)
            .with_prompt("API Key (leave empty if not needed)")
            .default("sk-dummy".to_string()) 
            .interact_text().map_err(map_err)?;

        config.cloud_model = Input::with_theme(theme)
            .with_prompt("Model Name")
            .default("llama3".to_string())
            .interact_text().map_err(map_err)?;
    }
    Ok(())
}

fn configure_media(config: &mut Config, theme: &ColorfulTheme) -> Result<()> {
    println!("\n--- Media Configuration ---");
    let options = vec![
        "Set Preferred Image Provider",
        "Set Preferred Video Provider",
        "Configure API Keys",
        "Back to Main Menu"
    ];

    loop {
        let selection = Select::with_theme(theme)
            .with_prompt("Media Settings")
            .items(&options)
            .default(0)
            .interact().map_err(map_err)?;

        match selection {
            0 => {
                let img_providers = vec!["openai", "stability", "fal", "replicate", "google", "leonardo", "together"];
                let img_sel = Select::with_theme(theme)
                    .with_prompt("Preferred Image Provider")
                    .items(&img_providers)
                    .default(0)
                    .interact().map_err(map_err)?;
                config.media_image_provider = img_providers[img_sel].to_string();
            }
            1 => {
                let vid_providers = vec!["runway", "luma", "kling", "veo", "minimax", "fal", "replicate"];
                let vid_sel = Select::with_theme(theme)
                    .with_prompt("Preferred Video Provider")
                    .items(&vid_providers)
                    .default(0)
                    .interact().map_err(map_err)?;
                config.media_video_provider = vid_providers[vid_sel].to_string();
            }
            2 => {
                 configure_media_keys(config, theme)?;
            }
            3 => break,
            _ => unreachable!(),
        }
    }
    Ok(())
}

fn configure_media_keys(config: &mut Config, theme: &ColorfulTheme) -> Result<()> {
    let providers = vec![
        "OpenAI", "Stability", "Runway", "Replicate", "Fal.ai", 
        "Google (Imagen/Veo)", "Leonardo", "Together", "Luma", 
        "Kling", "Minimax", "Back"
    ];
    
    loop {
        let sel = Select::with_theme(theme)
            .with_prompt("Select Provider to Configure Key")
            .items(&providers)
            .interact().map_err(map_err)?;

        if sel == providers.len() - 1 { break; } // Back

        match sel {
            0 => { config.cloud_api_key = Password::with_theme(theme).with_prompt("OpenAI API Key").interact().map_err(map_err)?; },
            1 => { config.stability_api_key = Password::with_theme(theme).with_prompt("Stability API Key").interact().map_err(map_err)?; },
            2 => { config.runway_api_key = Password::with_theme(theme).with_prompt("Runway API Key").interact().map_err(map_err)?; },
            3 => { config.replicate_api_key = Password::with_theme(theme).with_prompt("Replicate API Key").interact().map_err(map_err)?; },
            4 => { config.fal_api_key = Password::with_theme(theme).with_prompt("Fal.ai API Key").interact().map_err(map_err)?; },
            5 => { 
                let key = Password::with_theme(theme).with_prompt("Google API Key").interact().map_err(map_err)?; 
                config.imagen_api_key = key.clone();
                config.veo_api_key = key;
            },
            6 => { config.leonardo_api_key = Password::with_theme(theme).with_prompt("Leonardo API Key").interact().map_err(map_err)?; },
            7 => { config.together_api_key = Password::with_theme(theme).with_prompt("Together API Key").interact().map_err(map_err)?; },
            8 => { config.luma_api_key = Password::with_theme(theme).with_prompt("Luma API Key").interact().map_err(map_err)?; },
            9 => { config.kling_api_key = Password::with_theme(theme).with_prompt("Kling API Key").interact().map_err(map_err)?; },
            10 => { config.minimax_api_key = Password::with_theme(theme).with_prompt("Minimax API Key").interact().map_err(map_err)?; },
            _ => {}
        }
    }
    Ok(())
}
