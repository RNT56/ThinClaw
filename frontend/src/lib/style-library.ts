export interface StyleDefinition {
    id: string;
    label: string;
    description: string;
    promptSnippet: string;
}

export const STYLE_LIBRARY: StyleDefinition[] = [
    {
        id: "cyberpunk",
        label: "Cyberpunk",
        description: "Neon-drenched, futuristic, gritty tech aesthetic.",
        promptSnippet: "Cyberpunk style, neon lights, rainy futuristic streets, high-tech low-life, blade runner aesthetic, cinematic lighting, glowing circuitry, hyper-detailed cityscapes."
    },
    {
        id: "cypherpunk",
        label: "Cypherpunk",
        description: "Privacy-focused, encryption, and open-source rebellion.",
        promptSnippet: "Cypherpunk aesthetic, privacy-focused tech, encryption symbols, PGP keys, dark terminal screens with green code, hidden servers, utilitarian hardware, anti-surveillance gear, high-contrast gritty look, 1990s hacker culture."
    },
    {
        id: "meme",
        label: "Internet Meme",
        description: "Classic low-res internet image with Impact font.",
        promptSnippet: "Low-resolution internet meme style, white Impact font text with black outline at top and bottom, slightly compressed image quality, relatable funny content look."
    },
    {
        id: "cctv",
        label: "CCTV / Security",
        description: "Grainy security camera footage with timestamp.",
        promptSnippet: "CCTV security camera footage, grainy black and white or muted colors, digital timestamp overlay, fish-eye lens distortion, high-angle perspective, security monitor aesthetic."
    },
    {
        id: "photorealistic",
        label: "Photorealistic",
        description: "Stunningly life-like professional photography.",
        promptSnippet: "Hyper-photorealistic, shot on 85mm lens, extremely detailed textures, natural lighting, high dynamic range, sharp focus, professional color grading, 8k resolution."
    },
    {
        id: "isometric",
        label: "Isometric 3D",
        description: "Clean, miniature diorama perspective.",
        promptSnippet: "Isometric 3D perspective, miniature diorama style, clean 3D art, soft ambient lighting, vibrant colors, tilt-shift effect, high-quality rendered toy-like world."
    },
    {
        id: "natgeo",
        label: "Nature Documentary",
        description: "Vivid, high-end wildlife photography.",
        promptSnippet: "National Geographic style photography, high-end telephoto lens, stunning nature detail, natural sunlight, vivid realistic colors, magazine quality documentary shot."
    },
    {
        id: "renaissance",
        label: "Renaissance",
        description: "Classical oil painting with dramatic chiaroscuro.",
        promptSnippet: "Renaissance oil painting style, chiaroscuro lighting, dramatic shadows, fine brushwork, sfumato technique, classical art, masterpiece, museum quality."
    },
    {
        id: "unreal",
        label: "Unreal Engine",
        description: "High-end 3D render, hyper-realistic gaming look.",
        promptSnippet: "Hyper-realistic 3D render, Unreal Engine 5 style, 8k resolution, raytracing, global illumination, intricate textures, Octane render, cinematic composition."
    },
    {
        id: "ghibli",
        label: "Studio Ghibli",
        description: "Lush, hand-drawn anime with watercolor vibes.",
        promptSnippet: "Studio Ghibli anime style, hand-drawn look, lush colorful landscapes, watercolor textures, whimsical atmosphere, Hayao Miyazaki aesthetic, high quality animation art."
    },
    {
        id: "cinematic",
        label: "Cinematic",
        description: "Professional movie shot with shallow depth of field.",
        promptSnippet: "Cinematic movie shot, 35mm lens, anamorphic lens flare, shallow depth of field, bokeh, professional color grading, film grain, dramatic atmosphere."
    },
    {
        id: "claymation",
        label: "Claymation",
        description: "Tactile, stop-motion animation style.",
        promptSnippet: "Claymation style, stop-motion animation, tactile clay textures, fingerprints in clay, plasticine art, handcrafted look, vibrant colors, soft studio lighting."
    },
    {
        id: "blueprint",
        label: "Blueprint",
        description: "Technical architectural or engineering drawing.",
        promptSnippet: "Technical blueprint style, architectural drawing, white lines on blue background, engineering schematic, detailed measurements, drafting paper texture, clean lines."
    },
    {
        id: "monochrome",
        label: "Monochrome",
        description: "High-contrast, dramatic black and white.",
        promptSnippet: "High-contrast monochrome photography, dramatic black and white, deep shadows, bright highlights, film noir aesthetic, grainy texture, silhouetted shapes."
    },
    {
        id: "vaporwave",
        label: "Vaporwave",
        description: "Retro 80s, pink and teal, glitchy aesthetic.",
        promptSnippet: "Vaporwave aesthetic, retro-futurism, 1980s style, neon pink and teal color palette, glitch art, digital sunset, lo-fi textures, aesthetic statues."
    },
    {
        id: "pixelart",
        label: "Pixel Art",
        description: "Retro 16-bit gaming aesthetic.",
        promptSnippet: "High-quality pixel art, 16-bit retro gaming style, sharp pixels, vibrant colors, sprites, classic video game aesthetic, clean lines."
    },
    {
        id: "dystopian",
        label: "Dystopian",
        description: "Bleak, industrial, post-apocalyptic world.",
        promptSnippet: "Dystopian post-apocalyptic style, bleak atmosphere, rusted industrial decay, muted colors, fog, desolate landscapes, moody lighting, worn textures."
    },
    {
        id: "fantasy",
        label: "Epic Fantasy",
        description: "Magical, ethereal, lord of the rings style.",
        promptSnippet: "Epic fantasy art, magical atmosphere, ethereal lighting, grand scale, mystical landscapes, intricate detailed armor and robes, digital illustration masterpiece."
    },
    {
        id: "gta",
        label: "GTA Loading Screen",
        description: "Iconic saturated, cell-shaded illustration style.",
        promptSnippet: "GTA V loading screen art style, stylized digital illustration, cell-shaded, high-contrast, vibrant saturated colors, thick outlines, iconic video game cover art aesthetic."
    },
    {
        id: "steampunk",
        label: "Steampunk",
        description: "Victorian brass technology and steam power.",
        promptSnippet: "Steampunk aesthetic, victorian era technology, brass and copper machinery, gears, steam-powered gadgets, sepia tones, intricate mechanical details, clockwork art."
    },
    {
        id: "papercut",
        label: "Papercut Art",
        description: "3D layered papercraft with depth and shadows.",
        promptSnippet: "Layered papercut art style, 3D papercraft, shadows between layers of cut paper, colorful cutout shapes, handcrafted tactile feel, minimalist depth."
    },
    {
        id: "doubleexposure",
        label: "Double Exposure",
        description: "Artistic blend of two images in one silhouette.",
        promptSnippet: "Double exposure photography effect, two distinct images blended into one silhouette, ethereal atmosphere, artistic overlay, surreal composition, nature textures."
    }
];

export function findStyle(id: string): StyleDefinition | undefined {
    return STYLE_LIBRARY.find(s => s.id === id.toLowerCase());
}
