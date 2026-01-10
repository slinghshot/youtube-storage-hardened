use image::Rgb;

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
pub const DEFAULT_FPS: u32 = 30;

// Block size for each bit (larger = more resilient to compression)
pub const BLOCK_SIZE: u32 = 4; // 4x4 pixel block per bit

// Calculate blocks per frame
pub const BLOCKS_X: u32 = WIDTH / BLOCK_SIZE; // 320 blocks horizontally
pub const BLOCKS_Y: u32 = HEIGHT / BLOCK_SIZE; // 180 blocks vertically
pub const BITS_PER_FRAME: usize = (BLOCKS_X * BLOCKS_Y) as usize; // 57,600 bits
pub const BYTES_PER_FRAME: usize = BITS_PER_FRAME / 8; // 7,200 bytes

// Black and white colors
pub const COLOR_BLACK: Rgb<u8> = Rgb([0, 0, 0]); // Bit 0
pub const COLOR_WHITE: Rgb<u8> = Rgb([255, 255, 255]); // Bit 1
