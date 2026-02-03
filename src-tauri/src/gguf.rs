use serde::Serialize;
use specta::Type;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

#[derive(Serialize, Clone, Type, Debug, Default)]
pub struct GGUFMetadata {
    pub architecture: String,
    #[specta(type = f64)]
    pub context_length: u64,
    #[specta(type = f64)]
    pub embedding_length: u64,
    #[specta(type = f64)]
    pub block_count: u64,
    #[specta(type = f64)]
    pub head_count: u64,
    #[specta(type = f64)]
    pub head_count_kv: u64,
    pub file_type: u32,
}

pub fn read_gguf_metadata(path: &str) -> Result<GGUFMetadata, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;

    // Read Magic
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != b"GGUF" {
        return Err("Not a GGUF file".to_string());
    }

    // Version
    let mut version_bytes = [0u8; 4];
    file.read_exact(&mut version_bytes)
        .map_err(|e| e.to_string())?;
    let version = u32::from_le_bytes(version_bytes);
    if version != 2 && version != 3 {
        return Err(format!("Unsupported GGUF version: {}", version));
    }

    // Tensor Count
    let mut tensor_count_bytes = [0u8; 8];
    file.read_exact(&mut tensor_count_bytes)
        .map_err(|e| e.to_string())?;
    // let _tensor_count = u64::from_le_bytes(tensor_count_bytes);

    // Metadata KV Count
    let mut kv_count_bytes = [0u8; 8];
    file.read_exact(&mut kv_count_bytes)
        .map_err(|e| e.to_string())?;
    let kv_count = u64::from_le_bytes(kv_count_bytes);

    let mut metadata = GGUFMetadata::default();

    for _ in 0..kv_count {
        // Read Key
        let key = read_gguf_string(&mut file)?;

        // Read Value Type
        let mut type_bytes = [0u8; 4];
        file.read_exact(&mut type_bytes)
            .map_err(|e| e.to_string())?;
        let val_type = u32::from_le_bytes(type_bytes);

        // Process based on key
        match key.as_str() {
            "general.architecture" => {
                metadata.architecture = read_value_string(&mut file, val_type)?;
            }
            _ if key.ends_with(".context_length") => {
                metadata.context_length = read_value_u64(&mut file, val_type)?;
            }
            _ if key.ends_with(".embedding_length") => {
                metadata.embedding_length = read_value_u64(&mut file, val_type)?;
            }
            _ if key.ends_with(".block_count") => {
                metadata.block_count = read_value_u64(&mut file, val_type)?;
            }
            _ if key.ends_with(".attention.head_count") => {
                metadata.head_count = read_value_u64(&mut file, val_type)?;
            }
            _ if key.ends_with(".attention.head_count_kv") => {
                metadata.head_count_kv = read_value_u64(&mut file, val_type)?;
            }
            "general.file_type" => {
                metadata.file_type = read_value_u32(&mut file, val_type)?;
            }
            _ => {
                // Skip value
                skip_value(&mut file, val_type)?;
            }
        }
    }

    // Heuristic: if head_count_kv is 0 (missing), it usually means it equals head_count (no GQA)
    if metadata.head_count_kv == 0 {
        metadata.head_count_kv = metadata.head_count;
    }

    Ok(metadata)
}

fn read_gguf_string(file: &mut File) -> Result<String, String> {
    let mut len_bytes = [0u8; 8];
    file.read_exact(&mut len_bytes).map_err(|e| e.to_string())?;
    let len = u64::from_le_bytes(len_bytes) as usize;

    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).map_err(|e| e.to_string())?;

    String::from_utf8(buf).map_err(|e| e.to_string())
}

fn read_value_string(file: &mut File, val_type: u32) -> Result<String, String> {
    if val_type != 8 {
        return Err("Expected string".to_string());
    }
    read_gguf_string(file)
}

fn read_value_u64(file: &mut File, val_type: u32) -> Result<u64, String> {
    match val_type {
        4 => {
            // UINT32
            let mut b = [0u8; 4];
            file.read_exact(&mut b).map_err(|e| e.to_string())?;
            Ok(u32::from_le_bytes(b) as u64)
        }
        10 => {
            // UINT64
            let mut b = [0u8; 8];
            file.read_exact(&mut b).map_err(|e| e.to_string())?;
            Ok(u64::from_le_bytes(b))
        }
        _ => Err(format!("Expected UINT32/UINT64, got type {}", val_type)),
    }
}

fn read_value_u32(file: &mut File, val_type: u32) -> Result<u32, String> {
    if val_type != 4 {
        return Err("Expected UINT32".to_string());
    }
    let mut b = [0u8; 4];
    file.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(u32::from_le_bytes(b))
}

fn skip_value(file: &mut File, val_type: u32) -> Result<(), String> {
    match val_type {
        0..=7 | 11 => {
            // Fixed size types (1, 2, 4, 8 bytes)
            let sizes = [1, 1, 2, 2, 4, 4, 4, 1]; // UINT8, INT8, UINT16, INT16, UINT32, INT32, FLOAT32, BOOL
            let size = if val_type < 8 {
                sizes[val_type as usize]
            } else {
                1
            };
            file.seek(SeekFrom::Current(size))
                .map_err(|e| e.to_string())?;
        }
        8 => {
            // String
            let mut len_bytes = [0u8; 8];
            file.read_exact(&mut len_bytes).map_err(|e| e.to_string())?;
            let len = u64::from_le_bytes(len_bytes);
            file.seek(SeekFrom::Current(len as i64))
                .map_err(|e| e.to_string())?;
        }
        9 => {
            // Array
            let mut arr_type_bytes = [0u8; 4];
            file.read_exact(&mut arr_type_bytes)
                .map_err(|e| e.to_string())?;
            let arr_type = u32::from_le_bytes(arr_type_bytes);

            let mut len_bytes = [0u8; 8];
            file.read_exact(&mut len_bytes).map_err(|e| e.to_string())?;
            let len = u64::from_le_bytes(len_bytes);

            for _ in 0..len {
                skip_value(file, arr_type)?;
            }
        }
        10 | 12 | 13 => {
            // UINT64, INT64, FLOAT64 (8 bytes)
            file.seek(SeekFrom::Current(8)).map_err(|e| e.to_string())?;
        }
        _ => return Err(format!("Unknown GGUF type: {}", val_type)),
    }
    Ok(())
}
