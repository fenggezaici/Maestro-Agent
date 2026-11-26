//! 工具结果图像化的媒体管线：解码 → 1568px 长边缩放 → 重编码 → data URL。
//! 几何语义与前端 M3 压缩管线一致（同一长边）；编码策略按 alpha 分流——
//! 无透明通道走 JPEG q85（截图/照片类体积最优），含透明走 PNG 保 alpha。
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{DynamicImage, ImageReader};
use std::io::Cursor;

/// 与前端 attachments.ts IMAGE_MAX_EDGE 同一值——两侧几何语义必须一致，
/// 否则同一张图在用户附件与工具结果两条路径的 token 代价不同。
pub const IMAGE_MAX_EDGE: u32 = 1568;
/// 原文件硬上限——超出直接拒绝（解码炸弹与 token 双重防护）。
pub const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024;
/// 重编码后上限。约束来自 base64 膨胀：OpenAI wire 层对 data URL 字符串有
/// 3.5MB 过滤，2MiB 二进制 → 2.8MB 字符串留足余量（1568px JPEG q85 实际
/// 多在 100–500KB，此上限仅是深层兜底）。
pub const MAX_ENCODED_BYTES: usize = 2 * 1024 * 1024;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

/// 扩展名初筛（magic bytes 由 image crate 在解码时二次核验——
/// 扩展名只决定"要不要走图像路径"，不作格式信任依据）。
pub fn is_image_path(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub struct ProcessedImage {
    /// `data:image/...;base64,...`——与 ChatMessage.images 全域格式一致。
    pub data_url: String,
    pub width: u32,
    pub height: u32,
    pub encoded_bytes: usize,
    pub media_type: &'static str,
}

/// 非视觉模型的降级元数据：只读 header 不解码全图。
pub fn metadata(bytes: &[u8]) -> Result<String, String> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "无法识别图像格式".to_string())?;
    let format = reader.format().map(|f| format!("{f:?}")).unwrap_or_else(|| "未知".into());
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| format!("无法读取图像尺寸：{error}"))?;
    Ok(format!(
        "[图像文件：{format} {width}×{height}，{:.1} KiB。当前模型不支持视觉输入，图像内容未注入——如需分析请切换视觉模型。]",
        bytes.len() as f64 / 1024.0
    ))
}

pub fn process(bytes: &[u8]) -> Result<ProcessedImage, String> {
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(format!(
            "图像原文件 {:.1} MiB 超过 20 MiB 上限，未注入",
            bytes.len() as f64 / 1024.0 / 1024.0
        ));
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "无法识别图像格式（magic bytes 与已知类型不匹配）".to_string())?;
    // load 走 ImageReader 默认资源限制——解压炸弹在解码边界被拒。
    let decoded = reader.decode().map_err(|error| format!("图像解码失败：{error}"))?;
    let (mut width, mut height) = (decoded.width(), decoded.height());
    let has_alpha = decoded.color().has_alpha();
    let scaled: DynamicImage = if width.max(height) > IMAGE_MAX_EDGE {
        let out = decoded.thumbnail(IMAGE_MAX_EDGE, IMAGE_MAX_EDGE);
        width = out.width();
        height = out.height();
        out
    } else {
        decoded
    };
    let (encoded, media_type) = if has_alpha {
        let mut buf = Vec::new();
        scaled
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|error| format!("PNG 编码失败：{error}"))?;
        (buf, "image/png")
    } else {
        let rgb = scaled.to_rgb8();
        let mut buf = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85);
        encoder
            .encode_image(&rgb)
            .map_err(|error| format!("JPEG 编码失败：{error}"))?;
        (buf, "image/jpeg")
    };
    if encoded.len() > MAX_ENCODED_BYTES {
        return Err(format!(
            "缩放后仍 {:.1} MiB 超过 2 MiB 上限（{}×{} 含 alpha={has_alpha}），未注入",
            encoded.len() as f64 / 1024.0 / 1024.0,
            width,
            height
        ));
    }
    Ok(ProcessedImage {
        data_url: format!("data:{media_type};base64,{}", STANDARD.encode(&encoded)),
        width,
        height,
        encoded_bytes: encoded.len(),
        media_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_png(width: u32, height: u32, alpha: bool) -> Vec<u8> {
        let mut buf = Vec::new();
        if alpha {
            let img = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 128]));
            img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        } else {
            let img = image::RgbImage::from_pixel(width, height, image::Rgb([200, 100, 50]));
            img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        }
        buf
    }

    #[test]
    fn opaque_scales_to_max_edge_as_jpeg() {
        let png = make_png(3000, 2000, false);
        let out = process(&png).unwrap();
        assert_eq!(out.width, IMAGE_MAX_EDGE);
        assert_eq!(out.height, 1045); // 2000 * 1568/3000 取整
        assert_eq!(out.media_type, "image/jpeg");
        assert!(out.data_url.starts_with("data:image/jpeg;base64,"));
        assert!(out.encoded_bytes < png.len() || out.encoded_bytes < MAX_ENCODED_BYTES);
    }

    #[test]
    fn alpha_encodes_as_png() {
        let png = make_png(100, 80, true);
        let out = process(&png).unwrap();
        assert_eq!(out.media_type, "image/png");
        // 小图不缩放
        assert_eq!((out.width, out.height), (100, 80));
    }

    #[test]
    fn small_image_passes_without_resize() {
        let png = make_png(800, 600, false);
        let out = process(&png).unwrap();
        assert_eq!((out.width, out.height), (800, 600));
    }

    #[test]
    fn non_image_bytes_fail_with_honest_error() {
        assert!(process(b"this is not an image").is_err());
    }

    #[test]
    fn metadata_reports_format_and_dimensions() {
        let png = make_png(640, 480, false);
        let text = metadata(&png).unwrap();
        assert!(text.contains("640×480"), "{text}");
        assert!(text.contains("不支持视觉"), "{text}");
    }

    #[test]
    fn extension_screening() {
        assert!(is_image_path("a/b/shot.PNG"));
        assert!(is_image_path("x.jpeg"));
        assert!(is_image_path("x.webp"));
        assert!(!is_image_path("x.rs"));
        assert!(!is_image_path("noext"));
    }
}
