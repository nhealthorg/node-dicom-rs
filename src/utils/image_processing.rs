use dicom_object::{InMemDicomObject, from_reader};
use dicom_core::Tag;
use std::io::Cursor;
use image::{ImageBuffer, Luma, RgbImage, DynamicImage, ImageFormat};

/// Image rendering options
#[derive(Debug, Clone)]
pub struct ImageRenderOptions {
    /// Output width in pixels (None = original size)
    pub width: Option<u32>,
    /// Output height in pixels (None = original size)
    pub height: Option<u32>,
    /// JPEG quality (1-100, only for JPEG format)
    pub quality: Option<u8>,
    /// Window center for grayscale windowing
    pub window_center: Option<f32>,
    /// Window width for grayscale windowing
    pub window_width: Option<f32>,
    /// Apply VOI LUT from DICOM file
    pub apply_voi_lut: Option<bool>,
    /// Rescale intercept (for Hounsfield units in CT)
    pub rescale_intercept: Option<f64>,
    /// Rescale slope
    pub rescale_slope: Option<f64>,
    /// Convert to 8-bit output
    pub convert_to_8bit: Option<bool>,
    /// Frame number to extract (0-based, for multi-frame images)
    pub frame_number: Option<u32>,
    /// Output format
    pub format: ImageOutputFormat,
}

impl Default for ImageRenderOptions {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            quality: Some(90),
            window_center: None,
            window_width: None,
            apply_voi_lut: None,
            rescale_intercept: None,
            rescale_slope: None,
            convert_to_8bit: None,
            frame_number: None,
            format: ImageOutputFormat::Jpeg,
        }
    }
}

/// Output image format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageOutputFormat {
    Jpeg,
    Png,
    Bmp,
}

impl ImageOutputFormat {
    pub fn from_mime_type(mime: &str) -> Self {
        let lower = mime.to_lowercase();
        if lower.contains("png") {
            ImageOutputFormat::Png
        } else if lower.contains("bmp") {
            ImageOutputFormat::Bmp
        } else {
            ImageOutputFormat::Jpeg
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            ImageOutputFormat::Jpeg => "image/jpeg",
            ImageOutputFormat::Png => "image/png",
            ImageOutputFormat::Bmp => "image/bmp",
        }
    }
}

/// Render DICOM image to standard image format (JPEG/PNG/BMP)
/// 
/// This function handles:
/// - 8-bit and 16-bit grayscale images
/// - RGB images with conversion to grayscale
/// - Automatic or manual windowing for 16-bit images
/// - VOI LUT application with rescale intercept/slope
/// - Frame extraction from multi-frame images
/// - Resizing with high-quality Lanczos3 filter
/// - Multiple output formats (JPEG, PNG, BMP)
/// - Configurable JPEG quality
/// - 8-bit conversion for display
pub fn render_dicom_image(
    buffer: &[u8],
    options: &ImageRenderOptions,
) -> Result<Vec<u8>, String> {
    // Parse DICOM file
    let obj = from_reader(Cursor::new(buffer))
        .map_err(|e| format!("Failed to parse DICOM: {}", e))?;

    let encapsulated = obj.element(Tag(0x7FE0, 0x0010)).ok()
        .is_some_and(|e| matches!(e.value(), dicom_core::value::Value::PixelSequence(_)));
    if options.frame_number.is_some() || encapsulated {
        return render_decoded_frame(&obj, options.frame_number.unwrap_or(0), options);
    }

    render_dicom_object(&obj, options)
}

#[cfg(not(feature = "transcode"))]
fn render_decoded_frame(
    _obj: &dicom_object::DefaultDicomObject,
    _frame: u32,
    _options: &ImageRenderOptions,
) -> Result<Vec<u8>, String> {
    Err("Selected-frame and compressed rendering require the transcode feature".into())
}

