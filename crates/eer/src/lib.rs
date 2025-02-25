use std::fs::File;
use std::path::Path;
use std::io::{Read, Seek, SeekFrom};
use std::collections::HashMap;
use quick_xml::Reader;
use quick_xml::events::Event;
use tiff::decoder::Decoder;
use tiff::tags::Tag;
use tiff::decoder::ifd::Value;
use anyhow::{Result, anyhow};
use ndarray::Array2;
use serde_derive::Serialize;


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitstream_basic() {
        let data = vec![0b10110001];  // Test data
        let mut bs = BitStream::new(&data);
        
        // Reading bits in LSB->MSB order within the byte
        assert_eq!(bs.get_bits(3), 0b001);     // First 3 bits: 001
        assert_eq!(bs.get_bits(3), 0b110);     // Next 3 bits: 110
        assert_eq!(bs.get_bits(2), 0b10);      // Last 2 bits: 10
        assert!(bs.no_bits_left());            // Should be at end now
    }
}

/// BitStream provides bit-level reading capabilities from a byte buffer
pub struct BitStream<'a> {
    buffer: &'a [u8],
    bit_pos: usize,  // index of next bit to read (from the start of buffer)
}

impl<'a> BitStream<'a> {
    /// Creates a new BitStream from a byte slice
    pub fn new(data_bytes: &'a [u8]) -> Self {
        BitStream {
            buffer: data_bytes,
            bit_pos: 0,
        }
    }

    /// Reads n bits (LSB->MSB within each byte) and returns integer value
    #[inline(always)]
    pub fn get_bits(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        
        let byte_index = self.bit_pos / 8;
        let bit_offset = self.bit_pos % 8;
        
        // Read 4 bytes (or less if at end of buffer)
        let mut chunk: u32 = 0;
        for i in 0..4.min(self.buffer.len() - byte_index) {
            chunk |= (self.buffer[byte_index + i] as u32) << (i * 8);
        }
        
        // Extract n bits starting at bit_offset
        let mask = if n == 32 { !0 } else { (1 << n) - 1 };
        let val = (chunk >> bit_offset) & mask;
        
        self.bit_pos += n as usize;
        val
    }

    /// Returns true if there are no more bits left to read
    pub fn no_bits_left(&self) -> bool {
        (self.buffer.len() * 8) <= self.bit_pos
    }
}

// Custom TIFF tags for EER format
const TAG_POS_SKIP_BITS: u16 = 65007;
const TAG_HORZ_SUB_BITS: u16 = 65008;
const TAG_VERT_SUB_BITS: u16 = 65009;
pub const TAG_XML_DATA: u16 = 65001;

pub fn parse_xml_metadata(xml_str: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(xml_str);
    let mut buf = Vec::new();
    let mut metadata = HashMap::new();
    let mut current_name = None;
    
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if e.name().as_ref() == b"item" {
                    // Get the name attribute
                    for attr in e.attributes() {
                        if let Ok(attr) = attr {
                            if attr.key.as_ref() == b"name" {
                                if let Ok(name) = String::from_utf8(attr.value.to_vec()) {
                                    current_name = Some(name);
                                }
                            }
                        }
                    }
                }
            },
            Ok(Event::Text(e)) => {
                if let (Some(name), Ok(text)) = (current_name.as_ref(), e.unescape()) {
                    let text = text.trim();
                    if !text.is_empty() {
                        metadata.insert(name.clone(), text.to_string());
                    }
                }
            },
            Ok(Event::End(ref e)) => {
                if e.name().as_ref() == b"item" {
                    current_name = None;
                }
            },
            Ok(Event::Eof) => break,
            Err(e) => eprintln!("Error parsing XML: {}", e),
            _ => (),
        }
    }
    buf.clear();
    
    metadata
}

#[derive(Debug)]
pub struct CompressionParams {
    pub code_len: u32,
    pub horz_sub_bits: u32,
    pub vert_sub_bits: u32,
}

pub fn get_compression_params(decoder: &mut Decoder<File>) -> Result<CompressionParams> {
    let compression = decoder.get_tag_u32(Tag::Compression)?;
    
    match compression {
        65000 => Ok(CompressionParams {
            code_len: 8,
            horz_sub_bits: 2,
            vert_sub_bits: 2,
        }),
        65001 => Ok(CompressionParams {
            code_len: 7,
            horz_sub_bits: 2,
            vert_sub_bits: 2,
        }),
        65002 => {
            // Read from custom tags
            let code_len = decoder.get_tag_u32(Tag::Unknown(TAG_POS_SKIP_BITS))?;
            let horz_sub_bits = decoder.get_tag_u32(Tag::Unknown(TAG_HORZ_SUB_BITS))?;
            let vert_sub_bits = decoder.get_tag_u32(Tag::Unknown(TAG_VERT_SUB_BITS))?;
            
            Ok(CompressionParams {
                code_len,
                horz_sub_bits,
                vert_sub_bits,
            })
        },
        _ => Err(anyhow!("Unsupported compression type: {}", compression))
    }
}

