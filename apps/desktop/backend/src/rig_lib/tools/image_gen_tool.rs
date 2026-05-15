use crate::image_gen::{direct_media_generate_image as generate_image, ImageGenParams};

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use serde_json::json;
use tauri::Manager;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImageGenError {
    #[error("Image Generation failed: {0}")]
    Generation(String),
}

#[derive(Deserialize)]
pub struct ImageGenArgs {
    pub prompt: String,
    pub negative_prompt: Option<String>,
}

pub struct ImageGenTool {
    pub app: tauri::AppHandle,
}

impl Tool for ImageGenTool {
    const NAME: &'static str = "generate_image";

    type Error = ImageGenError;
    type Args = ImageGenArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "generate_image".to_string(),
            description: "Generate an image using Stable Diffusion based on a text prompt. Use this ONLY when the user explicitly asks to 'generate', 'create', or 'draw' an image.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "The detailed description of the image to generate"
                    },
                    "negative_prompt": {
                        "type": "string",
                        "description": "Optional details to exclude from the image"
                    }
                },
                "required": ["prompt"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let state_sidecar = self.app.state::<crate::sidecar::SidecarManager>();
        let state_config = self.app.state::<crate::config::ConfigManager>();

        // Notify UI
        use tauri::Emitter;
        #[derive(serde::Serialize, Clone, specta::Type)]
        struct WebSearchStatus {
            step: String,
            message: String,
        }
        let _ = self.app.emit(
            "web_search_status",
            WebSearchStatus {
                step: "generating".into(),
                message: "Generating image...".into(),
            },
        );

        let params = ImageGenParams {
            prompt: args.prompt.clone(),
            model: None, // Uses default selected
            vae: None,
            clip_l: None,
            clip_g: None,
            t5xxl: None,
            negative_prompt: args.negative_prompt,
            width: None,
            height: None,
            steps: None,
            cfg_scale: None,
            seed: None,
            schedule: None,
            sampling_method: None,
        };

        match generate_image(self.app.clone(), state_sidecar, state_config, params).await {
            Ok(res) => {
                // Return Markdown image link
                Ok(format!(
                    "![Generated Image]({})\n\n**Generated Image ID:** {}",
                    res.path, res.id
                ))
            }
            Err(e) => Err(ImageGenError::Generation(e)),
        }
    }
}