/// Functional-group values override shared values, which override legacy root attributes.
#[cfg(feature = "transcode")]
fn frame_attribute<'a>(
    obj: &'a InMemDicomObject,
    frame: u32,
    sequence: Tag,
    attribute: Tag,
) -> Option<&'a dicom_object::mem::InMemElement> {
    let nested = |group: Tag, index: usize| {
        obj.element(group).ok()?.items()?.get(index)?
            .element(sequence).ok()?.items()?.first()?
            .element(attribute).ok()
    };
    nested(Tag(0x5200, 0x9230), frame as usize)
        .or_else(|| nested(Tag(0x5200, 0x9229), 0))
        .or_else(|| obj.element(attribute).ok())
}

#[cfg(feature = "transcode")]
fn decode_rle8_frame(obj: &dicom_object::DefaultDicomObject, frame: u32, expected: usize) -> Result<Vec<u8>, String> {
    let error = || "Invalid 8-bit monochrome RLE frame".to_string();
    let element = obj.element(Tag(0x7FE0, 0x0010)).map_err(|_| error())?;
    let dicom_core::value::Value::PixelSequence(sequence) = element.value() else {
        return Err(error());
    };
    let fragment = sequence.fragments().get(frame as usize).ok_or_else(error)?;
    if fragment.len() < 64 || u32::from_le_bytes(fragment[0..4].try_into().unwrap()) != 1 {
        return Err(error());
    }
    let offset = u32::from_le_bytes(fragment[4..8].try_into().unwrap()) as usize;
    if offset < 64 {
        return Err(error());
    }
    let mut encoded = fragment.get(offset..).ok_or_else(error)?;
    let mut pixels = Vec::with_capacity(expected);
    while pixels.len() < expected {
        let (&control, rest) = encoded.split_first().ok_or_else(error)?;
        encoded = rest;
        match control as i8 {
            0..=127 => {
                let count = control as usize + 1;
                let literal = encoded.get(..count).ok_or_else(error)?;
                pixels.extend_from_slice(literal);
                encoded = &encoded[count..];
            }
            -127..=-1 => {
                let (&value, rest) = encoded.split_first().ok_or_else(error)?;
                pixels.resize(pixels.len() + (1 - control as i8 as i16) as usize, value);
                encoded = rest;
            }
            -128 => {}
        }
        if pixels.len() > expected {
            return Err(error());
        }
    }
    Ok(pixels)
}

