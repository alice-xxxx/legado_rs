//! Stateless Rust pixel operations used by the existing KMP `ImageOps` provider.
//!
//! The rule engine still decides when and how to transform an image. This module
//! only implements the pixel primitives, exchanging bounded PNG/base64 values
//! over the platform bridge so native image handles cannot leak across scripts.

use std::io::Cursor;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use image::imageops;
use image::{DynamicImage, ImageFormat, ImageReader, Limits, Rgba, RgbaImage};
use imageproc::geometric_transformations::{rotate_about_center_no_crop, Border, Interpolation};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_RPC_BYTES: usize = 96 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_BATCH_BASE64_BYTES: usize = 80 * 1024 * 1024;
const MAX_SPLIT_TILES: usize = 256;
const MAX_STITCH_IMAGES: usize = 32;

#[derive(Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ImageRequest {
    Decode {
        bytes_base64: String,
    },
    Encode {
        image_base64: String,
        format: String,
        quality: i32,
    },
    Split {
        image_base64: String,
        rows: u32,
        cols: u32,
    },
    Stitch {
        images_base64: Vec<String>,
        direction: String,
    },
    Crop {
        image_base64: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Rotate {
        image_base64: String,
        deg: i32,
    },
    Flip {
        image_base64: String,
        direction: String,
    },
    Size {
        image_base64: String,
    },
}

/// Handle the compact JSON protocol used by JVM/JNI and Kotlin/Native C ABI.
/// Errors are returned as plain diagnostic strings; request pixels and source
/// script text are never logged.
pub(crate) fn execute_json(input: &str) -> String {
    let result = if request_exceeds_limit(input.len()) {
        Err("Image request exceeds the 96 MiB transport limit".to_owned())
    } else {
        serde_json::from_str::<ImageRequest>(input)
            .map_err(|_| "Invalid image operation request".to_owned())
            .and_then(execute)
    };
    match result {
        Ok(value) => json!({ "ok": true, "value": value }).to_string(),
        Err(error) => json!({ "ok": false, "error": error }).to_string(),
    }
}

fn request_exceeds_limit(size_bytes: usize) -> bool {
    size_bytes > MAX_RPC_BYTES
}