pub fn compression_to_string(compression: u32) -> &'static str {
    match compression {
        1 => "None",
        2 => "CCITT Group 3",
        3 => "CCITT T4",
        4 => "CCITT T6",
        5 => "LZW",
        6 => "JPEG (old)",
        7 => "JPEG",
        8 => "Adobe Deflate",
        9 => "JBIG B&W",
        10 => "JBIG Color",
        32773 => "PackBits",
        _ => "Unknown",
    }
}

pub fn sample_format_to_string(format: u32) -> &'static str {
    match format {
        1 => "Unsigned integer",
        2 => "Signed integer",
        3 => "IEEE floating point",
        4 => "Undefined",
        _ => "Unknown",
    }
}

pub fn save_image(image: &Array2<u16>, path: &str) -> Result<()> {
    // Convert to f32 for calculations
    let float_img = image.mapv(|x| x as f32);
    
    // Apply log scaling (add 1 to avoid log(0))
    let log_img = float_img.mapv(|x| (x + 1.0).ln());
    
    // Find min and max of log values
    let min_val = log_img.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let max_val = log_img.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let range = max_val - min_val;
    
    // Normalize to [0,1] then scale to [0,255]
    let scaled = log_img.mapv(|x| (((x - min_val) / range) * 255.0) as u8);
    
    // Convert to image buffer
    let height = scaled.shape()[0];
    let width = scaled.shape()[1];
    let (v, _offset) = scaled.as_standard_layout().into_owned().into_raw_vec_and_offset();

    let img = image::GrayImage::from_raw(
        width as u32,
        height as u32,
        v
    ).ok_or_else(|| anyhow!("Failed to create image"))?;
    
    // Save
    img.save(path)?;
    Ok(())
}

#[derive(Debug)]
struct StripInfo {
    offset: u64,
    size: u64,
}

fn get_strips_info(decoder: &mut Decoder<File>) -> Result<Vec<StripInfo>> {
    let offsets = decoder.get_tag_u64_vec(Tag::StripOffsets)?;
    let sizes = decoder.get_tag_u64_vec(Tag::StripByteCounts)?;
    
    Ok(offsets.into_iter()
        .zip(sizes.into_iter())
        .map(|(offset, size)| StripInfo { offset, size })
        .collect())
}

pub fn decode_eer_frame(
    decoder: &mut Decoder<File>,
    params: &CompressionParams,
    file: &mut File,  // Take file handle as parameter
) -> Result<Array2<u16>> {
    let height = decoder.dimensions()?.1 as usize;
    let width = decoder.dimensions()?.0 as usize;
    let mut image = Array2::<u16>::zeros((height, width));
    
    let strips_info = get_strips_info(decoder)?;
    let pos_skip_max = (1 << params.code_len) - 1;
    let rows_per_strip = decoder.get_tag_u32(Tag::RowsPerStrip)? as usize;
    
    // Pre-allocate buffer for largest strip
    let max_strip_size = strips_info.iter().map(|s| s.size as usize).max().unwrap_or(0);
    let mut raw_data = vec![0u8; max_strip_size];
    
    for (strip_idx, strip_info) in strips_info.iter().enumerate() {
        // Read strip data
        file.seek(SeekFrom::Start(strip_info.offset))?;
        file.read_exact(&mut raw_data[..strip_info.size as usize])?;
        
        let mut bs = BitStream::new(&raw_data[..strip_info.size as usize]);
        
        let start_row = strip_idx * rows_per_strip;
        let end_row = (start_row + rows_per_strip).min(height);
        let strip_pixel_start = start_row * width;
        let strip_pixel_end = end_row * width;
        
        let mut pos = 0;
        while (strip_pixel_start + pos) < strip_pixel_end {
            let skip = bs.get_bits(params.code_len);
            pos += skip as usize;
            
            if (strip_pixel_start + pos) >= strip_pixel_end {
                break;
            }
            
            if skip < pos_skip_max {
                // Read subpixel bits (currently ignored)
                let _v_sub = bs.get_bits(params.vert_sub_bits);
                let _h_sub = bs.get_bits(params.horz_sub_bits);
                
                // Calculate pixel position more efficiently
                let global_pixel = strip_pixel_start + pos;
                let row = global_pixel / width;
                let col = global_pixel % width;
                
                // Direct array access is faster than using index operator
                let slice = image.as_slice_mut().unwrap();
                slice[row * width + col] += 1;
                
                pos += 1;
            }
            // skip == max => no event here, continue
        }
    }
    
    Ok(image)
}

