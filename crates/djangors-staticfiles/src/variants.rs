//! Image resizing and format conversion helpers (`images` feature).

use std::collections::HashMap;
use std::io::Cursor;

use image::imageops::FilterType;
use image::ImageFormat;

use crate::error::StaticFilesError;
use crate::storage::Storage;

/// Target output image format for [`generate_variant`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageOutputFormat {
    /// PNG lossless format.
    Png,
    /// JPEG format.
    Jpeg,
    /// WebP format.
    WebP,
}

impl ImageOutputFormat {
    fn as_image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::WebP => ImageFormat::WebP,
        }
    }

    /// File extension for this output format (without leading dot).
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
        }
    }
}

/// Resize strategy used when generating image variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeFit {
    /// Fit inside `width x height` while preserving aspect ratio.
    Contain,
    /// Fill `width x height` preserving aspect ratio and center-cropping overflow.
    Cover,
    /// Resize to exact `width x height` ignoring aspect ratio.
    Exact,
}

/// Specification for a derived image variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageVariantSpec {
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Target output encoding format.
    pub format: ImageOutputFormat,
    /// Resize fit mode.
    pub fit: ResizeFit,
}

/// Decode `input_bytes`, resize according to `spec`, and encode in the requested format.
pub fn generate_variant(
    input_bytes: &[u8],
    spec: &ImageVariantSpec,
) -> Result<Vec<u8>, StaticFilesError> {
    let img = image::load_from_memory(input_bytes)
        .map_err(|e| StaticFilesError::Image(format!("decode failed: {e}")))?;
    let resized = match spec.fit {
        ResizeFit::Contain => img.resize(spec.width, spec.height, FilterType::Lanczos3),
        ResizeFit::Cover => img.resize_to_fill(spec.width, spec.height, FilterType::Lanczos3),
        ResizeFit::Exact => img.resize_exact(spec.width, spec.height, FilterType::Lanczos3),
    };
    let mut out = Cursor::new(Vec::new());
    resized
        .write_to(&mut out, spec.format.as_image_format())
        .map_err(|e| StaticFilesError::Image(format!("encode failed: {e}")))?;
    Ok(out.into_inner())
}

/// Save `original` at `base_path` plus each named variant under `{stem}_{variant}.{ext}` in `storage`.
///
/// Returns a map from `"original"` and each variant name to the saved storage path.
pub async fn save_with_variants(
    storage: &dyn Storage,
    base_path: &str,
    original: Vec<u8>,
    variants: &[(&str, ImageVariantSpec)],
) -> Result<HashMap<String, String>, StaticFilesError> {
    let mut saved = HashMap::new();
    let orig_key = storage.save(base_path, original.clone()).await?;
    saved.insert("original".to_string(), orig_key);

    let stem = base_path
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(base_path);

    for (name, spec) in variants {
        let bytes = generate_variant(&original, spec)?;
        let variant_path = format!("{stem}_{name}.{}", spec.format.extension());
        let saved_path = storage.save(&variant_path, bytes).await?;
        saved.insert((*name).to_string(), saved_path);
    }

    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LocalDiskStorage;
    use image::{ImageBuffer, Rgba};

    #[tokio::test]
    async fn generates_variants_and_saves_to_storage() {
        let img = ImageBuffer::from_pixel(64, 32, Rgba([242, 120, 34, 255]));
        let mut png_bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut png_bytes, ImageFormat::Png)
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let storage = LocalDiskStorage::new(dir.path(), "/media");
        let saved = save_with_variants(
            &storage,
            "avatars/u1.png",
            png_bytes.into_inner(),
            &[
                (
                    "thumb",
                    ImageVariantSpec {
                        width: 16,
                        height: 16,
                        format: ImageOutputFormat::WebP,
                        fit: ResizeFit::Cover,
                    },
                ),
                (
                    "small",
                    ImageVariantSpec {
                        width: 32,
                        height: 32,
                        format: ImageOutputFormat::Jpeg,
                        fit: ResizeFit::Contain,
                    },
                ),
            ],
        )
        .await
        .unwrap();

        assert_eq!(saved.get("original").unwrap(), "avatars/u1.png");
        assert_eq!(saved.get("thumb").unwrap(), "avatars/u1_thumb.webp");
        assert_eq!(saved.get("small").unwrap(), "avatars/u1_small.jpg");
        assert!(storage.exists("avatars/u1_thumb.webp").await.unwrap());
    }
}