#[cfg(feature = "transcode")]
fn render_decoded_frame(
    obj: &dicom_object::DefaultDicomObject,
    frame: u32,
    options: &ImageRenderOptions,
) -> Result<Vec<u8>, String> {
    use dicom_pixeldata::PixelDecoder;

    let integer = |tag| {
        obj.element(tag).map_err(|e| e.to_string())?
            .to_int::<u16>().map_err(|e| e.to_string())
    };
    let frames = obj.element(Tag(0x0028, 0x0008)).ok()
        .and_then(|e| e.to_int::<u32>().ok()).unwrap_or(1);
    if frame >= frames {
        return Err(format!("Frame {} out of range (1-{})", frame + 1, frames));
    }
    let photometric = obj.element(Tag(0x0028, 0x0004))
        .map_err(|e| e.to_string())?.to_str().map_err(|e| e.to_string())?;
    if integer(Tag(0x0028, 0x0002))? != 1
        || !matches!(photometric.trim(), "MONOCHROME1" | "MONOCHROME2")
    {
        return Err(format!("Unsupported photometric interpretation for frame rendering: {}", photometric));
    }
    let bits = integer(Tag(0x0028, 0x0100))?;
    let stored = integer(Tag(0x0028, 0x0101))?;
    let high_bit = integer(Tag(0x0028, 0x0102))?;
    let representation = integer(Tag(0x0028, 0x0103))?;
    if !matches!(bits, 8 | 16) || stored == 0 || stored > bits
        || high_bit >= bits || high_bit + 1 < stored || representation > 1
    {
        return Err("Unsupported monochrome pixel layout (expected 8/16-bit integer samples)".into());
    }
    let rows = integer(Tag(0x0028, 0x0010))? as u32;
    let cols = integer(Tag(0x0028, 0x0011))? as u32;
    if rows == 0 || cols == 0 {
        return Err("Invalid image dimensions".into());
    }
    let expected = rows as usize * cols as usize * (bits as usize / 8);
    // dicom-transfer-syntax-registry 0.9.1 shifts mono RLE8 samples by one byte
    // and drops the last sample. Decode that one layout locally until upstream fixes it.
    let rle8 = if bits == 8 && obj.meta().transfer_syntax() == "1.2.840.10008.1.2.5" {
        Some(decode_rle8_frame(obj, frame, expected)?)
    } else {
        None
    };
    let decoded = if rle8.is_none() {
        Some(obj.decode_pixel_data_frame(frame)
            .map_err(|e| format!("Failed to decode frame {}: {}", frame + 1, e))?)
    } else {
        None
    };
    let data = rle8.as_deref()
        .or_else(|| decoded.as_ref().map(|d| d.data())).ok_or("Missing decoded frame")?;
    if data.len() != expected {
        return Err(format!("Decoded frame has unexpected size: expected {}, got {}", expected, data.len()));
    }
    let value = |sequence, tag| {
        frame_attribute(obj, frame, sequence, tag).and_then(|e| e.to_float64().ok())
    };
    let slope = options.rescale_slope
        .or_else(|| value(Tag(0x0028, 0x9145), Tag(0x0028, 0x1053))).unwrap_or(1.0);
    let intercept = options.rescale_intercept
        .or_else(|| value(Tag(0x0028, 0x9145), Tag(0x0028, 0x1052))).unwrap_or(0.0);
    let window = match (options.window_center, options.window_width) {
        (Some(c), Some(w)) => Some((c as f64, w as f64)),
        _ if options.apply_voi_lut.unwrap_or(false) => {
            value(Tag(0x0028, 0x9132), Tag(0x0028, 0x1050))
                .zip(value(Tag(0x0028, 0x9132), Tag(0x0028, 0x1051)))
        }
        _ => None,
    };
    if !slope.is_finite() || !intercept.is_finite()
        || window.is_some_and(|(c, w)| !c.is_finite() || !w.is_finite() || w < 1.0)
    {
        return Err("Invalid rescale or window parameters".into());
    }
    let shift = high_bit + 1 - stored;
    let mask = (1u32 << stored) - 1;
    let pixels: Vec<f64> = data.chunks_exact(bits as usize / 8).map(|bytes| {
        // Decoders and InMemDicomObject expose native-endian samples.
        let raw = if bits == 8 { bytes[0] as u32 }
            else { u16::from_ne_bytes([bytes[0], bytes[1]]) as u32 };
        let sample = (raw >> shift) & mask;
        let signed = if representation == 1 && sample & (1 << (stored - 1)) != 0 {
            sample as i32 - (1i32 << stored)
        } else {
            sample as i32
        };
        signed as f64 * slope + intercept
    }).collect();
    let (min, max) = pixels.iter().fold((f64::INFINITY, f64::NEG_INFINITY),
        |(min, max), &p| (min.min(p), max.max(p)));
    let grayscale: Vec<u8> = pixels.iter().map(|&p| {
        let normalized = if let Some((center, width)) = window {
            if width == 1.0 {
                if p > center - 0.5 { 1.0 } else { 0.0 }
            } else {
                ((p - (center - 0.5)) / (width - 1.0) + 0.5).clamp(0.0, 1.0)
            }
        } else if bits == 8 && options.rescale_slope.is_none()
            && slope == 1.0 && intercept == 0.0 && representation == 0
        {
            p / 255.0
        } else if max > min {
            (p - min) / (max - min)
        } else { 0.0 };
        let gray = (normalized * 255.0).round() as u8;
        if photometric.trim() == "MONOCHROME1" { 255 - gray } else { gray }
    }).collect();
    let gray = ImageBuffer::<Luma<u8>, _>::from_raw(cols, rows, grayscale)
        .ok_or("Failed to build frame image")?;
    let rgb = grayscale_to_rgb(&gray);
    let resized = resize_image(&rgb, options.width, options.height);
    encode_image(&resized, options)
}