pub fn decode_frames(
    decoder: &mut Decoder<File>,
    params: &mut CompressionParams,
    path: &Path,
    num_frames: u32,
    skip_frames: Option<u32>,
) -> Result<Array2<u16>> {
    let mut file = File::open(path)?;
    // Get dimensions from first frame
    let height = decoder.dimensions()?.1;
    let width = decoder.dimensions()?.0;
    let mut sum_image = Array2::<u16>::zeros((height as usize, width as usize));

    // Calculate effective number of frames to process
    let step = skip_frames.unwrap_or(1);
    let frames_to_process = (num_frames + step - 1) / step;
    
    // Decode and sum frames with skipping
    for frame_idx in (0..num_frames).step_by(step as usize) {
        println!("Decoding frame {} of {} (total frames to process: {})", 
                frame_idx + 1, num_frames, frames_to_process);
        
        let frame_image = decode_eer_frame(decoder, params, &mut file)?;
        sum_image += &frame_image;

        // Skip frames
        for _ in 0..step.min(num_frames - frame_idx - 1) {
            if decoder.more_images() {
                decoder.next_image()?;
                // Update compression params for new frame
                *params = get_compression_params(decoder)?;
            }
        }
    }

    Ok(sum_image)
}


#[derive(Debug, Serialize)]
pub enum VoxelType {
    UnsignedInt16,
}


#[derive(Debug, Serialize)]
pub struct ImageData {
    size_x: i32,
    size_y: i32,
    size_z: i32,
    size_t: i32,
    size_c: i32,
    voxel_type: VoxelType,
    voxel_spacing_x: f32,
    voxel_spacing_y: f32,
    voxel_spacing_z: f32,
}


pub fn generate_thumbnail(path: &Path, output_path: &Path) -> Result<()> {
    let file = File::open(path)?;
    let mut decoder = Decoder::new(file)?;
    
    // Get compression parameters
    let mut params = get_compression_params(&mut decoder)?;
    
    // Determine number of frames
    let mut num_frames = 1;
    let mut temp_decoder = decoder.clone();
    while temp_decoder.more_images() {
        num_frames += 1;
        temp_decoder.next_image()?;
    }
    
    // Decode frames with skipping (process every 10th frame for speed)
    let skip_frames = Some(10);
    let image = decode_frames(&mut decoder, &mut params, path, num_frames, skip_frames)?;
    
    // Save the thumbnail
    save_image(&image, output_path.to_str().unwrap())?;
    
    Ok(())
}

pub fn show_header_info(path: &Path) -> Result<()> {
    let file = File::open(path)?;
    let mut decoder = Decoder::new(file)?;
    
    let mut image_data = ImageData {
        size_x: 0,
        size_y: 0,
        size_z: 1,
        size_t: 1,
        size_c: 1,
        voxel_type: VoxelType::UnsignedInt16,
        voxel_spacing_x: 0.0,
        voxel_spacing_y: 0.0,
        voxel_spacing_z: 0.0,
    };
    
    if let Ok(dims) = decoder.dimensions() {
        image_data.size_x = dims.0 as i32;
        image_data.size_y = dims.1 as i32;
        
        // Get XML metadata
        match decoder.get_tag(Tag::Unknown(TAG_XML_DATA)) {
            Ok(value) => {
                // println!("Value (debug): {:?}", value);
        
                match value {
                    // You might still have other variants, handle them as needed
                    Value::List(list_of_values) => {
                        // println!("\nDebug: Found List variant with {} values", list_of_values.len());
                        // Convert [Byte(60), Byte(109), ...] into a real Vec<u8>
                        let bytes: Vec<u8> = list_of_values.iter()
                            .filter_map(|v| {
                                if let Value::Byte(b) = v {
                                    Some(*b)  // Byte(60) -> 60
                                } else {
                                    None      // skip any non-Byte items
                                }
                            })
                            .collect();
        
                        // Now try interpreting those bytes as UTF-8 text
                        if let Ok(xml_str) = String::from_utf8(bytes) {
                            // println!("\nDebug: Successfully converted bytes to UTF-8 string");
                            // println!("Debug: XML content:\n{}", xml_str);
                            let metadata = parse_xml_metadata(&xml_str);
                            
                            // Extract pixel sizes
                            if let Some(width) = metadata.get("sensorPixelSize.width") {
                                if let Ok(width) = width.parse::<f32>() {
                                    image_data.voxel_spacing_x = width;
                                }
                            }
                            if let Some(height) = metadata.get("sensorPixelSize.height") {
                                if let Ok(height) = height.parse::<f32>() {
                                    image_data.voxel_spacing_y = height;
                                }
                            }
                        } else {
                            println!("Not valid UTF-8");
                        }
                    },
        
                    // If you still have an Ascii or a single Byte variant, handle them here...
                    Value::Ascii(s) => {
                        println!("Ascii text: {s}");
                    }
                    Value::Byte(b) => {
                        println!("Single byte: {b}");
                    }
                    _ => {
                        println!("Unhandled variant");
                    }
                }
            }
            Err(e) => {
                println!("Error: {:?}", e);
            }
        }
        
        
    }
    
    // Count total pages
    let mut page_count = 1;
    while decoder.more_images() {
        page_count += 1;
        decoder.next_image()?;
    }
    
    println!("\nTotal number of pages in TIFF: {}", page_count);
    
    // Output JSON representation
    // println!("\nImage Data:");
    println!("{}", serde_json::to_string_pretty(&image_data)?);
    Ok(())
}