fn execute(request: ImageRequest) -> Result<Value, String> {
    match request {
        ImageRequest::Decode { bytes_base64 } => {
            let bytes = decode_base64(&bytes_base64, "image.decode input")?;
            let image = decode_image(&bytes)?;
            Ok(json!({ "pngBase64": encode_png_base64(&image)? }))
        }
        ImageRequest::Encode {
            image_base64,
            format,
            quality,
        } => {
            let image = decode_internal_image(&image_base64)?;
            let bytes = encode_image(&image, &format, quality)?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err("Encoded image exceeds the 64 MiB limit".to_owned());
            }
            Ok(json!({ "bytesBase64": BASE64.encode(bytes) }))
        }
        ImageRequest::Split {
            image_base64,
            rows,
            cols,
        } => {
            let image = decode_internal_image(&image_base64)?;
            let rows = usize::try_from(rows).map_err(|_| "Invalid split row count".to_owned())?;
            let cols =
                usize::try_from(cols).map_err(|_| "Invalid split column count".to_owned())?;
            if rows == 0 || cols == 0 {
                return Err("image.split rows and columns must be positive".to_owned());
            }
            if rows
                .checked_mul(cols)
                .is_none_or(|tiles| tiles > MAX_SPLIT_TILES)
            {
                return Err("image.split supports at most 256 tiles".to_owned());
            }
            let (width, height) = image.dimensions();
            if cols > width as usize || rows > height as usize {
                return Err("image.split tile count exceeds image dimensions".to_owned());
            }
            let cell_width = width / cols as u32;
            let cell_height = height / rows as u32;
            let mut tiles = Vec::with_capacity(rows * cols);
            let mut encoded_total = 0usize;
            for row in 0..rows {
                for col in 0..cols {
                    let x = col as u32 * cell_width;
                    let y = row as u32 * cell_height;
                    let tile_width = if col + 1 == cols {
                        width - x
                    } else {
                        cell_width
                    };
                    let tile_height = if row + 1 == rows {
                        height - y
                    } else {
                        cell_height
                    };
                    let tile = imageops::crop_imm(&image, x, y, tile_width, tile_height).to_image();
                    let encoded = encode_png_base64(&tile)?;
                    check_batch_payload(&mut encoded_total, encoded.len())?;
                    tiles.push(encoded);
                }
            }
            Ok(json!({ "imagesBase64": tiles }))
        }
        ImageRequest::Stitch {
            images_base64,
            direction,
        } => {
            if images_base64.is_empty() {
                return Err("image.stitch requires at least one image".to_owned());
            }
            if images_base64.len() > MAX_STITCH_IMAGES {
                return Err("image.stitch supports at most 32 images".to_owned());
            }
            let horizontal = match direction.to_ascii_lowercase().as_str() {
                "h" => true,
                "v" => false,
                _ => return Err("image.stitch direction must be 'h' or 'v'".to_owned()),
            };
            let mut images = Vec::with_capacity(images_base64.len());
            let (mut width, mut height) = (0u32, 0u32);
            for encoded in images_base64 {
                let bytes = decode_base64(&encoded, "image.stitch input")?;
                let (image_width, image_height) = image_dimensions(&bytes)?;
                let next_width = if horizontal {
                    width.checked_add(image_width)
                } else {
                    Some(width.max(image_width))
                }
                .ok_or_else(|| "image.stitch output width overflows".to_owned())?;
                let next_height = if horizontal {
                    Some(height.max(image_height))
                } else {
                    height.checked_add(image_height)
                }
                .ok_or_else(|| "image.stitch output height overflows".to_owned())?;
                // Check output dimensions before decoding and retaining this
                // image. The output area bounds the sum of accepted pixels.
                check_dimensions(next_width, next_height)?;
                images.push(decode_image(&bytes)?);
                width = next_width;
                height = next_height;
            }
            let mut result = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
            let mut offset = 0u32;
            for image in images {
                imageops::overlay(
                    &mut result,
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
            Ok(json!({ "pngBase64": encode_png_base64(&result)? }))
        }
        ImageRequest::Crop {
            image_base64,
            x,
            y,
            w,
            h,
        } => {
            let image = decode_internal_image(&image_base64)?;
            if x < 0 || y < 0 || w <= 0 || h <= 0 {
                return Err("image.crop coordinates and dimensions are invalid".to_owned());
            }
            let (x, y, w, h) = (x as u32, y as u32, w as u32, h as u32);
            if x.checked_add(w).is_none_or(|right| right > image.width())
                || y.checked_add(h)
                    .is_none_or(|bottom| bottom > image.height())
            {
                return Err("image.crop area exceeds image dimensions".to_owned());
            }
            let cropped = imageops::crop_imm(&image, x, y, w, h).to_image();
            Ok(json!({ "pngBase64": encode_png_base64(&cropped)? }))
        }
        ImageRequest::Rotate { image_base64, deg } => {
            let image = decode_internal_image(&image_base64)?;
            let angle = deg % 360;
            let rotated = match angle {
                0 => image,
                90 => imageops::rotate90(&image),
                180 => imageops::rotate180(&image),
                270 | -90 => imageops::rotate270(&image),
                -180 => imageops::rotate180(&image),
                -270 => imageops::rotate90(&image),
                _ => rotate_arbitrary(&image, angle)?,
            };
            Ok(json!({ "pngBase64": encode_png_base64(&rotated)? }))
        }
        ImageRequest::Flip {
            image_base64,
            direction,
        } => {
            let image = decode_internal_image(&image_base64)?;
            let result = match direction.to_ascii_lowercase().as_str() {
                "h" => imageops::flip_horizontal(&image),
                "v" => imageops::flip_vertical(&image),
                _ => return Err("image.flip direction must be 'h' or 'v'".to_owned()),
            };
            Ok(json!({ "pngBase64": encode_png_base64(&result)? }))
        }
        ImageRequest::Size { image_base64 } => {
            let image = decode_internal_image(&image_base64)?;
            Ok(json!({ "w": image.width(), "h": image.height() }))
        }
    }
}

fn decode_image(bytes: &[u8]) -> Result<RgbaImage, String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("Image input exceeds the 64 MiB limit".to_owned());
    }
    if bytes.is_empty() {
        return Err("Image input is empty".to_owned());
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Image format could not be detected".to_owned())?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 4);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "Image bytes could not be decoded".to_owned())?;
    check_dimensions(decoded.width(), decoded.height())?;
    Ok(decoded.to_rgba8())
}

fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Image format could not be detected".to_owned())?;
    let dimensions = reader
        .into_dimensions()
        .map_err(|_| "Image dimensions could not be read".to_owned())?;
    check_dimensions(dimensions.0, dimensions.1)?;
    Ok(dimensions)
}

