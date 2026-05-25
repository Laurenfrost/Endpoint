//! 文字封面生成：在渐变背景上渲染书名 + 作者，输出 1400×2100 PNG。
//!
//! 核心库不做 I/O——调用方负责读取字体字节、把返回的 PNG bytes 写入目标文件。

use std::io::Cursor;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{DynamicImage, ImageFormat, RgbImage};

const WIDTH: u32 = 1400;
const HEIGHT: u32 = 2100;
const MAX_WIDTH_RATIO: f32 = 0.85;

/// 封面背景风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCoverStyle {
    /// 深蓝竖向渐变（#1a2a4a → #0d1b2a）。
    Default,
    /// 蓝紫竖向渐变（#2d1b69 → #11478a）。
    Gradient,
}

/// 生成文字封面所需的全部参数。字体字节由调用方传入，模块本身不做 I/O。
pub struct TextCoverOptions<'a> {
    pub title: &'a str,
    pub author: &'a str,
    /// TTF / OTF 字体的原始字节。
    pub font_bytes: &'a [u8],
    pub style: TextCoverStyle,
}

#[derive(Debug, thiserror::Error)]
pub enum CoverError {
    #[error("字体加载失败: {0}")]
    Font(String),
    #[error("图片编码失败: {0}")]
    Encode(String),
}

/// 生成文字封面，返回 PNG 字节。
pub fn generate(opts: &TextCoverOptions<'_>) -> Result<Vec<u8>, CoverError> {
    let font = FontRef::try_from_slice(opts.font_bytes)
        .map_err(|e| CoverError::Font(e.to_string()))?;

    let mut img = RgbImage::new(WIDTH, HEIGHT);

    fill_gradient(&mut img, opts.style);

    // 书名：108px，y 在画布 40% 处，白色
    draw_text_block(&font, &mut img, opts.title, 108.0, HEIGHT as f32 * 0.40, [255, 255, 255]);
    // 作者：64px，y 在画布 60% 处，浅蓝
    draw_text_block(&font, &mut img, opts.author, 64.0, HEIGHT as f32 * 0.60, [180, 200, 230]);

    let mut buf = Vec::new();
    DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .map_err(|e| CoverError::Encode(e.to_string()))?;

    Ok(buf)
}

// ── 内部实现 ─────────────────────────────────────────────────────────────────

fn fill_gradient(img: &mut RgbImage, style: TextCoverStyle) {
    let (top, bot): ([u8; 3], [u8; 3]) = match style {
        TextCoverStyle::Default => ([0x1a, 0x2a, 0x4a], [0x0d, 0x1b, 0x2a]),
        TextCoverStyle::Gradient => ([0x2d, 0x1b, 0x69], [0x11, 0x47, 0x8a]),
    };
    for y in 0..HEIGHT {
        let t = y as f32 / (HEIGHT - 1) as f32;
        let r = lerp(top[0], bot[0], t);
        let g = lerp(top[1], bot[1], t);
        let b = lerp(top[2], bot[2], t);
        for x in 0..WIDTH {
            img.put_pixel(x, y, image::Rgb([r, g, b]));
        }
    }
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

/// 在 y_center 处居中绘制一段文字（自动换行）。
fn draw_text_block(
    font: &FontRef<'_>,
    img: &mut RgbImage,
    text: &str,
    size: f32,
    y_center: f32,
    color: [u8; 3],
) {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let max_px = WIDTH as f32 * MAX_WIDTH_RATIO;

    // 预计算每个字符的宽度，然后断行
    let char_widths: Vec<(char, f32)> = text
        .chars()
        .map(|c| (c, scaled.h_advance(scaled.glyph_id(c))))
        .collect();

    let lines = wrap_lines(&char_widths, max_px);

    let ascent = scaled.ascent();
    let descent = scaled.descent(); // 负值
    let line_height = ascent - descent;
    let n = lines.len() as f32;
    let block_top = y_center - n * line_height / 2.0;

    for (i, line) in lines.iter().enumerate() {
        let line_width: f32 = line.iter().map(|(_, w)| w).sum();
        let start_x = (WIDTH as f32 - line_width) / 2.0;
        let baseline_y = block_top + ascent + i as f32 * line_height;

        let mut x_cursor = start_x;
        for &(c, advance) in line {
            let gid = scaled.glyph_id(c);
            let glyph = gid.with_scale_and_position(scale, ab_glyph::point(x_cursor, baseline_y));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|x, y, coverage| {
                    let px = bounds.min.x as i32 + x as i32;
                    let py = bounds.min.y as i32 + y as i32;
                    if px >= 0 && py >= 0 && (px as u32) < WIDTH && (py as u32) < HEIGHT {
                        let pixel = img.get_pixel_mut(px as u32, py as u32);
                        let inv = 1.0 - coverage;
                        pixel[0] = (pixel[0] as f32 * inv + color[0] as f32 * coverage) as u8;
                        pixel[1] = (pixel[1] as f32 * inv + color[1] as f32 * coverage) as u8;
                        pixel[2] = (pixel[2] as f32 * inv + color[2] as f32 * coverage) as u8;
                    }
                });
            }
            x_cursor += advance;
        }
    }
}

/// 按字符边界断行，使每行不超过 max_width 像素。
fn wrap_lines(char_widths: &[(char, f32)], max_width: f32) -> Vec<Vec<(char, f32)>> {
    let mut lines: Vec<Vec<(char, f32)>> = Vec::new();
    let mut current: Vec<(char, f32)> = Vec::new();
    let mut current_w = 0.0f32;

    for &(c, w) in char_widths {
        if current_w + w > max_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
        }
        current.push((c, w));
        current_w += w;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new()); // 空文本，给一个空行占位
    }
    lines
}

// ── 单元测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_font_bytes_returns_font_error() {
        let opts = TextCoverOptions {
            title: "书名",
            author: "作者",
            font_bytes: b"this is not a font",
            style: TextCoverStyle::Default,
        };
        assert!(matches!(generate(&opts), Err(CoverError::Font(_))));
    }

    fn find_font_bytes() -> Option<Vec<u8>> {
        let candidates = [
            "src-tauri/resources/fonts/LXGWWenKai-Regular.ttf",
            "../../src-tauri/resources/fonts/LXGWWenKai-Regular.ttf",
        ];
        candidates.iter().find_map(|p| std::fs::read(p).ok())
    }

    #[test]
    fn default_style_generates_png() {
        let Some(bytes) = find_font_bytes() else {
            eprintln!("跳过：字体未找到，运行 fetch-fonts.ps1 后重试");
            return;
        };
        let opts = TextCoverOptions {
            title: "测试书名",
            author: "测试作者",
            font_bytes: &bytes,
            style: TextCoverStyle::Default,
        };
        let png = generate(&opts).expect("Default 风格封面生成应成功");
        assert_eq!(&png[..4], b"\x89PNG", "输出应为 PNG 格式");
    }

    #[test]
    fn gradient_style_generates_png() {
        let Some(bytes) = find_font_bytes() else {
            eprintln!("跳过：字体未找到，运行 fetch-fonts.ps1 后重试");
            return;
        };
        let opts = TextCoverOptions {
            title: "另一本书",
            author: "另一位作者",
            font_bytes: &bytes,
            style: TextCoverStyle::Gradient,
        };
        let png = generate(&opts).expect("Gradient 风格封面生成应成功");
        assert_eq!(&png[..4], b"\x89PNG", "输出应为 PNG 格式");
    }
}
