mod error;
pub use error::MrcError;

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use serde::Serialize;
use image::{ImageBuffer, Rgb};

#[derive(Debug, Serialize)]
pub enum VoxelType {
    Float32,
    Float64,
    Int8,
    UInt8,
    Int16,
    UInt16,
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

impl ImageData {
    pub fn from_mrc(header: &MrcHeader) -> Self {
        let voxel_type = match header.mode {
            0 => VoxelType::Int8,
            1 => VoxelType::Int16,
            2 => VoxelType::Float32,
            6 => VoxelType::UInt16,
            _ => VoxelType::Float32, // default to Float32 for unknown modes
        };

        ImageData {
            size_x: header.nx,
            size_y: header.ny,
            size_z: header.nz,
            size_t: 1,  // MRC files don't have time dimension
            size_c: 1,  // MRC files don't have channel dimension
            voxel_type,
            voxel_spacing_x: header.pixel_size[0],
            voxel_spacing_y: header.pixel_size[1],
            voxel_spacing_z: header.pixel_size[2],
        }
    }
}

#[derive(Debug)]
pub struct MrcHeader {
    nx: i32,
    ny: i32,
    nz: i32,
    mode: i32,
    cell_dims: [f32; 3],
    cell_angles: [f32; 3],
    map_axis: [i32; 3],
    pixel_size: [f32; 3],
}

impl MrcHeader {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, MrcError> {
        let mut header = MrcHeader {
            nx: reader.read_i32::<LittleEndian>()?,
            ny: reader.read_i32::<LittleEndian>()?,
            nz: reader.read_i32::<LittleEndian>()?,
            mode: reader.read_i32::<LittleEndian>()?,
            cell_dims: [0.0; 3],
            cell_angles: [0.0; 3],
            map_axis: [0; 3],
            pixel_size: [0.0; 3], // x, y, z in Angstroms
        };

        // Read cell dimensions at offset 10
        for dim in &mut header.cell_dims {
            *dim = reader.read_f32::<LittleEndian>()?;
        }

        // Skip to pixel size at offset 40
        reader.seek(SeekFrom::Start(40))?;
        
        // Read pixel sizes and divide by grid dimensions
        header.pixel_size[0] = reader.read_f32::<LittleEndian>()? / header.nx as f32;
        header.pixel_size[1] = reader.read_f32::<LittleEndian>()? / header.ny as f32;
        header.pixel_size[2] = reader.read_f32::<LittleEndian>()? / header.nz as f32;

        for angle in &mut header.cell_angles {
            *angle = reader.read_f32::<LittleEndian>()?;
        }

        for axis in &mut header.map_axis {
            *axis = reader.read_i32::<LittleEndian>()?;
        }

        if header.mode < 0 || header.mode > 6 {
            return Err(MrcError::Format("Invalid mode value".to_string()));
        }

        Ok(header)
    }
}

pub struct MrcFile {
    header: MrcHeader,
    image_data: ImageData,
    path: String,
}

impl MrcFile {
    pub fn open(path: &str) -> Result<Self, MrcError> {
        let mut file = File::open(path)?;
        let header = MrcHeader::read(&mut file)?;
        let image_data = ImageData::from_mrc(&header);
        
        Ok(MrcFile { 
            header, 
            image_data, 
            path: path.to_string() 
        })
    }

    pub fn get_image_data(&self) -> &ImageData {
        &self.image_data
    }

    pub fn save_thumbnail(&self, path: &str, downsample: u32) -> Result<(), MrcError> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(1024))?; // Skip header

        // Calculate thumbnail dimensions
        let thumb_width = (self.header.nx as u32 + downsample - 1) / downsample;
        let thumb_height = (self.header.ny as u32 + downsample - 1) / downsample;
        
        // Create buffer for downsampled data
        let mut downsampled = vec![0.0f32; (thumb_width * thumb_height) as usize];
        let mut min_val = f32::INFINITY;
        let mut max_val = f32::NEG_INFINITY;

        match self.header.mode {
            0 => { // 8-bit signed
                let mut buffer = [0i8; 1];
                for y in 0..thumb_height {
                    let src_y = (y * downsample) as usize;
                    for x in 0..thumb_width {
                        let src_x = (x * downsample) as usize;
                        let offset = 1024 + (src_y * self.header.nx as usize + src_x);
                        file.seek(SeekFrom::Start(offset as u64))?;
                        file.read_exact(unsafe { std::slice::from_raw_parts_mut(&mut buffer[0] as *mut i8 as *mut u8, 1) })?;
                        let value = buffer[0] as f32;
                        min_val = min_val.min(value);
                        max_val = max_val.max(value);
                        downsampled[(y * thumb_width + x) as usize] = value;
                    }
                }
            },
            1 => { // 16-bit signed
                let mut buffer = [0i16; 1];
                for y in 0..thumb_height {
                    let src_y = (y * downsample) as usize;
                    for x in 0..thumb_width {
                        let src_x = (x * downsample) as usize;
                        let offset = 1024 + 2 * (src_y * self.header.nx as usize + src_x);
                        file.seek(SeekFrom::Start(offset as u64))?;
                        file.read_i16_into::<LittleEndian>(&mut buffer)?;
                        let value = buffer[0] as f32;
                        min_val = min_val.min(value);
                        max_val = max_val.max(value);
                        downsampled[(y * thumb_width + x) as usize] = value;
                    }
                }
            },
            2 => { // 32-bit float
                let mut buffer = [0.0f32; 1];
                for y in 0..thumb_height {
                    let src_y = (y * downsample) as usize;
                    for x in 0..thumb_width {
                        let src_x = (x * downsample) as usize;
                        let offset = 1024 + 4 * (src_y * self.header.nx as usize + src_x);
                        file.seek(SeekFrom::Start(offset as u64))?;
                        file.read_f32_into::<LittleEndian>(&mut buffer)?;
                        let value = buffer[0];
                        min_val = min_val.min(value);
                        max_val = max_val.max(value);
                        downsampled[(y * thumb_width + x) as usize] = value;
                    }
                }
            },
            6 => { // 16-bit unsigned
                let mut buffer = [0u16; 1];
                for y in 0..thumb_height {
                    let src_y = (y * downsample) as usize;
                    for x in 0..thumb_width {
                        let src_x = (x * downsample) as usize;
                        let offset = 1024 + 2 * (src_y * self.header.nx as usize + src_x);
                        file.seek(SeekFrom::Start(offset as u64))?;
                        file.read_u16_into::<LittleEndian>(&mut buffer)?;
                        let value = buffer[0] as f32;
                        min_val = min_val.min(value);
                        max_val = max_val.max(value);
                        downsampled[(y * thumb_width + x) as usize] = value;
                    }
                }
            },
            _ => return Err(MrcError::Format("Unsupported mode for thumbnails".to_string())),
        }

        let range = max_val - min_val;
        
        // Create the thumbnail
        let mut img = ImageBuffer::new(thumb_width, thumb_height);
        
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            let idx = (y * thumb_width + x) as usize;
            let normalized = if range != 0.0 {
                (downsampled[idx] - min_val) / range
            } else {
                0.0
            };
            
            let value = (normalized * 255.0) as u8;
            *pixel = Rgb([value, value, value]);
        }
        
        img.save(path).map_err(|e| MrcError::Io(io::Error::new(io::ErrorKind::Other, e)))?;
        Ok(())
    }
}