fn decode_internal_image(encoded: &str) -> Result<RgbaImage, String> {
    let bytes = decode_base64(encoded, "internal image")?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("Internal image exceeds the 64 MiB limit".to_owned());
    }
    let image = decode_image(&bytes)?;
    Ok(image)
}

fn decode_base64(encoded: &str, label: &str) -> Result<Vec<u8>, String> {
    if encoded.len() > MAX_IMAGE_BYTES.saturating_mul(4).div_ceil(3) + 8 {
        return Err(format!("{label} exceeds the 64 MiB limit"));
    }
    BASE64
        .decode(encoded)
        .map_err(|_| format!("{label} is not valid base64"))
}

fn encode_png_base64(image: &RgbaImage) -> Result<String, String> {
    check_dimensions(image.width(), image.height())?;
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|_| "PNG encoding failed".to_owned())?;
    let bytes = encoded.into_inner();
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("PNG image exceeds the 64 MiB transport limit".to_owned());
    }
    Ok(BASE64.encode(bytes))
}

fn check_batch_payload(total: &mut usize, next: usize) -> Result<(), String> {
    let Some(combined) = total.checked_add(next) else {
        return Err("Image batch output exceeds its transport limit".to_owned());
    };
    if combined > MAX_BATCH_BASE64_BYTES {
        return Err("Image batch output exceeds the 80 MiB JSON limit".to_owned());
    }
    *total = combined;
    Ok(())
}

fn encode_image(image: &RgbaImage, format: &str, quality: i32) -> Result<Vec<u8>, String> {
    match format.to_ascii_lowercase().as_str() {
        "png" => {
            let mut cursor = Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(image.clone())
                .write_to(&mut cursor, ImageFormat::Png)
                .map_err(|_| "PNG encoding failed".to_owned())?;
            Ok(cursor.into_inner())
        }
        "jpg" | "jpeg" => {
            let mut rgb = image::RgbImage::new(image.width(), image.height());
            for (x, y, pixel) in image.enumerate_pixels() {
                let [red, green, blue, alpha] = pixel.0;
                let matte = |channel: u8| ((u16::from(channel) * u16::from(alpha)) / 255) as u8;
                rgb.put_pixel(x, y, image::Rgb([matte(red), matte(green), matte(blue)]));
            }
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut bytes,
                quality.clamp(0, 100) as u8,
            )
            .encode_image(&DynamicImage::ImageRgb8(rgb))
            .map_err(|_| "JPEG encoding failed".to_owned())?;
            Ok(bytes)
        }
        "webp" => {
            let encoded = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
                .encode(quality.clamp(0, 100) as f32);
            Ok(encoded.to_vec())
        }
        _ => Err("image.encode format must be png, jpg, jpeg, or webp".to_owned()),
    }
}

fn rotate_arbitrary(image: &RgbaImage, angle_degrees: i32) -> Result<RgbaImage, String> {
    let radians = (angle_degrees as f32).to_radians();
    let cos = radians.cos().abs();
    let sin = radians.sin().abs();
    // Match imageproc's own f32 bbox calculation exactly so a rounding edge
    // cannot make our cap precheck disagree with the allocated output size.
    let width = (image.height() as f32 * sin + image.width() as f32 * cos).ceil();
    let height = (image.height() as f32 * cos + image.width() as f32 * sin).ceil();
    if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 {
        return Err("image.rotate produced invalid dimensions".to_owned());
    }
    let width = width as u32;
    let height = height as u32;
    check_dimensions(width, height)?;
    let transparent = image.pixels().any(|pixel| pixel.0[3] < 255);
    let fill = if transparent {
        Rgba([0, 0, 0, 0])
    } else {
        Rgba([0, 0, 0, 255])
    };
    // This helper computes a bounding box and shifts the source center into the
    // output center. Padding first can underflow for long, narrow images whose
    // rotated bounding box is narrower than the original image.
    let rotated = rotate_about_center_no_crop(
        image,
        radians as f32,
        Interpolation::Bilinear,
        Border::Constant(fill),
    );
    debug_assert_eq!(rotated.dimensions(), (width, height));
    Ok(rotated)
}