/// Render DICOM image to file with specified format
pub fn render_dicom_to_file(
    buffer: &[u8],
    output_path: &str,
    options: &ImageRenderOptions,
) -> Result<String, String> {
    let output = render_dicom_image(buffer, options)?;
    
    std::fs::write(output_path, &output)
        .map_err(|e| format!("Failed to write file: {}", e))?;
    
    let format_str = match options.format {
        ImageOutputFormat::Jpeg => "JPEG",
        ImageOutputFormat::Png => "PNG",
        ImageOutputFormat::Bmp => "BMP",
    };
    
    Ok(format!("{} image saved to {} ({} bytes)", format_str, output_path, output.len()))
}

/// Render DICOM object to image format
pub fn render_dicom_object(
    obj: &InMemDicomObject,
    options: &ImageRenderOptions,
) -> Result<Vec<u8>, String> {
    // Get image dimensions
    let rows = obj.element(Tag(0x0028, 0x0010))
        .map_err(|e| format!("Failed to get Rows: {}", e))?
        .to_int::<u16>()
        .map_err(|e| format!("Failed to parse Rows: {}", e))? as u32;
    
    let cols = obj.element(Tag(0x0028, 0x0011))
        .map_err(|e| format!("Failed to get Columns: {}", e))?
        .to_int::<u16>()
        .map_err(|e| format!("Failed to parse Columns: {}", e))? as u32;
    
    let bits_allocated = obj.element(Tag(0x0028, 0x0100))
        .ok()
        .and_then(|e| e.to_int::<u16>().ok())
        .unwrap_or(16);
    
    let samples_per_pixel = obj.element(Tag(0x0028, 0x0002))
        .ok()
        .and_then(|e| e.to_int::<u16>().ok())
        .unwrap_or(1);
    
    let pixel_representation = obj.element(Tag(0x0028, 0x0103))
        .ok()
        .and_then(|e| e.to_int::<u16>().ok())
        .unwrap_or(0);
    
    let num_frames = obj.element(Tag(0x0028, 0x0008))
        .ok()
        .and_then(|e| e.to_int::<u32>().ok())
        .unwrap_or(1);
    
    // Get rescale parameters if available or from options
    let rescale_intercept = options.rescale_intercept
        .or_else(|| obj.element(Tag(0x0028, 0x1052))
            .ok()
            .and_then(|e| e.to_float64().ok()));
    
    let rescale_slope = options.rescale_slope
        .or_else(|| obj.element(Tag(0x0028, 0x1053))
            .ok()
            .and_then(|e| e.to_float64().ok()));
    
    // Get window center/width from file if VOI LUT requested
    let file_window_center = obj.element(Tag(0x0028, 0x1050))
        .ok()
        .and_then(|e| e.to_float64().ok());
    
    let file_window_width = obj.element(Tag(0x0028, 0x1051))
        .ok()
        .and_then(|e| e.to_float64().ok());
    
    // Determine final window parameters
    let (window_center, window_width) = if let (Some(c), Some(w)) = (options.window_center, options.window_width) {
        // Manual override
        (Some(c as f64), Some(w as f64))
    } else if options.apply_voi_lut.unwrap_or(false) {
        // Use file parameters
        (file_window_center, file_window_width)
    } else {
        (None, None)
    };
    
    // Get pixel data
    let pixel_data = obj.element(Tag(0x7FE0, 0x0010))
        .map_err(|e| format!("Pixel data not found: {}", e))?;
    
    let mut raw_pixels = pixel_data.to_bytes()
        .map_err(|e| format!("Failed to read pixel data: {}", e))?
        .to_vec();
    
    // Extract specific frame if requested
    if let Some(frame_num) = options.frame_number {
        if frame_num >= num_frames {
            return Err(format!("Frame {} out of range (0-{})", frame_num, num_frames - 1));
        }
        
        let bytes_per_pixel = (bits_allocated / 8) as usize;
        let frame_size = rows as usize * cols as usize * samples_per_pixel as usize * bytes_per_pixel;
        let start = frame_num as usize * frame_size;
        let end = start + frame_size;
        
        if end > raw_pixels.len() {
            return Err(format!("Frame extraction failed: calculated size {} exceeds buffer size {}", end, raw_pixels.len()));
        }
        
        raw_pixels = raw_pixels[start..end].to_vec();
    }
    
    // Convert to grayscale image with advanced processing
    let grayscale_img = if samples_per_pixel == 1 {
        // Grayscale image
        convert_grayscale_pixels_advanced(
            &raw_pixels,
            cols,
            rows,
            bits_allocated,
            pixel_representation,
            window_center,
            window_width,
            rescale_intercept,
            rescale_slope,
        )?
    } else if samples_per_pixel == 3 {
        // RGB image - convert to grayscale
        convert_rgb_to_grayscale(&raw_pixels, cols, rows, bits_allocated)?
    } else {
        return Err(format!("Unsupported samples per pixel: {}", samples_per_pixel));
    };
    
    // Convert to RGB for encoding
    let rgb_img = grayscale_to_rgb(&grayscale_img);
    
    // Resize if requested
    let final_img = if options.width.is_some() || options.height.is_some() {
        resize_image(&rgb_img, options.width, options.height)
    } else {
        rgb_img
    };
    
    // Encode to requested format
    encode_image(&final_img, options)
}

