import { useChatLayout } from '../ChatProvider';
import { ImagineGeneration, ImagineGallery } from '../../imagine';
import { convertFileSrc } from '@tauri-apps/api/core';

export function ImagineView() {
    const {
        activeImagineTab,
        setActiveImagineTab,
        imagineGenerating,
        generationProgress,
        lastGeneratedImage,
        setLastGeneratedImage,
        handleImagineGenerate,
    } = useChatLayout();

    return (
        <>
            {activeImagineTab === 'generate' ? (
                <ImagineGeneration
                    isGenerating={imagineGenerating}
                    progress={generationProgress}
                    lastGeneratedImage={lastGeneratedImage}
                    onGenerate={async (prompt, options) => {
                        await handleImagineGenerate(prompt, {
                            provider: options.provider as 'local' | 'nano-banana' | 'nano-banana-pro',
                            aspectRatio: options.aspectRatio,
                            resolution: options.resolution,
                            styleId: options.styleId,
                            sourceImages: options.sourceImages,
                            steps: options.steps,
                        });
                    }}
                />
            ) : (
                <ImagineGallery
                    onImageSelect={(image) => {
                        setLastGeneratedImage(convertFileSrc(image.filePath));
                        setActiveImagineTab('generate');
                    }}
                />
            )}
        </>
    );
}
