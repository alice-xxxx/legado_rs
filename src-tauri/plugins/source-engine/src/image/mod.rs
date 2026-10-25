//! 有界图片处理；原生参数在入口解析为确定的操作类型。
use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage, imageops};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Cursor;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Request {
    Decode {
        bytes_base64: String,
    },
    Size {
        image_base64: String,
    },
    Encode {
        image_base64: String,
        format: String,
        quality: u8,
    },
    Crop {
        image_base64: String,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
    Flip {
        image_base64: String,
        direction: Direction,
    },
    Rotate {
        image_base64: String,
        deg: i32,
    },
    Split {
        image_base64: String,
        rows: u32,
        cols: u32,
    },
    Stitch {
        images_base64: Vec<String>,
        direction: Direction,
    },
}
#[derive(Deserialize, Clone, Copy)]
enum Direction {
    #[serde(rename = "h")]
    Horizontal,
    #[serde(rename = "v")]
    Vertical,
}
fn bounds(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 || w > 16384 || h > 16384 || u64::from(w) * u64::from(h) > 16_000_000 {
        Err("图片尺寸超限".into())
    } else {
        Ok(())
    }
}
fn decode(data: &str) -> Result<DynamicImage> {
    if data.len() > 90 * 1024 * 1024 {
        return Err("图片数据超限".into());
    }
    let bytes = STANDARD.decode(data).map_err(|e| e.to_string())?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    bounds(image.width(), image.height())?;
    Ok(image)
}
fn png(image: &DynamicImage) -> Result<String> {
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(STANDARD.encode(out.into_inner()))
}
pub(crate) fn native(input: &str) -> String {
    let result = if input.len() > 96 * 1024 * 1024 {
        Err("图片请求超限".into())
    } else {
        serde_json::from_str(input)
            .map_err(|e| e.to_string())
            .and_then(process)
    };
    crate::ffi::response(result)
}
fn process(request: Request) -> Result<Value> {
    let image = match request {
        Request::Decode { bytes_base64 } => decode(&bytes_base64)?,
        Request::Size { image_base64 } => {
            let image = decode(&image_base64)?;
            return Ok(json!({"width":image.width(),"height":image.height()}));
        }
        Request::Encode {
            image_base64,
            format,
            quality,
        } => {
            let image = decode(&image_base64)?;
            let mut out = Cursor::new(Vec::new());
            if matches!(format.to_ascii_lowercase().as_str(), "jpg" | "jpeg") {
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                    .encode_image(&image.to_rgb8())
                    .map_err(|e| e.to_string())?;
            } else {
                let format = match format.to_ascii_lowercase().as_str() {
                    "png" => ImageFormat::Png,
                    "webp" => ImageFormat::WebP,
                    "gif" => ImageFormat::Gif,
                    "bmp" => ImageFormat::Bmp,
                    _ => return Err("图片格式无效".into()),
                };
                image
                    .write_to(&mut out, format)
                    .map_err(|e| e.to_string())?;
            }
            return Ok(json!({"bytesBase64":STANDARD.encode(out.into_inner())}));
        }
        Request::Crop {
            image_base64,
            x,
            y,
            w,
            h,
        } => {
            let image = decode(&image_base64)?;
            bounds(w, h)?;
            if x.checked_add(w).is_none_or(|v| v > image.width())
                || y.checked_add(h).is_none_or(|v| v > image.height())
            {
                return Err("裁剪范围无效".into());
            }
            image.crop_imm(x, y, w, h)
        }
        Request::Flip {
            image_base64,
            direction,
        } => {
            let image = decode(&image_base64)?;
            match direction {
                Direction::Horizontal => image.fliph(),
                Direction::Vertical => image.flipv(),
            }
        }
        Request::Rotate { image_base64, deg } => {
            let image = decode(&image_base64)?;
            match deg.rem_euclid(360) {
                0 => image,
                90 => image.rotate90(),
                180 => image.rotate180(),
                270 => image.rotate270(),
                angle => {
                    let radians = (angle as f32).to_radians();
                    let (sin, cos) = (radians.sin().abs(), radians.cos().abs());
                    bounds(
                        (image.width() as f32 * cos + image.height() as f32 * sin).ceil() as u32,
                        (image.width() as f32 * sin + image.height() as f32 * cos).ceil() as u32,
                    )?;
                    DynamicImage::ImageRgba8(
                        imageproc::geometric_transformations::rotate_about_center_no_crop(
                            &image.to_rgba8(),
                            radians,
                            imageproc::geometric_transformations::Interpolation::Bilinear,
                            imageproc::geometric_transformations::Border::Constant(Rgba([
                                0, 0, 0, 0,
                            ])),
                        ),
                    )
                }
            }
        }
        Request::Split {
            image_base64,
            rows,
            cols,
        } => {
            let image = decode(&image_base64)?;
            if rows == 0
                || cols == 0
                || rows > image.height()
                || cols > image.width()
                || rows.checked_mul(cols).is_none_or(|v| v > 256)
            {
                return Err("切分数量无效".into());
            }
            let mut parts = Vec::new();
            let mut total = 0;
            for r in 0..rows {
                for c in 0..cols {
                    let (x, y) = (c * image.width() / cols, r * image.height() / rows);
                    let part = png(&image.crop_imm(
                        x,
                        y,
                        (c + 1) * image.width() / cols - x,
                        (r + 1) * image.height() / rows - y,
                    ))?;
                    total += part.len();
                    if total > 80 * 1024 * 1024 {
                        return Err("切分结果超限".into());
                    }
                    parts.push(part);
                }
            }
            return Ok(json!({"imagesBase64":parts}));
        }
        Request::Stitch {
            images_base64,
            direction,
        } => {
            if images_base64.is_empty() || images_base64.len() > 32 {
                return Err("拼接数量无效".into());
            }
            let horizontal = matches!(direction, Direction::Horizontal);
            let (mut w, mut h) = (0u32, 0u32);
            let mut images = Vec::new();
            for data in images_base64 {
                let image = decode(&data)?;
                if horizontal {
                    w = w.checked_add(image.width()).ok_or("宽度溢出")?;
                    h = h.max(image.height());
                } else {
                    w = w.max(image.width());
                    h = h.checked_add(image.height()).ok_or("高度溢出")?;
                }
                bounds(w, h)?;
                images.push(image);
            }
            let mut output = RgbaImage::new(w, h);
            let mut offset = 0;
            for image in images {
                imageops::overlay(
                    &mut output,
                    &image,
                    if horizontal { offset } else { 0 },
                    if horizontal { 0 } else { offset },
                );
                offset += if horizontal {
                    image.width()
                } else {
                    image.height()
                } as i64;
            }
            DynamicImage::ImageRgba8(output)
        }
    };
    Ok(json!({"pngBase64":png(&image)?}))
}