/// Convert grayscale pixel data with advanced windowing and VOI LUT
fn convert_grayscale_pixels_advanced(
    raw_pixels: &[u8],
    cols: u32,
    rows: u32,
    bits_allocated: u16,
    pixel_representation: u16,
    window_center: Option<f64>,
    window_width: Option<f64>,
    rescale_intercept: Option<f64>,
    rescale_slope: Option<f64>,
) -> Result<ImageBuffer<Luma<u8>, Vec<u8>>, String> {
    let mut img_buffer = ImageBuffer::<Luma<u8>, Vec<u8>>::new(cols, rows);
    
    if bits_allocated == 8 {
        // 8-bit data - direct copy (no windowing for 8-bit)
        for (i, &pixel) in raw_pixels.iter().take((rows * cols) as usize).enumerate() {
            let x = (i as u32) % cols;
            let y = (i as u32) / cols;
            img_buffer.put_pixel(x, y, Luma([pixel]));
        }
    } else if bits_allocated == 16 {
        // 16-bit data - normalize to 8-bit with optional VOI LUT
        let pixels_16: Vec<f64> = if pixel_representation == 0 {
            // Unsigned
            raw_pixels
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]) as f64)
                .collect()
        } else {
            // Signed
            raw_pixels
                .chunks_exact(2)
                .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f64)
                .collect()
        };
        
        // Apply rescale if provided
        let rescaled: Vec<f64> = if let (Some(slope), Some(intercept)) = (rescale_slope, rescale_intercept) {
            pixels_16.iter().map(|&p| p * slope + intercept).collect()
        } else {
            pixels_16
        };
        
        // Determine windowing parameters
        let (min_val, max_val) = if let (Some(center), Some(width)) = (window_center, window_width) {
            // Use manual windowing
            let min = center - width / 2.0;
            let max = center + width / 2.0;
            (min, max)
        } else {
            // Auto window - find min/max
            let min = rescaled.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = rescaled.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            (min, max)
        };
        
        // Avoid division by zero
        let range = if (max_val - min_val).abs() < 1e-6 {
            1.0
        } else {
            max_val - min_val
        };
        
        // Normalize to 0-255
        for (i, &pixel_val) in rescaled.iter().enumerate().take((rows * cols) as usize) {
            let x = (i as u32) % cols;
            let y = (i as u32) / cols;
            
            let normalized = if pixel_val <= min_val {
                0u8
            } else if pixel_val >= max_val {
                255u8
            } else {
                (((pixel_val - min_val) / range) * 255.0) as u8
            };
            
            img_buffer.put_pixel(x, y, Luma([normalized]));
        }
    } else {
        return Err(format!("Unsupported bits allocated: {}", bits_allocated));
    }
    
    Ok(img_buffer)
}

