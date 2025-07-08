pub fn swap_half_word_byte_sequence_u32(value: u32) -> u32 {
    // 将每个16位半字互换
    ((value & 0x0000FFFF) << 16) | ((value & 0xFFFF0000) >> 16)
}

pub fn swap_word_byte_sequence_u32(value: u32) -> u32 {
    ((value & 0x000000FF) << 24)
        | ((value & 0x0000FF00) << 8)
        | ((value & 0x00FF0000) >> 8)
        | ((value & 0xFF000000) >> 24)
}
