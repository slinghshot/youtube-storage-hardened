pub const WIDTH: usize = 1280;
pub const HEIGHT: usize = 720;
pub const BLOCK: usize = 4;
pub const FRAME_BYTES: usize = WIDTH * HEIGHT / (BLOCK * BLOCK * 8);
pub const LEGACY_MAX_PAYLOAD: usize = 64 << 20;
pub const LEGACY_MAX_EXPANDED: u64 = 256 << 20;
pub const MAX_ENTRIES: usize = 4096;