/// Convert RGB pixel data to grayscale
fn convert_rgb_to_grayscale(
    raw_pixels: &[u8],
    cols: u32,
    rows: u32,
    bits_allocated: u16,
) -> Result<ImageBuffer<Luma<u8>, Vec<u8>>, String> {
    let mut img_buffer = ImageBuffer::<Luma<u8>, Vec<u8>>::new(cols, rows);
    
    if bits_allocated == 8 {
        // 8-bit RGB data
        for i in 0..(rows * cols) as usize {
            let idx = i * 3;
            if idx + 2 < raw_pixels.len() {
                let r = raw_pixels[idx] as f32;
                let g = raw_pixels[idx + 1] as f32;
                let b = raw_pixels[idx + 2] as f32;
                
                // Convert to grayscale using luminosity method
                let gray = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
                
                let x = (i as u32) % cols;
                let y = (i as u32) / cols;
                img_buffer.put_pixel(x, y, Luma([gray]));
            }
        }
    } else {
        return Err(format!("Unsupported bits allocated for RGB: {}", bits_allocated));
    }
    
    Ok(img_buffer)
}

/// Convert grayscale image to RGB
fn grayscale_to_rgb(img: &ImageBuffer<Luma<u8>, Vec<u8>>) -> RgbImage {
    ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
        let gray = img.get_pixel(x, y).0[0];
        image::Rgb([gray, gray, gray])
    })
}

/// Resize image with high-quality Lanczos3 filter
fn resize_image(
    img: &RgbImage,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> RgbImage {
    let (orig_width, orig_height) = (img.width(), img.height());
    
    let (new_width, new_height) = match (target_width, target_height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => {
            // Maintain aspect ratio based on width
            let aspect = orig_height as f32 / orig_width as f32;
            (w, (w as f32 * aspect) as u32)
        }
        (None, Some(h)) => {
            // Maintain aspect ratio based on height
            let aspect = orig_width as f32 / orig_height as f32;
            ((h as f32 * aspect) as u32, h)
        }
        (None, None) => return img.clone(),
    };
    
    image::imageops::resize(
        img,
        new_width,
        new_height,
        image::imageops::FilterType::Lanczos3
    )
}

/// Encode image to requested format
fn encode_image(
    img: &RgbImage,
    options: &ImageRenderOptions,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    
    match options.format {
        ImageOutputFormat::Png => {
            img.write_to(&mut Cursor::new(&mut output), ImageFormat::Png)
                .map_err(|e| format!("PNG encoding failed: {}", e))?;
        }
        ImageOutputFormat::Bmp => {
            img.write_to(&mut Cursor::new(&mut output), ImageFormat::Bmp)
                .map_err(|e| format!("BMP encoding failed: {}", e))?;
        }
        ImageOutputFormat::Jpeg => {
            let quality = options.quality.unwrap_or(90);
            let mut jpeg_encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut output, 
                quality
            );
            jpeg_encoder.encode(
                img,
                img.width(),
                img.height(),
                image::ExtendedColorType::Rgb8,
            ).map_err(|e| format!("JPEG encoding failed: {}", e))?;
        }
    }
    
    Ok(output)
}

