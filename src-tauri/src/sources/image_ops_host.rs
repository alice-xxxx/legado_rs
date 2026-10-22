//! 来源引擎的无状态图片运算，与资源 HTTP、缓存无关。
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{imageops, DynamicImage, ImageFormat, Rgba, RgbaImage};
use serde_json::{json, Value};
use std::io::Cursor;

fn decode(text: &str) -> Result<DynamicImage, String> {
    if text.len() > 90 * 1024 * 1024 {
        return Err("图片数据超过限制".into());
    }
    let bytes = STANDARD.decode(text).map_err(|e| e.to_string())?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    dimensions(image.width(), image.height())?;
    Ok(image)
}
fn dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 16_000_000
    {
        return Err("图片尺寸超过限制".into());
    }
    Ok(())
}
fn png(image: &DynamicImage) -> Result<String, String> {
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(STANDARD.encode(output.into_inner()))
}
fn execute(request: Value) -> Result<Value, String> {
    let field = |name: &str| request[name].as_str().ok_or_else(|| format!("缺少 {name}"));
    let number = |name: &str| request[name].as_i64().ok_or_else(|| format!("缺少 {name}"));
    let op = field("op")?;
    if op == "stitch" {
        let items = request["imagesBase64"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 32)
            .ok_or("拼接数量必须在 1 到 32 之间")?;
        let horizontal = match field("direction")? {
            "h" => true,
            "v" => false,
            _ => return Err("拼接方向必须为 h 或 v".into()),
        };
        let mut images = Vec::new();
        let (mut width, mut height) = (0u32, 0u32);
        for item in items {
            let image = decode(item.as_str().ok_or("图片必须是字符串")?)?;
            width = if horizontal {
                width.checked_add(image.width()).ok_or("宽度溢出")?
            } else {
                width.max(image.width())
            };
            height = if horizontal {
                height.max(image.height())
            } else {
                height.checked_add(image.height()).ok_or("高度溢出")?
            };
            dimensions(width, height)?;
            images.push(image);
        }
        let mut output = RgbaImage::new(width, height);
        let mut offset = 0;
        for image in images {
            imageops::overlay(
                &mut output,
                &image,
                if horizontal { offset as i64 } else { 0 },
                if horizontal { 0 } else { offset as i64 },
            );
            offset += if horizontal {
                image.width()
            } else {
                image.height()
            };
        }
        return Ok(json!({"pngBase64":png(&DynamicImage::ImageRgba8(output))?}));
    }
    let image = decode(field(if op == "decode" {
        "bytesBase64"
    } else {
        "imageBase64"
    })?)?;
    let image = match op {
        "decode" => image,
        "size" => return Ok(json!({"width":image.width(),"height":image.height()})),
        "encode" => {
            let format = field("format")?.to_ascii_lowercase();
            let mut output = Cursor::new(Vec::new());
            if matches!(format.as_str(), "jpg" | "jpeg") {
                let quality = number("quality")?.clamp(1, 100) as u8;
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality)
                    .encode_image(&image.to_rgb8())
                    .map_err(|e| e.to_string())?;
            } else {
                let format = match format.as_str() {
                    "png" => ImageFormat::Png,
                    "webp" => ImageFormat::WebP,
                    "gif" => ImageFormat::Gif,
                    "bmp" => ImageFormat::Bmp,
                    _ => return Err("不支持的图片格式".into()),
                };
                image
                    .write_to(&mut output, format)
                    .map_err(|e| e.to_string())?;
            }
            return Ok(json!({"bytesBase64":STANDARD.encode(output.into_inner())}));
        }
        "crop" => {
            let (x, y, w, h) = (number("x")?, number("y")?, number("w")?, number("h")?);
            if x < 0
                || y < 0
                || w <= 0
                || h <= 0
                || x.checked_add(w).is_none_or(|v| v > image.width() as i64)
                || y.checked_add(h).is_none_or(|v| v > image.height() as i64)
            {
                return Err("裁剪范围超出图片".into());
            }
            image.crop_imm(x as u32, y as u32, w as u32, h as u32)
        }
        "flip" => match field("direction")? {
            "h" => image.fliph(),
            "v" => image.flipv(),
            _ => return Err("翻转方向必须为 h 或 v".into()),
        },
        "rotate" => match number("deg")?.rem_euclid(360) {
            0 => image,
            90 => image.rotate90(),
            180 => image.rotate180(),
            270 => image.rotate270(),
            angle => {
                let radians = (angle as f32).to_radians();
                let (sin, cos) = (radians.sin().abs(), radians.cos().abs());
                dimensions(
                    (image.width() as f32 * cos + image.height() as f32 * sin).ceil() as u32,
                    (image.width() as f32 * sin + image.height() as f32 * cos).ceil() as u32,
                )?;
                DynamicImage::ImageRgba8(
                    imageproc::geometric_transformations::rotate_about_center_no_crop(
                        &image.to_rgba8(),
                        radians,
                        imageproc::geometric_transformations::Interpolation::Bilinear,
                        imageproc::geometric_transformations::Border::Constant(Rgba([0, 0, 0, 0])),
                    ),
                )
            }
        },
        "split" => {
            let (rows, cols) = (number("rows")?, number("cols")?);
            if rows <= 0
                || cols <= 0
                || rows > image.height() as i64
                || cols > image.width() as i64
                || rows.checked_mul(cols).is_none_or(|v| v > 256)
            {
                return Err("无效切分数量".into());
            }
            let (mut items, mut total) = (Vec::new(), 0usize);
            for row in 0..rows as u32 {
                for col in 0..cols as u32 {
                    let (x, y) = (
                        col * image.width() / cols as u32,
                        row * image.height() / rows as u32,
                    );
                    let (right, bottom) = (
                        (col + 1) * image.width() / cols as u32,
                        (row + 1) * image.height() / rows as u32,
                    );
                    let encoded = png(&image.crop_imm(x, y, right - x, bottom - y))?;
                    total += encoded.len();
                    if total > 80 * 1024 * 1024 {
                        return Err("切分结果超过限制".into());
                    }
                    items.push(encoded);
                }
            }
            return Ok(json!({"imagesBase64":items}));
        }
        _ => return Err("未知图片操作".into()),
    };
    dimensions(image.width(), image.height())?;
    Ok(json!({"pngBase64":png(&image)?}))
}
pub(crate) fn execute_json(input: &str) -> String {
    let result = if input.len() > 96 * 1024 * 1024 {
        Err("图片请求超过限制".to_owned())
    } else {
        serde_json::from_str(input)
            .map_err(|e| e.to_string())
            .and_then(execute)
    };
    match result {
        Ok(value) => json!({"ok":true,"value":value}).to_string(),
        Err(error) => json!({"ok":false,"error":error}).to_string(),
    }
}
