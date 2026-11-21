use std::fmt;

use serde::{Deserialize, Serialize};
use zstd::stream::{decode_all, encode_all};

/// Compression algorithm to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionAlgorithm {
    /// No compression.
    None,
    /// Zstandard compression (good balance of speed and ratio).
    Zstd,
}

impl Default for CompressionAlgorithm {
    fn default() -> Self {
        Self::Zstd
    }
}

impl fmt::Display for CompressionAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Zstd => write!(f, "zstd"),
        }
    }
}

/// Compression errors.
#[derive(Debug, thiserror::Error)]
pub enum CompressionError {
    #[error("Zstd compression error: {0}")]
    Zstd(String),

    #[error("Zstd decompression error: {0}")]
    ZstdDecompress(String),

    #[error("invalid compression algorithm: {0}")]
    InvalidAlgorithm(String),

    #[error("data too large: {size} bytes (max {max})")]
    DataTooLarge { size: usize, max: usize },

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Compressor for snapshot data.
#[derive(Debug, Clone)]
pub struct Compressor {
    algorithm: CompressionAlgorithm,
    level: i32,
}

impl Compressor {
    /// Creates a new compressor with default settings.
    pub fn new() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Zstd,
            level: 3, // Good balance of speed and compression
        }
    }

    /// Creates a compressor with specific algorithm.
    pub fn with_algorithm(algorithm: CompressionAlgorithm) -> Self {
        Self {
            algorithm,
            level: 3,
        }
    }

    /// Creates a compressor with specific compression level (for Zstd).
    pub fn with_level(level: i32) -> Self {
        Self {
            algorithm: CompressionAlgorithm::Zstd,
            level: level.clamp(1, 22),
        }
    }

    /// Compresses data.
    pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>, CompressionError> {
        match self.algorithm {
            CompressionAlgorithm::None => Ok(data.to_vec()),
            CompressionAlgorithm::Zstd => {
                let compressed = encode_all(data, self.level)
                    .map_err(|e| CompressionError::Zstd(e.to_string()))?;
                Ok(compressed)
            }
        }
    }

    /// Decompresses data.
    pub fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, CompressionError> {
        match self.algorithm {
            CompressionAlgorithm::None => Ok(data.to_vec()),
            CompressionAlgorithm::Zstd => {
                let decompressed = decode_all(data)
                    .map_err(|e| CompressionError::ZstdDecompress(e.to_string()))?;
                Ok(decompressed)
            }
        }
    }

    /// Compresses data and returns the compression ratio.
    pub fn compress_with_ratio(&self, data: &[u8]) -> Result<(Vec<u8>, f64), CompressionError> {
        let compressed = self.compress(data)?;
        let ratio = if !data.is_empty() {
            compressed.len() as f64 / data.len() as f64
        } else {
            1.0
        };
        Ok((compressed, ratio))
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

/// Streaming compressor for large data.
pub struct StreamingCompressor {
    algorithm: CompressionAlgorithm,
    level: i32,
}

impl StreamingCompressor {
    pub fn new(algorithm: CompressionAlgorithm) -> Self {
        Self {
            algorithm,
            level: 3,
        }
    }

    pub fn with_level(algorithm: CompressionAlgorithm, level: i32) -> Self {
        Self {
            algorithm,
            level: level.clamp(1, 22),
        }
    }

    /// Compresses data in chunks (for very large data).
    pub fn compress_stream(
        &self,
        input: &mut dyn std::io::Read,
        output: &mut dyn std::io::Write,
    ) -> Result<(), CompressionError> {
        match self.algorithm {
            CompressionAlgorithm::None => {
                std::io::copy(input, output)?;
                Ok(())
            }
            CompressionAlgorithm::Zstd => {
                let mut encoder = zstd::stream::Encoder::new(output, self.level)
                    .map_err(|e| CompressionError::Zstd(e.to_string()))?;
                std::io::copy(input, &mut encoder)?;
                encoder.finish()?;
                Ok(())
            }
        }
    }

    /// Decompresses data from a stream.
    pub fn decompress_stream(
        &self,
        input: &mut dyn std::io::Read,
        output: &mut dyn std::io::Write,
    ) -> Result<(), CompressionError> {
        match self.algorithm {
            CompressionAlgorithm::None => {
                std::io::copy(input, output)?;
                Ok(())
            }
            CompressionAlgorithm::Zstd => {
                let mut decoder = zstd::stream::Decoder::new(input)
                    .map_err(|e| CompressionError::ZstdDecompress(e.to_string()))?;
                std::io::copy(&mut decoder, output)?;
                Ok(())
            }
        }
    }
}

impl Default for StreamingCompressor {
    fn default() -> Self {
        Self::new(CompressionAlgorithm::Zstd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress() {
        let compressor = Compressor::new();
        let data = b"Hello, World! This is a test string for compression.";

        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_no_compression() {
        let compressor = Compressor::with_algorithm(CompressionAlgorithm::None);
        let data = b"Hello, World!";

        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(data.to_vec(), compressed);
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_compression_ratio() {
        let compressor = Compressor::new();
        let data = "A".repeat(1000).into_bytes();

        let (_compressed, ratio) = compressor.compress_with_ratio(&data).unwrap();

        // Highly compressible data should have low ratio
        assert!(ratio < 0.1);
    }

    #[test]
    fn test_streaming_compressor() {
        let compressor = StreamingCompressor::new(CompressionAlgorithm::Zstd);
        let data = b"Streaming compression test data for large files.";
        let mut input = std::io::Cursor::new(data);
        let mut output = Vec::new();

        compressor.compress_stream(&mut input, &mut output).unwrap();

        let decompressor = StreamingCompressor::new(CompressionAlgorithm::Zstd);
        let mut decompressed = Vec::new();
        decompressor
            .decompress_stream(&mut std::io::Cursor::new(&output), &mut decompressed)
            .unwrap();

        assert_eq!(data.to_vec(), decompressed);
    }
}
