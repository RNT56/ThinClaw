import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { toast } from 'sonner';
import {
    Search, Grid3X3, LayoutGrid,
    Download, Trash2, Copy, Heart,
    Calendar, Image as ImageIcon, X, RefreshCw
} from 'lucide-react';
import { ImagineMainIcon, ImagineModeIcon } from '../icons/ModeIcons';
import { cn } from '../../lib/utils';
import {
    imagineListImages,
    imagineDeleteImage,
    imagineToggleFavorite,
    imagineSearchImages,
    type GeneratedImage
} from '../../lib/imagine';
import { convertFileSrc } from '@tauri-apps/api/core';
import { downloadImageToDisk } from '../../lib/fs-utils';

interface ImagineGalleryProps {
    onImageSelect?: (image: GeneratedImage) => void;
    onRefresh?: () => void;
}

export function ImagineGallery({
    onImageSelect: _onImageSelect,
    onRefresh: _onRefresh
}: ImagineGalleryProps) {
    const [images, setImages] = useState<GeneratedImage[]>([]);
    const [loading, setLoading] = useState(true);
    const [searchQuery, setSearchQuery] = useState('');
    const [gridSize, setGridSize] = useState<'small' | 'large'>('small');
    const [showFavoritesOnly, setShowFavoritesOnly] = useState(false);
    const [selectedImages, setSelectedImages] = useState<Set<string>>(new Set());
    const [previewImage, setPreviewImage] = useState<GeneratedImage | null>(null);

    // Load images from backend
    const loadImages = useCallback(async () => {
        setLoading(true);
        try {
            const result = await imagineListImages(100, 0, showFavoritesOnly);
            setImages(result);
        } catch (err) {
            console.error('Failed to load images:', err);
        } finally {
            setLoading(false);
        }
    }, [showFavoritesOnly]);

    useEffect(() => {
        loadImages();
    }, [loadImages]);

    // Search images
    const handleSearch = useCallback(async () => {
        if (!searchQuery.trim()) {
            loadImages();
            return;
        }

        setLoading(true);
        try {
            const result = await imagineSearchImages(searchQuery);
            setImages(result);
        } catch (err) {
            console.error('Failed to search images:', err);
        } finally {
            setLoading(false);
        }
    }, [searchQuery, loadImages]);

    useEffect(() => {
        const debounce = setTimeout(() => {
            handleSearch();
        }, 300);
        return () => clearTimeout(debounce);
    }, [searchQuery, handleSearch]);

    // Delete image
    const handleDelete = async (id: string) => {
        try {
            await imagineDeleteImage(id);
            setImages(prev => prev.filter(img => img.id !== id));
            setSelectedImages(prev => {
                const newSet = new Set(prev);
                newSet.delete(id);
                return newSet;
            });
        } catch (err) {
            console.error('Failed to delete image:', err);
        }
    };

    // Toggle favorite
    const handleToggleFavorite = async (id: string, e: React.MouseEvent) => {
        e.stopPropagation();
        try {
            const newStatus = await imagineToggleFavorite(id);
            setImages(prev => prev.map(img =>
                img.id === id ? { ...img, isFavorite: newStatus } : img
            ));
        } catch (err) {
            console.error('Failed to toggle favorite:', err);
        }
    };

    // Download image
    const handleDownload = async (image: GeneratedImage) => {
        try {
            const assetUrl = convertFileSrc(image.filePath);
            const filename = `${image.prompt.slice(0, 50).replace(/[^a-z0-9]/gi, '_')}_${image.id.slice(0, 6)}.png`;

            await downloadImageToDisk(assetUrl, filename);

            // Show toast only for single downloads (bulk has its own toast)
            if (selectedImages.size === 0) {
                toast.success(`Saved to Downloads`);
            }
        } catch (err) {
            console.error('Failed to download image:', err);
            throw err; // Re-throw for bulk error handling
        }
    };

    // Copy image to clipboard
    const handleCopy = async (image: GeneratedImage) => {
        try {
            const imgSrc = convertFileSrc(image.filePath);

            // Create ClipboardItem with a Promise to preserve the user gesture
            const item = new ClipboardItem({
                'image/png': (async () => {
                    const response = await fetch(imgSrc);
                    const blob = await response.blob();

                    const img = await createImageBitmap(blob);
                    const canvas = document.createElement('canvas');
                    canvas.width = img.width;
                    canvas.height = img.height;
                    const ctx = canvas.getContext('2d');
                    if (!ctx) throw new Error('Failed to get canvas context');

                    ctx.drawImage(img, 0, 0);

                    return new Promise<Blob>((resolve, reject) => {
                        canvas.toBlob(b => {
                            if (b) resolve(b);
                            else reject(new Error('Canvas to Blob failed'));
                        }, 'image/png');
                    });
                })()
            });

            await navigator.clipboard.write([item]);
            toast.success("Copied to clipboard");
        } catch (err) {
            console.error('Failed to copy image:', err);
            toast.error(`Failed to copy: ${err instanceof Error ? err.message : String(err)}`);
        }
    };

    const toggleSelect = (id: string, e: React.MouseEvent) => {
        e.stopPropagation();
        const newSet = new Set(selectedImages);
        if (newSet.has(id)) {
            newSet.delete(id);
        } else {
            newSet.add(id);
        }
        setSelectedImages(newSet);
    };

    // Get image URL using Tauri's file protocol
    const getImageUrl = (filePath: string) => {
        return convertFileSrc(filePath);
    };

    return (
        <div className="flex-1 flex flex-col h-full">
            {/* Header */}
            <div className="p-6 border-b border-border/50">
                <div className="flex items-center justify-between mb-4">
                    <div>
                        <h2 className="text-xl font-bold text-foreground">Gallery</h2>
                        <p className="text-sm text-muted-foreground">
                            {images.length} creation{images.length !== 1 ? 's' : ''}
                        </p>
                    </div>

                    <div className="flex items-center gap-2">
                        {/* Refresh button */}
                        <button
                            onClick={loadImages}
                            className="p-2 rounded-lg text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
                        >
                            <RefreshCw className={cn("w-4 h-4", loading && "animate-spin")} />
                        </button>

                    </div>
                </div>

                {/* Search and Controls */}
                <div className="flex items-center gap-3">
                    {/* Search */}
                    <div className="flex-1 relative">
                        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
                        <input
                            type="text"
                            value={searchQuery}
                            onChange={(e) => setSearchQuery(e.target.value)}
                            placeholder="Search by prompt..."
                            className="w-full pl-10 pr-4 py-2 rounded-lg bg-muted/50 border border-border/50 text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/20 focus:border-primary/50 transition-all"
                        />
                    </div>

                    {/* Grid Size Toggle */}
                    <div className="flex rounded-lg bg-muted/50 border border-border/50 p-1">
                        <button
                            onClick={() => setGridSize('large')}
                            className={cn(
                                "p-2 rounded-md transition-colors",
                                gridSize === 'large' ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground"
                            )}
                        >
                            <LayoutGrid className="w-4 h-4" />
                        </button>
                        <button
                            onClick={() => setGridSize('small')}
                            className={cn(
                                "p-2 rounded-md transition-colors",
                                gridSize === 'small' ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground"
                            )}
                        >
                            <Grid3X3 className="w-4 h-4" />
                        </button>
                    </div>

                    {/* Favorites Filter */}
                    <button
                        onClick={() => setShowFavoritesOnly(!showFavoritesOnly)}
                        className={cn(
                            "flex items-center gap-2 px-3 py-2 rounded-lg border transition-colors",
                            showFavoritesOnly
                                ? "bg-primary/10 border-primary/30 text-primary"
                                : "bg-muted/50 border-border/50 text-muted-foreground hover:text-foreground"
                        )}
                    >
                        <Heart className={cn("w-4 h-4", showFavoritesOnly && "fill-current")} />
                        <span className="text-sm">Favorites</span>
                    </button>
                </div>
            </div>

            {/* Gallery Grid */}
            <div className="flex-1 overflow-y-auto p-6">
                {loading && images.length === 0 ? (
                    <div className="flex items-center justify-center h-full">
                        <ImagineModeIcon size={48} isActive={true} />
                    </div>
                ) : images.length === 0 ? (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        className="flex flex-col items-center justify-center h-full text-center"
                    >
                        <div className="w-20 h-20 rounded-2xl bg-gradient-to-br from-primary/10 to-primary/5 flex items-center justify-center mb-4">
                            <ImagineMainIcon size={40} isActive={false} />
                        </div>
                        <h3 className="text-lg font-medium text-muted-foreground mb-1">No images yet</h3>
                        <p className="text-sm text-muted-foreground/70">
                            {searchQuery ? 'No images match your search' : 'Start creating to fill your gallery'}
                        </p>
                    </motion.div>
                ) : (
                    <motion.div
                        className={cn(
                            "grid gap-4",
                            gridSize === 'large' ? "grid-cols-2 lg:grid-cols-3" : "grid-cols-3 lg:grid-cols-4 xl:grid-cols-5"
                        )}
                        layout
                    >
                        <AnimatePresence>
                            {images.map((image, index) => (
                                <motion.div
                                    key={image.id}
                                    layout
                                    initial={{ opacity: 0, scale: 0.9 }}
                                    animate={{ opacity: 1, scale: 1 }}
                                    exit={{ opacity: 0, scale: 0.9 }}
                                    transition={{ delay: index * 0.02 }}
                                    className={cn(
                                        "group relative rounded-xl overflow-hidden cursor-pointer",
                                        "ring-2 ring-transparent transition-all duration-200",
                                        selectedImages.has(image.id) && "ring-primary"
                                    )}
                                    onClick={(e) => {
                                        // If in bulk mode (at least one selected), click toggles selection.
                                        // Otherwise, open preview.
                                        if (selectedImages.size > 0) {
                                            toggleSelect(image.id, e);
                                        } else {
                                            setPreviewImage(image);
                                        }
                                    }}
                                >
                                    {/* Image */}
                                    <div className={cn(
                                        "aspect-square bg-muted overflow-hidden",
                                        gridSize === 'large' && "aspect-[4/3]"
                                    )}>
                                        <img
                                            src={getImageUrl(image.filePath)}
                                            alt={image.prompt}
                                            className="w-full h-full object-cover transition-transform duration-300 group-hover:scale-105"
                                            loading="lazy"
                                        />
                                    </div>

                                    {/* Favorite indicator */}
                                    {image.isFavorite && (
                                        <div className="absolute top-2 right-2">
                                            <Heart className="w-4 h-4 text-primary fill-current" />
                                        </div>
                                    )}

                                    {/* Selection checkbox */}
                                    <button
                                        onClick={(e) => toggleSelect(image.id, e)}
                                        className={cn(
                                            "absolute top-2 left-2 w-6 h-6 rounded-md border-2 transition-all duration-200",
                                            "flex items-center justify-center z-20",
                                            selectedImages.has(image.id)
                                                ? "bg-primary border-primary text-primary-foreground"
                                                : "bg-black/40 border-white/50 opacity-0 group-hover:opacity-100"
                                        )}
                                    >
                                        {selectedImages.has(image.id) && '✓'}
                                    </button>

                                    {/* Hover overlay */}
                                    <div className="absolute inset-0 bg-gradient-to-t from-black/70 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity duration-200">
                                        <div className="absolute bottom-0 left-0 right-0 p-3">
                                            <p className="text-white text-xs line-clamp-2 mb-2">{image.prompt}</p>
                                            <div className="flex items-center justify-between">
                                                <div className="flex items-center gap-1 text-white/70 text-[10px]">
                                                    <Calendar className="w-3 h-3" />
                                                    <span>{new Date(image.createdAt).toLocaleDateString()}</span>
                                                </div>
                                                <div className="flex items-center gap-1">
                                                    <button
                                                        onClick={(e) => handleToggleFavorite(image.id, e)}
                                                        className="p-1.5 rounded-md bg-white/20 text-white hover:bg-white/30 transition-colors"
                                                    >
                                                        <Heart className={cn("w-3 h-3", image.isFavorite && "fill-current text-primary")} />
                                                    </button>
                                                    <button
                                                        onClick={(e) => {
                                                            e.stopPropagation();
                                                            handleDownload(image);
                                                        }}
                                                        className="p-1.5 rounded-md bg-white/20 text-white hover:bg-white/30 transition-colors"
                                                    >
                                                        <Download className="w-3 h-3" />
                                                    </button>
                                                    <button
                                                        onClick={(e) => {
                                                            e.stopPropagation();
                                                            handleDelete(image.id);
                                                        }}
                                                        className="p-1.5 rounded-md bg-white/20 text-white hover:bg-red-500/50 transition-colors"
                                                    >
                                                        <Trash2 className="w-3 h-3" />
                                                    </button>
                                                </div>
                                            </div>
                                        </div>
                                    </div>
                                </motion.div>
                            ))}
                        </AnimatePresence>
                    </motion.div>
                )}
            </div>

            {/* Image Preview Modal */}
            <AnimatePresence>
                {previewImage && (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="fixed inset-0 z-50 bg-black/90 flex items-center justify-center p-8"
                        onClick={() => setPreviewImage(null)}
                    >
                        <motion.div
                            initial={{ scale: 0.9, opacity: 0 }}
                            animate={{ scale: 1, opacity: 1 }}
                            exit={{ scale: 0.9, opacity: 0 }}
                            className="relative max-w-4xl max-h-full"
                            onClick={(e) => e.stopPropagation()}
                        >
                            <img
                                src={getImageUrl(previewImage.filePath)}
                                alt={previewImage.prompt}
                                className="max-w-full max-h-[80vh] rounded-xl shadow-2xl"
                            />

                            {/* Close button */}
                            <button
                                onClick={() => setPreviewImage(null)}
                                className="absolute -top-4 -right-4 p-2 rounded-full bg-white/10 text-white hover:bg-white/20 transition-colors"
                            >
                                <X className="w-5 h-5" />
                            </button>

                            {/* Image info */}
                            <div className="absolute bottom-0 left-0 right-0 p-6 bg-gradient-to-t from-black/80 to-transparent rounded-b-xl">
                                <p className="text-white mb-2">{previewImage.prompt}</p>
                                <div className="flex items-center gap-4 text-white/60 text-sm">
                                    <div className="flex items-center gap-1">
                                        <ImagineModeIcon size={16} isActive={true} />
                                        <span>{previewImage.provider}</span>
                                    </div>
                                    <div className="flex items-center gap-1">
                                        <Calendar className="w-4 h-4" />
                                        <span>{new Date(previewImage.createdAt).toLocaleString()}</span>
                                    </div>
                                    {previewImage.width && previewImage.height && (
                                        <div className="flex items-center gap-1">
                                            <ImageIcon className="w-4 h-4" />
                                            <span>{previewImage.width} × {previewImage.height}</span>
                                        </div>
                                    )}
                                </div>

                                {/* Actions */}
                                <div className="flex gap-2 mt-4">
                                    <button
                                        onClick={(e) => handleToggleFavorite(previewImage.id, e)}
                                        className={cn(
                                            "flex items-center gap-2 px-4 py-2 rounded-lg transition-colors",
                                            previewImage.isFavorite
                                                ? "bg-primary/20 text-primary"
                                                : "bg-white/10 text-white hover:bg-white/20"
                                        )}
                                    >
                                        <Heart className={cn("w-4 h-4", previewImage.isFavorite && "fill-current")} />
                                        <span>{previewImage.isFavorite ? 'Favorited' : 'Favorite'}</span>
                                    </button>
                                    <button
                                        onClick={() => handleDownload(previewImage)}
                                        className="flex items-center gap-2 px-4 py-2 rounded-lg bg-white/10 text-white hover:bg-white/20 transition-colors"
                                    >
                                        <Download className="w-4 h-4" />
                                        <span>Download</span>
                                    </button>
                                    <button
                                        onClick={() => handleCopy(previewImage)}
                                        className="flex items-center gap-2 px-4 py-2 rounded-lg bg-white/10 text-white hover:bg-white/20 transition-colors"
                                    >
                                        <Copy className="w-4 h-4" />
                                        <span>Copy</span>
                                    </button>
                                    <button
                                        onClick={() => {
                                            handleDelete(previewImage.id);
                                            setPreviewImage(null);
                                        }}
                                        className="flex items-center gap-2 px-4 py-2 rounded-lg bg-red-500/20 text-red-400 hover:bg-red-500/30 transition-colors"
                                    >
                                        <Trash2 className="w-4 h-4" />
                                        <span>Delete</span>
                                    </button>
                                </div>
                            </div>
                        </motion.div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Floating Bulk Action Bar */}
            <AnimatePresence>
                {selectedImages.size > 0 && (
                    <motion.div
                        initial={{ opacity: 0, y: 50, scale: 0.95 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 50, scale: 0.95 }}
                        className="fixed bottom-8 left-1/2 -translate-x-1/2 z-40 bg-card/80 backdrop-blur-xl border border-primary/20 shadow-2xl rounded-2xl px-2 py-2 flex items-center gap-2 max-w-[90vw] overflow-hidden ring-1 ring-black/5"
                    >
                        {/* Counter Badge */}
                        <div className="flex items-center gap-2 px-3 py-1.5 bg-primary/10 rounded-xl mr-2">
                            <span className="text-xs font-bold text-primary uppercase tracking-wider">
                                {selectedImages.size} Selected
                            </span>
                        </div>

                        {/* Divider */}
                        <div className="w-px h-6 bg-border/50 mx-1" />

                        {/* Clear Selection */}
                        <button
                            onClick={() => setSelectedImages(new Set())}
                            className="flex flex-col items-center justify-center w-12 h-12 rounded-xl text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-all gap-0.5 group"
                            title="Clear Selection"
                        >
                            <X className="w-5 h-5 group-hover:scale-110 transition-transform" />
                            <span className="text-[9px] font-medium opacity-70">Clear</span>
                        </button>

                        {/* Copy Prompts */}
                        <button
                            onClick={async () => {
                                const selected = images.filter(img => selectedImages.has(img.id));
                                const prompts = selected.map(img => img.prompt).join('\n\n');
                                try {
                                    await navigator.clipboard.writeText(prompts);
                                    toast.success(`Copied ${selected.length} prompts to clipboard`);
                                    setSelectedImages(new Set());
                                } catch (e) {
                                    toast.error("Failed to copy prompts");
                                }
                            }}
                            className="flex flex-col items-center justify-center w-12 h-12 rounded-xl text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-all gap-0.5 group"
                            title="Copy Prompts"
                        >
                            <Copy className="w-5 h-5 group-hover:scale-110 transition-transform" />
                            <span className="text-[9px] font-medium opacity-70">Copy</span>
                        </button>

                        {/* Download Selected */}
                        <button
                            onClick={async () => {
                                const selected = images.filter(img => selectedImages.has(img.id));
                                let successCount = 0;

                                toast.promise((async () => {
                                    for (const img of selected) {
                                        try {
                                            await handleDownload(img);
                                            successCount++;
                                            // Small delay to prevent system overload
                                            await new Promise(r => setTimeout(r, 200));
                                        } catch (e) {
                                            console.error(`Failed to download ${img.id}`, e);
                                        }
                                    }
                                    // Return summary for the success message
                                    if (successCount === 0) throw new Error("No images downloaded");
                                    return `Saved ${successCount} images`;
                                })(), {
                                    loading: `Downloading ${selected.length} images...`,
                                    success: (data) => {
                                        setSelectedImages(new Set());
                                        return data;
                                    },
                                    error: "Failed to complete downloads"
                                });
                            }}
                            className="flex flex-col items-center justify-center w-12 h-12 rounded-xl text-muted-foreground hover:text-primary hover:bg-primary/10 transition-all gap-0.5 group"
                            title="Download Selected"
                        >
                            <Download className="w-5 h-5 group-hover:scale-110 transition-transform" />
                            <span className="text-[9px] font-medium opacity-70">Save</span>
                        </button>

                        {/* Delete Selected */}
                        <button
                            onClick={async () => {
                                if (confirm(`Are you sure you want to delete ${selectedImages.size} images?`)) {
                                    const selectedIds = Array.from(selectedImages);
                                    toast.promise(Promise.all(selectedIds.map(id => handleDelete(id))), {
                                        loading: "Deleting images...",
                                        success: () => {
                                            setSelectedImages(new Set());
                                            return "Images deleted";
                                        },
                                        error: "Failed to delete images"
                                    });
                                }
                            }}
                            className="flex flex-col items-center justify-center w-12 h-12 rounded-xl text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition-all gap-0.5 group"
                            title="Delete Selected"
                        >
                            <Trash2 className="w-5 h-5 group-hover:scale-110 transition-transform" />
                            <span className="text-[9px] font-medium opacity-70">Delete</span>
                        </button>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
