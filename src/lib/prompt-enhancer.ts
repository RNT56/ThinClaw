import { commands } from "./bindings";
import { findStyle } from "./style-library";

/**
 * System prompt used for image prompt enhancement.
 * Keep this visible and centralized.
 */
export const IMAGE_PROMPT_ENHANCE_SYSTEM_PROMPT = `Act as a professional prompt engineer for AI image generation. Rewrite the following user prompt to be more vivid, artistic, and detailed. Add keywords for lighting, style, composition, and high quality. {STYLE_SNIPPET}

Keep the output as a single descriptive paragraph under 75 words. Use NO conversational text or headers. Just the enhanced prompt.`;

/**
 * Clean the LLM output to extract just the enhanced prompt.
 */
export function cleanEnhancedPrompt(text: string): string {
    let enhanced = text.trim();
    // Strip common chat template pollution and thinking blocks
    enhanced = enhanced.replace(/<\|im_start\|>.*?<\|im_end\|>/gs, '');
    enhanced = enhanced.replace(/<\|im_start\|>assistant/g, '');
    enhanced = enhanced.replace(/<think>[\s\S]*?<\/think>/g, '');
    enhanced = enhanced.replace(/<\|im_end\|>/g, '');
    return enhanced.trim();
}

/**
 * Unified prompt enhancer that works for both local and cloud LLMs.
 */
export async function enhanceImagePrompt(
    prompt: string,
    styleId?: string,
    onStatusUpdate?: (status: string) => void
): Promise<string> {
    try {
        onStatusUpdate?.("Enhancing prompt...");

        const activeStyle = styleId ? findStyle(styleId) : null;
        const styleSnippet = activeStyle ? `\n\nREQUIRED STYLE: ${activeStyle.promptSnippet}` : "";
        const systemPrompt = IMAGE_PROMPT_ENHANCE_SYSTEM_PROMPT.replace("{STYLE_SNIPPET}", styleSnippet);

        // Use the unified backend command which routes to current local/cloud provider
        const res = await (commands as any).chatCompletion({
            model: "auto", // Backend resolves this
            messages: [
                { role: "system", content: systemPrompt },
                { role: "user", content: prompt }
            ],
            temperature: 0.7,
            top_p: 1.0,
            web_search_enabled: false,
            auto_mode: false
        });

        // Backend returns Result<String, String>, and our bindings might unwrap or return it raw
        // In most cases with specta Result, it's either the string or it throws
        const enhanced = cleanEnhancedPrompt(res);

        if (enhanced.length > 0) {
            console.log("[PromptEnhancer] Cleaned:", enhanced);
            return enhanced;
        }
        return prompt;
    } catch (e) {
        console.warn("Enhancement failed:", e);
        return prompt;
    }
}