/// Helper function to parse viewport string (e.g., "512,512" or "512x512")
pub fn parse_viewport(viewport: &str) -> Result<(u32, u32), String> {
    let parts: Vec<&str> = if viewport.contains(',') {
        viewport.split(',').collect()
    } else if viewport.contains('x') {
        viewport.split('x').collect()
    } else {
        return Err(format!("Invalid viewport format: {}", viewport));
    };
    
    if parts.len() != 2 {
        return Err(format!("Invalid viewport format: {}", viewport));
    }
    
    let width = parts[0].trim().parse::<u32>()
        .map_err(|_| format!("Invalid width: {}", parts[0]))?;
    let height = parts[1].trim().parse::<u32>()
        .map_err(|_| format!("Invalid height: {}", parts[1]))?;
    
    Ok((width, height))
}

/// Helper function to parse window string (e.g., "40,400")
pub fn parse_window(window: &str) -> Result<(f32, f32), String> {
    let parts: Vec<&str> = window.split(',').collect();
    
    if parts.len() != 2 {
        return Err(format!("Invalid window format: {}", window));
    }
    
    let center = parts[0].trim().parse::<f32>()
        .map_err(|_| format!("Invalid window center: {}", parts[0]))?;
    let width = parts[1].trim().parse::<f32>()
        .map_err(|_| format!("Invalid window width: {}", parts[1]))?;
    
    Ok((center, width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "transcode")]
    #[test]
    fn test_rle8_literal_repeat_noop_and_truncation() {
        use dicom_core::{value::PixelFragmentSequence, DataElement, VR};
        use dicom_object::FileMetaTableBuilder;

        let decode = |packets: &[u8]| {
            let mut fragment = vec![0; 64];
            fragment[0] = 1;
            fragment[4] = 64;
            fragment.extend_from_slice(packets);
            let obj = InMemDicomObject::from_element_iter([DataElement::new(
                Tag(0x7FE0, 0x0010),
                VR::OB,
                PixelFragmentSequence::new(vec![], vec![fragment]),
            )]);
            let file = obj.with_meta(FileMetaTableBuilder::new()
                .media_storage_sop_class_uid("1.2.840.10008.5.1.4.1.1.2")
                .media_storage_sop_instance_uid("1.2.3")
                .transfer_syntax("1.2.840.10008.1.2.5")).unwrap();
            decode_rle8_frame(&file, 0, 4)
        };
        assert_eq!(decode(&[3, 0, 10, 20, 255]).unwrap(), [0, 10, 20, 255]);
        assert_eq!(decode(&[128, 253, 42]).unwrap(), [42; 4]);
        assert_eq!(decode(&[1, 5, 6, 255, 7]).unwrap(), [5, 6, 7, 7]);
        for invalid in [&[][..], &[3, 1], &[252, 1], &[253]] {
            assert!(decode(invalid).is_err());
        }
    }

    #[test]
    fn test_parse_viewport() {
        assert_eq!(parse_viewport("512,512").unwrap(), (512, 512));
        assert_eq!(parse_viewport("512x512").unwrap(), (512, 512));
        assert_eq!(parse_viewport("1024,768").unwrap(), (1024, 768));
        assert!(parse_viewport("invalid").is_err());
        assert!(parse_viewport("512").is_err());
    }

    #[test]
    fn test_parse_window() {
        assert_eq!(parse_window("40,400").unwrap(), (40.0, 400.0));
        assert_eq!(parse_window("100,200").unwrap(), (100.0, 200.0));
        assert!(parse_window("invalid").is_err());
        assert!(parse_window("40").is_err());
    }

    #[test]
    fn test_image_output_format() {
        assert_eq!(ImageOutputFormat::from_mime_type("image/jpeg"), ImageOutputFormat::Jpeg);
        assert_eq!(ImageOutputFormat::from_mime_type("image/png"), ImageOutputFormat::Png);
        assert_eq!(ImageOutputFormat::from_mime_type("image/bmp"), ImageOutputFormat::Bmp);
        assert_eq!(ImageOutputFormat::from_mime_type("unknown"), ImageOutputFormat::Jpeg);
    }
}