fn check_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("Image dimensions must be positive".to_owned());
    }
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err("Image dimensions exceed 16384 pixels per side".to_owned());
    }
    if u64::from(width)
        .checked_mul(u64::from(height))
        .is_none_or(|pixels| pixels > MAX_IMAGE_PIXELS)
    {
        return Err("Image exceeds the 16 megapixel processing limit".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        check_batch_payload, execute_json, request_exceeds_limit, MAX_BATCH_BASE64_BYTES,
        MAX_IMAGE_BYTES, MAX_RPC_BYTES,
    };
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
    use serde_json::{json, Value};
    use std::io::Cursor;

    fn png_base64(image: &RgbaImage) -> String {
        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut cursor, ImageFormat::Png)
            .unwrap();
        BASE64.encode(cursor.into_inner())
    }

    fn value(input: Value) -> Value {
        let response: Value = serde_json::from_str(&execute_json(&input.to_string())).unwrap();
        assert_eq!(response["ok"], true, "unexpected image error: {response}");
        response["value"].clone()
    }

    fn decoded_png(encoded: &str) -> RgbaImage {
        let bytes = BASE64.decode(encoded).unwrap();
        image::load_from_memory(&bytes).unwrap().to_rgba8()
    }

    #[test]
    fn crop_split_and_stitch_preserve_expected_rgba_pixels() {
        let mut source = RgbaImage::new(3, 2);
        source.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        source.put_pixel(1, 0, Rgba([0, 255, 0, 255]));
        source.put_pixel(2, 0, Rgba([0, 0, 255, 255]));
        source.put_pixel(0, 1, Rgba([255, 255, 0, 255]));
        source.put_pixel(1, 1, Rgba([255, 0, 255, 255]));
        source.put_pixel(2, 1, Rgba([0, 255, 255, 255]));
        let source_png = png_base64(&source);
        let decoded = value(json!({"op":"decode", "bytesBase64":source_png}));
        let normalized = decoded["pngBase64"].as_str().unwrap();
        let size = value(json!({"op":"size", "imageBase64":normalized}));
        assert_eq!((size["w"].as_u64(), size["h"].as_u64()), (Some(3), Some(2)));

        let crop =
            value(json!({"op":"crop", "imageBase64":normalized, "x":1, "y":0, "w":2, "h":1}));
        let crop = decoded_png(crop["pngBase64"].as_str().unwrap());
        assert_eq!(crop.dimensions(), (2, 1));
        assert_eq!(*crop.get_pixel(0, 0), Rgba([0, 255, 0, 255]));
        assert_eq!(*crop.get_pixel(1, 0), Rgba([0, 0, 255, 255]));

        let split = value(json!({"op":"split", "imageBase64":normalized, "rows":1, "cols":3}));
        let parts = split["imagesBase64"].as_array().unwrap();
        assert_eq!(parts.len(), 3);
        let colors = parts
            .iter()
            .map(|part| decoded_png(part.as_str().unwrap()).get_pixel(0, 0).0)
            .collect::<Vec<_>>();
        assert_eq!(
            colors,
            vec![[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
        );

        let stitched = value(
            json!({"op":"stitch", "imagesBase64":[parts[0], parts[1], parts[2]], "direction":"h"}),
        );
        let stitched = decoded_png(stitched["pngBase64"].as_str().unwrap());
        assert_eq!(stitched.dimensions(), (3, 2));
        assert_eq!(*stitched.get_pixel(2, 1), Rgba([0, 255, 255, 255]));
    }

    #[test]
    fn rotate_flip_and_encoding_follow_imageops_contract() {
        let mut source = RgbaImage::new(2, 1);
        source.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        source.put_pixel(1, 0, Rgba([0, 0, 255, 255]));
        let source = png_base64(&source);
        let decoded = value(json!({"op":"decode", "bytesBase64":source}));
        let normalized = decoded["pngBase64"].as_str().unwrap();

        let rotated = value(json!({"op":"rotate", "imageBase64":normalized, "deg":90}));
        let rotated = decoded_png(rotated["pngBase64"].as_str().unwrap());
        assert_eq!(rotated.dimensions(), (1, 2));
        assert_eq!(*rotated.get_pixel(0, 0), Rgba([255, 0, 0, 255]));
        assert_eq!(*rotated.get_pixel(0, 1), Rgba([0, 0, 255, 255]));

        let flipped = value(json!({"op":"flip", "imageBase64":normalized, "direction":"h"}));
        let flipped = decoded_png(flipped["pngBase64"].as_str().unwrap());
        assert_eq!(*flipped.get_pixel(0, 0), Rgba([0, 0, 255, 255]));

        let jpeg =
            value(json!({"op":"encode", "imageBase64":normalized, "format":"jpg", "quality":100}));
        let jpeg_bytes = BASE64
            .decode(jpeg["bytesBase64"].as_str().unwrap())
            .unwrap();
        let jpeg = image::load_from_memory(&jpeg_bytes).unwrap().to_rgb8();
        assert!(jpeg.get_pixel(0, 0).0[0] > jpeg.get_pixel(0, 0).0[2]);

        let webp =
            value(json!({"op":"encode", "imageBase64":normalized, "format":"webp", "quality":100}));
        let webp_bytes = BASE64
            .decode(webp["bytesBase64"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            image::load_from_memory(&webp_bytes).unwrap().dimensions(),
            (2, 1)
        );
    }

    #[test]
    fn arbitrary_rotation_expands_canvas_and_preserves_alpha_corners() {
        let mut source = RgbaImage::from_pixel(2, 1, Rgba([0, 0, 0, 0]));
        source.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let decoded = value(json!({"op":"decode", "bytesBase64":png_base64(&source)}));
        let rotated = value(json!({"op":"rotate", "imageBase64":decoded["pngBase64"], "deg":45}));
        let rotated = decoded_png(rotated["pngBase64"].as_str().unwrap());
        assert_eq!(rotated.dimensions(), (3, 3));
        assert_eq!(rotated.get_pixel(0, 0).0[3], 0);
        assert!(rotated
            .pixels()
            .any(|pixel| pixel.0[0] > 100 && pixel.0[3] > 0));
    }

    #[test]
    fn arbitrary_rotation_uses_clockwise_imageops_orientation() {
        let mut source = RgbaImage::from_pixel(5, 3, Rgba([0, 0, 255, 255]));
        source.put_pixel(0, 0, Rgba([255, 0, 0, 255]));

        for (deg, expected_marker_region) in [(45, (2..=3, 0..=1)), (-45, (0..=1, 3..=4))] {
            let rotated = super::rotate_arbitrary(&source, deg).expect("rotate image");
            let marker = rotated
                .enumerate_pixels()
                .max_by_key(|(_, _, pixel)| i16::from(pixel.0[0]) - i16::from(pixel.0[2]))
                .map(|(x, y, _)| (x, y))
                .expect("rotated marker pixel");
            assert!(
                expected_marker_region.0.contains(&marker.0)
                    && expected_marker_region.1.contains(&marker.1),
                "rotation {deg} placed top-left red marker at {marker:?}"
            );
        }
    }

    #[test]
    fn arbitrary_rotation_keeps_skinny_images_intact_at_both_angle_signs() {
        for deg in [80, -80] {
            let source = RgbaImage::from_pixel(100, 10, Rgba([230, 40, 20, 255]));
            let decoded = value(json!({"op":"decode", "bytesBase64":png_base64(&source)}));
            let rotated =
                value(json!({"op":"rotate", "imageBase64":decoded["pngBase64"], "deg":deg}));
            let rotated = decoded_png(rotated["pngBase64"].as_str().unwrap());
            assert_eq!(rotated.dimensions(), (28, 101));
            let retained = rotated
                .pixels()
                .filter(|pixel| pixel.0[0] > 180 && pixel.0[1] > 20 && pixel.0[3] > 0)
                .count();
            assert!(
                retained > 750,
                "rotation {deg} retained only {retained} pixels"
            );
        }
    }

    #[test]
    fn invalid_geometry_payload_and_output_limits_return_bounded_errors() {
        let mut source = RgbaImage::new(1, 1);
        source.put_pixel(0, 0, Rgba([1, 2, 3, 255]));
        let encoded = png_base64(&source);
        let error: Value = serde_json::from_str(&execute_json(
            &json!({"op":"crop", "imageBase64":encoded, "x":-1, "y":0, "w":1, "h":1}).to_string(),
        ))
        .unwrap();
        assert_eq!(error["ok"], false);
        assert!(error["error"].as_str().unwrap().contains("coordinates"));
        let error: Value =
            serde_json::from_str(&execute_json("{\"op\":\"decode\",\"bytesBase64\":\"!\"}"))
                .unwrap();
        assert_eq!(error["ok"], false);
        assert!(request_exceeds_limit(MAX_RPC_BYTES + 1));
        assert!(!request_exceeds_limit(MAX_RPC_BYTES));
        assert_eq!(MAX_IMAGE_BYTES, 64 * 1024 * 1024);
    }

    #[test]
    fn image_batch_json_output_limit_rejects_before_accumulating_more_tiles() {
        let mut total = MAX_BATCH_BASE64_BYTES - 4;
        check_batch_payload(&mut total, 4).expect("exact limit is accepted");
        assert_eq!(total, MAX_BATCH_BASE64_BYTES);
        assert!(check_batch_payload(&mut total, 1)
            .unwrap_err()
            .contains("80 MiB"));
        assert_eq!(total, MAX_BATCH_BASE64_BYTES);
    }
}
